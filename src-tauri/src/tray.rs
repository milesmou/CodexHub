//! 系统托盘：菜单里直接列出租账号，点一下就切，不用开主窗口。

use crate::commands::{self, AppState};
use crate::codexapp;
use crate::model::{Account, AccountKind, Quota, Vault};
use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

/// 托盘图标 id，重建菜单时靠它找回托盘实例。
pub const TRAY_ID: &str = "codex-helper-tray";

/// 把「已用百分比」换算成「剩余百分比」文案。
///
/// 接口给的是已用，这里减一下 —— 菜单上要看的是「还剩多少能用」。
fn remaining_text(w: Option<&crate::model::QuotaWindow>) -> String {
    w.map(|w| format!("{:.0}%", (100.0 - w.used_percent).max(0.0)))
        .unwrap_or_else(|| "—".to_string())
}

/// 托盘里一行账号的文案：`● 欢哥的GPT · 5h 98% / 周 49%`
///
/// 这里的百分比是**剩余**额度（接口给的是已用，减过了），跟主界面口径一致。
fn account_line(account: &Account, quota: Option<&Quota>, is_current: bool) -> String {
    let dot = if is_current { "●" } else { "○" };

    let tag = match quota {
        Some(q) if q.ok => format!(
            "5h {} / 周 {}",
            remaining_text(q.primary.as_ref()),
            remaining_text(q.secondary.as_ref())
        ),
        Some(_) => "查询失败".to_string(),
        None => match account.kind {
            AccountKind::ThirdParty => "第三方 · 按量".to_string(),
            AccountKind::Official => "未查询".to_string(),
        },
    };

    format!("{dot} {} · {tag}", account.name)
}

/// 悬停提示：显示当前账号和它的额度（同样是剩余）。
fn tooltip(vault: &Vault) -> String {
    let base = "CodexHelper";
    let Some(cur) = vault
        .current_id
        .as_deref()
        .and_then(|id| vault.find(id))
    else {
        return format!("{base}\n未选择账号");
    };
    let q = vault.quota_cache.get(&cur.id);
    let detail = q
        .filter(|q| q.ok)
        .map(|q| {
            format!(
                "5 小时 {} · 每周 {}",
                remaining_text(q.primary.as_ref()),
                remaining_text(q.secondary.as_ref())
            )
        })
        .unwrap_or_else(|| "额度未查询".to_string());
    format!("{base}\n当前：{}\n{detail}", cur.name)
}

/// 根据账号库拼菜单。
fn build_menu(app: &AppHandle, vault: &Vault) -> tauri::Result<Menu<Wry>> {
    let mut b = MenuBuilder::new(app);
    b = b.item(&MenuItemBuilder::with_id("show", "显示主窗口").build(app)?);
    b = b.separator();

    let visible: Vec<&Account> = vault.accounts.iter().filter(|a| !a.hidden).collect();
    if visible.is_empty() {
        b = b.item(
            &MenuItemBuilder::with_id("empty", "（还没有账号，打开主窗口添加）")
                .enabled(false)
                .build(app)?,
        );
    } else {
        for a in visible {
            let label = account_line(a, vault.quota_cache.get(&a.id), vault.is_current(&a.id));
            b = b.item(
                &MenuItemBuilder::with_id(format!("switch:{}", a.id), label).build(app)?,
            );
        }
    }

    b = b.separator();
    b = b.item(&MenuItemBuilder::with_id("refresh", "刷新全部额度").build(app)?);
    b = b.separator();
    b = b.item(&MenuItemBuilder::with_id("quit", "退出").build(app)?);
    b.build()
}

/// 首次创建托盘。
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    // 菜单和提示文案都从同一份快照算，避免临时变量借用问题
    let (menu, tip) = {
        let state = app.state::<AppState>();
        let vault = state.vault.lock().unwrap();
        (build_menu(app, &vault)?, tooltip(&vault))
    };

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(tip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::toggle_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

/// 账号库变化后重建菜单。
///
/// **注意**：调用方通常正持有账号库的锁，所以这里只接收 `&Vault`，绝不再去 lock，
/// 否则 `std::sync::Mutex` 不可重入会直接死锁。
pub fn rebuild(app: &AppHandle, vault: &Vault) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if let Ok(menu) = build_menu(app, vault) {
        let _ = tray.set_menu(Some(menu));
    }
    let _ = tray.set_tooltip(Some(tooltip(vault)));
}

/// 托盘菜单点击。
fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref().to_string();

    match id.as_str() {
        "show" => crate::show_window(app),

        "refresh" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = commands::refresh_quotas_inner(&app, None).await;
            });
        }

        "quit" => app.exit(0),

        other => {
            if let Some(account_id) = other.strip_prefix("switch:") {
                let app = app.clone();
                let id = account_id.to_string();
                // 确认框要等用户点，切进程要等它退干净 —— 这一串都是阻塞性质的活，
                // 只能丢到后台，绝不能占着主线程（托盘事件就是在主线程上来的）
                tauri::async_runtime::spawn(async move {
                    switch_from_tray(app, id).await;
                });
            }
        }
    }
}

/// 托盘发起的切换：先弹原生确认框，确认后再关 Codex → 切账号 → 重开 Codex。
async fn switch_from_tray(app: AppHandle, account_id: String) {
    let name = {
        let state = app.state::<AppState>();
        let vault = state.vault.lock().unwrap();
        vault
            .find(&account_id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| account_id.clone())
    };

    if !confirm_switch(&app, &name).await {
        return;
    }

    let handle = app.clone();
    let id = account_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        commands::switch_account_inner(&handle, &id, true)
    })
    .await;

    match result {
        Ok(Ok(outcome)) => {
            eprintln!("[codex-helper] {}", outcome.message);
            // persist() 已经发过 accounts-changed，主窗口开着的话会自己刷新
            let _ = app.emit("toast", outcome.message);
        }
        Ok(Err(e)) => {
            eprintln!("[codex-helper] 切换失败：{e}");
            let _ = app.emit("toast", format!("切换失败：{e}"));
        }
        Err(e) => {
            eprintln!("[codex-helper] 切换任务异常：{e}");
            let _ = app.emit("toast", format!("切换任务异常：{e}"));
        }
    }
}

/// 切换前的原生确认框。
///
/// 主窗口没开的时候也用得上，所以走系统弹窗而不是前端模态框。
/// 文案会先探一下 Codex 到底在不在跑，免得对着没开的用户说「将关闭 0 个进程」。
async fn confirm_switch(app: &AppHandle, name: &str) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    // 扫进程表很快，但仍然是阻塞调用，放阻塞线程池里稳妥
    let count = match tauri::async_runtime::spawn_blocking(codexapp::status).await {
        Ok(s) => s.count,
        Err(e) => {
            eprintln!("[codex-helper] 查询 Codex 进程失败：{e}");
            0
        }
    };

    let body = if count == 0 {
        format!("Codex 当前没在运行，将只把账号切换为「{name}」。")
    } else {
        format!(
            "切换到「{name}」需要重启 Codex。\n\n\
             会先关闭 {count} 个 Codex 进程，切换账号后自动重新打开。\n\
             Codex 里正在进行的对话会中断，未保存的内容可能丢失。"
        )
    };

    // 回调式弹窗 → oneshot 转成 await，这样下面能直接按顺序写
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(body)
        .title("切换账号")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "切换并重启".to_string(),
            "取消".to_string(),
        ))
        .show(move |ok| {
            let _ = tx.send(ok);
        });

    rx.await.unwrap_or(false)
}
