use base64::engine::general_purpose::{URL_SAFE_NO_PAD, STANDARD};
use base64::Engine;
use chrono::{Duration, Utc};
use once_cell::sync::Lazy;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{oneshot, Mutex};
use url::Url;

use crate::config::{load_auth, save_auth, Account, AccountHealth, AppConfig};
use crate::error::{AppError, AppResult};
use crate::http_client::build_oauth_http_client;

const XAI_DISCOVERY: &str = "https://auth.x.ai/.well-known/openid-configuration";
/// Drop unfinished OAuth flows after this age to avoid unbounded pending map growth.
const PENDING_OAUTH_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Shared lightweight client for OAuth discovery / token exchange.
static OAUTH_HTTP: Lazy<parking_lot::RwLock<reqwest::Client>> = Lazy::new(|| {
    let config = crate::config::load_config().unwrap_or_default();
    let client = build_oauth_http_client(&config).unwrap_or_else(|_| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .expect("oauth http client")
    });
    parking_lot::RwLock::new(client)
});

/// Serialize concurrent "Add login" clicks so we don't spawn many placeholder accounts.
static LOGIN_GATE: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

pub fn rebuild_oauth_http_client(config: &AppConfig) -> AppResult<()> {
    let client = build_oauth_http_client(config)?;
    *OAUTH_HTTP.write() = client;
    Ok(())
}

