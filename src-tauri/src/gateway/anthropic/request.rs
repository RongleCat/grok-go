//! Anthropic Messages request → OpenAI Chat Completions request.

use super::schema::{is_batch_tool, normalize_schema};
use serde_json::{json, Value};

/// Result of converting a Claude Code / Anthropic body for xAI chat/completions.
#[derive(Debug, Clone)]
pub struct ConvertedChatRequest {
    pub body: Value,
    /// Original Anthropic model string from the client.
    pub requested_model: String,
    pub stream: bool,
}

/// Map Claude Code model names (haiku/sonnet/opus) before `resolve_model`.
///
/// Returns `(candidate_model, mapping_hint)`.
///
/// O-10: do **not** collapse Claude shell names to `default_model` here — that
/// skips tier mapping in [`crate::config::resolve_model`] /
/// [`crate::config::map_claude_shell_model`]. Return a stable Grok candidate
/// (haiku → non-reasoning; sonnet/opus → grok-4.5) so resolve_model can still
/// honor exact `model_mappings` keys when present.
pub fn map_client_model(requested: &str, default_model: &str) -> (String, String) {
    let trimmed = requested.trim();
    if trimmed.is_empty() {
        return (default_model.to_string(), "anthropic-empty-default".into());
    }
    let lower = trimmed.to_lowercase();
    // Already a grok / explicit xAI id — leave for resolve_model.
    if lower.starts_with("grok") || lower.contains("imagine") {
        return (trimmed.to_string(), "anthropic-passthrough".into());
    }
    // Claude family: tier map (must not use default_model for all shells).
    if let Some((model, reason)) = crate::config::map_claude_shell_model(trimmed) {
        return (model.to_string(), reason.into());
    }
    if lower.contains("claude") {
        // Generic claude-* without haiku/sonnet/opus → default text model.
        return (default_model.to_string(), "anthropic-claude-generic".into());
    }
    (trimmed.to_string(), "anthropic-passthrough".into())
}

/// Convert Anthropic `/v1/messages` JSON to OpenAI chat completions JSON.
pub fn anthropic_to_openai_chat(body: &Value) -> Result<ConvertedChatRequest, String> {
    let requested_model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    if requested_model.is_empty() {
        return Err("missing required field: model".into());
    }

    let stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut messages: Vec<Value> = Vec::new();

    // system → role=system (string or content blocks)
    if let Some(system) = body.get("system") {
        if let Some(text) = flatten_system(system) {
            if !text.is_empty() {
                messages.push(json!({"role": "system", "content": text}));
            }
        }
    }

    let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) else {
        return Err("missing required field: messages".into());
    };

    for msg in msgs {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("user");
        let content = msg.get("content");
        messages.extend(convert_message(role, content)?);
    }

    if messages.is_empty() {
        return Err("messages produced empty OpenAI history".into());
    }

    let mut out = json!({
        "model": requested_model,
        "messages": messages,
    });

    // max_tokens is required by Anthropic; OpenAI/xAI accept it optionally.
    if let Some(v) = body.get("max_tokens") {
        out["max_tokens"] = v.clone();
    } else {
        // Claude Code always sends max_tokens; default defensively.
        out["max_tokens"] = json!(4096);
    }
    if let Some(v) = body.get("temperature") {
        out["temperature"] = v.clone();
    }
    if let Some(v) = body.get("top_p") {
        out["top_p"] = v.clone();
    }
    if let Some(v) = body.get("stop_sequences") {
        out["stop"] = v.clone();
    }
    if stream {
        out["stream"] = json!(true);
        // Prefer usage on the final SSE chunk when the upstream supports it.
        out["stream_options"] = json!({"include_usage": true});
    }

    // tools
    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        let openai_tools: Vec<Value> = tools
            .iter()
            .filter(|t| !is_batch_tool(t))
            .map(|t| {
                let name = t.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let description = t.get("description").cloned().unwrap_or(Value::Null);
                let parameters = normalize_schema(
                    t.get("input_schema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                );
                json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": parameters
                    }
                })
            })
            .filter(|t| {
                t.pointer("/function/name")
                    .and_then(|n| n.as_str())
                    .map(|n| !n.is_empty())
                    .unwrap_or(false)
            })
            .collect();
        if !openai_tools.is_empty() {
            out["tools"] = json!(openai_tools);
        }
    }

    // tool_choice — must map object form; never pass Anthropic shape upstream.
    if let Some(tc) = body.get("tool_choice") {
        if let Some(mapped) = map_tool_choice(tc) {
            out["tool_choice"] = mapped;
        }
        // disable_parallel_tool_use (Anthropic) → parallel_tool_calls (OpenAI)
        if tc
            .get("disable_parallel_tool_use")
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            out["parallel_tool_calls"] = json!(false);
        }
    }

    // Strip Anthropic-only fields (thinking, metadata, cache, betas…) — already not copied.

    Ok(ConvertedChatRequest {
        body: out,
        requested_model,
        stream,
    })
}

