use serde::Serialize;
use tauri::{AppHandle, State};

use crate::auth::{OAuthManager, OAuthStart};
use crate::config::{
    load_auth, load_config, save_config, Account, AppConfig, AppIconStyle, random_token,
};
use crate::error::{AppError, AppResult};
use crate::gateway::server::{start_gateway, GatewayState};
use crate::integrations::{
    import_cc_switch_claude_provider, import_cc_switch_provider, inject_codex_agents_guide,
    integration_status, set_codex_mcp_inject, set_cursor_mcp_inject, set_opencode_mcp_inject,
    set_opencode_model_inject, set_workbuddy_mcp_inject, set_workbuddy_model_inject,
    set_grok_build_inject as set_grok_build_inject_impl,
    restore_grok_build_backup as restore_grok_build_backup_impl, IntegrationStatus,
};
use crate::account_import::{
    credential_to_account, is_duplicate, parse_import_payload, ImportAccountsOptions,
    ImportAccountsResult, ImportErrorItem,
};
use crate::auth::refresh_account;
use crate::router::{
    batch_update_accounts, list_accounts, remove_account, remove_accounts, save_accounts,
    update_account, BatchAccountPatch,
};
use crate::usage::{HeatmapDay, LogStoreStats, RequestLog, UsageStore, UsageSummary};
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
        healthy_accounts: auth
            .accounts
            .iter()
            .filter(|a| a.enabled && a.is_credentialed())
            .filter(|a| a.health != crate::config::AccountHealth::Cooldown)
            .count(),
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
    // Never persist an empty OAuth client id (would break Windows/macOS login).
    if config.xai_client_id.trim().is_empty() {
        config.xai_client_id = if existing.xai_client_id.trim().is_empty() {
            crate::config::DEFAULT_XAI_CLIENT_ID.into()
        } else {
            existing.xai_client_id.clone()
        };
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

/// Batch-import accounts from CPA / sub2api / 卡网 SSO paste formats.
///
/// Card SSO lines are converted to OAuth via Device Flow (no grok.com reverse).
/// Also accepts multi-line refresh tokens, CPA `xai-*.json`, sub2api credentials,
/// GrokGo `auth.json`, NDJSON, or arrays of the above.
#[tauri::command]
pub async fn import_accounts(
    payload: String,
    options: Option<ImportAccountsOptions>,
) -> AppResult<ImportAccountsResult> {
    let opts = options.unwrap_or_default();
    let parsed = parse_import_payload(&payload).map_err(AppError::msg)?;
    if parsed.is_empty() {
        return Err(AppError::msg("no credentials found in import payload"));
    }

    let config = load_config()?;
    let mut added = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut errors = Vec::new();
    let mut new_accounts = Vec::new();

    for (index, cred) in parsed.into_iter().enumerate() {
        let mut account = credential_to_account(&cred, &opts);
        // Check duplicates against live store + this batch (not a long-lived snapshot).
        if opts.skip_duplicates {
            let existing = load_auth()?.accounts;
            if is_duplicate(&existing, &account) || is_duplicate(&new_accounts, &account) {
                skipped += 1;
                continue;
            }
        }

        // Card / web SSO → OAuth device flow (pure Rust, no Python).
        let sso_cookie = crate::sso_convert::account_sso_cookie(&account).or_else(|| {
            cred.sso_token
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
        let has_oauth_rt = account
            .refresh_token
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let should_convert = sso_cookie.is_some()
            && (account.auth_kind == crate::config::AccountAuthKind::Sso || !has_oauth_rt);

        if should_convert {
            let sso = sso_cookie.unwrap();
            let label = account
                .email
                .clone()
                .unwrap_or_else(|| account.name.clone());
            match crate::sso_convert::convert_sso_cookie(&sso, account.email.as_deref()).await {
                Ok(converted) => {
                    crate::sso_convert::apply_oauth_to_account(
                        &mut account,
                        &converted,
                        Some(&sso),
                    );
                    tracing::info!(account = %label, "SSO→OAuth convert ok");
                }
                Err(err) => {
                    failed += 1;
                    errors.push(ImportErrorItem {
                        index: index + 1,
                        detail: format!("{label}: SSO→OAuth failed: {err}"),
                    });
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            }
            // Pace conversions to reduce auth.x.ai rate limits
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        } else if opts.validate_refresh {
            if account.refresh_token.is_some() {
                match refresh_account(&config, &mut account).await {
                    Ok(_) => {}
                    Err(err) => {
                        // Keep RT-only account if it has a refresh token — user can re-login later.
                        if account.access_token.is_none() {
                            failed += 1;
                            errors.push(ImportErrorItem {
                                index: index + 1,
                                detail: format!(
                                    "{}: {err}",
                                    account.email.as_deref().unwrap_or(account.name.as_str())
                                ),
                            });
                            continue;
                        }
                        tracing::warn!(
                            account = %account.name,
                            "import refresh failed but access_token present: {err}"
                        );
                    }
                }
            } else if account.access_token.is_none() {
                failed += 1;
                errors.push(ImportErrorItem {
                    index: index + 1,
                    detail: "missing both access_token and refresh_token".into(),
                });
                continue;
            }
        }

        if !account.is_credentialed() {
            failed += 1;
            errors.push(ImportErrorItem {
                index: index + 1,
                detail: format!(
                    "{}: not credentialed after import",
                    account.email.as_deref().unwrap_or(account.name.as_str())
                ),
            });
            continue;
        }

        new_accounts.push(account);
        added += 1;
    }

    // Append to the *live* store so concurrent deletes are not overwritten.
    let new_ids: Vec<String> = new_accounts.iter().map(|a| a.id.clone()).collect();
    let mut accounts = if !new_accounts.is_empty() {
        crate::config::append_accounts(new_accounts)?
    } else {
        load_auth()?.accounts
    };

    // Auto-refresh SuperGrok quota for successfully added accounts.
    if !new_ids.is_empty() {
        for id in &new_ids {
            match crate::quota::refresh_account_quota(id).await {
                Ok(_) => tracing::info!(account_id = %id, "post-import quota refresh ok"),
                Err(err) => tracing::warn!(
                    account_id = %id,
                    "post-import quota refresh failed (non-fatal): {err}"
                ),
            }
        }
        accounts = load_auth()?.accounts;
    }

    Ok(ImportAccountsResult {
        added,
        skipped,
        failed,
        accounts,
        errors,
    })
}

/// Re-convert legacy `auth_kind=sso` accounts (or any row with `sso_token` but no OAuth tokens).
#[tauri::command]
pub async fn convert_sso_accounts() -> AppResult<ImportAccountsResult> {
    let mut added = 0usize; // converted count
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut errors = Vec::new();

    let store = load_auth()?;
    let targets: Vec<(String, Option<String>, String)> = store
        .accounts
        .iter()
        .filter_map(|a| {
            let sso = crate::sso_convert::account_sso_cookie(a)?;
            // Skip already-OAuth credentialed accounts with refresh token.
            if a.is_credentialed()
                && a
                    .refresh_token
                    .as_ref()
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
            {
                return None;
            }
            Some((a.id.clone(), a.email.clone(), sso))
        })
        .collect();

    if targets.is_empty() {
        return Ok(ImportAccountsResult {
            added: 0,
            skipped: store.accounts.len(),
            failed: 0,
            accounts: store.accounts,
            errors: vec![ImportErrorItem {
                index: 0,
                detail: "no SSO accounts needing conversion".into(),
            }],
        });
    }

    for (index, (id, email, sso)) in targets.into_iter().enumerate() {
        let label = email.clone().unwrap_or_else(|| id.clone());
        // Work on a single-account clone; merge back by id so deletes win.
        let mut account = match load_auth()?.accounts.into_iter().find(|a| a.id == id) {
            Some(a) => a,
            None => {
                skipped += 1;
                continue;
            }
        };
        match crate::sso_convert::convert_sso_cookie(&sso, email.as_deref()).await {
            Ok(converted) => {
                crate::sso_convert::apply_oauth_to_account(&mut account, &converted, Some(&sso));
                if crate::config::apply_account_update(&account)? {
                    added += 1;
                    tracing::info!(account = %label, "SSO→OAuth re-convert ok");
                } else {
                    skipped += 1;
                }
            }
            Err(err) => {
                failed += 1;
                errors.push(ImportErrorItem {
                    index: index + 1,
                    detail: format!("{label}: {err}"),
                });
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }

    Ok(ImportAccountsResult {
        added,
        skipped,
        failed,
        accounts: load_auth()?.accounts,
        errors,
    })
}

/// Delete many accounts at once. Returns the updated account list.
#[tauri::command]
pub fn batch_delete_accounts(account_ids: Vec<String>) -> AppResult<Vec<Account>> {
    let n = remove_accounts(&account_ids)?;
    if n == 0 && !account_ids.is_empty() {
        // Still return current list, but surface a clear error so UI is not silent.
        return Err(AppError::msg(format!(
            "批量删除未匹配到任何账号（请求 {} 个 id，可能已过期，请刷新后重试）",
            account_ids.len()
        )));
    }
    list_accounts()
}

/// Batch-update enabled / weight / media flags / clear cooldown.
#[tauri::command]
pub fn batch_patch_accounts(
    account_ids: Vec<String>,
    patch: BatchAccountPatch,
) -> AppResult<Vec<Account>> {
    batch_update_accounts(&account_ids, patch)?;
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
pub fn get_log_stats() -> AppResult<LogStoreStats> {
    match UsageStore::open_default() {
        Ok(store) => store.stats(),
        Err(err) => {
            tracing::warn!("get_log_stats open: {err}");
            Ok(LogStoreStats {
                total_rows: 0,
                oldest_at: None,
                newest_at: None,
                db_bytes: 0,
                retention_days: 30,
                max_rows: 50_000,
            })
        }
    }
}

/// Delete logs older than `older_than_days` (relative to now).
#[tauri::command]
pub fn clear_logs_older_than(older_than_days: u32) -> AppResult<u64> {
    let days = older_than_days.max(0);
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
    match UsageStore::open_default() {
        Ok(store) => store.delete_before(&cutoff),
        Err(err) => {
            tracing::warn!("clear_logs_older_than open: {err}");
            Ok(0)
        }
    }
}

/// Delete logs in inclusive range. `from` / `to` are ISO-8601 / RFC3339 or `YYYY-MM-DD`.
#[tauri::command]
pub fn clear_logs_range(from: String, to: String) -> AppResult<u64> {
    let from = normalize_log_bound(&from, false);
    let to = normalize_log_bound(&to, true);
    match UsageStore::open_default() {
        Ok(store) => store.delete_range(&from, &to),
        Err(err) => {
            tracing::warn!("clear_logs_range open: {err}");
            Ok(0)
        }
    }
}

#[tauri::command]
pub fn prune_logs_now() -> AppResult<LogStoreStats> {
    match UsageStore::open_default() {
        Ok(store) => store.prune_now(),
        Err(err) => {
            tracing::warn!("prune_logs_now open: {err}");
            Err(err)
        }
    }
}

fn normalize_log_bound(s: &str, end_of_day: bool) -> String {
    let t = s.trim();
    if t.len() == 10 && t.chars().nth(4) == Some('-') {
        // YYYY-MM-DD
        if end_of_day {
            format!("{t}T23:59:59.999999999Z")
        } else {
            format!("{t}T00:00:00Z")
        }
    } else if !t.is_empty() {
        t.to_string()
    } else if end_of_day {
        chrono::Utc::now().to_rfc3339()
    } else {
        "1970-01-01T00:00:00Z".into()
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
pub fn restore_grok_build_backup() -> AppResult<IntegrationStatus> {
    restore_grok_build_backup_impl()
}

#[tauri::command]
pub fn import_to_cc_switch() -> AppResult<String> {
    import_cc_switch_provider()
}

#[tauri::command]
pub fn import_claude_to_cc_switch() -> AppResult<String> {
    import_cc_switch_claude_provider()
}

#[tauri::command]
pub fn export_provider_snippet() -> AppResult<String> {
    Ok(integration_status()?.provider_snippet)
}

#[tauri::command]
pub fn set_opencode_model_inject_cmd(enabled: bool) -> AppResult<IntegrationStatus> {
    set_opencode_model_inject(enabled)
}

#[tauri::command]
pub fn set_opencode_mcp_inject_cmd(enabled: bool) -> AppResult<IntegrationStatus> {
    set_opencode_mcp_inject(enabled)
}

#[tauri::command]
pub fn set_workbuddy_model_inject_cmd(enabled: bool) -> AppResult<IntegrationStatus> {
    set_workbuddy_model_inject(enabled)
}

#[tauri::command]
pub fn set_workbuddy_mcp_inject_cmd(enabled: bool) -> AppResult<IntegrationStatus> {
    set_workbuddy_mcp_inject(enabled)
}

#[tauri::command]
pub fn set_cursor_mcp_inject_cmd(enabled: bool) -> AppResult<IntegrationStatus> {
    set_cursor_mcp_inject(enabled)
}