fn oauth_http() -> reqwest::Client {
    OAUTH_HTTP.read().clone()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthStart {
    pub account_id: String,
    pub authorize_url: String,
    /// Whether the backend believes it launched a browser.
    pub browser_opened: bool,
}

#[derive(Debug, Clone)]
struct PendingOAuth {
    account_id: String,
    code_verifier: String,
    state: String,
    redirect_uri: String,
    created_at: Instant,
}

#[derive(Clone, Default)]
pub struct OAuthManager {
    pending: Arc<Mutex<HashMap<String, PendingOAuth>>>,
}

impl OAuthManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start OAuth. Prefer `account_id` for re-login so we always bind the same account
    /// (name/email may change after userinfo fill).
    pub async fn start_login(
        &self,
        config: &AppConfig,
        account_name: Option<String>,
        account_id: Option<String>,
    ) -> AppResult<OAuthStart> {
        let _gate = LOGIN_GATE.lock().await;

        // Always rebuild OAuth client from current config so proxy setting changes apply.
        let _ = rebuild_oauth_http_client(config);

        // 1) Network-facing steps first — never create accounts if discovery fails.
        let discovery = fetch_discovery().await?;

        // 2) Ensure local callback is listening before opening the browser.
        self.spawn_callback_server(config.oauth_redirect_port).await?;

        // 3) Resolve / create a single account record.
        let mut store = load_auth()?;
        let reusing_id = account_id
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // Drop unfinished placeholders so repeated clicks don't stack "Account N".
        // Keep the account we are re-logging into.
        store.accounts.retain(|a| {
            if reusing_id.as_deref() == Some(a.id.as_str()) {
                return true;
            }
            token_present(&a.access_token) || token_present(&a.refresh_token)
        });

        let account = if let Some(id) = reusing_id {
            store
                .accounts
                .iter()
                .find(|a| a.id == id)
                .cloned()
                .ok_or_else(|| AppError::msg(format!("account not found: {id}")))?
        } else if let Some(name) = account_name.filter(|s| !s.trim().is_empty()) {
            if let Some(acc) = store
                .accounts
                .iter()
                .find(|a| a.name == name || a.email.as_deref() == Some(name.as_str()))
                .cloned()
            {
                acc
            } else {
                let acc = Account::new(name);
                store.accounts.push(acc.clone());
                acc
            }
        } else {
            let acc = Account::new("Pending login");
            store.accounts.push(acc.clone());
            acc
        };
        save_auth(&store)?;

        let code_verifier = random_string(64);
        let code_challenge = pkce_challenge(&code_verifier);
        let state = random_string(24);
        // Must match the allowlist for the public client id used by hermes/grok-cli.
        // Registered form: http://127.0.0.1:56121/callback
        let redirect_uri = format!("http://127.0.0.1:{}/callback", config.oauth_redirect_port);

        let mut authorize = Url::parse(&discovery.authorization_endpoint)
            .map_err(|e| AppError::msg(format!("invalid authorization endpoint: {e}")))?;
        {
            let mut qp = authorize.query_pairs_mut();
            qp.append_pair("response_type", "code");
            qp.append_pair("client_id", &config.xai_client_id);
            qp.append_pair("redirect_uri", &redirect_uri);
            // grok-cli:access + api:access are required for SuperGrok / API quotas
            qp.append_pair(
                "scope",
                "openid profile email offline_access grok-cli:access api:access",
            );
            qp.append_pair("state", &state);
            qp.append_pair("code_challenge", &code_challenge);
            qp.append_pair("code_challenge_method", "S256");
            // Known working params for this public client (hermes-agent style)
            qp.append_pair("plan", "generic");
            qp.append_pair("referrer", "hermes-agent");
        }

        {
            let mut pending = self.pending.lock().await;
            // Evict stale flows so abandoned logins cannot leak forever.
            pending.retain(|_, p| p.created_at.elapsed() < PENDING_OAUTH_TTL);
            pending.insert(
                state.clone(),
                PendingOAuth {
                    account_id: account.id.clone(),
                    code_verifier,
                    state: state.clone(),
                    redirect_uri: redirect_uri.clone(),
                    created_at: Instant::now(),
                },
            );
        }

        Ok(OAuthStart {
            account_id: account.id,
            authorize_url: authorize.to_string(),
            browser_opened: false, // filled by command after open attempt
        })
    }

    async fn spawn_callback_server(&self, port: u16) -> AppResult<()> {
        // Track bind on the manager instance (not process-global) so rebuilds can re-bind
        // if the previous accept loop is gone; still avoid double-bind on the same port.
        static BIND_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<Option<u16>>> =
            once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(None));

        let mut bound = BIND_LOCK.lock().await;
        if *bound == Some(port) {
            // Port claimed by a previous successful start in this process.
            return Ok(());
        }

        let manager = self.clone();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(err) => {
                // If something else already listens (leftover from prior run), treat as OK
                // only when connection to callback path is possible — otherwise surface error.
                if is_local_port_open(port).await {
                    *bound = Some(port);
                    return Ok(());
                }
                return Err(AppError::msg(format!(
                    "oauth callback bind failed on 127.0.0.1:{port}: {err}"
                )));
            }
        };
        *bound = Some(port);
        drop(bound);

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let manager = manager.clone();
                tokio::spawn(async move {
                    let _ = handle_oauth_connection(stream, manager).await;
                });
            }
        });
        Ok(())
    }

    pub async fn complete(&self, config: &AppConfig, state: &str, code: &str) -> AppResult<String> {
        let pending = {
            let mut guard = self.pending.lock().await;
            guard.retain(|_, p| p.created_at.elapsed() < PENDING_OAUTH_TTL);
            guard
                .remove(state)
                .ok_or_else(|| AppError::msg("unknown oauth state"))?
        };
        let discovery = fetch_discovery().await?;
        let client = oauth_http();
        let resp = client
            .post(&discovery.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", config.xai_client_id.as_str()),
                ("code", code),
                ("redirect_uri", pending.redirect_uri.as_str()),
                ("code_verifier", pending.code_verifier.as_str()),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::msg(format!("oauth token exchange failed: {body}")));
        }
        let token: TokenResponse = resp.json().await?;
        let access = token.access_token.clone();
        // Prefer userinfo; fall back to email claim inside id_token / access JWT.
        let profile = fetch_userinfo(&discovery, &access).await;
        let email = profile
            .as_ref()
            .and_then(|p| p.email.clone())
            .filter(|e| !e.is_empty())
            .or_else(|| token.id_token.as_deref().and_then(email_from_jwt))
            .or_else(|| email_from_jwt(&access));
        let display = email
            .clone()
            .or_else(|| {
                profile.as_ref().and_then(|p| {
                    p.preferred_username
                        .clone()
                        .or_else(|| p.name.clone())
                        .filter(|s| !s.is_empty())
                })
            });

        let mut store = load_auth()?;
        if let Some(account) = store.accounts.iter_mut().find(|a| a.id == pending.account_id) {
            account.access_token = Some(token.access_token);
            if let Some(refresh) = token.refresh_token {
                account.refresh_token = Some(refresh);
            }
            account.token_type = Some(token.token_type.unwrap_or_else(|| "Bearer".into()));
            if let Some(expires_in) = token.expires_in {
                account.expires_at = Some(Utc::now() + Duration::seconds(expires_in as i64));
            }
            account.last_refresh = Some(Utc::now());
            account.health = AccountHealth::Healthy;
            account.enabled = true;
            account.consecutive_failures = 0;
            account.cooldown_until = None;
            account.last_upstream_error = None;
            if let Some(email) = email {
                account.email = Some(email.clone());
                account.name = email;
            } else if let Some(name) = display {
                account.name = name;
            }
            tracing::info!(
                account_id = %account.id,
                name = %account.name,
                email = ?account.email,
                "oauth login completed"
            );
        } else {
            tracing::warn!(
                account_id = %pending.account_id,
                "oauth complete: account missing from store"
            );
        }
        save_auth(&store)?;
        Ok(pending.account_id)
    }
}

