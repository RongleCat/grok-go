use chrono::Utc;
use once_cell::sync::Lazy;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::auth::TokenProbe;
use crate::config::{load_config, save_config, Account, AppConfig};
use crate::error::{AppError, AppResult};
use crate::paths::{
    agents_guide_file_path, app_home, cc_switch_db_path, codex_agents_md_path, codex_config_path,
    cursor_mcp_path, grok_build_auth_path, grok_build_config_path, grok_build_restore_dir,
    opencode_config_path, workbuddy_mcp_dot_path, workbuddy_mcp_path, workbuddy_models_path,
};

/// Serialize writes to `~/.grok/auth.json` (inject + background maintainer).
static GROK_AUTH_WRITE_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));
/// Ensure the maintainer loop is only started once per process.
static GROK_AUTH_MAINTAINER_STARTED: AtomicBool = AtomicBool::new(false);

/// How often to refresh a validated pool session into Grok Build `auth.json`.
/// Token lifetimes are usually hours; 15m keeps `expires_at` healthy without hammering IdP.
const GROK_AUTH_MAINTAIN_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Delay before the first maintenance tick so startup / gateway settle first.
const GROK_AUTH_MAINTAIN_INITIAL_DELAY: Duration = Duration::from_secs(45);
/// How many pool accounts to try (best → next) if userinfo rejects the token.
const GROK_AUTH_MAX_ACCOUNT_TRIES: usize = 3;

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

    // ── Other clients (OpenCode / WorkBuddy / Cursor) ──────────────────────
    /// OpenCode: custom GrokGo provider (+ model) present in opencode.json.
    pub opencode_model_injected: bool,
    /// OpenCode: grok-go MCP remote entry present.
    pub opencode_mcp_injected: bool,
    pub opencode_config_path: String,
    /// WorkBuddy: GrokGo model entry in models.json.
    pub workbuddy_model_injected: bool,
    /// WorkBuddy: grok-go MCP in .mcp.json / mcp.json.
    pub workbuddy_mcp_injected: bool,
    pub workbuddy_models_path: String,
    pub workbuddy_mcp_path: String,
    /// Cursor: grok-go MCP in ~/.cursor/mcp.json (BYOK model is copy-only).
    pub cursor_mcp_injected: bool,
    pub cursor_mcp_path: String,
    /// Cursor BYOK copy fields (Key/Base URL live in secure storage — no inject).
    pub cursor_byok_base_url: String,
    pub cursor_byok_token: String,
    pub cursor_byok_model: String,
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
    let opencode_path = opencode_config_path();
    let opencode_raw = if opencode_path.exists() {
        fs::read_to_string(&opencode_path).unwrap_or_default()
    } else {
        String::new()
    };
    let workbuddy_models = workbuddy_models_path();
    let workbuddy_mcp = workbuddy_mcp_path();
    let cursor_mcp = cursor_mcp_path();
    let byok = cursor_byok_fields(&config);
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
        opencode_model_injected: opencode_model_is_injected(&opencode_raw, &config),
        opencode_mcp_injected: opencode_mcp_is_injected(&opencode_raw, &config),
        opencode_config_path: opencode_path.display().to_string(),
        workbuddy_model_injected: workbuddy_model_is_injected(&workbuddy_models, &config),
        workbuddy_mcp_injected: client_mcp_json_is_injected(&workbuddy_mcp, &config),
        workbuddy_models_path: workbuddy_models.display().to_string(),
        workbuddy_mcp_path: workbuddy_mcp.display().to_string(),
        cursor_mcp_injected: client_mcp_json_is_injected(&cursor_mcp, &config),
        cursor_mcp_path: cursor_mcp.display().to_string(),
        cursor_byok_base_url: byok.base_url,
        cursor_byok_token: byok.token,
        cursor_byok_model: byok.model,
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

fn gateway_mcp_url(config: &AppConfig) -> String {
    let host = if config.lan_enabled {
        local_lan_host()
    } else {
        "127.0.0.1".into()
    };
    format!("http://{}:{}/mcp", host, config.actual_port)
}

/// Full OpenAI Chat Completions path (WorkBuddy / some clients require it).
fn gateway_chat_completions_url(config: &AppConfig) -> String {
    format!("{}/chat/completions", gateway_base_url(config))
}

struct CursorByokFields {
    base_url: String,
    token: String,
    model: String,
}

fn cursor_byok_fields(config: &AppConfig) -> CursorByokFields {
    CursorByokFields {
        base_url: gateway_base_url(config),
        token: if config.require_token {
            config.local_token.trim().to_string()
        } else {
            "not-required".into()
        },
        model: {
            let m = config.default_model.trim();
            if m.is_empty() {
                "grok-4.5".into()
            } else {
                m.to_string()
            }
        },
    }
}

// ── OpenCode ────────────────────────────────────────────────────────────────

const OPENCODE_PROVIDER_ID: &str = "grok-go";

fn read_json_object(path: &std::path::Path) -> AppResult<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let raw = fs::read_to_string(path)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(trimmed)
        .map_err(|e| AppError::msg(format!("invalid JSON {}: {e}", path.display())))
}

fn write_json_pretty(path: &std::path::Path, value: &Value) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| AppError::msg(format!("serialize JSON failed: {e}")))?;
    fs::write(path, format!("{body}\n"))?;
    Ok(())
}

fn backup_named(label: &str, content: &str, ext: &str) -> AppResult<()> {
    let backup_dir = app_home()?.join("backups");
    fs::create_dir_all(&backup_dir)?;
    let name = format!(
        "{label}-{}.{}",
        Utc::now().format("%Y%m%d-%H%M%S"),
        ext
    );
    fs::write(backup_dir.join(name), content)?;
    Ok(())
}

fn opencode_model_is_injected(raw: &str, config: &AppConfig) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    let Some(provider) = v
        .get("provider")
        .and_then(|p| p.get(OPENCODE_PROVIDER_ID))
    else {
        return false;
    };
    let base = provider
        .pointer("/options/baseURL")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    url_points_at_gateway(base, config)
}

fn opencode_mcp_is_injected(raw: &str, config: &AppConfig) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    let Some(mcp) = v.get("mcp").and_then(|m| m.as_object()) else {
        return false;
    };
    for id in MCP_SERVER_IDS {
        if let Some(entry) = mcp.get(*id) {
            let url = entry.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if url_points_at_gateway(url, config) && url.contains("mcp") {
                return true;
            }
        }
    }
    false
}

/// Inject OpenCode custom provider + default model (merge; does not wipe other keys).
pub fn set_opencode_model_inject(enabled: bool) -> AppResult<IntegrationStatus> {
    let config = load_config()?;
    let path = opencode_config_path();
    let original = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    if !original.is_empty() {
        backup_named("opencode-config", &original, "json")?;
    }
    let mut root = if original.trim().is_empty() {
        json!({ "$schema": "https://opencode.ai/config.json" })
    } else {
        read_json_object(&path)?
    };
    if !root.is_object() {
        root = json!({ "$schema": "https://opencode.ai/config.json" });
    }
    let obj = root
        .as_object_mut()
        .ok_or_else(|| AppError::msg("opencode.json root must be an object"))?;
    if enabled {
        let base = gateway_base_url(&config);
        let api_key = if config.require_token && !config.local_token.trim().is_empty() {
            config.local_token.trim().to_string()
        } else {
            "grok-go".into()
        };
        let default_model = {
            let m = config.default_model.trim();
            if m.is_empty() {
                "grok-4.5"
            } else {
                m
            }
        };
        let mut models = serde_json::Map::new();
        for id in ["grok-4.5", "grok-4.3"] {
            models.insert(id.to_string(), json!({ "name": id }));
        }
        // Ensure default is present even if not in the short list.
        if !models.contains_key(default_model) {
            models.insert(
                default_model.to_string(),
                json!({ "name": default_model }),
            );
        }
        let provider_entry = json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": "GrokGo",
            "options": {
                "baseURL": base,
                "apiKey": api_key,
            },
            "models": models,
        });
        let provider = obj
            .entry("provider")
            .or_insert_with(|| json!({}));
        if let Some(p) = provider.as_object_mut() {
            p.insert(OPENCODE_PROVIDER_ID.to_string(), provider_entry);
        } else {
            *provider = json!({ OPENCODE_PROVIDER_ID: provider_entry });
        }
        obj.insert(
            "model".into(),
            json!(format!("{OPENCODE_PROVIDER_ID}/{default_model}")),
        );
    } else {
        if let Some(provider) = obj.get_mut("provider").and_then(|p| p.as_object_mut()) {
            provider.remove(OPENCODE_PROVIDER_ID);
            if provider.is_empty() {
                obj.remove("provider");
            }
        }
        // Clear default model only when it pointed at our provider.
        if obj
            .get("model")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.starts_with(&format!("{OPENCODE_PROVIDER_ID}/")))
        {
            obj.remove("model");
        }
    }
    write_json_pretty(&path, &root)?;
    integration_status()
}

