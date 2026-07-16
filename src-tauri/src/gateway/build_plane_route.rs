//! Grok Build / SuperGrok plane routing: pure decision + header/body adapt helpers.
//!
//! When `AppConfig::experimental_impersonate_grok_build` is **off** (**default**),
//! only native Grok Build markers → cli-chat-proxy; everyone else → console
//! `api.x.ai` (API channel).
//!
//! When **on** (Grok Build session plane), non–Grok-Build clients (Codex /
//! OpenAI / Claude Code) use SuperGrok / cli-chat-proxy with Grok Build identity
//! headers. Prefer opt-in: SuperGrok path may risk account restriction.
//!
//! Media (images/videos) always stays on the console API — cli-chat-proxy does
//! not host those routes (official `xai-org/grok-build` sampling vs media split).
//!
//! ## Wire source of truth
//! Aligned with open-source **https://github.com/xai-org/grok-build**:
//! - `xai-grok-shell` `inject_url_derived_headers` / `add_cli_chat_proxy_headers_*`
//! - `xai-grok-sampler` `SamplingClient` default + per-request `x-grok-*` headers
//! - `xai-grok-workspace` `build_proxy_headers`
//! - `xai-grok-http` User-Agent rendering (`grok-shell/{ver} ({os}; {arch})`)
//! - `xai-grok-sampler::shared_http` connection pool / HTTP/2 keepalive

use axum::http::{header, HeaderMap, HeaderName};
use serde_json::Value;
use uuid::Uuid;

use crate::config::AppConfig;

/// Client version sent as `x-grok-client-version` (cli-chat-proxy 426 if too old).
/// Known-good against production proxy gates; not the OSS package `0.2.0-dev`.
pub const DEFAULT_GROK_CLIENT_VERSION: &str = "0.2.101";

/// Official product token in User-Agent (`xai-grok-http` AGENT_PRODUCT / DEFAULT_CLIENT_IDENTIFIER).
pub const GROK_SHELL_PRODUCT: &str = "grok-shell";

/// `GrokComConfig::default().token_header` / inject_url_derived_headers.
pub const XAI_TOKEN_AUTH_VALUE: &str = "xai-grok-cli";

/// Shell `inject_url_derived_headers` for cli-chat-proxy bases.
pub const AUTHENTICATE_RESPONSE_VALUE: &str = "authenticate-response";

/// `xai-grok-http::CLIENT_MODE_HEADER` default (`process_client_mode`).
pub const CLIENT_MODE_HEADER: &str = "x-grok-client-mode";
pub const CLIENT_MODE_INTERACTIVE: &str = "interactive";

/// Usage / log tag when traffic is forced onto the build plane by config.
pub const EXPERIMENTAL_BUILD_SOURCE: &str = "experimental-build";

/// Native Grok Build TUI / CLI traffic.
pub const NATIVE_BUILD_SOURCE: &str = "grok-build";

/// Outcome of plane resolution for a single gateway request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneDecision {
    /// Effective build-plane semantics (continuity sanitize, no nuclear strip, etc.).
    pub build_plane: bool,
    /// Forced by experimental flag (client lacked native Grok Build markers).
    pub experimental_impersonation: bool,
    /// Client already presented Grok Build markers.
    pub native_build_client: bool,
    /// Absolute upstream base without trailing slash (includes `/v1`).
    pub upstream_base: String,
    /// Override for `client_source` in usage logs when set.
    pub client_source_override: Option<&'static str>,
    /// Attach cli-chat-proxy identity headers on the outbound request.
    pub inject_build_headers: bool,
    /// Allow console Files offload (never on build chat plane).
    pub allow_files_offload: bool,
    /// Console-only input recovery (nuclear strip / messages-only retry).
    /// Off on build plane so we do not destroy cli-chat-proxy continuity fields.
    pub apply_codex_console_guards: bool,
    /// Premature agent-stop recovery for Codex/OpenAI clients.
    /// On for console **and** experimental impersonation; off only for **native**
    /// Grok Build TUI (its own agent loop handles tools).
    pub apply_empty_completion_recovery: bool,
    /// Inject compact Codex-compat tools (`x_search`, `image_gen`) into Responses.
    /// True for console and **experimental** impersonation; **false** for native Build TUI
    /// (preserves official tool list / prefix cache).
    pub inject_codex_compat_tools: bool,
    /// Path is image/video media (always console upstream).
    pub media_path: bool,
}