#[derive(Debug, Deserialize)]
struct Discovery {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    userinfo_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    email: Option<String>,
    preferred_username: Option<String>,
    name: Option<String>,
    #[serde(default)]
    preferred_username_alt: Option<String>,
}

async fn fetch_userinfo(discovery: &Discovery, access_token: &str) -> Option<UserInfo> {
    let url = discovery
        .userinfo_endpoint
        .clone()
        .unwrap_or_else(|| "https://auth.x.ai/oauth2/userinfo".into());
    let client = oauth_http();
    let resp = match client.get(&url).bearer_auth(access_token).send().await {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!("userinfo request failed: {err}");
            return None;
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!("userinfo fetch failed: {status} {body}");
        return None;
    }
    match resp.json::<UserInfo>().await {
        Ok(mut info) => {
            // Some providers put preferred_username under alternate keys only.
            if info.preferred_username.is_none() {
                info.preferred_username = info.preferred_username_alt.take();
            }
            Some(info)
        }
        Err(err) => {
            tracing::warn!("userinfo json parse failed: {err}");
            None
        }
    }
}

/// Decode JWT payload (no verify) and pull `email` / `preferred_username` claims.
fn email_from_jwt(token: &str) -> Option<String> {
    let payload = jwt_payload(token)?;
    payload
        .get("email")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && s.contains('@'))
        .map(|s| s.to_string())
        .or_else(|| {
            payload
                .get("preferred_username")
                .and_then(|v| v.as_str())
                .filter(|s| s.contains('@'))
                .map(|s| s.to_string())
        })
}

fn jwt_payload(token: &str) -> Option<serde_json::Value> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let decoded = b64url_decode(payload_b64)?;
    serde_json::from_slice(&decoded).ok()
}

fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut s = input.replace('-', "+").replace('_', "/");
    while s.len() % 4 != 0 {
        s.push('=');
    }
    STANDARD.decode(s).ok()
}

fn token_present(token: &Option<String>) -> bool {
    token.as_ref().map(|t| !t.trim().is_empty()).unwrap_or(false)
}

async fn is_local_port_open(port: u16) -> bool {
    tokio::net::TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .is_ok()
}

async fn fetch_discovery() -> AppResult<Discovery> {
    let client = oauth_http();
    // Hard cap so the UI never hangs indefinitely when auth.x.ai is unreachable.
    let send = client.get(XAI_DISCOVERY).send();
    let resp = match tokio::time::timeout(std::time::Duration::from_secs(20), send).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            let hint = proxy_hint();
            return Err(AppError::msg(format!(
                "无法访问 xAI OAuth（auth.x.ai）：{e}。{hint}"
            )));
        }
        Err(_) => {
            let hint = proxy_hint();
            return Err(AppError::msg(format!(
                "连接 auth.x.ai 超时（20s）。{hint}"
            )));
        }
    };
    if !resp.status().is_success() {
        return Err(AppError::msg(format!(
            "xAI OAuth discovery 返回 {}。{}",
            resp.status(),
            proxy_hint()
        )));
    }
    resp.json()
        .await
        .map_err(|e| AppError::msg(format!("解析 OAuth discovery 失败：{e}")))
}

fn proxy_hint() -> String {
    // Prefer live config status (settings toggle only; env is never auto-applied).
    if let Ok(cfg) = crate::config::load_config() {
        let status = crate::http_client::proxy_status_hint(&cfg);
        return format!("{status} 若仍无法访问 auth.x.ai，请到「设置」开启上游 HTTP 代理后保存再登录。");
    }
    "请到「设置」开启上游 HTTP 代理（如 http://127.0.0.1:7890）后保存再登录。".into()
}

pub async fn ensure_fresh_token(config: &AppConfig, account: &mut Account) -> AppResult<String> {
    if let Some(token) = account.access_token.clone() {
        let expiring = account
            .expires_at
            .map(|exp| exp <= Utc::now() + Duration::seconds(120))
            .unwrap_or(false);
        if !expiring {
            return Ok(token);
        }
    }
    refresh_account(config, account).await
}

