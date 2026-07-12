use serde::Serialize;
use tauri::{AppHandle, State};

use crate::auth::{OAuthManager, OAuthStart};
use crate::config::{
    load_auth, load_config, save_config, Account, AppConfig, AppIconStyle, random_token,
};
use crate::error::AppResult;
use crate::gateway::server::{start_gateway, GatewayState};
use crate::integrations::{
    import_cc_switch_provider, inject_codex_agents_guide, integration_status, set_codex_mcp_inject,
    set_grok_build_inject as set_grok_build_inject_impl, IntegrationStatus,
};
use crate::router::{list_accounts, remove_account, save_accounts, update_account};
use crate::usage::{HeatmapDay, RequestLog, UsageStore, UsageSummary};
use local_ip_address::local_ip;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub running: bool,
    pub preferred_port: u16,
    pub actual_port: u16,
    pub bind_host: String,
    pub lan_enabled: bool,
    pub require_token: bool,
    pub local_token: String,
    pub base_url: String,
    pub mcp_url: String,
    pub account_count: usize,
    pub healthy_accounts: usize,
    pub lan_address: Option<String>,
    pub today: UsageSummary,
}

#[tauri::command]
pub async fn get_status(gateway: State<'_, GatewayState>) -> AppResult<AppStatus> {
    let config = load_config()?;
    let auth = load_auth()?;
    let running = *gateway.running.lock().await;
    let host = if config.lan_enabled {
        local_ip().map(|ip| ip.to_string()).unwrap_or_else(|_| config.bind_host.clone())
    } else {
        "127.0.0.1".into()
    };
    let today = match UsageStore::open_default() {
        Ok(store) => store.today_summary().unwrap_or_else(|err| {
            tracing::warn!("today_summary failed: {err}");
            crate::usage::empty_summary()
        }),
        Err(err) => {
            tracing::warn!("usage db open failed in get_status: {err}");
            crate::usage::empty_summary()
        }
    };
    Ok(AppStatus {
        running,
        preferred_port: config.preferred_port,
        actual_port: config.actual_port,
        bind_host: config.bind_host.clone(),
        lan_enabled: config.lan_enabled,
        require_token: config.require_token,
        local_token: config.local_token.clone(),
        base_url: format!("http://{}:{}/v1", host, config.actual_port),
        mcp_url: format!("http://{}:{}/mcp", host, config.actual_port),
        account_count: auth.accounts.len(),
        healthy_accounts: auth.accounts.iter().filter(|a| a.enabled && a.access_token.is_some()).count(),
        lan_address: if config.lan_enabled { Some(host) } else { None },
        today,
    })
}

#[tauri::command]
pub async fn start_server(gateway: State<'_, GatewayState>) -> AppResult<AppStatus> {
    if !*gateway.running.lock().await {
        let _ = start_gateway(gateway.inner().clone()).await?;
    }
    get_status(gateway).await
}

#[tauri::command]
pub fn get_config() -> AppResult<AppConfig> {
    load_config()
}

#[tauri::command]
pub fn update_config(
    app: AppHandle,
    gateway: State<'_, GatewayState>,
    mut config: AppConfig,
) -> AppResult<AppConfig> {
    let existing = load_config()?;
    // preserve token unless rotated explicitly by empty/new value handling from UI
    if config.local_token.trim().is_empty() {
        config.local_token = existing.local_token;
    }
    if config.lan_enabled {
        if config.bind_host == "127.0.0.1" {
            config.bind_host = "0.0.0.0".into();
        }
    } else {
        config.bind_host = "127.0.0.1".into();
    }
    // Validate proxy URL when enabling so the client rebuild does not fail silently later.
    if config.http_proxy_enabled && config.http_proxy_url.trim().is_empty() {
        return Err(crate::error::AppError::msg(
            "HTTP proxy is enabled but URL is empty",
        ));
    }
    let proxy_changed = existing.http_proxy_enabled != config.http_proxy_enabled
        || existing.http_proxy_url != config.http_proxy_url
        || existing.xai_base_url != config.xai_base_url;
    let icon_changed = existing.app_icon != config.app_icon;
    let mcp_tools_changed = existing.mcp_enabled_tools != config.mcp_enabled_tools;
    save_config(&config)?;
    if proxy_changed {
        gateway.proxy.rebuild_client(&config)?;
        crate::auth::rebuild_oauth_http_client(&config)?;
    }
    if icon_changed {
        if let Err(err) = crate::apply_app_icon(&app, config.app_icon) {
            tracing::warn!("apply app icon after config update: {err}");
        }
    }
    if mcp_tools_changed {
        // Keep ~/.grok-go/agents-guide.md aligned with enabled MCP tools.
        if let Err(err) = crate::integrations::refresh_agents_guide_file() {
            tracing::warn!("refresh agents-guide after mcp tools change: {err}");
        }
    }
    Ok(config)
}

