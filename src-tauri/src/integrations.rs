use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use uuid::Uuid;

use crate::config::{load_config, save_config, AppConfig};
use crate::error::{AppError, AppResult};
use crate::paths::{
    agents_guide_file_path, app_home, cc_switch_db_path, codex_agents_md_path, codex_config_path,
    grok_build_auth_path, grok_build_config_path, grok_build_restore_dir,
};

/// Markers around the short reference line in Codex `AGENTS.md`.
/// Detection of "injected" is presence of this fixed start marker (or legacy ones).
const AGENTS_GUIDE_START: &str = "<!-- grok-go:agents-guide:start -->";
const AGENTS_GUIDE_END: &str = "<!-- grok-go:agents-guide:end -->";
/// Legacy full-block markers (pre short-ref design / Grok Proxy rename) — still stripped.
const AGENTS_GUIDE_START_LEGACY: &[&str] = &[
    "<!-- grok-go:agents-guide:start -->",
    "<!-- grok-proxy:agents-guide:start -->",
];
const AGENTS_GUIDE_END_LEGACY: &[&str] = &[
    "<!-- grok-go:agents-guide:end -->",
    "<!-- grok-proxy:agents-guide:end -->",
];
/// Fixed phrase used both in AGENTS.md and for runtime detection.
const AGENTS_GUIDE_REF_ANCHOR: &str = "grok-go:agents-guide-ref";

/// Canonical MCP server id written into Codex config.
const MCP_SERVER_ID: &str = "grok-go";
/// Legacy id from the Grok Proxy branding — still treated as injected when present.
const MCP_SERVER_ID_LEGACY: &str = "grok-proxy";
const MCP_SERVER_IDS: &[&str] = &[MCP_SERVER_ID, MCP_SERVER_ID_LEGACY];

/// Legacy mistaken inject key (single custom model) — cleaned up on inject/remove.
const GROK_BUILD_LEGACY_MODEL_KEY: &str = "grok-go";
/// Grok Build native inference endpoint key (SuperGrok / session plane).
const GROK_BUILD_CLI_CHAT_PROXY_KEY: &str = "cli_chat_proxy_base_url";
/// Expensive API-key mode key — never use for multi-account routing inject.
const GROK_BUILD_MODELS_BASE_URL_KEY: &str = "models_base_url";
const GROK_BUILD_RESTORE_CONFIG: &str = "config.toml";
const GROK_BUILD_RESTORE_AUTH: &str = "auth.json";
const GROK_BUILD_RESTORE_META: &str = "meta.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    pub codex_mcp_injected: bool,
    pub codex_config_path: String,
    pub codex_agents_injected: bool,
    /// Path to Codex global `AGENTS.md` (holds only a short reference line when injected).
    pub codex_agents_path: String,
    /// Absolute path to the versioned guide file under `~/.grok-go/agents-guide.md`.
    pub agents_guide_file_path: String,
    pub cc_switch_db_path: String,
    /// Whether Grok Build standard session routing points at this gateway.
    pub grok_build_injected: bool,
    pub grok_build_config_path: String,
    /// Path to Grok Build `auth.json` (session login; may be synced from pool).
    pub grok_build_auth_path: String,
    /// One-click restore snapshot is available under backups.
    pub grok_build_restore_available: bool,
    pub grok_build_restore_path: String,
    /// Enabled OAuth accounts that can serve Grok Build multi-route.
    pub grok_build_account_count: usize,
    /// Protocol label for UI (cli-chat-proxy / SuperGrok session).
    pub grok_build_protocol: String,
    /// Email of the session currently in ~/.grok/auth.json.
    pub grok_build_session_email: Option<String>,
    /// JWT `tier` claim currently present in ~/.grok/auth.json (if any).
    pub grok_build_session_tier: Option<i64>,
    /// JWT `referrer` claim (grok-build preferred; sub2api often fails TUI gate).
    pub grok_build_session_referrer: Option<String>,
    /// Warning when synced session is unlikely to pass Grok Build paywall.
    pub grok_build_session_warn: Option<String>,
    pub provider_snippet: String,
    pub mcp_snippet: String,
    /// Ready-to-paste standard session routing block.
    pub grok_build_snippet: String,
    /// Claude Code / CC Switch env JSON (ANTHROPIC_BASE_URL without `/v1`).
    pub claude_code_snippet: String,
}

pub fn integration_status() -> AppResult<IntegrationStatus> {
    let config = load_config()?;
    let codex_path = codex_config_path();
    let injected = if codex_path.exists() {
        let raw = fs::read_to_string(&codex_path).unwrap_or_default();
        codex_mcp_is_injected(&raw)
    } else {
        false
    };
    let agents_path = codex_agents_md_path();
    let agents_injected = agents_guide_ref_present_at(&agents_path);
    // Keep the on-disk guide in sync with this binary whenever the user has a ref.
    if agents_injected {
        let _ = ensure_agents_guide_file();
    }
    let guide_file = agents_guide_file_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let grok_path = grok_build_config_path();
    let grok_auth = grok_build_auth_path();
    let grok_injected = if grok_path.exists() {
        let raw = fs::read_to_string(&grok_path).unwrap_or_default();
        grok_build_is_injected(&raw, &config)
    } else {
        false
    };
    let restore_dir = grok_build_restore_dir().unwrap_or_else(|_| {
        std::path::PathBuf::from(".grok-go/backups/grok-build-pre-route")
    });
    let restore_available = restore_dir.join(GROK_BUILD_RESTORE_CONFIG).is_file()
        || restore_dir.join(GROK_BUILD_RESTORE_AUTH).is_file();
    let account_count = crate::config::load_auth()
        .map(|s| {
            s.accounts
                .iter()
                .filter(|a| a.enabled && a.is_credentialed())
                .count()
        })
        .unwrap_or(0);
    let session = read_grok_build_session_info();
    Ok(IntegrationStatus {
        codex_mcp_injected: injected,
        codex_config_path: codex_path.display().to_string(),
        codex_agents_injected: agents_injected,
        codex_agents_path: agents_path.display().to_string(),
        agents_guide_file_path: guide_file,
        cc_switch_db_path: cc_switch_db_path().display().to_string(),
        grok_build_injected: grok_injected,
        grok_build_config_path: grok_path.display().to_string(),
        grok_build_auth_path: grok_auth.display().to_string(),
        grok_build_restore_available: restore_available,
        grok_build_restore_path: restore_dir.display().to_string(),
        grok_build_account_count: account_count,
        grok_build_protocol: "cli-chat-proxy (SuperGrok / session)".into(),
        grok_build_session_email: session.as_ref().and_then(|s| s.email.clone()),
        grok_build_session_tier: session.as_ref().and_then(|s| s.tier),
        grok_build_session_referrer: session.as_ref().and_then(|s| s.referrer.clone()),
        grok_build_session_warn: session.as_ref().and_then(|s| s.warn.clone()),
        provider_snippet: provider_snippet(&config),
        mcp_snippet: mcp_snippet(&config),
        grok_build_snippet: grok_build_snippet(&config),
        claude_code_snippet: claude_code_settings_snippet(&config),
    })
}

/// Runtime check: parse `~/.codex/config.toml` and see if a known GrokGo / legacy
/// `mcp_servers.<id>` entry exists with a usable `url` (…/mcp).
///
/// Accepts both `[mcp_servers.grok-go]` (current) and `[mcp_servers.grok-proxy]` (legacy).
fn codex_mcp_is_injected(raw: &str) -> bool {
    match raw.parse::<toml_edit::DocumentMut>() {
        Ok(doc) => {
            let Some(servers) = doc.get("mcp_servers").and_then(|i| i.as_table()) else {
                return false;
            };
            for id in MCP_SERVER_IDS {
                if let Some(entry) = servers.get(id).and_then(|i| i.as_table()) {
                    if mcp_entry_is_active(entry) {
                        return true;
                    }
                }
            }
            false
        }
        // Fallback for partially invalid TOML: section headers only.
        Err(_) => MCP_SERVER_IDS
            .iter()
            .any(|id| raw.contains(&format!("[mcp_servers.{id}]"))),
    }
}

fn mcp_entry_is_active(entry: &toml_edit::Table) -> bool {
    match entry.get("url").and_then(|v| v.as_str()) {
        Some(url) => {
            let u = url.trim();
            !u.is_empty() && (u.contains("/mcp") || u.ends_with("mcp"))
        }
        // Table present without url — treat as not actively injected.
        None => false,
    }
}

pub fn set_codex_mcp_inject(enabled: bool) -> AppResult<IntegrationStatus> {
    let mut config = load_config()?;
    config.auto_inject_codex_mcp = enabled;
    save_config(&config)?;
    if enabled {
        inject_codex_mcp(&config)?;
        // Keep runtime guide in sync with currently enabled tools.
        let _ = ensure_agents_guide_file_with(&config);
        // If user already had agents ref, refresh it; otherwise leave AGENTS.md alone
        // until they explicitly inject the guide.
        if agents_guide_ref_present_at(&codex_agents_md_path()) {
            let _ = write_codex_agents_guide_ref();
        }
    } else {
        remove_codex_mcp()?;
        // MCP uninject also strips the managed AGENTS.md guide block.
        remove_codex_agents_guide()?;
    }
    integration_status()
}

