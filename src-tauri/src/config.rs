use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::paths::{auth_path, config_path};

/// In-memory config/auth to avoid re-reading JSON on every proxy request.
static CONFIG_CACHE: Lazy<RwLock<Option<AppConfig>>> = Lazy::new(|| RwLock::new(None));
static AUTH_CACHE: Lazy<RwLock<Option<AuthStore>>> = Lazy::new(|| RwLock::new(None));
/// Serializes load-modify-save on auth.json so async workers cannot resurrect deleted accounts.
static AUTH_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub preferred_port: u16,
    pub actual_port: u16,
    pub bind_host: String,
    pub lan_enabled: bool,
    pub require_token: bool,
    pub local_token: String,
    pub default_model: String,
    pub default_image_model: String,
    pub default_video_model: String,
    pub model_mappings: BTreeMap<String, String>,
    pub routing_strategy: RoutingStrategy,
    /// Stick multi-turn sessions to the same account (prompt-cache friendly). Default true.
    #[serde(default = "default_true")]
    pub session_affinity: bool,
    /// How long session→account bindings live (seconds). Default 3600.
    #[serde(default = "default_affinity_ttl")]
    pub session_affinity_ttl_secs: u64,
    /// Soft-weight picks by SuperGrok remaining % / rate-limit remaining. Default true.
    #[serde(default = "default_true")]
    pub quota_aware_routing: bool,
    /// Prefer accounts whose weekly quota resets soonest (use-it-or-lose-it). Default false.
    #[serde(default)]
    pub prefer_soonest_reset: bool,
    /// Soft per-account in-flight cap used as a pick preference. 0 = unlimited. Default 6.
    #[serde(default = "default_account_max_concurrency")]
    pub account_max_concurrency: u32,
    /// When true, `/v1/responses` silently retries once if upstream returns a
    /// reasoning-only empty completion (no message / tool call). Prevents Codex
    /// from ending the turn mid-task. Default true.
    #[serde(default = "default_true")]
    pub empty_completion_retry: bool,
    pub auto_inject_codex_mcp: bool,
    pub launch_on_startup: bool,
    pub minimize_to_tray: bool,
    /// Public hermes/grok-cli OAuth client id. Empty values are rejected at runtime
    /// and replaced with [`DEFAULT_XAI_CLIENT_ID`].
    #[serde(default = "default_xai_client_id")]
    pub xai_client_id: String,
    #[serde(default = "default_xai_base_url")]
    pub xai_base_url: String,
    #[serde(default = "default_oauth_redirect_port")]
    pub oauth_redirect_port: u16,
    /// When true, upstream xAI/OAuth HTTP goes through `http_proxy_url`.
    #[serde(default)]
    pub http_proxy_enabled: bool,
    /// e.g. `http://127.0.0.1:7890` or `socks5://127.0.0.1:1080`
    #[serde(default)]
    pub http_proxy_url: String,
    /// Dock / window / tray brand: dark (black bg, white logo) or light (white bg, black logo).
    #[serde(default)]
    pub app_icon: AppIconStyle,
    /// MCP tools exposed via `tools/list` / `tools/call`.
    /// `None` = all catalog tools (legacy default). `Some(list)` = only those names.
    #[serde(default)]
    pub mcp_enabled_tools: Option<Vec<String>>,
    /// Legacy: grok.com SSO reverse channel removed. Card SSO is converted to OAuth via Device Flow.
    /// Kept for config.json compatibility; ignored at runtime.
    #[serde(default)]
    pub sso_enabled: bool,
    /// Legacy unused (SSO reverse removed).
    #[serde(default)]
    pub sso_cf_clearance: String,
    /// Legacy unused (SSO reverse removed).
    #[serde(default)]
    pub sso_user_agent: String,
}

/// Canonical MCP tool ids shipped by the gateway (order matches tools/list).
pub fn default_mcp_tool_ids() -> &'static [&'static str] {
    &[
        "x_search",
        "image_gen",
        "image_generate",
        "image_edit",
        "video_generate",
        "video_edit",
    ]
}

impl AppConfig {
    /// Whether an MCP tool is enabled for listing and calling.
    pub fn mcp_tool_enabled(&self, name: &str) -> bool {
        match &self.mcp_enabled_tools {
            None => true,
            Some(list) => list.iter().any(|t| t == name),
        }
    }

