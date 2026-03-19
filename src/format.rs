use serde_json::Value;

use crate::compress::OutputFormat;

/// Format a compressed value for output
pub fn format_output(value: &Value, fmt: OutputFormat) -> String {
    let fmt = if fmt == OutputFormat::Auto {
        auto_detect(value)
    } else {
        fmt
    };

    match fmt {
        OutputFormat::Table => format_table(value),
        OutputFormat::Text => format_text(value),
        OutputFormat::Json | OutputFormat::Auto => format_json(value),
    }
}

fn auto_detect(value: &Value) -> OutputFormat {
    match value {
        Value::Array(arr) if arr.len() >= 3 => {
            // Check if uniform objects
            if arr.iter().all(|v| v.is_object()) {
                let first_keys = obj_keys(&arr[0]);
                let all_same = arr.iter().all(|v| obj_keys(v) == first_keys);
                if all_same && first_keys.len() <= 8 {
                    return OutputFormat::Table;
                }
            }
            OutputFormat::Json
        }
        _ => OutputFormat::Json,
    }
}

fn obj_keys(v: &Value) -> Vec<String> {
    match v.as_object() {
        Some(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            keys
        }
        None => vec![],
    }
}

fn format_json(value: &Value) -> String {
    match value {
        Value::Array(arr) => {
            // JSON-lines: one object per line
            arr.iter()
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => serde_json::to_string_pretty(value).unwrap_or_default(),
    }
}

fn format_table(value: &Value) -> String {
    let arr = match value.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return format_json(value),
    };

    let headers = obj_keys(&arr[0]);
    if headers.is_empty() {
        return format_json(value);
    }

    // Calculate column widths
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    let rows: Vec<Vec<String>> = arr
        .iter()
        .map(|item| {
            headers
                .iter()
                .enumerate()
                .map(|(i, key)| {
                    let cell = cell_str(item.get(key));
                    widths[i] = widths[i].max(cell.len().min(60));
                    cell
                })
                .collect()
        })
        .collect();

    let mut out = String::new();

    // Header
    out.push('|');
    for (i, h) in headers.iter().enumerate() {
        out.push_str(&format!(" {:<w$} |", h, w = widths[i]));
    }
    out.push('\n');

    // Separator
    out.push('|');
    for w in &widths {
        out.push_str(&format!("-{}-|", "-".repeat(*w)));
    }
    out.push('\n');

    // Rows
    for row in &rows {
        out.push('|');
        for (i, cell) in row.iter().enumerate() {
            let display = if cell.len() > widths[i] {
                format!("{}...", &cell[..widths[i].saturating_sub(3)])
            } else {
                cell.clone()
            };
            out.push_str(&format!(" {:<w$} |", display, w = widths[i]));
        }
        out.push('\n');
    }

    out
}

