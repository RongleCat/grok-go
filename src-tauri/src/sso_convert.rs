//! Convert Grok web SSO cookie JWT → official xAI OAuth tokens (pure Rust).
//!
//! Port of the device-flow path in `sso.py` / `sso_to_oauth.py`:
//! 1. Validate SSO cookie against `accounts.x.ai`
//! 2. OIDC Device Authorization (`/oauth2/device/code`)
//! 3. Browser-session `verify` + `approve` with the SSO cookie
//! 4. Poll `/oauth2/token` → access/refresh
//! 5. Optional userinfo for email
//!
//! After conversion, accounts use the existing OAuth gateway path only.
//! The grok.com reverse channel is intentionally **not** used.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Utc};
use rand::Rng;
use reqwest::cookie::Jar;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT};
use reqwest::redirect::Policy;
use serde::Deserialize;
use url::Url;

use crate::config::{load_config, Account, AccountAuthKind, AccountHealth, AppConfig};
use crate::http_client::resolve_proxy_url;

/// Same public hermes / grok-cli client used by OAuth login.
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const OIDC_ISSUER: &str = "https://auth.x.ai";
const SCOPES: &str = "openid profile email offline_access grok-cli:access api:access conversations:read conversations:write";
const ACCOUNTS_HOME: &str = "https://accounts.x.ai/";
const BROWSER_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct ConvertedOauth {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub token_type: String,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    #[serde(default = "default_interval")]
    interval: u64,
    #[serde(default = "default_expires")]
    expires_in: u64,
}

fn default_interval() -> u64 {
    5
}
fn default_expires() -> u64 {
    1800
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    email: Option<String>,
}

/// Convert one SSO cookie into OAuth tokens.
pub async fn convert_sso_cookie(
    sso_cookie: &str,
    email_hint: Option<&str>,
) -> Result<ConvertedOauth, String> {
    let sso = normalize_sso(sso_cookie);
    if sso.is_empty() {
        return Err("empty sso cookie".into());
    }

    let config = load_config().unwrap_or_default();
    let client_id = if config.xai_client_id.trim().is_empty() {
        CLIENT_ID.to_string()
    } else {
        config.xai_client_id.trim().to_string()
    };

    let (client, _jar) = build_sso_session_client(&config, &sso)?;

    // 1) Validate SSO session
    validate_sso_session(&client).await?;
    tracing::info!("SSO cookie valid on accounts.x.ai");

    // 2–4) Device flow with retries on rate-limit
    let max_retries = 8u32;
    let mut last_err = String::from("device flow failed");
    for attempt in 1..=max_retries {
        match device_flow_once(&client, &client_id).await {
            Ok(token) => {
                let mut email = email_hint
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                if email.is_none() {
                    email = fetch_userinfo_email(&client, &token.access_token).await;
                }
                return Ok(ConvertedOauth {
                    access_token: token.access_token,
                    refresh_token: token.refresh_token,
                    expires_in: token.expires_in.unwrap_or(21600),
                    token_type: token
                        .token_type
                        .unwrap_or_else(|| "Bearer".into()),
                    email,
                });
            }
            Err(err) => {
                last_err = err.clone();
                if is_rate_limited_msg(&err) && attempt < max_retries {
                    let delay = backoff_sec(15.0, attempt, 180.0);
                    tracing::warn!(
                        attempt,
                        delay_s = delay,
                        %err,
                        "SSO→OAuth rate limited; retrying"
                    );
                    tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                    continue;
                }
                if attempt < max_retries && is_transient(&err) {
                    let delay = backoff_sec(8.0, attempt, 60.0);
                    tracing::warn!(attempt, delay_s = delay, %err, "SSO→OAuth transient error");
                    tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                    continue;
                }
                return Err(err);
            }
        }
    }
    Err(last_err)
}

struct PolledToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
}

