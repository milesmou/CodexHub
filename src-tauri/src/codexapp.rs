//! Codex 桌面应用的进程控制。
//!
//! ## 为什么需要它
//!
//! Codex 是在**启动时**把 `~/.codex/auth.json` 读进内存的，之后一直用内存里那份。
//! 所以只改磁盘上的 auth.json 对已经跑着的 Codex 完全没用 —— 必须让它重启一次。
//! 更麻烦的是它还会在刷新 token 时**回写** auth.json，如果先改文件再关进程，
//! 进程退出时可能把旧账号又写回去。因此顺序必须是：
//!
//! **先杀干净 → 再写 auth.json → 最后重新拉起来**
//!
//! ## 要关哪些进程
//!
//! 本机装的是商店版 Codex（MSIX 包 `OpenAI.Codex`），它是个 Electron 应用：
//!
//! ```text
//! ChatGPT.exe                     ← 应用本体（Electron 主进程 + 若干渲染/GPU 子进程）
//! └ codex.exe ... app-server      ← 后端，真正拿 auth 的那个
//!   └ codex-code-mode-host.exe
//! ```
//!
//! 判定规则两条，命中任一即为目标：
//!
//! 1. **可执行文件在 Codex 的 MSIX 包目录下**（`...\WindowsApps\OpenAI.Codex_*`）
//!    —— 覆盖 ChatGPT.exe / Codex.exe 以及所有 Electron 子进程。
//!    必须按**路径**判，不能只按镜像名：`ChatGPT.exe` 这名字太通用，
//!    万一机器上还装了独立的 ChatGPT 客户端就会误杀。
//! 2. **镜像名是 Codex 的命令行工具**（`codex.exe` 等）
//!    —— 你自己在终端里敲的那个 `codex` 也在这条规则里，一并关掉，
//!    免得还有别的会话握着旧账号。
//!
//! 本工具自己叫 `codex-helper.exe`，不在名单里，不会自杀。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Codex 的命令行工具镜像名（这些进程的 exe 在
/// `%LOCALAPPDATA%\OpenAI\Codex\bin\<hash>\` 下，按名字匹配最省事）。
const HELPER_IMAGES: &[&str] = &[
    "codex.exe",
    "codex-code-mode-host.exe",
    "codex-command-runner.exe",
    "codex-windows-sandbox-setup.exe",
];

/// Codex 桌面应用 MSIX 包目录名的前缀。
const PKG_PREFIX: &str = "OpenAI.Codex_";

/// MSIX 包的安装根目录。
const WINDOWS_APPS: &str = r"C:\Program Files\WindowsApps";

/// 等待进程退出的上限。
const KILL_TIMEOUT: Duration = Duration::from_secs(12);

/// 一个待关闭的进程。
#[derive(Debug, Clone, Serialize)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    /// 可执行文件路径；拿不到时为空
    pub exe: Option<String>,
    /// 是不是 Codex 桌面应用本体（而不是 codex.exe 这类命令行工具）
    pub app: bool,
}

/// 当前 Codex 的进程情况，给前端弹确认框用。
#[derive(Debug, Serialize)]
pub struct AppStatus {
    /// 一共找到了几个待关闭的进程
    pub count: usize,
    /// 桌面应用（Electron）是否开着
    pub app_running: bool,
    pub procs: Vec<ProcInfo>,
}

impl AppStatus {
    fn from(procs: Vec<ProcInfo>) -> Self {
        let app_running = procs.iter().any(|p| p.app);
        Self {
            count: procs.len(),
            app_running,
            procs,
        }
    }
}

/// 路径是不是落在 Codex 的 MSIX 包目录里。
fn is_codex_app_path(path: &Path) -> bool {
    let s = path.to_string_lossy().to_lowercase();
    s.contains(r"\windowsapps\")
        && s.contains(&PKG_PREFIX.to_lowercase())
}

/// 扫一遍当前进程，挑出所有该关的。
///
/// 只读操作，不会动任何进程 —— 前端拿它来渲染确认框。
pub fn find_running() -> Vec<ProcInfo> {
    let me = std::process::id();
    let sys = sysinfo::System::new_all();
    let mut out = Vec::new();

    for (pid, proc_) in sys.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == me {
            continue; // 绝不杀自己
        }

        let name = proc_.name().to_string_lossy().to_string();
        let exe = proc_.exe().map(|p| p.to_path_buf());

        let is_app = exe.as_deref().map(is_codex_app_path).unwrap_or(false);
        let is_helper = HELPER_IMAGES
            .iter()
            .any(|h| name.eq_ignore_ascii_case(h));

        if !is_app && !is_helper {
            continue;
        }

        out.push(ProcInfo {
            pid: pid_u32,
            name,
            exe: exe.map(|p| p.display().to_string()),
            app: is_app,
        });
    }

    // 桌面应用本体排在前面，方便界面上先显示重点
    out.sort_by(|a, b| b.app.cmp(&a.app).then(a.pid.cmp(&b.pid)));
    out
}

/// 当前状态。
pub fn status() -> AppStatus {
    AppStatus::from(find_running())
}

