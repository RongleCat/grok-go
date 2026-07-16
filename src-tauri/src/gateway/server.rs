use axum::body::Body;
use axum::extract::{DefaultBodyLimit, OriginalUri, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};

/// Codex often attaches multiple design mockups as base64 `input_image` on `/v1/responses`.
/// Axum's default 2 MiB limit rejects those with 413 and the agent turn dies empty.
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;
use bytes::Bytes;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};

use crate::config::{load_auth, load_config, save_config, Account, AppConfig};
use crate::error::{AppError, AppResult};
use crate::gateway::proxy::{
    authorize_request, list_models_response, proxy_anthropic_count_tokens, proxy_anthropic_messages,
    proxy_json, ProxyContext,
};
use crate::auth::ensure_fresh_token;
use crate::router::{pick_account_for, replace_account_tokens, MediaCapability};
use crate::usage::{estimate_cost, RequestLog};
use crate::gateway::media_artifacts::{
    materialize_image_response, materialize_video_response, media_summary, mcp_media_content,
    poll_video_result, resolve_media_url,
};
use std::time::Duration;
use chrono::Utc;
use uuid::Uuid;

#[derive(Clone)]
pub struct GatewayState {
    pub proxy: ProxyContext,
    pub running: Arc<Mutex<bool>>,
    pub actual_addr: Arc<Mutex<Option<SocketAddr>>>,
}

impl GatewayState {
    pub fn new() -> Self {
        Self {
            proxy: ProxyContext::new(),
            running: Arc::new(Mutex::new(false)),
            actual_addr: Arc::new(Mutex::new(None)),
        }
    }
}

pub async fn start_gateway(state: GatewayState) -> AppResult<SocketAddr> {
    let mut config = load_config()?;
    let host = if config.lan_enabled {
        if config.bind_host == "127.0.0.1" {
            "0.0.0.0".to_string()
        } else {
            config.bind_host.clone()
        }
    } else {
        "127.0.0.1".to_string()
    };

    let mut port = config.preferred_port;
    let listener = loop {
        let addr = format!("{host}:{port}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => break listener,
            Err(_) => {
                port = port.saturating_add(1);
                if port > config.preferred_port + 50 {
                    return Err(AppError::msg("unable to bind gateway port"));
                }
            }
        }
    };
    let addr = listener.local_addr()?;
    config.actual_port = addr.port();
    config.bind_host = if config.lan_enabled { host } else { "127.0.0.1".into() };
    save_config(&config)?;
    {
        *state.running.lock().await = true;
        *state.actual_addr.lock().await = Some(addr);
    }

    let app = build_router(state.clone());
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            tracing::error!("gateway stopped: {err}");
        }
        *state.running.lock().await = false;
    });
    Ok(addr)
}

/// Build the gateway Axum router (also used by live integration tests).
pub fn build_router(state: GatewayState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        // Grok Build TUI paywall / GrowthBook + remote config (cli-chat-proxy).
        .route("/v1/user", get(build_plane_get))
        .route("/v1/settings", get(build_plane_get))
        .route("/v1/login-config", get(build_plane_get))
        .route("/v1/subagents/bundle", get(build_plane_get))
        .route("/v1/responses", post(responses))
        .route("/v1/responses/compact", post(responses_compact))
        .route("/v1/chat/completions", post(chat_completions))
        // Anthropic Messages API (Claude Code via ANTHROPIC_BASE_URL).
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/messages/count_tokens", post(anthropic_count_tokens))
        .route("/v1/images/generations", post(image_generations))
        .route("/v1/images/edits", post(image_edits))
        .route("/v1/videos/generations", post(video_generations))
        .route("/v1/videos/edits", post(video_edits))
        // Deferred video job poll (account-sticky via job_affinity).
        .route("/v1/videos/{request_id}", get(video_job_status))
        // O-02: simple tools HTTP API (same backend as MCP tools/call).
        .route("/v1/tools/{name}", post(tools_http_call))
        // xAI Files API proxy — upload once, reference by file_id in Responses.
        .route("/v1/files", any(files_collection))
        .route("/v1/files/{file_id}", any(files_item))
        .route("/mcp", any(mcp_endpoint))
        .route("/mcp/", any(mcp_endpoint))
        .with_state(state)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
}

async fn health(State(state): State<GatewayState>) -> Json<Value> {
    let running = *state.running.lock().await;
    let addr = *state.actual_addr.lock().await;
    let config = load_config().unwrap_or_default();
    let store = load_auth().unwrap_or_default();
    let accounts_routable = crate::router::routable_account_count(&store);
    let accounts_total = store.accounts.len();
    Json(json!({
        "ok": true,
        "running": running,
        "addr": addr.map(|a| a.to_string()),
        "port": config.actual_port,
        "lanEnabled": config.lan_enabled,
        "requireToken": config.require_token,
        // O-18
        "accountsRoutable": accounts_routable,
        "accountsTotal": accounts_total,
        "experimentalImpersonateGrokBuild": config.experimental_impersonate_grok_build,
        "anthropicThinkingMode": config.anthropic_thinking_mode,
    }))
}