fn flatten_system(system: &Value) -> Option<String> {
    match system {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => {
            let parts: Vec<&str> = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

fn convert_message(role: &str, content: Option<&Value>) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    let content = match content {
        Some(c) => c,
        None => {
            result.push(json!({"role": role, "content": ""}));
            return Ok(result);
        }
    };

    if let Some(text) = content.as_str() {
        result.push(json!({"role": role, "content": text}));
        return Ok(result);
    }

    let Some(blocks) = content.as_array() else {
        result.push(json!({"role": role, "content": content}));
        return Ok(result);
    };

    let mut content_parts: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();

    for block in blocks {
        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    content_parts.push(json!({"type": "text", "text": text}));
                }
            }
            "image" => {
                if let Some(source) = block.get("source") {
                    let media_type = source
                        .get("media_type")
                        .and_then(|m| m.as_str())
                        .unwrap_or("image/png");
                    let data = source.get("data").and_then(|d| d.as_str()).unwrap_or("");
                    if !data.is_empty() {
                        content_parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{media_type};base64,{data}")
                            }
                        }));
                    }
                }
            }
            "tool_use" => {
                let id = block
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() || name.is_empty() {
                    return Err("tool_use block requires id and name".into());
                }
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                let arguments = serde_json::to_string(&input).unwrap_or_else(|_| "{}".into());
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments
                    }
                }));
            }
            "tool_result" => {
                let tool_use_id = block
                    .get("tool_use_id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("");
                if tool_use_id.is_empty() {
                    return Err("tool_result block requires tool_use_id".into());
                }
                let content_str = flatten_tool_result_content(block.get("content"));
                result.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": content_str
                }));
            }
            "thinking" | "redacted_thinking" => {
                // Drop — xAI chat path has no Anthropic thinking wire format.
            }
            "document" => {
                // O-17: high-risk silent drop → explicit error
                return Err(
                    "document content blocks are not supported on GrokGo Anthropic path; extract text client-side or attach as image/text"
                        .into(),
                );
            }
            "" => {
                // cache_control-only or typeless — ignore
            }
            other => {
                // O-17: log unknown blocks (do not silently invent support)
                tracing::warn!(
                    target: "gateway",
                    block_type = other,
                    "unknown Anthropic content block dropped"
                );
            }
        }
    }

    if !content_parts.is_empty() || !tool_calls.is_empty() {
        let mut msg = json!({"role": role});
        if content_parts.is_empty() {
            msg["content"] = Value::Null;
        } else if content_parts.len() == 1 {
            if let Some(text) = content_parts[0].get("text") {
                msg["content"] = text.clone();
            } else {
                msg["content"] = json!(content_parts);
            }
        } else {
            msg["content"] = json!(content_parts);
        }
        if !tool_calls.is_empty() {
            msg["tool_calls"] = json!(tool_calls);
        }
        result.push(msg);
    }

    Ok(result)
}

