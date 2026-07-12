use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use uuid::Uuid;

use crate::config::{load_config, save_config, AppConfig};
use crate::error::{AppError, AppResult};
use crate::paths::{
    agents_guide_file_path, app_home, cc_switch_db_path, codex_agents_md_path, codex_config_path,
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
    pub provider_snippet: String,
    pub mcp_snippet: String,
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
    Ok(IntegrationStatus {
        codex_mcp_injected: injected,
        codex_config_path: codex_path.display().to_string(),
        codex_agents_injected: agents_injected,
        codex_agents_path: agents_path.display().to_string(),
        agents_guide_file_path: guide_file,
        cc_switch_db_path: cc_switch_db_path().display().to_string(),
        provider_snippet: provider_snippet(&config),
        mcp_snippet: mcp_snippet(&config),
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
    } else {
        remove_codex_mcp()?;
        // MCP uninject also strips the managed AGENTS.md guide block.
        remove_codex_agents_guide()?;
    }
    integration_status()
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
fn agents_guide_file_body() -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        r#"# GrokGo 工具指引

> 版本：{version}  
> 本文件由 GrokGo 维护，随软件版本更新。请勿手改（重新注入会覆盖）。

## 强制规则

- **参数以 MCP `tools/list` 为准**：直接调用，**禁止** web_search / 翻仓库 / 读 HANDOFF、server.rs 查参数。
- 媒体输入可用：`https://` / `data:` / **本地绝对路径** / `file://`（本地会自动转 data URL）。
- 返回一律是 `~/.grok-go/artifacts/` 下的**绝对本地路径** + `markdown`，用 `![image](/abs/path)` 渲染；**不要**展示远程 CDN URL。

## 工具速查

### `x_search`
- 必填：`query`
- 可选：`allowed_handles` `excluded_handles` `from_date` `to_date`（YYYY-MM-DD）

### `image_gen` / `image_generate`
- 必填：`prompt`
- 可选：`n`(1–4) `model` `size` `quality`(low|medium|high)

### `image_edit`
- 必填：`prompt` + `image_url`（URL 或本地路径）
- 可选：`model`

### `video_generate`（文生视频 / 图生视频 / 多图参考）
- 必填：`prompt`
- 模式（三选一）：
  1. 文生视频：仅 `prompt`
  2. 图生视频：`prompt` + `image_url`（首帧）
  3. 多图参考：`prompt` + `reference_image_urls`（1–7，勿与 `image_url` 同用）
- 可选：`duration`(1–15) `aspect_ratio`(1:1|16:9|9:16|4:3|3:4|3:2|2:3) `resolution`(480p|720p|1080p) `model`
- 示例图生视频：
  `{{"prompt":"轻推镜头，微风吹动毛发","image_url":"/abs/path.png","duration":6}}`

### `video_edit`
- 必填：`prompt` + `video_url`（URL 或本地路径）
- 可选：`model`
- 注意：编辑不支持自定义 duration/aspect_ratio

## 健康检查

```bash
curl -s http://127.0.0.1:8787/health
```

Responses API Base：`http://127.0.0.1:8787/v1`  
MCP：`http://127.0.0.1:8787/mcp`  
产物目录：`~/.grok-go/artifacts/`
"#
    )
}

/// Write/update the versioned guide under the app config directory.
fn ensure_agents_guide_file() -> AppResult<std::path::PathBuf> {
    let path = agents_guide_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, agents_guide_file_body())?;
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
    let config = load_config()?;
    let db_path = cc_switch_db_path();
    if !db_path.exists() {
        let payload = provider_export_json(&config);
        let export_path = crate::paths::app_home()?.join("cc-switch-provider-export.json");
        fs::write(&export_path, serde_json::to_string_pretty(&payload)?)?;
        return Ok(format!(
            "CC Switch DB not found. Exported provider JSON to {}",
            export_path.display()
        ));
    }

    let conn = Connection::open(db_path)?;
    let id = Uuid::new_v4().to_string();
    let name = "GrokGo";
    let settings = provider_settings_config(&config);
    let settings_text = serde_json::to_string(&settings)?;
    let now = Utc::now().timestamp_millis();
    conn.execute(
        r#"
        INSERT INTO providers (
          id, app_type, name, settings_config, website_url, category, created_at,
          sort_index, notes, icon, icon_color, meta, is_current, in_failover_queue,
          cost_multiplier, limit_daily_usd, limit_monthly_usd, provider_type
        ) VALUES (?1,'codex',?2,?3,NULL,'custom',?4,NULL,?5,NULL,NULL,'{}',0,0,'1.0',NULL,NULL,NULL)
        "#,
        params![id, name, settings_text, now, "Imported from GrokGo"],
    )?;
    Ok(format!("Imported provider into CC Switch with id {id}"))
}

fn provider_settings_config(config: &AppConfig) -> serde_json::Value {
    let host = if config.lan_enabled { local_lan_host() } else { "127.0.0.1".into() };
    let base = format!("http://{}:{}/v1", host, config.actual_port);
    let toml = format!(
        "model_provider = \"custom\"\nmodel = \"{}\"\ndisable_response_storage = true\n\n[model_providers.custom]\nname = \"grok-go\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"{}\"\nexperimental_bearer_token = \"{}\"\n",
        config.default_model, base, config.local_token
    );
    json!({
        "auth": {"OPENAI_API_KEY": config.local_token},
        "config": toml,
        "modelCatalog": {
            "models": [
                {"model": config.default_model, "displayName": config.default_model, "contextWindow": 500000}
            ]
        }
    })
}

fn provider_export_json(config: &AppConfig) -> serde_json::Value {
    json!({
        "app_type": "codex",
        "name": "GrokGo",
        "settings_config": provider_settings_config(config)
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
}