    /// Non-empty OAuth client id; falls back to the built-in public client.
    pub fn effective_xai_client_id(&self) -> &str {
        let t = self.xai_client_id.trim();
        if t.is_empty() {
            DEFAULT_XAI_CLIENT_ID
        } else {
            t
        }
    }
}

/// Application icon style variants shipped under `icons/variants/{dark,light}/`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AppIconStyle {
    /// Black background, white logo (default).
    #[default]
    Dark,
    /// White background, black logo.
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RoutingStrategy {
    WeightedRoundRobin,
    LeastRecentlyUsed,
    LowestErrorRate,
    /// Drain the primary healthy account before using backups (cold standby).
    FillFirst,
}

impl Default for RoutingStrategy {
    fn default() -> Self {
        Self::WeightedRoundRobin
    }
}

fn default_true() -> bool {
    true
}

fn default_affinity_ttl() -> u64 {
    3600
}

fn default_account_max_concurrency() -> u32 {
    6
}

/// Public hermes / grok-cli client registered with xAI (redirect `http://127.0.0.1:56121/callback`).
pub const DEFAULT_XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

fn default_xai_client_id() -> String {
    DEFAULT_XAI_CLIENT_ID.into()
}

fn default_xai_base_url() -> String {
    "https://api.x.ai/v1".into()
}

fn default_oauth_redirect_port() -> u16 {
    56121
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            preferred_port: 8787,
            actual_port: 8787,
            bind_host: "127.0.0.1".into(),
            lan_enabled: false,
            require_token: true,
            local_token: random_token(),
            default_model: "grok-4.5".into(),
            default_image_model: "grok-imagine-image-quality".into(),
            default_video_model: "grok-imagine-video".into(),
            model_mappings: BTreeMap::from([
                ("gpt-5.6".into(), "grok-4.5".into()),
                ("gpt-5.5".into(), "grok-4.5".into()),
            ]),
            routing_strategy: RoutingStrategy::WeightedRoundRobin,
            session_affinity: true,
            session_affinity_ttl_secs: 3600,
            quota_aware_routing: true,
            prefer_soonest_reset: false,
            account_max_concurrency: 6,
            empty_completion_retry: true,
            auto_inject_codex_mcp: false,
            launch_on_startup: false,
            minimize_to_tray: true,
            // Public hermes/grok-cli client — redirect_uri must match the
            // allowlist registered with xAI (exact port + path).
            xai_client_id: default_xai_client_id(),
            xai_base_url: default_xai_base_url(),
            oauth_redirect_port: default_oauth_redirect_port(),
            http_proxy_enabled: false,
            http_proxy_url: String::new(),
            app_icon: AppIconStyle::Dark,
            mcp_enabled_tools: None,
            sso_enabled: false,
            sso_cf_clearance: String::new(),
            sso_user_agent: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthStore {
    #[serde(default)]
    pub accounts: Vec<Account>,
}

/// How this account authenticates to upstream Grok/xAI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AccountAuthKind {
    /// Official xAI OAuth (access/refresh → api.x.ai / cli-chat-proxy).
    #[default]
    Oauth,
    /// Legacy: card SSO before conversion. Not routable until converted to OAuth.
    Sso,
}

/// Legacy SSO pool tier (reverse channel removed). Kept for auth.json compatibility.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SsoPoolTier {
    #[default]
    Basic,
    Super,
    Heavy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    /// OIDC email when available (also used as display name after login).
    #[serde(default)]
    pub email: Option<String>,
    pub enabled: bool,
    pub weight: u32,
    /// Authentication channel. Runtime routing requires OAuth tokens.
    #[serde(default)]
    pub auth_kind: AccountAuthKind,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    /// Original card SSO JWT (kept after SSO→OAuth convert for re-import; not used for API).
    #[serde(default)]
    pub sso_token: Option<String>,
    /// Optional password from card import (local notes only; never sent upstream).
    #[serde(default)]
    pub password_hint: Option<String>,
    /// Legacy field (SSO reverse removed).
    #[serde(default)]
    pub sso_pool: SsoPoolTier,
    pub token_type: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_refresh: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub health: AccountHealth,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub daily_limit_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
    pub notes: Option<String>,
    /// From upstream `x-ratelimit-*` / `x-rate-limit-*` response headers when present.
    #[serde(default)]
    pub rate_limit_limit: Option<u64>,
    #[serde(default)]
    pub rate_limit_remaining: Option<u64>,
    #[serde(default)]
    pub rate_limit_reset_at: Option<DateTime<Utc>>,
    /// Short last upstream error (status / rate-limit hint), for UI diagnostics.
    #[serde(default)]
    pub last_upstream_error: Option<String>,
    /// SuperGrok weekly credit quota from grok.com GrokBuildBilling.
    #[serde(default)]
    pub quota: Option<crate::quota::AccountQuotaSnapshot>,
    /// Whether this account may be selected for image generation. Default true.
    /// Toggle off for text-only / non-media subscription accounts.
    #[serde(default = "default_true")]
    pub supports_image: bool,
    /// Whether this account may be selected for video generation. Default true.
    #[serde(default = "default_true")]
    pub supports_video: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AccountHealth {
    Healthy,
    Degraded,
    Cooldown,
    Disabled,
}

