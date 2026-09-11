//! 开发工具：把 token 统计结果打到终端，用来核对口径对不对。
//!
//! 用法：
//! ```bash
//! cd src-tauri
//! cargo run --release --example dump_stats
//! cargo run --release --example dump_stats -- 7    # 只看最近 7 天
//! ```

use codex_hub_lib::stats;

fn main() {
    let days: Option<u32> = std::env::args().nth(1).and_then(|s| s.parse().ok());
    let s = stats::collect(days);

    println!("扫描文件   : {} 份", s.files);
    println!(
        "覆盖区间   : {} ~ {}",
        s.first_day.as_deref().unwrap_or("—"),
        s.last_day.as_deref().unwrap_or("—")
    );
    println!(
        "总计       : {:.2}M tokens / {} 次调用 / {} 条会话",
        s.total as f64 / 1e6,
        s.calls,
        s.threads
    );
    println!(
        "  input={:.2}M (其中 cached={:.2}M)  output={:.2}M  reasoning={:.2}M",
        s.input as f64 / 1e6,
        s.cached as f64 / 1e6,
        s.output as f64 / 1e6,
        s.reasoning as f64 / 1e6
    );
    println!();
    println!("{:<12} {:>12} {:>10} {:>8} {:>8}", "日期", "total", "calls", "会话", "占比");
    for d in &s.days {
        let pct = if s.total > 0 {
            d.total as f64 / s.total as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "{:<12} {:>12} {:>10} {:>8} {:>7.1}%",
            d.date,
            d.total,
            d.calls,
            d.threads,
            pct
        );
    }
    println!();
    println!("按模型：");
    for m in &s.models {
        println!(
            "  {:<24} {:>12}  ({:.1}%)  calls={}",
            m.model,
            m.total,
            if s.total > 0 {
                m.total as f64 / s.total as f64 * 100.0
            } else {
                0.0
            },
            m.calls
        );
    }
    println!();
    println!("口径：{}", s.note);
}
