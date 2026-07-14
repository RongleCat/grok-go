use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderName, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use chrono::Utc;
use futures_util::StreamExt;
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use crate::auth::{
    apply_rate_limit_headers, ensure_fresh_token, mark_failure_kind, mark_success, retry_after_secs,
    FailureKind,
};
use crate::config::{load_auth, load_config, resolve_model, AppConfig};
use crate::error::{AppError, AppResult};
use crate::gateway::empty_completion::{
    build_empty_completion_retry_request, extract_completed_response_from_sse, is_responses_path,
    recovery_quality_score, should_retry_premature_agent_stop, synthesize_forced_tool_response,
    SSE_BUFFER_LIMIT,
};
use crate::gateway::image_bridge::{
    collect_image_gen_calls, fulfill_image_gen_call, inject_image_generation_calls,
    MAX_IMAGE_TOOL_ROUNDS,
};
use crate::gateway::payload_optimize::{
    offload_large_text_blobs, optimize_responses_payload, OFFLOAD_TEXT_MIN,
};
use crate::gateway::sanitize::{
    is_compaction_blob_error, is_model_input_error, rewrite_responses_payload,
    rewrite_sse_data_line, sanitize_responses_request, sanitize_responses_request_ex,
    strip_opaque_context,
};
use crate::http_client::build_http_client;
use crate::concurrency::AccountPermit;
use crate::router::{
    pick_account_decision_cap, replace_account_tokens, routable_account_count_cap,
    touch_account_cache, MediaCapability,
};
use crate::session_affinity;
use crate::usage::{enqueue_request_log, estimate_cost, RequestLog};

#[derive(Clone)]
pub struct ProxyContext {
    /// Rebuildable shared client (connection pool). Clone of `reqwest::Client` is cheap (Arc).
    client: Arc<RwLock<reqwest::Client>>,
}

impl ProxyContext {
    pub fn new() -> Self {
        let config = load_config().unwrap_or_default();
        let client = build_http_client(&config).unwrap_or_else(|err| {
            tracing::error!("HTTP client build failed ({err}); using plain defaults");
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .no_proxy()
                .build()
                .expect("fallback reqwest client")
        });
        Self {
            client: Arc::new(RwLock::new(client)),
        }
    }

    pub fn client(&self) -> reqwest::Client {
        self.client.read().clone()
    }

    /// Rebuild after proxy / network settings change so new pools pick up the config.
    pub fn rebuild_client(&self, config: &AppConfig) -> AppResult<()> {
        let next = build_http_client(config)?;
        *self.client.write() = next;
        Ok(())
    }
}

pub async fn authorize_request(headers: &HeaderMap, config: &AppConfig) -> Result<(), Response> {
    if !config.require_token {
        return Ok(());
    }
    let expected = config.local_token.trim();
    if expected.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            crate::gateway::anthropic::anthropic_error_body(
                "authentication_error",
                "local token is empty; set requireToken=false or configure localToken",
            )
            .to_string(),
        )
            .into_response());
    }

    // Bearer (OpenAI / Codex / Grok Build)
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let bearer = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))
        .unwrap_or(auth)
        .trim();
    if !bearer.is_empty() && bearer == expected {
        return Ok(());
    }

    // Anthropic / Claude Code: x-api-key (and occasional bare Authorization without Bearer)
    let x_api_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .unwrap_or("");
    if !x_api_key.is_empty() && x_api_key == expected {
        return Ok(());
    }
    if !bearer.is_empty() && bearer == expected {
        return Ok(());
    }

    // Grok Build native plane sends the user's session OAuth token (auth.json).
    // GrokGo replaces it with a pool account token upstream; accept any non-empty
    // bearer when build-plane markers are present (local multi-account routing).
    if is_grok_build_plane(headers) && !bearer.is_empty() {
        return Ok(());
    }
    Err((
        StatusCode::UNAUTHORIZED,
        crate::gateway::anthropic::anthropic_error_body(
            "authentication_error",
            "invalid local token (use Authorization: Bearer or x-api-key)",
        )
        .to_string(),
    )
        .into_response())
}

/// True when the client is Grok Build CLI (session / SuperGrok credits plane).
///
/// Markers from official docs: `X-XAI-Token-Auth: xai-grok-cli`,
/// `x-grok-model-override`, and Grok CLI user-agents.
pub fn is_grok_build_plane(headers: &HeaderMap) -> bool {
    if let Some(v) = headers
        .get("x-xai-token-auth")
        .and_then(|v| v.to_str().ok())
    {
        let lower = v.to_ascii_lowercase();
        if lower.contains("grok-cli") || lower.contains("xai-grok") {
            return true;
        }
    }
    if headers.get("x-grok-model-override").is_some() {
        return true;
    }
    if let Some(ua) = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
    {
        let lower = ua.to_ascii_lowercase();
        if lower.contains("grok-cli")
            || lower.contains("grok-build")
            || lower.contains("xai-grok-shell")
            || lower.contains("xai-grok")
        {
            return true;
        }
    }
    false
}

/// Pick upstream base: Grok Build → cli-chat-proxy; otherwise console API.
pub fn resolve_upstream_base(config: &AppConfig, headers: &HeaderMap) -> String {
    if is_grok_build_plane(headers) {
        config.cli_chat_proxy_base_url.trim_end_matches('/').to_string()
    } else {
        config.xai_base_url.trim_end_matches('/').to_string()
    }
}

/// Minimum cli-chat-proxy client version (426 Upgrade Required below this).
const DEFAULT_GROK_CLIENT_VERSION: &str = "0.2.101";

/// Whether a client header should be forwarded on the Grok Build / cli-chat-proxy plane.
///
/// Includes all `x-grok-*` / `x-xai-*` plus a few non-prefixed headers the official
/// CLI sends (`User-Agent`, `x-email`, `x-models-etag`, tracing). Authorization is
/// rewritten to a pool token and must never pass through.
fn should_passthrough_build_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    // Hop-by-hop / rewritten by us.
    if matches!(
        lower.as_str(),
        "authorization"
            | "host"
            | "content-length"
            | "content-type"
            | "accept"
            | "connection"
            | "transfer-encoding"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "upgrade"
            | "cookie"
            | "set-cookie"
    ) {
        return false;
    }
    lower.starts_with("x-grok-")
        || lower.starts_with("x-xai-")
        || matches!(
            lower.as_str(),
            "user-agent"
                | "x-email"
                | "x-models-etag"
                | "x-authenticate"
                | "accept-language"
                | "x-request-id"
                | "traceparent"
                | "tracestate"
                | "baggage"
        )
}

/// Client headers that must reach cli-chat-proxy for native Grok Build behaviour.
fn collect_build_plane_passthrough_headers(headers: &HeaderMap) -> Vec<(HeaderName, String)> {
    let mut out = Vec::new();
    for (name, value) in headers.iter() {
        if !should_passthrough_build_header(name.as_str()) {
            continue;
        }
        if let Ok(v) = value.to_str() {
            if !v.is_empty() {
                out.push((name.clone(), v.to_string()));
            }
        }
    }
    // Ensure the canonical CLI marker is always present on the build plane.
    if !out
        .iter()
        .any(|(n, _)| n.as_str().eq_ignore_ascii_case("x-xai-token-auth"))
    {
        out.push((
            HeaderName::from_static("x-xai-token-auth"),
            "xai-grok-cli".into(),
        ));
    }
    // cli-chat-proxy rejects missing / outdated versions with 426.
    // Prefer client value; fall back to a known-good default so curl / old clients work.
    if !out
        .iter()
        .any(|(n, v)| {
            n.as_str().eq_ignore_ascii_case("x-grok-client-version") && !v.trim().is_empty()
        })
    {
        out.push((
            HeaderName::from_static("x-grok-client-version"),
            DEFAULT_GROK_CLIENT_VERSION.into(),
        ));
    }
    // Sensible User-Agent when callers (curl / tests) omit it.
    if !out
        .iter()
        .any(|(n, v)| n.as_str().eq_ignore_ascii_case("user-agent") && !v.trim().is_empty())
    {
        out.push((
            HeaderName::from_static("user-agent"),
            format!("xai-grok-shell/{DEFAULT_GROK_CLIENT_VERSION} (grok-build)"),
        ));
    }
    out
}

pub async fn proxy_json(
    ctx: &ProxyContext,
    method: Method,
    path: &str,
    headers: HeaderMap,
    body: Bytes,
    client_source: &str,
) -> Response {
    match proxy_json_inner(ctx, method, path, headers, body, client_source).await {
        Ok(resp) => resp,
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            json!({"error": {"message": err.to_string(), "type": "proxy_error"}}).to_string(),
        )
            .into_response(),
    }
}

/// Claude Code / Anthropic Messages API → xAI Chat Completions → Anthropic response.
///
/// Endpoint: `POST /v1/messages` (and streaming via `stream: true`).
pub async fn proxy_anthropic_messages(
    ctx: &ProxyContext,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match proxy_anthropic_messages_inner(ctx, headers, body).await {
        Ok(resp) => resp,
        Err(err) => {
            let status = StatusCode::BAD_GATEWAY;
            let body = crate::gateway::anthropic::anthropic_error_body("api_error", err.to_string());
            (status, body.to_string()).into_response()
        }
    }
}