pub async fn refresh_account(config: &AppConfig, account: &mut Account) -> AppResult<String> {
    let refresh = account
        .refresh_token
        .clone()
        .ok_or_else(|| AppError::msg("account missing refresh token; re-login required"))?;
    let discovery = fetch_discovery().await?;
    let client = oauth_http();
    let resp = client
        .post(&discovery.token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", config.xai_client_id.as_str()),
            ("refresh_token", refresh.as_str()),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        account.health = AccountHealth::Degraded;
        account.consecutive_failures += 1;
        account.last_failure_at = Some(Utc::now());
        return Err(AppError::msg("refresh token failed; re-login required"));
    }
    let token: TokenResponse = resp.json().await?;
    account.access_token = Some(token.access_token.clone());
    if let Some(refresh_token) = token.refresh_token {
        account.refresh_token = Some(refresh_token);
    }
    if let Some(expires_in) = token.expires_in {
        account.expires_at = Some(Utc::now() + Duration::seconds(expires_in as i64));
    }
    account.last_refresh = Some(Utc::now());
    account.health = AccountHealth::Healthy;
    account.consecutive_failures = 0;
    account.cooldown_until = None;
    Ok(token.access_token)
}

pub fn mark_success(account: &mut Account) {
    account.last_success_at = Some(Utc::now());
    account.consecutive_failures = 0;
    account.last_upstream_error = None;
    if account.health != AccountHealth::Disabled {
        account.health = AccountHealth::Healthy;
        account.cooldown_until = None;
    }
}

/// Why a request failed — drives cooldown policy.
/// Local "cooldown" is **not** xAI subscription quota; only rate-limit signals should lock out.
#[derive(Debug, Clone, Copy)]
pub enum FailureKind {
    /// TCP/TLS/proxy/DNS — do not lock the account.
    Transport,
    /// 401/403 after refresh path — degraded, re-login may be needed.
    Auth,
    /// 5xx / other soft errors — degrade; cooldown only after many consecutive hits.
    Soft,
    /// Explicit 429 / rate-limit headers — temporary cooldown (honors Retry-After when known).
    RateLimit { retry_after_secs: u64 },
}

pub fn mark_failure(account: &mut Account, hard: bool) {
    // Backward-compatible wrapper: hard ≈ auth-ish, soft ≈ soft.
    mark_failure_kind(
        account,
        if hard {
            FailureKind::Auth
        } else {
            FailureKind::Soft
        },
    );
}

pub fn mark_failure_kind(account: &mut Account, kind: FailureKind) {
    account.last_failure_at = Some(Utc::now());
    account.consecutive_failures = account.consecutive_failures.saturating_add(1);
    match kind {
        FailureKind::Transport => {
            // Transient network issues must not look like "account cooldown".
            if account.health != AccountHealth::Disabled
                && account.health != AccountHealth::Cooldown
            {
                account.health = AccountHealth::Degraded;
            }
            account.last_upstream_error =
                Some("transport error (network/proxy); account not locked".into());
        }
        FailureKind::Auth => {
            // Short local cooldown so multi-account routing skips this account on the next
            // pick (and in-request failover can move on). Not a permanent ban — Accounts
            // UI can clear cooldown; single-account users only wait briefly.
            if account.health != AccountHealth::Disabled {
                const AUTH_COOLDOWN_SECS: i64 = 60;
                account.health = AccountHealth::Cooldown;
                account.cooldown_until =
                    Some(Utc::now() + Duration::seconds(AUTH_COOLDOWN_SECS));
                account.last_upstream_error = Some(format!(
                    "auth error (401/403); cooldown {AUTH_COOLDOWN_SECS}s — re-login or fix console.x.ai permissions"
                ));
            }
        }
        FailureKind::Soft => {
            if account.health != AccountHealth::Disabled {
                // Only escalate to cooldown after many consecutive soft failures.
                if account.consecutive_failures >= 8 {
                    account.health = AccountHealth::Cooldown;
                    account.cooldown_until = Some(Utc::now() + Duration::seconds(60));
                    account.last_upstream_error =
                        Some("many consecutive upstream errors; short cooldown".into());
                } else {
                    account.health = AccountHealth::Degraded;
                    account.last_upstream_error = Some("upstream soft error".into());
                }
            }
        }
        FailureKind::RateLimit { retry_after_secs } => {
            let secs = retry_after_secs.clamp(5, 900);
            account.health = AccountHealth::Cooldown;
            account.cooldown_until = Some(Utc::now() + Duration::seconds(secs as i64));
            account.last_upstream_error = Some(format!(
                "rate limited (429); cooldown {secs}s — not a permanent ban"
            ));
        }
    }
}