async fn device_flow_once(client: &reqwest::Client, client_id: &str) -> Result<PolledToken, String> {
    tracing::info!("SSO→OAuth Device Flow starting");
    let dc = request_device_code(client, client_id).await?;
    tracing::info!(user_code = %dc.user_code, "device code issued");

    // Open verification URI so the SSO session is associated with the user_code.
    let ver = client
        .get(&dc.verification_uri_complete)
        .headers(browser_headers())
        .send()
        .await
        .map_err(|e| format!("verification_uri request failed: {e}"))?;
    let ver_url = ver.url().to_string();
    let ver_status = ver.status();
    let _ = ver.text().await;
    if is_rate_limited_url(&ver_url) {
        return Err(format!("rate_limited on verification_uri: {ver_url}"));
    }
    if !ver_status.is_success() && !ver_status.is_redirection() {
        tracing::debug!(%ver_status, %ver_url, "verification_uri non-success (continuing)");
    }

    // verify
    let verify_url = format!("{OIDC_ISSUER}/oauth2/device/verify");
    let verify_resp = client
        .post(&verify_url)
        .headers(form_headers())
        .form(&[("user_code", dc.user_code.as_str())])
        .send()
        .await
        .map_err(|e| format!("device/verify failed: {e}"))?;
    let verify_final = verify_resp.url().to_string();
    let verify_body = verify_resp.text().await.unwrap_or_default();
    if is_rate_limited_url(&verify_final) || is_rate_limited_body(&verify_body) {
        return Err(format!("rate_limited on device/verify: {verify_final}"));
    }
    if !verify_final.contains("consent") {
        return Err(format!(
            "device/verify failed (expected consent): url={verify_final} body={}",
            truncate(&verify_body, 200)
        ));
    }

    // approve
    let approve_url = format!("{OIDC_ISSUER}/oauth2/device/approve");
    let approve_resp = client
        .post(&approve_url)
        .headers(form_headers())
        .form(&[
            ("user_code", dc.user_code.as_str()),
            ("action", "allow"),
            ("principal_type", "User"),
            ("principal_id", ""),
        ])
        .send()
        .await
        .map_err(|e| format!("device/approve failed: {e}"))?;
    let approve_final = approve_resp.url().to_string();
    let approve_body = approve_resp.text().await.unwrap_or_default();
    if is_rate_limited_url(&approve_final) || is_rate_limited_body(&approve_body) {
        return Err(format!("rate_limited on device/approve: {approve_final}"));
    }
    if !approve_final.contains("done") {
        return Err(format!(
            "device/approve failed (expected done): url={approve_final} body={}",
            truncate(&approve_body, 200)
        ));
    }
    tracing::info!("SSO→OAuth device authorize confirmed");

    // poll token
    poll_token(
        client,
        client_id,
        &dc.device_code,
        dc.interval.max(1),
        dc.expires_in,
        90,
    )
    .await
}

async fn request_device_code(
    client: &reqwest::Client,
    client_id: &str,
) -> Result<DeviceCodeResponse, String> {
    let url = format!("{OIDC_ISSUER}/oauth2/device/code");
    let resp = client
        .post(&url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[("client_id", client_id), ("scope", SCOPES)])
        .send()
        .await
        .map_err(|e| format!("device/code request failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "device/code HTTP {status}: {}",
            truncate(&body, 240)
        ));
    }
    serde_json::from_str(&body).map_err(|e| format!("device/code JSON parse: {e}; body={}", truncate(&body, 200)))
}

async fn poll_token(
    client: &reqwest::Client,
    client_id: &str,
    device_code: &str,
    mut interval: u64,
    expires_in: u64,
    timeout_secs: u64,
) -> Result<PolledToken, String> {
    let url = format!("{OIDC_ISSUER}/oauth2/token");
    let deadline = Instant::now() + Duration::from_secs(timeout_secs.min(expires_in));
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(interval.max(1))).await;
        let resp = client
            .post(&url)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&[
                (
                    "grant_type",
                    "urn:ietf:params:oauth:grant-type:device_code",
                ),
                ("client_id", client_id),
                ("device_code", device_code),
            ])
            .send()
            .await
            .map_err(|e| format!("token poll failed: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let parsed: TokenResponse = serde_json::from_str(&body).unwrap_or(TokenResponse {
            access_token: None,
            refresh_token: None,
            expires_in: None,
            token_type: None,
            error: None,
        });
        if let Some(err) = parsed.error.as_deref() {
            match err {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += 5;
                    continue;
                }
                other => {
                    return Err(format!("token error: {other} ({})", truncate(&body, 200)));
                }
            }
        }
        if status.is_success() {
            if let Some(access) = parsed
                .access_token
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                return Ok(PolledToken {
                    access_token: access.to_string(),
                    refresh_token: parsed
                        .refresh_token
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                    expires_in: parsed.expires_in,
                    token_type: parsed.token_type,
                });
            }
        }
        // unexpected non-success without oauth error field
        if !status.is_success() {
            return Err(format!(
                "token HTTP {status}: {}",
                truncate(&body, 240)
            ));
        }
    }
    Err("token poll timed out".into())
}