/// Claude Code token preflight: `POST /v1/messages/count_tokens`.
pub async fn proxy_anthropic_count_tokens(
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let config = match load_config() {
        Ok(c) => c,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::gateway::anthropic::anthropic_error_body("api_error", err.to_string())
                    .to_string(),
            )
                .into_response();
        }
    };
    if let Err(resp) = authorize_request(&headers, &config).await {
        return resp;
    }
    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                crate::gateway::anthropic::anthropic_error_body(
                    "invalid_request_error",
                    format!("invalid JSON: {err}"),
                )
                .to_string(),
            )
                .into_response();
        }
    };
    let input_tokens = crate::gateway::anthropic::estimate_token_count(&value);
    Json(json!({
        "input_tokens": input_tokens
    }))
    .into_response()
}

async fn proxy_anthropic_messages_inner(
    ctx: &ProxyContext,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<Response> {
    use crate::gateway::anthropic::{
        anthropic_to_openai_chat, map_client_model, openai_chat_to_anthropic,
        openai_error_to_anthropic, OpenAiToAnthropicSse,
    };
    use crate::gateway::payload_optimize::optimize_responses_payload;
    use futures_util::stream;

    let config = load_config()?;
    authorize_request(&headers, &config)
        .await
        .map_err(|resp| AppError::msg(format!("unauthorized: {}", response_to_text(resp))))?;

    let anthropic_req: Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::msg(format!("invalid JSON body: {e}")))?;

    let converted = anthropic_to_openai_chat(&anthropic_req)
        .map_err(|e| AppError::msg(format!("anthropic request convert: {e}")))?;

    let (mapped_model, map_reason) =
        map_client_model(&converted.requested_model, &config.default_model);
    let (resolved_model, resolve_reason) = resolve_model(&config, &mapped_model);
    let mapping_reason = format!("anthropic:{map_reason}:{resolve_reason}");

    let mut chat_body = converted.body;
    chat_body["model"] = Value::String(resolved_model.clone());

    // Reuse multi-turn payload shrink (images / huge tool outputs).
    let opt = optimize_responses_payload(&mut chat_body);
    if opt.modified {
        opt.log_summary("/v1/messages");
    }

    let client_wants_stream = converted.stream
        || headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("text/event-stream"))
            .unwrap_or(false);
    if client_wants_stream {
        chat_body["stream"] = Value::Bool(true);
        if chat_body.get("stream_options").is_none() {
            chat_body["stream_options"] = json!({"include_usage": true});
        }
    }

    let outbound_body = Bytes::from(serde_json::to_vec(&chat_body)?);
    let session_key = session_affinity::extract_session_key(&headers, Some(&chat_body));
    let http = ctx.client();
    let upstream_base = config.xai_base_url.trim_end_matches('/');
    let url = format!("{upstream_base}/chat/completions");

    let build_upstream = {
        let url = url.clone();
        move |client: &reqwest::Client, token: &str, body: Bytes| {
            client
                .post(&url)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::ACCEPT,
                    if client_wants_stream {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                )
                .body(body)
        }
    };

    let started = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (mut account, _token, upstream) = send_with_account_failover(
        &config,
        &http,
        &url,
        &build_upstream,
        outbound_body,
        session_key.as_deref(),
        false, // Files offload is Responses-oriented; skip for Anthropic chat path
    )
    .await?;

    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    apply_rate_limit_headers(&mut account, &upstream_headers);
    let is_stream = status.is_success()
        && upstream_headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("text/event-stream") || v.contains("stream"))
            .unwrap_or(false);

    let latency_ms = started.elapsed().as_millis() as u64;
    let path = "/v1/messages";
    let client_source = "anthropic-messages";

    if is_stream {
        if status.is_success() {
            mark_success(&mut account);
            touch_account_cache(&account);
            if let Some(key) = session_key.as_ref() {
                if config.session_affinity {
                    session_affinity::bind(key, &account.id, config.session_affinity_ttl_secs);
                }
            }
        } else {
            apply_status_failure(&mut account, status, &upstream_headers);
            let _ = replace_account_tokens(&account);
        }

        let usage_tracker = Arc::new(StreamUsageTracker::new(
            request_id.clone(),
            Some(account.id.clone()),
            path.to_string(),
            Some(converted.requested_model.clone()),
            Some(resolved_model.clone()),
            status.as_u16(),
            latency_ms,
            client_source.to_string(),
            config.session_affinity,
            config.session_affinity_ttl_secs,
            account.id.clone(),
        ));
        // Shared converter so we can push chunks then finish() after upstream closes
        // (covers providers that omit `data: [DONE]`).
        let converter = Arc::new(parking_lot::Mutex::new(OpenAiToAnthropicSse::new()));
        let tracker = usage_tracker.clone();
        let conv_push = converter.clone();
        let head = upstream.bytes_stream().map(move |chunk| {
            chunk
                .map(|bytes| {
                    tracker.note_chunk(&bytes);
                    let mut c = conv_push.lock();
                    Bytes::from(c.push(&bytes))
                })
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        });
        let conv_fin = converter.clone();
        let tail = stream::once(async move {
            let mut c = conv_fin.lock();
            Ok::<Bytes, std::io::Error>(Bytes::from(c.finish()))
        });
        let body = Body::from_stream(head.chain(tail));
        let mut response = Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(body)
            .unwrap_or_else(|_| Response::new(Body::empty()));
        for (k, v) in upstream_headers.iter() {
            let key = k.as_str().to_ascii_lowercase();
            if matches!(key.as_str(), "x-request-id") {
                if let Ok(name) = HeaderName::from_bytes(k.as_ref()) {
                    response.headers_mut().insert(name, v.clone());
                }
            }
        }
        tracing::debug!(
            target: "gateway",
            %mapping_reason,
            model = %resolved_model,
            "anthropic messages stream started"
        );
        return Ok(response);
    }

    // Non-streaming JSON
    let bytes = upstream.bytes().await?;
    if status.is_success() {
        mark_success(&mut account);
        touch_account_cache(&account);
        if let Some(key) = session_key.as_ref() {
            if config.session_affinity {
                session_affinity::bind(key, &account.id, config.session_affinity_ttl_secs);
            }
        }
    } else {
        apply_status_failure(&mut account, status, &upstream_headers);
        let _ = replace_account_tokens(&account);
    }

    let upstream_json: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        json!({"error": {"message": String::from_utf8_lossy(&bytes), "type": "upstream_error"}})
    });

    let (out_status, out_value) = if status.is_success() {
        match openai_chat_to_anthropic(&upstream_json) {
            Ok(v) => (StatusCode::OK, v),
            Err(err) => (
                StatusCode::BAD_GATEWAY,
                crate::gateway::anthropic::anthropic_error_body(
                    "api_error",
                    format!("response convert failed: {err}"),
                ),
            ),
        }
    } else {
        (
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            openai_error_to_anthropic(status.as_u16(), &upstream_json),
        )
    };

    let (i, o, c) = extract_usage_tokens(&upstream_json);
    // Also read Anthropic-shaped usage if conversion already happened.
    let (i, o, c) = if i == 0 && o == 0 {
        let input = out_value
            .pointer("/usage/input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = out_value
            .pointer("/usage/output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        (input, output, c)
    } else {
        (i, o, c)
    };

    log_request(
        &request_id,
        Some(account.id.clone()),
        path,
        Some(converted.requested_model),
        Some(resolved_model),
        out_status.as_u16(),
        started.elapsed().as_millis() as u64,
        None,
        i,
        o,
        c,
        if out_status.is_success() {
            None
        } else {
            Some(out_value.to_string())
        },
        client_source,
        Some(mapping_reason),
    );

    let mut response = Response::builder()
        .status(out_status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&out_value)?))
        .unwrap_or_else(|_| Response::new(Body::empty()));
    copy_safe_headers(upstream_headers, response.headers_mut());
    Ok(response)
}

