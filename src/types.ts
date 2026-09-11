/** 与 Rust 端 model.rs 一一对应的类型定义。 */

export type AccountKind = "official" | "third_party";

export interface QuotaWindow {
  used_percent: number;
  window_seconds: number;
  reset_at: number;
  reset_after_seconds: number;
}

export interface Quota {
  ok: boolean;
  error?: string | null;
  plan_type?: string | null;
  email?: string | null;
  primary?: QuotaWindow | null;
  secondary?: QuotaWindow | null;
  code_review?: QuotaWindow | null;
  credits_balance?: string | null;
  has_credits: boolean;
  unlimited: boolean;
  limit_reached: boolean;
  fetched_at: number;
}

export interface AccountView {
  id: string;
  name: string;
  email?: string | null;
  plan_type?: string | null;
  kind: AccountKind;
  is_current: boolean;
  quota?: Quota | null;
  sort_index: number;
  hidden: boolean;
  source?: string | null;
}

export interface Settings {
  refresh_interval_secs: number;
  startup: boolean;
  shortcut: string;
  notify_on_limit: boolean;
  notify_on_reset: boolean;
  minimize_to_tray: boolean;
  /** 是否让任务栏状态浮层始终显示 */
  taskbar_status_enabled: boolean;
  /** 刷新时发现 5 小时窗口从未启动，就自动发一条会话把它点着 */
  warmup_auto: boolean;
}

export interface SwitchOutcome {
  ok: boolean;
  message: string;
  backup_path?: string | null;
  /** 是否顺带改写了 config.toml（该账号带了配置片段） */
  config_applied: boolean;
  /** config.toml 的备份路径 */
  config_backup_path?: string | null;
  /** 切换前关掉了几个 Codex 进程 */
  codex_killed: number;
  /** Codex 桌面应用是否已重新拉起 */
  codex_restarted: boolean;
  /** 重新拉起失败的原因（切换本身已成功） */
  codex_restart_error?: string | null;
}

/** 一个会被切换流程关掉的 Codex 进程。 */
export interface CodexAppProc {
  pid: number;
  name: string;
  /** 可执行文件路径，取不到时为空 */
  exe?: string | null;
  /** 是不是 Codex 桌面应用本体（而非 codex.exe 这类命令行工具） */
  app: boolean;
}

/** 当前 Codex 的进程情况，用于切换前的确认框。 */
export interface CodexAppStatus {
  /** 一共会关掉几个进程 */
  count: number;
  /** 桌面应用（Electron）是否开着 */
  app_running: boolean;
  procs: CodexAppProc[];
}

export interface Paths {
  data_dir: string;
  codex_home: string;
  auth_path: string;
}

// ---------------------------------------------------------------- 手动添加 / 编辑账号

/** 一份授权内容的即时校验结果（对应 Rust 的 AuthPreview）。 */
export interface AuthPreview {
  ok: boolean;
  error?: string | null;
  kind?: AccountKind | null;
  email?: string | null;
  account_id?: string | null;
  /** 凭证形态的可读描述，例如「OAuth 登录态（access_token 1234 字符，含 refresh_token）」 */
  credential?: string | null;
}

/** 用户通过系统文件框挑中的授权文件。 */
export interface PickedFile {
  path: string;
  content: string;
}

/** 新建账号表单。 */
export interface NewAccountPayload {
  name: string;
  kind: AccountKind;
  /** auth.json 原文 */
  auth: string;
  /** 第三方服务地址；官方账号忽略 */
  base_url?: string | null;
  /** 第三方模型列表；官方账号忽略 */
  models?: string[];
  /** 来源标记，仅作记录 */
  source?: string | null;
}

/** 编辑账号表单：只传要改的字段。 */
export interface EditAccountPayload {
  /** 传了就替换 auth（原文 JSON） */
  auth?: string | null;
  /** 传了就替换第三方服务地址 */
  base_url?: string | null;
  /** 传了就替换第三方模型列表 */
  models?: string[];
  name?: string | null;
}

/** 编辑弹窗的回填数据。 */
export interface AccountCredentials {
  name: string;
  auth: string;
  base_url?: string | null;
  models: string[];
}

/** 一次「激活 5 小时窗口」的结果。 */
export interface WarmupOutcome {
  ok: boolean;
  message: string;
  account_id: string;
  name: string;
  /** CLI 输出尾部，出错时用来排查 */
  log_tail?: string | null;
}

// ---------------------------------------------------------------- token 统计

/** 某一天的 token 用量（对应 Rust 的 DayStat）。 */
export interface TokenDay {
  /** 本地日期，YYYY-MM-DD */
  date: string;
  total: number;
  input: number;
  cached: number;
  output: number;
  reasoning: number;
  /** 当天的 API 调用次数 */
  calls: number;
  /** 当天涉及多少条会话 */
  threads: number;
}

/** 某个模型的总用量（对应 Rust 的 ModelStat）。 */
export interface TokenModel {
  model: string;
  total: number;
  input: number;
  cached: number;
  output: number;
  calls: number;
}

/** token 统计结果（对应 Rust 的 TokenStats）。 */
export interface TokenStats {
  /** 按日期升序，只含真正有消耗的天 */
  days: TokenDay[];
  /** 按总量降序 */
  models: TokenModel[];
  total: number;
  input: number;
  cached: number;
  output: number;
  reasoning: number;
  calls: number;
  threads: number;
  /** 扫过的会话文件数 */
  files: number;
  first_day?: string | null;
  last_day?: string | null;
  /** 口径说明 */
  note: string;
}
