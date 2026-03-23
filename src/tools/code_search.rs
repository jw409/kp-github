use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// Search code using GitHub code search syntax.
pub async fn search(
    client: &GithubClient,
    query: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let per_page = limit.unwrap_or(30).min(100);
    let url = format!("/search/code?q={}&per_page={per_page}", crate::util::urlencode(query));
    let result = client.api(&url, &[]).await?;

    crate::tools::search_util::extract_search_items(&result, limit)
}

#[cfg(test)]
fn search_url(query: &str, per_page: u32) -> String {
    format!("/search/code?q={}&per_page={per_page}", crate::util::urlencode(query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_search_url() {
        let url = search_url("fn main language:rust", 5);
        assert!(url.starts_with("/search/code?q="));
        assert!(url.contains("per_page=5"));
    }

    // search_util unit tests are in search_util.rs

    #[tokio::test]
    async fn test_search_code() {
        let client = GithubClient::mock(vec![json!({"total_count": 1, "items": [{"path": "a.rs"}]})]);
        let result = search(&client, "fn main", Some(5)).await.unwrap();
        assert!(result["items"].is_array());
        assert_eq!(result["total_count"], 1);
    }

    #[tokio::test]
    async fn test_search_code_no_limit() {
        let client = GithubClient::mock(vec![json!({"total_count": 1, "items": [{"path": "b.rs"}]})]);
        let result = search(&client, "fn main", None).await.unwrap();
        assert!(result["items"].is_array());
    }

    #[tokio::test]
    async fn test_search_code_with_limit() {
        let client = GithubClient::mock(vec![json!({"total_count": 3, "items": [{"id": 1}, {"id": 2}, {"id": 3}]})]);
        let result = search(&client, "test", Some(2)).await.unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_search_code_no_items() {
        let client = GithubClient::mock(vec![json!({"data": "other"})]);
        let result = search(&client, "foo", None).await.unwrap();
        assert!(result.is_object());
    }

    #[tokio::test]
    async fn test_search_code_non_object() {
        let client = GithubClient::mock(vec![json!("string_response")]);
        let result = search(&client, "foo", None).await.unwrap();
        assert!(result.is_string());
    }
}
