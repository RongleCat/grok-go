use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderName, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use chrono::Utc;
use futures_util::StreamExt;
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use crate::auth::{
    apply_rate_limit_headers, ensure_fresh_token, mark_failure_kind, mark_success, retry_after_secs,
    FailureKind,
};
use crate::config::{load_auth, load_config, resolve_model, AppConfig};
use crate::error::{AppError, AppResult};
use crate::gateway::image_bridge::{
    collect_image_gen_calls, fulfill_image_gen_call, inject_image_generation_calls,
    MAX_IMAGE_TOOL_ROUNDS,
};
use crate::gateway::sanitize::{
    is_compaction_blob_error, is_model_input_error, rewrite_responses_payload,
    rewrite_sse_data_line, sanitize_responses_request, strip_opaque_context,
};
use crate::http_client::build_http_client;
use crate::router::{
    pick_account_excluding, replace_account_tokens, routable_account_count, touch_account_cache,
};
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
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let token = auth.strip_prefix("Bearer ").unwrap_or(auth).trim();
    if token == config.local_token {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            json!({"error": {"message": "invalid local bearer token", "type": "auth_error"}}).to_string(),
        )
            .into_response())
    }
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

    if matches!(method, Method::POST | Method::PUT | Method::PATCH) && !outbound_body.is_empty() {
        if let Ok(mut value) = serde_json::from_slice::<Value>(&outbound_body) {
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
            if is_responses {
                let sanitized = sanitize_responses_request(&mut value);
                custom_tool_names = sanitized.custom_tool_names;
                has_image_gen_tools = sanitized.has_image_gen_tools;
                if sanitized.modified {
                    body_changed = true;
                }
                // Image tool loop needs a full JSON response; force non-stream upstream.
                if has_image_gen_tools && value.get("stream").and_then(|v| v.as_bool()) == Some(true)
                {
                    value["stream"] = Value::Bool(false);
                    body_changed = true;
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
    let url = format!("{}{}", config.xai_base_url.trim_end_matches('/'), path);
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

    let build_upstream = |client: &reqwest::Client, token: &str, body: Bytes| {
        let mut req = client
            .request(method.clone(), &url)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::ACCEPT, accept.as_str());
        if !body.is_empty() || matches!(method, Method::POST | Method::PUT | Method::PATCH) {
            // Force a single JSON content type for mutating requests (even empty body).
            req = req
                .header(header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        req
    };

    // Single-account happy path is unchanged (one pick + one send).
    // On account-scoped failures (401/403/429/5xx/transport), try other accounts
    // inside this request so the client does not see several hard failures in a row.
    let (mut account, token, upstream) = send_with_account_failover(
        &config,
        &http,
        &url,
        &build_upstream,
        outbound_body.clone(),
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
        } else {
            apply_status_failure(&mut account, status, &upstream_headers);
            let _ = replace_account_tokens(&account);
        }
        let custom_names = custom_tool_names.clone();
        let stream = upstream.bytes_stream().map(move |chunk| {
            chunk
                .map(|bytes| rewrite_sse_chunk(&bytes, &custom_names))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        });
        let body = Body::from_stream(stream);
        log_request(
            &request_id,
            Some(account.id.clone()),
            path,
            requested_model.clone(),
            resolved_model.clone(),
            status.as_u16(),
            latency_ms,
            None,
            0,
            0,
            0,
            None,
            client_source,
            mapping_reason.clone(),
        );
        let mut response = Response::builder().status(status).body(body).unwrap_or_else(|_| Response::new(Body::empty()));
        copy_safe_headers(upstream_headers, response.headers_mut());
        return Ok(response);
    }

    let mut bytes = upstream.bytes().await?;

    // Retry once when xAI rejects opaque context or input item shapes (Codex multi-turn).
    if !status.is_success() {
        let err_text = String::from_utf8_lossy(&bytes);
        if is_compaction_blob_error(&err_text) || is_model_input_error(&err_text) {
            if let Some(mut req_value) = parsed_request.clone() {
                // Nuclear strip first, then re-sanitize tools / custom_tool_call shapes.
                let _ = strip_opaque_context(&mut req_value);
                let _ = sanitize_responses_request(&mut req_value);
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
        bytes = Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| bytes.to_vec()));

        if let Some(usage) = value.get("usage") {
            input_tokens = usage.get("input_tokens").or_else(|| usage.get("prompt_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
            output_tokens = usage.get("output_tokens").or_else(|| usage.get("completion_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
            cache_tokens = usage
                .get("cache_read_input_tokens")
                .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }
    }
    let error_summary = if status.is_success() {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes).chars().take(500).collect())
    };
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
        client_source,
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
async fn send_with_account_failover<F>(
    config: &AppConfig,
    http: &reqwest::Client,
    url: &str,
    build_upstream: F,
    body: Bytes,
) -> AppResult<(crate::config::Account, String, reqwest::Response)>
where
    F: Fn(&reqwest::Client, &str, Bytes) -> reqwest::RequestBuilder,
{
    let mut excluded: Vec<String> = Vec::new();
    let mut last_transport: Option<String> = None;
    // Bound tries by currently routable accounts so we do not re-pick cooled-down ones.
    let initial_store = load_auth()?;
    let max_tries = routable_account_count(&initial_store)
        .min(MAX_ACCOUNT_TRIES)
        .max(1);

    for attempt in 0..max_tries {
        let store = if attempt == 0 {
            initial_store.clone()
        } else {
            load_auth()?
        };

        let mut account = match pick_account_excluding(config, &store, &excluded) {
            Ok(a) => a,
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
        excluded.push(account.id.clone());

        let token_before = account.access_token.clone();
        let mut token = match ensure_fresh_token(config, &mut account).await {
            Ok(t) => t,
            Err(err) => {
                tracing::warn!(
                    account = %account.id,
                    attempt,
                    "token refresh failed; trying next account: {err}"
                );
                mark_failure_kind(&mut account, FailureKind::Auth);
                let _ = replace_account_tokens(&account);
                last_transport = Some(format!("token refresh failed: {err}"));
                continue;
            }
        };
        if account.access_token != token_before {
            replace_account_tokens(&account)?;
        }

        let mut upstream = build_upstream(http, &token, body.clone()).send().await;
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
                        upstream = build_upstream(http, &token, body.clone()).send().await;
                    }
                    Err(err) => {
                        tracing::warn!(account = %account.id, "401 refresh failed: {err}");
                        mark_failure_kind(&mut account, FailureKind::Auth);
                        let _ = replace_account_tokens(&account);
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
                    // Peek whether another account exists before consuming the body away
                    // from a final error response… we always consume for failure marking.
                    let more = load_auth()
                        .ok()
                        .and_then(|s| pick_account_excluding(config, &s, &excluded).ok())
                        .is_some();
                    if more {
                        let headers = resp.headers().clone();
                        let body_bytes = resp.bytes().await.unwrap_or_default();
                        apply_status_failure(&mut account, status, &headers);
                        let _ = replace_account_tokens(&account);
                        let preview: String = String::from_utf8_lossy(&body_bytes)
                            .chars()
                            .take(160)
                            .collect();
                        tracing::info!(
                            account = %account.id,
                            %status,
                            attempt,
                            "upstream account-scoped failure; failing over. body={preview}"
                        );
                        last_transport = Some(format!("upstream {status}: {preview}"));
                        continue;
                    }
                }
                if attempt > 0 {
                    tracing::info!(
                        account = %account.id,
                        attempt,
                        %status,
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
    } else if code == 401 || code == 403 {
        mark_failure_kind(account, FailureKind::Auth);
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
    json!({
        "object": "list",
        "data": [
            {"id": config.default_model, "object": "model", "owned_by": "xai", "modality": "text"},
            {"id": "grok-4.20-reasoning", "object": "model", "owned_by": "xai", "modality": "text"},
            {"id": config.default_image_model, "object": "model", "owned_by": "xai", "modality": "image"},
            {"id": config.default_video_model, "object": "model", "owned_by": "xai", "modality": "video"}
        ]
    })
}
