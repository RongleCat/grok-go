mod account_import;
mod auth;
mod commands;
mod concurrency;
mod config;
mod error;
mod gateway;
mod http_client;
mod integrations;
mod paths;
mod quota;
mod router;
mod session_affinity;
mod sso_convert;
mod usage;

use auth::OAuthManager;
use config::AppIconStyle;
use gateway::server::{start_gateway, GatewayState};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewWindow, WindowEvent};

#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

// ── Embedded brand icons (variants under icons/variants/{dark,light}/) ────────
// macOS tray: black glyph template (system tints for light/dark menu bar).
#[cfg(target_os = "macos")]
const TRAY_DARK_BYTES: &[u8] = include_bytes!("../icons/variants/dark/tray-icon-36.png");
#[cfg(target_os = "macos")]
const TRAY_LIGHT_BYTES: &[u8] = include_bytes!("../icons/variants/light/tray-icon-36.png");
// Windows / Linux tray: solid black square + white logo (opaque). Transparent
// white-glyph assets render as an all-white blob in the Windows notification area.
#[cfg(not(target_os = "macos"))]
const TRAY_WIN_BYTES: &[u8] = include_bytes!("../icons/variants/dark/tray-icon-win.png");

const APP_ICON_DARK_BYTES: &[u8] = include_bytes!("../icons/variants/dark/icon.png");
const APP_ICON_LIGHT_BYTES: &[u8] = include_bytes!("../icons/variants/light/icon.png");

fn tray_bytes(style: AppIconStyle) -> &'static [u8] {
    #[cfg(target_os = "macos")]
    {
        match style {
            AppIconStyle::Dark => TRAY_DARK_BYTES,
            AppIconStyle::Light => TRAY_LIGHT_BYTES,
        }
    }
    // Windows / Linux: always black-bg white logo — readable on light & dark trays.
    // Icon style switch is hidden on Windows; keep tray consistent regardless.
    #[cfg(not(target_os = "macos"))]
    {
        let _ = style;
        TRAY_WIN_BYTES
    }
}

fn app_icon_bytes(style: AppIconStyle) -> &'static [u8] {
    match style {
        AppIconStyle::Dark => APP_ICON_DARK_BYTES,
        AppIconStyle::Light => APP_ICON_LIGHT_BYTES,
    }
}

