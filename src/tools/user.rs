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

    // Extract .items from search response
    if let Value::Object(ref map) = result {
        if let Some(items) = map.get("items") {
            return Ok(items.clone());
        }
    }
    Ok(result)
}

#[cfg(test)]
fn search_url(query: &str, per_page: u32) -> String {
    format!("/search/users?q={}&per_page={per_page}", crate::util::urlencode(query))
}

#[cfg(test)]
fn extract_search_items(result: &Value) -> Option<Value> {
    if let Value::Object(ref map) = result {
        if let Some(items) = map.get("items") {
            return Some(items.clone());
        }
    }
    None
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
        let items = extract_search_items(&result).unwrap();
        assert_eq!(items[0]["login"], "alice");
    }

    #[test]
    fn test_extract_search_items_none() {
        let result = json!({"data": []});
        assert!(extract_search_items(&result).is_none());
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
        let client = GithubClient::mock(vec![json!({"items": [{"login": "alice"}]})]);
        let result = search(&client, "location:sf", Some(10)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_search_users_no_items() {
        let client = GithubClient::mock(vec![json!({"data": "other"})]);
        let result = search(&client, "foo", None).await.unwrap();
        assert!(result.is_object());
    }
}