/// Inject OpenCode remote MCP entry for GrokGo (merge).
pub fn set_opencode_mcp_inject(enabled: bool) -> AppResult<IntegrationStatus> {
    let config = load_config()?;
    let path = opencode_config_path();
    let original = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    if !original.is_empty() {
        backup_named("opencode-config", &original, "json")?;
    }
    let mut root = if original.trim().is_empty() {
        json!({ "$schema": "https://opencode.ai/config.json" })
    } else {
        read_json_object(&path)?
    };
    if !root.is_object() {
        root = json!({ "$schema": "https://opencode.ai/config.json" });
    }
    let obj = root
        .as_object_mut()
        .ok_or_else(|| AppError::msg("opencode.json root must be an object"))?;
    if enabled {
        let url = gateway_mcp_url(&config);
        let mut entry = json!({
            "type": "remote",
            "url": url,
            "enabled": true,
            "oauth": false,
        });
        if config.require_token && !config.local_token.trim().is_empty() {
            entry["headers"] = json!({
                "Authorization": format!("Bearer {}", config.local_token.trim()),
            });
        }
        let mcp = obj.entry("mcp").or_insert_with(|| json!({}));
        if let Some(m) = mcp.as_object_mut() {
            m.remove(MCP_SERVER_ID_LEGACY);
            m.insert(MCP_SERVER_ID.to_string(), entry);
        } else {
            *mcp = json!({ MCP_SERVER_ID: entry });
        }
    } else if let Some(mcp) = obj.get_mut("mcp").and_then(|m| m.as_object_mut()) {
        for id in MCP_SERVER_IDS {
            mcp.remove(*id);
        }
        if mcp.is_empty() {
            obj.remove("mcp");
        }
    }
    write_json_pretty(&path, &root)?;
    integration_status()
}

// ── WorkBuddy ───────────────────────────────────────────────────────────────

fn workbuddy_model_is_injected(path: &std::path::Path, config: &AppConfig) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    let models = workbuddy_models_array(&v);
    models.iter().any(|m| {
        let url = m.get("url").and_then(|u| u.as_str()).unwrap_or("");
        url_points_at_gateway(url, config)
    })
}

/// Normalize WorkBuddy models.json: object `{models:[…]}` or bare array `[…]`.
fn workbuddy_models_array(v: &Value) -> Vec<Value> {
    if let Some(arr) = v.as_array() {
        return arr.clone();
    }
    if let Some(arr) = v.get("models").and_then(|m| m.as_array()) {
        return arr.clone();
    }
    Vec::new()
}

fn workbuddy_available_models(v: &Value) -> Option<Vec<String>> {
    v.get("availableModels").and_then(|a| a.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    })
}

fn client_mcp_json_is_injected(path: &std::path::Path, config: &AppConfig) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    let Some(servers) = v.get("mcpServers").and_then(|m| m.as_object()) else {
        return false;
    };
    for id in MCP_SERVER_IDS {
        if let Some(entry) = servers.get(*id) {
            let url = entry.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if url_points_at_gateway(url, config) && url.contains("mcp") {
                return true;
            }
        }
    }
    false
}

/// Inject WorkBuddy models.json entry (merge; object format preferred).
pub fn set_workbuddy_model_inject(enabled: bool) -> AppResult<IntegrationStatus> {
    let config = load_config()?;
    let path = workbuddy_models_path();
    let original = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    if !original.is_empty() {
        backup_named("workbuddy-models", &original, "json")?;
    }
    let parsed: Value = if original.trim().is_empty() {
        json!({ "models": [] })
    } else {
        serde_json::from_str(original.trim())
            .map_err(|e| AppError::msg(format!("invalid WorkBuddy models.json: {e}")))?
    };
    let mut models = workbuddy_models_array(&parsed);
    let mut available = workbuddy_available_models(&parsed);
    let model_id = {
        let m = config.default_model.trim();
        if m.is_empty() {
            "grok-4.5"
        } else {
            m
        }
    };
    // Also register grok-4.3 as a second optional model.
    let model_ids: &[&str] = if model_id == "grok-4.3" {
        &["grok-4.3", "grok-4.5"]
    } else {
        &["grok-4.5", "grok-4.3"]
    };

    if enabled {
        let url = gateway_chat_completions_url(&config);
        let api_key = if config.require_token && !config.local_token.trim().is_empty() {
            config.local_token.trim().to_string()
        } else {
            "grok-go".into()
        };
        for id in model_ids {
            let entry = json!({
                "id": id,
                "name": id,
                "vendor": "GrokGo",
                "url": url,
                "apiKey": api_key,
                "supportsToolCall": true,
                "supportsImages": true,
                "supportsReasoning": true,
                "maxInputTokens": 500_000,
                "maxOutputTokens": 65_536,
            });
            if let Some(pos) = models
                .iter()
                .position(|m| m.get("id").and_then(|x| x.as_str()) == Some(*id))
            {
                models[pos] = entry;
            } else {
                models.push(entry);
            }
            if let Some(ref mut av) = available {
                if !av.iter().any(|x| x == id) {
                    av.push((*id).to_string());
                }
            }
        }
    } else {
        models.retain(|m| {
            let url = m.get("url").and_then(|u| u.as_str()).unwrap_or("");
            !url_points_at_gateway(url, &config)
        });
        if let Some(ref mut av) = available {
            let keep: std::collections::HashSet<String> = models
                .iter()
                .filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(|s| s.to_string()))
                .collect();
            av.retain(|id| keep.contains(id));
        }
    }

    let mut out = json!({ "models": models });
    if let Some(av) = available {
        out["availableModels"] = json!(av);
    }
    write_json_pretty(&path, &out)?;
    integration_status()
}

/// Inject WorkBuddy MCP entry into **user** `mcp.json` (UI path).
/// Also strips any stale GrokGo keys from auto-generated `.mcp.json`.
pub fn set_workbuddy_mcp_inject(enabled: bool) -> AppResult<IntegrationStatus> {
    let config = load_config()?;
    let path = workbuddy_mcp_path();
    // WorkBuddy UI + agent messages reference `mcp.json` (not the connector-proxy `.mcp.json`).
    set_client_mcp_json_inject(
        &path,
        &config,
        enabled,
        "workbuddy-mcp",
        "http",
        /* include_disabled_flag */ true,
    )?;
    // Cleanup stale inject left in the connector-proxy file from earlier builds.
    let _ = remove_client_mcp_ids_from(&workbuddy_mcp_dot_path());
    integration_status()
}

// ── Cursor ──────────────────────────────────────────────────────────────────

/// Inject Cursor MCP only (BYOK model is copy-only — Key lives in secure storage).
pub fn set_cursor_mcp_inject(enabled: bool) -> AppResult<IntegrationStatus> {
    let config = load_config()?;
    let path = cursor_mcp_path();
    set_client_mcp_json_inject(
        &path,
        &config,
        enabled,
        "cursor-mcp",
        "streamable-http",
        /* include_disabled_flag */ false,
    )?;
    integration_status()
}

