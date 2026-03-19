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

    // Extract .items from search response
    match result {
        Value::Object(ref map) => {
            if let Some(items) = map.get("items") {
                if let Some(limit) = limit {
                    if let Some(arr) = items.as_array() {
                        let limited: Vec<Value> =
                            arr.iter().take(limit as usize).cloned().collect();
                        return Ok(Value::Array(limited));
                    }
                }
                return Ok(items.clone());
            }
            Ok(result)
        }
        _ => Ok(result),
    }
}

#[cfg(test)]
fn search_url(query: &str, per_page: u32) -> String {
    format!("/search/code?q={}&per_page={per_page}", crate::util::urlencode(query))
}

#[cfg(test)]
fn extract_search_items(result: &Value, limit: Option<u32>) -> Value {
    match result {
        Value::Object(ref map) => {
            if let Some(items) = map.get("items") {
                if let Some(limit) = limit {
                    if let Some(arr) = items.as_array() {
                        let limited: Vec<Value> =
                            arr.iter().take(limit as usize).cloned().collect();
                        return Value::Array(limited);
                    }
                }
                return items.clone();
            }
            result.clone()
        }
        _ => result.clone(),
    }
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

    #[test]
    fn test_extract_search_items_with_items() {
        let result = json!({"total_count": 2, "items": [{"path": "a.rs"}, {"path": "b.rs"}]});
        let items = extract_search_items(&result, None);
        assert_eq!(items.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_extract_search_items_with_limit() {
        let result = json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]});
        let items = extract_search_items(&result, Some(1));
        assert_eq!(items.as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_extract_search_items_no_items() {
        let result = json!({"data": "x"});
        let items = extract_search_items(&result, None);
        assert!(items.is_object());
    }

    #[test]
    fn test_extract_search_items_non_object() {
        let result = json!("string");
        let items = extract_search_items(&result, None);
        assert!(items.is_string());
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_search_code() {
        let client = GithubClient::mock(vec![json!({"items": [{"path": "a.rs"}]})]);
        let result = search(&client, "fn main", Some(5)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_search_code_no_limit() {
        let client = GithubClient::mock(vec![json!({"items": [{"path": "b.rs"}]})]);
        let result = search(&client, "fn main", None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_search_code_with_limit() {
        let client = GithubClient::mock(vec![json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]})]);
        let result = search(&client, "test", Some(2)).await.unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
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
