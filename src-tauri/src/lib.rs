//! 应用入口：装配插件、托盘、后台刷新，以及窗口显示/隐藏逻辑。

// 这几个模块开放出去，方便 examples/ 下的开发工具直接复用
// （例如 dump_vault 用来检查账号库里到底存了什么）
pub mod accounts;
pub mod codex;
pub mod codexapp;
pub mod crypto;
pub mod model;
pub mod quota;
pub mod stats;
pub mod store;
pub mod warmup;

mod commands;
mod scheduler;
mod tray;

use commands::AppState;
use model::StatusMode;
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 把主窗口显示出来并聚焦。
pub fn show_window(app: &AppHandle) {
    if let Some(status) = app.get_webview_window("taskbar-status") {
        let _ = status.hide();
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// 托盘左键 / 全局快捷键：显示中则收起，否则弹出。
pub fn toggle_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
            show_background_surface(app);
        } else {
            show_window(app);
        }
    }
}

/// Windows 在主窗口最小化或隐藏时会重新调整任务栏的 Z 序，顺手把状态浮层
/// 提回任务栏上方。状态窗不获取焦点，所以不会打断用户当前操作。
fn show_background_surface(app: &AppHandle) {
    let mode = {
        let state = app.state::<AppState>();
        let vault = state.vault.lock().unwrap();
        vault.settings.status_mode
    };
    if let Some(status) = app.get_webview_window("taskbar-status") {
        if mode == StatusMode::Taskbar {
            let _ = status.show();
            let _ = status.set_always_on_top(true);
        } else {
            let _ = status.hide();
        }
    }
}

/// 强制把状态窗放到 Windows TOPMOST 队列最前面，但绝不激活或抢焦点。
#[cfg(windows)]
fn force_status_topmost(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            SetWindowPos(
                hwnd.0,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

#[cfg(not(windows))]
fn force_status_topmost(window: &tauri::WebviewWindow) {
    let _ = window.set_always_on_top(true);
}

/// 任务栏本身也是 TOPMOST 窗口，用户点击它时会超过普通置顶窗。
/// 浮层可见期间低频重排一次 Z 序，确保点击任务栏后能立即恢复。
fn start_status_topmost_guard(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let taskbar_mode = {
                let state = app.state::<AppState>();
                let vault = state.vault.lock().unwrap();
                vault.settings.status_mode == StatusMode::Taskbar
            };
            if !taskbar_mode {
                continue;
            }
            if let Some(status) = app.get_webview_window("taskbar-status") {
                if status.is_visible().unwrap_or(false) {
                    force_status_topmost(&status);
                }
            }
        }
    });
}

/// 应用设置里的「全局快捷键」和「开机自启」。
///
/// 调用前请确保没有持有账号库的锁（这里会短暂加锁读设置）。
pub fn apply_settings(app: &AppHandle) {
    let settings = {
        let state = app.state::<AppState>();
        let vault = state.vault.lock().unwrap();
        vault.settings.clone()
    };

    // 先全部注销再按当前设置注册，避免残留旧的快捷键
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let combo = settings.shortcut.trim();
    if !combo.is_empty() {
        if let Err(e) = gs.register(combo) {
            eprintln!("[codex-hub] 全局快捷键 {combo} 注册失败：{e}");
        }
    }

    // 开机自启
    {
        use tauri_plugin_autostart::ManagerExt;
        let autolaunch = app.autolaunch();
        let result = if settings.startup {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        if let Err(e) = result {
            eprintln!("[codex-hub] 设置开机自启失败：{e}");
        }
    }


    // 两种常驻入口互斥。主窗口显示时，任务栏浮层暂时隐藏。
    if let Some(tray) = app.tray_by_id(tray::TRAY_ID) {
        let _ = tray.set_visible(settings.status_mode == StatusMode::Tray);
    }
    if let Some(status) = app.get_webview_window("taskbar-status") {
        let main_hidden = app
            .get_webview_window("main")
            .map(|w| !w.is_visible().unwrap_or(false) || w.is_minimized().unwrap_or(false))
            .unwrap_or(true);
        if settings.status_mode == StatusMode::Taskbar && main_hidden {
            let _ = status.show();
            let _ = status.set_always_on_top(true);
        } else {
            let _ = status.hide();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let vault = store::load();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        .on_menu_event(tray::handle_menu_event)
        .manage(AppState::new(vault))
        .manage(scheduler::NotifyState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_accounts,
            commands::refresh_quotas,
            commands::refresh_active_ids,
            commands::switch_account,
            commands::delete_account,
            commands::update_account,
            commands::get_settings,
            commands::set_settings,
            commands::popup_status_menu,
            commands::taskbar_status_anchor,
            commands::get_paths,
            commands::open_data_dir,
            commands::pick_auth_file,
            commands::preview_auth,
            commands::read_current_auth_text,
            commands::read_current_config_text,
            commands::read_account_credentials,
            commands::fetch_provider_models,
            commands::create_account,
            commands::update_account_credentials,
            commands::warmup_account,
            commands::warmup_all_dormant,
            commands::warmup_active_ids,
            commands::codex_cli_path,
            commands::token_stats,
            commands::codex_app_status,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // 用磁盘上的 auth.json 校准「当前账号」，比信任上次记录更靠谱
            commands::sync_current_from_disk(&handle);

            tray::build(&handle)?;
            apply_settings(&handle);
            start_status_topmost_guard(handle.clone());
            scheduler::start(handle.clone());

            // 拉一次额度，界面打开就有数据
            let refresh_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                let _ = commands::refresh_quotas_inner(&refresh_handle, None).await;
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, WindowEvent::Resized(_)) {
                if window.is_minimized().unwrap_or(false) {
                    let _ = window.hide();
                    show_background_surface(window.app_handle());
                }
            }

            // 浮层模式关闭主窗口时必须驻留；托盘模式继续遵循原来的开关。
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    let app = window.app_handle();
                    let keep_running = {
                        let state = app.state::<AppState>();
                        let vault = state.vault.lock().unwrap();
                        vault.settings.minimize_to_tray
                            || vault.settings.status_mode == StatusMode::Taskbar
                    };
                    if keep_running {
                        api.prevent_close();
                        let _ = window.hide();
                        show_background_surface(app);
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Codex Hub 启动失败");
}
