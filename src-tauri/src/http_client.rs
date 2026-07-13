//! Shared HTTP client construction for upstream xAI / OAuth calls.
//!
//! Proxy resolution is controlled **only** by app settings:
//! 1. When `http_proxy_enabled` + non-empty URL → use that proxy
//! 2. Otherwise → direct (`no_proxy`), environment `*_PROXY` vars are ignored
//!
//! Env vars may still appear in status hints as a suggestion, but never auto-apply
//! while the setting toggle is off (so `pnpm tauri dev` under Clash shell exports
//! does not silently force a broken socks proxy).

use std::time::Duration;

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};

pub fn build_http_client(config: &AppConfig) -> AppResult<reqwest::Client> {
    build_client(config, ClientKind::Api)
}

/// Client for auth.x.ai discovery / token / userinfo.
pub fn build_oauth_http_client(config: &AppConfig) -> AppResult<reqwest::Client> {
    build_client(config, ClientKind::Oauth)
}

#[derive(Clone, Copy, Debug)]
enum ClientKind {
    Api,
    Oauth,
}

fn build_client(config: &AppConfig, kind: ClientKind) -> AppResult<reqwest::Client> {
    let timeout = match kind {
        ClientKind::Api => Duration::from_secs(600),
        ClientKind::Oauth => Duration::from_secs(45),
    };
    let connect = match kind {
        ClientKind::Api => Duration::from_secs(20),
        ClientKind::Oauth => Duration::from_secs(15),
    };

    let mut builder = reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(connect)
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(8)
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(60))
        .user_agent(concat!("grok-go/", env!("CARGO_PKG_VERSION")));

    match resolve_proxy_url(config) {
        Some(url) => {
            let proxy = reqwest::Proxy::all(&url)
                .map_err(|e| AppError::msg(format!("invalid HTTP proxy URL `{url}`: {e}")))?;
            builder = builder.proxy(proxy);
            tracing::info!(
                target: "http_client",
                ?kind,
                proxy = %url,
                "HTTP client using proxy"
            );
        }
        None => {
            builder = builder.no_proxy();
            tracing::info!(target: "http_client", ?kind, "HTTP client direct (no proxy)");
        }
    }

    builder
        .build()
        .map_err(|e| AppError::msg(format!("failed to build HTTP client: {e}")))
}

/// Resolve upstream proxy URL from **app config only**.
/// Returns `None` when the toggle is off or the URL is empty → caller uses direct.
pub(crate) fn resolve_proxy_url(config: &AppConfig) -> Option<String> {
    if !config.http_proxy_enabled {
        return None;
    }
    let url = config.http_proxy_url.trim();
    if url.is_empty() {
        return None;
    }
    Some(url.to_string())
}

/// Read common proxy env vars (Clash / shell exports). Used only for diagnostics/hints.
pub fn env_proxy_url() -> Option<String> {
    for key in [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

pub fn proxy_status_hint(config: &AppConfig) -> String {
    if config.http_proxy_enabled {
        let url = config.http_proxy_url.trim();
        if url.is_empty() {
            return "上游代理已开启但 URL 为空".into();
        }
        return format!("上游代理: {url}");
    }
    if let Some(url) = env_proxy_url() {
        return format!(
            "当前直连（设置未启用上游代理，已忽略环境代理 {url}）。本机通常无法直连 api.x.ai，请到设置开启上游代理并填写地址（建议用 http://127.0.0.1:7890，也可填环境代理 {url}）"
        );
    }
    "当前直连（未配置上游代理）。若无法访问 api.x.ai，请到设置开启上游 HTTP 代理（如 http://127.0.0.1:7890）".into()
}

/// Flatten reqwest error + causes (e.g. "operation timed out") for user-facing messages.
pub fn format_reqwest_error(err: &reqwest::Error) -> String {
    use std::error::Error as _;
    let mut parts = vec![err.to_string()];
    let mut src = err.source();
    let mut depth = 0;
    while let Some(s) = src {
        // Cap depth; skip redundant duplicates.
        let text = s.to_string();
        if parts.last().map(|p| p != &text).unwrap_or(true) {
            parts.push(text);
        }
        src = s.source();
        depth += 1;
        if depth >= 6 {
            break;
        }
    }
    parts.join(" → ")
}

/// Best-effort open URL in the system browser (macOS/Linux/Windows).
pub fn open_browser_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("/usr/bin/open").arg(url).spawn() {
            Ok(_) => return Ok(()),
            Err(err) => tracing::warn!("/usr/bin/open failed: {err}"),
        }
    }
    #[cfg(target_os = "windows")]
    {
        // CRITICAL: OAuth URLs contain `&` query separators. Unquoted
        // `cmd /C start "" <url>` treats `&` as a command separator, so the
        // browser only gets `...?response_type=code` and auth.x.ai returns
        // "Missing or invalid client_id".
        //
        // `rundll32 url.dll,FileProtocolHandler` passes the full URL intact
        // (preferred). Fallbacks quote the URL for `cmd start` / PowerShell.
        match std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(err) => tracing::warn!("rundll32 FileProtocolHandler failed: {err}"),
        }
        let quoted = format!("\"{}\"", url.replace('"', ""));
        match std::process::Command::new("cmd")
            .args(["/C", "start", "", &quoted])
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(err) => tracing::warn!("cmd start failed: {err}"),
        }
        let ps = format!(
            "Start-Process '{}'",
            url.replace('\'', "''")
        );
        match std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(err) => tracing::warn!("powershell Start-Process failed: {err}"),
        }
    }
    #[cfg(target_os = "linux")]
    {
        match std::process::Command::new("xdg-open").arg(url).spawn() {
            Ok(_) => return Ok(()),
            Err(err) => tracing::warn!("xdg-open failed: {err}"),
        }
    }

    open::that(url).map_err(|e| format!("open browser failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool, url: &str) -> AppConfig {
        AppConfig {
            http_proxy_enabled: enabled,
            http_proxy_url: url.into(),
            ..AppConfig::default()
        }
    }

    #[test]
    fn resolve_proxy_respects_disabled_toggle() {
        // Even with a URL present, disabled toggle must not use it.
        assert_eq!(
            resolve_proxy_url(&cfg(false, "socks5://127.0.0.1:7890")),
            None
        );
    }

    #[test]
    fn resolve_proxy_uses_url_when_enabled() {
        assert_eq!(
            resolve_proxy_url(&cfg(true, "http://127.0.0.1:7890")),
            Some("http://127.0.0.1:7890".into())
        );
    }

    #[test]
    fn resolve_proxy_ignores_empty_url_when_enabled() {
        assert_eq!(resolve_proxy_url(&cfg(true, "   ")), None);
    }

    #[test]
    fn proxy_status_hint_when_enabled() {
        let h = proxy_status_hint(&cfg(true, "http://127.0.0.1:7890"));
        assert!(h.contains("上游代理: http://127.0.0.1:7890"));
    }

    #[test]
    fn proxy_status_hint_when_disabled_is_direct() {
        let h = proxy_status_hint(&cfg(false, ""));
        assert!(h.contains("当前直连"));
        // Must not claim we are actively using an env proxy for traffic.
        assert!(!h.starts_with("使用环境代理"));
    }
}