/// Point Grok Build's **standard cli-chat-proxy session plane** at this gateway.
///
/// Official SuperGrok path (not Custom Models / API-key mode):
/// - `~/.grok/config.toml` → `[endpoints] cli_chat_proxy_base_url = "http://127.0.0.1:PORT/v1"`
/// - Sync a pool OAuth session into `~/.grok/auth.json` so the TUI subscription gate can open
/// - Strip accidental `models_base_url` pointing at us (legacy API-key inject)
///
/// GrokGo accepts the client Bearer, replaces it with a pool token, and forwards to
/// `cli-chat-proxy.grok.com` with session affinity.
pub fn set_grok_build_inject(enabled: bool) -> AppResult<IntegrationStatus> {
    let config = load_config()?;
    if enabled {
        inject_grok_build_routing(&config)?;
    } else {
        remove_grok_build_routing(&config)?;
    }
    integration_status()
}

/// Restore `~/.grok/config.toml` + `auth.json` from the pre-inject snapshot.
pub fn restore_grok_build_backup() -> AppResult<IntegrationStatus> {
    let restore_dir = grok_build_restore_dir()?;
    let cfg_src = restore_dir.join(GROK_BUILD_RESTORE_CONFIG);
    let auth_src = restore_dir.join(GROK_BUILD_RESTORE_AUTH);
    if !cfg_src.is_file() && !auth_src.is_file() {
        return Err(AppError::msg(
            "没有可还原的 Grok Build 备份。请先在集成页开启多账号路由（开启前会自动备份）。",
        ));
    }

    let home = crate::paths::grok_build_home();
    fs::create_dir_all(&home)?;

    // Safety: timestamped copy of current live files before overwrite.
    let stamp = Utc::now().format("%Y%m%d-%H%M%S");
    let live_cfg = grok_build_config_path();
    let live_auth = grok_build_auth_path();
    if live_cfg.exists() {
        let _ = backup_file_to(
            &live_cfg,
            &app_home()?
                .join("backups")
                .join(format!("grok-config-before-restore-{stamp}.toml")),
        );
    }
    if live_auth.exists() {
        let _ = backup_file_to(
            &live_auth,
            &app_home()?
                .join("backups")
                .join(format!("grok-auth-before-restore-{stamp}.json")),
        );
    }

    if cfg_src.is_file() {
        fs::copy(&cfg_src, &live_cfg).map_err(|e| {
            AppError::msg(format!("还原 config.toml 失败：{e}"))
        })?;
    }
    if auth_src.is_file() {
        fs::copy(&auth_src, &live_auth).map_err(|e| {
            AppError::msg(format!("还原 auth.json 失败：{e}"))
        })?;
    }

    tracing::info!(
        target: "integrations",
        restore = %restore_dir.display(),
        "restored Grok Build config/auth from pre-route snapshot"
    );
    integration_status()
}

fn gateway_base_url(config: &AppConfig) -> String {
    let host = if config.lan_enabled {
        local_lan_host()
    } else {
        "127.0.0.1".into()
    };
    format!("http://{}:{}/v1", host, config.actual_port)
}

/// Claude Code `ANTHROPIC_BASE_URL` — host root **without** `/v1`
/// (client appends `/v1/messages`).
fn anthropic_base_url(config: &AppConfig) -> String {
    let host = if config.lan_enabled {
        local_lan_host()
    } else {
        "127.0.0.1".into()
    };
    format!("http://{}:{}", host, config.actual_port)
}

/// Snippet for copy/paste: standard cli-chat-proxy session routing.
fn grok_build_snippet(config: &AppConfig) -> String {
    let base = gateway_base_url(config);
    format!(
        r#"# ~/.grok/config.toml  — Grok Build 标准协议（SuperGrok / cli-chat-proxy）
# 开启集成时 GrokGo 会：
# 1) 写入 cli_chat_proxy_base_url → 本机网关
# 2) 用账号池较优 OAuth 会话同步到 ~/.grok/auth.json（客户端订阅门闸）
# 勿设置 models_base_url（那是 console API / Custom Models 路径）。
[endpoints]
{cli_key} = "{base}"

export GROK_CLI_CHAT_PROXY_BASE_URL="{base}"
# 然后重启 `grok`。推理选号 / failover 由 GrokGo 网关负责。
"#,
        cli_key = GROK_BUILD_CLI_CHAT_PROXY_KEY,
        base = base,
    )
}

fn url_points_at_gateway(url: &str, config: &AppConfig) -> bool {
    let u = url.trim().trim_end_matches('/');
    let expected = gateway_base_url(config).trim_end_matches('/').to_string();
    if u == expected {
        return true;
    }
    let port = config.actual_port;
    (u.contains(&format!("127.0.0.1:{port}"))
        || u.contains(&format!("localhost:{port}"))
        || u.contains(&format!("[::1]:{port}")))
        && (u.ends_with("/v1") || u.contains("/v1/") || u.ends_with(&format!(":{port}/v1")))
}

/// True when `[endpoints].cli_chat_proxy_base_url` points at this GrokGo instance.
fn grok_build_is_injected(raw: &str, config: &AppConfig) -> bool {
    let Ok(doc) = raw.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    let Some(endpoints) = doc.get("endpoints").and_then(|i| i.as_table()) else {
        return false;
    };
    let Some(base) = endpoints
        .get(GROK_BUILD_CLI_CHAT_PROXY_KEY)
        .and_then(|v| v.as_str())
    else {
        return false;
    };
    url_points_at_gateway(base, config)
}

fn inject_grok_build_routing(config: &AppConfig) -> AppResult<()> {
    let path = grok_build_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let original = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };

    // Snapshot login + config before first inject only.
    let already = grok_build_is_injected(&original, config);
    if !already {
        snapshot_grok_build_for_restore(&original)?;
    }
    backup_grok_build_config(&original)?;

    // Sync pool OAuth into ~/.grok/auth.json for TUI paywall / subscription gate.
    sync_grok_build_session_auth(config)?;

    let mut doc = if original.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        original
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| AppError::msg(format!("invalid ~/.grok/config.toml: {e}")))?
    };

    let endpoints = doc
        .entry("endpoints")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let table = endpoints
        .as_table_mut()
        .ok_or_else(|| AppError::msg("endpoints is not a table in ~/.grok/config.toml"))?;

    table[GROK_BUILD_CLI_CHAT_PROXY_KEY] = toml_edit::value(gateway_base_url(config));

    // Clean accidental API-key endpoint pointing at us (legacy mistake).
    if let Some(models_url) = table
        .get(GROK_BUILD_MODELS_BASE_URL_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
    {
        if url_points_at_gateway(&models_url, config) {
            table.remove(GROK_BUILD_MODELS_BASE_URL_KEY);
        }
    }

    if let Some(models) = doc.get_mut("model").and_then(|i| i.as_table_mut()) {
        models.remove(GROK_BUILD_LEGACY_MODEL_KEY);
        if models.is_empty() {
            doc.remove("model");
        }
    }

    fs::write(&path, doc.to_string())?;
    tracing::info!(
        target: "integrations",
        path = %path.display(),
        base = %gateway_base_url(config),
        "injected Grok Build standard cli-chat-proxy routing"
    );
    Ok(())
}

fn remove_grok_build_routing(config: &AppConfig) -> AppResult<()> {
    let path = grok_build_config_path();
    if !path.exists() {
        return Ok(());
    }
    let original = fs::read_to_string(&path)?;
    backup_grok_build_config(&original)?;
    let mut doc = original
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| AppError::msg(format!("invalid ~/.grok/config.toml: {e}")))?;

    if let Some(endpoints) = doc.get_mut("endpoints").and_then(|i| i.as_table_mut()) {
        let ours_cli = endpoints
            .get(GROK_BUILD_CLI_CHAT_PROXY_KEY)
            .and_then(|v| v.as_str())
            .map(|u| url_points_at_gateway(u, config))
            .unwrap_or(false);
        if ours_cli {
            endpoints.remove(GROK_BUILD_CLI_CHAT_PROXY_KEY);
        }
        let ours_models = endpoints
            .get(GROK_BUILD_MODELS_BASE_URL_KEY)
            .and_then(|v| v.as_str())
            .map(|u| url_points_at_gateway(u, config))
            .unwrap_or(false);
        if ours_models {
            endpoints.remove(GROK_BUILD_MODELS_BASE_URL_KEY);
        }
        if endpoints.is_empty() {
            doc.remove("endpoints");
        }
    }

    if let Some(models) = doc.get_mut("model").and_then(|i| i.as_table_mut()) {
        models.remove(GROK_BUILD_LEGACY_MODEL_KEY);
        if models.is_empty() {
            doc.remove("model");
        }
    }

    fs::write(path, doc.to_string())?;
    Ok(())
}

#[derive(Debug, Clone)]
struct GrokBuildSessionInfo {
    email: Option<String>,
    tier: Option<i64>,
    /// JWT `referrer` claim (e.g. grok-build / sub2api). GrowthBook gate cares about this.
    referrer: Option<String>,
    /// Human-readable risk note when session is unlikely to pass TUI paywall.
    warn: Option<String>,
}

/// Read Grok Build `auth.json` session email + JWT tier/referrer (no verify).
fn read_grok_build_session_info() -> Option<GrokBuildSessionInfo> {
    let path = grok_build_auth_path();
    let raw = fs::read_to_string(path).ok()?;
    let map: serde_json::Map<String, Value> = serde_json::from_str(&raw).ok()?;
    // Prefer official OIDC client entry shape: { "https://auth.x.ai::<client>": { key, email, ... } }
    for (_k, v) in map.iter() {
        let Some(obj) = v.as_object() else { continue };
        let token = obj
            .get("key")
            .or_else(|| obj.get("access_token"))
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty());
        let email = obj
            .get("email")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let payload = token.and_then(crate::auth::jwt_payload);
        let tier = payload
            .as_ref()
            .and_then(|p| p.get("tier").and_then(|x| x.as_i64()));
        let referrer = payload
            .as_ref()
            .and_then(|p| p.get("referrer").and_then(|x| x.as_str()))
            .map(|s| s.to_string());
        let warn = session_gate_warning(tier, referrer.as_deref(), payload.as_ref());
        if email.is_some() || tier.is_some() || token.is_some() {
            return Some(GrokBuildSessionInfo {
                email,
                tier,
                referrer,
                warn,
            });
        }
    }
    None
}

