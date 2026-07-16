//! Layered gateway / MCP / upstream error codes for agent clients.
//!
//! Outer OpenAI / Anthropic shapes stay parseable; stable codes go in
//! `error.code` (and optional `error.param` / headers).

use serde_json::{json, Value};

/// Stable machine-readable codes (string form for JSON `error.code`).
pub const GATEWAY_DOWN: &str = "GATEWAY_DOWN";
pub const UPSTREAM_TIMEOUT: &str = "UPSTREAM_TIMEOUT";
pub const TOOL_TIMEOUT: &str = "TOOL_TIMEOUT";
pub const TOOL_FAILED: &str = "TOOL_FAILED";
pub const ACCOUNT_COOLDOWN: &str = "ACCOUNT_COOLDOWN";
pub const CANCELLED: &str = "CANCELLED";
pub const INVALID_REQUEST: &str = "INVALID_REQUEST";
pub const UPSTREAM_ERROR: &str = "UPSTREAM_ERROR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayeredError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub hint: String,
    /// HTTP status for gateway-originated errors.
    pub status: u16,
}

impl LayeredError {
    pub fn openai_body(&self) -> Value {
        json!({
            "error": {
                "message": self.message,
                "type": openai_type_for(self.code),
                "code": self.code,
                "param": null,
                "retryable": self.retryable,
                "hint": self.hint,
            }
        })
    }

    pub fn anthropic_body(&self) -> Value {
        json!({
            "type": "error",
            "error": {
                "type": anthropic_type_for(self.code, self.status),
                "message": format!("[{}] {} — {}", self.code, self.message, self.hint),
                "code": self.code,
                "retryable": self.retryable,
                "hint": self.hint,
            }
        })
    }

    pub fn tool_envelope(&self, tool: &str) -> Value {
        json!({
            "ok": false,
            "tool": tool,
            "summary": self.message,
            "artifacts": [],
            "error": {
                "code": self.code,
                "retryable": self.retryable,
                "message": self.message,
                "hint": self.hint,
            }
        })
    }
}

fn openai_type_for(code: &str) -> &'static str {
    match code {
        INVALID_REQUEST => "invalid_request_error",
        GATEWAY_DOWN | UPSTREAM_TIMEOUT | TOOL_TIMEOUT => "api_error",
        ACCOUNT_COOLDOWN => "rate_limit_error",
        CANCELLED => "api_error",
        _ => "api_error",
    }
}

fn anthropic_type_for(code: &str, status: u16) -> &'static str {
    match code {
        INVALID_REQUEST => "invalid_request_error",
        ACCOUNT_COOLDOWN => "rate_limit_error",
        GATEWAY_DOWN if status == 502 || status == 503 => "api_error",
        _ => match status {
            401 | 403 => "authentication_error",
            429 => "rate_limit_error",
            400 => "invalid_request_error",
            404 => "not_found_error",
            _ => "api_error",
        },
    }
}

/// Classify a connection / I/O failure string (reqwest, OS, proxy).
pub fn classify_transport_error(err: &str) -> LayeredError {
    let lower = err.to_ascii_lowercase();
    if lower.contains("connection refused")
        || lower.contains("econnrefused")
        || lower.contains("connect error")
        || lower.contains("failed to connect")
        || lower.contains("os error 61")
        || lower.contains("os error 111")
    {
        return LayeredError {
            code: GATEWAY_DOWN,
            message: "local gateway or upstream endpoint refused the connection".into(),
            retryable: true,
            hint: "Start GrokGo gateway (port 8787) or ensure accounts are routable. Set NO_PROXY=127.0.0.1,localhost if a system proxy intercepts loopback.".into(),
            status: 502,
        };
    }
    if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("deadline exceeded")
        || lower.contains("operation timed out")
    {
        return LayeredError {
            code: UPSTREAM_TIMEOUT,
            message: "request timed out waiting for upstream".into(),
            retryable: true,
            hint: "Retry; long media jobs may still complete — poll job status or check ~/.grok-go/artifacts/.".into(),
            status: 504,
        };
    }
    if lower.contains("cancel") || lower.contains("aborted") || lower.contains("interrupted") {
        return LayeredError {
            code: CANCELLED,
            message: "request cancelled or aborted".into(),
            retryable: false,
            hint: "Client or gateway cancelled the request; re-send if still needed.".into(),
            status: 499,
        };
    }
    if lower.contains("cooldown") || lower.contains("rate limit") || lower.contains("429") {
        return LayeredError {
            code: ACCOUNT_COOLDOWN,
            message: "account cooling down or rate limited".into(),
            retryable: true,
            hint: "Wait for cooldown or enable another healthy account.".into(),
            status: 429,
        };
    }
    LayeredError {
        code: UPSTREAM_ERROR,
        message: err.to_string(),
        retryable: true,
        hint: "Check GrokGo health, accounts, and proxy settings.".into(),
        status: 502,
    }
}

