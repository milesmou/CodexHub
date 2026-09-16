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
    pub created_at: String,
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
        fp.is_some() && self.accounts.iter().any(|a| auth_fingerprint(&a.auth) == fp)
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
    use super::{auth_fingerprint, Quota, QuotaWindow};
    use serde_json::json;

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
