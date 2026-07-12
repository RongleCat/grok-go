//! Transform Codex/OpenAI Responses payloads into xAI-compatible shapes.
//!
//! Codex emits OpenAI-only tool types such as `custom` (free-form tools like
//! `apply_patch`). xAI only accepts:
//! `function | web_search | x_search | collections_search | file_search |
//! code_execution | code_interpreter | mcp | shell`.
//!
//! We convert `custom` ↔ `function` on the way out and rewrite matching
//! `function_call` items back to `custom_tool_call` on the way in so Codex
//! keeps working.

use serde_json::{json, Map, Value};
use std::collections::HashSet;

const XAI_TOOL_TYPES: &[&str] = &[
    "function",
    "web_search",
    "x_search",
    "collections_search",
    "file_search",
    "code_execution",
    "code_interpreter",
    "mcp",
    "shell",
];

#[derive(Debug, Default, Clone)]
pub struct SanitizeResult {
    /// Tool names that were originally `type: "custom"` and need response rewrite.
    pub custom_tool_names: HashSet<String>,
    /// True when the request JSON was mutated (only then should we re-serialize).
    pub modified: bool,
    /// Request includes image_gen / image_generation tools (proxy will fulfill server-side).
    pub has_image_gen_tools: bool,
}

/// Sanitize a Responses API request body before forwarding to xAI.
pub fn sanitize_responses_request(value: &mut Value) -> SanitizeResult {
    let mut result = SanitizeResult::default();

    // xAI does not honor OpenAI previous_response_id store semantics via this proxy.
    if let Some(obj) = value.as_object_mut() {
        for key in [
            "previous_response_id",
            "context_management",
            "prompt_cache_retention",
            "safety_identifier",
            "stream_options",
        ] {
            if obj.remove(key).is_some() {
                result.modified = true;
            }
        }
    }

    if let Some(tools) = value.get_mut("tools").and_then(|t| t.as_array_mut()) {
        let original = tools.clone();
        result.has_image_gen_tools = crate::gateway::image_bridge::request_has_image_tools(&Value::Array(original.clone()));
        let mut next = Vec::with_capacity(tools.len());
        for tool in tools.iter() {
            if let Some(converted) = convert_tool(tool, &mut result.custom_tool_names) {
                next.push(converted);
            }
        }
        // Deduplicate image_gen if both image_generation + image_gen appeared.
        let mut seen_image = false;
        next.retain(|t| {
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name == "image_gen" {
                if seen_image {
                    return false;
                }
                seen_image = true;
            }
            true
        });
        if next != original {
            result.modified = true;
        }
        if seen_image {
            result.has_image_gen_tools = true;
            result.custom_tool_names.insert("image_gen".into());
        }
        *tools = next;
    }

    // Ensure tools array exists for Codex custom-provider sessions (they omit built-ins).
    {
        let needs_tools = value
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true);
        if needs_tools {
            if let Some(obj) = value.as_object_mut() {
                obj.insert("tools".into(), json!([]));
                result.modified = true;
            }
        }
    }

    // Ensure X / web search + image_gen when tools are present (Codex agent loops).
    // Custom providers never get official built-in `image_gen`; we inject a function tool
    // and fulfill it server-side with Grok Imagine.
    if let Some(tools) = value.get_mut("tools").and_then(|t| t.as_array_mut()) {
        let has_x = tools.iter().any(|t| {
            matches!(
                t.get("type").and_then(|v| v.as_str()),
                Some("x_search" | "web_search")
            )
        });
        if !has_x {
            tools.push(json!({"type": "x_search"}));
            tools.push(json!({"type": "web_search"}));
            result.modified = true;
        }

        let has_image = tools.iter().any(|t| {
            let ty = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
            matches!(ty, "image_generation" | "image_gen")
                || crate::gateway::image_bridge::is_image_gen_name(name)
        });
        if !has_image {
            tools.push(crate::gateway::image_bridge::image_gen_function_tool());
            result.has_image_gen_tools = true;
            result.custom_tool_names.insert("image_gen".into());
            result.modified = true;
        } else {
            result.has_image_gen_tools = true;
            result.custom_tool_names.insert("image_gen".into());
        }
    }

    // Drop empty tools + orphan tool_choice (xAI 400 if tool_choice without tools).
    let tools_empty = value
        .get("tools")
        .map(|t| t.as_array().map(|a| a.is_empty()).unwrap_or(true))
        .unwrap_or(true);
    if tools_empty {
        if let Some(obj) = value.as_object_mut() {
            if obj.remove("tools").is_some() {
                result.modified = true;
            }
            if obj.remove("tool_choice").is_some() {
                result.modified = true;
            }
            if obj.remove("parallel_tool_calls").is_some() {
                result.modified = true;
            }
        }
    } else if let Some(choice) = value.get_mut("tool_choice") {
        let before = choice.clone();
        rewrite_tool_choice(choice, &result.custom_tool_names);
        if *choice != before {
            result.modified = true;
        }
    }

    // Normalize `input` for xAI ModelInput untagged enum.
    match value.get_mut("input") {
        Some(Value::Array(items)) => {
            let before = items.clone();
            *items = normalize_input_items(std::mem::take(items), &mut result.custom_tool_names);
            if *items != before {
                result.modified = true;
            }
        }
        Some(item) if item.is_object() => {
            let mut arr = vec![item.clone()];
            arr = normalize_input_items(arr, &mut result.custom_tool_names);
            if arr.len() == 1 {
                if arr[0] != *item {
                    *item = arr.remove(0);
                    result.modified = true;
                }
            } else {
                // became multi-item or empty — replace parent handled below via full rewrite
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("input".into(), Value::Array(arr));
                    result.modified = true;
                }
            }
        }
        _ => {}
    }

    result
}

