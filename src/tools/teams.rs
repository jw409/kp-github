use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// List teams for the authenticated user.
pub async fn list(client: &GithubClient, limit: Option<u32>) -> Result<Value, ClientError> {
    client.api_list("/user/teams", &[], limit).await
}

/// Get members of a team.
pub async fn members(
    client: &GithubClient,
    org: &str,
    team_slug: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    // GitHub team slugs are normalized to a-z0-9-, but defense in depth:
    // if a caller passes a display name by mistake, encode rather than 404.
    let encoded_slug = crate::util::urlencode_path(team_slug);
    let endpoint = format!("/orgs/{org}/teams/{encoded_slug}/members");
    client.api_list(&endpoint, &[], limit).await
}

#[cfg(test)]
fn members_endpoint(org: &str, team_slug: &str) -> String {
    let encoded_slug = crate::util::urlencode_path(team_slug);
    format!("/orgs/{org}/teams/{encoded_slug}/members")
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
        let result = list(&client, None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_members_teams() {
        let client = GithubClient::mock(vec![json!([{"login": "alice"}, {"login": "bob"}])]);
        let result = members(&client, "my-org", "core", None).await.unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }
}
