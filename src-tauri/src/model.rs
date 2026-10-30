//! 数据模型定义：账号、额度窗口、账号库结构。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 账号类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    /// 官方 ChatGPT 账号（OAuth 登录），有 5 小时 / 每周订阅额度
    Official,
    /// 第三方中转或纯 API Key，按量计费，没有订阅额度窗口
    ThirdParty,
}

/// 一个账号的完整记录（含凭证，落盘时会整体加密）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    /// 用户自定义备注名，比如「欢哥的GPT」
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    pub kind: AccountKind,
    /// 完整的 auth.json 内容，切换时原样写回 ~/.codex/auth.json
    pub auth: serde_json::Value,
    /// 可选的第三方服务 config.toml 片段；projects/history 始终使用共享配置
    #[serde(default)]
    pub config: Option<String>,
    /// 第三方服务地址；官方账号为空
    #[serde(default)]
    pub base_url: Option<String>,
    /// 第三方服务提供的模型列表；第一项作为默认模型
    #[serde(default)]
    pub models: Vec<String>,
    /// 来源标记：current / manual / 历史来源
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub sort_index: i32,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub warmup_schedule: WarmupSchedule,
    /// 本地日期；入队前持久化，防止重启后重复执行当天的定时任务。
    #[serde(default)]
    pub last_scheduled_date: Option<String>,
    /// 已入队但尚未完成的任务，供失败重试和重启恢复。
    #[serde(default)]
    pub pending_scheduled_warmup: Option<PendingScheduledWarmup>,
    /// 结果尚未确认的重置卡兑换；重试必须沿用同一卡和请求 ID。
    #[serde(default)]
    pub pending_reset_redemption: Option<ResetRedemption>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetRedemption {
    pub credit_id: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetCredit {
    pub id: String,
    pub expires_at: Option<i64>,
    pub granted_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetCredits {
    pub available_count: u64,
    /// 只保留当前套餐支持、未过期且可用的卡，最先到期的排在前面。
    pub credits: Vec<ResetCredit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingScheduledWarmup {
    pub date: String,
    pub schedule: WarmupSchedule,
    /// Unix 秒；同一账号重试期间不占用其他账号的执行队列。
    pub retry_after: i64,
}

/// 每日定时激活，时间以本机时区的午夜起分钟数表示。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WarmupSchedule {
    pub enabled: bool,
    pub quiet_start: u16,
    pub quiet_end: u16,
    pub activate_at: u16,
}

impl Default for WarmupSchedule {
    fn default() -> Self {
        Self {
            enabled: false,
            quiet_start: 0,
            quiet_end: 540,
            activate_at: 480,
        }
    }
}

impl WarmupSchedule {
    /// 开始包含、结束不包含；起止相同表示全天管控。
    pub fn contains(&self, minute: u16) -> bool {
        if self.quiet_start < self.quiet_end {
            minute >= self.quiet_start && minute < self.quiet_end
        } else {
            minute >= self.quiet_start || minute < self.quiet_end
        }
    }

    pub fn suppresses_auto(&self, minute: u16) -> bool {
        self.enabled && self.contains(minute)
    }

    pub fn validate(&self) -> Result<(), String> {
        if [self.quiet_start, self.quiet_end, self.activate_at]
            .iter()
            .any(|&m| m >= 1440)
        {
            return Err("时间必须在 00:00 至 23:59 之间".into());
        }
        if self.enabled && !self.contains(self.activate_at) {
            return Err("激活时刻必须位于管控时间段内（不含结束时刻）".into());
        }
        Ok(())
    }

    pub fn is_due(&self, minute: u16, date: &str, last_date: Option<&str>) -> bool {
        self.enabled
            && self.validate().is_ok()
            && minute == self.activate_at
            && last_date != Some(date)
    }

    /// 串行排队允许稍晚执行，但不可跨出该次管控区间后再次补跑。
    pub fn allows_queued(&self, date: &str, now: chrono::NaiveDateTime) -> bool {
        if !self.enabled || self.validate().is_err() {
            return false;
        }
        let Ok(day) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            return false;
        };
        let Some(start) = day.and_hms_opt(
            (self.activate_at / 60) as u32,
            (self.activate_at % 60) as u32,
            0,
        ) else {
            return false;
        };
        let remaining = if self.quiet_start == self.quiet_end {
            1440 - self.activate_at
        } else {
            (self.quiet_end + 1440 - self.activate_at) % 1440
        };
        let end = start + chrono::Duration::minutes(remaining as i64);
        now >= start && now < end
    }
}

/// 单个额度窗口。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuotaWindow {
    /// 已用百分比 0-100
    pub used_percent: f64,
    /// 窗口时长（秒），18000 = 5 小时，604800 = 7 天
    pub window_seconds: i64,
    /// 重置时刻（unix 秒）
    pub reset_at: i64,
    /// 距重置剩余秒数，接口直接返回，比本地算更准
    pub reset_after_seconds: i64,
}