async fn validate_sso_session(client: &reqwest::Client) -> Result<(), String> {
    let resp = client
        .get(ACCOUNTS_HOME)
        .headers(browser_headers())
        .send()
        .await
        .map_err(|e| format!("accounts.x.ai probe failed: {e}"))?;
    let final_url = resp.url().to_string();
    let status = resp.status();
    let _ = resp.text().await;
    let lower = final_url.to_ascii_lowercase();
    if lower.contains("sign-in") || lower.contains("sign-up") || lower.contains("login") {
        return Err(format!("sso invalid (redirected to login): {final_url}"));
    }
    if is_rate_limited_url(&final_url) {
        return Err(format!("rate_limited on accounts.x.ai: {final_url}"));
    }
    // 2xx/3xx without login redirect is good enough
    if status.is_client_error() && status.as_u16() != 404 {
        return Err(format!(
            "sso probe unexpected status {status} url={final_url}"
        ));
    }
    Ok(())
}

async fn fetch_userinfo_email(client: &reqwest::Client, access_token: &str) -> Option<String> {
    let url = format!("{OIDC_ISSUER}/oauth2/userinfo");
    let resp = client
        .get(url)
        .bearer_auth(access_token)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let info: UserInfo = resp.json().await.ok()?;
    info.email
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn build_sso_session_client(
    config: &AppConfig,
    sso: &str,
) -> Result<(reqwest::Client, Arc<Jar>), String> {
    let jar = Arc::new(Jar::default());
    // Cookie must be visible to both accounts.x.ai and auth.x.ai
    let accounts = Url::parse("https://accounts.x.ai/").map_err(|e| e.to_string())?;
    let auth = Url::parse("https://auth.x.ai/").map_err(|e| e.to_string())?;
    let cookie_line = format!("sso={sso}; Domain=.x.ai; Path=/");
    jar.add_cookie_str(&cookie_line, &accounts);
    jar.add_cookie_str(&cookie_line, &auth);

    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .connect_timeout(Duration::from_secs(15))
        .cookie_provider(jar.clone())
        .redirect(Policy::limited(20))
        .user_agent(BROWSER_UA)
        .tcp_nodelay(true);

    match resolve_proxy_url(config) {
        Some(proxy_url) => {
            let proxy = reqwest::Proxy::all(&proxy_url)
                .map_err(|e| format!("invalid HTTP proxy `{proxy_url}`: {e}"))?;
            builder = builder.proxy(proxy);
            tracing::info!(proxy = %proxy_url, "SSO→OAuth client using proxy");
        }
        None => {
            builder = builder.no_proxy();
        }
    }

    let client = builder
        .build()
        .map_err(|e| format!("failed to build SSO session client: {e}"))?;
    Ok((client, jar))
}

fn browser_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(USER_AGENT, HeaderValue::from_static(BROWSER_UA));
    h.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        ),
    );
    h.insert(
        "Accept-Language",
        HeaderValue::from_static("en-US,en;q=0.9"),
    );
    h.insert(REFERER, HeaderValue::from_static("https://accounts.x.ai/"));
    h
}

fn form_headers() -> HeaderMap {
    let mut h = browser_headers();
    h.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    h.insert(ORIGIN, HeaderValue::from_static("https://auth.x.ai"));
    h.insert(REFERER, HeaderValue::from_static("https://auth.x.ai/"));
    h.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"));
    h
}

fn normalize_sso(raw: &str) -> String {
    let t = raw.trim();
    let t = t
        .strip_prefix("sso=")
        .or_else(|| t.strip_prefix("SSO="))
        .unwrap_or(t)
        .trim();
    // Strip accidental quotes
    t.trim_matches(|c| c == '"' || c == '\'').to_string()
}