async fn proxy_json_inner(
    ctx: &ProxyContext,
    method: Method,
    path: &str,
    headers: HeaderMap,
    body: Bytes,
    client_source: &str,
) -> AppResult<Response> {
    let config = load_config()?;
    authorize_request(&headers, &config)
        .await
        .map_err(|resp| AppError::msg(format!("unauthorized: {}", response_to_text(resp))))?;

    let mut requested_model = None;
    let mut resolved_model = None;
    let mut mapping_reason = None;
    let mut outbound_body = body;
    let mut custom_tool_names: HashSet<String> = HashSet::new();
    let mut parsed_request: Option<Value> = None;
    let mut has_image_gen_tools = false;
    let mut client_wants_stream = false;
    // Capture session key before sanitize strips previous_response_id etc.
    let mut session_key: Option<String> = None;
    // Detect Grok Build early — native plane must keep continuity + skip Codex-only guards.
    let build_plane = is_grok_build_plane(&headers);

    if matches!(method, Method::POST | Method::PUT | Method::PATCH) && !outbound_body.is_empty() {
        if let Ok(mut value) = serde_json::from_slice::<Value>(&outbound_body) {
            session_key = session_affinity::extract_session_key(&headers, Some(&value));
            let mut body_changed = false;
            client_wants_stream = value
                .get("stream")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // Some clients only signal SSE via Accept.
            if !client_wants_stream {
                client_wants_stream = headers
                    .get(header::ACCEPT)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.contains("text/event-stream"))
                    .unwrap_or(false);
            }
            if let Some(model) = value.get("model").and_then(|m| m.as_str()).map(|s| s.to_string()) {
                requested_model = Some(model.clone());
                if path.contains("/images/") {
                    let resolved = if model.to_lowercase().contains("imagine") || model.to_lowercase().contains("image") {
                        model
                    } else {
                        config.default_image_model.clone()
                    };
                    value["model"] = Value::String(resolved.clone());
                    resolved_model = Some(resolved);
                    mapping_reason = Some("image-default".into());
                    body_changed = true;
                } else if path.contains("/videos/") {
                    let resolved = if model.to_lowercase().contains("imagine") || model.to_lowercase().contains("video") {
                        model
                    } else {
                        config.default_video_model.clone()
                    };
                    value["model"] = Value::String(resolved.clone());
                    resolved_model = Some(resolved);
                    mapping_reason = Some("video-default".into());
                    body_changed = true;
                } else {
                    let (resolved, reason) = resolve_model(&config, &model);
                    if resolved != model {
                        body_changed = true;
                    }
                    value["model"] = Value::String(resolved.clone());
                    resolved_model = Some(resolved);
                    mapping_reason = Some(reason);
                }
            }

            // Codex/OpenAI Responses → xAI tool compatibility (custom tools, etc.)
            let is_responses = path == "/responses"
                || path.ends_with("/responses")
                || path.ends_with("/responses/compact");
            let is_chat = path.contains("/chat/completions");
            if is_responses {
                // Build plane: keep previous_response_id / prompt_cache_retention for
                // cli-chat-proxy continuity. Codex console path still strips them.
                let sanitized = sanitize_responses_request_ex(&mut value, build_plane);
                custom_tool_names = sanitized.custom_tool_names;
                has_image_gen_tools = sanitized.has_image_gen_tools;
                if sanitized.modified {
                    body_changed = true;
                }
                // Image tool loop needs a full JSON response; force non-stream upstream.
                // Only for non-build (Codex) plane — Grok Build handles tools natively.
                if !build_plane
                    && has_image_gen_tools
                    && value.get("stream").and_then(|v| v.as_bool()) == Some(true)
                {
                    value["stream"] = Value::Bool(false);
                    body_changed = true;
                }
            }
            // Multi-turn agent loops re-send full history (incl. base64 images /
            // huge tool outputs). Shrink before upstream to cut token burn and
            // avoid forced stops once the body/context balloons.
            // Grok Build chat/responses still benefits from image/text prune.
            if is_responses || is_chat {
                let opt = optimize_responses_payload(&mut value);
                if opt.modified {
                    body_changed = true;
                    opt.log_summary(path);
                }
            }

            // Only re-serialize when we actually mutated the body. Blind re-encoding can
            // corrupt opaque compaction / encrypted_content blobs that xAI requires verbatim.
            if body_changed {
                outbound_body = Bytes::from(serde_json::to_vec(&value)?);
            }
            parsed_request = Some(value);
        }
    }

    let custom_tool_names = Arc::new(custom_tool_names);

    let http = ctx.client();
    let upstream_base = resolve_upstream_base(&config, &headers);
    let url = format!("{upstream_base}{path}");
    let effective_source = if build_plane {
        "grok-build"
    } else {
        client_source
    };
    if build_plane {
        tracing::debug!(
            target: "gateway",
            %path,
            upstream = %upstream_base,
            "routing Grok Build plane via cli-chat-proxy"
        );
    }
    let started = Instant::now();
    // Always send exactly one Content-Type. Forwarding the client's header on top of
    // our own produces duplicate Content-Type values; xAI then returns 415
    // "Expected request with `Content-Type: application/json`".
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .unwrap_or("application/json")
        .to_string();

    // Header-only session key when body was empty / non-JSON.
    if session_key.is_none() {
        session_key = session_affinity::extract_session_key(&headers, parsed_request.as_ref());
    }

    // Ensure stable prompt_cache_key (never inject rotating previous_response_id).
    if let (Some(key), Some(mut value)) = (session_key.as_ref(), parsed_request.clone()) {
        if session_affinity::ensure_prompt_cache_key(&mut value, key) {
            if let Ok(bytes) = serde_json::to_vec(&value) {
                outbound_body = Bytes::from(bytes);
                parsed_request = Some(value);
            }
        }
    }

    // Conversation / session continuity header for xAI prefix cache.
    // Prefer the client's own x-grok-conv-id (Grok Build) — rewriting it with a
    // derived seed/hash would split the cache namespace and burn tokens every turn.
    let client_conv_id = headers
        .get("x-grok-conv-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let conv_header = client_conv_id.or_else(|| {
        session_key
            .as_deref()
            .and_then(session_affinity::stable_cache_key)
    });

    let passthrough_headers = if build_plane {
        collect_build_plane_passthrough_headers(&headers)
    } else {
        Vec::new()
    };

    let build_upstream = {
        let conv_header = conv_header.clone();
        let url = url.clone();
        let accept = accept.clone();
        let passthrough_headers = passthrough_headers.clone();
        move |client: &reqwest::Client, token: &str, body: Bytes| {
            let mut req = client
                .request(method.clone(), &url)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::ACCEPT, accept.as_str());
            // Apply client CLI headers first, then pin conv-id so affinity wins.
            for (name, value) in &passthrough_headers {
                if name.as_str().eq_ignore_ascii_case("x-grok-conv-id") {
                    continue;
                }
                req = req.header(name.clone(), value.as_str());
            }
            if let Some(ref cid) = conv_header {
                req = req.header("x-grok-conv-id", cid.as_str());
            }
            if !body.is_empty() || matches!(method, Method::POST | Method::PUT | Method::PATCH) {
                // Force a single JSON content type for mutating requests (even empty body).
                req = req
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body);
            }
            req
        }
    };

    // Single-account happy path is unchanged (one pick + one send).
    // On account-scoped failures (401/403/429/5xx/transport), try other accounts
    // inside this request so the client does not see several hard failures in a row.
    // Files offload targets console Files API (api.x.ai) — never enable on Grok Build
    // plane where inference hits cli-chat-proxy (file_id would be wrong account/plane).
    let allow_files_offload = !build_plane
        && (path.contains("/responses") || path.contains("/chat/completions"));
    let (mut account, token, upstream) = send_with_account_failover(
        &config,
        &http,
        &url,
        &build_upstream,
        outbound_body.clone(),
        session_key.as_deref(),
        allow_files_offload,
    )
    .await?;

    let mut status = upstream.status();
    let mut upstream_headers = upstream.headers().clone();
    apply_rate_limit_headers(&mut account, &upstream_headers);
    let is_stream = status.is_success()
        && upstream_headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("text/event-stream") || v.contains("stream"))
            .unwrap_or(false);

    let request_id = Uuid::new_v4().to_string();
    let latency_ms = started.elapsed().as_millis() as u64;

    if is_stream {
        if status.is_success() {
            mark_success(&mut account);
            touch_account_cache(&account);
            if let Some(key) = session_key.as_ref() {
                if config.session_affinity {
                    session_affinity::bind(key, &account.id, config.session_affinity_ttl_secs);
                }
            }
        } else {
            apply_status_failure(&mut account, status, &upstream_headers);
            let _ = replace_account_tokens(&account);
        }

        // Responses streams: buffer → detect reasoning-only empty completion → retry once
        // before the client sees `response.completed` (Codex ends the turn otherwise).
        // Grok Build manages its own agent loop — extra silent retries double-bill SuperGrok
        // credits and can desync the TUI stream state.
        let guard_empty = config.empty_completion_retry
            && !build_plane
            && is_responses_path(path)
            && status.is_success()
            && parsed_request.is_some();
        if guard_empty {
            let recovered = buffer_sse_and_recover_empty_completion(
                upstream,
                &http,
                &token,
                &build_upstream,
                parsed_request.as_ref().unwrap(),
                custom_tool_names.as_ref(),
            )
            .await?;
            let (i, o, c) = extract_usage_tokens(&recovered.usage_source);
            if let Some(rid) = recovered.usage_source.get("id").and_then(|v| v.as_str()) {
                session_affinity::bind_response_chain(
                    rid,
                    &account.id,
                    config.session_affinity_ttl_secs,
                );
            }
            let latency_ms = started.elapsed().as_millis() as u64;
            let reason = recovered
                .retried
                .then_some("empty-completion-retry".to_string())
                .or(mapping_reason.clone());
            log_request(
                &request_id,
                Some(account.id.clone()),
                path,
                requested_model,
                resolved_model,
                status.as_u16(),
                latency_ms,
                None,
                i,
                o,
                c,
                None,
                effective_source,
                reason,
            );
            let mut response = Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(Body::from(recovered.sse))
                .unwrap_or_else(|_| Response::new(Body::empty()));
            for (k, v) in upstream_headers.iter() {
                let key = k.as_str().to_ascii_lowercase();
                if matches!(key.as_str(), "x-request-id" | "cache-control") {
                    if let Ok(name) = HeaderName::from_bytes(k.as_ref()) {
                        response.headers_mut().insert(name, v.clone());
                    }
                }
            }
            return Ok(response);
        }

        let custom_names = custom_tool_names.clone();
        // Scan SSE for usage (xAI puts cached_tokens under input_tokens_details).
        // Log on stream drop so streaming traffic is not silently recorded as 0 tokens.
        let usage_tracker = Arc::new(StreamUsageTracker::new(
            request_id.clone(),
            Some(account.id.clone()),
            path.to_string(),
            requested_model.clone(),
            resolved_model.clone(),
            status.as_u16(),
            latency_ms,
            effective_source.to_string(),
            config.session_affinity,
            config.session_affinity_ttl_secs,
            account.id.clone(),
        ));
        let tracker = usage_tracker.clone();
        let stream = upstream.bytes_stream().map(move |chunk| {
            chunk
                .map(|bytes| {
                    tracker.note_chunk(&bytes);
                    rewrite_sse_chunk(&bytes, &custom_names)
                })
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        });
        let body = Body::from_stream(stream);
        let mut response = Response::builder().status(status).body(body).unwrap_or_else(|_| Response::new(Body::empty()));
        copy_safe_headers(upstream_headers, response.headers_mut());
        return Ok(response);
    }

    let mut bytes = upstream.bytes().await?;

    // Retry once when xAI rejects opaque context or input item shapes (Codex multi-turn).
    // Skip nuclear strip on Grok Build plane: it removes prompt_cache_key / continuity
    // and forces a full re-tokenize (token blow-up + cache miss).
    if !status.is_success() && !build_plane {
        let err_text = String::from_utf8_lossy(&bytes);
        if is_compaction_blob_error(&err_text) || is_model_input_error(&err_text) {
            if let Some(mut req_value) = parsed_request.clone() {
                // Nuclear strip first, then re-sanitize tools / custom_tool_call shapes.
                let _ = strip_opaque_context(&mut req_value);
                let _ = sanitize_responses_request(&mut req_value);
                // Re-inject stable cache key after nuclear strip removed it.
                if let Some(key) = session_key.as_deref() {
                    let _ = session_affinity::ensure_prompt_cache_key(&mut req_value, key);
                }
                // Always retry once for these errors even if strip thought nothing changed.
                let retry_body = Bytes::from(serde_json::to_vec(&req_value)?);
                if let Ok(retry_resp) = build_upstream(&http, &token, retry_body).send().await {
                    let retry_status = retry_resp.status();
                    let retry_headers = retry_resp.headers().clone();
                    let retry_bytes = retry_resp.bytes().await?;
                    // If still failing, try messages-only fallback.
                    if !retry_status.is_success()
                        && is_compaction_blob_error(&String::from_utf8_lossy(&retry_bytes))
                    {
                        if let Some(mut nuclear) = parsed_request.clone() {
                            nuclear_messages_only(&mut nuclear);
                            let _ = sanitize_responses_request(&mut nuclear);
                            if let Some(key) = session_key.as_deref() {
                                let _ = session_affinity::ensure_prompt_cache_key(&mut nuclear, key);
                            }
                            let body2 = Bytes::from(serde_json::to_vec(&nuclear)?);
                            if let Ok(r2) = build_upstream(&http, &token, body2).send().await {
                                status = r2.status();
                                upstream_headers = r2.headers().clone();
                                bytes = r2.bytes().await?;
                                mapping_reason = Some("messages-only-retry".into());
                            }
                        }
                    } else {
                        status = retry_status;
                        upstream_headers = retry_headers;
                        bytes = retry_bytes;
                        mapping_reason = Some("input-sanitize-retry".into());
                    }
                }
            }
        }
    }

    // Re-apply headers after retries (status / headers may have changed).
    apply_rate_limit_headers(&mut account, &upstream_headers);

    if status.is_success() {
        mark_success(&mut account);
        touch_account_cache(&account);
    } else {
        apply_status_failure(&mut account, status, &upstream_headers);
        let _ = replace_account_tokens(&account);
    }

    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_tokens = 0u64;
    if let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) {
        // Server-side image_gen loop: fulfill Grok Imagine, feed results back, continue.
        if status.is_success()
            && (path == "/responses" || path.ends_with("/responses"))
            && has_image_gen_tools
        {
            if let Some(req_template) = parsed_request.clone() {
                match run_image_gen_tool_loop(
                    ctx,
                    &config,
                    &token,
                    &url,
                    &req_template,
                    value.clone(),
                )
                .await
                {
                    Ok(final_value) => {
                        value = final_value;
                        mapping_reason = Some("image-gen-bridge".into());
                    }
                    Err(err) => {
                        tracing::warn!("image_gen bridge failed: {err}");
                    }
                }
            }
        }

        // Reasoning-only / narration-only premature stop → silent retry so Codex keeps going.
        // Disabled on Grok Build plane (native agent loop; retries would double SuperGrok spend).
        if status.is_success()
            && config.empty_completion_retry
            && !build_plane
            && is_responses_path(path)
            && should_retry_premature_agent_stop(&value, parsed_request.as_ref())
        {
            if let Some(req_template) = parsed_request.as_ref() {
                match retry_empty_completion_once(
                    &http,
                    &token,
                    &build_upstream,
                    req_template,
                    &value,
                )
                .await
                {
                    Ok(Some(retried)) => {
                        value = retried;
                        mapping_reason = Some("empty-completion-retry".into());
                        tracing::warn!(
                            "recovered premature agent stop (empty/narration) via non-stream retry"
                        );
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "premature agent stop retry still empty/narration; passing through"
                        );
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "empty-completion retry failed");
                    }
                }
            }
        }

        // Deferred video jobs are account-scoped — pin owner for later GET poll.
        if status.is_success()
            && (path.contains("/videos/generations") || path.contains("/videos/edits"))
        {
            if let Some(rid) = crate::gateway::job_affinity::extract_video_request_id(&value) {
                crate::gateway::job_affinity::remember_video_job(&rid, &account.id);
            }
        }

        if status.is_success() && !custom_tool_names.is_empty() {
            let _ = rewrite_responses_payload(&mut value, &custom_tool_names);
        }

        // Grok Build: align /user identity with session JWT (pool account may differ).
        // Do not invent subscriptionTiers; allow_access comes from GET /v1/settings.
        if status.is_success() && is_user_profile_path(path) {
            if rewrite_user_profile_for_build_gate(&mut value, &headers) {
                mapping_reason = Some("user-profile-identity-align".into());
            }
        }

        bytes = Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| bytes.to_vec()));

        let (i, o, c) = extract_usage_tokens(&value);
        input_tokens = i;
        output_tokens = o;
        cache_tokens = c;
        if status.is_success() {
            if let Some(rid) = value
                .get("id")
                .and_then(|v| v.as_str())
                .or_else(|| value.pointer("/response/id").and_then(|v| v.as_str()))
            {
                session_affinity::bind_response_chain(
                    rid,
                    &account.id,
                    config.session_affinity_ttl_secs,
                );
            }
            if let Some(key) = session_key.as_ref() {
                if config.session_affinity {
                    session_affinity::bind(key, &account.id, config.session_affinity_ttl_secs);
                }
            }
        }
    }
    let error_summary = if status.is_success() {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes).chars().take(500).collect())
    };
    if cache_tokens > 0 {
        tracing::debug!(
            account = %account.id,
            input_tokens,
            output_tokens,
            cache_tokens,
            hit_pct = (cache_tokens as f64 / input_tokens.max(1) as f64 * 100.0),
            "prompt cache hit recorded"
        );
    }
    log_request(
        &request_id,
        Some(account.id.clone()),
        path,
        requested_model,
        resolved_model,
        status.as_u16(),
        latency_ms,
        None,
        input_tokens,
        output_tokens,
        cache_tokens,
        error_summary,
        effective_source,
        mapping_reason,
    );

    // Client asked for SSE but we may have forced JSON (e.g. image_gen tool loop).
    // Codex requires a proper Responses SSE ending with `type: response.completed`.
    if status.is_success() && client_wants_stream {
        let looks_like_sse = bytes.starts_with(b"event:") || bytes.starts_with(b"data:");
        if !looks_like_sse {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                let sse = responses_json_to_sse(&value);
                let mut response = Response::builder()
                    .status(status)
                    .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .header(header::CONNECTION, "keep-alive")
                    .body(Body::from(sse))
                    .unwrap_or_else(|_| Response::new(Body::empty()));
                // Don't copy upstream content-type (application/json).
                for (k, v) in upstream_headers.iter() {
                    let key = k.as_str().to_ascii_lowercase();
                    if matches!(key.as_str(), "x-request-id" | "cache-control") {
                        if let Ok(name) = HeaderName::from_bytes(k.as_ref()) {
                            response.headers_mut().insert(name, v.clone());
                        }
                    }
                }
                return Ok(response);
            }
        }
    }

    let mut response = Response::builder()
        .status(status)
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::empty()));
    copy_safe_headers(upstream_headers, response.headers_mut());
    Ok(response)
}

