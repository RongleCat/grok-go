use directories::BaseDirs;
use std::fs;
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

pub fn app_home() -> AppResult<PathBuf> {
    let base = BaseDirs::new().ok_or_else(|| AppError::msg("unable to resolve home directory"))?;
    let home = base.home_dir().join(".grok-go");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(home.join("artifacts"))?;
    fs::create_dir_all(home.join("logs"))?;
    fs::create_dir_all(home.join("backups"))?;
    Ok(home)
}

pub fn config_path() -> AppResult<PathBuf> {
    Ok(app_home()?.join("config.json"))
}

pub fn auth_path() -> AppResult<PathBuf> {
    Ok(app_home()?.join("auth.json"))
}

pub fn db_path() -> AppResult<PathBuf> {
    Ok(app_home()?.join("data.db"))
}

pub fn artifacts_dir() -> AppResult<PathBuf> {
    Ok(app_home()?.join("artifacts"))
}

/// Versioned tool guide written under the app config home (`~/.grok-go/agents-guide.md`).
/// Codex `AGENTS.md` only holds a short fixed reference to this file.
pub fn agents_guide_file_path() -> AppResult<PathBuf> {
    Ok(app_home()?.join("agents-guide.md"))
}

pub fn codex_home() -> PathBuf {
    std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            directories::BaseDirs::new()
                .map(|b| b.home_dir().join(".codex"))
                .unwrap_or_else(|| PathBuf::from(".codex"))
        })
}

pub fn codex_config_path() -> PathBuf {
    codex_home().join("config.toml")
}

/// Codex global agent instructions (`~/.codex/AGENTS.md`, or `$CODEX_HOME/AGENTS.md`).
pub fn codex_agents_md_path() -> PathBuf {
    codex_home().join("AGENTS.md")
}

pub fn cc_switch_db_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().join(".cc-switch").join("cc-switch.db"))
        .unwrap_or_else(|| PathBuf::from(".cc-switch/cc-switch.db"))
}

/// Official Grok Build / Grok CLI config home (`~/.grok`).
pub fn grok_build_home() -> PathBuf {
    std::env::var("GROK_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            directories::BaseDirs::new()
                .map(|b| b.home_dir().join(".grok"))
                .unwrap_or_else(|| PathBuf::from(".grok"))
        })
}

pub fn grok_build_config_path() -> PathBuf {
    grok_build_home().join("config.toml")
}

/// Grok Build session credentials (`~/.grok/auth.json`).
pub fn grok_build_auth_path() -> PathBuf {
    grok_build_home().join("auth.json")
}

/// Stable one-click restore snapshot taken before multi-account routing inject.
pub fn grok_build_restore_dir() -> AppResult<PathBuf> {
    let dir = app_home()?.join("backups").join("grok-build-pre-route");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// OpenCode global config (`~/.config/opencode/opencode.json`).
pub fn opencode_config_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.config_dir().join("opencode").join("opencode.json"))
        .unwrap_or_else(|| PathBuf::from(".config/opencode/opencode.json"))
}

/// WorkBuddy / CodeBuddy user config home.
/// Prefers `WORKBUDDY_CONFIG_DIR`, then `CODEBUDDY_CONFIG_DIR`, else `~/.workbuddy`.
pub fn workbuddy_home() -> PathBuf {
    if let Ok(p) = std::env::var("WORKBUDDY_CONFIG_DIR") {
        let t = p.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    if let Ok(p) = std::env::var("CODEBUDDY_CONFIG_DIR") {
        let t = p.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    directories::BaseDirs::new()
        .map(|b| b.home_dir().join(".workbuddy"))
        .unwrap_or_else(|| PathBuf::from(".workbuddy"))
}

pub fn workbuddy_models_path() -> PathBuf {
    workbuddy_home().join("models.json")
}

/// WorkBuddy **user-editable** MCP config (`~/.workbuddy/mcp.json`).
///
/// The desktop "配置 MCP" UI edits this path. A sibling `.mcp.json` may exist as an
/// auto-generated connector-proxy aggregate and must **not** be the inject target.
pub fn workbuddy_mcp_path() -> PathBuf {
    workbuddy_home().join("mcp.json")
}

/// WorkBuddy auto-generated / alternate MCP file (`~/.workbuddy/.mcp.json`).
/// Used only to detect/cleanup stale GrokGo entries — not the primary inject path.
pub fn workbuddy_mcp_dot_path() -> PathBuf {
    workbuddy_home().join(".mcp.json")
}

/// Cursor global MCP config (`~/.cursor/mcp.json`).
pub fn cursor_mcp_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().join(".cursor").join("mcp.json"))
        .unwrap_or_else(|| PathBuf::from(".cursor/mcp.json"))
}
