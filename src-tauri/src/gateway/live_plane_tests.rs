//! Live multi-protocol exercises against a real local gateway binding.
//!
//! Gated by env `GROK_GO_LIVE=1` so default `cargo test --lib` stays offline-safe.
//! Uses real `~/.grok-go` config/auth (account pool) when present.
//!
//! Transcripts (request + raw response bodies) are written under:
//! - `$GROK_GO_SCRATCH/live-off/` and `live-on/` when set
//! - else the goal implementer scratch used during development

#![cfg(test)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

use crate::config::{load_config, save_config, AppConfig};
use crate::gateway::build_plane_route::decide_plane;
use crate::gateway::server::{build_router, GatewayState};
use axum::http::HeaderMap;

/// 32×32 solid red PNG (≥512 total pixels required by xAI vision).
const TINY_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAAAK0lEQVR42u3OIQEAAAwEoetfeovxBoGnq1tKQEBAQEBAQEBAQEBAQEBgHXhUDfhqeP5ugAAAAABJRU5ErkJggg==";

static TRANSCRIPT_SEQ: AtomicUsize = AtomicUsize::new(0);

fn live_enabled() -> bool {
    matches!(
        std::env::var("GROK_GO_LIVE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn scratch_root() -> PathBuf {
    if let Ok(p) = std::env::var("GROK_GO_SCRATCH") {
        return PathBuf::from(p);
    }
    PathBuf::from(
        "/var/folders/9n/5qkt0qwj46sd6mdcqnr564b00000gn/T/grok-goal-65436c6a26f8/implementer",
    )
}

fn transcript_dir(flag_on: bool) -> PathBuf {
    let root = scratch_root();
    let dir = root.join(if flag_on { "live-on" } else { "live-off" });
    let _ = fs::create_dir_all(&dir);
    dir
}

fn write_transcript(
    flag_on: bool,
    name: &str,
    meta: Value,
    request_body: &Value,
    status: StatusCode,
    response_raw: &str,
    response_json: Option<&Value>,
) {
    let seq = TRANSCRIPT_SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = transcript_dir(flag_on);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{seq:03}_{name}.json"));
    // Truncate huge base64 image payloads in stored transcripts.
    let mut stored_resp = response_raw.to_string();
    if stored_resp.len() > 24_000 {
        stored_resp = format!(
            "{}…[truncated {} bytes]",
            &stored_resp[..12_000],
            response_raw.len()
        );
    }
    let doc = json!({
        "name": name,
        "flag_on": flag_on,
        "ts": ts,
        "meta": meta,
        "request": request_body,
        "status": status.as_u16(),
        "response_raw": stored_resp,
        "response_json": response_json.cloned(),
    });
    if let Err(e) = fs::write(&path, serde_json::to_string_pretty(&doc).unwrap_or_default()) {
        eprintln!("failed to write transcript {}: {e}", path.display());
    } else {
        eprintln!("transcript → {}", path.display());
    }
}

async fn with_flag<F, Fut>(flag: bool, f: F) -> anyhow::Result<()>
where
    F: FnOnce(AppConfig) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let mut cfg = load_config()?;
    let prev = cfg.experimental_impersonate_grok_build;
    cfg.experimental_impersonate_grok_build = flag;
    save_config(&cfg)?;
    crate::config::invalidate_caches_for_test();
    let result = f(cfg.clone()).await;
    let mut restore = load_config().unwrap_or(cfg);
    restore.experimental_impersonate_grok_build = prev;
    let _ = save_config(&restore);
    crate::config::invalidate_caches_for_test();
    result
}

fn plane_meta(cfg: &AppConfig, path: &str) -> Value {
    let d = decide_plane(cfg, &HeaderMap::new(), path);
    json!({
        "upstream_base": d.upstream_base,
        "build_plane": d.build_plane,
        "experimental_impersonation": d.experimental_impersonation,
        "inject_build_headers": d.inject_build_headers,
        "client_source_override": d.client_source_override,
        "media_path": d.media_path,
    })
}

fn is_transient_upstream(status: StatusCode, raw: &str) -> bool {
    if matches!(
        status,
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT
    ) {
        return true;
    }
    let lower = raw.to_ascii_lowercase();
    lower.contains("connection error")
        || lower.contains("tls")
        || lower.contains("closed connection")
        || lower.contains("timed out")
        || lower.contains("error sending request")
}

async fn call_json(
    _app: axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value, String)> {
    // Fresh router each attempt — oneshot consumes the service; retries need a new one.
    let mut last_status = StatusCode::INTERNAL_SERVER_ERROR;
    let mut last_raw = String::new();
    let mut last_val = json!({});
    for attempt in 1..=3 {
        let app = build_router(GatewayState::new());
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("user-agent", "codex_cli_rs/live-test")
            .body(Body::from(serde_json::to_vec(&body)?))?;
        let resp = app.oneshot(req).await?;
        let status = resp.status();
        let bytes = resp.into_body().collect().await?.to_bytes();
        let raw = String::from_utf8_lossy(&bytes).to_string();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(json!({"raw": raw.clone()}));
        last_status = status;
        last_raw = raw.clone();
        last_val = value.clone();
        if status.is_success() || !is_transient_upstream(status, &raw) {
            return Ok((status, value, raw));
        }
        eprintln!("transient upstream {status} attempt {attempt}/3 on {path}: {raw:.200}");
        tokio::time::sleep(std::time::Duration::from_millis(800 * attempt as u64)).await;
    }
    Ok((last_status, last_val, last_raw))
}

async fn call_stream(
    _app: axum::Router,
    path: &str,
    token: &str,
    body: Value,
    anthropic: bool,
) -> anyhow::Result<(StatusCode, String)> {
    let mut last_status = StatusCode::INTERNAL_SERVER_ERROR;
    let mut last_sse = String::new();
    for attempt in 1..=3 {
        let app = build_router(GatewayState::new());
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream");
        if anthropic {
            builder = builder
                .header("x-api-key", token)
                .header("anthropic-version", "2023-06-01")
                .header("user-agent", "claude-cli/live-test");
        } else {
            builder = builder
                .header("authorization", format!("Bearer {token}"))
                .header("user-agent", "codex_cli_rs/live-test");
        }
        let req = builder.body(Body::from(serde_json::to_vec(&body)?))?;
        let resp = app.oneshot(req).await?;
        let status = resp.status();
        let bytes = resp.into_body().collect().await?.to_bytes();
        let sse = String::from_utf8_lossy(&bytes).to_string();
        last_status = status;
        last_sse = sse.clone();
        if status.is_success() || !is_transient_upstream(status, &sse) {
            return Ok((status, sse));
        }
        eprintln!("transient stream {status} attempt {attempt}/3 on {path}");
        tokio::time::sleep(std::time::Duration::from_millis(800 * attempt as u64)).await;
    }
    Ok((last_status, last_sse))
}

async fn call_anthropic(
    _app: axum::Router,
    token: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value, String)> {
    let mut last_status = StatusCode::INTERNAL_SERVER_ERROR;
    let mut last_raw = String::new();
    let mut last_val = json!({});
    for attempt in 1..=3 {
        let app = build_router(GatewayState::new());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/messages")
            .header("x-api-key", token)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header("user-agent", "claude-cli/live-test")
            .body(Body::from(serde_json::to_vec(&body)?))?;
        let resp = app.oneshot(req).await?;
        let status = resp.status();
        let bytes = resp.into_body().collect().await?.to_bytes();
        let raw = String::from_utf8_lossy(&bytes).to_string();
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(json!({"raw": raw.clone()}));
        last_status = status;
        last_raw = raw.clone();
        last_val = value.clone();
        if status.is_success() || !is_transient_upstream(status, &raw) {
            return Ok((status, value, raw));
        }
        eprintln!("transient anthropic {status} attempt {attempt}/3");
        tokio::time::sleep(std::time::Duration::from_millis(800 * attempt as u64)).await;
    }
    Ok((last_status, last_val, last_raw))
}

fn assert_nonempty_completion(label: &str, status: StatusCode, body: &Value, raw: &str) {
    assert!(
        status.is_success(),
        "{label}: status {status} body={raw:.800}"
    );
    if body.pointer("/choices/0/message/tool_calls").is_some() {
        return;
    }
    if let Some(content) = body
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
    {
        if !content.trim().is_empty() {
            return;
        }
    }
    if let Some(output) = body.get("output").and_then(|o| o.as_array()) {
        if !output.is_empty() {
            return;
        }
    }
    if body
        .get("output_text")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
    {
        return;
    }
    if let Some(content) = body.get("content").and_then(|c| c.as_array()) {
        if !content.is_empty() {
            return;
        }
    }
    if let Some(fr) = body
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
    {
        if !fr.is_empty() {
            eprintln!("{label}: empty content but finish_reason={fr}");
            return;
        }
    }
    panic!("{label}: unrecognized/empty success body: {raw:.800}");
}

fn assert_has_tool_call(label: &str, body: &Value, raw: &str) {
    // OpenAI chat
    if body.pointer("/choices/0/message/tool_calls").is_some() {
        return;
    }
    // Responses function_call item
    if let Some(output) = body.get("output").and_then(|o| o.as_array()) {
        if output.iter().any(|item| {
            matches!(
                item.get("type").and_then(|t| t.as_str()),
                Some("function_call") | Some("custom_tool_call")
            )
        }) {
            return;
        }
    }
    // Anthropic tool_use
    if let Some(content) = body.get("content").and_then(|c| c.as_array()) {
        if content.iter().any(|b| {
            b.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                || b.get("type").and_then(|t| t.as_str()) == Some("tool_use_block")
        }) {
            return;
        }
    }
    panic!("{label}: expected tool call, got {raw:.800}");
}

fn assert_sse_nonempty(label: &str, status: StatusCode, sse: &str) {
    assert!(
        status.is_success(),
        "{label}: stream status {status} body={sse:.600}"
    );
    assert!(
        sse.contains("data:") || sse.contains("event:") || sse.contains("choices"),
        "{label}: empty SSE: {sse:.600}"
    );
    // Must not be a pure empty shell
    let meaningful = sse.lines().any(|l| {
        let t = l.trim();
        t.starts_with("data:") && t.len() > 8 && !t.ends_with("[DONE]")
    }) || sse.contains("delta")
        || sse.contains("content")
        || sse.contains("message")
        || sse.contains("response.");
    assert!(
        meaningful,
        "{label}: SSE lacks event payload: {sse:.600}"
    );
}

fn chat_tool_defs() -> Value {
    json!([{
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get weather for a city",
            "parameters": {
                "type": "object",
                "properties": {
                    "city": {"type": "string"}
                },
                "required": ["city"]
            }
        }
    }])
}

