import type { AccountView, Quota, QuotaWindow } from "./types";

/**
 * token 数量的可读写法。
 * 统计页里动辄上亿，全写出来根本没法扫一眼，所以统一压缩：
 *   1234        → 1,234
 *   123456      → 123.5K
 *   12345678    → 12.3M
 *   1234567890  → 1.23B
 */
export function fmtTokens(n: number): string {
  if (!Number.isFinite(n)) return "—";
  const abs = Math.abs(n);
  if (abs >= 1e9) return `${(n / 1e9).toFixed(2)}B`;
  if (abs >= 1e6) return `${(n / 1e6).toFixed(2)}M`;
  if (abs >= 1e4) return `${(n / 1e3).toFixed(1)}K`;
  return n.toLocaleString("zh-CN");
}

/** 把 0.1234 变成 "12.3%"。 */
export function fmtPercent(v: number, digits = 1): string {
  if (!Number.isFinite(v)) return "—";
  return `${(v * 100).toFixed(digits)}%`;
}

/** 窗口秒数翻译成中文标签。 */
export function windowLabel(seconds: number): string {
  if (!seconds) return "窗口";
  if (seconds <= 6 * 3600) return "5 小时";
  if (seconds <= 2 * 86400) return "每日";
  if (seconds <= 8 * 86400) return "每周";
  if (seconds <= 40 * 86400) return "每月";
  return "长期";
}

/** 秒数转「3 天 4 小时」这样的可读文案。 */
export function humanDuration(secs: number): string {
  if (secs <= 0) return "即将重置";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d} 天 ${h} 小时`;
  if (h > 0) return `${h} 小时 ${m} 分`;
  if (m > 0) return `${m} 分钟`;
  return `${Math.max(1, Math.floor(secs))} 秒`;
}

/**
 * 距离重置还有多久。
 * 优先用绝对时间 reset_at 现算，这样界面上的倒计时会自己走；
 * 没有 reset_at 时退回接口给的 reset_after_seconds。
 */
export function resetInSeconds(w: QuotaWindow): number {
  if (w.reset_at > 0) {
    return w.reset_at - Math.floor(Date.now() / 1000);
  }
  return w.reset_after_seconds;
}

/** 剩余百分比（接口给的是已用）。 */
export function remainingPercent(w: QuotaWindow | null | undefined): number | null {
  if (!w) return null;
  return Math.max(0, Math.min(100, 100 - w.used_percent));
}

/** 剩余额度对应的颜色：越少越红。 */
export function toneFor(remaining: number | null): string {
  if (remaining === null) return "var(--text-faint)";
  if (remaining <= 15) return "var(--danger)";
  if (remaining < 50) return "var(--warn)";
  return "var(--ok)";
}

/** 是否已经打满（任一窗口）。 */
export function isExhausted(q: Quota | null | undefined): boolean {
  if (!q || !q.ok) return false;
  if (q.limit_reached) return true;
  const full = (w?: QuotaWindow | null) => (w ? w.used_percent >= 100 : false);
  return full(q.primary) || full(q.secondary);
}

/**
 * 5 小时窗口是不是「从未启动」。是否当前可激活还要结合周额度判断。
 *
 * Codex 的 5 小时窗口是「用一次才开始计时」的。账号长期不用时，接口会返回
 * `used_percent = 0` 且 `reset_after_seconds` 恰好等于整个窗口长度（18000 秒），
 * 说明时钟压根没开始走，也就永远等不到重置。
 *
 * 判据两条件一起卡，跟后端 `QuotaWindow::is_dormant` 保持一致。
 * 只看 used_percent 是不行的：刚启动不久的窗口也会是 0%。
 */
export function isDormant5h(q: Quota | null | undefined): boolean {
  if (!q || !q.ok) return false;
  const w = q.primary;
  // window_seconds > 0 表示这个账号确实有主额度窗口
  if (!w || w.window_seconds <= 0) return false;
  return w.used_percent < 0.5 && w.reset_after_seconds >= w.window_seconds;
}

/**
 * 5 小时窗口虽然休眠，但当前额度状态不允许用会话把它点着。
 * 周额度恢复后，下一次刷新会重新开放手动/自动激活。
 */
export function isDormant5hBlocked(q: Quota | null | undefined): boolean {
  if (!q || !q.ok || !isDormant5h(q)) return false;
  return q.limit_reached || (q.secondary?.used_percent ?? 0) >= 100;
}

/** 取名字首字符做头像；中文取第一个字，英文取首字母。 */
export function initial(name: string, email?: string | null): string {
  const source = (name || email || "?").trim();
  return source.charAt(0).toUpperCase();
}

/**
 * 综合两个窗口给账号打分：剩余越少分越低。
 * 用来挑出「状态最佳」的账号。
 */
export function healthScore(a: AccountView): number | null {
  const q = a.quota;
  if (!q || !q.ok) return null;
  if (isExhausted(q)) return 0;
  const p = remainingPercent(q.primary);
  const s = remainingPercent(q.secondary);
  const values = [p, s].filter((v): v is number => v !== null);
  if (values.length === 0) return null;
  // 取两个窗口里更紧张的那个作为上限，再用平均值微调
  return Math.min(...values) * 0.7 + (values.reduce((x, y) => x + y, 0) / values.length) * 0.3;
}

/** 找出状态最佳的账号 id（分数最高且明显健康）。 */
export function pickBestAccount(accounts: AccountView[]): string | null {
  const scored = accounts
    .filter((a) => a.kind === "official" && !a.hidden)
    .map((a) => ({ id: a.id, score: healthScore(a) }))
    .filter((x): x is { id: string; score: number } => x.score !== null);

  if (scored.length === 0) return null;
  scored.sort((a, b) => b.score - a.score);
  const best = scored[0];
  // 分数太低就别推荐了
  if (best.score < 20) return null;
  return best.id;
}