/// Tool-level wait timeout (MCP long poll) — not a permanent hard failure.
pub fn tool_timeout_error(
    tool: &str,
    job_id: Option<&str>,
    artifacts_hint: Option<&[String]>,
) -> LayeredError {
    let mut hint = format!(
        "Wait timed out for `{tool}`; the job may still be running. Poll GET /v1/videos/{{request_id}} or list ~/.grok-go/artifacts/."
    );
    if let Some(id) = job_id {
        hint.push_str(&format!(" job_id={id}"));
    }
    if let Some(paths) = artifacts_hint {
        if !paths.is_empty() {
            hint.push_str(&format!(" recent_artifacts={}", paths.join(",")));
        }
    }
    LayeredError {
        code: TOOL_TIMEOUT,
        message: format!("MCP tool `{tool}` wait timed out"),
        retryable: true,
        hint,
        status: 504,
    }
}

pub fn tool_failed(tool: &str, message: impl Into<String>) -> LayeredError {
    LayeredError {
        code: TOOL_FAILED,
        message: message.into(),
        retryable: false,
        hint: format!("Tool `{tool}` failed; fix args or check upstream media/chat errors."),
        status: 400,
    }
}

pub fn invalid_request(message: impl Into<String>, hint: impl Into<String>) -> LayeredError {
    LayeredError {
        code: INVALID_REQUEST,
        message: message.into(),
        retryable: false,
        hint: hint.into(),
        status: 400,
    }
}

/// Standardized successful tool result envelope (MCP text + HTTP tools API).
pub fn tool_ok_envelope(tool: &str, summary: impl Into<String>, artifacts: &[String], raw: Option<Value>) -> Value {
    let mut body = json!({
        "ok": true,
        "tool": tool,
        "summary": summary.into(),
        "artifacts": artifacts,
        "error": null,
    });
    if let Some(r) = raw {
        body["raw"] = r;
    }
    body
}

/// MCP content wrapper that embeds the envelope as JSON text (agents parse easily).
pub fn mcp_content_from_envelope(envelope: &Value) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(envelope).unwrap_or_else(|_| envelope.to_string())
        }],
        "isError": envelope.get("ok").and_then(|v| v.as_bool()) == Some(false)
    })
}

/// TOOL_TIMEOUT MCP-friendly result (isError true but retryable + recovery fields).
pub fn tool_timeout_mcp_result(
    tool: &str,
    job_id: Option<&str>,
    artifacts: &[String],
) -> Value {
    let err = tool_timeout_error(tool, job_id, Some(artifacts));
    let mut envelope = err.tool_envelope(tool);
    if let Some(id) = job_id {
        envelope["job_id"] = json!(id);
        envelope["poll"] = json!(format!("/v1/videos/{id}"));
    }
    if !artifacts.is_empty() {
        envelope["artifacts"] = json!(artifacts);
    }
    mcp_content_from_envelope(&envelope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_connection_refused() {
        let e = classify_transport_error("connect ECONNREFUSED 127.0.0.1:8787");
        assert_eq!(e.code, GATEWAY_DOWN);
        assert!(e.retryable);
        assert!(e.hint.to_ascii_lowercase().contains("gateway"));
    }

    #[test]
    fn classifies_timeout() {
        let e = classify_transport_error("error sending request: operation timed out");
        assert_eq!(e.code, UPSTREAM_TIMEOUT);
        assert!(e.retryable);
    }

    #[test]
    fn classifies_cancel() {
        let e = classify_transport_error("request aborted by client");
        assert_eq!(e.code, CANCELLED);
        assert!(!e.retryable);
    }

    #[test]
    fn tool_timeout_has_recovery_fields() {
        let paths = vec!["/tmp/a.mp4".into()];
        let mcp = tool_timeout_mcp_result("video_generate", Some("vid-123"), &paths);
        let text = mcp["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("TOOL_TIMEOUT"));
        assert!(text.contains("retryable"));
        assert!(text.contains("vid-123") || text.contains("/v1/videos/"));
        assert!(mcp["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn openai_body_keeps_outer_error_object() {
        let e = classify_transport_error("connection refused");
        let body = e.openai_body();
        assert!(body.get("error").is_some());
        assert_eq!(body["error"]["code"], GATEWAY_DOWN);
        assert!(body["error"]["retryable"].as_bool().unwrap());
        assert!(!body["error"]["hint"].as_str().unwrap().is_empty());
    }

    #[test]
    fn anthropic_body_embeds_code_and_hint() {
        let e = classify_transport_error("operation timed out");
        let body = e.anthropic_body();
        assert_eq!(body["type"], "error");
        assert_eq!(body["error"]["code"], UPSTREAM_TIMEOUT);
        assert!(body["error"]["retryable"].as_bool().unwrap());
        let msg = body["error"]["message"].as_str().unwrap();
        assert!(msg.contains("UPSTREAM_TIMEOUT"));
        assert!(!body["error"]["hint"].as_str().unwrap().is_empty());
    }
}
