use serde_json::{json, Value};

use crate::github::client::ClientError;

/// Extract `.items` from a GitHub search API response, preserving `total_count`
/// as metadata so callers know if results are truncated.
///
/// Returns a JSON object: `{"items": [...], "total_count": N, "truncated": bool}`
pub fn extract_search_items(result: &Value, limit: Option<u32>) -> Result<Value, ClientError> {
    if let Value::Object(map) = result {
        let total_count = map.get("total_count").and_then(|v| v.as_u64()).unwrap_or(0);
        if let Some(items) = map.get("items") {
            let items = if let (Some(limit), Some(arr)) = (limit, items.as_array()) {
                Value::Array(arr.iter().take(limit as usize).cloned().collect())
            } else {
                items.clone()
            };
            let returned = items.as_array().map(|a| a.len() as u64).unwrap_or(0);
            return Ok(json!({
                "items": items,
                "total_count": total_count,
                "truncated": returned < total_count,
            }));
        }
    }
    Ok(result.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_preserves_total_count() {
        let result = json!({"total_count": 100, "items": [{"id": 1}, {"id": 2}]});
        let extracted = extract_search_items(&result, None).unwrap();
        assert_eq!(extracted["total_count"], 100);
        assert_eq!(extracted["items"].as_array().unwrap().len(), 2);
        assert_eq!(extracted["truncated"], true);
    }

    #[test]
    fn test_extract_not_truncated() {
        let result = json!({"total_count": 2, "items": [{"id": 1}, {"id": 2}]});
        let extracted = extract_search_items(&result, None).unwrap();
        assert_eq!(extracted["truncated"], false);
    }

    #[test]
    fn test_extract_with_limit() {
        let result = json!({"total_count": 5, "items": [{"id": 1}, {"id": 2}, {"id": 3}]});
        let extracted = extract_search_items(&result, Some(2)).unwrap();
        assert_eq!(extracted["items"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_extract_no_items_key() {
        let result = json!({"data": "other"});
        let extracted = extract_search_items(&result, None).unwrap();
        assert!(extracted.is_object());
        assert_eq!(extracted["data"], "other");
    }

    #[test]
    fn test_extract_non_object() {
        let result = json!("string");
        let extracted = extract_search_items(&result, None).unwrap();
        assert!(extracted.is_string());
    }
}
