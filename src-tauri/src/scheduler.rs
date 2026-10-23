//! 后台定时刷新额度，并在「额度耗尽 / 已重置」时发系统通知。

use crate::commands::{refresh_quotas_inner, AppState};
use crate::model::{AccountKind, Quota, Vault, WarmupSchedule};
use crate::quota::FetchOutcome;
use chrono::Timelike;
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
    start_scheduled(app.clone());
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

#[derive(Debug, Clone)]
pub struct ScheduledWarmup {
    pub id: String,
    pub date: String,
    pub schedule: WarmupSchedule,
}

/// 只在指定分钟入队，不对关机、睡眠或退出期间错过的时刻补跑。
fn collect_due(vault: &mut Vault, minute: u16, date: &str) -> Vec<ScheduledWarmup> {
    let mut accounts: Vec<_> = vault
        .accounts
        .iter_mut()
        .filter(|a| {
            a.kind == AccountKind::Official
                && a.warmup_schedule
                    .is_due(minute, date, a.last_scheduled_date.as_deref())
        })
        .collect();
    accounts.sort_by_key(|a| a.sort_index);
    accounts
        .into_iter()
        .map(|a| {
            a.last_scheduled_date = Some(date.to_string());
            ScheduledWarmup {
                id: a.id.clone(),
                date: date.to_string(),
                schedule: a.warmup_schedule.clone(),
            }
        })
        .collect()
}

fn claim_due(app: &AppHandle) -> Result<Vec<ScheduledWarmup>, String> {
    let now = chrono::Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let state = app.state::<AppState>();
    let mut vault = state.vault.lock().unwrap();
    let mut next = vault.clone();
    let jobs = collect_due(&mut next, (now.hour() * 60 + now.minute()) as u16, &date);
    if !jobs.is_empty() {
        // 必须先落盘再入队，持久化失败时不发送激活请求。
        crate::store::save(&next).map_err(|e| format!("{e}"))?;
        *vault = next;
    }
    Ok(jobs)
}

fn start_scheduled(app: AppHandle) {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let worker_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(job) = receiver.recv().await {
            crate::commands::run_scheduled_warmup(&worker_app, job).await;
        }
    });
    tauri::async_runtime::spawn(async move {
        loop {
            match claim_due(&app) {
                Ok(jobs) => {
                    for job in jobs {
                        if sender.send(job).is_err() {
                            return;
                        }
                    }
                }
                Err(error) => eprintln!("[codex-hub] 定时激活入队失败：{error}"),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

/// 判断账号是否已经耗尽（任一窗口打满，或服务端直接说限流）。
fn is_exhausted(q: &Quota) -> bool {
    if q.limit_reached {
        return true;
    }
    let full =
        |w: Option<&crate::model::QuotaWindow>| w.map(|w| w.used_percent >= 100.0).unwrap_or(false);
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
    let _ = app.notification().builder().title(title).body(body).show();
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    fn account(id: &str, kind: &str) -> crate::model::Account {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "kind": kind, "auth": {}, "created_at": "2026-10-23",
            "warmup_schedule": { "enabled": true, "quiet_start": 1320, "quiet_end": 540, "activate_at": 480 }
        })).unwrap()
    }

    #[test]
    fn scheduled_jobs_are_daily_persistent_and_sorted() {
        let mut vault = Vault::default();
        let mut later = account("later", "official");
        later.sort_index = 2;
        vault.accounts = vec![
            later,
            account("first", "official"),
            account("third-party", "third_party"),
        ];
        assert!(collect_due(&mut vault, 479, "2026-10-23").is_empty());
        let jobs = collect_due(&mut vault, 480, "2026-10-23");
        assert_eq!(
            jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
            vec!["first", "later"]
        );
        let mut restored: Vault =
            serde_json::from_str(&serde_json::to_string(&vault).unwrap()).unwrap();
        assert!(collect_due(&mut restored, 480, "2026-10-23").is_empty());
        assert!(collect_due(&mut restored, 481, "2026-10-24").is_empty());
        assert_eq!(collect_due(&mut restored, 480, "2026-10-24").len(), 2);
    }
}
