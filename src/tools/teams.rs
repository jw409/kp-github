use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// List teams for the authenticated user.
pub async fn list(client: &GithubClient) -> Result<Value, ClientError> {
    client.api("/user/teams", &[]).await
}

/// Get members of a team.
pub async fn members(
    client: &GithubClient,
    org: &str,
    team_slug: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/orgs/{org}/teams/{team_slug}/members");
    client.api(&endpoint, &[]).await
}

#[cfg(test)]
fn members_endpoint(org: &str, team_slug: &str) -> String {
    format!("/orgs/{org}/teams/{team_slug}/members")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_members_endpoint() {
        assert_eq!(members_endpoint("my-org", "core"), "/orgs/my-org/teams/core/members");
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_list_teams() {
        let client = GithubClient::mock(vec![json!([{"name": "core"}])]);
        let result = list(&client).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_members_teams() {
        let client = GithubClient::mock(vec![json!([{"login": "alice"}, {"login": "bob"}])]);
        let result = members(&client, "my-org", "core").await.unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }
}