fn session_gate_warning(
    tier: Option<i64>,
    referrer: Option<&str>,
    payload: Option<&Value>,
) -> Option<String> {
    let ref_l = referrer.unwrap_or("").to_ascii_lowercase();
    let scope = payload
        .and_then(|p| p.get("scope").and_then(|v| v.as_str()))
        .unwrap_or("");
    if !ref_l.is_empty() && ref_l != "grok-build" {
        return Some(format!(
            "当前会话 JWT referrer={referrer:?}（非 grok-build）。Grok Build TUI 的 GrowthBook 门闸可能仍拦截（显示 subscription required）。请用 GrokGo 对该账号重新 OAuth 登录（referrer=grok-build）后再开启集成。"
        ));
    }
    if !scope.contains("grok-cli:access") {
        return Some(
            "当前会话缺少 grok-cli:access 权限范围，Grok Build 可能无法正常鉴权。".into(),
        );
    }
    if tier.unwrap_or(0) < 2 {
        return Some(
            "当前会话 JWT tier 偏低，可能仍被订阅门闸拦截。请换 SuperGrok 账号并确保是 grok-build 登录面。".into(),
        );
    }
    None
}

/// Score pool accounts for Grok Build session login (client paywall / GrowthBook).
///
/// Order of importance (observed from Grok Build 0.2.x TUI):
/// 1. JWT `referrer=grok-build` — sub2api/CPA imports often keep referrer=sub2api and fail gate
///    even when `tier` looks like SuperGrok / x_premium_plus
/// 2. `grok-cli:access` + conversations scopes (official Grok Build OAuth surface)
/// 3. Higher JWT `tier` / remaining SuperGrok quota
fn score_account_for_grok_build_session(account: &crate::config::Account) -> i64 {
    if !account.enabled || !account.is_credentialed() {
        return i64::MIN / 4;
    }
    if matches!(account.auth_kind, crate::config::AccountAuthKind::Sso) {
        return i64::MIN / 4;
    }
    let mut score: i64 = 0;
    if let Some(token) = account.access_token.as_deref() {
        if let Some(payload) = crate::auth::jwt_payload(token) {
            let tier = payload.get("tier").and_then(|v| v.as_i64()).unwrap_or(0);
            score += tier.saturating_mul(100_000);

            let referrer = payload
                .get("referrer")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if referrer == "grok-build" {
                // Hard preference: TUI GrowthBook gate is calibrated for this surface.
                score += 50_000_000;
            } else if referrer.is_empty() {
                score += 1_000_000;
            } else if referrer.contains("sub2api")
                || referrer.contains("cpa")
                || referrer.contains("card")
            {
                // High tier from import pipelines often still gets allow_access=false.
                score -= 40_000_000;
            } else {
                score -= 10_000_000;
            }

            if let Some(scope) = payload.get("scope").and_then(|v| v.as_str()) {
                if scope.contains("grok-cli:access") {
                    score += 500_000;
                }
                if scope.contains("conversations:write") {
                    score += 200_000;
                }
                if scope.contains("conversations:read") {
                    score += 100_000;
                }
            }
        }
    } else if account.refresh_token.is_some() {
        // Refreshable but no access yet — still usable after ensure_fresh.
        score += 1_000;
    }
    if let Some(q) = account.quota.as_ref() {
        score += (q.remaining_percent.clamp(0.0, 100.0) * 100.0) as i64;
    }
    // Prefer tokens that are not already expired in local metadata.
    if let Some(exp) = account.expires_at {
        if exp > Utc::now() {
            score += 5_000;
        } else {
            score -= 2_000;
        }
    }
    if matches!(account.health, crate::config::AccountHealth::Healthy) {
        score += 1_000;
    }
    score
}

fn pick_best_account_for_grok_build_session(
    store: &crate::config::AuthStore,
) -> Option<crate::config::Account> {
    store
        .accounts
        .iter()
        .filter(|a| a.enabled && a.is_credentialed())
        .filter(|a| !matches!(a.auth_kind, crate::config::AccountAuthKind::Sso))
        .max_by_key(|a| score_account_for_grok_build_session(a))
        .cloned()
}

/// Write pool OAuth credentials into Grok Build `auth.json` session format so the
/// TUI subscription gate / check_subscription can pass for SuperGrok-capable accounts.
///
/// The pre-inject snapshot under backups/grok-build-pre-route keeps the user's original
/// `grok login` session for one-click restore.
fn sync_grok_build_session_auth(config: &AppConfig) -> AppResult<()> {
    let store = crate::config::load_auth()?;
    let Some(account) = pick_best_account_for_grok_build_session(&store) else {
        tracing::warn!(
            target: "integrations",
            "no credentialed OAuth account available to sync into ~/.grok/auth.json"
        );
        return Ok(());
    };

    // Refresh if expiring / missing access — keep pool store updated.
    let mut account = account;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::msg(format!("tokio runtime for token refresh: {e}")))?;
    match rt.block_on(crate::auth::ensure_fresh_token(config, &mut account)) {
        Ok(_) => {
            // Persist refreshed tokens back into GrokGo pool.
            if let Ok(mut pool) = crate::config::load_auth() {
                if let Some(slot) = pool.accounts.iter_mut().find(|a| a.id == account.id) {
                    *slot = account.clone();
                    let _ = crate::config::save_auth(&pool);
                }
            }
        }
        Err(err) => {
            // Still try writing existing access token if present.
            tracing::warn!(
                target: "integrations",
                account = %account.id,
                "refresh before Grok Build session sync failed: {err}"
            );
            if account.access_token.as_ref().map(|t| t.is_empty()).unwrap_or(true) {
                return Err(AppError::msg(format!(
                    "无法刷新账号 {} 的 token，Grok Build 会话同步失败：{err}",
                    account.email.clone().unwrap_or_else(|| account.name.clone())
                )));
            }
        }
    }

    let access = account
        .access_token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| AppError::msg("selected account has empty access token"))?;
    let refresh = account
        .refresh_token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| AppError::msg("selected account missing refresh token"))?;

    let payload = crate::auth::jwt_payload(&access).unwrap_or_else(|| json!({}));
    let client_id = config.effective_xai_client_id().to_string();
    let principal_id = payload
        .get("principal_id")
        .or_else(|| payload.get("sub"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let team_id = payload
        .get("team_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let email = account
        .email
        .clone()
        .or_else(|| {
            payload
                .get("email")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| account.name.clone());
    let expires_at = account
        .expires_at
        .map(|t| t.to_rfc3339())
        .or_else(|| {
            payload
                .get("exp")
                .and_then(|v| v.as_i64())
                .and_then(|secs| chrono::DateTime::<Utc>::from_timestamp(secs, 0))
                .map(|t| t.to_rfc3339())
        })
        .unwrap_or_else(|| (Utc::now() + chrono::Duration::hours(6)).to_rfc3339());
    let tier = payload.get("tier").and_then(|v| v.as_i64());

    let entry = json!({
        "key": access,
        "auth_mode": "oidc",
        "create_time": Utc::now().to_rfc3339(),
        "user_id": principal_id,
        "email": email,
        "principal_type": payload.get("principal_type").and_then(|v| v.as_str()).unwrap_or("User"),
        "principal_id": principal_id,
        "team_id": team_id,
        "coding_data_retention_opt_out": true,
        "refresh_token": refresh,
        "expires_at": expires_at,
        "oidc_issuer": "https://auth.x.ai",
        "oidc_client_id": client_id,
    });

    let key = format!("https://auth.x.ai::{client_id}");
    let mut root = serde_json::Map::new();
    root.insert(key, entry);
    let auth_path = grok_build_auth_path();
    if let Some(parent) = auth_path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Atomic-ish write
    let tmp = auth_path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&Value::Object(root))?)?;
    fs::rename(&tmp, &auth_path).map_err(|e| {
        // fallback copy
        let _ = fs::copy(&tmp, &auth_path);
        let _ = fs::remove_file(&tmp);
        if auth_path.exists() {
            Ok(())
        } else {
            Err(AppError::msg(format!("write ~/.grok/auth.json failed: {e}")))
        }
    }).or_else(|e| e)?;

    tracing::info!(
        target: "integrations",
        account = %account.id,
        email = %email,
        ?tier,
        path = %auth_path.display(),
        "synced Grok Build session auth from pool account"
    );
    Ok(())
}

fn backup_grok_build_config(content: &str) -> AppResult<()> {
    if content.is_empty() {
        return Ok(());
    }
    let backup_dir = app_home()?.join("backups");
    fs::create_dir_all(&backup_dir)?;
    let name = format!("grok-config-{}.toml", Utc::now().format("%Y%m%d-%H%M%S"));
    fs::write(backup_dir.join(name), content)?;
    Ok(())
}

/// Snapshot config.toml + auth.json for one-click restore (login credentials included).
fn snapshot_grok_build_for_restore(config_content: &str) -> AppResult<()> {
    let dir = grok_build_restore_dir()?;
    // config
    fs::write(dir.join(GROK_BUILD_RESTORE_CONFIG), config_content)?;
    // auth.json (may be missing if user never logged in)
    let auth_path = grok_build_auth_path();
    if auth_path.exists() {
        fs::copy(&auth_path, dir.join(GROK_BUILD_RESTORE_AUTH)).map_err(|e| {
            AppError::msg(format!("备份 Grok Build auth.json 失败：{e}"))
        })?;
    } else if dir.join(GROK_BUILD_RESTORE_AUTH).exists() {
        // Keep previous auth snapshot if live file missing.
    }
    let meta = json!({
        "savedAt": Utc::now().to_rfc3339(),
        "reason": "pre-multi-account-route-inject",
        "configPath": grok_build_config_path().display().to_string(),
        "authPath": auth_path.display().to_string(),
    });
    fs::write(
        dir.join(GROK_BUILD_RESTORE_META),
        serde_json::to_string_pretty(&meta)?,
    )?;
    tracing::info!(
        target: "integrations",
        dir = %dir.display(),
        "saved Grok Build pre-route restore snapshot"
    );
    Ok(())
}

fn backup_file_to(src: &std::path::Path, dest: &std::path::Path) -> AppResult<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dest).map_err(|e| AppError::msg(format!("backup {} failed: {e}", src.display())))?;
    Ok(())
}