/// xAI `ModelInput` allowlist (plus easy message objects with role).
/// NOTE: `reasoning` / `compaction` are intentionally NOT kept — opaque blobs from
/// Codex multi-turn often fail xAI decode ("Could not decode the compaction blob").
const KEEP_INPUT_TYPES: &[&str] = &[
    "message",
    "function_call",
    "function_call_output",
    "web_search_call",
    "x_search_call",
    "file_search_call",
    "code_interpreter_call",
    "code_execution_call",
    "mcp_call",
    "shell_call",
];

fn normalize_input_items(items: Vec<Value>, custom_names: &mut HashSet<String>) -> Vec<Value> {
    let mut out = Vec::with_capacity(items.len());
    for mut item in items {
        // Easy message: {role, content} without type — keep.
        if item.get("type").is_none() && item.get("role").is_some() {
            normalize_content_parts(&mut item);
            strip_encrypted_anywhere(&mut item);
            out.push(item);
            continue;
        }

        let ty = item
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        match ty.as_str() {
            // Opaque state — never forward (foreign/corrupt blobs cause 400).
            "compaction" | "reasoning" => {
                // Best-effort: preserve human-readable reasoning summary as plain text.
                if ty == "reasoning" {
                    if let Some(summary) = extract_reasoning_summary_text(&item) {
                        if !summary.is_empty() {
                            out.push(json!({
                                "role": "assistant",
                                "content": [{"type": "input_text", "text": format!("[reasoning] {summary}")}]
                            }));
                        }
                    }
                }
            }
            "custom_tool_call" => {
                if let Some(converted) = convert_custom_tool_call_item(&item, custom_names) {
                    out.push(converted);
                }
            }
            "custom_tool_call_output" => {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("type".into(), json!("function_call_output"));
                    obj.remove("encrypted_content");
                }
                out.push(item);
            }
            "function_call" => {
                normalize_function_call_item(&mut item);
                strip_encrypted_anywhere(&mut item);
                out.push(item);
            }
            "function_call_output" => {
                strip_encrypted_anywhere(&mut item);
                out.push(item);
            }
            // Completed image calls often carry huge b64 — collapse to a short note.
            "image_generation_call" => {
                let path = item
                    .get("path")
                    .or_else(|| item.get("output"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("image");
                out.push(json!({
                    "role": "assistant",
                    "content": [{"type": "input_text", "text": format!("[image generated: {path}]")}]
                }));
            }
            "output_text" | "input_text" => {
                if let Some(text) = item.get("text").cloned() {
                    out.push(json!({
                        "role": "user",
                        "content": [{"type": "input_text", "text": text}]
                    }));
                }
            }
            // OpenAI-only / unsupported — drop
            "computer_call"
            | "computer_call_output"
            | "local_shell_call"
            | "local_shell_call_output"
            | "tool_search_call"
            | "tool_search_output"
            | "apply_patch_call"
            | "apply_patch_call_output" => {}
            other if KEEP_INPUT_TYPES.contains(&other) || other.is_empty() => {
                normalize_content_parts(&mut item);
                strip_encrypted_anywhere(&mut item);
                out.push(item);
            }
            _ => {
                // Unknown tagged type: try keep if it looks like a message
                if item.get("role").is_some() {
                    normalize_content_parts(&mut item);
                    strip_encrypted_anywhere(&mut item);
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert("type".into(), json!("message"));
                    }
                    out.push(item);
                }
            }
        }
    }
    out
}

