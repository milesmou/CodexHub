//! Codex 自身的配置读写：定位 `~/.codex/auth.json` 并安全替换。

use crate::store;
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const MANAGED_PROVIDER_ID: &str = "codexhelper";

/// Codex 的配置目录，优先读 CODEX_HOME 环境变量。
pub fn codex_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("CODEX_HOME") {
        return PathBuf::from(dir);
    }
    store::home_dir().join(".codex")
}

/// 当前登录态文件。
pub fn auth_path() -> PathBuf {
    codex_home().join("auth.json")
}

/// 读取当前 auth.json。
pub fn read_auth() -> Result<Value> {
    let path = auth_path();
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("读取 auth.json 失败：{}", path.display()))?;
    let value: Value = serde_json::from_str(&raw).context("auth.json 不是合法 JSON")?;
    Ok(value)
}

/// 把当前 auth.json 备份到备份目录，返回备份文件路径。
pub fn backup_auth() -> Result<PathBuf> {
    store::ensure_dirs()?;
    let src = auth_path();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dst = store::backups_dir().join(format!("auth-{stamp}.json"));
    if src.exists() {
        fs::copy(&src, &dst)
            .with_context(|| format!("备份 auth.json 失败：{}", dst.display()))?;
    }
    Ok(dst)
}

/// 用指定账号的 auth 覆盖当前 auth.json。
///
/// 关键点：先备份、再写临时文件、最后原子重命名，
/// 这样即使 Codex 正在运行也不会读到写了一半的文件。
pub fn write_auth(auth: &Value) -> Result<PathBuf> {
    let backup = backup_auth()?;
    let path = auth_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建 Codex 目录失败：{}", parent.display()))?;
    }

    let text = serde_json::to_string_pretty(auth).context("auth 序列化失败")?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).with_context(|| format!("写入临时 auth 失败：{}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("替换 auth.json 失败：{}", path.display()))?;

    Ok(backup)
}

// ---------------------------------------------------------------- config.toml

/// Codex 的主配置文件。
pub fn config_path() -> PathBuf {
    codex_home().join("config.toml")
}

/// 读取 config.toml，文件不存在时返回空串。
pub fn read_config() -> String {
    fs::read_to_string(config_path()).unwrap_or_default()
}

/// 备份 config.toml，返回备份路径。
pub fn backup_config() -> Result<PathBuf> {
    store::ensure_dirs()?;
    let src = config_path();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dst = store::backups_dir().join(format!("config-{stamp}.toml"));
    if src.exists() {
        fs::copy(&src, &dst)
            .with_context(|| format!("备份 config.toml 失败：{}", dst.display()))?;
    }
    Ok(dst)
}

/// 原子写入 config.toml。
pub fn write_config_text(text: &str) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建 Codex 目录失败：{}", parent.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).with_context(|| format!("写入临时 config 失败：{}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("替换 config.toml 失败：{}", path.display()))?;
    Ok(())
}

/// 把一段 TOML 片段合并进现有 config.toml。
///
/// 只覆盖片段里出现的键，其余内容（包括注释、排版、projects/plugins 等）原样保留。
/// 这样做是为了避免「切换账号」把用户辛苦攒的配置冲掉。
pub fn merge_config(current: &str, snippet: &str) -> Result<String> {
    let snippet = snippet.trim();
    if snippet.is_empty() {
        return Ok(current.to_string());
    }

    let mut doc: toml_edit::DocumentMut = current
        .parse()
        .context("现有 config.toml 不是合法 TOML，已中止合并")?;
    let patch: toml_edit::DocumentMut = snippet
        .parse()
        .context("配置片段不是合法 TOML，已中止合并")?;

    merge_table(doc.as_table_mut(), patch.as_table());
    Ok(doc.to_string())
}

/// 递归合并两张表；遇到子表就往下钻，遇到普通值就直接覆盖。
fn merge_table(dst: &mut toml_edit::Table, src: &toml_edit::Table) {
    for (key, value) in src.iter() {
        match value {
            toml_edit::Item::Table(src_table) => match dst.get_mut(key) {
                Some(toml_edit::Item::Table(dst_table)) => {
                    merge_table(dst_table, src_table);
                }
                // 目标位置不是表（或不存在），整块替换
                _ => {
                    dst.insert(key, value.clone());
                }
            },
            other => {
                dst.insert(key, other.clone());
            }
        }
    }
}

/// CodexHelper 为当前第三方账号生成的模型目录。
pub fn model_catalog_path() -> PathBuf {
    codex_home().join("codex-helper-model-catalog.json")
}