/// Ensure `~/.grok-go/agents-guide.md` is current, then upsert a short fixed
/// reference into Codex global `AGENTS.md` (never paste the full guide there).
pub fn inject_codex_agents_guide() -> AppResult<IntegrationStatus> {
    ensure_agents_guide_file()?;
    write_codex_agents_guide_ref()?;
    integration_status()
}

fn inject_codex_mcp(config: &AppConfig) -> AppResult<()> {
    let path = codex_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let original = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    backup_codex_config(&original)?;

    let mut doc = original.parse::<toml_edit::DocumentMut>()
        .map_err(|e| AppError::msg(format!("invalid codex config.toml: {e}")))?;

    let host = if config.lan_enabled { local_lan_host() } else { "127.0.0.1".into() };
    let url = format!("http://{}:{}/mcp", host, config.actual_port);

    let servers = doc.entry("mcp_servers").or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let table = servers.as_table_mut().ok_or_else(|| AppError::msg("mcp_servers is not a table"))?;
    // Prefer canonical id; drop legacy key so Codex does not register two servers.
    table.remove(MCP_SERVER_ID_LEGACY);
    table.insert(
        MCP_SERVER_ID,
        toml_edit::Item::Table(grok_go_mcp_table(config, &url)),
    );
    fs::write(path, doc.to_string())?;
    Ok(())
}

/// Build `[mcp_servers.grok-go]` including local bearer auth for requireToken gateways.
fn grok_go_mcp_table(config: &AppConfig, url: &str) -> toml_edit::Table {
    let mut grok = toml_edit::Table::new();
    grok["url"] = toml_edit::value(url);
    // Codex HTTP MCP: static headers so requireToken gateways accept the connection.
    // https://developers.openai.com/codex/config-reference — mcp_servers.<id>.http_headers
    if config.require_token && !config.local_token.trim().is_empty() {
        let mut headers = toml_edit::Table::new();
        headers.set_implicit(true);
        headers["Authorization"] =
            toml_edit::value(format!("Bearer {}", config.local_token.trim()));
        grok.insert("http_headers", toml_edit::Item::Table(headers));
    }
    grok
}

fn remove_codex_mcp() -> AppResult<()> {
    let path = codex_config_path();
    if !path.exists() {
        return Ok(());
    }
    let original = fs::read_to_string(&path)?;
    backup_codex_config(&original)?;
    let mut doc = original.parse::<toml_edit::DocumentMut>()
        .map_err(|e| AppError::msg(format!("invalid codex config.toml: {e}")))?;
    if let Some(servers) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_mut()) {
        for id in MCP_SERVER_IDS {
            servers.remove(*id);
        }
        if servers.is_empty() {
            doc.remove("mcp_servers");
        }
    }
    fs::write(path, doc.to_string())?;
    Ok(())
}

fn backup_codex_config(content: &str) -> AppResult<()> {
    let backup_dir = crate::paths::app_home()?.join("backups");
    fs::create_dir_all(&backup_dir)?;
    let name = format!("codex-config-{}.toml", Utc::now().format("%Y%m%d-%H%M%S"));
    fs::write(backup_dir.join(name), content)?;
    Ok(())
}

/// Full guide body (versioned) written to `~/.grok-go/agents-guide.md`.
/// Only documents tools currently enabled in `AppConfig.mcp_enabled_tools`.
/// Runtime inject file for Codex — separate from the repo project `AGENTS.md` / llm-wiki.
fn agents_guide_file_body(config: &AppConfig) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let mut tools: Vec<&str> = Vec::new();
    for id in crate::config::default_mcp_tool_ids() {
        if !config.mcp_tool_enabled(id) {
            continue;
        }
        // Prefer documenting image_gen; skip alias when both are enabled.
        if *id == "image_generate" && config.mcp_tool_enabled("image_gen") {
            continue;
        }
        tools.push(*id);
    }

    let mut out = String::new();
    out.push_str(&format!(
        "# GrokGo 工具指引\n\n\
         > 版本：{version}  \n\
         > 本文件由 GrokGo 维护，随软件版本与「已启用 MCP 工具」同步更新。请勿手改（重新注入会覆盖）。\n\
         > 与仓库开发用 `AGENTS.md` / `llm-wiki` 无关；此处只服务运行时 MCP 调用。\n\n\
         ## 强制规则\n\n\
         - **参数以 MCP `tools/list` 为准**：直接调用，**禁止** web_search 或翻仓库源码猜参数。\n\
         - 媒体输入可用：`https://` / `data:` / **本地绝对路径** / `file://`（本地会自动转 data URL）。\n\
         - 返回一律是 `~/.grok-go/artifacts/` 下的**绝对本地路径** + `markdown`，用 `![image](/abs/path)` 渲染；**不要**展示远程 CDN URL。\n\n\
         ## 当前已启用工具\n\n"
    ));

    if tools.is_empty() {
        out.push_str("（当前未启用任何 MCP 工具。可在 GrokGo → 集成 → MCP 中开启。）\n\n");
    } else {
        for id in &tools {
            out.push_str(&tool_guide_section(id));
            out.push('\n');
        }
    }

    out.push_str(
        "## 健康检查\n\n\
         ```bash\n\
         curl -s http://127.0.0.1:8787/health\n\
         ```\n\n\
         Responses API Base：`http://127.0.0.1:8787/v1`  \n\
         MCP：`http://127.0.0.1:8787/mcp`  \n\
         产物目录：`~/.grok-go/artifacts/`\n",
    );
    out
}

fn tool_guide_section(id: &str) -> String {
    match id {
        "x_search" => "### `x_search`\n\
- 必填：`query`\n\
- 可选：`allowed_handles` `excluded_handles` `from_date` `to_date`（YYYY-MM-DD）\n"
            .into(),
        "image_gen" => "### `image_gen`（`image_generate` 同义别名）\n\
- 必填：`prompt`\n\
- 可选：`n`(1–4) `model` `size` `quality`(low|medium|high)\n"
            .into(),
        "image_generate" => "### `image_generate`（`image_gen` 别名）\n\
- 必填：`prompt`\n\
- 可选：`n`(1–4) `model` `size` `quality`(low|medium|high)\n"
            .into(),
        "image_edit" => "### `image_edit`\n\
- 必填：`prompt` + `image_url`（URL 或本地路径）\n\
- 可选：`model`\n"
            .into(),
        "video_generate" => "### `video_generate`（文生视频 / 图生视频 / 多图参考）\n\
- 必填：`prompt`\n\
- 模式（三选一）：\n\
  1. 文生视频：仅 `prompt`\n\
  2. 图生视频：`prompt` + `image_url`（首帧）\n\
  3. 多图参考：`prompt` + `reference_image_urls`（1–7，勿与 `image_url` 同用）\n\
- 可选：`duration`(1–15) `aspect_ratio`(1:1|16:9|9:16|4:3|3:4|3:2|2:3) `resolution`(480p|720p|1080p) `model`\n\
- 示例：`{\"prompt\":\"轻推镜头，微风吹动毛发\",\"image_url\":\"/abs/path.png\",\"duration\":6}`\n"
            .into(),
        "video_edit" => "### `video_edit`\n\
- 必填：`prompt` + `video_url`（URL 或本地路径）\n\
- 可选：`model`\n\
- 注意：编辑不支持自定义 duration/aspect_ratio\n"
            .into(),
        other => format!("### `{other}`\n- 参数以 MCP `tools/list` 为准。\n"),
    }
}

/// Write/update the versioned guide under the app config directory.
pub fn refresh_agents_guide_file() -> AppResult<std::path::PathBuf> {
    ensure_agents_guide_file()
}

fn ensure_agents_guide_file() -> AppResult<std::path::PathBuf> {
    let config = load_config()?;
    ensure_agents_guide_file_with(&config)
}

fn ensure_agents_guide_file_with(config: &AppConfig) -> AppResult<std::path::PathBuf> {
    let path = agents_guide_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, agents_guide_file_body(config))?;
    Ok(path)
}

/// Short fixed reference block for Codex `AGENTS.md` (path is absolute; anchor is fixed).
fn agents_guide_ref_block(guide_abs: &str) -> String {
    format!(
        "{AGENTS_GUIDE_START}\n\
         <!-- {AGENTS_GUIDE_REF_ANCHOR} -->\n\
         - GrokGo 工具完整说明见：`{guide_abs}`（随软件版本更新；调用 MCP 工具前请先阅读该文件）\n\
         {AGENTS_GUIDE_END}"
    )
}