/// O-02: `POST /v1/tools/{name}` — body = tool arguments JSON object.
async fn tools_http_call(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    let config = match load_config() {
        Ok(c) => c,
        Err(err) => {
            return layered_error_response(
                crate::gateway::error_codes::classify_transport_error(&err.to_string()),
            );
        }
    };
    if let Err(resp) = authorize_request(&headers, &config).await {
        return resp;
    }
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(err) => {
                return layered_error_response(crate::gateway::error_codes::invalid_request(
                    format!("invalid JSON body: {err}"),
                    "Body must be a JSON object of tool arguments.",
                ));
            }
        }
    };
    let mut args = args;
    let _ = crate::gateway::tool_surface::coerce_mcp_tool_arguments(&mut args);
    let params = json!({"name": name, "arguments": args});
    match handle_tool_call(&state, &config, params).await {
        Ok(value) => {
            // Prefer structured envelope when present in MCP content text.
            if let Some(text) = value
                .pointer("/content/0/text")
                .and_then(|t| t.as_str())
            {
                if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                    if parsed.get("ok").is_some() {
                        return Json(parsed).into_response();
                    }
                }
            }
            Json(json!({
                "ok": true,
                "tool": name,
                "result": value,
            }))
            .into_response()
        }
        Err(err) => {
            let layered = crate::gateway::error_codes::classify_transport_error(&err.to_string());
            // Tool failures that aren't transport: wrap as TOOL_FAILED
            let layered = if layered.code == crate::gateway::error_codes::UPSTREAM_ERROR {
                crate::gateway::error_codes::tool_failed(&name, err.to_string())
            } else {
                layered
            };
            (
                StatusCode::from_u16(layered.status).unwrap_or(StatusCode::BAD_GATEWAY),
                Json(layered.tool_envelope(&name)),
            )
                .into_response()
        }
    }
}