/// Shared merge inject/remove for Cursor / WorkBuddy style `mcpServers` JSON.
fn set_client_mcp_json_inject(
    path: &std::path::Path,
    config: &AppConfig,
    enabled: bool,
    backup_label: &str,
    transport_type: &str,
    include_disabled_flag: bool,
) -> AppResult<()> {
    let original = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    if !original.is_empty() {
        backup_named(backup_label, &original, "json")?;
    }
    let mut root = if original.trim().is_empty() {
        json!({ "mcpServers": {} })
    } else {
        serde_json::from_str(original.trim())
            .map_err(|e| AppError::msg(format!("invalid MCP JSON {}: {e}", path.display())))?
    };
    if !root.is_object() {
        root = json!({ "mcpServers": {} });
    }
    let obj = root
        .as_object_mut()
        .ok_or_else(|| AppError::msg("MCP config root must be an object"))?;
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    let servers = if let Some(s) = servers.as_object_mut() {
        s
    } else {
        *servers = json!({});
        servers.as_object_mut().unwrap()
    };

    if enabled {
        let url = gateway_mcp_url(config);
        let mut entry = json!({
            "type": transport_type,
            "url": url,
        });
        if include_disabled_flag {
            entry["disabled"] = json!(false);
        }
        if config.require_token && !config.local_token.trim().is_empty() {
            entry["headers"] = json!({
                "Authorization": format!("Bearer {}", config.local_token.trim()),
            });
        }
        servers.remove(MCP_SERVER_ID_LEGACY);
        servers.insert(MCP_SERVER_ID.to_string(), entry);
    } else {
        for id in MCP_SERVER_IDS {
            servers.remove(*id);
        }
    }
    write_json_pretty(path, &root)?;
    Ok(())
}

/// Best-effort: drop grok-go / grok-proxy keys from a mcpServers JSON file.
fn remove_client_mcp_ids_from(path: &std::path::Path) -> AppResult<()> {
    if !path.is_file() {
        return Ok(());
    }
    let original = fs::read_to_string(path)?;
    let mut root: Value = match serde_json::from_str(original.trim()) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let Some(servers) = root
        .get_mut("mcpServers")
        .and_then(|m| m.as_object_mut())
    else {
        return Ok(());
    };
    let mut changed = false;
    for id in MCP_SERVER_IDS {
        if servers.remove(*id).is_some() {
            changed = true;
        }
    }
    if changed {
        write_json_pretty(path, &root)?;
    }
    Ok(())
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

/// True when `url` points at this gateway (any path: `/v1`, `/mcp`, `/v1/chat/completions`, …).
fn url_points_at_gateway(url: &str, config: &AppConfig) -> bool {
    let u = url.trim().trim_end_matches('/');
    if u.is_empty() {
        return false;
    }
    let expected = gateway_base_url(config).trim_end_matches('/').to_string();
    if u == expected || u.starts_with(&format!("{expected}/")) {
        return true;
    }
    let port = config.actual_port;
    let host_hit = u.contains(&format!("127.0.0.1:{port}"))
        || u.contains(&format!("localhost:{port}"))
        || u.contains(&format!("[::1]:{port}"))
        || u.contains(&format!("0.0.0.0:{port}"));
    if !host_hit {
        // LAN inject may use a real interface IP.
        if let Ok(ip) = local_ip_address::local_ip() {
            if u.contains(&format!("{ip}:{port}")) {
                return true;
            }
        }
        return false;
    }
    true
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
    ranked_accounts_for_grok_build_session(store).into_iter().next()
}

/// Seconds before JWT / expires_at that Grok CLI may still treat as valid.
/// Grok docs mention `GROK_AUTH_EARLY_INVALIDATION_SECS` (default often ~300).
const GROK_AUTH_EARLY_INVALIDATION_SECS: i64 = 300;

/// Start a background loop that keeps `~/.grok/auth.json` fresh **only while**
/// Grok Build routing points at this gateway. Idempotent.
pub fn start_grok_build_auth_maintainer() {
    if GROK_AUTH_MAINTAINER_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tracing::info!(
            target: "integrations",
            interval_secs = GROK_AUTH_MAINTAIN_INTERVAL.as_secs(),
            "Grok Build auth maintainer started"
        );
        tokio::time::sleep(GROK_AUTH_MAINTAIN_INITIAL_DELAY).await;
        loop {
            if let Err(err) = maintain_grok_build_session_auth().await {
                tracing::warn!(
                    target: "integrations",
                    error = %err,
                    "Grok Build auth maintain tick failed"
                );
            }
            tokio::time::sleep(GROK_AUTH_MAINTAIN_INTERVAL).await;
        }
    });
}

/// One maintenance tick: if routing is injected, refresh + userinfo-probe a pool
/// account and write only after the IdP confirms the token is valid.
pub async fn maintain_grok_build_session_auth() -> AppResult<()> {
    let config = load_config()?;
    let path = grok_build_config_path();
    if !path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&path).unwrap_or_default();
    if !grok_build_is_injected(&raw, &config) {
        return Ok(());
    }
    sync_grok_build_session_auth_async(&config, /*require_success*/ false).await
}

/// Sync path for inject (sync command handlers). Blocks on async work.
fn sync_grok_build_session_auth(config: &AppConfig) -> AppResult<()> {
    // Prefer current runtime when already inside Tauri/tokio; fall back to a
    // dedicated runtime if called from a pure sync context.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| {
            handle.block_on(sync_grok_build_session_auth_async(config, true))
        });
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::msg(format!("tokio runtime for Grok Build auth sync: {e}")))?;
    rt.block_on(sync_grok_build_session_auth_async(config, true))
}

/// Refresh the best pool account(s), **probe OIDC userinfo**, then write
/// `~/.grok/auth.json` only when the IdP accepts the access token.
///
/// - `require_success`: inject path — surface errors to the user
/// - maintainer path: log and leave existing file alone on failure
async fn sync_grok_build_session_auth_async(
    config: &AppConfig,
    require_success: bool,
) -> AppResult<()> {
    let _guard = GROK_AUTH_WRITE_LOCK.lock().await;

    let store = crate::config::load_auth()?;
    let candidates = ranked_accounts_for_grok_build_session(&store);
    if candidates.is_empty() {
        let msg = "no credentialed OAuth account available to sync into ~/.grok/auth.json";
        tracing::warn!(target: "integrations", "{msg}");
        if require_success {
            return Err(AppError::msg(
                "没有可用的 OAuth 账号可同步到 Grok Build。请先在账号页登录。",
            ));
        }
        return Ok(());
    }

    let mut last_err: Option<String> = None;
    for account in candidates.into_iter().take(GROK_AUTH_MAX_ACCOUNT_TRIES) {
        match try_refresh_probe_and_write_session(config, account).await {
            Ok(email) => {
                tracing::info!(
                    target: "integrations",
                    %email,
                    path = %grok_build_auth_path().display(),
                    "Grok Build auth.json updated after userinfo validation"
                );
                return Ok(());
            }
            Err(err) => {
                tracing::warn!(
                    target: "integrations",
                    error = %err,
                    "candidate account failed Grok Build auth sync"
                );
                last_err = Some(err.to_string());
            }
        }
    }

    let detail = last_err.unwrap_or_else(|| "all candidates failed".into());
    if require_success {
        return Err(AppError::msg(format!(
            "无法把有效会话写入 ~/.grok/auth.json：{detail}\n\
             请在账号页重新登录后再开启 Grok Build，或先用 `grok login` 登录。"
        )));
    }
    // Maintainer: keep whatever is already on disk.
    Ok(())
}

fn ranked_accounts_for_grok_build_session(store: &crate::config::AuthStore) -> Vec<Account> {
    let mut list: Vec<Account> = store
        .accounts
        .iter()
        .filter(|a| a.enabled && a.is_credentialed())
        .filter(|a| !matches!(a.auth_kind, crate::config::AccountAuthKind::Sso))
        .cloned()
        .collect();
    list.sort_by_key(|a| std::cmp::Reverse(score_account_for_grok_build_session(a)));
    list
}