/// Build an OpenAI Responses API–compatible SSE stream from a completed JSON response.
/// Codex waits for an event whose data has `"type":"response.completed"`.
fn responses_json_to_sse(response: &Value) -> String {
    let mut out = String::with_capacity(4096);
    let resp_id = response
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("resp_proxy");

    let created = json!({
        "type": "response.created",
        "response": {
            "id": resp_id,
            "object": "response",
            "status": "in_progress",
            "model": response.get("model").cloned().unwrap_or(json!("")),
        }
    });
    push_sse(&mut out, "response.created", &created);

    if let Some(items) = response.get("output").and_then(|o| o.as_array()) {
        for (i, item) in items.iter().enumerate() {
            let added = json!({
                "type": "response.output_item.added",
                "output_index": i,
                "item": item,
            });
            push_sse(&mut out, "response.output_item.added", &added);

            // For message items, also emit a text done event so UIs can render content.
            if item.get("type").and_then(|t| t.as_str()) == Some("message") {
                if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                    for (ci, part) in content.iter().enumerate() {
                        let text = part
                            .get("text")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        if !text.is_empty() {
                            let delta = json!({
                                "type": "response.output_text.delta",
                                "output_index": i,
                                "content_index": ci,
                                "delta": text,
                            });
                            push_sse(&mut out, "response.output_text.delta", &delta);
                            let done = json!({
                                "type": "response.output_text.done",
                                "output_index": i,
                                "content_index": ci,
                                "text": text,
                            });
                            push_sse(&mut out, "response.output_text.done", &done);
                        }
                    }
                }
            }

            let done = json!({
                "type": "response.output_item.done",
                "output_index": i,
                "item": item,
            });
            push_sse(&mut out, "response.output_item.done", &done);
        }
    }

    let mut completed_response = response.clone();
    if let Some(obj) = completed_response.as_object_mut() {
        obj.insert("status".into(), json!("completed"));
        obj.entry("object").or_insert_with(|| json!("response"));
    }
    let completed = json!({
        "type": "response.completed",
        "response": completed_response,
    });
    push_sse(&mut out, "response.completed", &completed);
    out
}

