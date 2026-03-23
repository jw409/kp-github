use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// Get the authenticated user's profile.
pub async fn me(client: &GithubClient) -> Result<Value, ClientError> {
    client.api("/user", &[]).await
}

/// Search users.
pub async fn search(
    client: &GithubClient,
    query: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let per_page = limit.unwrap_or(30).min(100);
    let url = format!(
        "/search/users?q={}&per_page={per_page}",
        crate::util::urlencode(query)
    );
    let result = client.api(&url, &[]).await?;

    crate::tools::search_util::extract_search_items(&result, limit)
}

#[cfg(test)]
fn search_url(query: &str, per_page: u32) -> String {
    format!("/search/users?q={}&per_page={per_page}", crate::util::urlencode(query))
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_search_url() {
        let url = search_url("location:sf", 10);
        assert!(url.starts_with("/search/users?q="));
        assert!(url.contains("per_page=10"));
    }

    #[test]
    fn test_extract_search_items() {
        let result = json!({"total_count": 1, "items": [{"login": "alice"}]});
        let extracted = crate::tools::search_util::extract_search_items(&result, None).unwrap();
        assert_eq!(extracted["items"][0]["login"], "alice");
        assert_eq!(extracted["total_count"], 1);
    }

    #[test]
    fn test_extract_search_items_no_items_key() {
        let result = json!({"data": []});
        let extracted = crate::tools::search_util::extract_search_items(&result, None).unwrap();
        // Returns the original object when no "items" key
        assert!(extracted.is_object());
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_me_fn() {
        let client = GithubClient::mock(vec![json!({"login": "jw409"})]);
        let result = me(&client).await.unwrap();
        assert_eq!(result["login"], "jw409");
    }

    #[tokio::test]
    async fn test_search_users() {
        let client = GithubClient::mock(vec![json!({"total_count": 1, "items": [{"login": "alice"}]})]);
        let result = search(&client, "location:sf", Some(10)).await.unwrap();
        assert!(result["items"].is_array());
        assert_eq!(result["total_count"], 1);
    }

    #[tokio::test]
    async fn test_search_users_no_items() {
        let client = GithubClient::mock(vec![json!({"data": "other"})]);
        let result = search(&client, "foo", None).await.unwrap();
        assert!(result.is_object());
    }
}
