/** Tauri 命令调用封装。 */

import { invoke } from "@tauri-apps/api/core";
import type {
  AccountCredentials,
  AccountView,
  AuthPreview,
  CodexAppStatus,
  EditAccountPayload,
  NewAccountPayload,
  Paths,
  PickedFile,
  Settings,
  SwitchOutcome,
  TokenStats,
  WarmupOutcome,
} from "./types";

export const api = {
  listAccounts: () => invoke<AccountView[]>("list_accounts"),

  /** ids 为空表示刷新全部官方账号 */
  refreshQuotas: (ids?: string[]) =>
    invoke<AccountView[]>("refresh_quotas", { ids: ids ?? null }),

  /** 页面启动时同步已经由后台发起的额度查询 */
  refreshActiveIds: () => invoke<string[]>("refresh_active_ids"),

  switchAccount: (id: string, restartCodex = true) =>
    invoke<SwitchOutcome>("switch_account", { id, restartCodex }),

  /** 查一下切换时会被关掉哪些 Codex 进程（只读，不动进程） */
  codexAppStatus: () => invoke<CodexAppStatus>("codex_app_status"),


  deleteAccount: (id: string) => invoke<void>("delete_account", { id }),

  updateAccount: (
    id: string,
    patch: { name?: string; hidden?: boolean; sort_index?: number },
  ) => invoke<void>("update_account", { id, ...patch }),

  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
  popupStatusMenu: (cursorX: number) =>
    invoke<void>("popup_status_menu", { cursorX }),
  taskbarStatusAnchor: () =>
    invoke<{ x: number; y: number; width: number; height: number } | null>(
      "taskbar_status_anchor",
    ),

  getPaths: () => invoke<Paths>("get_paths"),
  openDataDir: () => invoke<void>("open_data_dir"),

  // -------------------------------------------------- 手动添加 / 编辑账号

  /** 校验一段 auth JSON 并返回预览；不抛错，错误在返回体里 */
  previewAuth: (text: string) => invoke<AuthPreview>("preview_auth", { text }),

  /** 弹系统文件框选一份授权文件；用户取消返回 null */
  pickAuthFile: (title?: string) =>
    invoke<PickedFile | null>("pick_auth_file", { title: title ?? null }),

  /** 读取当前 ~/.codex/auth.json 原文 */
  readCurrentAuthText: () => invoke<string>("read_current_auth_text"),

  /** 读取当前 ~/.codex/config.toml，并剔除共享的 projects/history 配置 */
  readCurrentConfigText: () => invoke<string>("read_current_config_text"),

  /** 读取指定账号存着的授权内容，用于编辑回填 */
  readAccountCredentials: (id: string) =>
    invoke<AccountCredentials>("read_account_credentials", { id }),

  /** 从第三方服务的 OpenAI 兼容接口获取模型列表 */
  fetchProviderModels: (baseUrl: string, apiKey: string) =>
    invoke<string[]>("fetch_provider_models", { baseUrl, apiKey }),

  createAccount: (payload: NewAccountPayload) =>
    invoke<AccountView>("create_account", { payload }),

  updateAccountCredentials: (id: string, payload: EditAccountPayload) =>
    invoke<AccountView>("update_account_credentials", { id, payload }),

  // -------------------------------------------------- 激活 5 小时窗口

  /** 用该账号发一条极简会话，把休眠的 5 小时窗口点着 */
  warmupAccount: (id: string) => invoke<WarmupOutcome>("warmup_account", { id }),

  /** 一次性点着所有「窗口未启动」的账号（后端串行执行） */
  warmupAllDormant: () => invoke<WarmupOutcome[]>("warmup_all_dormant"),

  /** 页面启动时同步已经由后台发起的激活请求 */
  warmupActiveIds: () => invoke<string[]>("warmup_active_ids"),

  /** 查 codex CLI 路径，找不到返回 null */
  codexCliPath: () => invoke<string | null>("codex_cli_path"),

  // -------------------------------------------------- token 统计

  /** 按天统计 token 消耗；days 为空表示统计全部历史 */
  tokenStats: (days?: number) =>
    invoke<TokenStats>("token_stats", { days: days ?? null }),
};

/** 把 invoke 抛出来的错误统一转成字符串。 */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