impl QuotaWindow {
    /// 这个窗口是不是「从未启动」。
    ///
    /// Codex 的 5 小时窗口是「用一次才开始计时」的：账号长期不用时，
    /// 接口会返回 `used_percent = 0` 且 `reset_after_seconds` 恰好等于
    /// 整个窗口长度（18000 秒），说明时钟压根没开始走，也就永远等不到重置。
    ///
    /// 判据用「一点没用 + 倒计时等于满窗口」两条一起卡，比只看 `used_percent`
    /// 准：只看百分比的话，刚启动不久的窗口也会被误判成未启动。
    pub fn is_dormant(&self) -> bool {
        self.window_seconds > 0
            && self.used_percent < 0.5
            && self.reset_after_seconds >= self.window_seconds
    }
}

/// 一次额度查询的结果。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Quota {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    /// 主窗口（通常是 5 小时）
    #[serde(default)]
    pub primary: Option<QuotaWindow>,
    /// 次窗口（通常是每周）
    #[serde(default)]
    pub secondary: Option<QuotaWindow>,
    /// 代码审查额度（部分套餐单独计）
    #[serde(default)]
    pub code_review: Option<QuotaWindow>,
    #[serde(default)]
    pub credits_balance: Option<String>,
    #[serde(default)]
    pub has_credits: bool,
    #[serde(default)]
    pub unlimited: bool,
    /// 是否已经撞到限流
    #[serde(default)]
    pub limit_reached: bool,
    #[serde(default)]
    pub reset_credits: Option<ResetCredits>,
    #[serde(default)]
    pub reset_credits_error: Option<String>,
    /// 查询时间（unix 秒）
    #[serde(default)]
    pub fetched_at: i64,
}

impl Quota {
    /// 5 小时窗口（primary）是否处于未启动状态。
    pub fn primary_dormant(&self) -> bool {
        self.ok && self.primary.as_ref().is_some_and(QuotaWindow::is_dormant)
    }

    /// 周额度是否已经耗尽。
    pub fn secondary_exhausted(&self) -> bool {
        self.secondary
            .as_ref()
            .is_some_and(|window| window.used_percent >= 100.0)
    }

    /// 休眠的 5 小时窗口当前是否真的能通过会话激活。
    ///
    /// 周额度打满后，服务端会在创建会话前直接限流，请求无法触发主窗口计时。
    /// 此时先等待周窗口恢复；下一次额度刷新会让它重新进入可激活队列。
    pub fn primary_warmup_ready(&self) -> bool {
        self.primary_dormant() && !self.secondary_exhausted() && !self.limit_reached
    }
}

/// 发给前端的账号视图（剥离 auth 凭证，避免凭证进入 WebView）。
#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub kind: AccountKind,
    pub is_current: bool,
    pub quota: Option<Quota>,
    pub sort_index: i32,
    pub hidden: bool,
    pub source: Option<String>,
    pub warmup_schedule: WarmupSchedule,
    pub reset_pending: bool,
}

/// 定时刷新与行为相关的设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// 后台刷新间隔（秒），0 表示关闭自动刷新
    pub refresh_interval_secs: u64,
    /// 开机自动启动
    pub startup: bool,
    /// 通过开机自启启动时不显示主窗口
    #[serde(default)]
    pub startup_silent: bool,
    /// 全局快捷键，空字符串表示不注册
    pub shortcut: String,
    /// 额度耗尽时发通知
    pub notify_on_limit: bool,
    /// 额度重置时发通知
    pub notify_on_reset: bool,
    /// 关闭窗口时最小化到托盘而不是退出
    pub minimize_to_tray: bool,
    /// 是否让任务栏状态浮层始终显示；托盘图标不受此项影响。
    #[serde(default = "default_true")]
    pub taskbar_status_enabled: bool,
    /// 自动激活：刷新时发现某账号的 5 小时窗口从未启动，就替它发一条会话把窗口点着。
    #[serde(default = "default_true")]
    pub warmup_auto: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_interval_secs: 300,
            startup: true,
            startup_silent: false,
            shortcut: "Ctrl+Alt+C".to_string(),
            notify_on_limit: false,
            notify_on_reset: true,
            minimize_to_tray: true,
            taskbar_status_enabled: true,
            warmup_auto: true,
        }
    }
}

/// 落盘的账号库。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Vault {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// 最近一次额度查询的缓存，key 是账号 id
    #[serde(default)]
    pub quota_cache: HashMap<String, Quota>,
    /// 当前激活账号 id
    #[serde(default)]
    pub current_id: Option<String>,
    #[serde(default)]
    pub settings: Settings,
}

fn default_version() -> u32 {
    1
}

impl Vault {
    pub fn is_current(&self, id: &str) -> bool {
        self.current_id.as_deref() == Some(id)
    }

    pub fn find(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.id == id)
    }

    /// 判断某个 auth 对象是否已存在于账号库（用 account_id + 邮箱做指纹）。
    pub fn contains_auth(&self, auth: &serde_json::Value) -> bool {
        let fp = auth_fingerprint(auth);
        fp.is_some()
            && self
                .accounts
                .iter()
                .any(|a| auth_fingerprint(&a.auth) == fp)
    }
}