fn push_sse(out: &mut String, event: &str, data: &Value) {
    out.push_str("event: ");
    out.push_str(event);
    out.push('\n');
    out.push_str("data: ");
    out.push_str(&data.to_string());
    out.push_str("\n\n");
}

struct EmptyCompletionStreamResult {
    sse: String,
    /// Best-effort JSON used for usage / session-id extraction.
    usage_source: Value,
    retried: bool,
}

/// Buffer an upstream Responses SSE, rewrite custom tools, and if the completed
/// payload is reasoning-only, retry once (non-stream) with a recovery nudge.
async fn buffer_sse_and_recover_empty_completion<F>(
    upstream: reqwest::Response,
    http: &reqwest::Client,
    token: &str,
    build_upstream: &F,
    original_request: &Value,
    custom_names: &HashSet<String>,
) -> AppResult<EmptyCompletionStreamResult>
where
    F: Fn(&reqwest::Client, &str, Bytes) -> reqwest::RequestBuilder,
{
    let mut buf = Vec::with_capacity(64 * 1024);
    let mut stream = upstream.bytes_stream();
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::msg(format!("sse buffer read: {e}")))?;
        if buf.len() + chunk.len() > SSE_BUFFER_LIMIT {
            truncated = true;
            // Keep a prefix so we can still return something useful.
            let room = SSE_BUFFER_LIMIT.saturating_sub(buf.len());
            if room > 0 {
                buf.extend_from_slice(&chunk[..room.min(chunk.len())]);
            }
            break;
        }
        buf.extend_from_slice(&chunk);
    }

    let rewritten = rewrite_sse_chunk(&Bytes::from(buf), custom_names);
    let mut sse = String::from_utf8_lossy(&rewritten).into_owned();
    let mut usage_source = extract_completed_response_from_sse(&sse).unwrap_or(json!({}));
    let mut retried = false;

    if truncated {
        tracing::warn!(
            limit = SSE_BUFFER_LIMIT,
            "SSE exceeded empty-completion buffer limit; skipping recovery"
        );
        return Ok(EmptyCompletionStreamResult {
            sse,
            usage_source,
            retried,
        });
    }

    // Only retry when we successfully parsed a completed premature-stop response.
    // Missing `response.completed` must not be treated as empty (would spuriously retry).
    if let Some(completed) = extract_completed_response_from_sse(&sse) {
        usage_source = completed;
        if should_retry_premature_agent_stop(&usage_source, Some(original_request)) {
            match retry_empty_completion_once(
                http,
                token,
                build_upstream,
                original_request,
                &usage_source,
            )
            .await
            {
                Ok(Some(mut value)) => {
                    if !custom_names.is_empty() {
                        let _ = rewrite_responses_payload(&mut value, custom_names);
                    }
                    sse = responses_json_to_sse(&value);
                    usage_source = value;
                    retried = true;
                    tracing::warn!(
                        "recovered premature agent stop (empty/narration) via buffered stream retry"
                    );
                }
                Ok(None) => {
                    tracing::warn!(
                        "premature agent stop stream retry still empty/narration; \
                         passing original SSE"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "empty-completion stream retry failed; passing original SSE"
                    );
                }
            }
        }
    }

    Ok(EmptyCompletionStreamResult {
        sse,
        usage_source,
        retried,
    })
}

/// Up to two silent non-stream recovery attempts for premature agent stops.
///
/// One attempt is not always enough: empty → narration-only still ends Codex.
/// Returns `Ok(Some(value))` when a non-premature payload is obtained, or when
/// a partial recovery is *better* than the original empty (e.g. message > pure
/// reasoning). `Ok(None)` only when nothing improved.
async fn retry_empty_completion_once<F>(
    http: &reqwest::Client,
    token: &str,
    build_upstream: &F,
    original_request: &Value,
    empty_response: &Value,
) -> AppResult<Option<Value>>
where
    F: Fn(&reqwest::Client, &str, Bytes) -> reqwest::RequestBuilder,
{
    // One soft retry (pinned tool_choice); if still narration, inject a synthetic call.
    // Two soft retries burned ~5s without producing tools (see session 019f5eaf).
    const MAX_ATTEMPTS: u32 = 1;
    let mut seed = empty_response.clone();
    let mut best = empty_response.clone();
    let mut best_score = recovery_quality_score(&best);
    for attempt in 1..=MAX_ATTEMPTS {
        let retry_req = build_empty_completion_retry_request(original_request, &seed);
        let retry_body = Bytes::from(serde_json::to_vec(&retry_req)?);
        let resp = build_upstream(http, token, retry_body)
            .send()
            .await
            .map_err(|e| AppError::msg(format!("empty-completion retry send: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                attempt,
                %status,
                body = %body.chars().take(240).collect::<String>(),
                "empty-completion retry upstream non-success"
            );
            // Keep best partial if we already improved over pure empty.
            break;
        }
        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = resp.bytes().await?;
        let value = if content_type.contains("text/event-stream") || content_type.contains("stream")
        {
            let text = String::from_utf8_lossy(&bytes);
            extract_completed_response_from_sse(&text).ok_or_else(|| {
                AppError::msg("empty-completion retry stream missing response.completed")
            })?
        } else {
            serde_json::from_slice::<Value>(&bytes)?
        };
        let score = recovery_quality_score(&value);
        if score > best_score {
            best = value.clone();
            best_score = score;
        }
        if !should_retry_premature_agent_stop(&value, Some(original_request)) {
            if attempt > 1 {
                tracing::warn!(attempt, "premature agent stop cleared after multi-retry");
            }
            return Ok(Some(value));
        }
        tracing::warn!(
            attempt,
            max = MAX_ATTEMPTS,
            score,
            "recovery attempt still empty/narration; retrying if budget remains"
        );
        seed = value;
    }
    // Soft retries (even with tool_choice pin) often still return narration.
    // Hard guarantee: inject a real function_call so Codex keeps the turn alive.
    if let Some(forced) = synthesize_forced_tool_response(original_request, &best) {
        tracing::warn!(
            best_score,
            "recovery exhausted; injecting synthetic tool call to keep Codex loop alive"
        );
        return Ok(Some(forced));
    }
    let original_score = recovery_quality_score(empty_response);
    if best_score > original_score {
        tracing::warn!(
            best_score,
            original_score,
            "recovery exhausted; returning best partial instead of pure empty"
        );
        return Ok(Some(best));
    }
    Ok(None)
}

/// Last-resort request: only plain user/assistant text messages.
fn nuclear_messages_only(value: &mut Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.remove("previous_response_id");
        obj.remove("context_management");
        obj.remove("prompt_cache_key");
    }
    let Some(Value::Array(items)) = value.get("input").cloned() else {
        return;
    };
    let mut kept = Vec::new();
    for item in items {
        let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "user" && role != "assistant" && role != "system" {
            // typed message?
            if item.get("type").and_then(|t| t.as_str()) != Some("message") {
                continue;
            }
        }
        let mut text = String::new();
        if let Some(content) = item.get("content") {
            match content {
                Value::String(s) => text = s.clone(),
                Value::Array(parts) => {
                    for p in parts {
                        if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                    }
                }
                _ => {}
            }
        }
        if text.is_empty() {
            continue;
        }
        let role = if role.is_empty() { "user" } else { role };
        kept.push(json!({
            "role": role,
            "content": [{"type": "input_text", "text": text}]
        }));
    }
    if kept.is_empty() {
        kept.push(json!({
            "role": "user",
            "content": [{"type": "input_text", "text": "Continue."}]
        }));
    }
    // Keep only the last few turns to avoid context bloat.
    let start = kept.len().saturating_sub(12);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("input".into(), Value::Array(kept[start..].to_vec()));
    }
}

/// Fulfill image_gen function_calls with Grok Imagine and continue the Responses loop.
async fn run_image_gen_tool_loop(
    ctx: &ProxyContext,
    config: &AppConfig,
    token: &str,
    responses_url: &str,
    request_template: &Value,
    mut response: Value,
) -> AppResult<Value> {
    let mut fulfilled_pairs: Vec<(Value, Value)> = Vec::new();
    let mut working_input = request_template
        .get("input")
        .cloned()
        .unwrap_or_else(|| json!([]));

    for _round in 0..MAX_IMAGE_TOOL_ROUNDS {
        let calls = collect_image_gen_calls(&response);
        if calls.is_empty() {
            break;
        }

        // Append model output + tool results to conversation input.
        if let Some(output) = response.get("output").cloned() {
            match working_input {
                Value::Array(ref mut arr) => {
                    if let Value::Array(items) = output {
                        arr.extend(items);
                    }
                }
                other => {
                    let mut arr = vec![other];
                    if let Value::Array(items) = output {
                        arr.extend(items);
                    }
                    working_input = Value::Array(arr);
                }
            }
        }

        let http = ctx.client();
        for call in &calls {
            // Stay on the same OAuth account as the parent /responses turn.
            let result = fulfill_image_gen_call(&http, config, call, Some(token)).await?;
            fulfilled_pairs.push((call.clone(), result.clone()));
            if let Value::Array(ref mut arr) = working_input {
                arr.push(result);
            }
        }

        // Continue generation with tool outputs.
        let mut next_req = request_template.clone();
        if let Some(obj) = next_req.as_object_mut() {
            obj.insert("input".into(), working_input.clone());
            obj.insert("stream".into(), Value::Bool(false));
            // Prefer continuing from previous response when available.
            if let Some(id) = response.get("id").cloned() {
                obj.insert("previous_response_id".into(), id);
                // When using previous_response_id, only send new tool outputs.
                let only_outputs: Vec<Value> = fulfilled_pairs
                    .iter()
                    .map(|(_, r)| r.clone())
                    .collect();
                // Safer: send full reconstructed input without previous_response_id for xAI.
                obj.remove("previous_response_id");
                obj.insert("input".into(), working_input.clone());
                let _ = only_outputs;
                let _ = id;
            }
        }

        let resp = http
            .post(responses_url)
            .bearer_auth(token)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&next_req)
            .send()
            .await?;
        if !resp.status().is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(AppError::msg(format!(
                "image tool loop upstream failed: {err_body}"
            )));
        }
        response = resp.json().await?;
    }

    if !fulfilled_pairs.is_empty() {
        inject_image_generation_calls(&mut response, &fulfilled_pairs);
    }
    Ok(response)
}