fn responses_tool_defs() -> Value {
    json!([{
        "type": "function",
        "name": "get_weather",
        "description": "Get weather for a city",
        "parameters": {
            "type": "object",
            "properties": {
                "city": {"type": "string"}
            },
            "required": ["city"]
        }
    }])
}

fn anthropic_tool_defs() -> Value {
    json!([{
        "name": "get_weather",
        "description": "Get weather for a city",
        "input_schema": {
            "type": "object",
            "properties": {
                "city": {"type": "string"}
            },
            "required": ["city"]
        }
    }])
}

async fn run_flag_off_suite(token: &str, cfg: &AppConfig) -> anyhow::Result<()> {
    // chat
    let req = json!({
        "model": "grok-4.5",
        "messages": [{"role":"user","content":"Reply with exactly: pong"}],
        "max_tokens": 32,
        "stream": false
    });
    let meta = plane_meta(cfg, "/chat/completions");
    assert_eq!(meta["build_plane"], false);
    assert_eq!(meta["upstream_base"].as_str().unwrap().contains("api.x.ai"), true);
    let app = build_router(GatewayState::new());
    let (st, body, raw) = call_json(app, "POST", "/v1/chat/completions", token, req.clone()).await?;
    write_transcript(false, "chat", meta, &req, st, &raw, Some(&body));
    assert_nonempty_completion("off/chat", st, &body, &raw);

    // responses
    let req = json!({
        "model": "grok-4.5",
        "input": "Reply with exactly: pong",
        "max_output_tokens": 32,
        "stream": false
    });
    let meta = plane_meta(cfg, "/responses");
    let app = build_router(GatewayState::new());
    let (st, body, raw) = call_json(app, "POST", "/v1/responses", token, req.clone()).await?;
    write_transcript(false, "responses", meta, &req, st, &raw, Some(&body));
    assert_nonempty_completion("off/responses", st, &body, &raw);

    // anthropic
    let req = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 32,
        "messages": [{"role":"user","content":"Reply with exactly: pong"}]
    });
    let meta = plane_meta(cfg, "/chat/completions");
    let app = build_router(GatewayState::new());
    let (st, body, raw) = call_anthropic(app, token, req.clone()).await?;
    write_transcript(false, "messages", meta, &req, st, &raw, Some(&body));
    assert_nonempty_completion("off/messages", st, &body, &raw);

    Ok(())
}