impl Default for AccountHealth {
    fn default() -> Self {
        Self::Healthy
    }
}

impl Account {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            email: None,
            enabled: true,
            weight: 1,
            auth_kind: AccountAuthKind::Oauth,
            access_token: None,
            refresh_token: None,
            sso_token: None,
            password_hint: None,
            sso_pool: SsoPoolTier::Basic,
            token_type: Some("Bearer".into()),
            expires_at: None,
            last_refresh: None,
            last_success_at: None,
            last_failure_at: None,
            consecutive_failures: 0,
            health: AccountHealth::Healthy,
            cooldown_until: None,
            daily_limit_usd: None,
            monthly_limit_usd: None,
            notes: None,
            rate_limit_limit: None,
            rate_limit_remaining: None,
            rate_limit_reset_at: None,
            last_upstream_error: None,
            quota: None,
            supports_image: true,
            supports_video: true,
        }
    }

    /// True if this account can be used for upstream OAuth API calls.
    /// Legacy `auth_kind=sso` rows without access/refresh are **not** credentialed
    /// (convert with SSO→OAuth first).
    pub fn is_credentialed(&self) -> bool {
        self.access_token
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
            || self
                .refresh_token
                .as_ref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
    }

    /// Effective SSO JWT (prefers dedicated field, falls back to access_token).
    pub fn effective_sso_token(&self) -> Option<&str> {
        self.sso_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.access_token
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && s.starts_with("eyJ"))
            })
    }
}

pub fn load_config() -> AppResult<AppConfig> {
    if let Some(cached) = CONFIG_CACHE.read().clone() {
        return Ok(cached);
    }
    let path = config_path()?;
    let cfg = match load_json_file::<AppConfig>(&path, "config")? {
        Some(cfg) => cfg,
        None => {
            let cfg = AppConfig::default();
            save_config(&cfg)?;
            return Ok(cfg);
        }
    };
    *CONFIG_CACHE.write() = Some(cfg.clone());
    Ok(cfg)
}

pub fn save_config(config: &AppConfig) -> AppResult<()> {
    let path = config_path()?;
    write_json_atomic(&path, config)?;
    *CONFIG_CACHE.write() = Some(config.clone());
    Ok(())
}

pub fn load_auth() -> AppResult<AuthStore> {
    let _guard = AUTH_LOCK.lock();
    load_auth_unlocked()
}

pub fn save_auth(store: &AuthStore) -> AppResult<()> {
    let _guard = AUTH_LOCK.lock();
    save_auth_unlocked(store)
}

fn load_auth_unlocked() -> AppResult<AuthStore> {
    if let Some(cached) = AUTH_CACHE.read().clone() {
        return Ok(cached);
    }
    let path = auth_path()?;
    let store = match load_json_file::<AuthStore>(&path, "auth")? {
        Some(store) => store,
        None => {
            let store = AuthStore::default();
            save_auth_unlocked(&store)?;
            return Ok(store);
        }
    };
    *AUTH_CACHE.write() = Some(store.clone());
    Ok(store)
}

fn save_auth_unlocked(store: &AuthStore) -> AppResult<()> {
    let path = auth_path()?;
    write_json_atomic(&path, store)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    *AUTH_CACHE.write() = Some(store.clone());
    tracing::debug!(
        path = %path.display(),
        accounts = store.accounts.len(),
        "auth.json saved"
    );
    Ok(())
}

