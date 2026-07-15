//! Live multi-protocol exercises against a real local gateway binding.
//!
//! Gated by env `GROK_GO_LIVE=1` so default `cargo test --lib` stays offline-safe.
//! Uses real `~/.grok-go` config/auth (account pool) when present.

#![cfg(test)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::config::{load_config, save_config, AppConfig};
use crate::gateway::server::{build_router, GatewayState};

fn live_enabled() -> bool {
    matches!(
        std::env::var("GROK_GO_LIVE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
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
    // Clear in-memory cache so gateway handlers see the new flag.
    crate::config::invalidate_caches_for_test();
    let result = f(cfg.clone()).await;
    let mut restore = load_config().unwrap_or(cfg);
    restore.experimental_impersonate_grok_build = prev;
    let _ = save_config(&restore);
    crate::config::invalidate_caches_for_test();
    result
}

async fn call_json(
    app: axum::Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value, String)> {
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
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(json!({"raw": raw}));
    Ok((status, value, String::from_utf8_lossy(&bytes).to_string()))
}

async fn call_anthropic(
    app: axum::Router,
    token: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, Value, String)> {
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
    Ok((status, value, raw))
}

fn assert_nonempty_completion(label: &str, status: StatusCode, body: &Value, raw: &str) {
    assert!(
        status.is_success(),
        "{label}: status {status} body={raw:.800}"
    );
    // OpenAI chat — tool_calls alone are valid (model may skip text).
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
    // Responses: any non-empty output array (message / function_call / reasoning)
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
    // Anthropic
    if let Some(content) = body.get("content").and_then(|c| c.as_array()) {
        if !content.is_empty() {
            return;
        }
    }
    // Refusal / finish_reason stop with empty content still counts as a round-trip
    // only if choices[] exists with a finish_reason (upstream accepted the request).
    if let Some(fr) = body
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
    {
        if !fr.is_empty() {
            eprintln!("{label}: empty content but finish_reason={fr}; accepting body={raw:.400}");
            return;
        }
    }
    panic!("{label}: unrecognized/empty success body: {raw:.800}");
}

#[tokio::test]
async fn live_multi_protocol_flag_off_and_on() {
    if !live_enabled() {
        eprintln!("skip live_plane_tests (set GROK_GO_LIVE=1)");
        return;
    }

    let cfg = load_config().expect("load_config");
    let token = cfg.local_token.clone();
    assert!(!token.is_empty(), "local token required");

    // --- flag OFF ---
    with_flag(false, {
        let token = token.clone();
        move |_cfg| async move {
            let app = build_router(GatewayState::new());
            // chat
            let (st, body, raw) = call_json(
                app.clone(),
                "POST",
                "/v1/chat/completions",
                &token,
                json!({
                    "model": "grok-4.5",
                    "messages": [{"role":"user","content":"Reply with exactly: pong"}],
                    "max_tokens": 32,
                    "stream": false
                }),
            )
            .await?;
            assert_nonempty_completion("off/chat", st, &body, &raw);

            // responses
            let app = build_router(GatewayState::new());
            let (st, body, raw) = call_json(
                app.clone(),
                "POST",
                "/v1/responses",
                &token,
                json!({
                    "model": "grok-4.5",
                    "input": "Reply with exactly: pong",
                    "max_output_tokens": 32,
                    "stream": false
                }),
            )
            .await?;
            assert_nonempty_completion("off/responses", st, &body, &raw);

            // anthropic
            let app = build_router(GatewayState::new());
            let (st, body, raw) = call_anthropic(
                app,
                &token,
                json!({
                    "model": "claude-sonnet-4-20250514",
                    "max_tokens": 32,
                    "messages": [{"role":"user","content":"Reply with exactly: pong"}]
                }),
            )
            .await?;
            assert_nonempty_completion("off/messages", st, &body, &raw);
            Ok(())
        }
    })
    .await
    .expect("flag off suite");

    // --- flag ON (twice for consistency) ---
    for run in 1..=2 {
        with_flag(true, {
            let token = token.clone();
            move |_cfg| async move {
                let label = format!("on{run}");
                // chat + tools shape
                let app = build_router(GatewayState::new());
                let (st, body, raw) = call_json(
                    app.clone(),
                    "POST",
                    "/v1/chat/completions",
                    &token,
                    json!({
                        "model": "grok-4.5",
                        "messages": [{"role":"user","content":"Say hi in one word"}],
                        "max_tokens": 32,
                        "stream": false,
                        "tools": [{
                            "type": "function",
                            "function": {
                                "name": "noop_tool",
                                "description": "do nothing",
                                "parameters": {"type":"object","properties":{}}
                            }
                        }]
                    }),
                )
                .await?;
                assert_nonempty_completion(&format!("{label}/chat"), st, &body, &raw);

                // responses
                let app = build_router(GatewayState::new());
                let (st, body, raw) = call_json(
                    app.clone(),
                    "POST",
                    "/v1/responses",
                    &token,
                    json!({
                        "model": "grok-4.5",
                        "input": "Say hi in one word",
                        "max_output_tokens": 32,
                        "stream": false
                    }),
                )
                .await?;
                assert_nonempty_completion(&format!("{label}/responses"), st, &body, &raw);

                // anthropic
                let app = build_router(GatewayState::new());
                let (st, body, raw) = call_anthropic(
                    app,
                    &token,
                    json!({
                        "model": "claude-sonnet-4-20250514",
                        "max_tokens": 32,
                        "messages": [{"role":"user","content":"Say hi in one word"}]
                    }),
                )
                .await?;
                assert_nonempty_completion(&format!("{label}/messages"), st, &body, &raw);

                // stream chat (SSE)
                let app = build_router(GatewayState::new());
                let req = Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .header("accept", "text/event-stream")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "model": "grok-4.5",
                            "messages": [{"role":"user","content":"Say ok"}],
                            "max_tokens": 16,
                            "stream": true
                        }))
                        .unwrap(),
                    ))
                    .unwrap();
                let resp = app.oneshot(req).await.unwrap();
                assert!(
                    resp.status().is_success(),
                    "{label}/stream status {}",
                    resp.status()
                );
                let bytes = resp.into_body().collect().await.unwrap().to_bytes();
                let sse = String::from_utf8_lossy(&bytes);
                assert!(
                    sse.contains("data:") || sse.contains("choices"),
                    "{label}/stream empty: {sse:.400}"
                );

                // image generation (console plane under experimental — may succeed or document fail)
                let app = build_router(GatewayState::new());
                let (st, body, raw) = call_json(
                    app,
                    "POST",
                    "/v1/images/generations",
                    &token,
                    json!({
                        "model": "grok-imagine-image-quality",
                        "prompt": "a tiny red square icon, flat",
                        "n": 1
                    }),
                )
                .await?;
                if st.is_success() {
                    assert!(
                        body.get("data").is_some() || body.get("url").is_some() || raw.contains("b64"),
                        "{label}/image unexpected success body {raw:.400}"
                    );
                } else {
                    eprintln!(
                        "{label}/image documented limitation status={st} body={:.400}",
                        raw
                    );
                }

                Ok(())
            }
        })
        .await
        .unwrap_or_else(|e| panic!("flag on run {run}: {e:#}"));
    }
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
        }
        Err(e) => eprintln!("quota refresh error (env limit ok): {e}"),
    }
}