/// Runtime detection: fixed anchor present in Codex AGENTS.md (new or legacy full block).
fn agents_guide_ref_present_at(path: &std::path::Path) -> bool {
    if !path.exists() {
        return false;
    }
    fs::read_to_string(path)
        .map(|raw| {
            raw.contains(AGENTS_GUIDE_REF_ANCHOR)
                || raw.contains(AGENTS_GUIDE_START)
                || raw.contains("<!-- grok-proxy:agents-guide:start -->")
        })
        .unwrap_or(false)
}

/// Replace existing managed block(s), or append if missing.
fn upsert_agents_guide_ref_content(existing: &str, guide_abs: &str) -> String {
    let block = agents_guide_ref_block(guide_abs);
    match strip_all_agents_guide_blocks(existing) {
        Some(stripped) => {
            let base = stripped.trim_end();
            if base.is_empty() {
                format!("{block}\n")
            } else {
                format!("{base}\n\n{block}\n")
            }
        }
        None => {
            let base = existing.trim_end();
            if base.is_empty() {
                format!("{block}\n")
            } else {
                format!("{base}\n\n{block}\n")
            }
        }
    }
}

/// Strip current and legacy managed blocks. Returns `None` if none found.
fn strip_all_agents_guide_blocks(content: &str) -> Option<String> {
    let mut current = content.to_string();
    let mut changed = false;
    loop {
        let Some((start_idx, end_idx)) = find_next_agents_block_range(&current) else {
            break;
        };
        changed = true;
        let mut before = current[..start_idx].to_string();
        let after = current[end_idx..].to_string();
        while before.ends_with("\n\n\n") {
            before.pop();
        }
        let after = after.trim_start_matches(['\r', '\n']);
        current = if before.is_empty() {
            after.to_string()
        } else if after.is_empty() {
            before.trim_end().to_string()
        } else {
            format!("{}\n\n{}", before.trim_end(), after)
        };
    }
    if changed {
        Some(current)
    } else {
        None
    }
}

/// Find the earliest managed agents-guide block as `[start, end)` byte range.
fn find_next_agents_block_range(content: &str) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for start in AGENTS_GUIDE_START_LEGACY {
        let Some(start_idx) = content.find(start) else {
            continue;
        };
        let from = &content[start_idx..];
        let Some((end_rel, end_len)) = AGENTS_GUIDE_END_LEGACY.iter().find_map(|end| {
            from.find(end).map(|p| (p, end.len()))
        }) else {
            continue;
        };
        let end_idx = start_idx + end_rel + end_len;
        best = Some(match best {
            Some((bs, be)) if bs <= start_idx => (bs, be),
            _ => (start_idx, end_idx),
        });
    }
    best
}

fn write_codex_agents_guide_ref() -> AppResult<()> {
    let guide = ensure_agents_guide_file()?;
    let guide_abs = guide.display().to_string();
    let path = codex_agents_md_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let original = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    backup_codex_agents_md(&original)?;
    let next = upsert_agents_guide_ref_content(&original, &guide_abs);
    fs::write(path, next)?;
    Ok(())
}

fn remove_codex_agents_guide() -> AppResult<()> {
    let path = codex_agents_md_path();
    if !path.exists() {
        return Ok(());
    }
    let original = fs::read_to_string(&path)?;
    if !agents_guide_ref_present_at(&path) && strip_all_agents_guide_blocks(&original).is_none() {
        return Ok(());
    }
    backup_codex_agents_md(&original)?;
    match strip_all_agents_guide_blocks(&original) {
        Some(stripped) => {
            let trimmed = stripped.trim();
            if trimmed.is_empty() {
                let _ = fs::remove_file(&path);
            } else {
                fs::write(&path, format!("{trimmed}\n"))?;
            }
        }
        None => {}
    }
    Ok(())
}

fn backup_codex_agents_md(content: &str) -> AppResult<()> {
    if content.is_empty() {
        return Ok(());
    }
    let backup_dir = app_home()?.join("backups");
    fs::create_dir_all(&backup_dir)?;
    let name = format!("codex-agents-{}.md", Utc::now().format("%Y%m%d-%H%M%S"));
    fs::write(backup_dir.join(name), content)?;
    Ok(())
}

pub fn import_cc_switch_provider() -> AppResult<String> {
    import_cc_switch_provider_for_app("codex")
}

/// Import GrokGo as a Claude Code provider into CC Switch (`app_type = claude`).
pub fn import_cc_switch_claude_provider() -> AppResult<String> {
    import_cc_switch_provider_for_app("claude")
}

fn import_cc_switch_provider_for_app(app_type: &str) -> AppResult<String> {
    let app_type = match app_type {
        "claude" => "claude",
        _ => "codex",
    };
    let config = load_config()?;
    // If MCP inject is currently on (flag or live codex config), ship MCP with the provider.
    let include_mcp = config.auto_inject_codex_mcp || codex_mcp_currently_injected();
    let db_path = cc_switch_db_path();
    if !db_path.exists() {
        let payload = if app_type == "claude" {
            claude_provider_export_json(&config)
        } else {
            provider_export_json(&config, include_mcp)
        };
        let export_name = if app_type == "claude" {
            "cc-switch-claude-provider-export.json"
        } else {
            "cc-switch-provider-export.json"
        };
        let export_path = crate::paths::app_home()?.join(export_name);
        fs::write(&export_path, serde_json::to_string_pretty(&payload)?)?;
        return Ok(format!(
            "未检测到 CC Switch 数据库（{}）。已把配置导出到：\n{}\n请先安装并打开一次 CC Switch，或在 CC Switch 里手动导入该 JSON。",
            db_path.display(),
            export_path.display()
        ));
    }

    let conn = Connection::open(&db_path).map_err(|e| {
        AppError::msg(format!(
            "无法打开 CC Switch 数据库（{}）：{e}\n请确认 CC Switch 已关闭占用或路径正确。",
            db_path.display()
        ))
    })?;

    let name = "GrokGo";
    let settings = if app_type == "claude" {
        claude_provider_settings_config(&config)
    } else {
        provider_settings_config(&config, include_mcp)
    };
    let settings_text = serde_json::to_string(&settings)?;
    let now = Utc::now().timestamp_millis();
    let notes = if app_type == "claude" {
        "由 GrokGo 同步（Claude Code / Anthropic Messages）"
    } else if include_mcp {
        "由 GrokGo 同步（含 MCP）"
    } else {
        "由 GrokGo 同步"
    };

    // Prefer updating an existing GrokGo provider for this app_type.
    let existing = find_existing_grokgo_provider_for_app(&conn, app_type)?;
    let (action_zh, provider_id) = if let Some((id, _created)) = existing {
        conn.execute(
            r#"
            UPDATE providers
            SET name = ?1,
                settings_config = ?2,
                notes = ?3,
                category = 'custom'
            WHERE id = ?4 AND app_type = ?5
            "#,
            params![name, settings_text, notes, id, app_type],
        )
        .map_err(|e| AppError::msg(format!("更新 CC Switch 中的 GrokGo 配置失败：{e}")))?;
        let removed = remove_duplicate_grokgo_providers_for_app(&conn, app_type, &id).unwrap_or(0);
        let mut action = "已更新".to_string();
        if removed > 0 {
            action = format!("已更新（并清理了 {removed} 条重复的 GrokGo 配置）");
        }
        (action, id)
    } else {
        let id = Uuid::new_v4().to_string();
        conn.execute(
            r#"
            INSERT INTO providers (
              id, app_type, name, settings_config, website_url, category, created_at,
              sort_index, notes, icon, icon_color, meta, is_current, in_failover_queue,
              cost_multiplier, limit_daily_usd, limit_monthly_usd, provider_type
            ) VALUES (?1,?2,?3,?4,NULL,'custom',?5,NULL,?6,NULL,NULL,'{}',0,0,'1.0',NULL,NULL,NULL)
            "#,
            params![id, app_type, name, settings_text, now, notes],
        )
        .map_err(|e| AppError::msg(format!("写入 CC Switch 失败：{e}")))?;
        ("已新增".to_string(), id)
    };

    let mut mcp_part = String::new();
    // Always try MCP upsert for Claude (enable Claude app flag); for Codex only when include_mcp.
    let should_mcp = app_type == "claude" || include_mcp;
    if should_mcp {
        match upsert_cc_switch_mcp_server_for_app(&conn, &config, app_type) {
            Ok(msg) => {
                if msg.contains("updated") {
                    mcp_part = " MCP 已同步更新。".into();
                } else if msg.contains("inserted") {
                    mcp_part = " MCP 已一并写入。".into();
                } else if msg.contains("missing") {
                    mcp_part = " （当前 CC Switch 版本无 MCP 表，已跳过 MCP）".into();
                } else {
                    mcp_part = format!(" MCP：{msg}");
                }
            }
            Err(err) => {
                tracing::warn!("cc-switch mcp_servers upsert failed: {err}");
                mcp_part = " Provider 已就绪，但 MCP 同步失败（可稍后在集成页重试）。".into();
            }
        }
    }

    let import_model = crate::config::cc_switch_import_default_model(&config.default_model);
    if app_type == "claude" {
        let base = anthropic_base_url(&config);
        return Ok(format!(
            "{action_zh} CC Switch 中的 GrokGo（Claude Code）配置。\n\
             已写入 ANTHROPIC_BASE_URL={base}（Messages 兼容层）。\n\
             模型：Haiku/Sonnet/Opus → {import_model}（可在 model_mappings 覆盖）。\n\
             请在 CC Switch 切换到 Claude 应用并选用 GrokGo，然后重启 Claude Code。{}",
            mcp_part.trim_end(),
        )
        .trim()
        .to_string()
            + &format!("\n（配置 id：{provider_id}）"));
    }

    let reasoning_note =
        if crate::config::xai_model_default_reasoning_effort(import_model).is_some() {
            " 已启用思考深度（model_reasoning_effort）。"
        } else {
            ""
        };
    Ok(format!(
        "{action_zh} CC Switch 中的 GrokGo 配置。\n已挂载可用模型：grok-4.5 / grok-4.3；默认：{}。{}\n网关：http://{}:{}/v1。{}",
        import_model,
        reasoning_note,
        if config.lan_enabled {
            local_lan_host()
        } else {
            "127.0.0.1".into()
        },
        config.actual_port,
        mcp_part.trim_end(),
    )
    .trim()
    .to_string()
        + &format!("\n（配置 id：{provider_id}）"))
}

