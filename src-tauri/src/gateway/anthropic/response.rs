//! OpenAI Chat Completions response → Anthropic Messages response.

use serde_json::{json, Value};

/// How reasoning is exposed on the Anthropic Messages path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingMode {
    /// Do not emit thinking blocks (default — better TTFT for Claude Code).
    #[default]
    Hide,
    /// Forward full reasoning as thinking blocks.
    Passthrough,
    /// Emit a short summary thinking block only.
    Summary,
}

impl ThinkingMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "passthrough" | "pass" | "full" => Self::Passthrough,
            "summary" => Self::Summary,
            _ => Self::Hide,
        }
    }
}

/// Convert a non-streaming OpenAI chat completion JSON body to Anthropic Messages.
pub fn openai_chat_to_anthropic(body: &Value) -> Result<Value, String> {
    openai_chat_to_anthropic_with_thinking(body, ThinkingMode::Hide)
}

/// Same as [`openai_chat_to_anthropic`] with explicit thinking mode.
pub fn openai_chat_to_anthropic_with_thinking(
    body: &Value,
    thinking: ThinkingMode,
) -> Result<Value, String> {
    // Pass through Anthropic-shaped errors if we already rewrote them.
    if body.get("type").and_then(|t| t.as_str()) == Some("error") {
        return Ok(body.clone());
    }

    let choices = body
        .get("choices")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "upstream response missing choices".to_string())?;
    let choice = choices
        .first()
        .ok_or_else(|| "upstream response has empty choices".to_string())?;
    let message = choice
        .get("message")
        .ok_or_else(|| "upstream choice missing message".to_string())?;

    let mut content: Vec<Value> = Vec::new();

    // Optional reasoning → thinking block (controlled by ThinkingMode).
    if thinking != ThinkingMode::Hide {
        if let Some(reasoning) = message
            .get("reasoning_content")
            .or_else(|| message.get("reasoning"))
            .and_then(|r| r.as_str())
        {
            if !reasoning.is_empty() {
                let text = match thinking {
                    ThinkingMode::Summary => {
                        let t = reasoning.trim();
                        if t.chars().count() > 240 {
                            format!("{}…", t.chars().take(240).collect::<String>())
                        } else {
                            t.to_string()
                        }
                    }
                    _ => reasoning.to_string(),
                };
                content.push(json!({
                    "type": "thinking",
                    "thinking": text
                }));
            }
        }
    }

    if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            content.push(json!({"type": "text", "text": text}));
        }
    } else if let Some(parts) = message.get("content").and_then(|c| c.as_array()) {
        for part in parts {
            if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    if !text.is_empty() {
                        content.push(json!({"type": "text", "text": text}));
                    }
                }
            }
        }
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
        for tc in tool_calls {
            let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let func = tc.get("function").cloned().unwrap_or(json!({}));
            let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args_str = func
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
            if id.is_empty() || name.is_empty() {
                continue;
            }
            content.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
    }

    // Empty content is invalid for Anthropic — emit empty text block.
    if content.is_empty() {
        content.push(json!({"type": "text", "text": ""}));
    }

    let stop_reason = map_stop_reason(
        choice
            .get("finish_reason")
            .and_then(|r| r.as_str()),
        message.get("tool_calls").and_then(|t| t.as_array()).map(|a| !a.is_empty()).unwrap_or(false),
    );

    let usage = body.get("usage").cloned().unwrap_or(json!({}));
    let input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_read = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.get("cache_read_input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let mut usage_out = json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens
    });
    if cache_read > 0 {
        usage_out["cache_read_input_tokens"] = json!(cache_read);
    }

    let id = body
        .get("id")
        .and_then(|i| i.as_str())
        .map(|s| {
            if s.starts_with("msg_") {
                s.to_string()
            } else {
                format!("msg_{s}")
            }
        })
        .unwrap_or_else(|| format!("msg_{}", uuid_like()));

    Ok(json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": body.get("model").and_then(|m| m.as_str()).unwrap_or(""),
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": usage_out
    }))
}

/// Map OpenAI `finish_reason` → Anthropic `stop_reason`.
///
/// If the model returned tool_calls but finish_reason is missing/wrong, force `tool_use`
/// so Claude Code actually runs tools.
pub fn map_stop_reason(finish_reason: Option<&str>, has_tool_calls: bool) -> &'static str {
    match finish_reason {
        Some("tool_calls") | Some("function_call") => "tool_use",
        Some("length") => "max_tokens",
        Some("content_filter") => "refusal",
        Some("stop") | Some("end_turn") => {
            if has_tool_calls {
                "tool_use"
            } else {
                "end_turn"
            }
        }
        Some("max_tokens") => "max_tokens",
        _ => {
            if has_tool_calls {
                "tool_use"
            } else {
                "end_turn"
            }
        }
    }
}