/// Rewrite SSE chunks so custom tools come back as `custom_tool_call` for Codex.
fn rewrite_sse_chunk(bytes: &Bytes, custom_names: &HashSet<String>) -> Bytes {
    if custom_names.is_empty() {
        return bytes.clone();
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => return bytes.clone(),
    };
    // Fast path: no function_call-looking payload → skip JSON parse work.
    if !text.contains("function_call") && !text.contains("\"name\"") {
        return bytes.clone();
    }
    let mut out = String::with_capacity(text.len() + 32);
    let mut changed = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let ending = &line[trimmed.len()..];
        if let Some(data) = trimmed.strip_prefix("data:") {
            let payload = data.strip_prefix(' ').unwrap_or(data);
            let rewritten = rewrite_sse_data_line(payload, custom_names);
            if rewritten.as_str() != payload {
                changed = true;
            }
            out.push_str("data: ");
            out.push_str(&rewritten);
            out.push_str(ending);
        } else {
            out.push_str(line);
        }
    }
    if !changed {
        return bytes.clone();
    }
    Bytes::from(out)
}


fn is_user_profile_path(path: &str) -> bool {
    let p = path.split('?').next().unwrap_or(path);
    p == "/user" || p.ends_with("/user")
}

/// Paid tiers observed from cli-chat-proxy `/user?include=subscription`.
fn is_paid_subscription_tier(tier: &str) -> bool {
    let t = tier.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "free" | "basic" | "none" | "null" | "anonymous"
    ) {
        return false;
    }
    // Keep SuperGrok* as-is (already target shape).
    true
}

/// Rewrite `/user` JSON for Grok Build client paywall / GrowthBook.
///
/// Returns true when the body was modified.
fn rewrite_user_profile_for_build_gate(value: &mut Value, headers: &HeaderMap) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    let mut changed = false;

    // Identity: prefer claims from the client Bearer (session in ~/.grok/auth.json).
    if let Some(token) = client_bearer_token(headers) {
        if let Some(payload) = crate::auth::jwt_payload(token) {
            let principal = payload
                .get("principal_id")
                .or_else(|| payload.get("sub"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            if let Some(pid) = principal {
                for key in ["userId", "principalId", "user_id", "principal_id"] {
                    if obj.get(key).and_then(|v| v.as_str()) != Some(pid) {
                        obj.insert(key.to_string(), Value::String(pid.to_string()));
                        changed = true;
                    }
                }
            }
            if let Some(email) = payload.get("email").and_then(|v| v.as_str()) {
                if obj.get("email").and_then(|v| v.as_str()) != Some(email) {
                    obj.insert("email".into(), Value::String(email.to_string()));
                    changed = true;
                }
            }
        }
    }

    // Do NOT rewrite subscriptionTiers to "SuperGrok".
    // Client paywall whitelist recognizes API enums like GrokPro / XPremiumPlus;
    // inventing "SuperGrok" yields paywall_check_no_subscription and still gated.
    // allow_access comes from GET /v1/settings (remote settings), not from this string.

    // Ensure code access flag is present for paid sessions.
    let tier = obj
        .get("subscriptionTiers")
        .or_else(|| obj.get("subscription_tier"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if is_paid_subscription_tier(tier) {
        match obj.get("hasGrokCodeAccess") {
            Some(Value::Bool(true)) => {}
            _ => {
                obj.insert("hasGrokCodeAccess".into(), Value::Bool(true));
                changed = true;
            }
        }
        if obj.get("userBlockedReason").is_some()
            && !obj
                .get("userBlockedReason")
                .map(|v| v.is_null())
                .unwrap_or(false)
        {
            obj.insert("userBlockedReason".into(), Value::Null);
            changed = true;
        }
    }

    changed
}

fn client_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())?
        .trim();
    let token = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))
        .unwrap_or(auth)
        .trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

fn copy_safe_headers(src: HeaderMap, dst: &mut HeaderMap) {
    for (k, v) in src.iter() {
        let key = k.as_str().to_ascii_lowercase();
        if matches!(key.as_str(), "content-type" | "cache-control" | "x-request-id") {
            if let Ok(name) = HeaderName::from_bytes(k.as_ref()) {
                dst.insert(name, v.clone());
            }
        }
    }
}

/// Account-scoped failures worth trying another credential on (same request).
fn status_is_account_failover(status: StatusCode) -> bool {
    let code = status.as_u16();
    matches!(code, 401 | 403 | 429) || status.is_server_error()
}

/// Cap in-request failovers so a full outage does not multiply latency too hard.
const MAX_ACCOUNT_TRIES: usize = 3;

/// Pick account → refresh token → upstream send, with same-request failover.
///
/// Happy path (first account OK): one pick + one send — same cost as before.
///
/// When `allow_files_offload` is true and the body is large, huge text blobs
/// are uploaded to the xAI Files API on the **selected** account and replaced
/// with `file_id` references before the upstream call.
async fn send_with_account_failover<F>(
    config: &AppConfig,
    http: &reqwest::Client,
    url: &str,
    build_upstream: F,
    body: Bytes,
    session_key: Option<&str>,
    allow_files_offload: bool,
) -> AppResult<(crate::config::Account, String, reqwest::Response)>
where
    F: Fn(&reqwest::Client, &str, Bytes) -> reqwest::RequestBuilder,
{
    let mut excluded: Vec<String> = Vec::new();
    let mut last_transport: Option<String> = None;
    let capability = MediaCapability::from_upstream_path(url);
    // Bound tries by currently routable accounts so we do not re-pick cooled-down ones.
    let initial_store = load_auth()?;
    let max_tries = routable_account_count_cap(&initial_store, capability)
        .min(MAX_ACCOUNT_TRIES)
        .max(1);

    for attempt in 0..max_tries {
        let store = if attempt == 0 {
            initial_store.clone()
        } else {
            load_auth()?
        };

        let decision = match pick_account_decision_cap(
            config,
            &store,
            &excluded,
            session_key,
            capability,
        ) {
            Ok(d) => d,
            Err(err) => {
                if let Some(detail) = last_transport.take() {
                    let hint = crate::http_client::proxy_status_hint(config);
                    return Err(AppError::msg(format!(
                        "无法连接上游 {url}: {detail}。{hint}"
                    )));
                }
                return Err(err);
            }
        };
        let mut account = decision.account;
        excluded.push(account.id.clone());
        // Soft in-flight counter for this attempt (released when permit drops).
        let _permit = AccountPermit::acquire(&account.id);

        tracing::debug!(
            account = %account.id,
            layer = decision.layer,
            sticky = decision.sticky_hit,
            attempt,
            session = session_key.unwrap_or(""),
            "account selected"
        );

        let token_before = account.access_token.clone();
        let mut token = match ensure_fresh_token(config, &mut account).await {
            Ok(t) => t,
            Err(err) => {
                tracing::warn!(
                    account = %account.id,
                    attempt,
                    "token refresh failed; trying next account: {err}"
                );
                mark_failure_kind(&mut account, FailureKind::Unauthorized);
                let _ = replace_account_tokens(&account);
                if let Some(key) = session_key {
                    session_affinity::invalidate(key);
                }
                last_transport = Some(format!("token refresh failed: {err}"));
                continue;
            }
        };
        if account.access_token != token_before {
            replace_account_tokens(&account)?;
        }

        // Per-account Files offload: only when body is heavy enough to matter.
        // Failures fall back to already-truncated sync optimize (never block the turn).
        // Only attempt Files API offload when body is large enough that a 32k+
        // text blob could exist (cheap gate before JSON parse + uploads).
        let send_body = if allow_files_offload && body.len() >= OFFLOAD_TEXT_MIN {

            match serde_json::from_slice::<Value>(&body) {
                Ok(mut value) => {
                    match offload_large_text_blobs(
                        &mut value,
                        http,
                        &config.xai_base_url,
                        &token,
                        &account.id,
                    )
                    .await
                    {
                        Ok(stats) => {
                            if stats.modified {
                                stats.log_summary("files-offload");
                                Bytes::from(
                                    serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec()),
                                )
                            } else {
                                body.clone()
                            }
                        }
                        Err(err) => {
                            tracing::warn!(
                                account = %account.id,
                                "files offload skipped: {err}"
                            );
                            body.clone()
                        }
                    }
                }
                Err(_) => body.clone(),
            }
        } else {
            body.clone()
        };

        let mut upstream = build_upstream(http, &token, send_body.clone()).send().await;
        if let Ok(resp) = &upstream {
            if resp.status() == StatusCode::UNAUTHORIZED {
                // Same-account refresh once, then re-send (not a full failover yet).
                let before = account.access_token.clone();
                match ensure_fresh_token(config, &mut account).await {
                    Ok(t) => {
                        token = t;
                        if account.access_token != before {
                            let _ = replace_account_tokens(&account);
                        }
                        upstream = build_upstream(http, &token, send_body.clone()).send().await;
                    }
                    Err(err) => {
                        tracing::warn!(account = %account.id, "401 refresh failed: {err}");
                        mark_failure_kind(&mut account, FailureKind::Unauthorized);
                        let _ = replace_account_tokens(&account);
                        if let Some(key) = session_key {
                            session_affinity::invalidate(key);
                        }
                        last_transport = Some(format!("401 refresh failed: {err}"));
                        continue;
                    }
                }
            }
        }

        match upstream {
            Ok(resp) => {
                let status = resp.status();
                let can_failover = status_is_account_failover(status) && attempt + 1 < max_tries;
                if can_failover {
                    let more = load_auth()
                        .ok()
                        .and_then(|s| {
                            pick_account_decision_cap(
                                config,
                                &s,
                                &excluded,
                                session_key,
                                capability,
                            )
                            .ok()
                            .map(|_| ())
                        })
                        .is_some();
                    if more {
                        let headers = resp.headers().clone();
                        let body_bytes = resp.bytes().await.unwrap_or_default();
                        apply_status_failure(&mut account, status, &headers);
                        let _ = replace_account_tokens(&account);
                        if let Some(key) = session_key {
                            session_affinity::invalidate(key);
                        }
                        let preview: String = String::from_utf8_lossy(&body_bytes)
                            .chars()
                            .take(160)
                            .collect();
                        tracing::info!(
                            account = %account.id,
                            %status,
                            attempt,
                            layer = decision.layer,
                            "upstream account-scoped failure; failing over. body={preview}"
                        );
                        last_transport = Some(format!("upstream {status}: {preview}"));
                        continue;
                    }
                }
                if status.is_success() {
                    if let Some(key) = session_key {
                        if config.session_affinity {
                            session_affinity::bind(
                                key,
                                &account.id,
                                config.session_affinity_ttl_secs,
                            );
                        }
                    }
                }
                if attempt > 0 {
                    tracing::info!(
                        account = %account.id,
                        attempt,
                        %status,
                        sticky = decision.sticky_hit,
                        "account failover succeeded"
                    );
                }
                return Ok((account, token, resp));
            }
            Err(err) => {
                mark_failure_kind(&mut account, FailureKind::Transport);
                let _ = replace_account_tokens(&account);
                let detail = crate::http_client::format_reqwest_error(&err);
                tracing::warn!(
                    account = %account.id,
                    attempt,
                    "transport error; trying next account if any: {detail}"
                );
                last_transport = Some(detail);
                continue;
            }
        }
    }

    if let Some(detail) = last_transport {
        let hint = crate::http_client::proxy_status_hint(config);
        return Err(AppError::msg(format!(
            "无法连接上游 {url}: {detail}。{hint}"
        )));
    }
    Err(AppError::msg(format!(
        "无法连接上游 {url}: all account attempts failed"
    )))
}