async fn models(State(_state): State<GatewayState>, headers: HeaderMap) -> Response {
    let config = match load_config() {
        Ok(c) => c,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    if let Err(resp) = authorize_request(&headers, &config).await {
        return resp;
    }
    let value = list_models_response(&config).await;
    Json(value).into_response()
}

/// Proxy Grok Build remote-config GETs to cli-chat-proxy (preserve query string).
///
/// Critical paths observed in Grok Build 0.2.x:
/// - `/v1/user?include=subscription` — paywall subscription probe
/// - `/v1/settings` — GrowthBook-style remote settings including **`allow_access`**
/// - `/v1/login-config`, `/v1/subagents/bundle` — startup remote config
///
/// Missing `/settings` makes the client keep `allow_access=false` forever even when
/// `/user` returns a paid tier (logs: paywall_check_gate_kept_allow_access_false).
async fn build_plane_get(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    uri: OriginalUri,
) -> Response {
    let full = uri
        .0
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/v1/settings");
    let upstream_path = full.strip_prefix("/v1").unwrap_or(full);
    let upstream_path = if upstream_path.is_empty() {
        "/".to_string()
    } else if upstream_path.starts_with('/') {
        upstream_path.to_string()
    } else {
        format!("/{upstream_path}")
    };
    let source = if upstream_path.starts_with("/user") {
        "grok-build-user"
    } else if upstream_path.starts_with("/settings") {
        "grok-build-settings"
    } else {
        "grok-build-remote"
    };
    proxy_json(
        &state.proxy,
        Method::GET,
        &upstream_path,
        headers,
        Bytes::new(),
        source,
    )
    .await
}

async fn responses(State(state): State<GatewayState>, headers: HeaderMap, body: Bytes) -> Response {
    proxy_json(&state.proxy, Method::POST, "/responses", headers, body, "responses").await
}

async fn responses_compact(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // xAI context compaction endpoint used by multi-turn agents before tool loops.
    proxy_json(
        &state.proxy,
        Method::POST,
        "/responses/compact",
        headers,
        body,
        "responses-compact",
    )
    .await
}

async fn chat_completions(State(state): State<GatewayState>, headers: HeaderMap, body: Bytes) -> Response {
    proxy_json(&state.proxy, Method::POST, "/chat/completions", headers, body, "openai-compat").await
}

async fn anthropic_messages(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    proxy_anthropic_messages(&state.proxy, headers, body).await
}

async fn anthropic_count_tokens(headers: HeaderMap, body: Bytes) -> Response {
    proxy_anthropic_count_tokens(headers, body).await
}

async fn image_generations(State(state): State<GatewayState>, headers: HeaderMap, body: Bytes) -> Response {
    proxy_json(&state.proxy, Method::POST, "/images/generations", headers, body, "images").await
}

async fn image_edits(State(state): State<GatewayState>, headers: HeaderMap, body: Bytes) -> Response {
    proxy_json(&state.proxy, Method::POST, "/images/edits", headers, body, "images").await
}

async fn video_generations(State(state): State<GatewayState>, headers: HeaderMap, body: Bytes) -> Response {
    proxy_json(&state.proxy, Method::POST, "/videos/generations", headers, body, "videos").await
}

async fn video_edits(State(state): State<GatewayState>, headers: HeaderMap, body: Bytes) -> Response {
    proxy_json(&state.proxy, Method::POST, "/videos/edits", headers, body, "videos").await
}

/// `POST /v1/files` (multipart upload) or `GET /v1/files` (list).
async fn files_collection(
    State(state): State<GatewayState>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    files_proxy(&state, method, "/files", headers, body, None).await
}

/// `GET|DELETE /v1/files/{file_id}`.
async fn files_item(
    State(state): State<GatewayState>,
    method: Method,
    Path(file_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = format!("/files/{file_id}");
    files_proxy(&state, method, &path, headers, body, Some(file_id)).await
}

async fn files_proxy(
    state: &GatewayState,
    method: Method,
    path: &str,
    headers: HeaderMap,
    body: Bytes,
    _file_id: Option<String>,
) -> Response {
    let config = match load_config() {
        Ok(c) => c,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    if let Err(resp) = authorize_request(&headers, &config).await {
        return resp;
    }

    // Sticky account from session headers so multi-turn file_id refs stay valid.
    let session_key = crate::session_affinity::extract_session_key(&headers, None);
    let store = match load_auth() {
        Ok(s) => s,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    let decision = match crate::router::pick_account_decision_cap(
        &config,
        &store,
        &[],
        session_key.as_deref(),
        MediaCapability::Any,
    ) {
        Ok(d) => d,
        Err(err) => return error_response(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
    };
    let mut account = decision.account;
    let token = match ensure_fresh_token(&config, &mut account).await {
        Ok(t) => t,
        Err(err) => return error_response(StatusCode::UNAUTHORIZED, err.to_string()),
    };
    let _ = replace_account_tokens(&account);

    let http = state.proxy.client();
    let base = config.xai_base_url.trim_end_matches('/');

    // Multipart upload: forward raw body + content-type (do not re-encode as JSON).
    if method == Method::POST && path == "/files" {
        let ct = headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("multipart/form-data")
            .to_string();
        let url = format!("{base}/files");
        let resp = match http
            .post(&url)
            .bearer_auth(&token)
            .header(axum::http::header::CONTENT_TYPE, ct)
            .body(body.to_vec())
            .send()
            .await
        {
            Ok(r) => r,
            Err(err) => {
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    format!("files upload upstream: {err}"),
                );
            }
        };
        let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let bytes = resp.bytes().await.unwrap_or_default();
        // Bind session → account so later /responses with this file_id stay sticky.
        // (Do not content-hash the raw multipart envelope — boundaries make it useless.)
        if status.is_success() {
            if let Some(key) = session_key.as_ref() {
                if config.session_affinity {
                    crate::session_affinity::bind(
                        key,
                        &account.id,
                        config.session_affinity_ttl_secs,
                    );
                }
            }
        }
        return Response::builder()
            .status(status)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(bytes))
            .unwrap_or_else(|_| Response::new(Body::empty()));
    }

    let reqwest_method = match method {
        Method::GET => reqwest::Method::GET,
        Method::DELETE => reqwest::Method::DELETE,
        Method::POST => reqwest::Method::POST,
        other => {
            return error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                format!("files proxy does not support {other}"),
            );
        }
    };
    match crate::gateway::files_api::proxy_files_json(
        &http,
        &config.xai_base_url,
        &token,
        reqwest_method,
        path,
        None,
    )
    .await
    {
        Ok((code, value)) => {
            let status = StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY);
            (status, Json(value)).into_response()
        }
        Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
    }
}

/// Poll a deferred video job. Uses sticky account from submit when known;
/// otherwise tries enabled accounts until one owns the job (non-404).
async fn video_job_status(
    State(state): State<GatewayState>,
    Path(request_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let config = match load_config() {
        Ok(c) => c,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    if let Err(resp) = authorize_request(&headers, &config).await {
        return resp;
    }
    match poll_video_job_http(&state, &config, &request_id).await {
        Ok(value) => Json(value).into_response(),
        Err(err) => error_response(StatusCode::BAD_GATEWAY, err.to_string()),
    }
}

/// One-shot GET poll for HTTP clients (Codex/curl), with account affinity.
async fn poll_video_job_http(
    state: &GatewayState,
    config: &AppConfig,
    request_id: &str,
) -> AppResult<Value> {
    let store = load_auth()?;
    let preferred = crate::gateway::job_affinity::lookup_video_job_account(request_id);

    // Build try order: sticky owner first, then other enabled accounts.
    let mut ordered: Vec<Account> = Vec::new();
    if let Some(ref id) = preferred {
        if let Some(a) = store.accounts.iter().find(|a| &a.id == id) {
            ordered.push(a.clone());
        }
    }
    for a in &store.accounts {
        if a.enabled && a.is_credentialed() && !ordered.iter().any(|x| x.id == a.id) {
            ordered.push(a.clone());
        }
    }
    if ordered.is_empty() {
        return Err(AppError::msg(
            "no logged-in accounts available for video poll",
        ));
    }

    let client = state.proxy.client();
    let base = config.xai_base_url.trim_end_matches('/');
    let url = format!("{base}/videos/{request_id}");
    let mut last_err = String::new();

    for mut account in ordered {
        let before = account.access_token.clone();
        let token = match ensure_fresh_token(config, &mut account).await {
            Ok(t) => t,
            Err(err) => {
                last_err = format!("token refresh failed for {}: {err}", account.id);
                continue;
            }
        };
        if account.access_token != before {
            let _ = replace_account_tokens(&account);
        }

        let resp = client
            .get(&url)
            .bearer_auth(&token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| AppError::msg(format!("video poll failed: {e}")))?;
        let status = resp.status();
        let value: Value = resp.json().await.unwrap_or(json!({}));
        if status.as_u16() == 404 {
            // Wrong account for this job — try next.
            last_err = format!("404 on {}: {value}", account.id);
            continue;
        }
        if !status.is_success() {
            last_err = format!("HTTP {status} on {}: {value}", account.id);
            // Auth failures: try next account; hard errors still try others once.
            if matches!(status.as_u16(), 401 | 403) {
                continue;
            }
            return Err(AppError::msg(last_err));
        }
        // Pin owner for subsequent polls.
        crate::gateway::job_affinity::remember_video_job(request_id, &account.id);
        return Ok(value);
    }

    Err(AppError::msg(format!(
        "video poll failed for {request_id}: no account owns this job ({last_err})"
    )))
}

/// Preferred protocol versions for Codex / modern MCP clients.
const MCP_PROTOCOL_LATEST: &str = "2025-06-18";
const MCP_PROTOCOL_LEGACY: &str = "2024-11-05";

fn is_mcp_notification(payload: &Value, method: &str) -> bool {
    // JSON-RPC notifications omit `id`. Codex streamable HTTP clients also send
    // methods under the `notifications/*` namespace after initialize.
    if method.starts_with("notifications/") {
        return true;
    }
    !payload
        .as_object()
        .map(|object| object.contains_key("id"))
        .unwrap_or(false)
}

fn negotiated_protocol_version(payload: &Value) -> String {
    let requested = payload
        .pointer("/params/protocolVersion")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    match requested {
        MCP_PROTOCOL_LATEST | MCP_PROTOCOL_LEGACY => requested.to_string(),
        _ if !requested.is_empty() => requested.to_string(),
        _ => MCP_PROTOCOL_LATEST.to_string(),
    }
}

fn mcp_tools_catalog_all() -> Value {
    // Self-contained schemas: agents must call tools immediately without
    // web_search, codebase search, or reading source files for parameters.
    json!([
        {
            "name": "x_search",
            "description": "Search X (Twitter). CALL IMMEDIATELY — do NOT use web_search/shell/repo for X. Returns JSON text of posts.\n\nWHEN: any X/Twitter search, account posts, trending discussion.\nRETURNS: JSON text of matching posts/snippets.\nEXAMPLE: {\"query\":\"xAI Grok\",\"allowed_handles\":[\"xai\"],\"from_date\":\"2026-01-01\"}",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "REQUIRED. Search keywords/phrases. Example: \"xAI Grok launch\""
                    },
                    "allowed_handles": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "OPTIONAL. Only posts from these handles (no @). Example: [\"xai\",\"elonmusk\"]"
                    },
                    "excluded_handles": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "OPTIONAL. Exclude these handles (no @)."
                    },
                    "from_date": {
                        "type": "string",
                        "description": "OPTIONAL. Inclusive start date YYYY-MM-DD."
                    },
                    "to_date": {
                        "type": "string",
                        "description": "OPTIONAL. Inclusive end date YYYY-MM-DD."
                    }
                },
                "required": ["query"]
            },
            "annotations": {
                "title": "X Search",
                "readOnlyHint": true,
                "openWorldHint": true
            }
        },
        {
            "name": "image_gen",
            "description": "Generate an image (text-to-image). CALL IMMEDIATELY for any draw/generate/create image request. Do NOT search the web, codebase, or use SVG/Pillow.\n\nARGS (complete — nothing else needed):\n- prompt (string, REQUIRED): image description\n- n (integer 1-4, optional, default 1)\n- model (string, optional; default grok-imagine-image-quality)\n- size (string, optional e.g. 1024x1024; may be ignored)\n- quality (low|medium|high, optional)\n\nRETURNS JSON with absolute local paths only:\n- path / file: primary absolute path under ~/.grok-go/artifacts/\n- files: all local paths\n- markdown: ready ![image](/abs/path.png) for Codex rendering\nNever display remote CDN urls — always use path/markdown.\nEXAMPLE: {\"prompt\":\"a gray tabby kitten hugging an otter\"}",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "REQUIRED. Detailed image generation prompt (EN or ZH)."
                    },
                    "size": {
                        "type": "string",
                        "description": "OPTIONAL. Size hint e.g. \"1024x1024\". Upstream may ignore."
                    },
                    "quality": {
                        "type": "string",
                        "enum": ["low", "medium", "high"],
                        "description": "OPTIONAL. Quality hint: low | medium | high."
                    },
                    "n": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 4,
                        "default": 1,
                        "description": "OPTIONAL. Number of images (default 1, max 4)."
                    },
                    "model": {
                        "type": "string",
                        "description": "OPTIONAL. Model id. Default: grok-imagine-image-quality."
                    }
                },
                "required": ["prompt"]
            },
            "annotations": {
                "title": "Generate Image",
                "readOnlyHint": false,
                "openWorldHint": false
            }
        },
        {
            "name": "image_generate",
            "description": "Alias of image_gen. Identical args/return. Prefer image_gen. CALL IMMEDIATELY — do not search web/repo.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "REQUIRED. Same as image_gen.prompt."
                    },
                    "model": {
                        "type": "string",
                        "description": "OPTIONAL. Same as image_gen.model."
                    },
                    "n": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 4,
                        "default": 1,
                        "description": "OPTIONAL. Same as image_gen.n (default 1)."
                    },
                    "size": {
                        "type": "string",
                        "description": "OPTIONAL. Same as image_gen.size."
                    },
                    "quality": {
                        "type": "string",
                        "enum": ["low", "medium", "high"],
                        "description": "OPTIONAL. Same as image_gen.quality."
                    }
                },
                "required": ["prompt"]
            },
            "annotations": {
                "title": "Generate Image (alias)",
                "readOnlyHint": false
            }
        },
        {
            "name": "image_edit",
            "description": "Edit an existing image. CALL IMMEDIATELY — do NOT search web/repo for parameters.\n\nARGS (complete):\n- prompt (string, REQUIRED): edit instruction, e.g. \"make the sky sunset orange\"\n- image_url (string, REQUIRED): source image — accepts https:// URL, data: URL, absolute local path (/Users/.../x.png), or file:// path. Local paths are auto-inlined.\n- model (string, optional; default grok-imagine-image-quality)\n\nRETURNS: path/files/markdown absolute local paths (same shape as image_gen).\nEXAMPLE: {\"prompt\":\"add orange scarf\",\"image_url\":\"/Users/me/.grok-go/artifacts/img.png\"}",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "REQUIRED. What to change. Example: \"make the sky sunset orange\""
                    },
                    "image_url": {
                        "type": "string",
                        "description": "REQUIRED. Source image: https://… | data:image/…;base64,… | absolute local path | file://… Local paths supported."
                    },
                    "model": {
                        "type": "string",
                        "description": "OPTIONAL. Default: grok-imagine-image-quality."
                    }
                },
                "required": ["prompt", "image_url"]
            },
            "annotations": {
                "title": "Edit Image",
                "readOnlyHint": false
            }
        },
        {
            "name": "video_generate",
            "description": "Generate a video (text-to-video OR image-to-video OR multi-reference). CALL IMMEDIATELY — do NOT web_search or grep the repo for parameters. This schema is complete.\n\nMODES (pick one):\n1) Text-to-video: prompt only\n2) Image-to-video (图生视频 / animate still): prompt + image_url (starting frame)\n3) Reference-to-video: prompt + reference_image_urls (1–7 style/content refs). Do NOT combine image_url with reference_image_urls.\n\nARGS (complete):\n- prompt (string, REQUIRED): scene + motion description\n- image_url (string, optional): starting-frame image for image-to-video. Accepts https://, data:, absolute local path, file://\n- reference_image_urls (string[], optional): 1–7 reference images (same URL/path forms). Alternative to image_url\n- duration (number 1–15, optional): seconds (default upstream ~5–8)\n- aspect_ratio (string, optional): 1:1 | 16:9 | 9:16 | 4:3 | 3:4 | 3:2 | 2:3. Image-to-video defaults to source image AR if omitted\n- resolution (string, optional): 480p | 720p | 1080p (1080p only on some models / image-to-video)\n- model (string, optional; default grok-imagine-video)\n- wait (boolean, optional, default true): if false, return job_id immediately and poll GET /v1/videos/{id}\n\nBEHAVIOR: by default submits job, polls until done, downloads mp4 to ~/.grok-go/artifacts/, returns local path. With wait=false returns job_id + poll path without waiting.\nRETURNS JSON: path/file/files (absolute local mp4), markdown ![video](/abs/path.mp4). Never show remote CDN urls.\n\nEXAMPLES:\n- Text: {\"prompt\":\"cat running through tall grass at golden hour\",\"duration\":8,\"aspect_ratio\":\"16:9\",\"resolution\":\"720p\"}\n- Image-to-video: {\"prompt\":\"gentle camera push-in, soft wind in fur\",\"image_url\":\"/Users/me/.grok-go/artifacts/cat.png\",\"duration\":6}\n- Refs: {\"prompt\":\"cinematic fashion walk\",\"reference_image_urls\":[\"/tmp/a.jpg\",\"/tmp/b.jpg\"],\"duration\":6}\n- Async submit: {\"prompt\":\"ocean waves\",\"wait\":false}",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "REQUIRED. Scene + motion prompt (EN or ZH)."
                    },
                    "image_url": {
                        "type": "string",
                        "description": "OPTIONAL (image-to-video). Starting-frame image: https:// | data: | absolute local path | file://. Mutually exclusive with reference_image_urls."
                    },
                    "reference_image_urls": {
                        "type": "array",
                        "items": {"type": "string"},
                        "minItems": 1,
                        "maxItems": 7,
                        "description": "OPTIONAL (reference-to-video). 1–7 reference images (https/data/local path/file://). Mutually exclusive with image_url."
                    },
                    "duration": {
                        "type": "number",
                        "minimum": 1,
                        "maximum": 15,
                        "description": "OPTIONAL. Clip length in seconds (1–15)."
                    },
                    "aspect_ratio": {
                        "type": "string",
                        "enum": ["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"],
                        "description": "OPTIONAL. Output aspect ratio. Image-to-video defaults to source image AR if omitted."
                    },
                    "resolution": {
                        "type": "string",
                        "enum": ["480p", "720p", "1080p"],
                        "description": "OPTIONAL. Output resolution. Default often 480p; 1080p limited by model/mode."
                    },
                    "model": {
                        "type": "string",
                        "description": "OPTIONAL. Default: grok-imagine-video."
                    },
                    "wait": {
                        "type": "boolean",
                        "default": true,
                        "description": "OPTIONAL. Default true = poll until done. false = submit only and return job_id + poll path GET /v1/videos/{id}."
                    }
                },
                "required": ["prompt"]
            },
            "annotations": {
                "title": "Generate Video",
                "readOnlyHint": false
            }
        },
        {
            "name": "video_edit",
            "description": "Edit an existing video with a natural-language instruction. CALL IMMEDIATELY — do NOT search web/repo.\n\nARGS (complete):\n- prompt (string, REQUIRED): how to edit the video\n- video_url (string, REQUIRED): source video — https:// URL, data: URL, absolute local path, or file://. Local paths auto-inlined.\n- model (string, optional; default grok-imagine-video)\n\nNOTE: duration/aspect_ratio/resolution are NOT supported for edits (output matches source, duration capped ~8.7s).\nRETURNS: path/files/markdown absolute local mp4 (same shape as video_generate).\nEXAMPLE: {\"prompt\":\"make it snow\",\"video_url\":\"/Users/me/.grok-go/artifacts/vid.mp4\"}",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "REQUIRED. Edit instruction."
                    },
                    "video_url": {
                        "type": "string",
                        "description": "REQUIRED. Source video: https:// | data: | absolute local path | file://."
                    },
                    "model": {
                        "type": "string",
                        "description": "OPTIONAL. Default: grok-imagine-video."
                    }
                },
                "required": ["prompt", "video_url"]
            },
            "annotations": {
                "title": "Edit Video",
                "readOnlyHint": false
            }
        }
    ])
}

