//! Recover from premature agent stops that make Codex end mid-task.
//!
//! Structural rule (task-agnostic):
//! - request exposes tools
//! - response is `completed` with **no tool call**
//! - response is **not** a clear final answer (question / long text / delivery)
//!
//! Covers both pure reasoning-only empties and short status-only messages.
//! Soft retry + synthetic shell probe keep the Codex agent loop alive.

use serde_json::{json, Value};

/// User-visible recovery instruction injected on retry.
pub const EMPTY_COMPLETION_NUDGE: &str = "\
[grok-go recovery] Your previous model turn ended without a tool call \
(reasoning-only and/or status narration only). The agent runtime will stop \
the task if you only announce the next step. Immediately continue the user's \
unfinished work: emit a tool call now (preferred), or produce a final concrete \
answer that fully completes the request. Do not end with reasoning or \
\"I'll do X next\" narration alone.";

/// Output item types that count as real tool / side-effect work.
const TOOL_OUTPUT_TYPES: &[&str] = &[
    "function_call",
    "custom_tool_call",
    "tool_call",
    "image_generation_call",
    "web_search_call",
    "file_search_call",
    "code_interpreter_call",
    "computer_call",
    "mcp_call",
    "local_shell_call",
    "shell_call",
    "apply_patch_call",
    "bash_code_execution_call",
    "text_editor_code_execution_call",
];

/// Messages longer than this are treated as real final answers (leave alone).
const FINAL_ANSWER_MIN_CHARS: usize = 280;

/// Neutral probe injected when soft recovery still produces no tool call.
/// Side-effect free; only keeps the agent loop alive.
pub const RECOVERY_PROBE_CMD: &str = "echo grok-go-continue";

/// True when a completed Responses payload has no message and no tool calls —
/// only reasoning / empty / non-actionable items.
pub fn is_reasoning_only_empty_completion(response: &Value) -> bool {
    let response = unwrap_response(response);
    if !is_clean_completed(response) {
        return false;
    }
    let Some(output) = response.get("output").and_then(|o| o.as_array()) else {
        return false;
    };
    if output.is_empty() {
        return true;
    }
    for item in output {
        if item_is_actionable(item) {
            return false;
        }
    }
    true
}

/// True when the request is an agent turn (has tools) but the model completed
/// without any tool call and without a clear final answer.
///
/// Structure-only — no task/domain keyword lists.
pub fn is_tool_less_non_final_stop(response: &Value, request: Option<&Value>) -> bool {
    if !request_has_tools(request) {
        return false;
    }
    let response = unwrap_response(response);
    if !is_clean_completed(response) {
        return false;
    }
    let Some(output) = response.get("output").and_then(|o| o.as_array()) else {
        return false;
    };
    if output_has_tool_call(output) {
        return false;
    }
    // No message at all → premature (reasoning-only / empty output already covered
    // by is_reasoning_only_empty_completion, but keep this path consistent).
    let Some(msg) = extract_assistant_message_text(response) else {
        return !output.is_empty(); // has non-message items only, or empty handled elsewhere
    };
    !looks_like_final_answer(&msg)
}

/// Unified gate: reasoning-only empty **or** tools present + no tool call + non-final.
pub fn should_retry_premature_agent_stop(response: &Value, request: Option<&Value>) -> bool {
    is_reasoning_only_empty_completion(response)
        || is_tool_less_non_final_stop(response, request)
}

/// Backward-compatible alias used in older docs/tests naming.
pub fn is_narration_only_premature_stop(response: &Value, request: Option<&Value>) -> bool {
    is_tool_less_non_final_stop(response, request)
}

fn unwrap_response(response: &Value) -> &Value {
    response
        .get("response")
        .filter(|r| r.is_object())
        .unwrap_or(response)
}

fn is_clean_completed(response: &Value) -> bool {
    if response.get("error").map(|e| !e.is_null()).unwrap_or(false) {
        return false;
    }
    if response
        .get("incomplete_details")
        .map(|d| !d.is_null())
        .unwrap_or(false)
    {
        return false;
    }
    let status = response
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("completed");
    status == "completed" || status == "complete"
}

