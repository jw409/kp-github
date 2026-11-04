//! Exercise account routing through the MCP wire protocol and real HTTP clients.
use crate::{
    accounts::{Accounts, EventSetup},
    events::EventsClient,
    github::client::GithubClient,
    server,
};
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

struct Mcp {
    read: BufReader<ReadHalf<DuplexStream>>,
    write: WriteHalf<DuplexStream>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mcp {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Mcp {
    async fn start(handler: server::KpGithubServer) -> Self {
        let (client, server) = tokio::io::duplex(65536);
        let task = tokio::spawn(async move {
            handler
                .serve(server)
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let (read, write) = tokio::io::split(client);
        let mut this = Self {
            read: BufReader::new(read),
            write,
            task,
        };
        this.send(json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"scope-test","version":"1"}
        }})).await;
        assert!(this.reply().await.get("result").is_some());
        this.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        this
    }
    async fn send(&mut self, request: Value) {
        self.write
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
    }
    async fn reply(&mut self) -> Value {
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.read.read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

#[tokio::test]
async fn concurrent_sessions_keep_accounts_orgs_resources_and_events_separate() {
    let api = MockServer::start().await;
    let events = MockServer::start().await;
    for name in ["work-dev", "other-dev"] {
        for org in ["FirstOrg", "SecondOrg"] {
            Mock::given(method("GET"))
                .and(path(format!("/repos/{org}/repo")))
                .and(header("authorization", format!("Bearer {name}")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"name":format!("{name}-{org}")})),
                )
                .mount(&api)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/v1/listeners"))
            .and(header("authorization", format!("Bearer event-{name}")))
            .and(header("x-kp-github-account", name))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"listeners":[name]})))
            .expect(1)
            .mount(&events)
            .await;
    }
    let handler = server::configured(Accounts::with_clients(
        ["work-dev", "other-dev"]
            .into_iter()
            .map(|name| {
                (
                    name,
                    GithubClient::http_with_base_url(&api.uri(), name),
                    EventSetup {
                        client: Some(Arc::new(EventsClient::with_test_account(
                            &events.uri(),
                            &format!("event-{name}"),
                            name,
                        ))),
                        error: None,
                    },
                )
            })
            .collect(),
    ));
    let mut first = Mcp::start(handler.clone()).await;
    let mut second = Mcp::start(handler).await;
    // Identical request IDs in separate sessions; several requests concurrently
    // in each session, crossing both organizations with each credential.
    for index in 0..12 {
        for (client, name) in [(&mut first, "work-dev"), (&mut second, "other-dev")] {
            let org = if index % 2 == 0 {
                "FirstOrg"
            } else {
                "SecondOrg"
            };
            client.send(json!({"jsonrpc":"2.0","id":index+1,"method":"tools/call","params":{
                "name":"github_repos_get","arguments":{"github_account":name,"owner":org,"repo":"repo","format":"json"}
            }})).await;
        }
    }
    for _ in 0..12 {
        for (client, name) in [(&mut first, "work-dev"), (&mut second, "other-dev")] {
            let response = client.reply().await;
            let org = if response["id"].as_u64().unwrap() % 2 == 1 {
                "FirstOrg"
            } else {
                "SecondOrg"
            };
            assert!(
                response["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("{name}-{org}")),
                "{response}"
            );
        }
    }
    for (client, name) in [(&mut first, "work-dev"), (&mut second, "other-dev")] {
        client
            .send(
                json!({"jsonrpc":"2.0","id":30,"method":"resources/read","params":{
                    "uri":format!("github://accounts/{name}/repos/SecondOrg/repo")
                }}),
            )
            .await;
        assert!(client.reply().await["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains(name));
        client
            .send(
                json!({"jsonrpc":"2.0","id":31,"method":"tools/call","params":{
                    "name":"github_events","arguments":{"github_account":name,"action":"list"}
                }}),
            )
            .await;
        assert!(client.reply().await["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(name));
    }
    let before = api.received_requests().await.unwrap().len();
    for arguments in [
        json!({}),
        json!({"github_account":"unlisted"}),
        json!({"github_account":null}),
        json!({"github_account":""}),
    ] {
        first.send(json!({"jsonrpc":"2.0","id":40,"method":"tools/call","params":{"name":"github_user_me","arguments":arguments}})).await;
        assert!(first.reply().await.get("error").is_some());
    }
    first.send(json!({"jsonrpc":"2.0","id":41,"method":"resources/read","params":{"uri":"github://repos/FirstOrg/repo"}})).await;
    assert!(first.reply().await.get("error").is_some());
    assert_eq!(api.received_requests().await.unwrap().len(), before);
    first
        .send(json!({"jsonrpc":"2.0","id":42,"method":"tools/list","params":{}}))
        .await;
    let response = first.reply().await;
    let tools = response["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 67);
    for tool in tools {
        assert_eq!(
            tool["inputSchema"]["properties"]["github_account"]["enum"],
            json!(["other-dev", "work-dev"])
        );
        assert!(tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("github_account")));
        if let Some(variants) = tool["inputSchema"]["oneOf"].as_array() {
            for variant in variants {
                assert!(variant["properties"].get("github_account").is_some());
            }
        }
    }
}