fn flatten_tool_result_content(content: Option<&Value>) -> String {
    match content {
        None => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| {
                if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                    Some(t.to_string())
                } else if b.is_string() {
                    b.as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn map_tool_choice(tc: &Value) -> Option<Value> {
    // String form (rare)
    if let Some(s) = tc.as_str() {
        return Some(json!(s));
    }
    let ty = tc.get("type").and_then(|t| t.as_str())?;
    match ty {
        "auto" => Some(json!("auto")),
        "any" => Some(json!("required")),
        "none" => Some(json!("none")),
        "tool" => {
            let name = tc.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name.is_empty() {
                return Some(json!("auto"));
            }
            Some(json!({
                "type": "function",
                "function": { "name": name }
            }))
        }
        _ => None,
    }
}

/// Rough token estimate for `/v1/messages/count_tokens` (Claude Code preflight).
pub fn estimate_token_count(body: &Value) -> u64 {
    let mut chars = 0usize;
    if let Some(s) = body.get("system") {
        chars += estimate_value_chars(s);
    }
    if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
        for m in msgs {
            chars += estimate_value_chars(m);
        }
    }
    if let Some(tools) = body.get("tools") {
        chars += estimate_value_chars(tools);
    }
    // ~4 chars per token heuristic + small fixed overhead.
    ((chars / 4) as u64).saturating_add(8)
}

fn estimate_value_chars(v: &Value) -> usize {
    match v {
        Value::String(s) => s.len(),
        Value::Array(a) => a.iter().map(estimate_value_chars).sum(),
        Value::Object(o) => o.values().map(estimate_value_chars).sum(),
        Value::Number(n) => n.to_string().len(),
        Value::Bool(_) | Value::Null => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_user_message() {
        let body = json!({
            "model": "claude-sonnet-4",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let conv = anthropic_to_openai_chat(&body).unwrap();
        assert_eq!(conv.body["messages"][0]["role"], "user");
        assert_eq!(conv.body["messages"][0]["content"], "Hello");
        assert_eq!(conv.body["max_tokens"], 1024);
        assert!(!conv.stream);
    }

    #[test]
    fn system_and_tools_and_tool_choice() {
        let body = json!({
            "model": "claude-3-opus",
            "max_tokens": 512,
            "system": [{"type": "text", "text": "Be concise"}],
            "messages": [{"role": "user", "content": "weather?"}],
            "tools": [{
                "name": "get_weather",
                "description": "Weather",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "city": {"type": "string", "format": "uri"}
                    }
                }
            }],
            "tool_choice": {"type": "any", "disable_parallel_tool_use": true}
        });
        let conv = anthropic_to_openai_chat(&body).unwrap();
        assert_eq!(conv.body["messages"][0]["role"], "system");
        assert_eq!(conv.body["messages"][0]["content"], "Be concise");
        assert_eq!(conv.body["tools"][0]["type"], "function");
        assert_eq!(conv.body["tools"][0]["function"]["name"], "get_weather");
        assert!(conv.body["tools"][0]["function"]["parameters"]["properties"]["city"]
            .get("format")
            .is_none());
        assert_eq!(conv.body["tool_choice"], "required");
        assert_eq!(conv.body["parallel_tool_calls"], false);
    }

    #[test]
    fn tool_use_and_tool_result_roundtrip_shape() {
        let body = json!({
            "model": "grok-4.5",
            "max_tokens": 256,
            "messages": [
                {
                    "role": "assistant",
                    "content": [
                        {"type": "text", "text": "checking"},
                        {
                            "type": "tool_use",
                            "id": "call_abc",
                            "name": "Bash",
                            "input": {"command": "ls"}
                        }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_abc",
                            "content": [{"type": "text", "text": "a.txt"}]
                        }
                    ]
                }
            ]
        });
        let conv = anthropic_to_openai_chat(&body).unwrap();
        let msgs = conv.body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "assistant");
        assert_eq!(msgs[0]["tool_calls"][0]["id"], "call_abc");
        assert_eq!(msgs[0]["tool_calls"][0]["function"]["name"], "Bash");
        let args: Value =
            serde_json::from_str(msgs[0]["tool_calls"][0]["function"]["arguments"].as_str().unwrap())
                .unwrap();
        assert_eq!(args["command"], "ls");
        assert_eq!(msgs[1]["role"], "tool");
        assert_eq!(msgs[1]["tool_call_id"], "call_abc");
        assert_eq!(msgs[1]["content"], "a.txt");
    }

    /// R2-12 / D-002: multi-segment tool_result content must flatten to one tool message.
    #[test]
    fn tool_result_multi_content_segments_flatten() {
        let body = json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 64,
            "messages": [
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "call_1",
                        "name": "Read",
                        "input": {"path": "a"}
                    }]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "call_1",
                        "content": [
                            {"type": "text", "text": "line-a"},
                            {"type": "text", "text": "line-b"}
                        ]
                    }]
                }
            ]
        });
        let conv = anthropic_to_openai_chat(&body).unwrap();
        let msgs = conv.body["messages"].as_array().unwrap();
        let tool_msg = msgs.iter().find(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool")).unwrap();
        let content = tool_msg.get("content").and_then(|c| c.as_str()).unwrap();
        assert!(content.contains("line-a"));
        assert!(content.contains("line-b"));
    }

    #[test]
    fn document_block_is_explicit_error() {
        let body = json!({
            "model": "m",
            "max_tokens": 8,
            "messages": [{
                "role": "user",
                "content": [{"type": "document", "source": {"type": "base64", "data": "xx"}}]
            }]
        });
        let err = anthropic_to_openai_chat(&body).unwrap_err();
        assert!(err.to_ascii_lowercase().contains("document"));
    }

    #[test]
    fn filters_batch_tool() {
        let body = json!({
            "model": "x",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [
                {"type": "BatchTool", "name": "BatchTool", "input_schema": {}},
                {"name": "Read", "input_schema": {"type": "object"}}
            ]
        });
        let conv = anthropic_to_openai_chat(&body).unwrap();
        let tools = conv.body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["function"]["name"], "Read");
    }

    #[test]
    fn map_client_model_claude_alias() {
        let (m, reason) = map_client_model("claude-sonnet-4-20250514", "grok-4.5");
        assert_eq!(m, "grok-4.5");
        assert_eq!(reason, "claude-tier-sonnet");
        let (m2, _) = map_client_model("grok-4.3", "grok-4.5");
        assert_eq!(m2, "grok-4.3");
    }

    #[test]
    fn map_client_model_haiku_is_non_reasoning_tier() {
        let (m, reason) = map_client_model("claude-haiku-4-5-20251001", "grok-4.5");
        assert_eq!(m, "grok-4.20-0309-non-reasoning");
        assert_eq!(reason, "claude-tier-haiku");
        // Must not collapse to the caller's default_model.
        assert_ne!(m, "grok-4.5");
        let (opus, r) = map_client_model("claude-opus-4-6", "grok-4.5");
        assert_eq!(opus, "grok-4.5");
        assert_eq!(r, "claude-tier-opus");
    }

    #[test]
    fn tool_choice_named_tool() {
        let body = json!({
            "model": "m",
            "max_tokens": 10,
            "messages": [{"role": "user", "content": "x"}],
            "tool_choice": {"type": "tool", "name": "Read"}
        });
        let conv = anthropic_to_openai_chat(&body).unwrap();
        assert_eq!(conv.body["tool_choice"]["type"], "function");
        assert_eq!(conv.body["tool_choice"]["function"]["name"], "Read");
    }
}