/// Refresh → userinfo probe → write. Never writes without a Valid probe.
async fn try_refresh_probe_and_write_session(
    config: &AppConfig,
    mut account: Account,
) -> AppResult<String> {
    // Force refresh so we do not probe a stale access token from disk.
    crate::auth::refresh_account(config, &mut account)
        .await
        .map_err(|err| {
            AppError::msg(format!(
                "刷新账号 {} 失败：{err}",
                account
                    .email
                    .clone()
                    .unwrap_or_else(|| account.name.clone())
            ))
        })?;

    // Persist refreshed tokens without full-store overwrite (preserves fresher quota).
    let _ = crate::config::apply_account_update(&account);

    let access = account
        .access_token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| AppError::msg("empty access token after refresh"))?;
    let refresh = account
        .refresh_token
        .clone()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| AppError::msg("missing refresh token after refresh"))?;

    // Live validation against auth.x.ai — only write if IdP accepts the token.
    let probe = crate::auth::probe_access_token_userinfo(&access).await?;
    match &probe {
        TokenProbe::Valid { email, .. } => {
            tracing::info!(
                target: "integrations",
                account = %account.id,
                ?email,
                "userinfo accepted access token"
            );
        }
        TokenProbe::Invalid { status, detail } => {
            return Err(AppError::msg(format!(
                "userinfo 拒绝账号 {} 的 token (HTTP {status}): {detail}",
                account
                    .email
                    .clone()
                    .unwrap_or_else(|| account.name.clone())
            )));
        }
        TokenProbe::Unreachable { detail } => {
            return Err(AppError::msg(format!(
                "无法校验 token（userinfo 不可达）：{detail}"
            )));
        }
    }

    // Reject near-expiry JWT even if userinfo briefly accepted it.
    if let Some(exp) = crate::auth::jwt_payload(&access)
        .and_then(|p| p.get("exp").and_then(|x| x.as_i64()))
        .and_then(|secs| chrono::DateTime::<Utc>::from_timestamp(secs, 0))
    {
        let horizon = Utc::now() + chrono::Duration::seconds(GROK_AUTH_EARLY_INVALIDATION_SECS);
        if exp <= horizon {
            return Err(AppError::msg(format!(
                "access token 将在 {} 前失效，拒绝写入",
                exp.to_rfc3339()
            )));
        }
    }

    let email = write_grok_build_auth_entry(config, &account, &access, &refresh, &probe)?;
    Ok(email)
}