fn strip_encrypted_anywhere(item: &mut Value) {
    if let Some(obj) = item.as_object_mut() {
        obj.remove("encrypted_content");
        if let Some(content) = obj.get_mut("content").and_then(|c| c.as_array_mut()) {
            for part in content.iter_mut() {
                strip_encrypted_anywhere(part);
            }
        }
        if let Some(summary) = obj.get_mut("summary").and_then(|c| c.as_array_mut()) {
            for part in summary.iter_mut() {
                strip_encrypted_anywhere(part);
            }
        }
    }
}

fn extract_reasoning_summary_text(item: &Value) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(summary) = item.get("summary").and_then(|s| s.as_array()) {
        for p in summary {
            if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                parts.push(t.to_string());
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn convert_custom_tool_call_item(item: &Value, custom_names: &mut HashSet<String>) -> Option<Value> {
    let obj = item.as_object()?;
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("custom_tool")
        .to_string();
    custom_names.insert(name.clone());
    let call_id = obj
        .get("call_id")
        .cloned()
        .unwrap_or_else(|| json!(format!("call_{}", name)));

    // xAI requires arguments to be a JSON *object* encoded as a string.
    let arguments = if let Some(input) = obj.get("input") {
        custom_tool_arguments_json(input)
    } else if let Some(Value::String(args)) = obj.get("arguments") {
        // already arguments — ensure object
        if args.trim_start().starts_with('{') {
            args.clone()
        } else {
            json!({"input": args}).to_string()
        }
    } else {
        "{}".into()
    };

    Some(json!({
        "type": "function_call",
        "call_id": call_id,
        "name": name,
        "arguments": arguments
    }))
}

fn custom_tool_arguments_json(input: &Value) -> String {
    match input {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.starts_with('{') {
                if serde_json::from_str::<Value>(trimmed).is_ok() {
                    return trimmed.to_string();
                }
            }
            json!({"input": s}).to_string()
        }
        Value::Object(_) => input.to_string(),
        other => json!({"input": other}).to_string(),
    }
}

fn normalize_function_call_item(item: &mut Value) {
    let Some(obj) = item.as_object_mut() else {
        return;
    };
    obj.remove("status");
    // Ensure arguments is object JSON string
    match obj.get("arguments").cloned() {
        Some(Value::String(s)) => {
            let t = s.trim();
            if !t.starts_with('{') {
                obj.insert(
                    "arguments".into(),
                    Value::String(json!({"input": s}).to_string()),
                );
            }
        }
        Some(Value::Object(map)) => {
            obj.insert(
                "arguments".into(),
                Value::String(Value::Object(map).to_string()),
            );
        }
        Some(other) => {
            obj.insert(
                "arguments".into(),
                Value::String(json!({"input": other}).to_string()),
            );
        }
        None => {
            obj.insert("arguments".into(), Value::String("{}".into()));
        }
    }
}

fn normalize_content_parts(item: &mut Value) {
    let Some(content) = item.get_mut("content").and_then(|c| c.as_array_mut()) else {
        return;
    };
    for part in content.iter_mut() {
        let Some(obj) = part.as_object_mut() else {
            continue;
        };
        match obj.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "output_text" => {
                obj.insert("type".into(), json!("input_text"));
            }
            "output_image" => {
                obj.insert("type".into(), json!("input_image"));
            }
            _ => {}
        }
    }
}