/// Find existing GrokGo provider for `app_type` (`codex` | `claude`).
fn find_existing_grokgo_provider_for_app(
    conn: &Connection,
    app_type: &str,
) -> AppResult<Option<(String, i64)>> {
    // Match by name (primary) or legacy notes / config fingerprints.
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, COALESCE(created_at, 0), COALESCE(is_current, 0)
            FROM providers
            WHERE app_type = ?1
              AND (
                name = 'GrokGo'
                OR notes LIKE '%GrokGo%'
                OR settings_config LIKE '%model_providers.grok-go%'
                OR settings_config LIKE '%"name": "grok-go"%'
                OR settings_config LIKE '%name = "grok-go"%'
                OR settings_config LIKE '%name = "GrokGo"%'
                OR settings_config LIKE '%ANTHROPIC_BASE_URL%'
              )
            ORDER BY is_current DESC, created_at DESC
            "#,
        )
        .map_err(|e| AppError::msg(format!("查询 CC Switch 配置失败：{e}")))?;
    let mut rows = stmt
        .query(params![app_type])
        .map_err(|e| AppError::msg(format!("查询 CC Switch 配置失败：{e}")))?;
    if let Some(row) = rows
        .next()
        .map_err(|e| AppError::msg(format!("读取 CC Switch 配置失败：{e}")))?
    {
        let id: String = row.get(0)?;
        let created: i64 = row.get(1)?;
        return Ok(Some((id, created)));
    }
    Ok(None)
}

/// Remove other GrokGo providers for `app_type` after we kept `keep_id`.
fn remove_duplicate_grokgo_providers_for_app(
    conn: &Connection,
    app_type: &str,
    keep_id: &str,
) -> AppResult<usize> {
    let n = conn.execute(
        r#"
        DELETE FROM providers
        WHERE app_type = ?1
          AND id != ?2
          AND (
            name = 'GrokGo'
            OR notes LIKE '%GrokGo%'
            OR notes LIKE '%Imported from GrokGo%'
            OR notes LIKE '%由 GrokGo%'
          )
        "#,
        params![app_type, keep_id],
    )?;
    Ok(n)
}

fn codex_mcp_currently_injected() -> bool {
    let path = codex_config_path();
    if !path.exists() {
        return false;
    }
    fs::read_to_string(path)
        .map(|raw| codex_mcp_is_injected(&raw))
        .unwrap_or(false)
}

fn upsert_cc_switch_mcp_server_for_app(
    conn: &Connection,
    config: &AppConfig,
    app_type: &str,
) -> AppResult<String> {
    let has_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='mcp_servers' LIMIT 1",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !has_table {
        return Ok("mcp_servers table missing".into());
    }

    let host = if config.lan_enabled {
        local_lan_host()
    } else {
        "127.0.0.1".into()
    };
    let url = format!("http://{}:{}/mcp", host, config.actual_port);
    let mut server = json!({
        "type": "http",
        "url": url,
    });
    if config.require_token && !config.local_token.trim().is_empty() {
        server["headers"] = json!({
            "Authorization": format!("Bearer {}", config.local_token.trim())
        });
    }
    let server_text = serde_json::to_string(&server)?;
    let id = "grok-go";
    let name = "GrokGo";
    let description = "GrokGo local MCP (x_search / image / video)";

    let existing: bool = conn
        .query_row(
            "SELECT 1 FROM mcp_servers WHERE id = ?1 LIMIT 1",
            params![id],
            |_| Ok(true),
        )
        .unwrap_or(false);

    let enable_codex = app_type == "codex";
    let enable_claude = app_type == "claude";
    // When updating for one app, preserve the other flag if the column exists.
    let has_claude_col = conn
        .prepare("SELECT enabled_claude FROM mcp_servers LIMIT 0")
        .is_ok();

    if existing {
        if has_claude_col {
            if enable_claude {
                conn.execute(
                    r#"
                    UPDATE mcp_servers
                    SET name = ?1,
                        server_config = ?2,
                        description = ?3,
                        enabled_claude = 1
                    WHERE id = ?4
                    "#,
                    params![name, server_text, description, id],
                )?;
                Ok("updated mcp_servers.grok-go (enabled_claude=1)".into())
            } else {
                conn.execute(
                    r#"
                    UPDATE mcp_servers
                    SET name = ?1,
                        server_config = ?2,
                        description = ?3,
                        enabled_codex = 1
                    WHERE id = ?4
                    "#,
                    params![name, server_text, description, id],
                )?;
                Ok("updated mcp_servers.grok-go (enabled_codex=1)".into())
            }
        } else {
            conn.execute(
                r#"
                UPDATE mcp_servers
                SET name = ?1,
                    server_config = ?2,
                    description = ?3,
                    enabled_codex = 1
                WHERE id = ?4
                "#,
                params![name, server_text, description, id],
            )?;
            Ok("updated mcp_servers.grok-go (enabled_codex=1)".into())
        }
    } else if has_claude_col {
        conn.execute(
            r#"
            INSERT INTO mcp_servers (
              id, name, server_config, description, tags, enabled_codex, enabled_claude
            ) VALUES (?1, ?2, ?3, ?4, '[]', ?5, ?6)
            "#,
            params![
                id,
                name,
                server_text,
                description,
                if enable_codex { 1 } else { 0 },
                if enable_claude { 1 } else { 0 }
            ],
        )?;
        Ok(format!(
            "inserted mcp_servers.grok-go (enabled_codex={}, enabled_claude={})",
            if enable_codex { 1 } else { 0 },
            if enable_claude { 1 } else { 0 }
        ))
    } else {
        conn.execute(
            r#"
            INSERT INTO mcp_servers (id, name, server_config, description, tags, enabled_codex)
            VALUES (?1, ?2, ?3, ?4, '[]', 1)
            "#,
            params![id, name, server_text, description],
        )?;
        Ok("inserted mcp_servers.grok-go (enabled_codex=1)".into())
    }
}

/// CC Switch Claude provider `settings_config` shape (env block for Claude Code).
fn claude_provider_settings_config(config: &AppConfig) -> serde_json::Value {
    let base = anthropic_base_url(config);
    let token = config.local_token.trim();
    let model = crate::config::cc_switch_import_default_model(&config.default_model);
    // Haiku can map to a lighter model when catalog allows; keep simple: same default
    // unless user default is already grok-4.3.
    let haiku = if model == "grok-4.5" {
        "grok-4.3"
    } else {
        model
    };
    json!({
        "env": {
            "ANTHROPIC_BASE_URL": base,
            "ANTHROPIC_AUTH_TOKEN": token,
            "ANTHROPIC_API_KEY": token,
            "ANTHROPIC_MODEL": model,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL": haiku,
            "ANTHROPIC_DEFAULT_SONNET_MODEL": model,
            "ANTHROPIC_DEFAULT_OPUS_MODEL": model
        }
    })
}

fn claude_provider_export_json(config: &AppConfig) -> serde_json::Value {
    json!({
        "app_type": "claude",
        "name": "GrokGo",
        "settings_config": claude_provider_settings_config(config)
    })
}

/// Human-readable snippet for the Integrations → Claude Code tab.
pub fn claude_code_settings_snippet(config: &AppConfig) -> String {
    let settings = claude_provider_settings_config(config);
    serde_json::to_string_pretty(&settings).unwrap_or_else(|_| "{}".into())
}

fn provider_settings_config(config: &AppConfig, include_mcp: bool) -> serde_json::Value {
    use crate::config::{cc_switch_import_default_model, xai_model_default_reasoning_effort};
    let host = if config.lan_enabled {
        local_lan_host()
    } else {
        "127.0.0.1".into()
    };
    let base = format!("http://{}:{}/v1", host, config.actual_port);
    // Only offer usable Codex models in the imported provider; clamp default
    // if the app default is something not in the import list.
    let import_model = cc_switch_import_default_model(&config.default_model);
    // Use provider id `grok-go` (not anonymous "custom") so CC Switch / Codex UI
    // can list concrete models instead of a blank custom slot.
    let mut toml = format!(
        "model_provider = \"grok-go\"\n\
         model = \"{import_model}\"\n"
    );
    // Only emit when the default model accepts reasoning.effort (otherwise
    // Codex may send an effort the upstream rejects).
    if let Some(effort) = xai_model_default_reasoning_effort(import_model) {
        toml.push_str(&format!("model_reasoning_effort = \"{effort}\"\n"));
    }
    toml.push_str(&format!(
        "disable_response_storage = true\n\
         \n\
         [model_providers.grok-go]\n\
         name = \"GrokGo\"\n\
         wire_api = \"responses\"\n\
         requires_openai_auth = true\n\
         base_url = \"{}\"\n\
         experimental_bearer_token = \"{}\"\n",
        base, config.local_token
    ));
    if include_mcp {
        let mcp_url = format!("http://{}:{}/mcp", host, config.actual_port);
        toml.push_str("\n[mcp_servers.grok-go]\n");
        toml.push_str(&format!("url = \"{mcp_url}\"\n"));
        if config.require_token && !config.local_token.trim().is_empty() {
            toml.push_str("\n[mcp_servers.grok-go.http_headers]\n");
            toml.push_str(&format!(
                "Authorization = \"Bearer {}\"\n",
                config.local_token.trim()
            ));
        }
    }
    json!({
        "auth": {"OPENAI_API_KEY": config.local_token},
        "config": toml,
        "modelCatalog": {
            "models": codex_model_catalog_models(config)
        }
    })
}