fn apply_status_failure(
    account: &mut crate::config::Account,
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
) {
    apply_rate_limit_headers(account, headers);
    let code = status.as_u16();
    if code == 429 {
        let secs = retry_after_secs(headers).unwrap_or(60);
        mark_failure_kind(account, FailureKind::RateLimit { retry_after_secs: secs });
    } else if code == 401 {
        mark_failure_kind(account, FailureKind::Unauthorized);
    } else if code == 403 {
        mark_failure_kind(account, FailureKind::Forbidden);
    } else if status.is_server_error() {
        mark_failure_kind(account, FailureKind::Soft);
    } else {
        // Other 4xx (validation etc.) — don't cooldown the account.
        mark_failure_kind(account, FailureKind::Soft);
        if account.consecutive_failures > 0 {
            account.consecutive_failures = account.consecutive_failures.saturating_sub(1);
        }
        if let Some(err) = account.last_upstream_error.as_mut() {
            *err = format!("upstream {code}");
        } else {
            account.last_upstream_error = Some(format!("upstream {code}"));
        }
    }
}

/// Pull token usage from xAI / OpenAI-shaped Responses or Chat payloads.
///
/// xAI Responses (confirmed by sub2api fixtures) uses:
/// `usage.input_tokens_details.cached_tokens`
/// not Anthropic-style `cache_read_input_tokens`.
fn extract_usage_tokens(value: &Value) -> (u64, u64, u64) {
    let usage = value
        .get("usage")
        .or_else(|| value.pointer("/response/usage"));
    let Some(usage) = usage else {
        return (0, 0, 0);
    };
    let input = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache = usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens"))
        .or_else(|| usage.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    (input, output, cache)
}

/// Scans SSE chunks for `response.completed` usage and logs once when dropped.
struct StreamUsageTracker {
    request_id: String,
    account_id: Option<String>,
    endpoint: String,
    requested_model: Option<String>,
    resolved_model: Option<String>,
    status_code: u16,
    latency_ms: u64,
    client_source: String,
    session_affinity: bool,
    session_affinity_ttl_secs: u64,
    account_id_for_bind: String,
    input: AtomicU64,
    output: AtomicU64,
    cache: AtomicU64,
    logged: AtomicBool,
    /// Carry incomplete UTF-8 / partial SSE lines across chunks.
    pending: parking_lot::Mutex<String>,
}

impl StreamUsageTracker {
    #[allow(clippy::too_many_arguments)]
    fn new(
        request_id: String,
        account_id: Option<String>,
        endpoint: String,
        requested_model: Option<String>,
        resolved_model: Option<String>,
        status_code: u16,
        latency_ms: u64,
        client_source: String,
        session_affinity: bool,
        session_affinity_ttl_secs: u64,
        account_id_for_bind: String,
    ) -> Self {
        Self {
            request_id,
            account_id,
            endpoint,
            requested_model,
            resolved_model,
            status_code,
            latency_ms,
            client_source,
            session_affinity,
            session_affinity_ttl_secs,
            account_id_for_bind,
            input: AtomicU64::new(0),
            output: AtomicU64::new(0),
            cache: AtomicU64::new(0),
            logged: AtomicBool::new(false),
            pending: parking_lot::Mutex::new(String::new()),
        }
    }

    fn note_chunk(&self, bytes: &[u8]) {
        let chunk = String::from_utf8_lossy(bytes);
        let mut pending = self.pending.lock();
        pending.push_str(&chunk);
        // Process complete lines; keep trailing partial line.
        while let Some(pos) = pending.find('\n') {
            let line = pending[..pos].trim_end_matches('\r').to_string();
            *pending = pending[pos + 1..].to_string();
            self.note_sse_line(&line);
        }
        // Bound pending buffer against pathological streams.
        if pending.len() > 512 * 1024 {
            pending.clear();
        }
    }

    fn note_sse_line(&self, line: &str) {
        let data = if let Some(rest) = line.strip_prefix("data:") {
            rest.trim()
        } else if line.starts_with('{') {
            line
        } else {
            return;
        };
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        // Bind new response id for sticky chain when present.
        if let Some(rid) = value
            .pointer("/response/id")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("id").and_then(|v| v.as_str()))
        {
            if !rid.is_empty()
                && value
                    .get("type")
                    .and_then(|t| t.as_str())
                    .map(|t| t == "response.completed" || t.ends_with(".completed"))
                    .unwrap_or(true)
            {
                session_affinity::bind_response_chain(
                    rid,
                    &self.account_id_for_bind,
                    self.session_affinity_ttl_secs,
                );
            }
        }
        let (i, o, c) = extract_usage_tokens(&value);
        // Also nested under event.response.usage when type is response.completed
        let (i2, o2, c2) = value
            .get("response")
            .map(extract_usage_tokens)
            .unwrap_or((0, 0, 0));
        let input = i.max(i2);
        let output = o.max(o2);
        let cache = c.max(c2);
        if input > 0 {
            self.input.fetch_max(input, AtomicOrdering::Relaxed);
        }
        if output > 0 {
            self.output.fetch_max(output, AtomicOrdering::Relaxed);
        }
        if cache > 0 {
            self.cache.fetch_max(cache, AtomicOrdering::Relaxed);
        }
        let _ = self.session_affinity;
    }

    fn finish(&self) {
        if self.logged.swap(true, AtomicOrdering::SeqCst) {
            return;
        }
        let input = self.input.load(AtomicOrdering::Relaxed);
        let output = self.output.load(AtomicOrdering::Relaxed);
        let cache = self.cache.load(AtomicOrdering::Relaxed);
        if cache > 0 {
            tracing::debug!(
                input,
                output,
                cache,
                "stream prompt cache hit recorded"
            );
        }
        log_request(
            &self.request_id,
            self.account_id.clone(),
            &self.endpoint,
            self.requested_model.clone(),
            self.resolved_model.clone(),
            self.status_code,
            self.latency_ms,
            None,
            input,
            output,
            cache,
            None,
            &self.client_source,
            None,
        );
    }
}

impl Drop for StreamUsageTracker {
    fn drop(&mut self) {
        self.finish();
    }
}