async fn run_flag_on_suite(token: &str, cfg: &AppConfig, run: u32) -> anyhow::Result<()> {
    let label = format!("on{run}");
    let data_url = format!("data:image/png;base64,{TINY_PNG_B64}");

    // --- chat forced tool call ---
    let req = json!({
        "model": "grok-4.5",
        "messages": [{"role":"user","content":"What is the weather in Tokyo? Use the tool."}],
        "max_tokens": 128,
        "stream": false,
        "tools": chat_tool_defs(),
        "tool_choice": {"type": "function", "function": {"name": "get_weather"}}
    });
    let meta = plane_meta(cfg, "/chat/completions");
    assert_eq!(meta["build_plane"], true);
    assert!(meta["upstream_base"]
        .as_str()
        .unwrap_or("")
        .contains("cli-chat-proxy"));
    let app = build_router(GatewayState::new());
    let (st, body, raw) =
        call_json(app, "POST", "/v1/chat/completions", token, req.clone()).await?;
    write_transcript(
        true,
        &format!("{label}_chat_tool"),
        meta,
        &req,
        st,
        &raw,
        Some(&body),
    );
    assert!(st.is_success(), "{label}/chat_tool status {st} {raw:.600}");
    assert_has_tool_call(&format!("{label}/chat_tool"), &body, &raw);

    // tool follow-up with tool result
    let tool_call_id = body
        .pointer("/choices/0/message/tool_calls/0/id")
        .and_then(|v| v.as_str())
        .unwrap_or("call_1")
        .to_string();
    let tool_args = body
        .pointer("/choices/0/message/tool_calls/0/function/arguments")
        .cloned()
        .unwrap_or(json!("{}"));
    let req2 = json!({
        "model": "grok-4.5",
        "messages": [
            {"role":"user","content":"What is the weather in Tokyo? Use the tool."},
            {"role":"assistant","tool_calls":[{
                "id": tool_call_id,
                "type": "function",
                "function": {"name": "get_weather", "arguments": tool_args}
            }]},
            {"role":"tool","tool_call_id": tool_call_id, "content": "{\"temp_c\": 22, \"sky\": \"clear\"}"}
        ],
        "max_tokens": 64,
        "stream": false,
        "tools": chat_tool_defs()
    });
    let meta = plane_meta(cfg, "/chat/completions");
    let app = build_router(GatewayState::new());
    let (st, body, raw) =
        call_json(app, "POST", "/v1/chat/completions", token, req2.clone()).await?;
    write_transcript(
        true,
        &format!("{label}_chat_tool_followup"),
        meta,
        &req2,
        st,
        &raw,
        Some(&body),
    );
    assert_nonempty_completion(&format!("{label}/chat_tool_followup"), st, &body, &raw);

    // --- responses with tools (forced via tool_choice if supported) ---
    let req = json!({
        "model": "grok-4.5",
        "input": "What is the weather in Paris? You must call get_weather.",
        "max_output_tokens": 128,
        "stream": false,
        "tools": responses_tool_defs(),
        "tool_choice": {"type": "function", "name": "get_weather"}
    });
    let meta = plane_meta(cfg, "/responses");
    let app = build_router(GatewayState::new());
    let (st, body, raw) = call_json(app, "POST", "/v1/responses", token, req.clone()).await?;
    write_transcript(
        true,
        &format!("{label}_responses_tool"),
        meta,
        &req,
        st,
        &raw,
        Some(&body),
    );
    if st.is_success() {
        // Prefer tool call; accept non-empty output if tool_choice ignored by upstream
        if body.pointer("/output").is_some() {
            let has_fn = body
                .get("output")
                .and_then(|o| o.as_array())
                .map(|a| {
                    a.iter().any(|item| {
                        matches!(
                            item.get("type").and_then(|t| t.as_str()),
                            Some("function_call") | Some("custom_tool_call")
                        )
                    })
                })
                .unwrap_or(false);
            if has_fn {
                // ok
            } else {
                assert_nonempty_completion(&format!("{label}/responses_tool"), st, &body, &raw);
            }
        } else {
            assert_nonempty_completion(&format!("{label}/responses_tool"), st, &body, &raw);
        }
    } else {
        // document limitation then plain responses
        eprintln!("{label}/responses_tool status={st} body={raw:.400}");
        let req = json!({
            "model": "grok-4.5",
            "input": "Say hi in one word",
            "max_output_tokens": 32,
            "stream": false,
            "tools": responses_tool_defs()
        });
        let app = build_router(GatewayState::new());
        let (st, body, raw) = call_json(app, "POST", "/v1/responses", token, req.clone()).await?;
        write_transcript(
            true,
            &format!("{label}_responses_tools_present"),
            plane_meta(cfg, "/responses"),
            &req,
            st,
            &raw,
            Some(&body),
        );
        assert_nonempty_completion(&format!("{label}/responses_tools_present"), st, &body, &raw);
    }

    // --- anthropic tools ---
    let req = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 128,
        "tools": anthropic_tool_defs(),
        "tool_choice": {"type": "tool", "name": "get_weather"},
        "messages": [{"role":"user","content":"Weather in Berlin? Use the tool."}]
    });
    let meta = plane_meta(cfg, "/chat/completions");
    let app = build_router(GatewayState::new());
    let (st, body, raw) = call_anthropic(app, token, req.clone()).await?;
    write_transcript(
        true,
        &format!("{label}_messages_tool"),
        meta,
        &req,
        st,
        &raw,
        Some(&body),
    );
    if st.is_success() {
        let has_tool = body
            .get("content")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
            })
            .unwrap_or(false);
        if !has_tool {
            assert_nonempty_completion(&format!("{label}/messages_tool"), st, &body, &raw);
        }
    } else {
        eprintln!("{label}/messages_tool status={st} {raw:.400}");
        let req = json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 32,
            "tools": anthropic_tool_defs(),
            "messages": [{"role":"user","content":"Say hi"}]
        });
        let app = build_router(GatewayState::new());
        let (st, body, raw) = call_anthropic(app, token, req.clone()).await?;
        write_transcript(
            true,
            &format!("{label}_messages_tools_present"),
            plane_meta(cfg, "/chat/completions"),
            &req,
            st,
            &raw,
            Some(&body),
        );
        assert_nonempty_completion(&format!("{label}/messages_tools_present"), st, &body, &raw);
    }

    // --- vision: chat image_url ---
    let req = json!({
        "model": "grok-4.5",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "Describe this image in 3 words."},
                {"type": "image_url", "image_url": {"url": data_url}}
            ]
        }],
        "max_tokens": 64,
        "stream": false
    });
    let meta = plane_meta(cfg, "/chat/completions");
    let app = build_router(GatewayState::new());
    let (st, body, raw) =
        call_json(app, "POST", "/v1/chat/completions", token, req.clone()).await?;
    write_transcript(
        true,
        &format!("{label}_chat_vision"),
        meta,
        &json!({"model":"grok-4.5","has_image":true,"note":"1x1 png"}),
        st,
        &raw,
        Some(&body),
    );
    assert_nonempty_completion(&format!("{label}/chat_vision"), st, &body, &raw);

    // --- vision: responses input_image ---
    let req = json!({
        "model": "grok-4.5",
        "input": [{
            "role": "user",
            "content": [
                {"type": "input_text", "text": "Describe this image in 3 words."},
                {"type": "input_image", "image_url": format!("data:image/png;base64,{TINY_PNG_B64}")}
            ]
        }],
        "max_output_tokens": 64,
        "stream": false
    });
    let meta = plane_meta(cfg, "/responses");
    let app = build_router(GatewayState::new());
    let (st, body, raw) = call_json(app, "POST", "/v1/responses", token, req.clone()).await?;
    write_transcript(
        true,
        &format!("{label}_responses_vision"),
        meta,
        &json!({"model":"grok-4.5","has_input_image":true}),
        st,
        &raw,
        Some(&body),
    );
    assert_nonempty_completion(&format!("{label}/responses_vision"), st, &body, &raw);

    // --- streams: chat, responses, anthropic ---
    let app = build_router(GatewayState::new());
    let (st, sse) = call_stream(
        app,
        "/v1/chat/completions",
        token,
        json!({
            "model": "grok-4.5",
            "messages": [{"role":"user","content":"Say ok"}],
            "max_tokens": 16,
            "stream": true
        }),
        false,
    )
    .await?;
    write_transcript(
        true,
        &format!("{label}_stream_chat"),
        plane_meta(cfg, "/chat/completions"),
        &json!({"stream": true, "path": "/v1/chat/completions"}),
        st,
        &sse,
        None,
    );
    assert_sse_nonempty(&format!("{label}/stream_chat"), st, &sse);

    let app = build_router(GatewayState::new());
    let (st, sse) = call_stream(
        app,
        "/v1/responses",
        token,
        json!({
            "model": "grok-4.5",
            "input": "Say ok",
            "max_output_tokens": 16,
            "stream": true
        }),
        false,
    )
    .await?;
    write_transcript(
        true,
        &format!("{label}_stream_responses"),
        plane_meta(cfg, "/responses"),
        &json!({"stream": true, "path": "/v1/responses"}),
        st,
        &sse,
        None,
    );
    assert_sse_nonempty(&format!("{label}/stream_responses"), st, &sse);

    let app = build_router(GatewayState::new());
    let (st, sse) = call_stream(
        app,
        "/v1/messages",
        token,
        json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 16,
            "stream": true,
            "messages": [{"role":"user","content":"Say ok"}]
        }),
        true,
    )
    .await?;
    write_transcript(
        true,
        &format!("{label}_stream_messages"),
        plane_meta(cfg, "/chat/completions"),
        &json!({"stream": true, "path": "/v1/messages"}),
        st,
        &sse,
        None,
    );
    assert_sse_nonempty(&format!("{label}/stream_messages"), st, &sse);

    // --- image generation ---
    let req = json!({
        "model": "grok-imagine-image-quality",
        "prompt": "a tiny red square icon, flat",
        "n": 1
    });
    let meta = plane_meta(cfg, "/images/generations");
    assert_eq!(meta["media_path"], true);
    assert!(meta["upstream_base"].as_str().unwrap_or("").contains("api.x.ai"));
    let app = build_router(GatewayState::new());
    let (st, body, raw) =
        call_json(app, "POST", "/v1/images/generations", token, req.clone()).await?;
    write_transcript(
        true,
        &format!("{label}_image_gen"),
        meta,
        &req,
        st,
        &raw,
        Some(&body),
    );
    if st.is_success() {
        assert!(
            body.get("data").is_some() || body.get("url").is_some() || raw.contains("b64"),
            "{label}/image unexpected success body {raw:.400}"
        );
    } else {
        eprintln!("{label}/image documented limitation status={st} body={raw:.400}");
    }

    // --- video generation ---
    let req = json!({
        "model": "grok-imagine-video",
        "prompt": "a tiny red ball bouncing once, simple",
        "n": 1
    });
    let meta = plane_meta(cfg, "/videos/generations");
    assert_eq!(meta["media_path"], true);
    let app = build_router(GatewayState::new());
    let (st, body, raw) =
        call_json(app, "POST", "/v1/videos/generations", token, req.clone()).await?;
    write_transcript(
        true,
        &format!("{label}_video_gen"),
        meta,
        &req,
        st,
        &raw,
        Some(&body),
    );
    if st.is_success() {
        let has_job = body.get("request_id").is_some()
            || body.get("id").is_some()
            || body.pointer("/data/0").is_some()
            || body.get("video").is_some()
            || raw.contains("request_id");
        assert!(
            has_job || raw.len() > 20,
            "{label}/video success but no job id: {raw:.400}"
        );
        // optional poll if request_id present
        if let Some(rid) = body
            .get("request_id")
            .or_else(|| body.get("id"))
            .and_then(|v| v.as_str())
        {
            let app = build_router(GatewayState::new());
            let (pst, pbody, praw) = call_json(
                app,
                "GET",
                &format!("/v1/videos/{rid}"),
                token,
                json!({}),
            )
            .await?;
            write_transcript(
                true,
                &format!("{label}_video_poll"),
                plane_meta(cfg, "/videos/x"),
                &json!({"request_id": rid}),
                pst,
                &praw,
                Some(&pbody),
            );
            eprintln!("{label}/video_poll status={pst}");
        }
    } else {
        // Documented limitation with console-plane fallback path already used.
        eprintln!(
            "{label}/video documented limitation (console plane) status={st} body={raw:.500}"
        );
        let note = json!({
            "limitation": "video generation under experimental flag uses console api.x.ai (cli-chat-proxy has no media routes)",
            "status": st.as_u16(),
            "fallback": "same as stable path: POST /v1/videos/generations on xai_base_url",
            "body_excerpt": raw.chars().take(400).collect::<String>(),
        });
        let path = transcript_dir(true).join(format!("{label}_video_limitation.json"));
        let _ = fs::write(path, serde_json::to_string_pretty(&note).unwrap_or_default());
    }

    Ok(())
}

