// 发布版不弹黑色控制台窗口（调试版保留，方便看日志）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    codex_hub_lib::run()
}