fn request_has_tools(request: Option<&Value>) -> bool {
    request
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

fn output_has_tool_call(output: &[Value]) -> bool {
    output.iter().any(|item| {
        let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        TOOL_OUTPUT_TYPES.contains(&ty)
    })
}

fn item_is_actionable(item: &Value) -> bool {
    let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if TOOL_OUTPUT_TYPES.contains(&ty) {
        return true;
    }
    if ty == "message" {
        return message_has_visible_text(item);
    }
    // Unknown types: treat as actionable so we don't retry aggressively.
    if !ty.is_empty() && ty != "reasoning" && ty != "compaction" && ty != "thinking" {
        return true;
    }
    false
}

fn message_has_visible_text(item: &Value) -> bool {
    extract_text_from_message_item(item)
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false)
}

fn extract_assistant_message_text(response: &Value) -> Option<String> {
    let output = response.get("output")?.as_array()?;
    let mut parts: Vec<String> = Vec::new();
    for item in output {
        if item.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        if let Some(t) = extract_text_from_message_item(item) {
            let t = t.trim();
            if !t.is_empty() {
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

fn extract_text_from_message_item(item: &Value) -> Option<String> {
    let content = item.get("content")?;
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let mut out = String::new();
            for p in parts {
                let part_ty = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if matches!(
                    part_ty,
                    "output_text" | "text" | "input_text" | "summary_text" | ""
                ) {
                    if let Some(text) = p.get("text").and_then(|t| t.as_str()) {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(text);
                    }
                } else if let Some(text) = p.get("text").and_then(|t| t.as_str()) {
                    // other text-bearing parts
                    if !text.trim().is_empty() {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(text);
                    }
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

/// Structural "this looks like a real end, don't force tools".
///
/// No domain / language progressive-intent word lists — only shape signals:
/// length, user questions, delivery/path markers.
fn looks_like_final_answer(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    // Long prose is treated as a finished answer.
    if t.chars().count() >= FINAL_ANSWER_MIN_CHARS {
        return true;
    }
    // Asking the user → yield control, don't auto-continue.
    if t.contains('?') || t.contains('？') {
        return true;
    }
    looks_like_final_delivery(t)
}

fn looks_like_final_delivery(text: &str) -> bool {
    let lower = text.to_lowercase();
    // Generic delivery / completion markers (not task-domain specific).
    const FINAL_MARKERS: &[&str] = &[
        "已完成",
        "做完了",
        "完成了",
        "here's the",
        "here is the",
        "completed successfully",
        "all done",
        "task complete",
        "saved to",
        "written to",
        "wrote to",
        "output path",
        "file path",
    ];
    for m in FINAL_MARKERS {
        if lower.contains(m) || text.contains(m) {
            return true;
        }
    }
    // Absolute paths often mean a concrete artifact was reported.
    if text.contains("/Users/")
        || text.contains("/home/")
        || text.contains("~/")
        || text.contains("C:\\")
        || text.contains("c:\\")
    {
        return true;
    }
    false
}

/// Extract the completed response object from an SSE body (last `response.completed`).
pub fn extract_completed_response_from_sse(sse: &str) -> Option<Value> {
    let mut last: Option<Value> = None;
    for line in sse.lines() {
        let data = if let Some(rest) = line.strip_prefix("data:") {
            rest.trim()
        } else {
            continue;
        };
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        let ty = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if ty == "response.completed" {
            if let Some(resp) = value.get("response").cloned() {
                last = Some(resp);
            } else {
                last = Some(value);
            }
        }
    }
    last
}

/// Build a one-shot non-stream retry request with continuity + recovery nudge.
///
/// When the original request exposes tools, pin `tool_choice` to a concrete shell
/// function (or `"required"`) so the recovery sample cannot end as pure
/// reasoning / status narration again.
pub fn build_empty_completion_retry_request(
    original: &Value,
    empty_response: &Value,
) -> Value {
    let mut retry = original.clone();
    if let Some(obj) = retry.as_object_mut() {
        obj.insert("stream".into(), json!(false));
        // Avoid sticky incomplete chains that may re-bias toward empty stops.
        obj.remove("previous_response_id");
        if let Some(name) = pick_shell_tool_name(original) {
            // Pin the tool — plain "required" is often ignored by upstream Grok.
            obj.insert(
                "tool_choice".into(),
                json!({"type": "function", "name": name}),
            );
        } else if request_has_tools(Some(original)) {
            obj.insert("tool_choice".into(), json!("required"));
        }
    }

    let mut input_items = normalize_input_to_array(retry.get("input"));

    if let Some(summary) = extract_reasoning_summary(empty_response) {
        if !summary.trim().is_empty() {
            input_items.push(json!({
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": format!("[previous incomplete turn — reasoning]\n{summary}")
                }]
            }));
        }
    }
    if let Some(msg) = extract_assistant_message_text(unwrap_response(empty_response)) {
        if !msg.trim().is_empty() {
            input_items.push(json!({
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": format!("[previous incomplete turn — narration only, no tool call]\n{msg}")
                }]
            }));
        }
    }

    input_items.push(json!({
        "role": "user",
        "content": [{
            "type": "input_text",
            "text": EMPTY_COMPLETION_NUDGE
        }]
    }));

    if let Some(obj) = retry.as_object_mut() {
        obj.insert("input".into(), Value::Array(input_items));
    }
    retry
}

/// Rank a response for recovery fallback: tool call ≫ user-visible message ≫ reasoning.
/// Used when all recovery attempts are still "premature" — never prefer pure empty
/// over a later attempt that at least produced a message.
pub fn recovery_quality_score(response: &Value) -> i32 {
    let response = unwrap_response(response);
    let Some(output) = response.get("output").and_then(|o| o.as_array()) else {
        return 0;
    };
    if output.is_empty() {
        return 0;
    }
    if output_has_tool_call(output) {
        return 100;
    }
    if extract_assistant_message_text(response).is_some() {
        return 50;
    }
    // reasoning / compaction only
    10
}

/// Prefer shell-like tools Codex actually runs mid-task.
const SHELL_TOOL_CANDIDATES: &[&str] = &[
    "exec_command",
    "shell",
    "run_terminal_cmd",
    "Bash",
    "bash",
    "local_shell",
];

/// Pick a shell/exec tool name from the request's tools list.
pub fn pick_shell_tool_name(request: &Value) -> Option<String> {
    let tools = request.get("tools")?.as_array()?;
    let mut names: Vec<String> = Vec::new();
    for t in tools {
        if let Some(n) = t.get("name").and_then(|v| v.as_str()) {
            if !n.is_empty() {
                names.push(n.to_string());
            }
        }
        if let Some(n) = t.pointer("/function/name").and_then(|v| v.as_str()) {
            if !n.is_empty() {
                names.push(n.to_string());
            }
        }
    }
    for cand in SHELL_TOOL_CANDIDATES {
        if names.iter().any(|n| n == cand) {
            return Some((*cand).to_string());
        }
    }
    // Fall back to first function-shaped tool.
    names.into_iter().next()
}

/// Last-resort: invent a real function_call so Codex's agent loop continues.
///
/// Upstream may ignore `tool_choice`; soft retries then still return narration.
/// Injecting an `exec_command` (or similar) is the only guaranteed way to set
/// Codex `model_needs_follow_up=true` again.
pub fn synthesize_forced_tool_response(
    original_request: &Value,
    premature_response: &Value,
) -> Option<Value> {
    let tool_name = pick_shell_tool_name(original_request)?;
    let cmd = RECOVERY_PROBE_CMD;
    let call_id = format!(
        "call_grokgo_recovery_{}",
        &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
    );
    let args = match tool_name.as_str() {
        // Codex primary shell tool
        "exec_command" => json!({ "cmd": cmd }).to_string(),
        "shell" | "Bash" | "bash" | "local_shell" => json!({ "command": cmd }).to_string(),
        "run_terminal_cmd" => json!({ "command": cmd }).to_string(),
        // Generic: most function tools accept a free-form object; prefer cmd.
        _ => json!({ "cmd": cmd, "command": cmd }).to_string(),
    };

    let base = unwrap_response(premature_response);
    let mut out = if base.is_object() {
        base.clone()
    } else {
        json!({})
    };

    let status_text = extract_assistant_message_text(base)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Continuing unfinished work.".into());
    let user_line = format!(
        "{status_text}\n[grok-go] auto-continue: model ended without a tool call; injected neutral probe."
    );

    let mut output_items: Vec<Value> = Vec::new();
    // Keep prior reasoning summaries for continuity when present.
    if let Some(arr) = base.get("output").and_then(|o| o.as_array()) {
        for item in arr {
            let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if ty == "reasoning" {
                output_items.push(item.clone());
            }
        }
    }
    output_items.push(json!({
        "type": "message",
        "role": "assistant",
        "status": "completed",
        "content": [{"type": "output_text", "text": user_line}]
    }));
    output_items.push(json!({
        "type": "function_call",
        "id": format!("fc_{call_id}"),
        "call_id": call_id,
        "name": tool_name,
        "arguments": args,
        "status": "completed"
    }));

    if let Some(obj) = out.as_object_mut() {
        obj.insert("status".into(), json!("completed"));
        obj.insert("output".into(), Value::Array(output_items));
        obj.insert("error".into(), Value::Null);
        obj.insert("incomplete_details".into(), Value::Null);
        if !obj.contains_key("id") {
            obj.insert(
                "id".into(),
                json!(format!("resp_grokgo_recovery_{}", &call_id[call_id.len().saturating_sub(8)..])),
            );
        }
        if !obj.contains_key("object") {
            obj.insert("object".into(), json!("response"));
        }
    }
    Some(out)
}

fn normalize_input_to_array(input: Option<&Value>) -> Vec<Value> {
    match input {
        None => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                Vec::new()
            } else {
                vec![json!({
                    "role": "user",
                    "content": [{"type": "input_text", "text": s}]
                })]
            }
        }
        Some(other) => vec![other.clone()],
    }
}

fn extract_reasoning_summary(response: &Value) -> Option<String> {
    let response = unwrap_response(response);
    let output = response.get("output")?.as_array()?;
    let mut parts: Vec<String> = Vec::new();
    for item in output {
        let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if ty != "reasoning" {
            continue;
        }
        if let Some(summary) = item.get("summary").and_then(|s| s.as_array()) {
            for s in summary {
                if let Some(text) = s.get("text").and_then(|t| t.as_str()) {
                    if !text.trim().is_empty() {
                        parts.push(text.trim().to_string());
                    }
                } else if let Some(text) = s.as_str() {
                    if !text.trim().is_empty() {
                        parts.push(text.trim().to_string());
                    }
                }
            }
        }
        if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
            for c in content {
                if let Some(text) = c.get("text").and_then(|t| t.as_str()) {
                    if !text.trim().is_empty() {
                        parts.push(text.trim().to_string());
                    }
                }
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// True when `path` is the Responses API endpoint we should guard.
pub fn is_responses_path(path: &str) -> bool {
    path == "/responses"
        || path.ends_with("/responses")
        || path == "/v1/responses"
        || path.ends_with("/v1/responses")
}

/// Cap buffered SSE size for empty-completion inspection (bytes).
pub const SSE_BUFFER_LIMIT: usize = 24 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tools_req() -> Value {
        json!({
            "model": "grok-4.5",
            "tools": [{"type": "function", "name": "exec_command"}],
            "input": [{"role": "user", "content": [{"type": "input_text", "text": "build pdf"}]}]
        })
    }

    #[test]
    fn detects_reasoning_only_completed() {
        let v = json!({
            "id": "34f3529a",
            "status": "completed",
            "error": null,
            "incomplete_details": null,
            "output": [{
                "type": "reasoning",
                "id": "rs_1",
                "status": "completed",
                "summary": [{"type": "summary_text", "text": "Let me view the pages."}]
            }],
            "usage": {"input_tokens": 22059, "output_tokens": 133}
        });
        assert!(is_reasoning_only_empty_completion(&v));
        assert!(should_retry_premature_agent_stop(&v, Some(&tools_req())));
    }

    #[test]
    fn accepts_function_call() {
        let v = json!({
            "status": "completed",
            "output": [
                {"type": "reasoning", "summary": [{"text": "run ls"}]},
                {"type": "function_call", "name": "exec_command", "arguments": "{}"}
            ]
        });
        assert!(!is_reasoning_only_empty_completion(&v));
        assert!(!should_retry_premature_agent_stop(&v, Some(&tools_req())));
    }

    #[test]
    fn final_message_without_tools_request_not_retried() {
        let v = json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "done"}]
            }]
        });
        // reasoning-only detector: has message text → false
        assert!(!is_reasoning_only_empty_completion(&v));
        // no tools on request → narration path off
        assert!(!should_retry_premature_agent_stop(&v, None));
    }

    #[test]
    fn rejects_empty_message_as_empty() {
        let v = json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "  "}]
            }]
        });
        assert!(is_reasoning_only_empty_completion(&v));
    }

    /// Fixture matching Codex session 019f6852… premature stop shape:
    /// completed + reasoning only + tools were in the request → must recover.
    #[test]
    fn session_019f6852_style_reasoning_only_is_premature() {
        let v = json!({
            "id": "resp_test",
            "status": "completed",
            "output": [{
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "I need to view the reference PDF pages to understand the structure."}]
            }]
        });
        assert!(is_reasoning_only_empty_completion(&v));
        assert!(should_retry_premature_agent_stop(&v, Some(&tools_req())));
    }

    #[test]
    fn detects_short_status_without_tool_as_premature() {
        let v = json!({
            "status": "completed",
            "error": null,
            "incomplete_details": null,
            "output": [
                {
                    "type": "reasoning",
                    "summary": [{"text": "I need to continue the work."}]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "先对照参考 PDF 和现有素材预览，摸清页面顺序和车票拼接方式。"}]
                }
            ]
        });
        assert!(!is_reasoning_only_empty_completion(&v));
        // Structure: tools + no tool_call + short non-final message.
        assert!(is_tool_less_non_final_stop(&v, Some(&tools_req())));
        assert!(should_retry_premature_agent_stop(&v, Some(&tools_req())));
        // Without tools in request, do not force retry.
        assert!(!is_tool_less_non_final_stop(&v, None));
    }

    #[test]
    fn long_message_without_tool_is_treated_as_final() {
        let long = "x".repeat(300);
        let v = json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": long}]
            }]
        });
        assert!(!should_retry_premature_agent_stop(&v, Some(&tools_req())));
    }

    #[test]
    fn does_not_retry_final_delivery_message() {
        let v = json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "已完成，成品路径：/Users/ronglecat/Downloads/out.pdf"}]
            }]
        });
        assert!(!should_retry_premature_agent_stop(&v, Some(&tools_req())));
    }

    #[test]
    fn does_not_retry_user_question() {
        let v = json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "需要我按 5 页还是 6 页输出？"}]
            }]
        });
        assert!(!should_retry_premature_agent_stop(&v, Some(&tools_req())));
    }

    #[test]
    fn skips_errored_or_incomplete() {
        let err = json!({
            "status": "completed",
            "error": {"message": "boom"},
            "output": []
        });
        assert!(!is_reasoning_only_empty_completion(&err));

        let incomplete = json!({
            "status": "incomplete",
            "incomplete_details": {"reason": "max_output_tokens"},
            "output": [{"type": "reasoning", "summary": []}]
        });
        assert!(!is_reasoning_only_empty_completion(&incomplete));
    }

    #[test]
    fn parse_completed_from_sse() {
        let sse = r#"event: response.created
data: {"type":"response.created","response":{"id":"r1","status":"in_progress"}}

event: response.output_item.done
data: {"type":"response.output_item.done","item":{"type":"reasoning","summary":[{"text":"hi"}]}}

event: response.completed
data: {"type":"response.completed","response":{"id":"r1","status":"completed","output":[{"type":"reasoning","summary":[{"text":"hi"}]}]}}

"#;
        let parsed = extract_completed_response_from_sse(sse).expect("parsed");
        assert_eq!(parsed.get("id").and_then(|v| v.as_str()), Some("r1"));
        assert!(is_reasoning_only_empty_completion(&parsed));
    }

    #[test]
    fn retry_request_appends_nudge_forces_tool_choice_and_prior_narration() {
        let original = json!({
            "model": "grok-4.5",
            "stream": true,
            "previous_response_id": "resp_old",
            "input": [
                {"role": "user", "content": [{"type": "input_text", "text": "build the pdf"}]}
            ],
            "tools": [{"type": "function", "name": "exec_command"}]
        });
        let empty = json!({
            "status": "completed",
            "output": [
                {"type": "reasoning", "summary": [{"text": "I should view pages next"}]},
                {"type": "message", "content": [{"type": "output_text", "text": "先对照参考 PDF"}]}
            ]
        });
        let retry = build_empty_completion_retry_request(&original, &empty);
        assert_eq!(retry.get("stream").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            retry.pointer("/tool_choice/type").and_then(|v| v.as_str()),
            Some("function")
        );
        assert_eq!(
            retry.pointer("/tool_choice/name").and_then(|v| v.as_str()),
            Some("exec_command")
        );
        assert!(retry.get("previous_response_id").is_none());
        let input = retry.get("input").and_then(|v| v.as_array()).unwrap();
        let joined = input
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("grok-go recovery"));
        assert!(joined.contains("view pages"));
        assert!(joined.contains("先对照参考 PDF"));
        assert!(joined.contains("narration only"));
    }

    #[test]
    fn quality_score_prefers_message_over_reasoning_only() {
        let empty = json!({
            "status": "completed",
            "output": [{"type": "reasoning", "summary": [{"text": "mixed orientations"}]}]
        });
        let with_msg = json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "先看图"}]
            }]
        });
        let with_tool = json!({
            "status": "completed",
            "output": [{"type": "function_call", "name": "exec_command", "arguments": "{}"}]
        });
        assert!(recovery_quality_score(&with_tool) > recovery_quality_score(&with_msg));
        assert!(recovery_quality_score(&with_msg) > recovery_quality_score(&empty));
    }

    #[test]
    fn synthesizes_forced_exec_command_when_tools_present() {
        let req = json!({
            "tools": [{"type": "function", "name": "exec_command"}],
            "input": [{
                "role": "user",
                "content": [{"type": "input_text", "text": "处理 /Users/ronglecat/Downloads/0706出差"}]
            }]
        });
        let premature = json!({
            "id": "r1",
            "status": "completed",
            "output": [
                {"type": "reasoning", "summary": [{"text": "I'll look at the reference images"}]},
                {"type": "message", "content": [{"type": "output_text", "text": "正在对照参考 PDF 与现有素材的页面结构，先看参考页和已生成预览。"}]}
            ]
        });
        let forced = synthesize_forced_tool_response(&req, &premature).expect("forced");
        assert!(!should_retry_premature_agent_stop(&forced, Some(&req)));
        assert_eq!(recovery_quality_score(&forced), 100);
        let output = forced.get("output").and_then(|o| o.as_array()).unwrap();
        assert!(output.iter().any(|i| i.get("type").and_then(|t| t.as_str()) == Some("function_call")));
        let fc = output
            .iter()
            .find(|i| i.get("type").and_then(|t| t.as_str()) == Some("function_call"))
            .unwrap();
        assert_eq!(fc.get("name").and_then(|n| n.as_str()), Some("exec_command"));
        let args = fc.get("arguments").and_then(|a| a.as_str()).unwrap();
        assert!(
            args.contains(RECOVERY_PROBE_CMD),
            "expected neutral probe, got {args}"
        );
        assert!(!args.contains("0706"), "probe must not embed task paths");
    }
}
