//! 后台定时刷新额度，并在「额度耗尽 / 已重置」时发系统通知。

use crate::commands::{refresh_quotas_inner, AppState};
use crate::model::{AccountKind, PendingScheduledWarmup, Quota, Vault, WarmupSchedule};
use crate::quota::FetchOutcome;
use chrono::Timelike;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, Mutex};
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

/// 新任务仍只在指定分钟入队；已入队任务可在原管控区间内重试。
fn collect_due(
    vault: &mut Vault,
    now: chrono::NaiveDateTime,
    timestamp: i64,
    in_flight: &HashSet<String>,
) -> (Vec<ScheduledWarmup>, bool) {
    let minute = (now.hour() * 60 + now.minute()) as u16;
    let date = now.format("%Y-%m-%d").to_string();
    let mut changed = false;
    let mut accounts: Vec<_> = vault.accounts.iter_mut().collect();
    accounts.sort_by_key(|a| a.sort_index);
    let mut jobs = Vec::new();
    for a in accounts {
        if a.pending_scheduled_warmup.as_ref().is_some_and(|pending| {
            a.kind != AccountKind::Official
                || pending.schedule != a.warmup_schedule
                || !pending.schedule.allows_queued(&pending.date, now)
        }) {
            a.pending_scheduled_warmup = None;
            changed = true;
        }
        if a.kind == AccountKind::Official
            && a.warmup_schedule
                .is_due(minute, &date, a.last_scheduled_date.as_deref())
        {
            a.last_scheduled_date = Some(date.clone());
            a.pending_scheduled_warmup = Some(PendingScheduledWarmup {
                date: date.clone(),
                schedule: a.warmup_schedule.clone(),
                retry_after: timestamp,
            });
            changed = true;
        }
        if let Some(pending) = a.pending_scheduled_warmup.as_mut() {
            if pending.retry_after > timestamp || in_flight.contains(&a.id) {
                continue;
            }
            // 先持久化保护期。执行途中重启也不会立即重复发送请求。
            pending.retry_after = timestamp + 1800;
            changed = true;
            jobs.push(ScheduledWarmup {
                id: a.id.clone(),
                date: pending.date.clone(),
                schedule: pending.schedule.clone(),
            });
        }
    }
    (jobs, changed)
}

fn claim_due(app: &AppHandle, in_flight: &HashSet<String>) -> Result<Vec<ScheduledWarmup>, String> {
    let now = chrono::Local::now();
    let state = app.state::<AppState>();
    let mut vault = state.vault.lock().unwrap();
    let mut next = vault.clone();
    let (jobs, changed) = collect_due(&mut next, now.naive_local(), now.timestamp(), in_flight);
    if changed {
        // 必须先落盘再入队，持久化失败时不发送激活请求。
        crate::store::save(&next).map_err(|e| format!("{e}"))?;
        for account in &next.accounts {
            if account.pending_scheduled_warmup.is_none()
                && vault
                    .find(&account.id)
                    .is_some_and(|old| old.pending_scheduled_warmup.is_some())
            {
                record(
                    "expired",
                    &account.id,
                    "设置已变更或已离开原管控区间，停止重试",
                );
            }
        }
        *vault = next;
    }
    Ok(jobs)
}

pub enum ScheduledResult {
    Finished,
    RetryAfter(i64),
}

fn apply_result(
    vault: &mut Vault,
    job: &ScheduledWarmup,
    result: &ScheduledResult,
    now: i64,
) -> bool {
    let Some(account) = vault.find_mut(&job.id) else {
        return false;
    };
    let Some(pending) = account.pending_scheduled_warmup.as_mut() else {
        return false;
    };
    if pending.date != job.date || pending.schedule != job.schedule {
        return false;
    }
    match result {
        ScheduledResult::Finished => account.pending_scheduled_warmup = None,
        ScheduledResult::RetryAfter(seconds) => pending.retry_after = now + seconds,
    }
    true
}

fn finish_job(
    app: &AppHandle,
    job: &ScheduledWarmup,
    result: ScheduledResult,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut vault = state.vault.lock().unwrap();
    let mut next = vault.clone();
    if !apply_result(&mut next, job, &result, chrono::Utc::now().timestamp()) {
        return Ok(());
    }
    crate::store::save(&next).map_err(|e| format!("{e}"))?;
    *vault = next;
    if let ScheduledResult::RetryAfter(seconds) = result {
        record(
            "retry",
            &job.id,
            &format!("{seconds} 秒后重试，仅在原管控区间内执行"),
        );
    }
    Ok(())
}

/// 发布版没有控制台，将定时任务事件保存在本地；只记录账号 id 和执行结果。
pub fn record(event: &str, id: &str, message: &str) {
    static LOG_LOCK: Mutex<()> = Mutex::new(());
    let _lock = LOG_LOCK.lock().unwrap();
    let result = (|| -> std::io::Result<()> {
        let dir = crate::store::data_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("scheduled-warmup.jsonl");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
            let previous = dir.join("scheduled-warmup.previous.jsonl");
            if previous.exists() {
                std::fs::remove_file(&previous)?;
            }
            std::fs::rename(&path, previous)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "time": chrono::Local::now().to_rfc3339(), "event": event,
                "account_id": id, "message": message,
            })
        )
    })();
    eprintln!("[codex-hub] 定时激活 {event}（{id}）：{message}");
    if let Err(error) = result {
        eprintln!("[codex-hub] 定时日志写入失败：{error}");
    }
}