/// Atomically load → mutate → save under the auth lock.
/// Use for any multi-step mutation that must not race with deletes.
#[allow(dead_code)]
pub fn with_auth_mut<F, R>(f: F) -> AppResult<R>
where
    F: FnOnce(&mut AuthStore) -> AppResult<R>,
{
    let _guard = AUTH_LOCK.lock();
    let mut store = load_auth_unlocked()?;
    let out = f(&mut store)?;
    save_auth_unlocked(&store)?;
    Ok(out)
}

/// Merge fields of an existing account into the **current** store.
///
/// Critical: never re-inserts an account that was deleted while async work ran.
/// Call this after `await` (token refresh, quota probe, etc.) instead of
/// `save_auth` on a stale full-store clone.
///
/// Concurrent proxy traffic may have fresher rate-limit / success timestamps on
/// the live slot — those win over a stale snapshot that started before `await`.
///
/// Returns `true` if the account was found and updated, `false` if it was gone
/// (deleted) — callers should treat that as a no-op, not re-create the row.
pub fn apply_account_update(account: &Account) -> AppResult<bool> {
    let _guard = AUTH_LOCK.lock();
    let mut store = load_auth_unlocked()?;
    if let Some(slot) = store.accounts.iter_mut().find(|a| a.id == account.id) {
        let live_rl_limit = slot.rate_limit_limit;
        let live_rl_rem = slot.rate_limit_remaining;
        let live_rl_reset = slot.rate_limit_reset_at;
        let live_success = slot.last_success_at;
        let live_failure = slot.last_failure_at;
        let live_upstream_err = slot.last_upstream_error.clone();
        let live_failures = slot.consecutive_failures;
        let live_health = slot.health.clone();
        let live_cooldown = slot.cooldown_until;

        *slot = account.clone();

        // Prefer the more-consumed rate-limit snapshot (lower remaining = newer traffic).
        match (live_rl_rem, slot.rate_limit_remaining) {
            (Some(live), Some(incoming)) if live < incoming => {
                slot.rate_limit_remaining = Some(live);
                if live_rl_limit.is_some() {
                    slot.rate_limit_limit = live_rl_limit;
                }
                if live_rl_reset.is_some() {
                    slot.rate_limit_reset_at = live_rl_reset;
                }
            }
            (Some(live), None) => {
                slot.rate_limit_remaining = Some(live);
                slot.rate_limit_limit = live_rl_limit.or(slot.rate_limit_limit);
                slot.rate_limit_reset_at = live_rl_reset.or(slot.rate_limit_reset_at);
            }
            _ => {}
        }
        // Prefer newer success / failure markers from the hot path.
        match (live_success, slot.last_success_at) {
            (Some(l), Some(i)) if l > i => slot.last_success_at = Some(l),
            (Some(l), None) => slot.last_success_at = Some(l),
            _ => {}
        }
        match (live_failure, slot.last_failure_at) {
            (Some(l), Some(i)) if l > i => {
                slot.last_failure_at = Some(l);
                slot.consecutive_failures = live_failures;
                slot.last_upstream_error = live_upstream_err.or(slot.last_upstream_error.clone());
                slot.health = live_health;
                slot.cooldown_until = live_cooldown;
            }
            (Some(l), None) => {
                slot.last_failure_at = Some(l);
                slot.consecutive_failures = live_failures.max(slot.consecutive_failures);
                if slot.last_upstream_error.is_none() {
                    slot.last_upstream_error = live_upstream_err;
                }
            }
            _ => {}
        }

        save_auth_unlocked(&store)?;
        Ok(true)
    } else {
        tracing::info!(
            account_id = %account.id,
            "skip account update — id no longer in auth store (deleted)"
        );
        Ok(false)
    }
}

/// Append new accounts to the live store (used by import after async convert).
pub fn append_accounts(new_accounts: Vec<Account>) -> AppResult<Vec<Account>> {
    if new_accounts.is_empty() {
        return Ok(load_auth()?.accounts);
    }
    let _guard = AUTH_LOCK.lock();
    let mut store = load_auth_unlocked()?;
    store.accounts.extend(new_accounts);
    save_auth_unlocked(&store)?;
    Ok(store.accounts)
}

