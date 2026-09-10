//! 手动创建 / 编辑账号：把用户提供的授权文件变成一条账号记录。
//!
//! 支持的两种形态（跟 Codex 自己认的格式一致）：
//!
//! - **官方账号**：完整的 `auth.json`，含 `tokens.access_token` 等
//! - **第三方账号**：`{"OPENAI_API_KEY": "sk-..."}`，配合 config.toml 里的
//!   `[model_providers.*]` 指向中转地址

use crate::codex;
use crate::model::{Account, AccountKind};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 前端提交的新账号表单。
#[derive(Debug, Deserialize)]
pub struct NewAccountPayload {
    /// 备注名
    pub name: String,
    /// auth.json 原文
    pub auth: String,
    /// config.toml 片段，可空
    #[serde(default)]
    pub config: Option<String>,
    /// 来源标记，仅作记录
    #[serde(default)]
    pub source: Option<String>,
}

/// 给前端做即时校验 / 预览用。
#[derive(Debug, Serialize)]
pub struct AuthPreview {
    pub ok: bool,
    pub error: Option<String>,
    pub kind: Option<AccountKind>,
    pub email: Option<String>,
    pub account_id: Option<String>,
    /// 凭证形态的可读描述
    pub credential: Option<String>,
}

/// 解析用户粘贴的 auth JSON。
pub fn parse_auth_text(text: &str) -> Result<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("授权内容不能为空"));
    }
    let value: Value = serde_json::from_str(trimmed)
        .context("不是合法的 JSON，请检查是否复制完整（含首尾大括号）")?;
    if !value.is_object() {
        return Err(anyhow!("授权内容应该是一个 JSON 对象"));
    }
    Ok(value)
}

/// 判断这份凭证属于官方还是第三方。
pub fn classify(auth: &Value) -> AccountKind {
    if codex::is_official_auth(auth) {
        AccountKind::Official
    } else {
        AccountKind::ThirdParty
    }
}

/// 提取「凭证长什么样」的可读描述，用于界面上给个直观反馈。
fn describe_credential(auth: &Value) -> Option<String> {
    if let Some(t) = auth.get("tokens") {
        if let Some(at) = t.get("access_token").and_then(|v| v.as_str()) {
            let fresh = t
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());
            return Some(if fresh {
                format!("OAuth 登录态（access_token {} 字符，含 refresh_token）", at.len())
            } else {
                format!("OAuth access_token {} 字符，但没有 refresh_token", at.len())
            });
        }
    }
    if let Some(key) = auth.get("OPENAI_API_KEY").and_then(|v| v.as_str()) {
        let head: String = key.chars().take(7).collect();
        return Some(format!("API Key（{head}…，共 {} 字符）", key.len()));
    }
    if auth
        .get("OPENAI_API_KEY")
        .map(|v| v.is_null())
        .unwrap_or(false)
    {
        return Some("OPENAI_API_KEY 为 null，且没有 OAuth 凭证".to_string());
    }
    None
}

/// 校验并生成预览。
pub fn preview(auth: &Value) -> AuthPreview {
    let kind = classify(auth);
    let credential = describe_credential(auth);

    // 官方账号必须有能用的 access_token
    let error = match kind {
        AccountKind::Official => {
            if codex::account_id_from_auth(auth).is_none() {
                Some("缺少 account_id，可能不是完整的登录态".to_string())
            } else {
                None
            }
        }
        AccountKind::ThirdParty => {
            let has_key = auth
                .get("OPENAI_API_KEY")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty());
            if !has_key {
                Some("第三方账号需要在 auth 里填 OPENAI_API_KEY".to_string())
            } else {
                None
            }
        }
    };

    AuthPreview {
        ok: error.is_none(),
        error,
        kind: Some(kind),
        email: codex::email_from_auth(auth),
        account_id: codex::account_id_from_auth(auth),
        credential,
    }
}

/// 校验 config 片段是不是合法 TOML。
pub fn validate_config_snippet(snippet: &str) -> Result<()> {
    if snippet.trim().is_empty() {
        return Ok(());
    }
    snippet
        .parse::<toml_edit::DocumentMut>()
        .context("配置片段不是合法 TOML")?;
    Ok(())
}

/// 规范化账号自带的 config.toml 片段。
///
/// 官方账号只使用 auth.json，永远不保存配置片段。第三方账号可以保存服务商配置，
/// 但 `projects` 和 `history` 属于所有账号共用的 Codex 工作环境，不能跟着账号切换。
pub fn normalize_config_snippet(
    kind: AccountKind,
    snippet: Option<&str>,
) -> Result<Option<String>> {
    if kind == AccountKind::Official {
        return Ok(None);
    }

    let Some(snippet) = snippet.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let mut doc = snippet
        .parse::<toml_edit::DocumentMut>()
        .context("配置片段不是合法 TOML")?;

    // 项目信任设置和会话持久化策略属于共享环境，不进入任何账号的私有配置。
    doc.as_table_mut().remove("projects");
    doc.as_table_mut().remove("history");

    if doc.as_table().is_empty() {
        Ok(None)
    } else {
        Ok(Some(doc.to_string()))
    }
}

/// 由表单构造一条账号记录。
pub fn build_account(
    payload: &NewAccountPayload,
    sort_index: i32,
    id: String,
) -> Result<Account> {
    let auth = parse_auth_text(&payload.auth)?;
    let preview = preview(&auth);

    if let Some(err) = preview.error {
        // 有 error 仍然允许保存（用户可能就是要存个半成品），
        // 但这里选择直接拦下来，避免存进去一堆切了也用不了的东西
        return Err(anyhow!(err));
    }

    let kind = classify(&auth);
    let config = normalize_config_snippet(kind, payload.config.as_deref())?;

    let name = payload.name.trim();
    let name = if name.is_empty() {
        codex::default_name_for(&auth)
    } else {
        name.to_string()
    };

    Ok(Account {
        id,
        name,
        email: codex::email_from_auth(&auth),
        plan_type: None,
        account_id: codex::account_id_from_auth(&auth),
        kind,
        auth,
        config,
        source: Some(payload.source.clone().unwrap_or_else(|| "manual".to_string())),
        cc_id: None,
        sort_index,
        hidden: false,
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_account_never_keeps_config() {
        let config = normalize_config_snippet(AccountKind::Official, Some("not valid toml ["))
            .expect("official config should be ignored");
        assert!(config.is_none());
    }

    #[test]
    fn third_party_config_excludes_shared_projects_and_history() {
        let input = r#"
model_provider = "custom"

[model_providers.custom]
name = "custom"
base_url = "https://relay.example.com/v1"

[projects."D:/work"]
trust_level = "trusted"

[history]
persistence = "save-all"
"#;

        let normalized = normalize_config_snippet(AccountKind::ThirdParty, Some(input))
            .expect("valid config")
            .expect("provider config remains");
        let doc = normalized.parse::<toml_edit::DocumentMut>().expect("valid TOML");

        assert_eq!(doc["model_provider"].as_str(), Some("custom"));
        assert!(doc.get("model_providers").is_some());
        assert!(doc.get("projects").is_none());
        assert!(doc.get("history").is_none());
    }
}
