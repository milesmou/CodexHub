//! 应用入口：装配插件、托盘、后台刷新，以及窗口显示/隐藏逻辑。

// 这几个模块开放出去，方便 examples/ 下的开发工具直接复用
// （例如 dump_vault 用来检查账号库里到底存了什么）
pub mod accounts;
pub mod ccswitch;
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
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 把主窗口显示出来并聚焦。
pub fn show_window(app: &AppHandle) {
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
        } else {
            let _ = win.show();
            let _ = win.unminimize();
            let _ = win.set_focus();
        }
    }
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
            eprintln!("[codex-helper] 全局快捷键 {combo} 注册失败：{e}");
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
            eprintln!("[codex-helper] 设置开机自启失败：{e}");
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
        .manage(AppState::new(vault))
        .manage(scheduler::NotifyState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_accounts,
            commands::refresh_quotas,
            commands::switch_account,
            commands::import_from_ccswitch,
            commands::delete_account,
            commands::update_account,
            commands::get_settings,
            commands::set_settings,
            commands::get_paths,
            commands::open_data_dir,
            commands::pick_auth_file,
            commands::preview_auth,
            commands::read_current_auth_text,
            commands::read_current_config_text,
            commands::read_account_credentials,
            commands::create_account,
            commands::update_account_credentials,
            commands::warmup_account,
            commands::warmup_all_dormant,
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
            scheduler::start(handle.clone());

            // 首次启动账号库为空时，自动从 cc-switch 导一次，省得用户手动点
            let is_empty = {
                let state = handle.state::<AppState>();
                let vault = state.vault.lock().unwrap();
                vault.accounts.is_empty()
            };
            if is_empty {
                match commands::import_from_ccswitch_inner(&handle) {
                    Ok(r) if r.imported > 0 => {
                        eprintln!("[codex-helper] 首次启动自动导入：{}", r.message);
                    }
                    _ => {}
                }
            }

            // 拉一次额度，界面打开就有数据
            let refresh_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                let _ = commands::refresh_quotas_inner(&refresh_handle, None).await;
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // 点关闭时按设置最小化到托盘，而不是真的退出
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let minimize = {
                    let state = app.state::<AppState>();
                    let vault = state.vault.lock().unwrap();
                    vault.settings.minimize_to_tray
                };
                if minimize {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Codex 多账号管家启动失败");
}