/// Apply tray + window icon for the selected brand style.
///
/// Platform notes:
/// - **Windows / Linux**: `WebviewWindow::set_icon` updates the window/taskbar icon immediately.
/// - **macOS**: the Dock icon is owned by `NSApplication`, not the window. We also call
///   `NSApp.setApplicationIconImage` so Dock updates without restart. macOS may still
///   cache Dock glyphs briefly; tray template icons always use a black glyph and are
///   recolored by the system.
pub fn apply_app_icon(app: &AppHandle, style: AppIconStyle) -> Result<(), String> {
    let tray_img =
        Image::from_bytes(tray_bytes(style)).map_err(|e| format!("tray icon decode: {e}"))?;
    if let Some(tray) = app.tray_by_id("main") {
        tray.set_icon(Some(tray_img))
            .map_err(|e| format!("set tray icon: {e}"))?;
        // macOS template works for both (black glyph); Windows uses colored glyphs.
        let _ = tray.set_icon_as_template(cfg!(target_os = "macos"));
    }

    let icon_bytes = app_icon_bytes(style);
    let win_img =
        Image::from_bytes(icon_bytes).map_err(|e| format!("app icon decode: {e}"))?;
    if let Some(window) = app.get_webview_window("main") {
        window
            .set_icon(win_img)
            .map_err(|e| format!("set window icon: {e}"))?;
    }

    // macOS Dock is application-level; window.set_icon alone often does not change it.
    #[cfg(target_os = "macos")]
    {
        if let Err(err) = set_macos_application_icon(icon_bytes) {
            tracing::warn!("set macOS application icon: {err}");
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn set_macos_application_icon(png_bytes: &[u8]) -> Result<(), String> {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let mtm = MainThreadMarker::new().ok_or_else(|| {
        "macOS application icon must be applied on the main thread".to_string()
    })?;
    let data = NSData::with_bytes(png_bytes);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| "failed to decode PNG into NSImage".to_string())?;
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: NSApplication is main-thread only; we hold MainThreadMarker.
    unsafe {
        app.setApplicationIconImage(Some(&image));
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .compact()
        .init();

    // ensure config/db exist; start async usage writer (prune + WAL checkpoint)
    let _ = config::load_config();
    let _ = config::load_auth();
    // Schema first so the first UI query never hits a missing table.
    match usage::UsageStore::open_default() {
        Ok(_) => tracing::info!("usage database ready"),
        Err(err) => tracing::error!("failed to initialize usage database: {err}"),
    }
    usage::init_log_writer();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(GatewayState::new())
        .manage(OAuthManager::new())
        .setup(|app| {
            let gateway = app.state::<GatewayState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                match start_gateway(gateway).await {
                    Ok(addr) => tracing::info!("gateway listening on {addr}"),
                    Err(err) => tracing::error!("failed to start gateway: {err}"),
                }
            });

            // Keep ~/.grok/auth.json fresh while Grok Build routing is enabled:
            // refresh pool token → userinfo probe → write only if IdP accepts it.
            integrations::start_grok_build_auth_maintainer();
            // Sequential silent SuperGrok quota refresh (never fan-out all accounts).
            quota::start_quota_refresh_maintainer();

            setup_tray(app)?;
            // Apply configured style immediately. Default is Dark (black bg);
            // main bundle icons also ship as Dark so Dock matches before this runs.
            apply_configured_app_icon(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let minimize = config::load_config()
                    .map(|c| c.minimize_to_tray)
                    .unwrap_or(true);
                // Always intercept close so we can either hide-to-tray or confirm quit.
                api.prevent_close();
                let app = window.app_handle().clone();
                if minimize {
                    // Keep tray; hide Dock / taskbar entry so the app is tray-only until reopened.
                    if let Some(main) = app.get_webview_window("main") {
                        hide_main_window_to_tray(&main);
                    } else {
                        let _ = window.hide();
                    }
                } else {
                    // Quit path: confirm first — closing stops the local gateway process.
                    if confirm_quit_and_stop_proxy() {
                        app.exit(0);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::start_server,
            commands::get_config,
            commands::update_config,
            commands::set_app_icon,
            commands::rotate_token,
            commands::list_model_options,
            commands::get_accounts,
            commands::upsert_account,
            commands::delete_account,
            commands::replace_accounts,
            commands::import_accounts,
            commands::convert_sso_accounts,
            commands::batch_delete_accounts,
            commands::batch_patch_accounts,
            commands::clear_account_cooldown,
            commands::refresh_account_quota,
            commands::refresh_all_account_quotas,
            commands::start_oauth_login,
            commands::get_usage_summary,
            commands::get_recent_logs,
            commands::get_heatmap,
            commands::clear_logs,
            commands::get_log_stats,
            commands::clear_logs_older_than,
            commands::clear_logs_range,
            commands::prune_logs_now,
            commands::get_integrations,
            commands::set_mcp_inject,
            commands::inject_agents_guide,
            commands::set_grok_build_inject,
            commands::restore_grok_build_backup,
            commands::import_to_cc_switch,
            commands::import_claude_to_cc_switch,
            commands::export_provider_snippet,
            commands::set_opencode_model_inject_cmd,
            commands::set_opencode_mcp_inject_cmd,
            commands::set_workbuddy_model_inject_cmd,
            commands::set_workbuddy_mcp_inject_cmd,
            commands::set_cursor_mcp_inject_cmd,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Re-apply once the event loop is ready so Dock/taskbar pick up the
            // configured style (bundle default is already Dark).
            if let tauri::RunEvent::Ready = event {
                apply_configured_app_icon(app);
            }
        });
}

fn apply_configured_app_icon(app: &AppHandle) {
    let style = config::load_config()
        .map(|c| c.app_icon)
        .unwrap_or_default();
    if let Err(err) = apply_app_icon(app, style) {
        tracing::warn!("apply app icon ({style:?}): {err}");
    }
}

/// Hide main window while keeping the tray icon. Removes Dock (macOS) / taskbar
/// (Windows/Linux) entry so the app is only reachable via the tray.
fn hide_main_window_to_tray(window: &WebviewWindow) {
    let _ = window.hide();
    // Hide from taskbar / app switcher list (no-op on some platforms if unsupported).
    if let Err(err) = window.set_skip_taskbar(true) {
        tracing::debug!("set_skip_taskbar(true): {err}");
    }
    #[cfg(target_os = "macos")]
    {
        // Accessory = no Dock icon; menu bar / tray still works.
        if let Err(err) = window
            .app_handle()
            .set_activation_policy(ActivationPolicy::Accessory)
        {
            tracing::debug!("set_activation_policy(Accessory): {err}");
        }
    }
}

/// Restore main window + Dock / taskbar presence after tray hide.
fn show_main_window_from_tray(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        if let Err(err) = app.set_activation_policy(ActivationPolicy::Regular) {
            tracing::debug!("set_activation_policy(Regular): {err}");
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        if let Err(err) = window.set_skip_taskbar(false) {
            tracing::debug!("set_skip_taskbar(false): {err}");
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Native confirm before quitting when "minimize to tray" is off.
/// Quitting ends the process and therefore stops the local gateway.
fn confirm_quit_and_stop_proxy() -> bool {
    rfd::MessageDialog::new()
        .set_title("退出 GrokGo")
        .set_description(
            "关闭窗口将退出应用，本地代理网关会停止，依赖本机网关的客户端将无法连接。\n\n确定退出吗？",
        )
        .set_buttons(rfd::MessageButtons::OkCancel)
        .set_level(rfd::MessageLevel::Warning)
        .show()
        == rfd::MessageDialogResult::Ok
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show_i = MenuItem::with_id(app, "show", "Show GrokGo", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

    let style = config::load_config()
        .map(|c| c.app_icon)
        .unwrap_or_default();
    let icon = Image::from_bytes(tray_bytes(style))?;

    let _tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        // macOS template: system tints black glyph for light/dark menu bar.
        .icon_as_template(cfg!(target_os = "macos"))
        .menu(&menu)
        .show_menu_on_left_click(cfg!(not(target_os = "macos")))
        .tooltip("GrokGo")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                show_main_window_from_tray(app);
            }
            "quit" => {
                // Explicit quit from tray — no second confirm (user chose Quit).
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // macOS: left-click shows window; menu via right-click / ctrl-click.
            // Windows: left-click also toggles window; menu on left when configured.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        hide_main_window_to_tray(&window);
                    } else {
                        show_main_window_from_tray(app);
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}