fn log_request(
    request_id: &str,
    account_id: Option<String>,
    endpoint: &str,
    requested_model: Option<String>,
    resolved_model: Option<String>,
    status_code: u16,
    latency_ms: u64,
    first_token_ms: Option<u64>,
    input_tokens: u64,
    output_tokens: u64,
    cache_tokens: u64,
    error_summary: Option<String>,
    client_source: &str,
    _mapping_reason: Option<String>,
) {
    // Async bounded queue — never open SQLite on the request path.
    enqueue_request_log(RequestLog {
        request_id: request_id.to_string(),
        account_id,
        endpoint: endpoint.to_string(),
        requested_model,
        resolved_model,
        status_code,
        latency_ms,
        first_token_ms,
        input_tokens,
        output_tokens,
        cache_tokens,
        estimated_cost_usd: estimate_cost(input_tokens, output_tokens, cache_tokens),
        error_summary,
        client_source: client_source.to_string(),
        created_at: Utc::now(),
    });
}

#[cfg(test)]
mod build_plane_tests {
    use super::{is_grok_build_plane, resolve_upstream_base};
    use crate::config::AppConfig;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn detects_xai_token_auth_and_model_override() {
        let mut h = HeaderMap::new();
        h.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
        assert!(is_grok_build_plane(&h));

        let mut h2 = HeaderMap::new();
        h2.insert("x-grok-model-override", HeaderValue::from_static("grok-build"));
        assert!(is_grok_build_plane(&h2));

        let plain = HeaderMap::new();
        assert!(!is_grok_build_plane(&plain));
    }

    #[test]
    fn upstream_base_uses_cli_chat_proxy_for_build_plane() {
        let mut cfg = AppConfig::default();
        cfg.xai_base_url = "https://api.x.ai/v1".into();
        cfg.cli_chat_proxy_base_url = "https://cli-chat-proxy.grok.com/v1".into();
        let mut h = HeaderMap::new();
        h.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
        assert_eq!(
            resolve_upstream_base(&cfg, &h),
            "https://cli-chat-proxy.grok.com/v1"
        );
        let plain = HeaderMap::new();
        assert_eq!(resolve_upstream_base(&cfg, &plain), "https://api.x.ai/v1");
    }

    #[test]
    fn build_plane_sanitize_preserves_continuity_for_cache() {
        use crate::gateway::sanitize::sanitize_responses_request_ex;
        use serde_json::json;
        let mut body = json!({
            "previous_response_id": "resp_native",
            "prompt_cache_key": "stable-thread",
            "prompt_cache_retention": "24h",
            "model": "grok-4.5",
            "input": [{"role": "user", "content": "hi"}],
            "tools": []
        });
        sanitize_responses_request_ex(&mut body, true);
        assert_eq!(
            body.get("previous_response_id").and_then(|v| v.as_str()),
            Some("resp_native")
        );
        assert_eq!(
            body.get("prompt_cache_key").and_then(|v| v.as_str()),
            Some("stable-thread")
        );
        assert_eq!(
            body.get("prompt_cache_retention").and_then(|v| v.as_str()),
            Some("24h")
        );
    }

    #[test]
    fn session_key_from_grok_conv_header_is_stable_cache_key() {
        use crate::session_affinity;
        let mut h = HeaderMap::new();
        h.insert("x-grok-conv-id", HeaderValue::from_static("cli-conv-42"));
        let key = session_affinity::extract_session_key(&h, None).unwrap();
        assert_eq!(
            session_affinity::stable_cache_key(&key).as_deref(),
            Some("cli-conv-42")
        );
    }

    #[test]
    fn build_plane_injects_client_version_when_missing() {
        use super::{collect_build_plane_passthrough_headers, DEFAULT_GROK_CLIENT_VERSION};
        let h = HeaderMap::new();
        let headers = collect_build_plane_passthrough_headers(&h);
        let ver = headers
            .iter()
            .find(|(n, _)| n.as_str().eq_ignore_ascii_case("x-grok-client-version"))
            .map(|(_, v)| v.as_str());
        assert_eq!(ver, Some(DEFAULT_GROK_CLIENT_VERSION));
    }

    #[test]
    fn build_plane_preserves_client_version_when_present() {
        use super::collect_build_plane_passthrough_headers;
        let mut h = HeaderMap::new();
        h.insert(
            "x-grok-client-version",
            HeaderValue::from_static("0.2.200"),
        );
        let headers = collect_build_plane_passthrough_headers(&h);
        let ver = headers
            .iter()
            .find(|(n, _)| n.as_str().eq_ignore_ascii_case("x-grok-client-version"))
            .map(|(_, v)| v.as_str());
        assert_eq!(ver, Some("0.2.200"));
    }

    #[test]
    fn build_plane_forwards_non_prefixed_client_headers() {
        use super::collect_build_plane_passthrough_headers;
        let mut h = HeaderMap::new();
        h.insert("user-agent", HeaderValue::from_static("xai-grok-shell/0.2.101"));
        h.insert("x-email", HeaderValue::from_static("a@b.com"));
        h.insert("x-models-etag", HeaderValue::from_static("etag-1"));
        h.insert("accept-language", HeaderValue::from_static("zh-CN"));
        h.insert("traceparent", HeaderValue::from_static("00-abc-def-01"));
        // Must never pass through.
        h.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );
        let headers = collect_build_plane_passthrough_headers(&h);
        let names: Vec<_> = headers
            .iter()
            .map(|(n, _)| n.as_str().to_ascii_lowercase())
            .collect();
        assert!(names.iter().any(|n| n == "user-agent"));
        assert!(names.iter().any(|n| n == "x-email"));
        assert!(names.iter().any(|n| n == "x-models-etag"));
        assert!(names.iter().any(|n| n == "accept-language"));
        assert!(names.iter().any(|n| n == "traceparent"));
        assert!(!names.iter().any(|n| n == "authorization"));
        let ua = headers
            .iter()
            .find(|(n, _)| n.as_str().eq_ignore_ascii_case("user-agent"))
            .map(|(_, v)| v.as_str());
        assert_eq!(ua, Some("xai-grok-shell/0.2.101"));
    }

    #[test]
    fn aligns_user_profile_identity_without_inventing_supergrok_tier() {
        use super::{is_user_profile_path, rewrite_user_profile_for_build_gate};
        use serde_json::json;
        assert!(is_user_profile_path("/user?include=subscription"));
        assert!(is_user_profile_path("/user"));
        assert!(!is_user_profile_path("/responses"));

        // Unsigned JWT payload {"principal_id":"sess-1","sub":"sess-1"}
        let jwt = "eyJhbGciOiJub25lIn0.eyJwcmluY2lwYWxfaWQiOiJzZXNzLTEiLCJzdWIiOiJzZXNzLTEifQ.sig";
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {jwt}")).unwrap(),
        );
        let mut value = json!({
            "userId": "pool-user",
            "principalId": "pool-user",
            "subscriptionTiers": "XPremiumPlus",
            "hasGrokCodeAccess": true
        });
        assert!(rewrite_user_profile_for_build_gate(&mut value, &h));
        assert_eq!(value.get("userId").and_then(|v| v.as_str()), Some("sess-1"));
        assert_eq!(
            value.get("subscriptionTiers").and_then(|v| v.as_str()),
            Some("XPremiumPlus"),
            "must keep real API subscription enum"
        );
    }
}

#[cfg(test)]
mod usage_extract_tests {
    use super::extract_usage_tokens;
    use serde_json::json;

    #[test]
    fn extracts_xai_input_tokens_details_cached() {
        let v = json!({
            "usage": {
                "input_tokens": 100,
                "output_tokens": 5,
                "input_tokens_details": { "cached_tokens": 80 }
            }
        });
        assert_eq!(extract_usage_tokens(&v), (100, 5, 80));
    }

    #[test]
    fn extracts_nested_response_usage() {
        let v = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 50,
                    "output_tokens": 2,
                    "input_tokens_details": { "cached_tokens": 40 }
                }
            }
        });
        assert_eq!(extract_usage_tokens(&v), (50, 2, 40));
    }

    #[test]
    fn extracts_openai_prompt_tokens_details() {
        let v = json!({
            "usage": {
                "prompt_tokens": 20,
                "completion_tokens": 3,
                "prompt_tokens_details": { "cached_tokens": 10 }
            }
        });
        assert_eq!(extract_usage_tokens(&v), (20, 3, 10));
    }
}

fn response_to_text(resp: Response) -> String {
    format!("status {}", resp.status())
}

pub async fn list_models_response(config: &AppConfig) -> Value {
    // Prefer upstream models, fallback to curated list
    if let Ok(value) = fetch_upstream_models(config).await {
        return value;
    }
    curated_models(config)
}

/// Fetch raw OpenAI-style `/models` payload from xAI when auth + network allow.
pub async fn fetch_upstream_models(config: &AppConfig) -> AppResult<Value> {
    let client = build_http_client(config)?;
    let store = load_auth()?;
    let token = store
        .accounts
        .iter()
        .find_map(|a| a.access_token.clone())
        .ok_or_else(|| AppError::msg("no access token for models list"))?;
    let resp = client
        .get(format!(
            "{}/models",
            config.xai_base_url.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(AppError::msg(format!(
            "upstream models list failed: {}",
            resp.status()
        )));
    }
    Ok(resp.json().await?)
}

fn curated_models(config: &AppConfig) -> Value {
    use crate::config::known_xai_text_models;
    let mut data = Vec::new();
    let mut seen = std::collections::HashSet::<String>::new();
    let mut push = |id: &str, modality: &str| {
        if id.is_empty() || !seen.insert(id.to_string()) {
            return;
        }
        data.push(json!({
            "id": id,
            "object": "model",
            "owned_by": "xai",
            "modality": modality,
        }));
    };
    push(&config.default_model, "text");
    for id in known_xai_text_models() {
        push(id, "text");
    }
    push(&config.default_image_model, "image");
    push("grok-imagine-image", "image");
    push(&config.default_video_model, "video");
    push("grok-imagine-video-1.5", "video");
    json!({ "object": "list", "data": data })
}