/// Strip opaque compaction / encrypted blobs so a follow-up can proceed when
/// xAI rejects a corrupted or foreign compaction item.
///
/// Nuclear fallback: keep only plain user/assistant messages + tool call I/O.
pub fn strip_opaque_context(value: &mut Value) -> bool {
    let mut changed = false;
    if let Some(obj) = value.as_object_mut() {
        for key in [
            "previous_response_id",
            "context_management",
            "prompt_cache_key",
            "prompt_cache_retention",
        ] {
            if obj.remove(key).is_some() {
                changed = true;
            }
        }
    }
    match value.get_mut("input") {
        Some(Value::Array(items)) => {
            let before = items.clone();
            let mut kept = Vec::new();
            for item in items.iter() {
                let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if ty == "compaction" || ty == "reasoning" {
                    if let Some(summary) = extract_reasoning_summary_text(item) {
                        if !summary.is_empty() {
                            kept.push(json!({
                                "role": "assistant",
                                "content": [{"type": "input_text", "text": format!("[reasoning] {summary}")}]
                            }));
                        }
                    }
                    continue;
                }
                let mut item = item.clone();
                strip_encrypted_anywhere(&mut item);
                // Keep messages / function I/O / easy role messages only.
                if item.get("role").is_some()
                    || matches!(
                        ty,
                        "message"
                            | "function_call"
                            | "function_call_output"
                            | "custom_tool_call"
                            | "custom_tool_call_output"
                            | ""
                    )
                {
                    kept.push(item);
                }
            }
            // Always keep at least the last user-looking content if everything was stripped.
            if kept.is_empty() {
                if let Some(last) = before.last() {
                    kept.push(json!({
                        "role": "user",
                        "content": [{"type": "input_text", "text": last.to_string()}]
                    }));
                }
            }
            *items = kept;
            if *items != before {
                changed = true;
            }
        }
        Some(item) if item.is_object() => {
            let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if ty == "compaction" || ty == "reasoning" || item.get("encrypted_content").is_some() {
                *item = json!({
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Continue."}]
                });
                changed = true;
            }
        }
        _ => {}
    }
    changed
}

pub fn is_compaction_blob_error(body: &str) -> bool {
    body.contains("compaction blob")
        || body.contains("Could not decode the compaction")
        || body.contains("ModelInput")
}

pub fn is_model_input_error(body: &str) -> bool {
    body.contains("ModelInput") || body.contains("untagged enum")
}

fn convert_tool(tool: &Value, custom_names: &mut HashSet<String>) -> Option<Value> {
    let obj = tool.as_object()?;
    let ty = obj.get("type").and_then(|v| v.as_str()).unwrap_or("function");

    match ty {
        "custom" => {
            let name = obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("custom_tool")
                .to_string();
            if crate::gateway::image_bridge::is_image_gen_name(&name) {
                custom_names.insert("image_gen".into());
                return Some(crate::gateway::image_bridge::image_gen_function_tool());
            }
            custom_names.insert(name.clone());
            let description = obj
                .get("description")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            // Free-form custom tools (apply_patch, etc.) become a single-string function.
            Some(json!({
                "type": "function",
                "name": name,
                "description": description,
                "parameters": {
                    "type": "object",
                    "properties": {
                        "input": {
                            "type": "string",
                            "description": "Free-form tool input text"
                        }
                    },
                    "required": ["input"]
                }
            }))
        }
        "function" => {
            let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if crate::gateway::image_bridge::is_image_gen_name(name) {
                custom_names.insert("image_gen".into());
                return Some(crate::gateway::image_bridge::image_gen_function_tool());
            }
            Some(normalize_function_tool(tool))
        }
        // OpenAI / Codex image generation → function tool fulfilled by grok-go.
        "image_generation" | "image_gen" => {
            custom_names.insert("image_gen".into());
            Some(crate::gateway::image_bridge::image_gen_function_tool())
        }
        // OpenAI aliases / previews → xAI built-ins
        "web_search" | "web_search_preview" => Some(json!({"type": "web_search"})),
        "local_shell" => Some(json!({"type": "shell"})),
        "code_interpreter" => Some(json!({"type": "code_interpreter"})),
        "code_execution" => Some(json!({"type": "code_execution"})),
        "x_search" | "collections_search" | "file_search" | "mcp" | "shell" => {
            Some(tool.clone())
        }
        // Computer-use and other OpenAI-only built-ins are not on xAI — drop.
        "computer_use" | "computer_use_preview" | "file_search_preview" => None,
        other if XAI_TOOL_TYPES.contains(&other) => Some(tool.clone()),
        _ => {
            // Unknown type: try to salvage if it looks like a function (has name + schema).
            if obj.get("name").is_some()
                && (obj.get("parameters").is_some() || obj.get("inputSchema").is_some())
            {
                let mut v = tool.clone();
                if let Some(m) = v.as_object_mut() {
                    m.insert("type".into(), json!("function"));
                }
                Some(normalize_function_tool(&v))
            } else {
                None
            }
        }
    }
}

