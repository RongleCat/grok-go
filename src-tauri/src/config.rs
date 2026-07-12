use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use uuid::Uuid;

use crate::error::AppResult;
use crate::paths::{auth_path, config_path};

/// In-memory config/auth to avoid re-reading JSON on every proxy request.
static CONFIG_CACHE: Lazy<RwLock<Option<AppConfig>>> = Lazy::new(|| RwLock::new(None));
static AUTH_CACHE: Lazy<RwLock<Option<AuthStore>>> = Lazy::new(|| RwLock::new(None));

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
    pub auto_inject_codex_mcp: bool,
    pub launch_on_startup: bool,
    pub minimize_to_tray: bool,
    pub xai_client_id: String,
    pub xai_base_url: String,
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
            auto_inject_codex_mcp: false,
            launch_on_startup: false,
            minimize_to_tray: true,
            // Public hermes/grok-cli client — redirect_uri must match the
            // allowlist registered with xAI (exact port + path).
            xai_client_id: "b1a00492-073a-47ea-816f-4c329264a828".into(),
            xai_base_url: "https://api.x.ai/v1".into(),
            oauth_redirect_port: 56121,
            http_proxy_enabled: false,
            http_proxy_url: String::new(),
            app_icon: AppIconStyle::Dark,
            mcp_enabled_tools: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthStore {
    #[serde(default)]
    pub accounts: Vec<Account>,
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
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
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
            access_token: None,
            refresh_token: None,
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
        }
    }
}

pub fn load_config() -> AppResult<AppConfig> {
    if let Some(cached) = CONFIG_CACHE.read().clone() {
        return Ok(cached);
    }
    let path = config_path()?;
    if !path.exists() {
        let cfg = AppConfig::default();
        save_config(&cfg)?;
        return Ok(cfg);
    }
    let raw = fs::read_to_string(path)?;
    let cfg: AppConfig = serde_json::from_str(&raw)?;
    *CONFIG_CACHE.write() = Some(cfg.clone());
    Ok(cfg)
}

pub fn save_config(config: &AppConfig) -> AppResult<()> {
    let path = config_path()?;
    fs::write(path, serde_json::to_string_pretty(config)?)?;
    *CONFIG_CACHE.write() = Some(config.clone());
    Ok(())
}

pub fn load_auth() -> AppResult<AuthStore> {
    if let Some(cached) = AUTH_CACHE.read().clone() {
        return Ok(cached);
    }
    let path = auth_path()?;
    if !path.exists() {
        let store = AuthStore::default();
        save_auth(&store)?;
        return Ok(store);
    }
    let raw = fs::read_to_string(path)?;
    let store: AuthStore = serde_json::from_str(&raw)?;
    *AUTH_CACHE.write() = Some(store.clone());
    Ok(store)
}

pub fn save_auth(store: &AuthStore) -> AppResult<()> {
    let path = auth_path()?;
    fs::write(&path, serde_json::to_string_pretty(store)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    *AUTH_CACHE.write() = Some(store.clone());
    Ok(())
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

pub fn resolve_model(config: &AppConfig, requested: &str) -> (String, String) {
    if let Some(mapped) = config.model_mappings.get(requested) {
        return (mapped.clone(), "mapped".into());
    }
    let lower = requested.to_lowercase();
    let is_media = lower.contains("image") || lower.contains("video") || lower.contains("imagine");
    let looks_like_grok = lower.starts_with("grok-") || lower.starts_with("grok");
    if looks_like_grok && !is_media {
        return (requested.to_string(), "passthrough".into());
    }
    (config.default_model.clone(), "default-fallback".into())
}