/// 从 auth.json 中提取稳定指纹，用于去重和识别当前账号。
///
/// refresh_token 每次刷新后都可能轮换，不能把它作为已有账号的主身份，
/// 否则同一个账号刷新一次就会匹配失败。官方账号优先使用稳定的 account_id；
/// 只有旧格式里没有 account_id 时，才退回 refresh_token 前缀。
pub fn auth_fingerprint(auth: &serde_json::Value) -> Option<String> {
    let t = auth.get("tokens")?;
    let acc = t.get("account_id").and_then(|v| v.as_str()).unwrap_or("");
    let rt = t
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !acc.is_empty() {
        return Some(format!("account:{acc}"));
    }
    if !rt.is_empty() {
        return Some(format!("refresh:{}", &rt[..rt.len().min(16)]));
    }
    None
}

#[cfg(test)]
mod auth_fingerprint_tests {
    use super::{auth_fingerprint, Account, Quota, QuotaWindow, WarmupSchedule};
    use serde_json::json;

    #[test]
    fn schedule_boundaries_and_cross_midnight() {
        let mut schedule = WarmupSchedule {
            enabled: true,
            quiet_start: 1320,
            quiet_end: 540,
            activate_at: 480,
        };
        assert!(schedule.validate().is_ok());
        for minute in [1320, 1439, 0, 480, 539] {
            assert!(schedule.suppresses_auto(minute));
        }
        for minute in [540, 900, 1319] {
            assert!(!schedule.suppresses_auto(minute));
        }
        schedule.quiet_start = 420;
        assert!(!schedule.suppresses_auto(419));
        assert!(schedule.suppresses_auto(420));
        assert!(!schedule.suppresses_auto(540));
        schedule.enabled = false;
        assert!(!schedule.suppresses_auto(480));
        assert!(!schedule.is_due(480, "2026-10-23", None));
    }

    #[test]
    fn schedule_validation_and_full_day() {
        let mut schedule = WarmupSchedule {
            enabled: true,
            ..Default::default()
        };
        schedule.activate_at = schedule.quiet_end;
        assert!(schedule.validate().is_err());
        schedule.quiet_end = schedule.quiet_start;
        assert!(schedule.validate().is_ok());
        for minute in [0, 539, 900, 1439] {
            assert!(schedule.suppresses_auto(minute));
        }
        assert!(schedule.is_due(540, "2026-10-23", None));
        assert!(!schedule.is_due(540, "2026-10-23", Some("2026-10-23")));
        schedule.quiet_start = 1440;
        assert!(schedule.validate().is_err());
    }

    #[test]
    fn existing_accounts_default_to_no_schedule() {
        let account: Account = serde_json::from_value(json!({
            "id": "old", "name": "old", "kind": "official", "auth": {}, "created_at": "2026-10-23"
        }))
        .unwrap();
        assert!(!account.warmup_schedule.enabled);
        assert_eq!(account.last_scheduled_date, None);
    }

    #[test]
    fn queued_schedule_expires_at_its_own_interval_end() {
        let mut schedule = WarmupSchedule {
            enabled: true,
            quiet_start: 1320,
            quiet_end: 540,
            activate_at: 480,
        };
        let time =
            |value: &str| chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").unwrap();
        assert!(schedule.allows_queued("2026-10-23", time("2026-10-23 08:30")));
        assert!(!schedule.allows_queued("2026-10-23", time("2026-10-23 09:00")));
        assert!(!schedule.allows_queued("2026-10-23", time("2026-10-23 22:00")));
        assert!(!schedule.allows_queued("2026-10-23", time("2026-10-23 07:59")));
        schedule.activate_at = 1380;
        assert!(schedule.allows_queued("2026-10-23", time("2026-10-24 00:30")));
        assert!(!schedule.allows_queued("2026-10-23", time("2026-10-24 09:00")));
    }

    #[test]
    fn fingerprint_survives_refresh_token_rotation() {
        let before = json!({"tokens": {"account_id": "acc-1", "refresh_token": "old"}});
        let after = json!({"tokens": {"account_id": "acc-1", "refresh_token": "new"}});
        assert_eq!(auth_fingerprint(&before), auth_fingerprint(&after));
    }

    #[test]
    fn dormant_primary_waits_until_weekly_quota_recovers() {
        let dormant = QuotaWindow {
            used_percent: 0.0,
            window_seconds: 18_000,
            reset_at: 0,
            reset_after_seconds: 18_000,
        };
        let mut quota = Quota {
            ok: true,
            primary: Some(dormant),
            secondary: Some(QuotaWindow {
                used_percent: 100.0,
                window_seconds: 604_800,
                reset_at: 0,
                reset_after_seconds: 3_600,
            }),
            ..Default::default()
        };

        assert!(quota.primary_dormant());
        assert!(!quota.primary_warmup_ready());

        quota.secondary.as_mut().unwrap().used_percent = 99.0;
        assert!(quota.primary_warmup_ready());
    }
}