/// Anthropic-style error envelope (Claude Code parses `error.type`).
pub fn anthropic_error_body(error_type: &str, message: impl AsRef<str>) -> Value {
    json!({
        "type": "error",
        "error": {
            "type": error_type,
            "message": message.as_ref()
        }
    })
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

/// Convert OpenAI / xAI error JSON into Anthropic error shape when possible.
///
/// xAI often returns `{"code":"invalid-argument","error":"<string>"}` (error is a
/// **string**, not `{message}`). Older extractors only read `/error/message`, so
/// Claude Code only saw the useless "upstream error".
pub fn openai_error_to_anthropic(status: u16, body: &Value) -> Value {
    let message = extract_upstream_error_message(body);
    let error_type = match status {
        401 | 403 => "authentication_error",
        429 => "rate_limit_error",
        400 => "invalid_request_error",
        404 => "not_found_error",
        529 | 503 => "overloaded_error",
        _ => "api_error",
    };
    anthropic_error_body(error_type, message)
}

/// Pull a human-readable message from OpenAI-shaped or xAI-shaped error bodies.
pub fn extract_upstream_error_message(body: &Value) -> String {
    // OpenAI: { "error": { "message": "...", "type": "..." } }
    if let Some(m) = body.pointer("/error/message").and_then(|m| m.as_str()) {
        if !m.trim().is_empty() {
            return m.to_string();
        }
    }
    // xAI: { "code": "invalid-argument", "error": "..." }  (error is a string)
    if let Some(m) = body.get("error").and_then(|e| e.as_str()) {
        if !m.trim().is_empty() {
            let code = body.get("code").and_then(|c| c.as_str()).unwrap_or("");
            if code.is_empty() {
                return m.to_string();
            }
            return format!("{code}: {m}");
        }
    }
    // Nested string under error.error
    if let Some(m) = body.pointer("/error/error").and_then(|m| m.as_str()) {
        if !m.trim().is_empty() {
            return m.to_string();
        }
    }
    if let Some(m) = body.get("message").and_then(|m| m.as_str()) {
        if !m.trim().is_empty() {
            return m.to_string();
        }
    }
    // Last resort: compact JSON so logs/UI still show something actionable.
    let compact = body.to_string();
    if compact.len() > 8 && compact != "null" && compact != "{}" {
        if compact.len() > 800 {
            return format!("{}…", &compact[..800]);
        }
        return compact;
    }
    "upstream error".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_response() {
        let body = json!({
            "id": "chatcmpl-1",
            "model": "grok-4.5",
            "choices": [{
                "message": {"role": "assistant", "content": "Hi"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1}
        });
        let out = openai_chat_to_anthropic(&body).unwrap();
        assert_eq!(out["type"], "message");
        assert_eq!(out["content"][0]["text"], "Hi");
        assert_eq!(out["stop_reason"], "end_turn");
        assert_eq!(out["usage"]["input_tokens"], 3);
        assert!(out["id"].as_str().unwrap().starts_with("msg_"));
    }

    #[test]
    fn hide_thinking_omits_reasoning_blocks() {
        let body = json!({
            "id": "chatcmpl-2",
            "model": "grok-4.5",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "done",
                    "reasoning_content": "long chain of thought"
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        });
        let hidden = openai_chat_to_anthropic_with_thinking(&body, ThinkingMode::Hide).unwrap();
        assert!(hidden["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b.get("type").and_then(|t| t.as_str()) != Some("thinking")));
        let pass = openai_chat_to_anthropic_with_thinking(&body, ThinkingMode::Passthrough).unwrap();
        assert!(pass["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("thinking")));
    }

    #[test]
    fn xai_string_error_is_surfaced() {
        let body = json!({
            "code": "invalid-argument",
            "error": "Invalid tool call arguments: unexpected end of JSON"
        });
        let out = openai_error_to_anthropic(400, &body);
        assert_eq!(out["type"], "error");
        assert_eq!(out["error"]["type"], "invalid_request_error");
        let msg = out["error"]["message"].as_str().unwrap();
        assert!(msg.contains("Invalid tool call"));
        assert!(msg.contains("invalid-argument"));
        assert!(!msg.contains("upstream error") || msg.len() > 20);
    }

    #[test]
    fn openai_object_error_is_surfaced() {
        let body = json!({
            "error": {"message": "context_length_exceeded", "type": "invalid_request_error"}
        });
        let out = openai_error_to_anthropic(400, &body);
        assert_eq!(out["error"]["message"], "context_length_exceeded");
    }

    #[test]
    fn tool_calls_response() {
        let body = json!({
            "id": "chatcmpl-2",
            "model": "grok-4.5",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "Read",
                            "arguments": "{\"path\":\"a.rs\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        let out = openai_chat_to_anthropic(&body).unwrap();
        assert_eq!(out["stop_reason"], "tool_use");
        assert_eq!(out["content"][0]["type"], "tool_use");
        assert_eq!(out["content"][0]["id"], "call_1");
        assert_eq!(out["content"][0]["name"], "Read");
        assert_eq!(out["content"][0]["input"]["path"], "a.rs");
    }

    #[test]
    fn forces_tool_use_when_finish_reason_wrong() {
        assert_eq!(map_stop_reason(Some("stop"), true), "tool_use");
        assert_eq!(map_stop_reason(None, true), "tool_use");
    }
}