/// Remove accounts by id and **verify** the write hit disk.
pub fn delete_accounts_persistent(account_ids: &[String]) -> AppResult<usize> {
    if account_ids.is_empty() {
        return Ok(0);
    }
    let id_set: std::collections::HashSet<String> = account_ids
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if id_set.is_empty() {
        return Ok(0);
    }

    let _guard = AUTH_LOCK.lock();
    let mut store = load_auth_unlocked()?;
    let before = store.accounts.len();
    store.accounts.retain(|a| !id_set.contains(a.id.trim()));
    let removed = before.saturating_sub(store.accounts.len());
    if removed == 0 {
        return Ok(0);
    }
    save_auth_unlocked(&store)?;

    // Drop cache and re-read from disk to prove persistence (catches write failures).
    *AUTH_CACHE.write() = None;
    let verified = load_auth_unlocked()?;
    let still_present: Vec<&str> = verified
        .accounts
        .iter()
        .filter(|a| id_set.contains(a.id.trim()))
        .map(|a| a.id.as_str())
        .collect();
    if !still_present.is_empty() {
        return Err(AppError::msg(format!(
            "delete did not persist: {} id(s) still on disk",
            still_present.len()
        )));
    }
    tracing::info!(
        removed,
        remaining = verified.accounts.len(),
        path = %auth_path()?.display(),
        "accounts deleted and verified on disk"
    );
    Ok(removed)
}

/// Load a JSON config file, recovering from common Windows first-run failures:
/// empty file, whitespace-only, UTF-8 BOM, or corrupt JSON.
///
/// Returns `Ok(None)` when the file is missing or was reset to defaults (caller
/// should write a fresh default). Returns `Ok(Some(T))` when parse succeeds.
fn load_json_file<T: for<'de> Deserialize<'de>>(
    path: &Path,
    label: &str,
) -> AppResult<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    // Strip BOM before trim: Notepad "UTF-8 with BOM" is common on Windows.
    let trimmed = strip_utf8_bom(&raw).trim();
    if trimmed.is_empty() {
        tracing::warn!(
            "{label} file is empty at {}; recreating defaults",
            path.display()
        );
        backup_corrupt_file(path, label, "empty");
        return Ok(None);
    }
    match serde_json::from_str::<T>(trimmed) {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            tracing::error!(
                "{label} file is invalid JSON at {} ({err}); backing up and recreating defaults",
                path.display()
            );
            backup_corrupt_file(path, label, "invalid-json");
            Ok(None)
        }
    }
}

fn strip_utf8_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// Move a bad file aside under `~/.grok-go/backups/` so users can recover tokens.
fn backup_corrupt_file(path: &Path, label: &str, reason: &str) {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let backups = parent.join("backups");
    if let Err(err) = fs::create_dir_all(&backups) {
        tracing::warn!("failed to create backups dir {}: {err}", backups.display());
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(label);
    let dest = if backups.is_dir() {
        backups.join(format!("{file_name}.{reason}.{ts}.bak"))
    } else {
        path.with_extension(format!("{reason}.{ts}.bak"))
    };
    match fs::rename(path, &dest) {
        Ok(()) => tracing::warn!("backed up corrupt {label} to {}", dest.display()),
        Err(err) => {
            // Windows can fail rename if the file is locked; try copy+remove.
            tracing::warn!(
                "rename backup failed for {label} ({}): {err}; trying copy",
                path.display()
            );
            if let Err(copy_err) = fs::copy(path, &dest) {
                tracing::error!("copy backup also failed for {label}: {copy_err}");
            } else {
                let _ = fs::remove_file(path);
                tracing::warn!("backed up corrupt {label} via copy to {}", dest.display());
            }
        }
    }
}

/// Atomic-ish JSON write: temp file in the same directory then rename.
/// Avoids truncated config/auth files if the process crashes mid-write
/// (empty file → classic `expected value at line 1 column 1` on next launch).
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value)?;
    let tmp = atomic_tmp_path(path);
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    // Prefer rename. On Windows, rename cannot replace an existing file — remove first.
    // Keep the temp path in errors so a failed finalize does not silently lose data.
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) if path.exists() => {
            fs::remove_file(path)?;
            fs::rename(&tmp, path).map_err(|rename_err| {
                crate::error::AppError::msg(format!(
                    "failed to finalize {} (temp left at {}): {rename_err} (earlier: {err})",
                    path.display(),
                    tmp.display()
                ))
            })
        }
        Err(err) => Err(crate::error::AppError::msg(format!(
            "failed to finalize {} (temp left at {}): {err}",
            path.display(),
            tmp.display()
        ))),
    }
}

fn atomic_tmp_path(path: &Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file.json");
    path.with_file_name(format!(".{file_name}.{ts}.tmp"))
}