/// Models shown in CC Switch / Codex picker after GrokGo import.
///
/// Only [`crate::config::cc_switch_import_models`] — empirically usable with
/// Codex + depth UI. No 4.20 fixed variants, multi-agent, build, media, or
/// Cursor-only names.
///
/// Each entry carries Codex catalog reasoning fields
/// (`default_reasoning_level` / `supported_reasoning_levels`).
fn codex_model_catalog_models(config: &AppConfig) -> Vec<serde_json::Value> {
    use crate::config::{
        cc_switch_import_default_model, cc_switch_import_models, xai_model_default_reasoning_effort,
        xai_model_reasoning_efforts,
    };
    let preferred = cc_switch_import_default_model(&config.default_model);
    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut push = |model: &str, display: &str| {
        if model.is_empty() {
            return;
        }
        if out
            .iter()
            .any(|m| m.get("model").and_then(|v| v.as_str()) == Some(model))
        {
            return;
        }
        let mut entry = json!({
            "model": model,
            "displayName": display,
            "contextWindow": 500000
        });
        if let (Some(levels), Some(default_effort)) = (
            xai_model_reasoning_efforts(model),
            xai_model_default_reasoning_effort(model),
        ) {
            let level_objs: Vec<serde_json::Value> = levels
                .iter()
                .map(|effort| {
                    json!({
                        "effort": effort,
                        "description": reasoning_effort_description(effort),
                    })
                })
                .collect();
            // snake_case matches Codex `cc-switch-model-catalog.json` / native
            // catalog schema that CC Switch writes on apply.
            entry["default_reasoning_level"] = json!(default_effort);
            entry["supported_reasoning_levels"] = json!(level_objs);
            entry["supports_reasoning_summaries"] = json!(true);
            // camelCase mirrors (in case a CC Switch build reads these)
            entry["defaultReasoningLevel"] = json!(default_effort);
            entry["supportedReasoningLevels"] = json!(level_objs);
            entry["supportsReasoningSummaries"] = json!(true);
        }
        out.push(entry);
    };
    // Preferred default first for nicer picker UX, then the rest of the allowlist.
    push(preferred, preferred);
    for id in cc_switch_import_models() {
        push(id, id);
    }
    out
}

fn reasoning_effort_description(effort: &str) -> &'static str {
    match effort {
        "none" => "No extra reasoning — fastest replies",
        "low" => "Fast responses with lighter reasoning",
        "medium" => "Balances speed and reasoning depth for everyday tasks",
        "high" => "Greater reasoning depth for complex problems",
        "xhigh" => "Extra high reasoning depth for complex problems",
        _ => "Reasoning effort level",
    }
}

fn provider_export_json(config: &AppConfig, include_mcp: bool) -> serde_json::Value {
    json!({
        "app_type": "codex",
        "name": "GrokGo",
        "settings_config": provider_settings_config(config, include_mcp)
    })
}

fn provider_snippet(config: &AppConfig) -> String {
    let host = if config.lan_enabled { local_lan_host() } else { "127.0.0.1".into() };
    format!(
        "[model_providers.grok-go]\nname = \"grok-go\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"http://{}:{}/v1\"\nexperimental_bearer_token = \"{}\"\n",
        host, config.actual_port, config.local_token
    )
}

fn mcp_snippet(config: &AppConfig) -> String {
    let host = if config.lan_enabled { local_lan_host() } else { "127.0.0.1".into() };
    let url = format!("http://{}:{}/mcp", host, config.actual_port);
    if config.require_token && !config.local_token.trim().is_empty() {
        format!(
            "[mcp_servers.grok-go]\nurl = \"{url}\"\n\n[mcp_servers.grok-go.http_headers]\nAuthorization = \"Bearer {}\"\n",
            config.local_token.trim()
        )
    } else {
        format!("[mcp_servers.grok-go]\nurl = \"{url}\"\n")
    }
}

