import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  getAllWindows,
  getCurrentWindow,
  PhysicalPosition,
  PhysicalSize,
  primaryMonitor,
} from "@tauri-apps/api/window";
import { api } from "./api";
import { remainingPercent } from "./format";
import type { AccountView } from "./types";

const WIDTH = 138;
const HEIGHT = 32;
// 给系统托盘、时钟和“显示桌面”按钮预留的位置。
const RIGHT_RESERVE = 218;

/** 把透明小窗放进主显示器任务栏的空白区域。 */
async function placeOnTaskbar() {
  const [monitor, anchor] = await Promise.all([
    primaryMonitor(),
    api.taskbarStatusAnchor().catch(() => null),
  ]);
  if (!monitor) return;

  const win = getCurrentWindow();
  const scale = monitor.scaleFactor;
  const width = Math.round(WIDTH * scale);
  const wantedHeight = Math.round(HEIGHT * scale);
  const reserve = Math.round(RIGHT_RESERVE * scale);
  const mon = monitor.position;
  const size = monitor.size;
  const work = monitor.workArea;
  const monRight = mon.x + size.width;
  const monBottom = mon.y + size.height;
  const workRight = work.position.x + work.size.width;
  const workBottom = work.position.y + work.size.height;

  const bottomBar = monBottom - workBottom;
  const topBar = work.position.y - mon.y;
  const rightBar = monRight - workRight;

  let x: number;
  let y: number;
  let height: number;

  if (anchor && anchor.width > 0 && anchor.height > 0) {
    height = Math.min(wantedHeight, Math.max(24, anchor.height - 6));
    x = anchor.x - width - Math.round(6 * scale);
    y = anchor.y + Math.floor((anchor.height - height) / 2);
    await win.setSize(new PhysicalSize(width, height));
    await win.setPosition(new PhysicalPosition(x, y));
    return;
  }

  if (bottomBar > 4 || (topBar <= 4 && rightBar <= 4)) {
    // Windows 11 的常见布局；自动隐藏任务栏时按底部 48px 兜底。
    const barHeight = bottomBar > 4 ? bottomBar : Math.round(48 * scale);
    height = Math.min(wantedHeight, Math.max(24, barHeight - 6));
    x = monRight - reserve - width;
    y = monBottom - barHeight + Math.floor((barHeight - height) / 2);
  } else if (topBar > 4) {
    height = Math.min(wantedHeight, Math.max(24, topBar - 6));
    x = monRight - reserve - width;
    y = mon.y + Math.floor((topBar - height) / 2);
  } else {
    // 竖向任务栏空间不足以容纳横排文字，贴在其底部内侧。
    height = wantedHeight;
    x = workRight + Math.max(0, Math.floor((rightBar - width) / 2));
    y = monBottom - reserve - height;
  }

  await win.setSize(new PhysicalSize(width, height));
  await win.setPosition(new PhysicalPosition(x, y));
}

/** Windows 的最小化事件在部分版本上不稳定，状态窗自己复核一次可见性。 */
async function syncVisibility() {
  const settings = await api.getSettings();
  const status = getCurrentWindow();

  if (settings.taskbar_status_enabled) {
    await status.show();
    await status.setAlwaysOnTop(true);
  } else {
    await status.hide();
  }
}

export default function TaskbarStatus() {
  const [accounts, setAccounts] = useState<AccountView[]>([]);
  const [refreshingIds, setRefreshingIds] = useState<Set<string>>(() => new Set());

  const reload = useCallback(() => {
    void api.listAccounts().then(setAccounts).catch(() => undefined);
  }, []);

  useEffect(() => {
    void placeOnTaskbar().then(syncVisibility).catch(() => undefined);
    reload();

    const timer = window.setInterval(
      () => void placeOnTaskbar().then(syncVisibility).catch(() => undefined),
      3000,
    );
    const disposers: Array<() => void> = [];
    let alive = true;
    void Promise.all([
      listen("accounts-changed", reload),
      listen("settings-changed", () => {
        void syncVisibility().catch(() => undefined);
      }),
      listen<string[]>("refresh-started", (event) => {
        setRefreshingIds((current) => new Set([...current, ...event.payload]));
      }),
      listen<string[]>("refresh-finished", (event) => {
        setRefreshingIds((current) => {
          const next = new Set(current);
          event.payload.forEach((id) => next.delete(id));
          return next;
        });
        reload();
      }),
    ]).then((subscriptions) => {
      if (alive) {
        disposers.push(...subscriptions);
        void api
          .refreshActiveIds()
          .then((ids) => setRefreshingIds(new Set(ids)))
          .catch(() => undefined);
      } else subscriptions.forEach((dispose) => dispose());
    });

    return () => {
      alive = false;
      window.clearInterval(timer);
      disposers.forEach((dispose) => dispose());
    };
  }, [reload]);

  const current = accounts.find((account) => account.is_current) ?? null;
  const status = useMemo(() => {
    if (!current) return { text: "Codex · 未选择账号", tone: "idle" };
    if (current.kind === "third_party") {
      return { text: `${current.name} · 按量`, tone: "healthy" };
    }
    if (!current.quota?.ok) {
      return { text: `${current.name} · 未查询`, tone: "idle" };
    }
    const fiveHour = remainingPercent(current.quota.primary);
    const weekly = remainingPercent(current.quota.secondary);
    const min = Math.min(fiveHour ?? 100, weekly ?? 100);
    return {
      text: `5h ${fiveHour === null ? "—" : Math.round(fiveHour)}% · 周 ${weekly === null ? "—" : Math.round(weekly)}%`,
      tone: min <= 15 ? "danger" : min < 50 ? "warning" : "healthy",
    };
  }, [current]);

  const refreshing = current ? refreshingIds.has(current.id) : false;
  const tooltipText = current
    ? `当前账号：${current.name}\n${status.text}`
    : status.text;

  async function openMainWindow() {
    const main = (await getAllWindows()).find((win) => win.label === "main");
    if (!main) return;
    await main.show();
    await main.unminimize();
    await main.setFocus();
  }

  return (
    <button
      className={`taskbar-status compact-status ${status.tone}`}
      aria-label={`${tooltipText}，单击打开主窗口，右键打开菜单`}
      onClick={() => void openMainWindow()}
      onContextMenu={(event) => {
        event.preventDefault();
        void api.popupStatusMenu(event.clientX);
      }}
    >
      <span className={`taskbar-dot ${refreshing ? "pulse" : ""}`} />
      <span className="taskbar-text">{refreshing ? "额度刷新中…" : status.text}</span>
    </button>
  );
}