/// Update an account in the in-memory cache only (no disk I/O).
/// Use on the hot success path so every request does not rewrite auth.json.
pub fn patch_account_cache(account: &Account) {
    let mut guard = AUTH_CACHE.write();
    if let Some(store) = guard.as_mut() {
        if let Some(slot) = store.accounts.iter_mut().find(|a| a.id == account.id) {
            *slot = account.clone();
        }
    }
}

pub fn random_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(40)
        .map(char::from)
        .collect()
}

/// Known xAI **text** model ids (from `GET /v1/models`). Media models are separate.
/// Kept in sync with public xAI catalog; curated `/v1/models` list uses this.
pub fn known_xai_text_models() -> &'static [&'static str] {
    &[
        "grok-4.5",
        "grok-4.3",
        "grok-4.20-0309-reasoning",
        "grok-4.20-0309-non-reasoning",
        "grok-4.20-multi-agent-0309",
        "grok-build-0.1",
    ]
}

/// Models written into the CC Switch / Codex provider import catalog.
///
/// Only ids that are usable day-to-day with GrokGo + Codex (picker + depth).
/// Other xAI text ids may still pass through the gateway, but are not offered
/// in the imported provider model list.
pub fn cc_switch_import_models() -> &'static [&'static str] {
    &["grok-4.5", "grok-4.3"]
}

/// Default model id for CC Switch import when the app default is not in
/// [`cc_switch_import_models`].
pub fn cc_switch_import_default_model(app_default: &str) -> &'static str {
    let trimmed = app_default.trim();
    cc_switch_import_models()
        .iter()
        .find(|id| id.eq_ignore_ascii_case(trimmed))
        .copied()
        .unwrap_or("grok-4.5")
}

/// Effort levels accepted by xAI `reasoning.effort` for a text model (live-probed).
///
/// - `grok-4.5` / multi-agent: `low` | `medium` | `high` (no `none`, no `xhigh`)
/// - `grok-4.3`: also accepts `none`
/// - `grok-4.20-*-reasoning/non-reasoning` and `grok-build-0.1`: reject effort
///
/// Used by CC Switch import (`model_reasoning_effort` + catalog
/// `supported_reasoning_levels`) so Codex can show depth controls only where
/// the upstream API actually accepts them.
pub fn xai_model_reasoning_efforts(model_id: &str) -> Option<&'static [&'static str]> {
    let id = model_id.trim();
    if id.is_empty() {
        return None;
    }
    // Exact first, then case-insensitive match against known catalog.
    let canon = known_xai_text_models()
        .iter()
        .find(|m| m.eq_ignore_ascii_case(id))
        .copied()
        .unwrap_or(id);
    match canon {
        "grok-4.5" | "grok-4.20-multi-agent-0309" => Some(&["low", "medium", "high"]),
        "grok-4.3" => Some(&["none", "low", "medium", "high"]),
        _ => None,
    }
}

/// Default Codex `model_reasoning_effort` when the model supports depth control.
pub fn xai_model_default_reasoning_effort(model_id: &str) -> Option<&'static str> {
    xai_model_reasoning_efforts(model_id).map(|_| "medium")
}

/// True for image/video Imagine model ids (not used as Codex primary chat model).
pub fn is_xai_media_model_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    lower.contains("imagine")
        || lower.contains("image")
        || lower.contains("video")
        || lower.contains("tts")
        || lower.contains("voice")
}

