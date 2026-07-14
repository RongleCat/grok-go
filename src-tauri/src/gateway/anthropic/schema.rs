//! JSON Schema cleanup for OpenAI / xAI tool `parameters`.
//!
//! Adapted from anthropic-proxy-rs `normalize_schema` (MIT) — Claude Code often
//! emits schemas with `format: "uri"`, null defaults, or missing `required` that
//! some OpenAI-compatible backends reject.

use serde_json::Value;

/// Normalize a tool `input_schema` into a safer OpenAI `parameters` object.
pub fn normalize_schema(schema: Value) -> Value {
    match schema {
        Value::Object(mut obj) => {
            obj.retain(|_, value| !value.is_null());

            if obj.get("format").and_then(|v| v.as_str()) == Some("uri") {
                obj.remove("format");
            }

            if let Some(properties) = obj.get_mut("properties").and_then(|v| v.as_object_mut()) {
                let keys: Vec<String> = properties.keys().cloned().collect();
                for key in keys {
                    match properties.get(&key).cloned() {
                        Some(Value::Null) | None => {
                            properties.remove(&key);
                        }
                        Some(value) => {
                            properties.insert(key, normalize_schema(value));
                        }
                    }
                }
            }

            for key in [
                "items",
                "additionalProperties",
                "contains",
                "not",
                "if",
                "then",
                "else",
            ] {
                if let Some(value) = obj.get_mut(key) {
                    *value = normalize_schema(value.clone());
                }
            }

            for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
                if let Some(values) = obj.get_mut(key).and_then(|v| v.as_array_mut()) {
                    for value in values.iter_mut() {
                        *value = normalize_schema(value.clone());
                    }
                }
            }

            if obj.get("type").and_then(|v| v.as_str()) == Some("object")
                && !obj.contains_key("required")
            {
                obj.insert("required".into(), Value::Array(Vec::new()));
            }

            if let Some(required) = obj.get_mut("required") {
                if !required.is_array() {
                    *required = Value::Array(Vec::new());
                }
            }

            Value::Object(obj)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(normalize_schema).collect()),
        other => other,
    }
}

/// Claude Code occasionally registers an internal BatchTool that OpenAI backends reject.
pub fn is_batch_tool(tool: &Value) -> bool {
    tool.get("type").and_then(|v| v.as_str()) == Some("BatchTool")
        || tool.get("name").and_then(|v| v.as_str()) == Some("BatchTool")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_uri_format_and_nulls() {
        let schema = json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "format": "uri" },
                "gone": null
            }
        });
        let out = normalize_schema(schema);
        assert!(out["properties"].get("gone").is_none());
        assert!(out["properties"]["url"].get("format").is_none());
        assert_eq!(out["required"], json!([]));
    }

    #[test]
    fn detects_batch_tool() {
        assert!(is_batch_tool(&json!({"type": "BatchTool", "name": "x"})));
        assert!(!is_batch_tool(&json!({"name": "Bash", "input_schema": {}})));
    }
}
