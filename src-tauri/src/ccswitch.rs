//! 从 cc-switch 导入已有的 Codex 账号。
//!
//! cc-switch 把每个 Provider 的完整配置塞进 `~/.cc-switch/cc-switch.db`
//! 的 `providers.settings_config` 字段（一个 JSON），结构大致是：
//!
//! ```json
//! {
//!   "auth": "{ ...auth.json 的内容... }",
//!   "config": "model = \"gpt-5.6-sol\"\n",
//!   "modelCatalog": { "models": [ ... ] }
//! }
//! ```
//!
//! 其中 `auth` 可能是「JSON 字符串」也可能已经是「对象」，这里两种都兼容。

use crate::codex;
use crate::model::{Account, AccountKind};
use crate::store;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// cc-switch 的数据库路径。
pub fn db_path() -> PathBuf {
    store::home_dir().join(".cc-switch").join("cc-switch.db")
}

/// 从 cc-switch 读出来的一条 Provider。
pub struct CcProvider {
    pub id: String,
    pub name: String,
    pub auth: Value,
    pub config: Option<String>,
    pub official_by_category: bool,
    pub sort_index: i32,
    pub is_current: bool,
}

/// 读取 cc-switch 里所有 `app_type = 'codex'` 的 Provider。
pub fn read_codex_providers() -> Result<Vec<CcProvider>> {
    let path = db_path();
    if !path.exists() {
        return Err(anyhow!("未找到 cc-switch 数据库：{}", path.display()));
    }

    // cc-switch 可能在运行且开了 WAL，只读打开偶发失败，所以退一步用读写模式（只执行 SELECT）
    let conn = rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .or_else(|_| rusqlite::Connection::open(&path))
    .with_context(|| format!("打开 cc-switch 数据库失败：{}", path.display()))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, name, settings_config, category, sort_index, is_current
             FROM providers WHERE app_type = 'codex'",
        )
        .context("查询 providers 表失败（cc-switch 表结构可能已变化）")?;

    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let name: Option<String> = row.get(1)?;
            let settings: Option<String> = row.get(2)?;
            let category: Option<String> = row.get(3)?;
            let sort_index: Option<i64> = row.get(4)?;
            let is_current: Option<i64> = row.get(5)?;
            Ok((id, name, settings, category, sort_index, is_current))
        })
        .context("遍历 providers 失败")?;

    let mut out = Vec::new();
    for row in rows {
        let (id, name, settings, category, sort_index, is_current) = row?;
        let Some(settings) = settings else { continue };

        let parsed: Value = match serde_json::from_str(&settings) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[codex-helper] 跳过 {id}：settings_config 解析失败 {e}");
                continue;
            }
        };

        let Some(auth) = extract_auth(&parsed) else {
            continue;
        };

        let config = parsed
            .get("config")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);

        out.push(CcProvider {
            id: id.clone(),
            name: name.unwrap_or_else(|| id.clone()),
            auth,
            config,
            official_by_category: category.as_deref() == Some("official"),
            sort_index: sort_index.unwrap_or(0) as i32,
            is_current: is_current.unwrap_or(0) != 0,
        });
    }

    Ok(out)
}

/// settings_config 里的 auth 可能是字符串也可能是对象。
fn extract_auth(settings: &Value) -> Option<Value> {
    match settings.get("auth")? {
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            serde_json::from_str(s).ok()
        }
        other if other.is_object() => Some(other.clone()),
        _ => None,
    }
}

/// 把 cc-switch 的 Provider 转换成我们的账号记录。
pub fn to_account(p: &CcProvider) -> Account {
    let official = codex::is_official_auth(&p.auth) || p.official_by_category;
    let kind = if official {
        AccountKind::Official
    } else {
        AccountKind::ThirdParty
    };

    let email = codex::email_from_auth(&p.auth);
    let account_id = codex::account_id_from_auth(&p.auth);

    Account {
        id: uuid::Uuid::new_v4().to_string(),
        name: p.name.clone(),
        email: email.clone(),
        plan_type: None,
        account_id,
        kind,
        auth: p.auth.clone(),
        config: p.config.clone(),
        source: Some("cc-switch".to_string()),
        cc_id: Some(p.id.clone()),
        sort_index: p.sort_index,
        hidden: false,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// 把 cc-switch 的「当前 Codex Provider」改成指定 id。
///
/// 同时改两处，cc-switch 读哪边都不会错：
/// 1. 数据库 `providers.is_current`
/// 2. `~/.cc-switch/settings.json` 的 `currentProviderCodex`
///
/// 这是尽力而为的同步：cc-switch 若正在运行可能覆盖回去，
/// 所以调用方不要把它当作切换成功的必要条件。
pub fn set_current_provider(cc_id: &str) -> Result<()> {
    let path = db_path();
    if !path.exists() {
        return Err(anyhow!("cc-switch 数据库不存在"));
    }

    let conn = rusqlite::Connection::open(&path)
        .with_context(|| format!("打开 cc-switch 数据库失败：{}", path.display()))?;
    let tx = conn.unchecked_transaction().context("开启事务失败")?;
    tx.execute(
        "UPDATE providers SET is_current = 0 WHERE app_type = 'codex'",
        [],
    )
    .context("重置 is_current 失败")?;
    tx.execute(
        "UPDATE providers SET is_current = 1 WHERE app_type = 'codex' AND id = ?1",
        [cc_id],
    )
    .context("设置 is_current 失败")?;
    tx.commit().context("提交事务失败")?;

    // 顺手同步 settings.json
    let settings_path = store::home_dir().join(".cc-switch").join("settings.json");
    if settings_path.exists() {
        if let Ok(text) = fs::read_to_string(&settings_path) {
            if let Ok(mut value) = serde_json::from_str::<Value>(&text) {
                value["currentProviderCodex"] = Value::String(cc_id.to_string());
                if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                    let _ = fs::write(&settings_path, pretty);
                }
            }
        }
    }

    Ok(())
}