/// Filter the full catalog by `AppConfig.mcp_enabled_tools`.
fn mcp_tools_catalog(config: &AppConfig) -> Value {
    let all = mcp_tools_catalog_all();
    let Some(arr) = all.as_array() else {
        return all;
    };
    let filtered: Vec<Value> = arr
        .iter()
        .filter(|tool| {
            tool.get("name")
                .and_then(|n| n.as_str())
                .map(|name| config.mcp_tool_enabled(name))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    Value::Array(filtered)
}

/// MCP notifications must not return a JSON-RPC response body. Codex's streamable
/// HTTP client deserializes the HTTP body as `JsonRpcMessage` and fatals on a
/// `result` payload for `notifications/initialized`.
fn mcp_notification_ack() -> Response {
    (
        StatusCode::ACCEPTED,
        [("content-type", "application/json")],
        Body::empty(),
    )
        .into_response()
}

fn mcp_json_result(id: Value, result: Value) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn mcp_json_error(id: Value, code: i64, message: impl Into<String>) -> Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message.into()}
    }))
    .into_response()
}

async fn mcp_endpoint(State(state): State<GatewayState>, headers: HeaderMap, body: Bytes) -> Response {
    let config = match load_config() {
        Ok(c) => c,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    if let Err(resp) = authorize_request(&headers, &config).await {
        return resp;
    }

    let payload: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(err) => {
                return mcp_json_error(Value::Null, -32700, format!("parse error: {err}"));
            }
        }
    };
    let method = payload.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let has_id = payload
        .as_object()
        .map(|object| object.contains_key("id"))
        .unwrap_or(false);
    let id = payload.get("id").cloned().unwrap_or(Value::Null);

    // Notifications (including notifications/initialized): no JSON-RPC response.
    if is_mcp_notification(&payload, method) {
        return mcp_notification_ack();
    }

    let result = match method {
        "initialize" => json!({
            "protocolVersion": negotiated_protocol_version(&payload),
            "capabilities": {
                "tools": {"listChanged": false}
            },
            "serverInfo": {"name": "grok-go", "version": "0.1.0"}
        }),
        "tools/list" => json!({
            "tools": mcp_tools_catalog(&config)
        }),
        "tools/call" => {
            match handle_tool_call(
                &state,
                &config,
                payload.get("params").cloned().unwrap_or(json!({})),
            )
            .await
            {
                Ok(value) => value,
                Err(err) => {
                    return mcp_json_error(id, -32000, err.to_string());
                }
            }
        }
        // JSON-RPC ping is a request when it carries an id.
        "ping" => json!({}),
        // Answer empty lists for optional MCP surfaces instead of hard-failing.
        "resources/list" => json!({"resources": []}),
        "resources/templates/list" => json!({"resourceTemplates": []}),
        "prompts/list" => json!({"prompts": []}),
        _ => {
            if !has_id {
                return mcp_notification_ack();
            }
            return mcp_json_error(id, -32601, format!("method not found: {method}"));
        }
    };

    mcp_json_result(id, result)
}