fn is_rate_limited_msg(msg: &str) -> bool {
    is_rate_limited_body(msg) || is_rate_limited_url(msg)
}

fn is_rate_limited_url(url: &str) -> bool {
    let b = url.to_ascii_lowercase();
    b.contains("rate_limited")
        || b.contains("rate-limited")
        || b.contains("too_many_requests")
        || b.contains("ratelimit")
}

fn is_rate_limited_body(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    b.contains("rate_limited")
        || b.contains("rate-limited")
        || b.contains("too_many_requests")
        || b.contains("ratelimit")
}

fn is_transient(msg: &str) -> bool {
    let b = msg.to_ascii_lowercase();
    b.contains("timeout")
        || b.contains("timed out")
        || b.contains("connection")
        || b.contains("reset")
        || b.contains("temporarily")
}

fn backoff_sec(base: f64, attempt: u32, cap: f64) -> f64 {
    let base = if base <= 0.0 { 10.0 } else { base };
    let attempt = attempt.max(1);
    let shift = (attempt - 1).min(4);
    let d = (base * (2f64.powi(shift as i32))).min(cap);
    let jitter: f64 = rand::thread_rng().gen_range(0.0..5.0);
    d + jitter
}

fn truncate(s: &str, max: usize) -> String {
    let t = s.replace('\n', " ");
    if t.chars().count() <= max {
        t
    } else {
        format!("{}…", t.chars().take(max).collect::<String>())
    }
}

/// Apply conversion result onto an account (always `auth_kind=oauth`).
pub fn apply_oauth_to_account(
    account: &mut Account,
    converted: &ConvertedOauth,
    sso_source: Option<&str>,
) {
    account.auth_kind = AccountAuthKind::Oauth;
    account.access_token = Some(converted.access_token.clone());
    if let Some(rt) = converted
        .refresh_token
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        account.refresh_token = Some(rt.to_string());
    }
    account.token_type = Some(if converted.token_type.trim().is_empty() {
        "Bearer".into()
    } else {
        converted.token_type.clone()
    });
    account.expires_at = Some(Utc::now() + ChronoDuration::seconds(converted.expires_in as i64));
    account.last_refresh = Some(Utc::now());
    if let Some(email) = converted
        .email
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        account.email = Some(email.to_string());
        if account.name.starts_with("Imported ")
            || account.name.contains('@')
            || account.name.eq_ignore_ascii_case("sso")
            || account
                .email
                .as_ref()
                .map(|e| account.name == *e)
                .unwrap_or(false)
        {
            account.name = email.to_string();
        }
    }
    // Keep original SSO for re-convert notes only; not used for routing.
    if let Some(sso) = sso_source.map(str::trim).filter(|s| !s.is_empty()) {
        account.sso_token = Some(sso.to_string());
    }
    account.health = AccountHealth::Healthy;
    account.last_upstream_error = None;
    let note = format!(
        "SSO→OAuth device-flow ({})",
        Utc::now().format("%Y-%m-%d %H:%M")
    );
    account.notes = Some(match account.notes.as_ref() {
        Some(n) if !n.is_empty() => format!("{n}; {note}"),
        _ => note,
    });
}

/// Extract SSO JWT for conversion from account fields.
pub fn account_sso_cookie(account: &Account) -> Option<String> {
    if let Some(s) = account
        .sso_token
        .as_ref()
        .map(|s| normalize_sso(s))
        .filter(|s| !s.is_empty())
    {
        return Some(s);
    }
    // Legacy: SSO stored in access_token when auth_kind=sso
    if account.auth_kind == AccountAuthKind::Sso {
        if let Some(s) = account
            .access_token
            .as_ref()
            .map(|s| normalize_sso(s))
            .filter(|s| s.starts_with("eyJ"))
        {
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_prefix() {
        assert_eq!(normalize_sso("sso=abc.def"), "abc.def");
        assert_eq!(normalize_sso("  'eyJxx'  "), "eyJxx");
    }

    #[test]
    fn rate_limit_detect() {
        assert!(is_rate_limited_url("https://auth.x.ai/rate_limited"));
        assert!(is_rate_limited_body("error: too_many_requests"));
    }
}
