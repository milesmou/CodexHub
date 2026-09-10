//! 开发工具：把加密的账号库解开，看看里面到底存了什么。
//!
//! 只打印摘要（邮箱、套餐、额度、凭证长度），**不会输出任何 token 明文**。
//!
//! 用法：
//! ```bash
//! cd src-tauri
//! cargo run --release --example dump_vault
//! ```

use codex_helper_lib::model::AccountKind;
use codex_helper_lib::store;

fn main() {
    let vault = store::load();

    println!("账号库路径 : {}", store::vault_path().display());
    println!("账号数量   : {}", vault.accounts.len());
    println!(
        "当前账号   : {}",
        vault.current_id.as_deref().unwrap_or("（未标记）")
    );
    println!("刷新间隔   : {} 秒", vault.settings.refresh_interval_secs);
    println!("全局快捷键 : {}", vault.settings.shortcut);
    println!();

    for (i, a) in vault.accounts.iter().enumerate() {
        let kind = match a.kind {
            AccountKind::Official => "官方",
            AccountKind::ThirdParty => "第三方",
        };
        let cred = a
            .auth
            .get("tokens")
            .and_then(|t| t.get("access_token"))
            .and_then(|v| v.as_str())
            .map(|s| format!("{} 字符", s.len()))
            .unwrap_or_else(|| "无 OAuth 凭证".to_string());

        println!(
            "[{}] {} （{}）{}{}",
            i + 1,
            a.name,
            kind,
            if vault.is_current(&a.id) { "★当前 " } else { "" },
            a.source.as_deref().unwrap_or("-"),
        );
        println!("    邮箱     : {}", a.email.as_deref().unwrap_or("—"));
        println!("    套餐     : {}", a.plan_type.as_deref().unwrap_or("—"));
        println!("    cc_id    : {}", a.cc_id.as_deref().unwrap_or("—"));
        println!("    凭证     : {cred}");

        match vault.quota_cache.get(&a.id) {
            Some(q) if q.ok => {
                let win = |name: &str, w: &Option<codex_helper_lib::model::QuotaWindow>| {
                    if let Some(w) = w {
                        let left = 100.0 - w.used_percent;
                        println!(
                            "    {name:<8} : 已用 {:.0}% / 剩余 {:.0}% / {} 秒后重置",
                            w.used_percent, left, w.reset_after_seconds
                        );
                    } else {
                        println!("    {name:<8} : —");
                    }
                };
                println!("    套餐来源 : 接口");
                win("5 小时", &q.primary);
                win("每周", &q.secondary);
                if let Some(b) = &q.credits_balance {
                    println!("    积分余额 : {b}");
                }
            }
            Some(q) => println!("    额度     : 查询失败 - {}", q.error.as_deref().unwrap_or("?")),
            None => println!("    额度     : 尚未查询"),
        }
        println!();
    }
}
