use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
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
}

impl Default for RoutingStrategy {
    fn default() -> Self {
        Self::WeightedRoundRobin
    }
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
        }
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
    if let Some(cached) = AUTH_CACHE.read().clone() {
        return Ok(cached);
    }
    let path = auth_path()?;
    let store = match load_json_file::<AuthStore>(&path, "auth")? {
        Some(store) => store,
        None => {
            let store = AuthStore::default();
            save_auth(&store)?;
            return Ok(store);
        }
    };
    *AUTH_CACHE.write() = Some(store.clone());
    Ok(store)
}

pub fn save_auth(store: &AuthStore) -> AppResult<()> {
    let path = auth_path()?;
    write_json_atomic(&path, store)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    *AUTH_CACHE.write() = Some(store.clone());
    Ok(())
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