/// Switch dock/window/tray brand icon and persist preference.
#[tauri::command]
pub fn set_app_icon(app: AppHandle, style: AppIconStyle) -> AppResult<AppConfig> {
    let mut config = load_config()?;
    config.app_icon = style;
    save_config(&config)?;
    crate::apply_app_icon(&app, style).map_err(crate::error::AppError::msg)?;
    Ok(config)
}

#[tauri::command]
pub fn rotate_token() -> AppResult<AppConfig> {
    let mut config = load_config()?;
    config.local_token = random_token();
    save_config(&config)?;
    Ok(config)
}

/// Curated + live model IDs for mapping / settings dropdowns.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOptions {
    pub codex: Vec<String>,
    pub grok_text: Vec<String>,
    pub grok_image: Vec<String>,
    pub grok_video: Vec<String>,
}

#[tauri::command]
pub async fn list_model_options() -> AppResult<ModelOptions> {
    let config = load_config()?;
    let mut grok_text = curated_grok_text(&config);
    let mut grok_image = curated_grok_image(&config);
    let mut grok_video = curated_grok_video(&config);

    // Merge live xAI /models when reachable.
    if let Ok(value) = crate::gateway::proxy::fetch_upstream_models(&config).await {
        if let Some(arr) = value.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let lower = id.to_ascii_lowercase();
                if lower.contains("image") || lower.contains("imagine-image") {
                    push_unique(&mut grok_image, id);
                } else if lower.contains("video") || lower.contains("imagine-video") {
                    push_unique(&mut grok_video, id);
                } else if lower.starts_with("grok") {
                    push_unique(&mut grok_text, id);
                }
            }
        }
    }

    grok_text.sort();
    grok_image.sort();
    grok_video.sort();

    Ok(ModelOptions {
        codex: curated_codex_models(),
        grok_text,
        grok_image,
        grok_video,
    })
}

fn push_unique(list: &mut Vec<String>, id: &str) {
    if !list.iter().any(|x| x == id) {
        list.push(id.to_string());
    }
}

/// Codex CLI / IDE model labels commonly selected by clients (map-from side).
fn curated_codex_models() -> Vec<String> {
    vec![
        "gpt-5.6-sol".into(),
        "gpt-5.6-terra".into(),
        "gpt-5.6-luna".into(),
        "gpt-5.6".into(),
        "gpt-5.5".into(),
        "gpt-5.4".into(),
        "gpt-5.3-codex".into(),
        "gpt-5.2-codex".into(),
        "gpt-5.1-codex".into(),
        "gpt-5.1-codex-max".into(),
        "gpt-5.1-codex-mini".into(),
        "gpt-5-codex".into(),
        "codex-mini-latest".into(),
        "o3".into(),
        "o4-mini".into(),
        "gpt-4.1".into(),
        "gpt-4.1-mini".into(),
    ]
}

fn curated_grok_text(config: &AppConfig) -> Vec<String> {
    let mut v = vec![
        config.default_model.clone(),
        "grok-4.5".into(),
        "grok-4.20-reasoning".into(),
        "grok-4".into(),
        "grok-3".into(),
        "grok-3-mini".into(),
        "grok-2".into(),
    ];
    v.sort();
    v.dedup();
    v
}

fn curated_grok_image(config: &AppConfig) -> Vec<String> {
    let mut v = vec![
        config.default_image_model.clone(),
        "grok-imagine-image-quality".into(),
        "grok-imagine-image".into(),
        "grok-2-image".into(),
    ];
    v.sort();
    v.dedup();
    v
}

fn curated_grok_video(config: &AppConfig) -> Vec<String> {
    let mut v = vec![
        config.default_video_model.clone(),
        "grok-imagine-video".into(),
    ];
    v.sort();
    v.dedup();
    v
}

#[tauri::command]
pub fn get_accounts() -> AppResult<Vec<Account>> {
    list_accounts()
}

#[tauri::command]
pub fn upsert_account(account: Account) -> AppResult<Vec<Account>> {
    update_account(account)?;
    list_accounts()
}

#[tauri::command]
pub fn delete_account(account_id: String) -> AppResult<Vec<Account>> {
    remove_account(&account_id)?;
    list_accounts()
}

/// Replace the entire account list (used by settings backup import).
#[tauri::command]
pub fn replace_accounts(accounts: Vec<Account>) -> AppResult<Vec<Account>> {
    save_accounts(accounts)?;
    list_accounts()
}