/// 关掉给定的这批进程，并**等到它们真的退出**。
///
/// 等到退出这一步不能省：进程退出时可能还有机会回写 auth.json，
/// 没等干净就写新凭证，会被它盖回去。
pub fn kill_all(procs: &[ProcInfo]) -> Result<usize, String> {
    if procs.is_empty() {
        return Ok(0);
    }

    // 一次性把 PID 全传进去，少起几个进程
    let mut cmd = Command::new("taskkill");
    cmd.arg("/F");
    for p in procs {
        cmd.arg("/PID").arg(p.pid.to_string());
    }
    // taskkill 对「进程已经没了」会返回非零，这属于正常情况，
    // 真正的判据是下面轮询的结果，所以这里不检查退出码
    let _ = cmd.output();

    let deadline = Instant::now() + KILL_TIMEOUT;
    loop {
        let remain = find_running();
        if remain.is_empty() {
            return Ok(procs.len());
        }
        if Instant::now() >= deadline {
            let names = remain
                .iter()
                .map(|p| format!("{} (PID {})", p.name, p.pid))
                .collect::<Vec<_>>()
                .join("、");
            return Err(format!(
                "等待 Codex 退出超时，还有 {} 个进程没关掉：{names}",
                remain.len()
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// MSIX 包目录名 → 应用用户模型 ID（AUMID）。
///
/// 目录名格式是 `{Name}_{Version}_{Arch}__{PublisherHash}`，例如
/// `OpenAI.Codex_26.903.8094.0_x64__2p2nqsd0c76g0`；
/// 而 AUMID 只要 `{Name}_{PublisherHash}!{AppId}`，跟版本无关。
/// 包名里不允许出现下划线，所以按 `_` 切分是安全的。
fn aumid_from_dir(dir_name: &str) -> Option<String> {
    let parts: Vec<&str> = dir_name.split('_').collect();
    if parts.len() < 5 {
        return None;
    }
    let name = parts[0];
    let hash = parts[parts.len() - 1];
    if name.is_empty() || hash.is_empty() {
        return None;
    }
    // AppId 固定是 App（见包的 AppxManifest）
    Some(format!("{name}_{hash}!App"))
}

/// 从可执行文件路径往上找，定位它所属的包目录。
fn package_dir_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|a| {
            a.file_name()
                .map(|n| n.to_string_lossy().starts_with(PKG_PREFIX))
                .unwrap_or(false)
        })
        .map(|a| a.to_path_buf())
}

/// 兜底：直接在 WindowsApps 下找版本号最大的那个 Codex 包目录。
///
/// 用在「进程没在跑、拿不到 exe 路径」的场景。非提权进程也能列这个目录，
/// 所以不需要额外权限。
fn newest_package_dir() -> Option<PathBuf> {
    let mut best: Option<(String, PathBuf)> = None;
    for entry in fs::read_dir(WINDOWS_APPS).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(PKG_PREFIX) {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // 目录名里版本号在第二位，字典序比较对「同为数字」的版本段是够用的；
        // 真正在跑的时候会优先用进程路径，这里只是兜底
        if best.as_ref().map(|(n, _)| name > *n).unwrap_or(true) {
            best = Some((name, path));
        }
    }
    best.map(|(_, p)| p)
}

/// 把 Codex 桌面应用重新拉起来。
///
/// `hint` 是被关掉的那个进程的 exe 路径（有的话就按它定位版本，
/// 保证重启的是原来那个版本，而不是 WindowsApps 里别的残留版本）。
///
/// 返回实际使用的 AUMID，仅用于日志。
pub fn restart(hint: Option<&Path>) -> Result<String, String> {
    let pkg_dir = hint
        .and_then(package_dir_of)
        .or_else(newest_package_dir)
        .ok_or_else(|| "找不到 Codex 桌面应用的安装目录".to_string())?;

    let dir_name = pkg_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let aumid = aumid_from_dir(&dir_name)
        .ok_or_else(|| format!("无法从目录名解析出应用 ID：{dir_name}"))?;

    // 走 shell:AppsFolder 启动，这样应用能拿到正常的包身份。
    // explorer.exe 启动成功也会返回非零退出码，所以只看能不能起来，不看退出码。
    Command::new("explorer.exe")
        .arg(format!(r"shell:AppsFolder\{aumid}"))
        .spawn()
        .map_err(|e| format!("拉起 Codex 失败：{e}"))?;

    Ok(aumid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aumid_parsing() {
        assert_eq!(
            aumid_from_dir("OpenAI.Codex_26.903.8094.0_x64__2p2nqsd0c76g0").as_deref(),
            Some("OpenAI.Codex_2p2nqsd0c76g0!App")
        );
        assert_eq!(aumid_from_dir("OpenAI.Codex"), None);
        assert_eq!(aumid_from_dir(""), None);
    }

    #[test]
    fn app_path_detection() {
        let ok = PathBuf::from(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.903.8094.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );
        assert!(is_codex_app_path(&ok));

        // 同名的独立 ChatGPT 客户端不能被误判
        let other = PathBuf::from(r"C:\Users\me\AppData\Local\Programs\ChatGPT\ChatGPT.exe");
        assert!(!is_codex_app_path(&other));

        // 我们的管家自己不能被算进去
        let helper = PathBuf::from(r"D:\Workspace\CodexHelper\src-tauri\target\debug\codex-helper.exe");
        assert!(!is_codex_app_path(&helper));
    }

    #[test]
    fn package_dir_lookup() {
        let exe = PathBuf::from(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.903.8094.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );
        let dir = package_dir_of(&exe).unwrap();
        assert_eq!(
            dir.file_name().unwrap().to_string_lossy(),
            "OpenAI.Codex_26.903.8094.0_x64__2p2nqsd0c76g0"
        );
        assert!(package_dir_of(Path::new(r"C:\Windows\explorer.exe")).is_none());
    }
}