fn local_lan_host() -> String {
    local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "0.0.0.0".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_appends_short_ref_when_missing() {
        let guide = "/Users/me/.grok-go/agents-guide.md";
        let out = upsert_agents_guide_ref_content("# User notes\n\nhello", guide);
        assert!(out.contains("# User notes"));
        assert!(out.contains("hello"));
        assert!(out.contains(AGENTS_GUIDE_START));
        assert!(out.contains(AGENTS_GUIDE_REF_ANCHOR));
        assert!(out.contains(guide));
        // Full tool docs must NOT be pasted into AGENTS.md.
        assert!(!out.contains("## 强制规则"));
        assert!(!out.contains("`x_search`"));
        assert_eq!(out.matches(AGENTS_GUIDE_START).count(), 1);
    }

    #[test]
    fn upsert_replaces_existing_ref_only() {
        let guide = "/tmp/agents-guide.md";
        let first = upsert_agents_guide_ref_content("user A", guide);
        let second = upsert_agents_guide_ref_content(&first.replace("user A", "user B"), guide);
        assert!(second.contains("user B"));
        assert!(!second.contains("user A"));
        assert_eq!(second.matches(AGENTS_GUIDE_START).count(), 1);
        assert_eq!(second.matches(AGENTS_GUIDE_END).count(), 1);
        assert!(second.contains(AGENTS_GUIDE_REF_ANCHOR));
    }

    #[test]
    fn strip_removes_legacy_full_block() {
        let legacy = r#"keep me

## <!-- grok-proxy:agents-guide:start -->

## Grok Proxy 工具

- 搜索 X / Twitter：直接调用 `x_search`

<!-- grok-proxy:agents-guide:end -->
"#;
        let stripped = strip_all_agents_guide_blocks(legacy).expect("block present");
        assert!(stripped.contains("keep me"));
        assert!(!stripped.contains("grok-proxy:agents-guide"));
        assert!(!stripped.contains("x_search"));
    }

    #[test]
    fn strip_none_when_no_block() {
        assert!(strip_all_agents_guide_blocks("just user text").is_none());
    }

    #[test]
    fn detect_ref_anchor() {
        let guide = "/Users/me/.grok-go/agents-guide.md";
        let content = upsert_agents_guide_ref_content("", guide);
        assert!(content.contains(AGENTS_GUIDE_REF_ANCHOR));
        assert!(content.contains(AGENTS_GUIDE_START));
    }

    #[test]
    fn agents_guide_lists_only_enabled_tools() {
        let mut cfg = AppConfig::default();
        cfg.mcp_enabled_tools = Some(vec!["x_search".into(), "image_gen".into()]);
        let body = agents_guide_file_body(&cfg);
        assert!(body.contains("`x_search`"));
        assert!(body.contains("`image_gen`"));
        assert!(!body.contains("`video_generate`"));
        assert!(!body.contains("`video_edit`"));
        assert!(body.contains("与仓库开发用"));
    }

    #[test]
    fn model_catalog_includes_only_usable_import_models() {
        let cfg = AppConfig::default();
        let models = codex_model_catalog_models(&cfg);
        let ids: Vec<&str> = models
            .iter()
            .filter_map(|m| m.get("model").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(ids, vec!["grok-4.5", "grok-4.3"]);
        // Unusable / unoffered in import.
        for bad in [
            "grok-4.20-0309-reasoning",
            "grok-4.20-0309-non-reasoning",
            "grok-4.20-multi-agent-0309",
            "grok-build-0.1",
            "composer",
            "imagine",
        ] {
            assert!(
                !ids.iter().any(|id| id.to_ascii_lowercase().contains(bad)),
                "unexpected model containing {bad}: {ids:?}"
            );
        }
        let settings = provider_settings_config(&cfg, false);
        let toml = settings.get("config").and_then(|c| c.as_str()).unwrap_or("");
        assert!(toml.contains("model_provider = \"grok-go\""));
        assert!(toml.contains("model = \"grok-4.5\""));
        assert!(toml.contains("[model_providers.grok-go]"));
        assert!(!toml.contains("model_provider = \"custom\""));
    }

    #[test]
    fn model_catalog_marks_reasoning_depth_for_import_models() {
        let cfg = AppConfig::default();
        let models = codex_model_catalog_models(&cfg);
        let by_id = |id: &str| {
            models
                .iter()
                .find(|m| m.get("model").and_then(|v| v.as_str()) == Some(id))
                .cloned()
                .expect(id)
        };

        let g45 = by_id("grok-4.5");
        assert_eq!(
            g45.get("default_reasoning_level").and_then(|v| v.as_str()),
            Some("medium")
        );
        let levels: Vec<&str> = g45["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|l| l.get("effort").and_then(|e| e.as_str()))
            .collect();
        assert_eq!(levels, vec!["low", "medium", "high"]);
        assert!(!levels.contains(&"none"));
        assert!(!levels.contains(&"xhigh"));

        let g43 = by_id("grok-4.3");
        let levels43: Vec<&str> = g43["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|l| l.get("effort").and_then(|e| e.as_str()))
            .collect();
        assert_eq!(levels43, vec!["none", "low", "medium", "high"]);

        // Default is clamped into the import allowlist → always has effort.
        let settings = provider_settings_config(&cfg, false);
        let toml = settings["config"].as_str().unwrap();
        assert!(toml.contains("model_reasoning_effort = \"medium\""));

        // App default outside allowlist still imports as grok-4.5 + effort.
        let mut cfg2 = AppConfig::default();
        cfg2.default_model = "grok-build-0.1".into();
        let models2 = codex_model_catalog_models(&cfg2);
        let ids2: Vec<&str> = models2
            .iter()
            .filter_map(|m| m.get("model").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(ids2, vec!["grok-4.5", "grok-4.3"]);
        let toml2 = provider_settings_config(&cfg2, false)["config"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(toml2.contains("model = \"grok-4.5\""));
        assert!(toml2.contains("model_reasoning_effort = \"medium\""));
        assert!(!toml2.contains("grok-build-0.1"));
    }

    #[test]
    fn claude_provider_settings_has_anthropic_env() {
        let mut cfg = AppConfig::default();
        cfg.actual_port = 8787;
        cfg.preferred_port = 8787;
        cfg.local_token = "tok_test".into();
        cfg.default_model = "grok-4.5".into();
        cfg.lan_enabled = false;
        let settings = claude_provider_settings_config(&cfg);
        let env = settings.get("env").expect("env");
        assert_eq!(
            env.get("ANTHROPIC_BASE_URL").and_then(|v| v.as_str()),
            Some("http://127.0.0.1:8787")
        );
        assert_eq!(
            env.get("ANTHROPIC_AUTH_TOKEN").and_then(|v| v.as_str()),
            Some("tok_test")
        );
        assert_eq!(
            env.get("ANTHROPIC_MODEL").and_then(|v| v.as_str()),
            Some("grok-4.5")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
                .and_then(|v| v.as_str()),
            Some("grok-4.3")
        );
        assert_eq!(
            env.get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                .and_then(|v| v.as_str()),
            Some("grok-4.5")
        );
        // Must not include /v1 suffix.
        let base = env
            .get("ANTHROPIC_BASE_URL")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(!base.ends_with("/v1"));

        let export = claude_provider_export_json(&cfg);
        assert_eq!(export.get("app_type").and_then(|v| v.as_str()), Some("claude"));
        assert_eq!(export.get("name").and_then(|v| v.as_str()), Some("GrokGo"));
    }

    #[test]
    fn provider_settings_includes_mcp_when_requested() {
        let mut cfg = AppConfig::default();
        cfg.actual_port = 8787;
        cfg.local_token = "tok123".into();
        cfg.require_token = true;
        let with_mcp = provider_settings_config(&cfg, true);
        let toml = with_mcp["config"].as_str().unwrap();
        assert!(toml.contains("[mcp_servers.grok-go]"));
        assert!(toml.contains("http://127.0.0.1:8787/mcp"));
        assert!(toml.contains("Bearer tok123"));
        let no_mcp = provider_settings_config(&cfg, false);
        let toml2 = no_mcp["config"].as_str().unwrap();
        assert!(!toml2.contains("[mcp_servers.grok-go]"));
    }

    #[test]
    fn mcp_detects_canonical_grok_go() {
        let raw = r#"
[mcp_servers.grok-go]
url = "http://127.0.0.1:8787/mcp"
"#;
        assert!(codex_mcp_is_injected(raw));
    }

    #[test]
    fn mcp_detects_legacy_grok_proxy() {
        // User configs written before the GrokGo rename still count as injected.
        let raw = r#"
[mcp_servers.grok-proxy]
url = "http://127.0.0.1:8787/mcp"
"#;
        assert!(codex_mcp_is_injected(raw));
    }

    #[test]
    fn mcp_ignores_empty_or_unrelated() {
        assert!(!codex_mcp_is_injected(""));
        assert!(!codex_mcp_is_injected("[mcp_servers.other]\nurl = \"http://x\"\n"));
        // Present but no /mcp path → not our gateway.
        assert!(!codex_mcp_is_injected(
            "[mcp_servers.grok-proxy]\nurl = \"http://127.0.0.1:8787/v1\"\n"
        ));
        // Header alone without url.
        assert!(!codex_mcp_is_injected("[mcp_servers.grok-go]\nenabled = true\n"));
    }

    #[test]
    fn scores_prefer_grok_build_referrer_over_sub2api_high_tier() {
        use crate::config::{Account, AccountAuthKind};
        let mut sub2 = Account::new("sub2");
        sub2.enabled = true;
        sub2.auth_kind = AccountAuthKind::Oauth;
        sub2.refresh_token = Some("rt".into());
        // tier=4 but referrer=sub2api — often fails TUI GrowthBook gate
        sub2.access_token = Some(
            "eyJhbGciOiJub25lIn0.eyJ0aWVyIjo0LCJyZWZlcnJlciI6InN1YjJhcGkiLCJzY29wZSI6Im9wZW5pZCBncm9rLWNsaTphY2Nlc3MgYXBpOmFjY2VzcyJ9.sig".into(),
        );

        let mut native = Account::new("native");
        native.enabled = true;
        native.auth_kind = AccountAuthKind::Oauth;
        native.refresh_token = Some("rt".into());
        // lower tier but official grok-build surface
        native.access_token = Some(
            "eyJhbGciOiJub25lIn0.eyJ0aWVyIjoxLCJyZWZlcnJlciI6Imdyb2stYnVpbGQiLCJzY29wZSI6Im9wZW5pZCBncm9rLWNsaTphY2Nlc3MgYXBpOmFjY2VzcyBjb252ZXJzYXRpb25zOnJlYWQgY29udmVyc2F0aW9uczp3cml0ZSJ9.sig".into(),
        );

        assert!(
            score_account_for_grok_build_session(&native)
                > score_account_for_grok_build_session(&sub2)
        );
    }

    #[test]
    fn scores_higher_tier_for_session_pick() {
        use crate::config::{Account, AccountAuthKind};
        let mut low = Account::new("low");
        low.enabled = true;
        low.auth_kind = AccountAuthKind::Oauth;
        low.refresh_token = Some("rt".into());
        low.access_token = None;

        let mut high = Account::new("high");
        high.enabled = true;
        high.auth_kind = AccountAuthKind::Oauth;
        high.refresh_token = Some("rt".into());
        high.access_token = Some(
            "eyJhbGciOiJub25lIn0.eyJ0aWVyIjo0LCJyZWZlcnJlciI6Imdyb2stYnVpbGQiLCJzY29wZSI6Imdyb2stY2xpOmFjY2VzcyJ9.sig".into(),
        );

        assert!(
            score_account_for_grok_build_session(&high)
                > score_account_for_grok_build_session(&low)
        );
    }

    #[test]
    fn picks_enabled_credentialed_over_disabled() {
        use crate::config::{Account, AccountAuthKind, AuthStore};
        let mut disabled = Account::new("d");
        disabled.enabled = false;
        disabled.auth_kind = AccountAuthKind::Oauth;
        disabled.access_token = Some(
            "eyJhbGciOiJub25lIn0.eyJ0aWVyIjo0fQ.sig".into(),
        );
        disabled.refresh_token = Some("rt".into());
        let mut enabled = Account::new("e");
        enabled.enabled = true;
        enabled.auth_kind = AccountAuthKind::Oauth;
        enabled.access_token = Some(
            "eyJhbGciOiJub25lIn0.eyJ0aWVyIjoxfQ.sig".into(),
        );
        enabled.refresh_token = Some("rt".into());
        let store = AuthStore {
            accounts: vec![disabled, enabled.clone()],
        };
        let picked = pick_best_account_for_grok_build_session(&store).unwrap();
        assert_eq!(picked.name, enabled.name);
    }

    #[test]
    fn grok_build_detects_cli_chat_proxy_base_url() {
        let mut cfg = AppConfig::default();
        cfg.actual_port = 8787;
        let raw = r#"
[endpoints]
cli_chat_proxy_base_url = "http://127.0.0.1:8787/v1"
"#;
        assert!(grok_build_is_injected(raw, &cfg));
        // models_base_url alone is NOT the standard inject path.
        let api_only = r#"
[endpoints]
models_base_url = "http://127.0.0.1:8787/v1"
"#;
        assert!(!grok_build_is_injected(api_only, &cfg));
    }

    #[test]
    fn grok_build_ignores_single_model_only() {
        let mut cfg = AppConfig::default();
        cfg.actual_port = 8787;
        // Wrong legacy approach: only [model.grok-go], no endpoints.
        let raw = r#"
[model.grok-go]
model = "grok-build"
base_url = "http://127.0.0.1:8787/v1"
api_key = "x"
"#;
        assert!(!grok_build_is_injected(raw, &cfg));
    }

    #[test]
    fn grok_build_snippet_is_standard_session() {
        let mut cfg = AppConfig::default();
        cfg.actual_port = 8787;
        let snip = grok_build_snippet(&cfg);
        assert!(snip.contains("cli_chat_proxy_base_url"));
        assert!(snip.contains("GROK_CLI_CHAT_PROXY_BASE_URL"));
        // Must not assign models_base_url (comment warning is fine).
        assert!(!snip.contains("models_base_url ="));
        assert!(!snip.contains("XAI_API_KEY"));
        assert!(!snip.contains("GROK_MODELS_BASE_URL"));
    }


}