/// Apply rate-limit snapshot from upstream response headers (when xAI sends them).
pub fn apply_rate_limit_headers(account: &mut Account, headers: &reqwest::header::HeaderMap) {
    let limit = header_u64(headers, &["x-ratelimit-limit-requests", "x-rate-limit-limit", "x-ratelimit-limit"]);
    let remaining = header_u64(
        headers,
        &[
            "x-ratelimit-remaining-requests",
            "x-rate-limit-remaining",
            "x-ratelimit-remaining",
        ],
    );
    let reset = header_u64(
        headers,
        &[
            "x-ratelimit-reset-requests",
            "x-rate-limit-reset",
            "x-ratelimit-reset",
        ],
    )
    .and_then(|v| {
        // Unix seconds if large; otherwise delta seconds.
        if v > 1_000_000_000 {
            chrono::DateTime::from_timestamp(v as i64, 0)
        } else {
            Some(Utc::now() + Duration::seconds(v as i64))
        }
    });
    if limit.is_some() {
        account.rate_limit_limit = limit;
    }
    if remaining.is_some() {
        account.rate_limit_remaining = remaining;
    }
    if reset.is_some() {
        account.rate_limit_reset_at = reset;
    }
}

pub fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| {
            header_u64(
                headers,
                &["x-ratelimit-reset-requests", "x-rate-limit-reset", "x-ratelimit-reset"],
            )
            .map(|v| {
                if v > 1_000_000_000 {
                    let now = Utc::now().timestamp().max(0) as u64;
                    v.saturating_sub(now).max(5)
                } else {
                    v.max(5)
                }
            })
        })
}

fn header_u64(headers: &reqwest::header::HeaderMap, names: &[&str]) -> Option<u64> {
    for name in names {
        if let Some(v) = headers.get(*name).and_then(|h| h.to_str().ok()) {
            if let Ok(n) = v.trim().parse::<u64>() {
                return Some(n);
            }
            // Some APIs send floats.
            if let Ok(f) = v.trim().parse::<f64>() {
                return Some(f as u64);
            }
        }
    }
    None
}

/// Clear expired cooldowns so UI / routing match reality.
pub fn clear_expired_cooldowns_in_store(store: &mut crate::config::AuthStore) -> bool {
    let now = Utc::now();
    let mut changed = false;
    for account in &mut store.accounts {
        if account.health == AccountHealth::Cooldown {
            let expired = account
                .cooldown_until
                .map(|t| t <= now)
                .unwrap_or(true);
            if expired {
                account.health = AccountHealth::Healthy;
                account.cooldown_until = None;
                account.consecutive_failures = 0;
                changed = true;
            }
        }
    }
    changed
}

/// Force-clear cooldown on one account (user action).
pub fn clear_account_cooldown(account_id: &str) -> AppResult<()> {
    let mut store = load_auth()?;
    if let Some(account) = store.accounts.iter_mut().find(|a| a.id == account_id) {
        account.health = AccountHealth::Healthy;
        account.cooldown_until = None;
        account.consecutive_failures = 0;
        account.last_upstream_error = None;
        save_auth(&store)?;
        Ok(())
    } else {
        Err(AppError::msg("account not found"))
    }
}

fn random_string(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

async fn handle_oauth_connection(mut stream: tokio::net::TcpStream, manager: OAuthManager) -> AppResult<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req.lines().next().unwrap_or_default();
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
    let url = format!("http://127.0.0.1{path}");
    let parsed = Url::parse(&url).map_err(|e| AppError::msg(e.to_string()))?;
    let mut pairs = parsed.query_pairs();
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (k, v) in pairs.by_ref() {
        match k.as_ref() {
            "code" => code = Some(v.to_string()),
            "state" => state = Some(v.to_string()),
            "error" => error = Some(v.to_string()),
            _ => {}
        }
    }
    let body = if let Some(err) = error {
        format!("<html><body><h2>Login failed</h2><p>{err}</p></body></html>")
    } else if let (Some(code), Some(state)) = (code, state) {
        let config = crate::config::load_config()?;
        match manager.complete(&config, &state, &code).await {
            Ok(_) => "<html><body><h2>Login successful</h2><p>You can close this window and return to GrokGo.</p></body></html>".into(),
            Err(err) => format!("<html><body><h2>Login failed</h2><p>{err}</p></body></html>"),
        }
    } else {
        "<html><body><h2>Invalid callback</h2></body></html>".into()
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(resp.as_bytes()).await?;
    Ok(())
}

// silence unused import warning helper
#[allow(dead_code)]
fn _unused() {
    let _ = STANDARD;
    let _ = oneshot::channel::<()>();
}
