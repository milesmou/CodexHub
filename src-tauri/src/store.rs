//! 账号库的落盘与读取。
//!
//! 数据目录：`~/.codex-helper/`
//! - `vault.enc`  —— AES-256-GCM 加密后的账号库
//! - `backups/`   —— 每次切换前备份的 auth.json
//!
//! 写入采用「临时文件 + 原子重命名」，避免写一半断电把库写坏。

use crate::crypto;
use crate::model::Vault;
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// 用户主目录，Windows 取 USERPROFILE，其他平台兜底 HOME。
pub fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 本工具的数据目录。
pub fn data_dir() -> PathBuf {
    home_dir().join(".codex-helper")
}

/// 加密账号库路径。
pub fn vault_path() -> PathBuf {
    data_dir().join("vault.enc")
}

/// 备份目录。
pub fn backups_dir() -> PathBuf {
    data_dir().join("backups")
}

/// 确保数据目录存在。
pub fn ensure_dirs() -> Result<()> {
    fs::create_dir_all(data_dir())
        .with_context(|| format!("创建数据目录失败：{}", data_dir().display()))?;
    fs::create_dir_all(backups_dir())
        .with_context(|| format!("创建备份目录失败：{}", backups_dir().display()))?;
    Ok(())
}

/// 读取账号库。文件不存在或解密失败时返回空库，保证程序仍能启动。
pub fn load() -> Vault {
    let path = vault_path();
    if !path.exists() {
        return Vault::default();
    }
    match fs::read(&path).map_err(anyhow::Error::from).and_then(|blob| {
        let plain = crypto::decrypt(&blob)?;
        let vault: Vault = serde_json::from_slice(&plain).context("账号库 JSON 解析失败")?;
        Ok(vault)
    }) {
        Ok(v) => v,
        Err(e) => {
            // 不让启动直接失败：打日志并退回空库，用户可重新导入一次
            eprintln!("[codex-hub] 账号库读取失败：{e}");
            Vault::default()
        }
    }
}

/// 原子写入账号库。
pub fn save(vault: &Vault) -> Result<()> {
    ensure_dirs()?;
    let plain = serde_json::to_vec(vault).context("账号库序列化失败")?;
    let blob = crypto::encrypt(&plain)?;

    let target = vault_path();
    let tmp = target.with_extension("enc.tmp");
    fs::write(&tmp, &blob).with_context(|| format!("写入临时文件失败：{}", tmp.display()))?;
    fs::rename(&tmp, &target).with_context(|| format!("替换账号库失败：{}", target.display()))?;
    Ok(())
}
