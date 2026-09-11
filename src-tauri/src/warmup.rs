//! 激活账号的 5 小时额度窗口。
//!
//! ## 背景
//!
//! Codex 的 5 小时窗口是「用一次才开始计时」的。账号长期不用时，窗口处于
//! 休眠状态 —— 接口会返回 `used_percent = 0` 且 `reset_after_seconds` 恰好
//! 等于整个窗口长度（18000 秒）。这时候时钟没开始走，也就永远等不到重置。
//!
//! 本模块的做法：**以该账号的身份发一条极简会话**，把窗口「点着」，
//! 之后 5 小时一到就会正常重置一次。
//!
//! ## 关键设计：不碰用户的登录态
//!
//! 直觉做法是「切到该账号 → 发请求 → 切回来」，但那会在中途改写
//! `~/.codex/auth.json`，万一被打断就会留下错乱的登录态。
//!
//! 这里改成给这次调用单独准备一个**临时 CODEX_HOME**：把该账号的 auth.json
//! 写进去，用环境变量指过去。`codex exec` 的 `--ignore-user-config` 只跳过
//! config.toml，认证依然是读 `CODEX_HOME` 下的 auth.json —— 正好够用。
//!
//! 跑完无论成败都立刻删目录，里面存着明文凭证。

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// 单次激活的超时。
///
/// 实测：正常成功是一次往返，几秒就完；**失败时** CLI 会先重连 websocket 5 次、
/// 再退到 HTTPS 重试 5 次，光重试就要 35 秒左右。给到 90 秒是为了让它把
/// 自己的错误信息吐出来 —— 卡太紧只会拿到一句「超时」，反而没法排查。
const TIMEOUT: Duration = Duration::from_secs(90);

/// 发出去的提示词。目的只是让服务端记一笔「这个账号活跃了」，
/// 所以越短越省，也别让模型真去干活。
const PROMPT: &str = "Reply with the single word: ok";

/// 临时 CODEX_HOME 的 config.toml：只读沙箱 + 不询问，
/// 保证 CLI 在无人值守下也能跑完，且不会去改任何文件。
const SANDBOX_CONFIG: &str = r#"sandbox_mode = "read-only"
approval_policy = "never"
"#;

/// 一次激活的结果。
#[derive(Debug, Serialize)]
pub struct WarmupOutcome {
    pub ok: bool,
    pub message: String,
    pub account_id: String,
    pub name: String,
    /// CLI 输出的尾部，出问题时用来排查
    pub log_tail: Option<String>,
}

// ---------------------------------------------------------------- 定位 CLI

/// 找到 codex CLI 可执行文件。
///
/// Windows 上它装在带版本哈希的目录里：
/// `%LOCALAPPDATA%\OpenAI\Codex\bin\<hash>\codex.exe`，
/// 升级后目录名会变，所以**按修改时间挑最新的**，而不是把路径记死。
/// 也支持用环境变量 `CODEX_CLI_PATH` 直接指定。
pub fn find_cli() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("CODEX_CLI_PATH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1) 官方安装目录
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let bin = PathBuf::from(local)
            .join("OpenAI")
            .join("Codex")
            .join("bin");
        candidates.extend(scan_bin_dir(&bin));
    }

    // 2) npm 全局安装（老版本或 npm 装的）
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let bin = PathBuf::from(appdata)
            .join("npm")
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin");
        candidates.extend(scan_bin_dir(&bin));
    }

    // 3) PATH 兜底
    if let Some(p) = which_in_path("codex.exe").or_else(|| which_in_path("codex")) {
        candidates.push(p);
    }

    candidates
        .into_iter()
        .filter(|p| p.is_file())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .ok_or_else(|| {
            anyhow!("没找到 codex CLI。请先安装 Codex，或用环境变量 CODEX_CLI_PATH 指定 codex.exe 的路径。")
        })
}

/// 扫描 `<dir>\*\codex.exe` 这种「版本哈希子目录」结构，带上 dir 本身。
fn scan_bin_dir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();

    let direct = dir.join("codex.exe");
    if direct.is_file() {
        out.push(direct);
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("codex.exe");
            if candidate.is_file() {
                out.push(candidate);
            }
        }
    }

    out
}

