import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 期望前端跑在固定端口，且不要清屏以便看到 Rust 端的日志
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      // src-tauri 由 cargo 自己监听，前端 watcher 忽略掉避免重复触发
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "chrome110",
    minify: "esbuild",
    sourcemap: false,
  },
});