pub fn resolve_model(config: &AppConfig, requested: &str) -> (String, String) {
    let trimmed = requested.trim();
    if trimmed.is_empty() {
        return (config.default_model.clone(), "default-fallback".into());
    }
    if let Some(mapped) = config.model_mappings.get(trimmed) {
        return (mapped.clone(), "mapped".into());
    }
    // Case-insensitive map lookup.
    let lower = trimmed.to_lowercase();
    for (k, v) in &config.model_mappings {
        if k.to_lowercase() == lower {
            return (v.clone(), "mapped".into());
        }
    }
    let is_media = is_xai_media_model_id(trimmed);
    let looks_like_grok = lower.starts_with("grok-") || lower.starts_with("grok");
    // Known text catalog ids pass through as-is.
    if known_xai_text_models()
        .iter()
        .any(|id| id.eq_ignore_ascii_case(trimmed))
    {
        // Preserve canonical casing from catalog when possible.
        let canon = known_xai_text_models()
            .iter()
            .find(|id| id.eq_ignore_ascii_case(trimmed))
            .copied()
            .unwrap_or(trimmed);
        return (canon.to_string(), "passthrough".into());
    }
    if looks_like_grok && !is_media {
        return (trimmed.to_string(), "passthrough".into());
    }
    (config.default_model.clone(), "default-fallback".into())
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("grok-go-config-{label}-{ts}"));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn strip_bom_and_whitespace() {
        assert_eq!(strip_utf8_bom("\u{feff}{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_utf8_bom("plain"), "plain");
    }

    #[test]
    fn reasoning_efforts_only_for_probed_models() {
        assert_eq!(
            xai_model_reasoning_efforts("grok-4.5"),
            Some(&["low", "medium", "high"][..])
        );
        assert_eq!(
            xai_model_reasoning_efforts("GROK-4.3"),
            Some(&["none", "low", "medium", "high"][..])
        );
        assert_eq!(
            xai_model_reasoning_efforts("grok-4.20-multi-agent-0309"),
            Some(&["low", "medium", "high"][..])
        );
        assert!(xai_model_reasoning_efforts("grok-4.20-0309-reasoning").is_none());
        assert!(xai_model_reasoning_efforts("grok-4.20-0309-non-reasoning").is_none());
        assert!(xai_model_reasoning_efforts("grok-build-0.1").is_none());
        assert_eq!(xai_model_default_reasoning_effort("grok-4.5"), Some("medium"));
        assert!(xai_model_default_reasoning_effort("grok-build-0.1").is_none());
    }

    #[test]
    fn resolve_passes_known_text_models_and_maps_gpt() {
        let cfg = AppConfig::default();
        let (m, reason) = resolve_model(&cfg, "grok-4.3");
        assert_eq!(m, "grok-4.3");
        assert_eq!(reason, "passthrough");
        let (m2, r2) = resolve_model(&cfg, "grok-4.20-0309-reasoning");
        assert_eq!(m2, "grok-4.20-0309-reasoning");
        assert_eq!(r2, "passthrough");
        // Unknown Cursor-style names fall back to default Grok (not a real xAI id).
        let (m3, r3) = resolve_model(&cfg, "Composer 2.5");
        assert_eq!(m3, "grok-4.5");
        assert_eq!(r3, "default-fallback");
        // Explicit gpt → grok aliases still map.
        let (m4, r4) = resolve_model(&cfg, "gpt-5.5");
        assert_eq!(m4, "grok-4.5");
        assert_eq!(r4, "mapped");
    }

    #[test]
    fn load_json_empty_file_returns_none() {
        let dir = temp_dir("empty");
        let path = dir.join("auth.json");
        fs::write(&path, "").expect("write empty");
        let result = load_json_file::<AuthStore>(&path, "auth").expect("load");
        assert!(result.is_none());
        // Empty file should have been backed up / moved aside.
        assert!(!path.exists());
        let backups = dir.join("backups");
        assert!(backups.is_dir());
        assert!(fs::read_dir(&backups).unwrap().count() >= 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_json_bom_prefixed_ok() {
        let dir = temp_dir("bom");
        let path = dir.join("auth.json");
        let body = "\u{feff}{\n  \"accounts\": []\n}\n";
        fs::write(&path, body).expect("write bom");
        let result = load_json_file::<AuthStore>(&path, "auth").expect("load");
        assert!(result.is_some());
        assert!(result.unwrap().accounts.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_json_invalid_backs_up_and_returns_none() {
        let dir = temp_dir("bad");
        let path = dir.join("config.json");
        fs::write(&path, "not-json-at-all").expect("write bad");
        let result = load_json_file::<AppConfig>(&path, "config").expect("load");
        assert!(result.is_none());
        assert!(!path.exists());
        let backups = dir.join("backups");
        assert!(backups.is_dir());
        let count = fs::read_dir(&backups).unwrap().count();
        assert!(count >= 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn write_json_atomic_roundtrip() {
        let dir = temp_dir("atomic");
        let path = dir.join("config.json");
        let cfg = AppConfig::default();
        write_json_atomic(&path, &cfg).expect("write");
        let loaded: AppConfig = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.preferred_port, cfg.preferred_port);
        // Overwrite existing (Windows rename path).
        let mut cfg2 = cfg.clone();
        cfg2.preferred_port = 9999;
        write_json_atomic(&path, &cfg2).expect("overwrite");
        let loaded2: AppConfig = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded2.preferred_port, 9999);
        let _ = fs::remove_dir_all(dir);
    }
}