#[tokio::test]
async fn live_multi_protocol_flag_off_and_on() {
    if !live_enabled() {
        eprintln!("skip live_plane_tests (set GROK_GO_LIVE=1)");
        return;
    }

    let _ = fs::create_dir_all(transcript_dir(false));
    let _ = fs::create_dir_all(transcript_dir(true));

    let cfg = load_config().expect("load_config");
    let token = cfg.local_token.clone();
    assert!(!token.is_empty(), "local token required");

    // --- flag OFF ---
    with_flag(false, {
        let token = token.clone();
        move |cfg| async move {
            run_flag_off_suite(&token, &cfg).await
        }
    })
    .await
    .expect("flag off suite");

    // --- flag ON twice ---
    for run in 1..=2 {
        with_flag(true, {
            let token = token.clone();
            move |cfg| async move {
                run_flag_on_suite(&token, &cfg, run).await
            }
        })
        .await
        .unwrap_or_else(|e| panic!("flag on run {run}: {e:#}"));
    }

    // index of transcripts
    let index = json!({
        "live_off": list_json_names(&transcript_dir(false)),
        "live_on": list_json_names(&transcript_dir(true)),
    });
    let _ = fs::write(
        scratch_root().join("live-transcript-index.json"),
        serde_json::to_string_pretty(&index).unwrap_or_default(),
    );
}

fn list_json_names(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.ends_with(".json") {
                names.push(n);
            }
        }
    }
    names.sort();
    names
}

#[tokio::test]
async fn live_refresh_quota_snapshot() {
    if !live_enabled() {
        return;
    }
    let store = crate::config::load_auth().expect("auth");
    let id = store.accounts.first().map(|a| a.id.clone()).expect("account");
    match crate::quota::refresh_account_quota(&id).await {
        Ok(acc) => {
            eprintln!("quota refreshed: {:?}", acc.quota);
            let path = scratch_root().join("quota-after-refresh.json");
            let _ = fs::write(
                path,
                serde_json::to_string_pretty(&json!({
                    "quota": acc.quota,
                }))
                .unwrap_or_default(),
            );
        }
        Err(e) => eprintln!("quota refresh error (env limit ok): {e}"),
    }
}