fn start_scheduled(app: AppHandle) {
    record(
        "scheduler_started",
        "",
        "定时调度已启动，已入队任务支持区间内失败重试",
    );
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<ScheduledWarmup>();
    let in_flight = Arc::new(Mutex::new(HashSet::new()));
    let worker_in_flight = in_flight.clone();
    let worker_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(job) = receiver.recv().await {
            record("started", &job.id, &format!("执行 {} 的定时任务", job.date));
            let result = crate::commands::run_scheduled_warmup(&worker_app, job.clone()).await;
            if let Err(error) = finish_job(&worker_app, &job, result) {
                record("persist_failed", &job.id, &error);
            }
            worker_in_flight.lock().unwrap().remove(&job.id);
        }
    });
    tauri::async_runtime::spawn(async move {
        loop {
            {
                let mut active = in_flight.lock().unwrap();
                match claim_due(&app, &active) {
                    Ok(jobs) => {
                        for job in jobs {
                            record("queued", &job.id, &format!("任务日期 {}", job.date));
                            active.insert(job.id.clone());
                            if sender.send(job).is_err() {
                                return;
                            }
                        }
                    }
                    Err(error) => record("queue_failed", "", &error),
                }
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

    fn time(value: &str) -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").unwrap()
    }

    fn collect_due(vault: &mut Vault, minute: u16, date: &str) -> Vec<ScheduledWarmup> {
        let now = time(&format!("{date} {:02}:{:02}", minute / 60, minute % 60));
        super::collect_due(vault, now, now.and_utc().timestamp(), &HashSet::new()).0
    }

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

    #[test]
    fn pending_job_survives_restart_but_does_not_duplicate_while_running() {
        let mut vault = Vault::default();
        vault.accounts = vec![account("first", "official")];
        assert_eq!(collect_due(&mut vault, 480, "2026-10-26").len(), 1);
        let mut restored: Vault =
            serde_json::from_str(&serde_json::to_string(&vault).unwrap()).unwrap();
        assert!(collect_due(&mut restored, 509, "2026-10-26").is_empty());
        let now = time("2026-10-26 08:30");
        let active = HashSet::from(["first".to_string()]);
        assert!(
            super::collect_due(&mut restored, now, now.and_utc().timestamp(), &active)
                .0
                .is_empty()
        );
        assert_eq!(collect_due(&mut restored, 510, "2026-10-26").len(), 1);
        // 同一分钟下一轮扫描也不会重复入队。
        assert!(collect_due(&mut restored, 510, "2026-10-26").is_empty());
    }

    #[test]
    fn retries_expire_at_interval_end_and_do_not_block_other_accounts() {
        let mut vault = Vault::default();
        let mut second = account("second", "official");
        second.warmup_schedule.activate_at = 485;
        vault.accounts = vec![account("first", "official"), second];
        assert_eq!(collect_due(&mut vault, 480, "2026-10-26").len(), 1);
        let jobs = collect_due(&mut vault, 485, "2026-10-26");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "second");
        assert!(collect_due(&mut vault, 540, "2026-10-26").is_empty());
        assert!(vault
            .accounts
            .iter()
            .all(|a| a.pending_scheduled_warmup.is_none()));
        assert!(collect_due(&mut vault, 1320, "2026-10-26").is_empty());
    }

    #[test]
    fn retries_cross_midnight_and_cancel_on_schedule_change() {
        let mut vault = Vault::default();
        let mut first = account("first", "official");
        first.warmup_schedule.activate_at = 1380;
        vault.accounts = vec![first];
        assert_eq!(collect_due(&mut vault, 1380, "2026-10-25").len(), 1);
        let jobs = collect_due(&mut vault, 30, "2026-10-26");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].date, "2026-10-25");
        vault.accounts[0].warmup_schedule.activate_at = 1370;
        assert!(collect_due(&mut vault, 60, "2026-10-26").is_empty());
        assert!(vault.accounts[0].pending_scheduled_warmup.is_none());
    }

    #[test]
    fn failed_query_retries_after_a_minute_and_completion_stops_the_daily_job() {
        let mut vault = Vault::default();
        vault.accounts = vec![account("first", "official")];
        let job = collect_due(&mut vault, 480, "2026-10-26").remove(0);
        let now = time("2026-10-26 08:00").and_utc().timestamp();
        assert!(apply_result(
            &mut vault,
            &job,
            &ScheduledResult::RetryAfter(60),
            now
        ));
        let mut restored: Vault =
            serde_json::from_str(&serde_json::to_string(&vault).unwrap()).unwrap();
        assert!(collect_due(&mut restored, 480, "2026-10-26").is_empty());
        assert_eq!(collect_due(&mut restored, 481, "2026-10-26").len(), 1);
        assert!(apply_result(
            &mut restored,
            &job,
            &ScheduledResult::Finished,
            now + 60
        ));
        assert!(collect_due(&mut restored, 481, "2026-10-26").is_empty());
        assert!(collect_due(&mut restored, 520, "2026-10-26").is_empty());
        assert_eq!(collect_due(&mut restored, 480, "2026-10-27").len(), 1);
        // 旧任务的异步结果不能清掉新一天的任务。
        assert!(!apply_result(
            &mut restored,
            &job,
            &ScheduledResult::Finished,
            now + 86400
        ));
        assert!(restored.accounts[0].pending_scheduled_warmup.is_some());
    }
}
