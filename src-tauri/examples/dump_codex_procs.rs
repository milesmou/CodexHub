//! 开发工具：看看切换账号时会被关掉哪些 Codex 进程。
//!
//! **只读**，不会关任何进程 —— 用来核对识别规则有没有误伤。
//!
//! 用法：
//! ```bash
//! cd src-tauri
//! cargo run --release --example dump_codex_procs
//! ```

use codex_helper_lib::codexapp;

fn main() {
    let status = codexapp::status();

    println!("待关闭进程数 : {}", status.count);
    println!("桌面应用在运行: {}", if status.app_running { "是" } else { "否" });
    println!();
    println!("{:<8} {:<6} {:<32} {}", "PID", "类型", "进程名", "可执行文件");
    for p in &status.procs {
        println!(
            "{:<8} {:<6} {:<32} {}",
            p.pid,
            if p.app { "桌面" } else { "命令行" },
            p.name,
            p.exe.as_deref().unwrap_or("（取不到）")
        );
    }

    if status.count == 0 {
        println!("（当前没有 Codex 进程在跑）");
    }
}