/// True when the client is Grok Build CLI (session / SuperGrok credits plane).
///
/// Markers from official docs / CLIProxyAPI: `X-XAI-Token-Auth: xai-grok-cli`,
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

/// Image / video REST routes (cli-chat-proxy does not serve these).
pub fn is_media_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("/images/")
        || p.contains("/videos/")
        || p.ends_with("/videos")
        || p.contains("/videos?")
}

/// Inference-ish paths that may use Files offload on the console plane.
pub fn is_inference_files_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("/responses") || p.contains("/chat/completions")
}

/// Pick upstream base from an already-decided build flag (test helper / thin wrapper).
pub fn resolve_upstream_base(config: &AppConfig, headers: &HeaderMap) -> String {
    decide_plane(config, headers, "/responses").upstream_base
}

/// Core plane decision: config flag × client markers × path.
pub fn decide_plane(config: &AppConfig, headers: &HeaderMap, path: &str) -> PlaneDecision {
    let native = is_grok_build_plane(headers);
    let experimental = config.experimental_impersonate_grok_build;
    let media = is_media_path(path);
    let console = config.xai_base_url.trim_end_matches('/').to_string();
    let build_base = config
        .cli_chat_proxy_base_url
        .trim_end_matches('/')
        .to_string();

    // Media always uses console API (CLIProxyAPI + wire survey).
    if media {
        return PlaneDecision {
            build_plane: false,
            experimental_impersonation: experimental && !native,
            native_build_client: native,
            upstream_base: console,
            client_source_override: if native {
                Some(NATIVE_BUILD_SOURCE)
            } else if experimental {
                // Inference is on build plane; media still console — tag for clarity.
                Some("experimental-build-media")
            } else {
                None
            },
            inject_build_headers: false,
            allow_files_offload: false,
            apply_codex_console_guards: false,
            // Media is not an agent tool loop; no empty-completion recovery.
            apply_empty_completion_recovery: false,
            inject_codex_compat_tools: false,
            media_path: true,
        };
    }

    let force = experimental && !native;
    let build = native || force;

    PlaneDecision {
        build_plane: build,
        experimental_impersonation: force,
        native_build_client: native,
        upstream_base: if build {
            build_base
        } else {
            console
        },
        client_source_override: if force {
            Some(EXPERIMENTAL_BUILD_SOURCE)
        } else if native {
            Some(NATIVE_BUILD_SOURCE)
        } else {
            None
        },
        inject_build_headers: build,
        allow_files_offload: !build && is_inference_files_path(path),
        // Nuclear strip only on pure console (would burn build-plane cache).
        apply_codex_console_guards: !build,
        // Codex via experimental build still needs premature-stop recovery.
        apply_empty_completion_recovery: !native,
        // Console always; experimental Build fills the tool vacuum; native TUI never.
        inject_codex_compat_tools: !native,
        media_path: false,
    }
}

/// Resolve effective client_source for usage logs.
pub fn effective_client_source(decision: &PlaneDecision, fallback: &str) -> String {
    decision
        .client_source_override
        .unwrap_or(fallback)
        .to_string()
}

/// Whether a client header should be forwarded on the Grok Build / cli-chat-proxy plane.
///
/// Includes all `x-grok-*` / `x-xai-*` plus non-prefixed headers the official
/// CLI sends (`User-Agent`, `x-email`, `x-userid`, tracing). Authorization is
/// rewritten to a pool token and must never pass through.
pub fn should_passthrough_build_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
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
                | "x-userid"
                | "x-models-etag"
                | "x-authenticate"
                | "x-authenticateresponse"
                | "accept-language"
                | "x-request-id"
                | "traceparent"
                | "tracestate"
                | "baggage"
        )
}