async fn handle_tool_call(state: &GatewayState, config: &AppConfig, params: Value) -> AppResult<Value> {
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or_default();
    if name.is_empty() {
        return Err(AppError::msg("tool name is required"));
    }
    if !config.mcp_tool_enabled(name) {
        return Err(AppError::msg(format!(
            "MCP tool `{name}` is disabled in GrokGo settings"
        )));
    }
    let mut args = params.get("arguments").cloned().unwrap_or(json!({}));
    // O-13: coerce whole floats (e.g. session_id: 60619.0) to integers.
    let _ = crate::gateway::tool_surface::coerce_mcp_tool_arguments(&mut args);
    match name {
        "x_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
            let mut tool = json!({"type": "x_search"});
            if let Some(v) = args.get("allowed_handles") { tool["allowed_x_handles"] = v.clone(); }
            if let Some(v) = args.get("excluded_handles") { tool["excluded_x_handles"] = v.clone(); }
            if let Some(v) = args.get("from_date") { tool["from_date"] = v.clone(); }
            if let Some(v) = args.get("to_date") { tool["to_date"] = v.clone(); }
            let body = json!({
                "model": config.default_model,
                "input": query,
                "tools": [tool]
            });
            let up = call_upstream(state, config, "/responses", body, "mcp-x_search").await?;
            let envelope = crate::gateway::error_codes::tool_ok_envelope(
                name,
                "x_search completed",
                &[],
                Some(up.value.clone()),
            );
            Ok(crate::gateway::error_codes::mcp_content_from_envelope(&envelope))
        }
        // Codex skill looks for `image_gen`; keep `image_generate` as alias.
        "image_gen" | "image_generate" => {
            let prompt = args
                .get("prompt")
                .or_else(|| args.get("input"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(&config.default_image_model)
                .to_string();
            let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(1);
            let body = json!({"model": model, "prompt": prompt, "n": n});
            let up = call_upstream(state, config, "/images/generations", body, "mcp-image").await?;
            let client = state.proxy.client();
            let files = materialize_image_response(&client, &up.value).await?;
            if files.is_empty() {
                return Err(AppError::msg(format!(
                    "image generated but failed to materialize local files; upstream={}",
                    up.value
                )));
            }
            let summary = media_summary(name, &model, prompt, &files, &up.value, "image");
            Ok(mcp_media_content(&summary))
        }
        "image_edit" => {
            let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or_default();
            let raw_image = args
                .get("image_url")
                .or_else(|| args.get("image"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if raw_image.trim().is_empty() {
                return Err(AppError::msg("image_edit requires image_url"));
            }
            let image_url = resolve_media_url(raw_image)?;
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(&config.default_image_model)
                .to_string();
            let body = json!({
                "model": model,
                "prompt": prompt,
                "image": {"url": image_url, "type": "image_url"}
            });
            let up = call_upstream(state, config, "/images/edits", body, "mcp-image").await?;
            let client = state.proxy.client();
            let files = materialize_image_response(&client, &up.value).await?;
            if files.is_empty() {
                return Err(AppError::msg(format!(
                    "image edit succeeded but failed to materialize local files; upstream={}",
                    up.value
                )));
            }
            let summary = media_summary(name, &model, prompt, &files, &up.value, "image");
            Ok(mcp_media_content(&summary))
        }
        "video_generate" => {
            let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or_default();
            if prompt.trim().is_empty() {
                return Err(AppError::msg("video_generate requires prompt"));
            }
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(&config.default_video_model)
                .to_string();
            let image_url = args
                .get("image_url")
                .or_else(|| args.get("image"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let ref_urls: Vec<String> = args
                .get("reference_image_urls")
                .or_else(|| args.get("reference_images"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .filter(|s| !s.trim().is_empty())
                        .collect()
                })
                .unwrap_or_default();

            if image_url.is_some() && !ref_urls.is_empty() {
                return Err(AppError::msg(
                    "video_generate: pass either image_url (image-to-video) OR reference_image_urls, not both",
                ));
            }

            let mut body = json!({"model": model, "prompt": prompt});
            if let Some(raw) = image_url {
                let url = resolve_media_url(raw)?;
                body["image"] = json!({"url": url});
            } else if !ref_urls.is_empty() {
                let mut refs = Vec::new();
                for raw in ref_urls {
                    refs.push(json!({"url": resolve_media_url(&raw)?}));
                }
                body["reference_images"] = Value::Array(refs);
            }
            if let Some(duration) = args.get("duration") {
                body["duration"] = duration.clone();
            }
            if let Some(ar) = args.get("aspect_ratio").and_then(|v| v.as_str()) {
                body["aspect_ratio"] = json!(ar);
            }
            if let Some(res) = args.get("resolution").and_then(|v| v.as_str()) {
                body["resolution"] = json!(res);
            }

            // Submit + poll must use the same OAuth account (job is account-scoped).
            // O-07: clients may pass wait=false to get job_id immediately and poll
            // GET /v1/videos/{request_id} (existing affinity route).
            let wait = args
                .get("wait")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let up = call_upstream(state, config, "/videos/generations", body, "mcp-video").await?;
            let request_id = up
                .value
                .get("request_id")
                .or_else(|| up.value.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if !wait {
                if let Some(rid) = request_id.as_deref() {
                    crate::gateway::job_affinity::remember_video_job(rid, &up.account.id);
                    let envelope = json!({
                        "ok": true,
                        "tool": name,
                        "summary": "video job submitted; poll for completion",
                        "job_id": rid,
                        "poll": format!("/v1/videos/{rid}"),
                        "artifacts": [],
                        "error": null,
                        "raw": up.value,
                    });
                    return Ok(crate::gateway::error_codes::mcp_content_from_envelope(&envelope));
                }
            }
            match resolve_video_job(state, config, &up.value, &up.account).await {
                Ok(final_resp) => {
                    let client = state.proxy.client();
                    let files = materialize_video_response(&client, &final_resp).await?;
                    if files.is_empty() {
                        return Err(AppError::msg(format!(
                            "video generated but failed to materialize local files; upstream={final_resp}"
                        )));
                    }
                    let summary = media_summary(name, &model, prompt, &files, &final_resp, "video");
                    Ok(mcp_media_content(&summary))
                }
                Err(err) => {
                    // O-05: timeout ≠ permanent failure
                    let msg = err.to_string();
                    if msg.to_ascii_lowercase().contains("timed out") {
                        let arts = recent_video_artifacts(3);
                        return Ok(crate::gateway::error_codes::tool_timeout_mcp_result(
                            name,
                            request_id.as_deref(),
                            &arts,
                        ));
                    }
                    Err(err)
                }
            }
        }
        "video_edit" => {
            let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or_default();
            let raw_video = args
                .get("video_url")
                .or_else(|| args.get("video"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if raw_video.trim().is_empty() {
                return Err(AppError::msg("video_edit requires video_url"));
            }
            let video_url = resolve_media_url(raw_video)?;
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(&config.default_video_model)
                .to_string();
            let body = json!({
                "model": model,
                "prompt": prompt,
                "video": {"url": video_url}
            });
            let up = call_upstream(state, config, "/videos/edits", body, "mcp-video").await?;
            let request_id = up
                .value
                .get("request_id")
                .or_else(|| up.value.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            match resolve_video_job(state, config, &up.value, &up.account).await {
                Ok(final_resp) => {
                    let client = state.proxy.client();
                    let files = materialize_video_response(&client, &final_resp).await?;
                    if files.is_empty() {
                        return Err(AppError::msg(format!(
                            "video edit succeeded but failed to materialize local files; upstream={final_resp}"
                        )));
                    }
                    let summary = media_summary(name, &model, prompt, &files, &final_resp, "video");
                    Ok(mcp_media_content(&summary))
                }
                Err(err) => {
                    let msg = err.to_string();
                    if msg.to_ascii_lowercase().contains("timed out") {
                        let arts = recent_video_artifacts(3);
                        return Ok(crate::gateway::error_codes::tool_timeout_mcp_result(
                            name,
                            request_id.as_deref(),
                            &arts,
                        ));
                    }
                    Err(err)
                }
            }
        }
        _ => Err(AppError::msg(format!("unknown tool: {name}"))),
    }
}

fn recent_video_artifacts(limit: usize) -> Vec<String> {
    let Ok(dir) = crate::paths::artifacts_dir() else {
        return Vec::new();
    };
    crate::gateway::tool_surface::recent_artifacts(&dir, Some("mp4"), limit)
}

/// Result of one upstream POST, including the account that made it (needed for sticky video poll).
struct UpstreamResult {
    value: Value,
    account: Account,
}

/// If submit already contains a playable video URL, return it; otherwise poll by request_id.
///
/// **Must** poll with the same account that submitted. xAI video `request_id`s are
/// account-scoped; polling with a different WRR account returns HTTP 404
/// `{"code":"not-found","error":"Failed to read static file."}`.
async fn resolve_video_job(
    state: &GatewayState,
    config: &AppConfig,
    submit: &Value,
    submit_account: &Account,
) -> AppResult<Value> {
    // Immediate result (some paths may embed video.url without polling).
    if submit.pointer("/video/url").and_then(|v| v.as_str()).is_some()
        || submit
            .pointer("/data/0/url")
            .and_then(|v| v.as_str())
            .is_some()
    {
        return Ok(submit.clone());
    }

    let request_id = submit
        .get("request_id")
        .or_else(|| submit.get("id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::msg(format!("video submit missing request_id: {submit}")))?
        .to_string();

    let mut account = submit_account.clone();
    let before = account.access_token.clone();
    let token = ensure_fresh_token(config, &mut account).await?;
    if account.access_token != before {
        replace_account_tokens(&account)?;
    }

    let client = state.proxy.client();
    poll_video_result(
        &client,
        &config.xai_base_url,
        &token,
        &request_id,
        Duration::from_secs(360),
    )
    .await
}

async fn call_upstream(
    state: &GatewayState,
    config: &AppConfig,
    path: &str,
    body: Value,
    source: &str,
) -> AppResult<UpstreamResult> {
    let store = load_auth()?;
    let capability = MediaCapability::from_upstream_path(path);
    let mut account = pick_account_for(config, &store, capability)?;
    let before = account.access_token.clone();
    let token = ensure_fresh_token(config, &mut account).await?;
    if account.access_token != before {
        replace_account_tokens(&account)?;
    }
    let url = format!("{}{}", config.xai_base_url.trim_end_matches('/'), path);
    let started = std::time::Instant::now();
    let resp = state
        .proxy
        .client()
        .post(url)
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let value: Value = resp.json().await.unwrap_or(json!({}));
    let usage = value.get("usage").cloned().unwrap_or(json!({}));
    let input_tokens = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    // xAI: input_tokens_details.cached_tokens (not Anthropic cache_read_input_tokens).
    let cache_tokens = usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens"))
        .or_else(|| usage.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    crate::usage::enqueue_request_log(RequestLog {
        request_id: Uuid::new_v4().to_string(),
        account_id: Some(account.id.clone()),
        endpoint: path.to_string(),
        requested_model: body.get("model").and_then(|v| v.as_str()).map(|s| s.to_string()),
        resolved_model: body.get("model").and_then(|v| v.as_str()).map(|s| s.to_string()),
        status_code: status.as_u16(),
        latency_ms: started.elapsed().as_millis() as u64,
        first_token_ms: None,
        input_tokens,
        output_tokens,
        cache_tokens,
        estimated_cost_usd: estimate_cost(input_tokens, output_tokens, cache_tokens),
        error_summary: if status.is_success() {
            None
        } else {
            Some(value.to_string().chars().take(500).collect())
        },
        client_source: source.to_string(),
        created_at: Utc::now(),
    });
    if !status.is_success() {
        return Err(AppError::msg(format!("upstream {status}: {value}")));
    }
    // Pin deferred video jobs to the submitting account for sticky poll.
    if path.contains("/videos/generations") || path.contains("/videos/edits") {
        if let Some(rid) = crate::gateway::job_affinity::extract_video_request_id(&value) {
            crate::gateway::job_affinity::remember_video_job(&rid, &account.id);
        }
    }
    Ok(UpstreamResult { value, account })
}

fn error_response(status: StatusCode, message: String) -> Response {
    let layered = crate::gateway::error_codes::classify_transport_error(&message);
    // Preserve caller status when it is a deliberate HTTP code (auth etc.).
    let st = if status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || status == StatusCode::BAD_REQUEST
    {
        status
    } else {
        StatusCode::from_u16(layered.status).unwrap_or(status)
    };
    let mut body = layered.openai_body();
    if let Some(obj) = body.get_mut("error").and_then(|e| e.as_object_mut()) {
        obj.insert("message".into(), json!(message));
    }
    (st, Json(body)).into_response()
}

fn layered_error_response(err: crate::gateway::error_codes::LayeredError) -> Response {
    (
        StatusCode::from_u16(err.status).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(err.openai_body()),
    )
        .into_response()
}


#[cfg(test)]
mod mcp_handshake_tests {
    use super::{
        is_mcp_notification, mcp_notification_ack, mcp_tools_catalog, negotiated_protocol_version,
        MCP_PROTOCOL_LATEST, MCP_PROTOCOL_LEGACY,
    };
    use crate::config::AppConfig;
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use serde_json::json;

    #[test]
    fn notifications_are_detected_without_id() {
        let payload = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(is_mcp_notification(&payload, "notifications/initialized"));
    }

    #[test]
    fn requests_with_id_are_not_notifications() {
        let payload = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        assert!(!is_mcp_notification(&payload, "ping"));
    }

    #[test]
    fn protocol_version_echoes_codex_latest() {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"}
        });
        assert_eq!(negotiated_protocol_version(&payload), MCP_PROTOCOL_LATEST);
    }

    #[test]
    fn protocol_version_supports_legacy() {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {"protocolVersion": "2024-11-05"}
        });
        assert_eq!(negotiated_protocol_version(&payload), MCP_PROTOCOL_LEGACY);
    }

    #[test]
    fn tools_catalog_includes_video_generate() {
        let cfg = AppConfig::default();
        let tools = mcp_tools_catalog(&cfg);
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&"video_generate"));
        assert!(names.contains(&"image_gen"));
        assert_eq!(names.len(), 6);
    }

    #[test]
    fn tools_catalog_respects_enabled_filter() {
        let mut cfg = AppConfig::default();
        cfg.mcp_enabled_tools = Some(vec!["x_search".into(), "image_gen".into()]);
        let tools = mcp_tools_catalog(&cfg);
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(|n| n.as_str()))
            .collect();
        assert_eq!(names, vec!["x_search", "image_gen"]);
    }

    #[test]
    fn video_generate_schema_is_self_contained() {
        let tools = mcp_tools_catalog(&AppConfig::default());
        let video = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t.get("name").and_then(|n| n.as_str()) == Some("video_generate"))
            .expect("video_generate tool");
        let desc = video.get("description").and_then(|d| d.as_str()).unwrap();
        assert!(desc.contains("CALL IMMEDIATELY"), "should discourage searching");
        assert!(desc.contains("image_url"), "must document image-to-video");
        assert!(desc.contains("reference_image_urls"));
        assert!(desc.contains("aspect_ratio"));
        assert!(desc.contains("resolution"));
        let props = video.pointer("/inputSchema/properties").unwrap();
        for key in [
            "prompt",
            "image_url",
            "reference_image_urls",
            "duration",
            "aspect_ratio",
            "resolution",
            "model",
            "wait",
        ] {
            assert!(props.get(key).is_some(), "missing property {key}");
        }
        assert_eq!(
            props.pointer("/wait/type").and_then(|v| v.as_str()),
            Some("boolean")
        );
        assert!(desc.contains("wait"), "description must document wait");
    }

    #[tokio::test]
    async fn notification_ack_has_empty_body() {
        let response = mcp_notification_ack().into_response();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert!(body.is_empty(), "notification response must not include JSON-RPC body");
    }
}