/// 根据模型名称生成 Codex 可加载的模型目录。
pub fn write_model_catalog(models: &[String]) -> Result<PathBuf> {
    let path = model_catalog_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建 Codex 目录失败：{}", parent.display()))?;
    }

    let entries: Vec<Value> = models
        .iter()
        .enumerate()
        .map(|(index, model)| {
            serde_json::json!({
                "additional_speed_tiers": [],
                "availability_nux": null,
                "base_instructions": "You are Codex, a coding agent. You and the user share the same workspace and collaborate to achieve the user's goals.",
                "context_window": 128000,
                "default_reasoning_level": "medium",
                "default_reasoning_summary": "none",
                "description": model,
                "display_name": model,
                "effective_context_window_percent": 95,
                "experimental_supported_tools": [],
                "input_modalities": ["text", "image"],
                "max_context_window": 128000,
                "priority": 1000 + index,
                "service_tiers": [],
                "shell_type": "shell_command",
                "slug": model,
                "support_verbosity": false,
                "supported_in_api": true,
                "supported_reasoning_levels": [{
                    "description": "Balances speed and reasoning depth for everyday tasks",
                    "effort": "medium"
                }],
                "supports_image_detail_original": false,
                "supports_parallel_tool_calls": false,
                "supports_reasoning_summaries": true,
                "supports_search_tool": false,
                "truncation_policy": { "limit": 10000, "mode": "bytes" },
                "upgrade": null,
                "visibility": "list"
            })
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::json!({ "models": entries }))
        .context("序列化模型目录失败")?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).context("写入临时模型目录失败")?;
    fs::rename(&tmp, &path).context("替换模型目录失败")?;
    Ok(path)
}

/// 生成 CodexHelper 托管的第三方服务配置。
pub fn managed_third_party_config(base_url: &str, models: &[String]) -> String {
    let mut doc = toml_edit::DocumentMut::new();
    doc["model"] = toml_edit::value(&models[0]);
    doc["model_provider"] = toml_edit::value(MANAGED_PROVIDER_ID);
    doc["model_catalog_json"] =
        toml_edit::value(model_catalog_path().to_string_lossy().to_string());
    doc["model_providers"][MANAGED_PROVIDER_ID]["name"] = toml_edit::value("CodexHelper");
    doc["model_providers"][MANAGED_PROVIDER_ID]["base_url"] = toml_edit::value(base_url);
    doc["model_providers"][MANAGED_PROVIDER_ID]["wire_api"] = toml_edit::value("responses");
    doc["model_providers"][MANAGED_PROVIDER_ID]["requires_openai_auth"] =
        toml_edit::value(true);
    doc.to_string()
}

/// 移除上一个第三方账号由 CodexHelper 托管的配置，保留项目、会话和用户其他设置。
pub fn clear_managed_third_party_config(current: &str) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = current
        .parse()
        .context("现有 config.toml 不是合法 TOML，已中止切换")?;
    let owned_provider = doc
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        == Some(MANAGED_PROVIDER_ID);
    if owned_provider {
        doc.as_table_mut().remove("model_provider");
        doc.as_table_mut().remove("model");
    }

    let owned_catalog = doc
        .get("model_catalog_json")
        .and_then(toml_edit::Item::as_str)
        .is_some_and(|value| PathBuf::from(value) == model_catalog_path());
    if owned_catalog {
        doc.as_table_mut().remove("model_catalog_json");
    }

    if let Some(toml_edit::Item::Table(providers)) = doc.get_mut("model_providers") {
        providers.remove(MANAGED_PROVIDER_ID);
        if providers.is_empty() {
            doc.as_table_mut().remove("model_providers");
        }
    }
    Ok(doc.to_string())
}

/// 校验一个 auth 对象是否可用作「官方账号」（含有 OAuth token）。
pub fn is_official_auth(auth: &Value) -> bool {
    auth.get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// 解析 JWT 的 payload 段（不校验签名），用于本地读取邮箱等信息。
pub fn jwt_payload(token: &str) -> Option<Value> {
    use base64::Engine as _;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(parts[1]))
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

/// 从 auth 中尽力提取邮箱，避免为了显示邮箱而多打一次接口。
pub fn email_from_auth(auth: &Value) -> Option<String> {
    // 优先看 id_token 的 email 声明
    if let Some(idt) = auth
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(|v| v.as_str())
    {
        if let Some(payload) = jwt_payload(idt) {
            for key in ["email", "preferred_username", "https://api.openai.com/profile"] {
                if let Some(v) = payload.get(key) {
                    if let Some(s) = v.as_str() {
                        return Some(s.to_string());
                    }
                    if let Some(s) = v.get("email").and_then(|x| x.as_str()) {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    // 兜底看 access_token
    if let Some(at) = auth
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
    {
        if let Some(payload) = jwt_payload(at) {
            if let Some(s) = payload.get("email").and_then(|v| v.as_str()) {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// 提取 account_id。
pub fn account_id_from_auth(auth: &Value) -> Option<String> {
    auth.get("tokens")
        .and_then(|t| t.get("account_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            // 有些版本把 account_id 藏在 access_token 的 claim 里
            let at = auth.get("tokens")?.get("access_token")?.as_str()?;
            let payload = jwt_payload(at)?;
            payload
                .get("https://api.openai.com/auth")
                .and_then(|x| x.get("chatgpt_account_id"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
}

/// 生成一个「看起来像」的账号备注名，用于导入时的默认命名。
pub fn default_name_for(auth: &Value) -> String {
    email_from_auth(auth).unwrap_or_else(|| "未命名账号".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_provider_config_can_be_removed_without_touching_shared_settings() {
        let managed = managed_third_party_config(
            "https://relay.example.com/v1",
            &["custom-model".to_string()],
        );
        let current = format!(
            "{managed}\n[projects.\"D:/work\"]\ntrust_level = \"trusted\"\n\n[history]\npersistence = \"save-all\"\n"
        );
        let cleared = clear_managed_third_party_config(&current).unwrap();
        let doc = cleared.parse::<toml_edit::DocumentMut>().unwrap();

        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model_catalog_json").is_none());
        assert!(doc.get("projects").is_some());
        assert!(doc.get("history").is_some());
    }
}