/// Official User-Agent shape from `xai-grok-http::UserAgent::render` when
/// origin product == agent product (`grok-shell/{ver} ({os}; {arch})`).
pub fn official_grok_shell_user_agent() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" | "arm64" => "aarch64",
        "x86_64" => "x86_64",
        other => other,
    };
    format!("{GROK_SHELL_PRODUCT}/{DEFAULT_GROK_CLIENT_VERSION} ({os}; {arch})")
}

/// True when `base_url` is a cli-chat-proxy / chat-proxy host (official
/// `is_cli_chat_proxy_url` / `build_proxy_headers` check).
pub fn is_cli_chat_proxy_url(base_url: &str) -> bool {
    let lower = base_url.to_ascii_lowercase();
    lower.contains("cli-chat-proxy") || lower.contains("chat-proxy")
}

/// Optional sampling context for per-request `x-grok-*` headers
/// (`xai-grok-sampler::GrokRequestHeaders`).
#[derive(Debug, Clone, Default)]
pub struct BuildPlaneHeaderContext<'a> {
    pub model_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub conv_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    /// Force rewrite of non-Grok User-Agents (experimental impersonation).
    pub force_official_ua: bool,
}

/// Client headers that must reach cli-chat-proxy for native or impersonated Grok Build.
///
/// Mirrors official inject set:
/// - `X-XAI-Token-Auth: xai-grok-cli`
/// - `x-authenticateresponse: authenticate-response`
/// - `x-grok-client-mode` (default interactive)
/// - `x-grok-client-identifier: grok-shell`
/// - `x-grok-client-version`
/// - User-Agent `grok-shell/{ver} ({os}; {arch})`
/// - sampling: `x-grok-conv-id`, `x-grok-req-id`, `x-grok-model-override`,
///   `x-grok-session-id`, `x-grok-agent-id` when context supplied
pub fn collect_build_plane_passthrough_headers(headers: &HeaderMap) -> Vec<(HeaderName, String)> {
    collect_build_plane_headers(headers, &BuildPlaneHeaderContext::default())
}

/// Full outbound header bag for cli-chat-proxy (passthrough + official inject).
pub fn collect_build_plane_headers(
    headers: &HeaderMap,
    ctx: &BuildPlaneHeaderContext<'_>,
) -> Vec<(HeaderName, String)> {
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

    // --- URL-derived headers (inject_url_derived_headers) ---
    ensure_header(&mut out, "x-xai-token-auth", XAI_TOKEN_AUTH_VALUE);
    ensure_header(
        &mut out,
        "x-authenticateresponse",
        AUTHENTICATE_RESPONSE_VALUE,
    );
    ensure_header(&mut out, CLIENT_MODE_HEADER, CLIENT_MODE_INTERACTIVE);
    ensure_header(
        &mut out,
        "x-grok-client-identifier",
        GROK_SHELL_PRODUCT,
    );
    ensure_header(
        &mut out,
        "x-grok-client-version",
        DEFAULT_GROK_CLIENT_VERSION,
    );

    // User-Agent: official grok-shell form; rewrite non-Grok when impersonating
    // or when missing.
    let ua_idx = out
        .iter()
        .position(|(n, _)| n.as_str().eq_ignore_ascii_case("user-agent"));
    let official_ua = official_grok_shell_user_agent();
    match ua_idx {
        None => {
            out.push((HeaderName::from_static("user-agent"), official_ua));
        }
        Some(idx) => {
            let ua = out[idx].1.to_ascii_lowercase();
            let looks_grok = ua.contains("grok-shell")
                || ua.contains("xai-grok-shell")
                || ua.contains("xai-grok-workspace")
                || ua.contains("grok-build")
                || ua.contains("grok-cli");
            if ctx.force_official_ua || !looks_grok {
                out[idx].1 = official_ua;
            }
        }
    }

    // --- Per-request sampling headers (GrokRequestHeaders) ---
    if let Some(model) = ctx.model_id.map(str::trim).filter(|s| !s.is_empty()) {
        // Native client may already send override; only fill when missing.
        if !has_header(&out, "x-grok-model-override") {
            out.push((
                HeaderName::from_static("x-grok-model-override"),
                model.to_string(),
            ));
        }
    }
    if let Some(sid) = ctx.session_id.map(str::trim).filter(|s| !s.is_empty()) {
        if !has_header(&out, "x-grok-session-id") {
            out.push((
                HeaderName::from_static("x-grok-session-id"),
                sid.to_string(),
            ));
        }
        // conv-id is the primary cache namespace for multi-turn on build plane.
        if !has_header(&out, "x-grok-conv-id") {
            if let Some(cid) = ctx.conv_id.map(str::trim).filter(|s| !s.is_empty()) {
                out.push((HeaderName::from_static("x-grok-conv-id"), cid.to_string()));
            } else {
                out.push((
                    HeaderName::from_static("x-grok-conv-id"),
                    sid.to_string(),
                ));
            }
        }
    } else if let Some(cid) = ctx.conv_id.map(str::trim).filter(|s| !s.is_empty()) {
        if !has_header(&out, "x-grok-conv-id") {
            out.push((HeaderName::from_static("x-grok-conv-id"), cid.to_string()));
        }
    }
    if !has_header(&out, "x-grok-req-id") {
        out.push((
            HeaderName::from_static("x-grok-req-id"),
            Uuid::new_v4().to_string(),
        ));
    }
    let agent = ctx
        .agent_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main");
    if !has_header(&out, "x-grok-agent-id") {
        out.push((
            HeaderName::from_static("x-grok-agent-id"),
            agent.to_string(),
        ));
    }

    out
}