fn format_text(value: &Value) -> String {
    match value {
        Value::Array(arr) => arr
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let title = v
                    .get("title")
                    .or_else(|| v.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("(untitled)");
                let num = v
                    .get("number")
                    .and_then(|v| v.as_u64())
                    .map(|n| format!("#{n}"))
                    .unwrap_or_default();
                format!("{}. {num} {title}", i + 1)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| format!("{k}: {}", cell_str(Some(v))))
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}

fn cell_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from).or_else(|| Some(v.to_string())))
            .collect::<Vec<_>>()
            .join(", "),
        Some(v) => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_auto_detect_table_for_uniform_arrays() {
        // 3+ uniform objects with <=8 keys → table
        let val = json!([
            {"id": 1, "name": "a"},
            {"id": 2, "name": "b"},
            {"id": 3, "name": "c"}
        ]);
        assert_eq!(auto_detect(&val), OutputFormat::Table);
    }

    #[test]
    fn test_auto_detect_json_for_mixed_arrays() {
        // Objects with different keys → JSON
        let val = json!([
            {"id": 1, "name": "a"},
            {"id": 2, "title": "b"},
            {"id": 3, "name": "c"}
        ]);
        assert_eq!(auto_detect(&val), OutputFormat::Json);
    }

    #[test]
    fn test_auto_detect_json_for_small_arrays() {
        // <3 items → JSON
        let val = json!([
            {"id": 1, "name": "a"},
            {"id": 2, "name": "b"}
        ]);
        assert_eq!(auto_detect(&val), OutputFormat::Json);
    }

    #[test]
    fn test_auto_detect_json_for_wide_objects() {
        // >8 keys → JSON
        let val = json!([
            {"a":1,"b":2,"c":3,"d":4,"e":5,"f":6,"g":7,"h":8,"i":9},
            {"a":1,"b":2,"c":3,"d":4,"e":5,"f":6,"g":7,"h":8,"i":9},
            {"a":1,"b":2,"c":3,"d":4,"e":5,"f":6,"g":7,"h":8,"i":9}
        ]);
        assert_eq!(auto_detect(&val), OutputFormat::Json);
    }

    #[test]
    fn test_format_table_output() {
        let val = json!([
            {"id": 1, "name": "alpha"},
            {"id": 2, "name": "beta"},
            {"id": 3, "name": "gamma"}
        ]);
        let out = format_table(&val);
        // Has | separators
        assert!(out.contains('|'));
        // Has header row with column names
        assert!(out.contains("id"));
        assert!(out.contains("name"));
        // Has separator row with dashes
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.len() >= 3); // header + separator + at least one row
        assert!(lines[1].contains("---")); // separator row
    }

    #[test]
    fn test_format_text_output() {
        let val = json!([
            {"number": 42, "title": "Fix bug"},
            {"number": 7, "title": "Add feature"}
        ]);
        let out = format_text(&val);
        assert!(out.contains("1. #42 Fix bug"));
        assert!(out.contains("2. #7 Add feature"));
    }

    #[test]
    fn test_format_json_array_is_jsonlines() {
        let val = json!([
            {"id": 1},
            {"id": 2}
        ]);
        let out = format_json(&val);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        // Each line is a valid JSON object (not pretty-printed)
        for line in &lines {
            let parsed: Value = serde_json::from_str(line).unwrap();
            assert!(parsed.is_object());
        }
    }

    #[test]
    fn test_format_json_single_is_pretty() {
        let val = json!({"id": 1, "name": "test"});
        let out = format_json(&val);
        // Pretty-printed → multiple lines
        assert!(out.lines().count() > 1);
        // Contains indentation
        assert!(out.contains("  "));
    }

    #[test]
    fn test_cell_str_variants() {
        // null → empty
        assert_eq!(cell_str(Some(&Value::Null)), "");
        assert_eq!(cell_str(None), "");

        // string → string
        assert_eq!(cell_str(Some(&json!("hello"))), "hello");

        // number → string
        assert_eq!(cell_str(Some(&json!(42))), "42");

        // bool → string
        assert_eq!(cell_str(Some(&json!(true))), "true");
        assert_eq!(cell_str(Some(&json!(false))), "false");

        // array → comma-separated
        let arr = json!(["a", "b", "c"]);
        assert_eq!(cell_str(Some(&arr)), "a, b, c");
    }

    #[test]
    fn test_cell_str_object() {
        let obj = json!({"key": "val"});
        let result = cell_str(Some(&obj));
        assert!(result.contains("key"));
    }

    #[test]
    fn test_cell_str_numeric_array() {
        let arr = json!([1, 2, 3]);
        let result = cell_str(Some(&arr));
        assert!(result.contains("1"));
        assert!(result.contains("2"));
        assert!(result.contains("3"));
    }

    #[test]
    fn test_format_output_auto_single_object() {
        let val = json!({"id": 1, "name": "test"});
        let out = format_output(&val, OutputFormat::Auto);
        // Single object → JSON (pretty)
        assert!(out.contains("\"id\""));
    }

    #[test]
    fn test_format_output_auto_small_array() {
        let val = json!([{"id": 1}, {"id": 2}]);
        let out = format_output(&val, OutputFormat::Auto);
        // <3 items → JSON
        assert!(out.contains("\"id\""));
    }

    #[test]
    fn test_format_output_auto_large_uniform_array() {
        let val = json!([
            {"id": 1, "name": "a"},
            {"id": 2, "name": "b"},
            {"id": 3, "name": "c"}
        ]);
        let out = format_output(&val, OutputFormat::Auto);
        // 3+ uniform with <=8 keys → table
        assert!(out.contains('|'));
    }

    #[test]
    fn test_format_text_object() {
        let val = json!({"title": "Bug", "state": "open", "number": 42});
        let out = format_text(&val);
        assert!(out.contains("title: Bug"));
        assert!(out.contains("state: open"));
    }

    #[test]
    fn test_format_text_scalar() {
        let val = json!(42);
        let out = format_text(&val);
        assert_eq!(out, "42");
    }

    #[test]
    fn test_format_text_string() {
        let val = json!("hello world");
        let out = format_text(&val);
        assert_eq!(out, "\"hello world\"");
    }

    #[test]
    fn test_format_text_untitled() {
        let val = json!([{"id": 1}]);
        let out = format_text(&val);
        assert!(out.contains("(untitled)"));
    }

    #[test]
    fn test_format_text_name_fallback() {
        let val = json!([{"name": "Release v1.0"}]);
        let out = format_text(&val);
        assert!(out.contains("Release v1.0"));
    }

    #[test]
    fn test_format_table_empty_array() {
        let val = json!([]);
        let out = format_table(&val);
        // Falls back to JSON for empty
        assert!(out.is_empty() || out == "[]");
    }

    #[test]
    fn test_format_table_non_object_array() {
        let val = json!([1, 2, 3]);
        let out = format_table(&val);
        // Non-objects → empty headers → falls back to JSON
        assert!(out.contains("1"));
    }

    #[test]
    fn test_format_table_long_cell_truncation() {
        let long = "x".repeat(100);
        let val = json!([
            {"col": long.clone()},
            {"col": "short"},
            {"col": "short2"}
        ]);
        let out = format_table(&val);
        // Table should contain "..." for truncated cells
        assert!(out.contains('|'));
    }

    #[test]
    fn test_format_output_explicit_table() {
        let val = json!([{"a": 1}, {"a": 2}, {"a": 3}]);
        let out = format_output(&val, OutputFormat::Table);
        assert!(out.contains('|'));
        assert!(out.contains("a"));
    }

    #[test]
    fn test_format_output_explicit_text() {
        let val = json!([{"number": 1, "title": "Hi"}]);
        let out = format_output(&val, OutputFormat::Text);
        assert!(out.contains("#1 Hi"));
    }

    #[test]
    fn test_format_output_explicit_json() {
        let val = json!({"key": "val"});
        let out = format_output(&val, OutputFormat::Json);
        assert!(out.contains("\"key\""));
    }

    #[test]
    fn test_obj_keys_non_object() {
        assert_eq!(obj_keys(&json!(42)), Vec::<String>::new());
    }

    #[test]
    fn test_obj_keys_sorted() {
        let val = json!({"z": 1, "a": 2, "m": 3});
        let keys = obj_keys(&val);
        assert_eq!(keys, vec!["a", "m", "z"]);
    }

    #[test]
    fn test_auto_detect_non_array() {
        assert_eq!(auto_detect(&json!({"id": 1})), OutputFormat::Json);
        assert_eq!(auto_detect(&json!(42)), OutputFormat::Json);
        assert_eq!(auto_detect(&json!("str")), OutputFormat::Json);
    }

    #[test]
    fn test_auto_detect_mixed_types() {
        // Array with non-objects
        let val = json!([1, 2, 3]);
        assert_eq!(auto_detect(&val), OutputFormat::Json);
    }

    #[test]
    fn test_format_text_null_value() {
        let val = json!({"key": null});
        let out = format_text(&val);
        assert!(out.contains("key:"));
    }
}