#[tauri::command]
pub fn clear_account_cooldown(account_id: String) -> AppResult<Vec<Account>> {
    crate::auth::clear_account_cooldown(&account_id)?;
    list_accounts()
}

/// Fetch SuperGrok weekly credit quota for one account (remaining % + reset time).
#[tauri::command]
pub async fn refresh_account_quota(account_id: String) -> AppResult<Vec<Account>> {
    crate::quota::refresh_account_quota(&account_id).await?;
    list_accounts()
}

/// Fetch SuperGrok weekly credit quota for every signed-in account.
#[tauri::command]
pub async fn refresh_all_account_quotas() -> AppResult<Vec<Account>> {
    crate::quota::refresh_all_account_quotas().await
}

#[tauri::command]
pub async fn start_oauth_login(
    app: tauri::AppHandle,
    oauth: State<'_, OAuthManager>,
    account_name: Option<String>,
    account_id: Option<String>,
) -> AppResult<OAuthStart> {
    let config = load_config()?;
    let mut start = oauth
        .start_login(&config, account_name, account_id)
        .await?;

    // 1) OS-native open (most reliable on macOS: /usr/bin/open)
    let mut opened = crate::http_client::open_browser_url(&start.authorize_url).is_ok();

    // 2) Tauri opener plugin
    if !opened {
        use tauri_plugin_opener::OpenerExt;
        match app.opener().open_url(&start.authorize_url, None::<&str>) {
            Ok(()) => opened = true,
            Err(err) => tracing::warn!("opener open_url failed: {err}"),
        }
    }

    // 3) `open` crate last resort
    if !opened {
        opened = open::that(&start.authorize_url).is_ok();
    }

    start.browser_opened = opened;
    if !opened {
        tracing::warn!(
            "could not open browser automatically; url={}",
            start.authorize_url
        );
    }
    Ok(start)
}

#[tauri::command]
pub fn get_usage_summary() -> AppResult<UsageSummary> {
    match UsageStore::open_default() {
        Ok(store) => store.today_summary().or_else(|err| {
            tracing::warn!("get_usage_summary: {err}");
            Ok(crate::usage::empty_summary())
        }),
        Err(err) => {
            tracing::warn!("get_usage_summary open: {err}");
            Ok(crate::usage::empty_summary())
        }
    }
}

#[tauri::command]
pub fn get_recent_logs(limit: Option<usize>, offset: Option<usize>) -> AppResult<Vec<RequestLog>> {
    match UsageStore::open_default() {
        Ok(store) => store
            .recent(limit.unwrap_or(50), offset.unwrap_or(0))
            .or_else(|err| {
                tracing::warn!("get_recent_logs: {err}");
                Ok(Vec::new())
            }),
        Err(err) => {
            tracing::warn!("get_recent_logs open: {err}");
            Ok(Vec::new())
        }
    }
}

#[tauri::command]
pub fn get_heatmap(days: Option<i64>) -> AppResult<Vec<HeatmapDay>> {
    // Default: ~1 full year (53 weeks), same span as GitHub contribution graph.
    match UsageStore::open_default() {
        Ok(store) => store.heatmap(days.unwrap_or(371)).or_else(|err| {
            tracing::warn!("get_heatmap: {err}");
            Ok(Vec::new())
        }),
        Err(err) => {
            tracing::warn!("get_heatmap open: {err}");
            Ok(Vec::new())
        }
    }
}

#[tauri::command]
pub fn clear_logs() -> AppResult<()> {
    match UsageStore::open_default() {
        Ok(store) => store.clear(),
        Err(err) => {
            tracing::warn!("clear_logs open: {err}");
            Ok(())
        }
    }
}

#[tauri::command]
pub fn get_integrations() -> AppResult<IntegrationStatus> {
    integration_status()
}

#[tauri::command]
pub fn set_mcp_inject(enabled: bool) -> AppResult<IntegrationStatus> {
    set_codex_mcp_inject(enabled)
}

#[tauri::command]
pub fn inject_agents_guide() -> AppResult<IntegrationStatus> {
    inject_codex_agents_guide()
}

#[tauri::command]
pub fn set_grok_build_inject(enabled: bool) -> AppResult<IntegrationStatus> {
    set_grok_build_inject_impl(enabled)
}

#[tauri::command]
pub fn import_to_cc_switch() -> AppResult<String> {
    import_cc_switch_provider()
}

#[tauri::command]
pub fn export_provider_snippet() -> AppResult<String> {
    Ok(integration_status()?.provider_snippet)
}