/// 在 PATH 里找一个可执行文件（省得为一个函数引依赖）。
fn which_in_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
}

// ---------------------------------------------------------------- 执行

/// 用指定账号发一条极简会话，把 5 小时窗口点着。
///
/// 成功时返回 CLI 输出尾部（给界面看个凭据），失败时返回错误。
pub async fn warmup(auth: &serde_json::Value) -> Result<String> {
    let cli = find_cli()?;

    // 每个账号一个独立临时目录，避免并发时互相覆盖
    let home = crate::store::data_dir()
        .join("warmup")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&home)
        .with_context(|| format!("创建临时目录失败：{}", home.display()))?;

    let result = run_isolated(&cli, &home, auth).await;

    // 里面是明文凭证，无论成败都得清掉
    if let Err(e) = std::fs::remove_dir_all(&home) {
        eprintln!("[codex-hub] 清理临时目录失败 {}：{e}", home.display());
    }

    result
}

/// 在隔离的 CODEX_HOME 里跑一次 `codex exec`。
async fn run_isolated(cli: &Path, home: &Path, auth: &serde_json::Value) -> Result<String> {
    // CLI 就是从这里读登录态的
    let auth_text = serde_json::to_string_pretty(auth).context("序列化 auth 失败")?;
    std::fs::write(home.join("auth.json"), auth_text).context("写入临时 auth.json 失败")?;
    std::fs::write(home.join("config.toml"), SANDBOX_CONFIG)
        .context("写入临时 config.toml 失败")?;

    let mut cmd = tokio::process::Command::new(cli);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd.arg("exec")
        .arg("--skip-git-repo-check") // 临时目录不是 git 仓库
        .arg("--ephemeral") // 不落会话文件
        .arg("--ignore-user-config") // 不吃用户的 config.toml（auth 仍读 CODEX_HOME）
        .arg("-s")
        .arg("read-only")
        .arg("-C")
        .arg(home) // 工作目录就指到空目录，模型没东西可动
        .arg(PROMPT)
        .env("CODEX_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true); // 超时丢弃 future 时连带杀掉子进程

    let child = cmd.spawn().context("启动 codex CLI 失败")?;

    let output = match tokio::time::timeout(TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(anyhow!("等待 codex CLI 结束失败：{e}")),
        Err(_) => {
            return Err(anyhow!(
                "codex CLI 超过 {} 秒没有返回，已放弃（子进程已终止）",
                TIMEOUT.as_secs()
            ))
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let tail = tail_of(&format!("{stdout}\n{stderr}"), 400);

    if !output.status.success() {
        return Err(anyhow!(
            "codex CLI 退出码 {}：{tail}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "未知".to_string())
        ));
    }

    Ok(tail)
}

/// 取字符串末尾若干字符，用来把 CLI 输出塞进错误信息。
fn tail_of(s: &str, n: usize) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return "（CLI 没有输出）".to_string();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    let start = chars.len().saturating_sub(n);
    chars[start..].iter().collect()
}

// ---------------------------------------------------------------- 挑选目标

/// 找出所有「5 小时窗口尚未启动」的官方账号 id。
///
/// 依据是额度缓存里 primary 窗口的 `is_dormant()`，所以**必须先刷新过一次额度**，
/// 缓存里没数据或查询失败的账号不会被选中（宁可不点，也不要瞎点）。
pub fn dormant_ids(vault: &crate::model::Vault) -> Vec<String> {
    use crate::model::AccountKind;

    let mut list: Vec<&crate::model::Account> = vault
        .accounts
        .iter()
        .filter(|a| a.kind == AccountKind::Official)
        .filter(|a| {
            vault
                .quota_cache
                .get(&a.id)
                .is_some_and(crate::model::Quota::primary_dormant)
        })
        .collect();

    list.sort_by_key(|a| a.sort_index);
    list.into_iter().map(|a| a.id.clone()).collect()
}