fn normalize_function_tool(tool: &Value) -> Value {
    let mut out = Map::new();
    out.insert("type".into(), json!("function"));

    // Chat Completions nested form: {type:function, function:{name,description,parameters}}
    if let Some(inner) = tool.get("function").and_then(|v| v.as_object()) {
        if let Some(name) = inner.get("name") {
            out.insert("name".into(), name.clone());
        }
        if let Some(desc) = inner.get("description") {
            out.insert("description".into(), desc.clone());
        }
        let params = inner
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        out.insert("parameters".into(), params);
        if let Some(strict) = inner.get("strict") {
            out.insert("strict".into(), strict.clone());
        }
        return Value::Object(out);
    }

    if let Some(name) = tool.get("name") {
        out.insert("name".into(), name.clone());
    }
    if let Some(desc) = tool.get("description") {
        out.insert("description".into(), desc.clone());
    }

    // Codex MCP-style tools use `inputSchema` instead of `parameters`.
    let params = tool
        .get("parameters")
        .or_else(|| tool.get("inputSchema"))
        .cloned()
        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
    out.insert("parameters".into(), params);

    if let Some(strict) = tool.get("strict") {
        out.insert("strict".into(), strict.clone());
    }

    Value::Object(out)
}

fn rewrite_tool_choice(choice: &mut Value, custom_names: &HashSet<String>) {
    match choice {
        Value::Object(obj) => {
            let ty = obj.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if ty == "custom" {
                obj.insert("type".into(), json!("function"));
            }
            // Nested {type:function, function:{name}} already ok for xAI.
            // Flat {type:function, name} also ok.
            if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                if custom_names.contains(name) {
                    obj.insert("type".into(), json!("function"));
                }
            }
        }
        Value::String(s) if s == "custom" => {
            *choice = json!("auto");
        }
        _ => {}
    }
}

/// Rewrite an upstream Responses JSON value so Codex sees custom tool calls again.
/// Returns true if anything changed (caller should only re-serialize when true —
/// re-serializing otherwise can corrupt opaque `encrypted_content` / compaction blobs).
pub fn rewrite_responses_payload(value: &mut Value, custom_names: &HashSet<String>) -> bool {
    if custom_names.is_empty() {
        return false;
    }
    let mut changed = false;

    if let Some(output) = value.get_mut("output").and_then(|o| o.as_array_mut()) {
        for item in output.iter_mut() {
            if rewrite_output_item(item, custom_names) {
                changed = true;
            }
        }
    }

    if let Some(item) = value.get_mut("item") {
        if rewrite_output_item(item, custom_names) {
            changed = true;
        }
    }

    // Streaming events sometimes nest under `response`.
    if let Some(response) = value.get_mut("response") {
        if rewrite_responses_payload(response, custom_names) {
            changed = true;
        }
    }
    changed
}

fn rewrite_output_item(item: &mut Value, custom_names: &HashSet<String>) -> bool {
    let Some(obj) = item.as_object_mut() else {
        return false;
    };
    let ty = obj
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if ty == "function_call" && custom_names.contains(&name) {
        obj.insert("type".into(), json!("custom_tool_call"));
        if let Some(args) = obj.remove("arguments") {
            let input = extract_custom_input(&args);
            obj.insert("input".into(), Value::String(input));
        }
        if !obj.contains_key("id") {
            let call_id = obj
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("call");
            obj.insert("id".into(), json!(format!("ctc_{call_id}")));
        }
        if !obj.contains_key("status") {
            obj.insert("status".into(), json!("completed"));
        }
        return true;
    }
    false
}