fn has_header(headers: &[(HeaderName, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(n, v)| n.as_str().eq_ignore_ascii_case(name) && !v.trim().is_empty())
}

/// Insert header if missing or empty (does not overwrite client values).
fn ensure_header(headers: &mut Vec<(HeaderName, String)>, name: &'static str, value: &str) {
    if has_header(headers, name) {
        return;
    }
    if let Ok(hn) = HeaderName::from_bytes(name.as_bytes()) {
        headers.push((hn, value.to_string()));
    }
}

/// Build-plane body adapt for chat completions: strip fields cli-chat-proxy may reject
/// that OpenAI clients sometimes send. Idempotent; returns whether the body changed.
///
/// Responses path uses [`crate::gateway::sanitize::sanitize_responses_request_ex`] plus
/// [`adapt_responses_body_for_build_plane`].
pub fn adapt_chat_body_for_build_plane(value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    let mut modified = false;
    // Strip OpenAI-console-only extras (official sampler never sends these).
    for key in ["service_tier", "safety_identifier"] {
        if obj.remove(key).is_some() {
            modified = true;
        }
    }
    // Ensure tools array items are function-shaped (OpenAI chat already is).
    if let Some(tools) = obj.get_mut("tools").and_then(|t| t.as_array_mut()) {
        for tool in tools.iter_mut() {
            if let Some(t) = tool.as_object_mut() {
                // Some clients send type:"custom" at chat layer — map to function.
                if t.get("type").and_then(|v| v.as_str()) == Some("custom") {
                    t.insert("type".into(), Value::String("function".into()));
                    modified = true;
                }
            }
        }
    }
    modified
}

/// Responses body continuity for cli-chat-proxy (cache / multi-turn).
///
/// Official sampling keeps `previous_response_id` when the shell has a prior turn
/// and may set `prompt_cache_key` for stable prefix cache. Continuity is also
/// carried via `x-grok-conv-id` headers. This fills missing body keys only —
/// never strips client-provided values on the build plane.
pub fn adapt_responses_body_for_build_plane(
    value: &mut Value,
    session_key: Option<&str>,
) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    let mut modified = false;
    // Stable prompt_cache_key for multi-turn when client omitted it.
    if obj
        .get("prompt_cache_key")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        if let Some(key) = session_key.map(str::trim).filter(|s| !s.is_empty()) {
            obj.insert(
                "prompt_cache_key".into(),
                Value::String(key.to_string()),
            );
            modified = true;
        }
    }
    // Prefer 24h retention when absent so SuperGrok can reuse prefixes across turns.
    // Official may leave this None; cli-chat-proxy accepts the field when present.
    if obj.get("prompt_cache_retention").is_none() {
        obj.insert(
            "prompt_cache_retention".into(),
            Value::String("24h".into()),
        );
        modified = true;
    }
    modified
}

