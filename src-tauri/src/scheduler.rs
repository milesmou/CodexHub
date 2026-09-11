//! 后台定时刷新额度，并在「额度耗尽 / 已重置」时发系统通知。

use crate::commands::{refresh_quotas_inner, AppState};
use crate::model::Quota;
use crate::quota::FetchOutcome;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

/// 记录上一次看到的「是否已耗尽」，只在状态翻转时通知，避免每分钟骚扰一次。
#[derive(Default)]
pub struct NotifyState {
    pub last_limited: Mutex<HashMap<String, bool>>,
}

/// 启动后台刷新循环。间隔从设置里实时读取，改设置不用重启。
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            // 先取间隔，作用域结束后锁就释放了，绝不会跨越 await
            let interval = {
                let state = app.state::<AppState>();
                let vault = state.vault.lock().unwrap();
                vault.settings.refresh_interval_secs
            };

            if interval == 0 {
                // 关闭自动刷新时低频空转，方便随时改回来
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }

            tokio::time::sleep(Duration::from_secs(interval)).await;

            if let Err(e) = refresh_quotas_inner(&app, None).await {
                eprintln!("[codex-hub] 定时刷新失败：{e}");
            }
        }
    });
}

/// 判断账号是否已经耗尽（任一窗口打满，或服务端直接说限流）。
fn is_exhausted(q: &Quota) -> bool {
    if q.limit_reached {
        return true;
    }
    let full = |w: Option<&crate::model::QuotaWindow>| {
        w.map(|w| w.used_percent >= 100.0).unwrap_or(false)
    };
    full(q.primary.as_ref()) || full(q.secondary.as_ref())
}

/// 刷新完检查一遍，该通知就通知。
pub fn check_and_notify(app: &AppHandle, results: &[(String, FetchOutcome)]) {
    let (settings, names) = {
        let state = app.state::<AppState>();
        let vault = state.vault.lock().unwrap();
        let names: HashMap<String, String> = vault
            .accounts
            .iter()
            .map(|a| (a.id.clone(), a.name.clone()))
            .collect();
        (vault.settings.clone(), names)
    };

    let notify_state = app.state::<NotifyState>();

    for (id, outcome) in results {
        if !outcome.quota.ok {
            continue;
        }
        let name = names
            .get(id)
            .cloned()
            .unwrap_or_else(|| "未知账号".to_string());
        let now_limited = is_exhausted(&outcome.quota);

        let previous = {
            let mut map = notify_state.last_limited.lock().unwrap();
            map.insert(id.clone(), now_limited)
        };

        match (previous, now_limited) {
            // 刚打满
            (Some(false), true) | (None, true) if settings.notify_on_limit => {
                send(
                    app,
                    "额度已耗尽",
                    &format!("「{name}」额度已用完，建议切到其他账号"),
                );
            }
            // 刚恢复
            (Some(true), false) if settings.notify_on_reset => {
                send(app, "额度已重置", &format!("「{name}」的额度已经恢复"));
            }
            _ => {}
        }
    }
}

fn send(app: &AppHandle, title: &str, body: &str) {
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}