fn extract_custom_input(args: &Value) -> String {
    match args {
        Value::String(s) => {
            // arguments is usually a JSON string.
            if let Ok(Value::Object(map)) = serde_json::from_str(s) {
                if let Some(Value::String(input)) = map.get("input") {
                    return input.clone();
                }
                // Single string property fallback
                if map.len() == 1 {
                    if let Some(Value::String(only)) = map.values().next() {
                        return only.clone();
                    }
                }
            }
            s.clone()
        }
        Value::Object(map) => {
            if let Some(Value::String(input)) = map.get("input") {
                return input.clone();
            }
            args.to_string()
        }
        other => other.to_string(),
    }
}

/// Rewrite one SSE `data:` JSON payload line (without the `data:` prefix).
/// Preserves the original line bytes when nothing was rewritten.
pub fn rewrite_sse_data_line(line: &str, custom_names: &HashSet<String>) -> String {
    if custom_names.is_empty() || line.is_empty() || line == "[DONE]" {
        return line.to_string();
    }
    // Fast path: skip parse/re-serialize unless this chunk likely has a tool call.
    if !line.contains("function_call") {
        return line.to_string();
    }
    match serde_json::from_str::<Value>(line) {
        Ok(mut value) => {
            if rewrite_responses_payload(&mut value, custom_names) {
                value.to_string()
            } else {
                line.to_string()
            }
        }
        Err(_) => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_custom_tool_to_function() {
        let mut body = json!({
            "model": "grok-4.5",
            "input": "hi",
            "tools": [
                {
                    "type": "custom",
                    "name": "apply_patch",
                    "description": "Apply a patch"
                },
                {
                    "type": "function",
                    "name": "shell",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"command": {"type": "string"}},
                        "required": ["command"]
                    }
                },
                {"type": "computer_use_preview"}
            ]
        });
        let result = sanitize_responses_request(&mut body);
        assert!(result.custom_tool_names.contains("apply_patch"));
        let tools = body["tools"].as_array().unwrap();
        // apply_patch + shell + injected x_search + web_search
        assert!(tools.len() >= 2);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "apply_patch");
        assert!(tools[0].get("parameters").is_some());
        assert_eq!(tools[1]["name"], "shell");
        assert!(tools[1].get("parameters").is_some());
        assert!(tools[1].get("inputSchema").is_none());
        let types: Vec<_> = tools
            .iter()
            .filter_map(|t| t.get("type").and_then(|v| v.as_str()))
            .collect();
        assert!(types.contains(&"x_search"));
        assert!(types.contains(&"web_search"));
    }

    #[test]
    fn rewrites_function_call_back_to_custom() {
        let custom = HashSet::from(["apply_patch".to_string()]);
        let mut payload = json!({
            "output": [{
                "type": "function_call",
                "call_id": "call-1",
                "name": "apply_patch",
                "arguments": "{\"input\":\"*** Begin Patch ***\"}"
            }]
        });
        rewrite_responses_payload(&mut payload, &custom);
        assert_eq!(payload["output"][0]["type"], "custom_tool_call");
        assert_eq!(payload["output"][0]["input"], "*** Begin Patch ***");
    }

    #[test]
    fn sanitizes_custom_tool_outputs_in_input() {
        let mut body = json!({
            "input": [
                {"type": "custom_tool_call_output", "call_id": "c1", "output": "ok"},
                {
                    "type": "custom_tool_call",
                    "call_id": "c2",
                    "name": "apply_patch",
                    "input": "patch body",
                    "status": "completed",
                    "id": "ctc_c2"
                }
            ]
        });
        sanitize_responses_request(&mut body);
        assert_eq!(body["input"][0]["type"], "function_call_output");
        assert_eq!(body["input"][1]["type"], "function_call");
        assert!(body["input"][1].get("arguments").is_some());
        assert!(body["input"][1].get("input").is_none());
    }

    #[test]
    fn strips_compaction_and_reasoning() {
        let mut body = json!({
            "previous_response_id": "resp_1",
            "input": [
                {"type": "compaction", "encrypted_content": "BLOB"},
                {"type": "reasoning", "encrypted_content": "R1", "summary": [{"text": "think"}]},
                {"role": "user", "content": "hi"}
            ]
        });
        assert!(strip_opaque_context(&mut body));
        assert!(body.get("previous_response_id").is_none());
        let input = body["input"].as_array().unwrap();
        // reasoning summary preserved as assistant text + user message
        assert!(input.len() >= 1);
        assert!(input.iter().any(|i| i.get("role").and_then(|r| r.as_str()) == Some("user")));
    }

    #[test]
    fn sanitize_drops_compaction_proactively() {
        let mut body = json!({
            "model": "grok-4.5",
            "input": [
                {"type": "compaction", "encrypted_content": "FOREIGN_BLOB"},
                {"type": "reasoning", "encrypted_content": "ENC", "summary": []},
                {"role": "user", "content": [{"type": "input_text", "text": "draw a cat"}]}
            ]
        });
        sanitize_responses_request(&mut body);
        let input = body["input"].as_array().unwrap();
        assert!(input.iter().all(|i| {
            let ty = i.get("type").and_then(|t| t.as_str()).unwrap_or("");
            ty != "compaction" && ty != "reasoning"
        }));
        assert!(input.iter().any(|i| i.get("role").and_then(|r| r.as_str()) == Some("user")));
        // image_gen injected
        assert!(body["tools"].as_array().unwrap().iter().any(|t| {
            t.get("name").and_then(|n| n.as_str()) == Some("image_gen")
        }));
    }

    #[test]
    fn converts_custom_tool_call_and_output_text() {
        let mut body = json!({
            "model": "grok-4.5",
            "previous_response_id": "resp_x",
            "input": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "hello"}]
                },
                {
                    "type": "custom_tool_call",
                    "call_id": "c0",
                    "name": "ApplyPatch",
                    "input": "*** Begin Patch",
                    "status": "completed",
                    "id": "ctc_c0"
                },
                {
                    "type": "custom_tool_call_output",
                    "call_id": "c0",
                    "output": "ok"
                },
                {"type": "computer_call", "call_id": "x"}
            ],
            "tools": [],
            "tool_choice": "auto"
        });
        let r = sanitize_responses_request(&mut body);
        assert!(r.modified);
        assert!(body.get("previous_response_id").is_none());
        // empty tools are refilled with x_search / web_search / image_gen
        assert!(body.get("tools").and_then(|t| t.as_array()).map(|a| !a.is_empty()).unwrap_or(false));
        let input = body["input"].as_array().unwrap();
        // computer_call dropped; custom converted; output_text -> input_text
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["name"], "ApplyPatch");
        assert!(input[1].get("input").is_none());
        assert!(input[1].get("status").is_none());
        let args: Value = serde_json::from_str(input[1]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["input"], "*** Begin Patch");
        assert_eq!(input[2]["type"], "function_call_output");
    }

    #[test]
    fn injects_x_search_and_image_gen_when_missing() {
        let mut body = json!({
            "model": "grok-4.5",
            "tools": [{"type": "function", "name": "shell", "parameters": {"type": "object"}}]
        });
        let r = sanitize_responses_request(&mut body);
        assert!(r.has_image_gen_tools);
        let tools = body["tools"].as_array().unwrap();
        let types: Vec<_> = tools
            .iter()
            .filter_map(|t| t.get("type").and_then(|v| v.as_str()))
            .collect();
        let names: Vec<_> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()))
            .collect();
        assert!(types.contains(&"x_search"));
        assert!(types.contains(&"web_search"));
        assert!(names.contains(&"image_gen"));
    }

    #[test]
    fn injects_tools_when_absent() {
        let mut body = json!({
            "model": "grok-4.5",
            "input": "draw a cat"
        });
        let r = sanitize_responses_request(&mut body);
        assert!(r.has_image_gen_tools);
        assert!(body.get("tools").and_then(|t| t.as_array()).map(|a| !a.is_empty()).unwrap_or(false));
    }
}
