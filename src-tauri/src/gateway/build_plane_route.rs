//! Experimental Grok Build plane routing: pure decision + header/body adapt helpers.
//!
//! When `AppConfig::experimental_impersonate_grok_build` is **off** (default),
//! behaviour matches pre-feature dual-plane detection (native Grok Build markers
//! → cli-chat-proxy; everyone else → console `api.x.ai`).
//!
//! When **on**, non–Grok-Build clients (Codex / OpenAI / Claude Code) are forced
//! onto the SuperGrok / cli-chat-proxy chat plane with required identity headers.
//! Media (images/videos) stays on the console API — cli-chat-proxy does not host
//! those routes (CLIProxyAPI / wire survey).

use axum::http::{header, HeaderMap, HeaderName};
use serde_json::Value;

use crate::config::AppConfig;

/// Minimum cli-chat-proxy client version (426 Upgrade Required below this).
pub const DEFAULT_GROK_CLIENT_VERSION: &str = "0.2.101";

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
    /// Apply Codex-console-only recovery (empty-completion, nuclear strip, stream buffer).
    pub apply_codex_console_guards: bool,
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
        apply_codex_console_guards: !build,
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
/// Includes all `x-grok-*` / `x-xai-*` plus a few non-prefixed headers the official
/// CLI sends (`User-Agent`, `x-email`, `x-models-etag`, tracing). Authorization is
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
                | "x-models-etag"
                | "x-authenticate"
                | "accept-language"
                | "x-request-id"
                | "traceparent"
                | "tracestate"
                | "baggage"
        )
}

/// Client headers that must reach cli-chat-proxy for native or impersonated Grok Build.
///
/// Always ensures `X-XAI-Token-Auth`, `x-grok-client-version`, and a Grok-like UA.
pub fn collect_build_plane_passthrough_headers(headers: &HeaderMap) -> Vec<(HeaderName, String)> {
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
    // Sensible User-Agent when callers (curl / tests / Codex) omit it.
    if !out
        .iter()
        .any(|(n, v)| n.as_str().eq_ignore_ascii_case("user-agent") && !v.trim().is_empty())
    {
        out.push((
            HeaderName::from_static("user-agent"),
            format!("xai-grok-shell/{DEFAULT_GROK_CLIENT_VERSION} (grok-build)"),
        ));
    }
    // When impersonating, rewrite non-Grok UAs so cli-chat-proxy sees a CLI-like client.
    // Keep explicit Grok UAs from native clients.
    if let Some(idx) = out
        .iter()
        .position(|(n, _)| n.as_str().eq_ignore_ascii_case("user-agent"))
    {
        let ua = out[idx].1.to_ascii_lowercase();
        let looks_grok = ua.contains("grok") || ua.contains("xai-grok");
        if !looks_grok {
            out[idx].1 = format!("xai-grok-shell/{DEFAULT_GROK_CLIENT_VERSION} (grok-build)");
        }
    }
    out
}

/// Build-plane body adapt for chat completions: strip fields cli-chat-proxy may reject
/// that OpenAI clients sometimes send. Idempotent; returns whether the body changed.
///
/// Responses path uses [`crate::gateway::sanitize::sanitize_responses_request_ex`] instead.
pub fn adapt_chat_body_for_build_plane(value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    let mut modified = false;
    // OpenAI-only stream_options is harmless on many planes but strip if empty object issues.
    // Keep stream_options when present (include_usage) — xAI accepts it.
    // Remove parallel_tool_calls only if false? Keep as-is; xAI accepts.
    // Strip service_tier / safety_identifier style OpenAI extras if present.
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
    fn flag_off_plain_client_stays_console() {
        let d = decide_plane(&cfg(false), &HeaderMap::new(), "/responses");
        assert!(!d.build_plane);
        assert!(!d.experimental_impersonation);
        assert_eq!(d.upstream_base, "https://api.x.ai/v1");
        assert!(d.apply_codex_console_guards);
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
            assert!(!d.apply_codex_console_guards);
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
    fn inject_headers_include_markers_for_plain_client() {
        let headers = collect_build_plane_passthrough_headers(&HeaderMap::new());
        let map: std::collections::HashMap<_, _> = headers
            .iter()
            .map(|(n, v)| (n.as_str().to_ascii_lowercase(), v.as_str()))
            .collect();
        assert_eq!(map.get("x-xai-token-auth").copied(), Some("xai-grok-cli"));
        assert_eq!(
            map.get("x-grok-client-version").copied(),
            Some(DEFAULT_GROK_CLIENT_VERSION)
        );
        assert!(map
            .get("user-agent")
            .copied()
            .unwrap_or("")
            .contains("grok"));
    }

    #[test]
    fn inject_headers_rewrites_codex_ua() {
        let mut h = HeaderMap::new();
        h.insert(
            header::USER_AGENT,
            HeaderValue::from_static("codex_cli_rs/0.1.0"),
        );
        let headers = collect_build_plane_passthrough_headers(&h);
        let ua = headers
            .iter()
            .find(|(n, _)| n.as_str().eq_ignore_ascii_case("user-agent"))
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert!(ua.contains("grok"), "got {ua}");
        assert!(!ua.contains("codex"));
    }

    #[test]
    fn inject_headers_keeps_native_ua() {
        let mut h = HeaderMap::new();
        h.insert(
            header::USER_AGENT,
            HeaderValue::from_static("xai-grok-shell/0.2.200 (grok-build)"),
        );
        let headers = collect_build_plane_passthrough_headers(&h);
        let ua = headers
            .iter()
            .find(|(n, _)| n.as_str().eq_ignore_ascii_case("user-agent"))
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert_eq!(ua, "xai-grok-shell/0.2.200 (grok-build)");
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
        use crate::gateway::sanitize::sanitize_responses_request_ex;
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
        let r = sanitize_responses_request_ex(&mut body, true);
        assert!(!r.modified || body.get("previous_response_id").is_some());
        assert_eq!(
            body.get("previous_response_id").and_then(|v| v.as_str()),
            Some("resp_1")
        );
        assert!(body_has_tools(&body));
        assert!(body_has_vision_input(&body));
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