/// Whether vision / image input is present in an OpenAI-ish body (chat or responses).
pub fn body_has_vision_input(value: &Value) -> bool {
    fn walk(v: &Value) -> bool {
        match v {
            Value::Object(map) => {
                if let Some(t) = map.get("type").and_then(|x| x.as_str()) {
                    if matches!(
                        t,
                        "image_url"
                            | "input_image"
                            | "image"
                            | "input_file"
                    ) {
                        return true;
                    }
                }
                if map.contains_key("image_url") {
                    return true;
                }
                map.values().any(walk)
            }
            Value::Array(arr) => arr.iter().any(walk),
            _ => false,
        }
    }
    walk(value)
}

/// Whether tools / tool_choice indicate a tool-calling turn.
pub fn body_has_tools(value: &Value) -> bool {
    value
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    fn cfg(experimental: bool) -> AppConfig {
        let mut c = AppConfig::default();
        c.xai_base_url = "https://api.x.ai/v1".into();
        c.cli_chat_proxy_base_url = "https://cli-chat-proxy.grok.com/v1".into();
        c.experimental_impersonate_grok_build = experimental;
        c
    }

    #[test]
    fn default_config_uses_api_channel() {
        assert!(!AppConfig::default().experimental_impersonate_grok_build);
        // Round-trip default JSON: field present false. Omit key → serde default false.
        let mut value = serde_json::to_value(AppConfig::default()).expect("to_value");
        assert_eq!(
            value.get("experimentalImpersonateGrokBuild"),
            Some(&serde_json::json!(false))
        );
        value
            .as_object_mut()
            .unwrap()
            .remove("experimentalImpersonateGrokBuild");
        let omit_key: AppConfig = serde_json::from_value(value).expect("omit field defaults off");
        assert!(!omit_key.experimental_impersonate_grok_build);
        let d = decide_plane(&AppConfig::default(), &HeaderMap::new(), "/responses");
        assert!(!d.build_plane);
        assert!(!d.experimental_impersonation);
        assert_eq!(d.upstream_base, "https://api.x.ai/v1");
    }

    #[test]
    fn flag_off_plain_client_stays_console() {
        let d = decide_plane(&cfg(false), &HeaderMap::new(), "/responses");
        assert!(!d.build_plane);
        assert!(!d.experimental_impersonation);
        assert_eq!(d.upstream_base, "https://api.x.ai/v1");
        assert!(d.apply_codex_console_guards);
        assert!(d.apply_empty_completion_recovery);
        assert!(d.inject_codex_compat_tools);
        assert!(d.allow_files_offload);
        assert!(!d.inject_build_headers);
        assert!(d.client_source_override.is_none());
    }

    #[test]
    fn flag_off_native_markers_use_cli_chat_proxy() {
        let mut h = HeaderMap::new();
        h.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
        let d = decide_plane(&cfg(false), &h, "/responses");
        assert!(d.build_plane);
        assert!(d.native_build_client);
        assert!(!d.experimental_impersonation);
        assert_eq!(d.upstream_base, "https://cli-chat-proxy.grok.com/v1");
        assert_eq!(d.client_source_override, Some(NATIVE_BUILD_SOURCE));
        assert!(!d.apply_codex_console_guards);
        // Native Grok Build TUI: no Codex empty-completion recovery / tool inject.
        assert!(!d.apply_empty_completion_recovery);
        assert!(!d.inject_codex_compat_tools);
        assert!(!d.allow_files_offload);
    }

    #[test]
    fn flag_on_forces_build_for_codex_openai_claude_paths() {
        for path in [
            "/responses",
            "/chat/completions",
            "/messages",
            "/v1/responses",
        ] {
            let d = decide_plane(&cfg(true), &HeaderMap::new(), path);
            assert!(d.build_plane, "path {path}");
            assert!(d.experimental_impersonation, "path {path}");
            assert_eq!(d.upstream_base, "https://cli-chat-proxy.grok.com/v1");
            assert_eq!(d.client_source_override, Some(EXPERIMENTAL_BUILD_SOURCE));
            assert!(d.inject_build_headers);
            // Nuclear strip stays off on build plane...
            assert!(!d.apply_codex_console_guards, "path {path}");
            // ...but Codex premature-stop recovery stays on (session 019f6852 regression).
            assert!(d.apply_empty_completion_recovery, "path {path}");
            assert!(d.inject_codex_compat_tools, "path {path}");
            assert!(!d.allow_files_offload);
        }
    }

    #[test]
    fn flag_on_media_stays_console() {
        for path in [
            "/images/generations",
            "/images/edits",
            "/videos/generations",
            "/videos/edits",
            "/videos/abc",
        ] {
            let d = decide_plane(&cfg(true), &HeaderMap::new(), path);
            assert!(!d.build_plane, "path {path}");
            assert!(d.media_path, "path {path}");
            assert_eq!(d.upstream_base, "https://api.x.ai/v1");
            assert!(!d.inject_build_headers);
            assert_eq!(
                d.client_source_override,
                Some("experimental-build-media")
            );
        }
    }

    #[test]
    fn flag_on_does_not_double_force_native_client() {
        let mut h = HeaderMap::new();
        h.insert("x-xai-token-auth", HeaderValue::from_static("xai-grok-cli"));
        let d = decide_plane(&cfg(true), &h, "/chat/completions");
        assert!(d.build_plane);
        assert!(d.native_build_client);
        assert!(!d.experimental_impersonation);
        assert_eq!(d.client_source_override, Some(NATIVE_BUILD_SOURCE));
    }

    #[test]
    fn inject_headers_include_official_cli_chat_proxy_markers() {
        let headers = collect_build_plane_passthrough_headers(&HeaderMap::new());
        let map: std::collections::HashMap<_, _> = headers
            .iter()
            .map(|(n, v)| (n.as_str().to_ascii_lowercase(), v.as_str().to_string()))
            .collect();
        assert_eq!(
            map.get("x-xai-token-auth").map(|s| s.as_str()),
            Some(XAI_TOKEN_AUTH_VALUE)
        );
        assert_eq!(
            map.get("x-authenticateresponse").map(|s| s.as_str()),
            Some(AUTHENTICATE_RESPONSE_VALUE)
        );
        assert_eq!(
            map.get("x-grok-client-mode").map(|s| s.as_str()),
            Some(CLIENT_MODE_INTERACTIVE)
        );
        assert_eq!(
            map.get("x-grok-client-identifier").map(|s| s.as_str()),
            Some(GROK_SHELL_PRODUCT)
        );
        assert_eq!(
            map.get("x-grok-client-version").map(|s| s.as_str()),
            Some(DEFAULT_GROK_CLIENT_VERSION)
        );
        let ua = map.get("user-agent").map(|s| s.as_str()).unwrap_or("");
        assert!(ua.starts_with("grok-shell/"), "ua={ua}");
        assert!(ua.contains('(') && ua.contains(';'), "ua={ua}");
        // sampling defaults
        assert!(map.contains_key("x-grok-req-id"));
        assert_eq!(map.get("x-grok-agent-id").map(|s| s.as_str()), Some("main"));
    }

    #[test]
    fn inject_headers_rewrites_codex_ua() {
        let mut h = HeaderMap::new();
        h.insert(
            header::USER_AGENT,
            HeaderValue::from_static("codex_cli_rs/0.1.0"),
        );
        let headers = collect_build_plane_headers(
            &h,
            &BuildPlaneHeaderContext {
                force_official_ua: true,
                ..Default::default()
            },
        );
        let ua = headers
            .iter()
            .find(|(n, _)| n.as_str().eq_ignore_ascii_case("user-agent"))
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert!(ua.starts_with("grok-shell/"), "got {ua}");
        assert!(!ua.contains("codex"));
    }

    #[test]
    fn inject_headers_keeps_native_ua() {
        let mut h = HeaderMap::new();
        h.insert(
            header::USER_AGENT,
            HeaderValue::from_static("xai-grok-shell/0.2.200 (macos; aarch64)"),
        );
        let headers = collect_build_plane_headers(
            &h,
            &BuildPlaneHeaderContext {
                force_official_ua: false,
                ..Default::default()
            },
        );
        let ua = headers
            .iter()
            .find(|(n, _)| n.as_str().eq_ignore_ascii_case("user-agent"))
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert_eq!(ua, "xai-grok-shell/0.2.200 (macos; aarch64)");
    }

    #[test]
    fn inject_headers_sampling_context_model_session_conv() {
        let headers = collect_build_plane_headers(
            &HeaderMap::new(),
            &BuildPlaneHeaderContext {
                model_id: Some("grok-4.5"),
                session_id: Some("sess-abc"),
                conv_id: Some("conv-xyz"),
                agent_id: Some("gateway"),
                force_official_ua: true,
            },
        );
        let map: std::collections::HashMap<_, _> = headers
            .iter()
            .map(|(n, v)| (n.as_str().to_ascii_lowercase(), v.as_str()))
            .collect();
        assert_eq!(map.get("x-grok-model-override").copied(), Some("grok-4.5"));
        assert_eq!(map.get("x-grok-session-id").copied(), Some("sess-abc"));
        assert_eq!(map.get("x-grok-conv-id").copied(), Some("conv-xyz"));
        assert_eq!(map.get("x-grok-agent-id").copied(), Some("gateway"));
    }

    #[test]
    fn responses_adapt_fills_cache_keys() {
        let mut body = json!({
            "model": "grok-4.5",
            "input": "hi"
        });
        assert!(adapt_responses_body_for_build_plane(
            &mut body,
            Some("thread-1")
        ));
        assert_eq!(
            body.get("prompt_cache_key").and_then(|v| v.as_str()),
            Some("thread-1")
        );
        assert_eq!(
            body.get("prompt_cache_retention").and_then(|v| v.as_str()),
            Some("24h")
        );
        // Does not overwrite existing
        body["prompt_cache_key"] = json!("keep-me");
        body["prompt_cache_retention"] = json!("in_memory");
        assert!(!adapt_responses_body_for_build_plane(
            &mut body,
            Some("other")
        ));
        assert_eq!(
            body.get("prompt_cache_key").and_then(|v| v.as_str()),
            Some("keep-me")
        );
    }

    #[test]
    fn detects_cli_chat_proxy_url() {
        assert!(is_cli_chat_proxy_url(
            "https://cli-chat-proxy.grok.com/v1"
        ));
        assert!(is_cli_chat_proxy_url("https://chat-proxy.example/v1"));
        assert!(!is_cli_chat_proxy_url("https://api.x.ai/v1"));
    }

    #[test]
    fn chat_adapt_strips_openai_extras() {
        let mut body = json!({
            "model": "grok-4.5",
            "messages": [{"role":"user","content":"hi"}],
            "service_tier": "default",
            "safety_identifier": "x",
            "tools": [{"type":"custom","function":{"name":"f","parameters":{}}}]
        });
        assert!(adapt_chat_body_for_build_plane(&mut body));
        assert!(body.get("service_tier").is_none());
        assert_eq!(
            body["tools"][0]["type"].as_str(),
            Some("function")
        );
    }

    #[test]
    fn detects_vision_and_tools() {
        let vision = json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type":"text","text":"what"},
                    {"type":"image_url","image_url":{"url":"data:image/png;base64,xx"}}
                ]
            }]
        });
        assert!(body_has_vision_input(&vision));
        let tools = json!({"tools":[{"type":"function","function":{"name":"t"}}]});
        assert!(body_has_tools(&tools));
        assert!(!body_has_vision_input(&tools));
    }

    #[test]
    fn effective_source_prefers_override() {
        let d = decide_plane(&cfg(true), &HeaderMap::new(), "/responses");
        assert_eq!(
            effective_client_source(&d, "openai-responses"),
            EXPERIMENTAL_BUILD_SOURCE
        );
        let plain = decide_plane(&cfg(false), &HeaderMap::new(), "/responses");
        assert_eq!(
            effective_client_source(&plain, "openai-responses"),
            "openai-responses"
        );
    }

    /// End-to-end pure-path matrix for the three client protocols the gateway exposes.
    #[test]
    fn protocol_matrix_flag_off_and_on() {
        let protocols = [
            ("/responses", "openai-responses"),
            ("/chat/completions", "openai-chat"),
            ("/messages", "anthropic-messages"),
        ];
        for (path, src) in protocols {
            let off = decide_plane(&cfg(false), &HeaderMap::new(), path);
            assert!(!off.build_plane, "{path} off");
            assert_eq!(effective_client_source(&off, src), src);
            assert!(off.apply_codex_console_guards, "{path} off guards");

            let on = decide_plane(&cfg(true), &HeaderMap::new(), path);
            assert!(on.build_plane, "{path} on");
            assert!(on.experimental_impersonation, "{path} on force");
            assert_eq!(
                effective_client_source(&on, src),
                EXPERIMENTAL_BUILD_SOURCE
            );
            assert!(on.inject_build_headers, "{path} on headers");
            let headers = collect_build_plane_passthrough_headers(&HeaderMap::new());
            assert!(headers.iter().any(|(n, v)| {
                n.as_str().eq_ignore_ascii_case("x-xai-token-auth") && v == "xai-grok-cli"
            }));
        }
    }

    #[test]
    fn responses_sanitize_preserves_tools_and_vision_on_build_plane() {
        use crate::gateway::sanitize::{sanitize_responses_request_ex, sanitize_responses_request_opts, SanitizeOpts};
        let mut body = json!({
            "model": "grok-4.5",
            "previous_response_id": "resp_1",
            "prompt_cache_key": "thread-a",
            "input": [{
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "describe"},
                    {"type": "input_image", "image_url": "data:image/png;base64,aaa"}
                ]
            }],
            "tools": [{
                "type": "function",
                "name": "read_file",
                "parameters": {"type": "object", "properties": {}}
            }]
        });
        assert!(body_has_vision_input(&body));
        assert!(body_has_tools(&body));
        // Native preserve: no inject
        let r = sanitize_responses_request_ex(&mut body, true);
        assert!(!r.modified || body.get("previous_response_id").is_some());
        assert_eq!(
            body.get("previous_response_id").and_then(|v| v.as_str()),
            Some("resp_1")
        );
        assert!(body_has_tools(&body));
        assert!(body_has_vision_input(&body));
        assert_eq!(body["tools"].as_array().map(|a| a.len()), Some(1));

        // Experimental: preserve continuity + inject codex compat
        let mut exp = body.clone();
        let r2 = sanitize_responses_request_opts(
            &mut exp,
            SanitizeOpts {
                preserve_native_continuity: true,
                inject_codex_compat_tools: true,
            },
        );
        assert!(r2.modified);
        assert_eq!(
            exp.get("previous_response_id").and_then(|v| v.as_str()),
            Some("resp_1")
        );
        let types: Vec<_> = exp["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("type").and_then(|v| v.as_str()))
            .collect();
        assert!(types.contains(&"x_search"));
        assert!(types.iter().any(|t| *t == "function" || *t == "image_gen" || *t == "image_generation") || exp["tools"].as_array().unwrap().iter().any(|t| t.get("name").and_then(|n| n.as_str()) == Some("image_gen")));
    }

    #[test]
    fn stream_flag_and_tool_roundtrip_shape_survive_chat_adapt() {
        let mut body = json!({
            "model": "grok-4.5",
            "stream": true,
            "messages": [{"role":"user","content":"hi"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "parameters": {"type":"object","properties":{"city":{"type":"string"}}}
                }
            }],
            "tool_choice": "auto",
            "service_tier": "default"
        });
        assert!(adapt_chat_body_for_build_plane(&mut body));
        assert_eq!(body.get("stream").and_then(|v| v.as_bool()), Some(true));
        assert!(body_has_tools(&body));
        assert_eq!(
            body["tools"][0]["function"]["name"].as_str(),
            Some("get_weather")
        );
        assert!(body.get("service_tier").is_none());
    }
}