/// Merge validated credentials into `~/.grok/auth.json` (preserves profile fields).
fn write_grok_build_auth_entry(
    config: &AppConfig,
    account: &Account,
    access: &str,
    refresh: &str,
    probe: &TokenProbe,
) -> AppResult<String> {
    let payload = crate::auth::jwt_payload(access).unwrap_or_else(|| json!({}));
    let client_id = config.effective_xai_client_id().to_string();
    let principal_id = payload
        .get("principal_id")
        .or_else(|| payload.get("sub"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if principal_id.is_empty() {
        return Err(AppError::msg(
            "token missing principal_id/sub; refuse writing Grok Build auth.json",
        ));
    }
    let team_id = payload
        .get("team_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let probe_email = match probe {
        TokenProbe::Valid { email, .. } => email.clone(),
        _ => None,
    };
    let email = account
        .email
        .clone()
        .or(probe_email)
        .or_else(|| {
            payload
                .get("email")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| account.name.clone());
    let expires_at = payload
        .get("exp")
        .and_then(|v| v.as_i64())
        .and_then(|secs| chrono::DateTime::<Utc>::from_timestamp(secs, 0))
        .map(|t| t.to_rfc3339())
        .or_else(|| account.expires_at.map(|t| t.to_rfc3339()))
        .unwrap_or_else(|| (Utc::now() + chrono::Duration::hours(6)).to_rfc3339());
    let tier = payload.get("tier").and_then(|v| v.as_i64());

    let auth_path = grok_build_auth_path();
    let mut root: serde_json::Map<String, Value> = fs::read_to_string(&auth_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let entry_key = format!("https://auth.x.ai::{client_id}");
    let mut entry = root
        .get(&entry_key)
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    entry.insert("key".into(), json!(access));
    entry.insert("auth_mode".into(), json!("oidc"));
    if !entry.contains_key("create_time") {
        entry.insert("create_time".into(), json!(Utc::now().to_rfc3339()));
    }
    entry.insert("user_id".into(), json!(principal_id.clone()));
    entry.insert("email".into(), json!(email.clone()));
    entry.insert(
        "principal_type".into(),
        json!(payload
            .get("principal_type")
            .and_then(|v| v.as_str())
            .unwrap_or("User")),
    );
    entry.insert("principal_id".into(), json!(principal_id));
    if let Some(tid) = team_id {
        entry.insert("team_id".into(), json!(tid));
    }
    entry.insert("coding_data_retention_opt_out".into(), json!(true));
    entry.insert("refresh_token".into(), json!(refresh));
    entry.insert("expires_at".into(), json!(expires_at));
    entry.insert("oidc_issuer".into(), json!("https://auth.x.ai"));
    entry.insert("oidc_client_id".into(), json!(client_id));

    root.insert(entry_key, Value::Object(entry));
    if let Some(parent) = auth_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = auth_path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&Value::Object(root))?)?;
    fs::rename(&tmp, &auth_path).map_err(|e| {
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
        "wrote validated session into Grok Build auth.json"
    );
    Ok(email)
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

/// MCP image tools: documented as fallback only — Codex native imagegen wins.
fn is_mcp_image_tool(id: &str) -> bool {
    matches!(id, "image_gen" | "image_generate" | "image_edit")
}

/// Full guide body (versioned) written to `~/.grok-go/agents-guide.md`.
/// Only documents tools currently enabled in `AppConfig.mcp_enabled_tools`.
/// Runtime inject file for Codex — separate from the repo project `AGENTS.md` / llm-wiki.
fn agents_guide_file_body(config: &AppConfig) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let port = config.actual_port;
    let mcp_url = format!("http://127.0.0.1:{port}/mcp");
    let api_base = format!("http://127.0.0.1:{port}/v1");
    let health_url = format!("http://127.0.0.1:{port}/health");

    let mut primary: Vec<&str> = Vec::new();
    let mut image_fallback: Vec<&str> = Vec::new();
    for id in crate::config::default_mcp_tool_ids() {
        if !config.mcp_tool_enabled(id) {
            continue;
        }
        // Prefer documenting image_gen; skip alias when both are enabled.
        if *id == "image_generate" && config.mcp_tool_enabled("image_gen") {
            continue;
        }
        if is_mcp_image_tool(id) {
            image_fallback.push(*id);
        } else {
            primary.push(*id);
        }
    }

    let tools_http = format!("{api_base}/tools");
    let mut out = String::new();
    out.push_str(&format!(
        "# GrokGo 工具指引\n\n\
         > 版本：{version}  \n\
         > 本文件由 GrokGo 维护，随软件版本与「已启用 MCP 工具」同步更新。请勿手改（重新注入会覆盖）。\n\
         > 与仓库开发用 `AGENTS.md` / `llm-wiki` 无关；此处只服务运行时 MCP / 工具分流。\n\n\
         ## 决策树（按顺序判断）\n\n\
         ### 分支 A — 本轮 tools 已注入（含 `x_search` / `image_gen` / `mcp__grok-go__*`）\n\n\
         - **直接 function_call / tool_use**，不要绕 shell。\n\
         - 图片：优先会话里的 `image_gen`（GrokGo 注入或 MCP）；内置 `imagegen` 不稳时再换 MCP。\n\n\
         ### 分支 B — health 通，但本轮 **没有** 上述工具\n\n\
         - **只允许**下列旁路（禁止发明 web_search / 浏览器 / RapidAPI 顶替）：\n\
         1. **简易 Tools HTTP**（推荐）:\n\
         ```bash\n\
         curl -sS -X POST '{tools_http}/x_search' \\\n\
           -H 'Authorization: Bearer <localToken>' \\\n\
           -H 'Content-Type: application/json' \\\n\
           -d '{{\"query\":\"example\"}}'\n\
         ```\n\
         2. **MCP JSON-RPC** 固定模板:\n\
         ```bash\n\
         curl -sS -X POST '{mcp_url}' \\\n\
           -H 'Authorization: Bearer <localToken>' \\\n\
           -H 'Content-Type: application/json' \\\n\
           -d '{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"x_search\",\"arguments\":{{\"query\":\"example\"}}}}}}'\n\
         ```\n\
         - 鉴权：Bearer = GrokGo 本地 token（`~/.codex/config.toml` → `[mcp_servers.grok-go].http_headers`）。\n\
         - 先 `GET {health_url}`；再调工具。参数以本文件 / `tools/list` 为准。\n\n\
         ### 分支 C — `{health_url}` 失败或网关不可达\n\n\
         - **可以**降级到其他手段，但**必须在回复中说明**：网关挂了 / 连接拒绝 / 超时。\n\
         - 不要静默改用 web_search 假装走了 GrokGo。\n\n\
         ### 错误语义\n\n\
         - 响应可能含 `error.code`：`GATEWAY_DOWN` / `UPSTREAM_TIMEOUT` / `TOOL_TIMEOUT` / `TOOL_FAILED`。\n\
         - `TOOL_TIMEOUT`（尤其视频）**不是**彻底失败：检查 `job_id` / `poll`（`GET {api_base}/videos/{{id}}`）或 `~/.grok-go/artifacts/`。\n\n\
         ## 媒体约定\n\n\
         - 输入：`https://` / `data:` / **本地绝对路径** / `file://`。\n\
         - 返回：`~/.grok-go/artifacts/` 绝对路径 + `markdown`；**不要**展示远程 CDN URL。\n\n\
         ## 策略矩阵（仿冒 Build vs 原生）\n\n\
         | 客户端 | 聊天上游 | 注入 x_search/image_gen | empty-completion |\n\
         |---|---|---|---|\n\
         | Native Grok Build TUI | cli-chat-proxy | 否 | 否 |\n\
         | Codex 仿冒 Build | cli-chat-proxy | 是 | 是 |\n\
         | Codex console | api.x.ai | 是 | 是 |\n\n"
    ));

    if primary.is_empty() && image_fallback.is_empty() {
        out.push_str("## 当前已启用工具\n\n");
        out.push_str("（当前未启用任何 MCP 工具。可在 GrokGo → 集成 → MCP 中开启。）\n\n");
    } else {
        out.push_str("## 当前应优先走 GrokGo MCP 的工具\n\n");
        if primary.is_empty() {
            out.push_str(
                "（当前未启用非图片类 MCP 工具。可在 GrokGo → 集成 → MCP 中开启 `x_search` / 视频等。）\n\n",
            );
        } else {
            for id in &primary {
                out.push_str(&tool_guide_section(id));
                out.push('\n');
            }
        }

        if !image_fallback.is_empty() {
            out.push_str(
                "## MCP 图片备选（非默认）\n\n\
                 > 生图默认用 Codex 内置 `imagegen`/`image_gen`。以下仅在原生不可用时使用。\n\n",
            );
            for id in &image_fallback {
                out.push_str(&tool_guide_section(id));
                out.push('\n');
            }
        }
    }

    out.push_str(&format!(
        "## 健康检查与端点\n\n\
         ```bash\n\
         curl -s {health_url}\n\
         ```\n\n\
         Responses API Base：`{api_base}`  \n\
         MCP：`{mcp_url}`  \n\
         产物目录：`~/.grok-go/artifacts/`\n"
    ));
    out
}

fn tool_guide_section(id: &str) -> String {
    match id {
        "x_search" => "### `x_search`（必须走 GrokGo MCP）\n\
- 必填：`query`\n\
- 可选：`allowed_handles` `excluded_handles` `from_date` `to_date`（YYYY-MM-DD）\n\
- **禁止**用 web_search / Chrome / twitter241 顶替；参数以 `tools/list` 为准\n"
            .into(),
        "image_gen" => "### `image_gen`（`image_generate` 同义别名）— MCP 备选\n\
- **非默认**：优先 Codex 内置 `imagegen`/`image_gen`\n\
- 必填：`prompt`\n\
- 可选：`n`(1–4) `model` `size` `quality`(low|medium|high)\n"
            .into(),
        "image_generate" => "### `image_generate`（`image_gen` 别名）— MCP 备选\n\
- **非默认**：优先 Codex 内置 `imagegen`/`image_gen`\n\
- 必填：`prompt`\n\
- 可选：`n`(1–4) `model` `size` `quality`(low|medium|high)\n"
            .into(),
        "image_edit" => "### `image_edit` — MCP 备选\n\
- **非默认**：优先 Codex 内置图片编辑能力（若有）\n\
- 必填：`prompt` + `image_url`（URL 或本地路径）\n\
- 可选：`model`\n"
            .into(),
        "video_generate" => "### `video_generate`（必须走 GrokGo MCP；文生 / 图生 / 多图参考）\n\
- 必填：`prompt`\n\
- 模式（三选一）：\n\
  1. 文生视频：仅 `prompt`\n\
  2. 图生视频：`prompt` + `image_url`（首帧）\n\
  3. 多图参考：`prompt` + `reference_image_urls`（1–7，勿与 `image_url` 同用）\n\
- 可选：`duration`(1–15) `aspect_ratio`(1:1|16:9|9:16|4:3|3:4|3:2|2:3) `resolution`(480p|720p|1080p) `model`\n\
- 示例：`{\"prompt\":\"轻推镜头，微风吹动毛发\",\"image_url\":\"/abs/path.png\",\"duration\":6}`\n\
- 参数以 `tools/list` 为准；**禁止**翻仓库猜参\n"
            .into(),
        "video_edit" => "### `video_edit`（必须走 GrokGo MCP）\n\
- 必填：`prompt` + `video_url`（URL 或本地路径）\n\
- 可选：`model`\n\
- 注意：编辑不支持自定义 duration/aspect_ratio\n\
- 参数以 `tools/list` 为准\n"
            .into(),
        other => format!(
            "### `{other}`（必须走 GrokGo MCP）\n\
- 参数以 MCP `tools/list` 为准；先 list 再 call\n"
        ),
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
/// Keeps hard routing rules inline so agents see them without opening the full guide.
fn agents_guide_ref_block(guide_abs: &str, mcp_port: u16) -> String {
    let mcp_url = format!("http://127.0.0.1:{mcp_port}/mcp");
    format!(
        "{AGENTS_GUIDE_START}\n\
         <!-- {AGENTS_GUIDE_REF_ANCHOR} -->\n\
         - **GrokGo 强制分流**（完整说明：`{guide_abs}`，随软件更新；调用前请读）：\n\
           - **图片**：优先 Codex 内置 `imagegen`/`image_gen`；**不要**默认走 GrokGo MCP 的 `image_gen`/`image_generate`/`image_edit`。\n\
           - **其余已启用能力**（尤其 `x_search`、`video_generate`、`video_edit`）：**必须**优先 GrokGo MCP — `{mcp_url}` + `Authorization: Bearer <localToken>`（见 `~/.codex/config.toml` `[mcp_servers.grok-go]`）；先 `tools/list` 再 `tools/call`。\n\
           - **禁止**因会话未注入 `mcp__grok-go__*`、无原生 `x_search`、或 `tool_search` 失效就改用 `web_search` / Chrome / twitter241 / 翻仓库猜参数。\n\
           - **仅当** `/health` 或 MCP 明确失败时可降级，并说明原因。参数以 `tools/list` 与上述 guide 为准。\n\
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
fn upsert_agents_guide_ref_content(existing: &str, guide_abs: &str, mcp_port: u16) -> String {
    let block = agents_guide_ref_block(guide_abs, mcp_port);
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
    let config = load_config()?;
    let guide = ensure_agents_guide_file_with(&config)?;
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
    let next = upsert_agents_guide_ref_content(&original, &guide_abs, config.actual_port);
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

    // Codex sessions key off `model_provider` id in config.toml / session_meta.
    // Import must NOT overwrite the user's current provider row — copy a new GrokGo
    // slot that reuses the same model_provider id so history stays continuous.
    let active_codex = if app_type == "codex" {
        read_active_codex_provider()
    } else {
        None
    };
    let (provider_key, source_display) = if app_type == "codex" {
        match active_codex.as_ref() {
            Some(a) => (a.id.clone(), a.display_name.clone()),
            None => ("grok-go".into(), "GrokGo".into()),
        }
    } else {
        ("grok-go".into(), "GrokGo".into())
    };

    // New/updated GrokGo *copy* display name — never steal the original provider's name.
    let name = if app_type == "codex" && provider_key != "grok-go" {
        format!("GrokGo · {source_display}")
    } else {
        "GrokGo".to_string()
    };

    let notes = if app_type == "claude" {
        "由 GrokGo 同步（Claude Code / Anthropic Messages）".to_string()
    } else if include_mcp {
        format!("由 GrokGo 同步（含 MCP；复制槽；model_provider={provider_key}）")
    } else {
        format!("由 GrokGo 同步（复制槽；model_provider={provider_key}）")
    };

    let website_url = grokgo_provider_website_url(app_type, &config);
    // Only touch *our* previous GrokGo import copies — never the live third-party row.
    let existing: Option<CcSwitchProviderRow> = if app_type == "codex" {
        find_our_codex_grokgo_copy(&conn, &provider_key)?
    } else {
        match find_existing_grokgo_provider_for_app(&conn, app_type)? {
            Some((id, _)) => {
                let display_name: String = conn
                    .query_row(
                        "SELECT name FROM providers WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .unwrap_or_else(|_| "GrokGo".into());
                Some(CcSwitchProviderRow { id, display_name })
            }
            None => None,
        }
    };

    let settings = if app_type == "claude" {
        claude_provider_settings_config(&config)
    } else {
        // Keep model_provider id = currently applied Codex provider (session key).
        provider_settings_config_for_id(&config, include_mcp, &provider_key, &name)
    };
    let settings_text = serde_json::to_string(&settings)?;
    let now = Utc::now().timestamp_millis();

    let (action_zh, provider_id) = if let Some(row) = existing {
        // Refresh our previous copy only.
        conn.execute(
            r#"
            UPDATE providers
            SET name = ?1,
                settings_config = ?2,
                notes = ?3,
                website_url = ?4,
                category = 'custom',
                icon = NULL,
                icon_color = NULL
            WHERE id = ?5 AND app_type = ?6
            "#,
            params![name, settings_text, notes, website_url, row.id, app_type],
        )
        .map_err(|e| AppError::msg(format!("更新 CC Switch 中的 GrokGo 副本失败：{e}")))?;
        let removed = remove_duplicate_grokgo_providers_for_app(&conn, app_type, &row.id).unwrap_or(0);
        let mut action = if app_type == "codex" && provider_key != "grok-go" {
            format!(
                "已更新 GrokGo 副本「{name}」（model_provider={provider_key} 与当前一致；未改动原服务商配置）"
            )
        } else {
            "已更新".to_string()
        };
        if removed > 0 {
            action = format!("{action}（并清理了 {removed} 条重复的 GrokGo 配置）");
        }
        (action, row.id)
    } else {
        // Always INSERT a new copy — do not overwrite is_current / third-party rows.
        let id = Uuid::new_v4().to_string();
        conn.execute(
            r#"
            INSERT INTO providers (
              id, app_type, name, settings_config, website_url, category, created_at,
              sort_index, notes, icon, icon_color, meta, is_current, in_failover_queue,
              cost_multiplier, limit_daily_usd, limit_monthly_usd, provider_type
            ) VALUES (?1,?2,?3,?4,?5,'custom',?6,NULL,?7,NULL,NULL,'{}',0,0,'1.0',NULL,NULL,NULL)
            "#,
            params![id, app_type, name, settings_text, website_url, now, notes],
        )
        .map_err(|e| AppError::msg(format!("写入 CC Switch 失败：{e}")))?;
        let action = if app_type == "codex" && provider_key != "grok-go" {
            format!(
                "已复制新增「{name}」（model_provider={provider_key} 与当前一致；原服务商配置未改动）"
            )
        } else {
            "已新增".to_string()
        };
        (action, id)
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
        "{action_zh}\n\
         请在 CC Switch 中切换到该副本（不要改原服务商）。\n\
         model_provider = \"{provider_key}\"（与当前 Codex 会话标识一致）\n\
         已挂载可用模型：grok-4.5 / grok-4.3；默认：{}。{}\n\
         网关：http://{}:{}/v1。{}",
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

/// Active Codex provider identity from `~/.codex/config.toml`.
#[derive(Debug, Clone)]
struct ActiveCodexProvider {
    /// `model_provider = "…"` value (session_meta key).
    id: String,
    /// Prefer `[model_providers.<id>].name`, else the id itself.
    display_name: String,
}

/// Target row in CC Switch `providers` table.
#[derive(Debug, Clone)]
struct CcSwitchProviderRow {
    id: String,
    display_name: String,
}

/// Read current Codex `model_provider` so import can preserve session continuity.
fn read_active_codex_provider() -> Option<ActiveCodexProvider> {
    let path = codex_config_path();
    let raw = fs::read_to_string(&path).ok()?;
    let doc = raw.parse::<toml_edit::DocumentMut>().ok()?;
    let id = doc
        .get("model_provider")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    if !is_safe_codex_provider_id(&id) {
        return None;
    }
    let display_name = doc
        .get("model_providers")
        .and_then(|i| i.as_table())
        .and_then(|t| t.get(id.as_str()))
        .and_then(|i| i.as_table())
        .and_then(|t| t.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(id.as_str())
        .to_string();
    Some(ActiveCodexProvider { id, display_name })
}

fn is_safe_codex_provider_id(id: &str) -> bool {
    let t = id.trim();
    if t.is_empty() || t.len() > 64 {
        return false;
    }
    // Bare TOML keys + common CC Switch ids (alnum, -, _). Digits-only allowed (quoted in TOML).
    t.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Find *our* previous GrokGo import copy for Codex (never the user's live provider).
///
/// Matches rows we wrote (`notes` / name prefix) that already carry `provider_key`
/// as `model_provider`, so re-import updates the copy instead of creating unlimited clones.
fn find_our_codex_grokgo_copy(
    conn: &Connection,
    provider_key: &str,
) -> AppResult<Option<CcSwitchProviderRow>> {
    let like_provider = format!("%model_provider = \"{provider_key}\"%");
    let like_table = format!("%model_providers.{provider_key}%");
    // Prefer an explicit GrokGo-owned row that already uses this model_provider id.
    if let Some(row) = conn
        .query_row(
            r#"
            SELECT id, name FROM providers
            WHERE app_type = 'codex'
              AND (
                name = 'GrokGo'
                OR name LIKE 'GrokGo · %'
                OR notes LIKE '%由 GrokGo 同步%'
                OR notes LIKE '%Imported from GrokGo%'
              )
              AND (
                settings_config LIKE ?1
                OR settings_config LIKE ?2
                OR ?3 = 'grok-go'
              )
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            params![like_provider, like_table, provider_key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| AppError::msg(format!("查询 GrokGo 副本失败：{e}")))?
    {
        return Ok(Some(CcSwitchProviderRow {
            id: row.0,
            display_name: row.1,
        }));
    }

    // Fallback: any legacy GrokGo row (will be rewritten to current provider_key).
    if provider_key == "grok-go" {
        if let Some((id, _)) = find_existing_grokgo_provider_for_app(conn, "codex")? {
            let name: String = conn
                .query_row(
                    "SELECT name FROM providers WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .unwrap_or_else(|_| "GrokGo".into());
            return Ok(Some(CcSwitchProviderRow {
                id,
                display_name: name,
            }));
        }
    }
    Ok(None)
}

/// Website URL written into CC Switch for GrokGo providers.
/// Local gateway — never reuse third-party marketing URLs (DeepSeek / Kimi / …).
fn grokgo_provider_website_url(app_type: &str, config: &AppConfig) -> String {
    if app_type == "claude" {
        anthropic_base_url(config)
    } else {
        gateway_base_url(config)
    }
}

/// Find existing GrokGo provider for `app_type` (`codex` | `claude`).
///
/// Identity is intentionally strict:
/// - **name = GrokGo** (primary, both apps)
/// - **notes** written by our sync
/// - **Codex** settings fingerprints (`model_providers.grok-go`, …)
/// - **Claude** only when `settings_config` already contains *this* local gateway
///   base URL. Bare `ANTHROPIC_BASE_URL` must NOT match — every Claude provider
///   in CC Switch has that key (DeepSeek, Kimi, Z.ai, …).
fn find_existing_grokgo_provider_for_app(
    conn: &Connection,
    app_type: &str,
) -> AppResult<Option<(String, i64)>> {
    let config = load_config().unwrap_or_default();
    let local_anthropic = anthropic_base_url(&config);
    // Containment match on our gateway root (no `/v1`). Escape LIKE metacharacters.
    let local_base_like = format!(
        "%{}%",
        local_anthropic
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );

    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, COALESCE(created_at, 0), COALESCE(is_current, 0)
            FROM providers
            WHERE app_type = ?1
              AND (
                name = 'GrokGo'
                OR notes LIKE '%由 GrokGo 同步%'
                OR notes LIKE '%Imported from GrokGo%'
                OR (
                  ?1 = 'codex'
                  AND (
                    settings_config LIKE '%model_providers.grok-go%'
                    OR settings_config LIKE '%"name": "grok-go"%'
                    OR settings_config LIKE '%name = "grok-go"%'
                    OR settings_config LIKE '%name = "GrokGo"%'
                  )
                )
                OR (
                  ?1 = 'claude'
                  AND settings_config LIKE ?2 ESCAPE '\'
                )
              )
            ORDER BY
              CASE WHEN name = 'GrokGo' THEN 0 ELSE 1 END,
              is_current DESC,
              created_at DESC
            "#,
        )
        .map_err(|e| AppError::msg(format!("查询 CC Switch 配置失败：{e}")))?;
    let mut rows = stmt
        .query(params![app_type, local_base_like])
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
/// Only rows that are clearly ours (name / our notes) — never third-party providers.
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
    provider_settings_config_for_id(config, include_mcp, "grok-go", "GrokGo")
}

/// Build CC Switch `settings_config` for Codex, using a specific provider id.
///
/// `provider_id` becomes both `model_provider = "…"` and the
/// `[model_providers.<id>]` table key — this is what Codex stores on sessions.
fn provider_settings_config_for_id(
    config: &AppConfig,
    include_mcp: bool,
    provider_id: &str,
    display_name: &str,
) -> serde_json::Value {
    use crate::config::{cc_switch_import_default_model, xai_model_default_reasoning_effort};
    let host = if config.lan_enabled {
        local_lan_host()
    } else {
        "127.0.0.1".into()
    };
    let base = format!("http://{}:{}/v1", host, config.actual_port);
    let import_model = cc_switch_import_default_model(&config.default_model);
    let pid = if is_safe_codex_provider_id(provider_id) {
        provider_id.trim()
    } else {
        "grok-go"
    };
    let dname = {
        let t = display_name.trim();
        if t.is_empty() {
            pid
        } else {
            t
        }
    };
    // Quote TOML table keys that are not bare keys (e.g. digits-only "98").
    let table_key = toml_provider_table_key(pid);
    let mut toml = format!(
        "model_provider = \"{pid}\"\n\
         model = \"{import_model}\"\n"
    );
    if let Some(effort) = xai_model_default_reasoning_effort(import_model) {
        toml.push_str(&format!("model_reasoning_effort = \"{effort}\"\n"));
    }
    // Escape display name for TOML basic string.
    let dname_esc = dname.replace('\\', "\\\\").replace('"', "\\\"");
    toml.push_str(&format!(
        "disable_response_storage = true\n\
         \n\
         [model_providers.{table_key}]\n\
         name = \"{dname_esc}\"\n\
         wire_api = \"responses\"\n\
         requires_openai_auth = true\n\
         base_url = \"{base}\"\n\
         experimental_bearer_token = \"{}\"\n",
        config.local_token
    ));
    if include_mcp {
        // Keep a stable MCP server id so inject/remove paths stay consistent.
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

/// TOML table key for `[model_providers.…]` — quote when not a bare key.
fn toml_provider_table_key(id: &str) -> String {
    let bare = id
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_')
        .unwrap_or(false)
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        id.to_string()
    } else {
        format!("\"{}\"", id.replace('"', ""))
    }
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
    let (key, name) = match read_active_codex_provider() {
        Some(a) => (a.id, a.display_name),
        None => ("grok-go".into(), "GrokGo".into()),
    };
    json!({
        "app_type": "codex",
        "name": name,
        "settings_config": provider_settings_config_for_id(config, include_mcp, &key, &name)
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
        let out = upsert_agents_guide_ref_content("# User notes\n\nhello", guide, 8787);
        assert!(out.contains("# User notes"));
        assert!(out.contains("hello"));
        assert!(out.contains(AGENTS_GUIDE_START));
        assert!(out.contains(AGENTS_GUIDE_REF_ANCHOR));
        assert!(out.contains(guide));
        // Hard routing rules stay in the short ref; full tool catalog stays in guide file.
        assert!(out.contains("强制分流"));
        assert!(out.contains("http://127.0.0.1:8787/mcp"));
        assert!(out.contains("tools/list"));
        assert!(out.contains("web_search"));
        assert!(!out.contains("## 强制分流（必读）"));
        assert!(!out.contains("### `x_search`"));
        assert_eq!(out.matches(AGENTS_GUIDE_START).count(), 1);
    }

    #[test]
    fn upsert_replaces_existing_ref_only() {
        let guide = "/tmp/agents-guide.md";
        let first = upsert_agents_guide_ref_content("user A", guide, 8787);
        let second =
            upsert_agents_guide_ref_content(&first.replace("user A", "user B"), guide, 8787);
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
        let content = upsert_agents_guide_ref_content("", guide, 8787);
        assert!(content.contains(AGENTS_GUIDE_REF_ANCHOR));
        assert!(content.contains(AGENTS_GUIDE_START));
    }

    #[test]
    fn agents_guide_lists_only_enabled_tools() {
        let mut cfg = AppConfig::default();
        cfg.actual_port = 8787;
        cfg.mcp_enabled_tools = Some(vec!["x_search".into(), "image_gen".into()]);
        let body = agents_guide_file_body(&cfg);
        // Enabled tools get full sections; disabled tools only appear in routing policy text.
        assert!(body.contains("### `x_search`"));
        assert!(body.contains("### `image_gen`"));
        assert!(!body.contains("### `video_generate`"));
        assert!(!body.contains("### `video_edit`"));
        assert!(!body.contains("### `image_edit`"));
        assert!(body.contains("与仓库开发用"));
        // O-03 decision tree + bypass templates.
        assert!(body.contains("决策树"));
        assert!(body.contains("分支 A"));
        assert!(body.contains("分支 B"));
        assert!(body.contains("分支 C"));
        assert!(body.contains("/v1/tools/x_search") || body.contains("/tools/x_search"));
        assert!(body.contains("tools/call"));
        assert!(body.contains("http://127.0.0.1:8787/mcp"));
        assert!(body.contains("TOOL_TIMEOUT") || body.contains("health"));
        assert!(body.contains("策略矩阵") || body.contains("仿冒 Build"));
        assert!(body.contains("## MCP 图片备选") || body.contains("image_gen"));
        let primary_idx = body
            .find("## 当前应优先走 GrokGo MCP 的工具")
            .expect("primary section");
        let x_idx = body.find("### `x_search`").expect("x_search");
        assert!(primary_idx < x_idx);
    }

    #[test]
    fn agents_guide_ref_uses_configured_port() {
        let guide = "/tmp/agents-guide.md";
        let out = agents_guide_ref_block(guide, 9999);
        assert!(out.contains("http://127.0.0.1:9999/mcp"));
        assert!(!out.contains("http://127.0.0.1:8787/mcp"));
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
    fn provider_settings_preserves_custom_provider_id() {
        let cfg = AppConfig::default();
        let settings =
            provider_settings_config_for_id(&cfg, false, "sub2api", "Sub2API");
        let toml = settings.get("config").and_then(|c| c.as_str()).unwrap_or("");
        assert!(toml.contains("model_provider = \"sub2api\""));
        assert!(toml.contains("[model_providers.sub2api]"));
        assert!(toml.contains("name = \"Sub2API\""));
        assert!(!toml.contains("model_provider = \"grok-go\""));
    }

    #[test]
    fn provider_settings_quotes_numeric_provider_id() {
        let cfg = AppConfig::default();
        let settings = provider_settings_config_for_id(&cfg, true, "98", "98");
        let toml = settings.get("config").and_then(|c| c.as_str()).unwrap_or("");
        assert!(toml.contains("model_provider = \"98\""));
        assert!(toml.contains("[model_providers.\"98\"]"));
        assert!(toml.contains("[mcp_servers.grok-go]"));
    }

    #[test]
    fn safe_provider_id_validation() {
        assert!(is_safe_codex_provider_id("sub2api"));
        assert!(is_safe_codex_provider_id("grok-go"));
        assert!(is_safe_codex_provider_id("98"));
        assert!(!is_safe_codex_provider_id(""));
        assert!(!is_safe_codex_provider_id("bad id"));
        assert!(!is_safe_codex_provider_id("a/b"));
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

    /// Regression: bare `ANTHROPIC_BASE_URL` must NOT claim DeepSeek/Kimi/etc.
    /// Only name/notes/local-gateway fingerprints identify our Claude provider.
    #[test]
    fn find_claude_provider_ignores_third_party_anthropic_env() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE providers (
              id TEXT NOT NULL,
              app_type TEXT NOT NULL,
              name TEXT NOT NULL,
              settings_config TEXT NOT NULL,
              website_url TEXT,
              category TEXT,
              created_at INTEGER,
              notes TEXT,
              is_current BOOLEAN NOT NULL DEFAULT 0,
              PRIMARY KEY (id, app_type)
            );
            INSERT INTO providers (id, app_type, name, settings_config, website_url, created_at, notes, is_current)
            VALUES
              ('deepseek-1', 'claude', 'DeepSeek',
               '{"env":{"ANTHROPIC_BASE_URL":"https://api.deepseek.com/anthropic","ANTHROPIC_MODEL":"deepseek-v4-pro"}}',
               'https://platform.deepseek.com', 1, '', 1),
              ('kimi-1', 'claude', 'Kimi',
               '{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com/coding/"}}',
               'https://www.kimi.com', 2, '', 0);
            "#,
        )
        .unwrap();

        // No GrokGo row yet → must not match third-party Claude providers.
        let found = find_existing_grokgo_provider_for_app(&conn, "claude").unwrap();
        assert!(
            found.is_none(),
            "must not hijack DeepSeek/Kimi via bare ANTHROPIC_BASE_URL, got {found:?}"
        );

        // A true GrokGo row (by name) is found even if settings were corrupted.
        conn.execute(
            r#"
            INSERT INTO providers (id, app_type, name, settings_config, website_url, created_at, notes, is_current)
            VALUES ('gg-1', 'claude', 'GrokGo',
                    '{"env":{"ANTHROPIC_BASE_URL":"https://api.deepseek.com/anthropic"}}',
                    'https://platform.deepseek.com', 3, '由 GrokGo 同步（Claude Code / Anthropic Messages）', 0)
            "#,
            [],
        )
        .unwrap();
        let found = find_existing_grokgo_provider_for_app(&conn, "claude").unwrap();
        assert_eq!(found.map(|(id, _)| id), Some("gg-1".into()));
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

    /// Build a minimal unsigned JWT with the given `exp` claim (seconds since epoch).
    fn test_jwt_with_exp(exp: i64) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload =
            URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp},"scope":"openid grok-cli:access"}}"#));
        format!("{header}.{payload}.sig")
    }

    /// Same acceptance criteria as `existing_grok_auth_is_usable` (unit-testable without env).
    fn auth_map_is_usable(map: &serde_json::Map<String, Value>) -> bool {
        let horizon = Utc::now() + chrono::Duration::seconds(GROK_AUTH_EARLY_INVALIDATION_SECS);
        for (_k, v) in map.iter() {
            let Some(obj) = v.as_object() else { continue };
            let Some(token) = obj
                .get("key")
                .or_else(|| obj.get("access_token"))
                .and_then(|x| x.as_str())
                .filter(|s| !s.trim().is_empty())
            else {
                continue;
            };
            let has_refresh = obj
                .get("refresh_token")
                .and_then(|x| x.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let jwt_ok = crate::auth::jwt_payload(token)
                .and_then(|p| p.get("exp").and_then(|x| x.as_i64()))
                .and_then(|secs| chrono::DateTime::<Utc>::from_timestamp(secs, 0))
                .map(|exp| exp > horizon)
                .unwrap_or(false);
            if jwt_ok {
                return true;
            }
            let meta_ok = obj
                .get("expires_at")
                .and_then(|x| x.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc) > horizon)
                .unwrap_or(false);
            if has_refresh && meta_ok {
                return true;
            }
        }
        false
    }

    #[test]
    fn existing_grok_auth_usable_when_jwt_not_near_expiry() {
        let far = (Utc::now() + chrono::Duration::hours(2)).timestamp();
        let body = serde_json::json!({
            "https://auth.x.ai::client": {
                "key": test_jwt_with_exp(far),
                "refresh_token": "rt",
                "expires_at": (Utc::now() + chrono::Duration::hours(2)).to_rfc3339(),
            }
        });
        let map = body.as_object().unwrap().clone();
        assert!(auth_map_is_usable(&map), "fresh JWT must be usable");

        let past = (Utc::now() - chrono::Duration::hours(1)).timestamp();
        let body2 = serde_json::json!({
            "https://auth.x.ai::client": {
                "key": test_jwt_with_exp(past),
                "refresh_token": "rt",
                "expires_at": (Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
            }
        });
        let map2 = body2.as_object().unwrap().clone();
        assert!(
            !auth_map_is_usable(&map2),
            "expired JWT must NOT be treated as usable"
        );
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

    #[test]
    fn opencode_detects_provider_and_mcp() {
        let mut cfg = AppConfig::default();
        cfg.actual_port = 8787;
        let raw = r#"{
          "model": "grok-go/grok-4.5",
          "provider": {
            "grok-go": {
              "options": { "baseURL": "http://127.0.0.1:8787/v1" }
            }
          },
          "mcp": {
            "grok-go": {
              "type": "remote",
              "url": "http://127.0.0.1:8787/mcp",
              "enabled": true
            }
          }
        }"#;
        assert!(opencode_model_is_injected(raw, &cfg));
        assert!(opencode_mcp_is_injected(raw, &cfg));
        assert!(!opencode_model_is_injected("{}", &cfg));
        assert!(!opencode_mcp_is_injected("{}", &cfg));
    }

    #[test]
    fn client_mcp_json_merge_preserves_others() {
        let dir = std::env::temp_dir().join(format!("grok-go-mcp-test-{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("mcp.json");
        fs::write(
            &path,
            r#"{"mcpServers":{"keep-me":{"url":"http://example.com/mcp"}}}"#,
        )
        .unwrap();
        let mut cfg = AppConfig::default();
        cfg.actual_port = 8787;
        cfg.require_token = true;
        cfg.local_token = "tok-test".into();
        set_client_mcp_json_inject(&path, &cfg, true, "test-mcp", "http", true).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert!(v.pointer("/mcpServers/keep-me").is_some());
        assert!(v.pointer("/mcpServers/grok-go").is_some());
        let url = v
            .pointer("/mcpServers/grok-go/url")
            .and_then(|u| u.as_str())
            .unwrap();
        assert!(url.contains("8787"));
        assert!(url.contains("/mcp"));
        assert_eq!(
            v.pointer("/mcpServers/grok-go/type")
                .and_then(|t| t.as_str()),
            Some("http")
        );
        set_client_mcp_json_inject(&path, &cfg, false, "test-mcp", "http", true).unwrap();
        let raw2 = fs::read_to_string(&path).unwrap();
        let v2: Value = serde_json::from_str(&raw2).unwrap();
        assert!(v2.pointer("/mcpServers/keep-me").is_some());
        assert!(v2.pointer("/mcpServers/grok-go").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn workbuddy_models_array_and_object() {
        let arr = json!([{ "id": "a", "url": "http://127.0.0.1:8787/v1/chat/completions" }]);
        let obj = json!({
            "models": [{ "id": "b", "url": "http://127.0.0.1:8787/v1/chat/completions" }],
            "availableModels": ["b"]
        });
        assert_eq!(workbuddy_models_array(&arr).len(), 1);
        assert_eq!(workbuddy_models_array(&obj).len(), 1);
        assert_eq!(
            workbuddy_available_models(&obj).as_ref().map(|v| v.len()),
            Some(1)
        );
        let mut cfg = AppConfig::default();
        cfg.actual_port = 8787;
        // Detection needs a real path — write temp.
        let dir = std::env::temp_dir().join(format!("grok-go-wb-{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("models.json");
        fs::write(&path, serde_json::to_string(&obj).unwrap()).unwrap();
        assert!(workbuddy_model_is_injected(&path, &cfg));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cursor_byok_fields_use_gateway() {
        let mut cfg = AppConfig::default();
        cfg.actual_port = 9999;
        cfg.require_token = true;
        cfg.local_token = "abc".into();
        cfg.default_model = "grok-4.5".into();
        let f = cursor_byok_fields(&cfg);
        assert_eq!(f.base_url, "http://127.0.0.1:9999/v1");
        assert_eq!(f.token, "abc");
        assert_eq!(f.model, "grok-4.5");
    }
}
