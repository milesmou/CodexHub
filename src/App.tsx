import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorText } from "./api";
import type { AccountView, CodexAppStatus, Paths, Settings } from "./types";
import { AccountCard } from "./components/AccountCard";
import { AddAccountDialog } from "./components/AddAccountDialog";
import { SettingsDrawer } from "./components/SettingsDrawer";
import { StatsPage } from "./components/StatsPage";
import { SwitchConfirm } from "./components/SwitchConfirm";
import { isDormant5h, isExhausted, pickBestAccount } from "./format";

/** 添加 / 编辑账号弹窗的打开状态 */
type DialogState =
  | { mode: "create" }
  | { mode: "edit"; account: AccountView }
  | null;

/** 等待用户确认的切换请求：进程信息是异步查的，所以先占位再回填 */
interface PendingSwitch {
  account: AccountView;
  /** null = 还在查 */
  status: CodexAppStatus | null;
  /** 查进程失败时的原因；失败也允许继续切换 */
  statusError: string | null;
}

/** 相对时间文案 */
function agoText(secs: number): string {
  if (secs < 10) return "刚刚更新";
  if (secs < 60) return `${secs} 秒前更新`;
  if (secs < 3600) return `${Math.floor(secs / 60)} 分钟前更新`;
  return `${Math.floor(secs / 3600)} 小时前更新`;
}

export default function App() {
  const [accounts, setAccounts] = useState<AccountView[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [paths, setPaths] = useState<Paths | null>(null);

  const [ready, setReady] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshingIds, setRefreshingIds] = useState<Set<string>>(() => new Set());
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [warmingId, setWarmingId] = useState<string | null>(null);
  const [warmingAll, setWarmingAll] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showStats, setShowStats] = useState(false);
  const [dialog, setDialog] = useState<DialogState>(null);
  const [pendingSwitch, setPendingSwitch] = useState<PendingSwitch | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  // 每 5 秒重渲染一次，让「重置倒计时」和「多久前更新」自己走
  const [, setTick] = useState(0);
  const toastTimer = useRef<number | null>(null);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 3600);
  }, []);

  const reload = useCallback(async () => {
    setAccounts(await api.listAccounts());
  }, []);

  // 首次加载
  useEffect(() => {
    (async () => {
      try {
        const [acc, st, p] = await Promise.all([
          api.listAccounts(),
          api.getSettings(),
          api.getPaths(),
        ]);
        setAccounts(acc);
        setSettings(st);
        setPaths(p);
      } catch (e) {
        showToast(errorText(e));
      } finally {
        setReady(true);
      }
    })();
  }, [showToast]);

  // 订阅后端事件
  useEffect(() => {
    const disposers: Array<() => void> = [];
    let alive = true;

    (async () => {
      const subs: Array<Promise<() => void>> = [
        listen("accounts-changed", () => {
          void reload();
        }),
        listen<string[]>("refresh-started", (e) => {
          setRefreshingIds((current) => new Set([...current, ...e.payload]));
        }),
        listen<string[]>("refresh-finished", (e) => {
          setRefreshingIds((current) => {
            const next = new Set(current);
            e.payload.forEach((id) => next.delete(id));
            return next;
          });
          void reload();
        }),
        listen<string>("toast", (e) => showToast(e.payload)),
        listen("settings-changed", () => {
          void api.getSettings().then(setSettings);
        }),
        listen<string>("warmup-started", (e) => {
          setWarmingId(e.payload);
        }),
        // 自动、单个和批量激活统一在这里结束状态并重查对应账号。
        listen<string>("warmup-finished", (e) => {
          setWarmingId((id) => (id === e.payload ? null : id));
          void api.refreshQuotas([e.payload]).then(setAccounts).catch(() => undefined);
        }),
      ];
      const results = await Promise.all(subs);
      if (alive) {
        disposers.push(...results);
        void Promise.all([api.refreshActiveIds(), api.warmupActiveIds()])
          .then(([refreshIds, warmupIds]) => {
            setRefreshingIds(new Set(refreshIds));
            setWarmingId(warmupIds[0] ?? null);
          })
          .catch(() => undefined);
      } else results.forEach((d) => d());
    })();

    return () => {
      alive = false;
      disposers.forEach((d) => d());
    };
  }, [reload, showToast]);

  useEffect(() => {
    const t = window.setInterval(() => setTick((v) => v + 1), 5000);
    return () => window.clearInterval(t);
  }, []);

  // ------------------------------------------------------------ 操作

  const doRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      setAccounts(await api.refreshQuotas());
    } catch (e) {
      showToast(errorText(e));
    } finally {
      setRefreshing(false);
    }
  }, [showToast]);

  /**
   * 点卡片上的「切换」：先查一下 Codex 进程情况，再弹确认框。
   *
   * 切换账号必须把 Codex 关掉重开 —— 它在启动时就把 auth.json 读进内存了，
   * 光改文件不重启是无效的。这是个有破坏性的动作（会中断正在进行的对话），
   * 所以先让用户过目一遍要关掉什么。
   */
  const requestSwitch = useCallback(
    (id: string) => {
      const account = accounts.find((a) => a.id === id);
      if (!account) return;

      setPendingSwitch({ account, status: null, statusError: null });
      api
        .codexAppStatus()
        .then((status) =>
          setPendingSwitch((p) => (p && p.account.id === id ? { ...p, status } : p)),
        )
        .catch((e) =>
          setPendingSwitch((p) =>
            p && p.account.id === id ? { ...p, statusError: errorText(e) } : p,
          ),
        );
    },
    [accounts],
  );

  /** 确认框里点了「切换并重启」：真正执行 */
  const doSwitch = useCallback(async () => {
    const target = pendingSwitch;
    if (!target) return;

    setSwitchingId(target.account.id);
    try {
      const r = await api.switchAccount(target.account.id, true);
      showToast(r.message);
      await reload();
    } catch (e) {
      showToast(errorText(e));
    } finally {
      setSwitchingId(null);
      setPendingSwitch(null);
    }
  }, [pendingSwitch, reload, showToast]);

  /** 激活单个账号的 5 小时窗口 */
  const doWarmup = useCallback(
    async (id: string) => {
      // 批量进行中就别再插一条了，否则会重复发请求
      if (warmingAll || warmingId) return;
      setWarmingId(id);
      try {
        const r = await api.warmupAccount(id);
        showToast(r.message);
      } catch (e) {
        showToast(errorText(e));
      } finally {
        setWarmingId(null);
      }
    },
    [reload, showToast, warmingAll, warmingId],
  );

  /** 一键激活所有未启动的窗口 */
  const doWarmupAll = useCallback(async () => {
    if (warmingAll || warmingId) return;
    setWarmingAll(true);
    try {
      const results = await api.warmupAllDormant();
      const okIds = results.filter((r) => r.ok).map((r) => r.account_id);
      const failed = results.length - okIds.length;
      showToast(
        failed > 0
          ? `激活成功 ${okIds.length} 个，失败 ${failed} 个`
          : `已激活 ${okIds.length} 个账号的 5 小时窗口`,
      );
    } catch (e) {
      showToast(errorText(e));
    } finally {
      setWarmingAll(false);
    }
  }, [reload, showToast, warmingAll, warmingId]);

  const doRename = useCallback(
    async (id: string, name: string) => {
      try {
        await api.updateAccount(id, { name });
        await reload();
      } catch (e) {
        showToast(errorText(e));
      }
    },
    [reload, showToast],
  );

  const doToggleHidden = useCallback(
    async (id: string, hidden: boolean) => {
      try {
        await api.updateAccount(id, { hidden });
        await reload();
      } catch (e) {
        showToast(errorText(e));
      }
    },
    [reload, showToast],
  );

  const doDelete = useCallback(
    async (id: string) => {
      try {
        await api.deleteAccount(id);
        await reload();
      } catch (e) {
        showToast(errorText(e));
      }
    },
    [reload, showToast],
  );

  /** 打开某个账号的「编辑授权」弹窗 */
  const doEdit = useCallback(
    (id: string) => {
      const acc = accounts.find((a) => a.id === id);
      if (acc) setDialog({ mode: "edit", account: acc });
    },
    [accounts],
  );

  /** 添加 / 编辑保存成功：刷新列表，并顺带拉一次新账号的额度 */
  const onDialogSaved = useCallback(
    async (name: string, id: string) => {
      setDialog(null);
      showToast(`已保存「${name}」`);
      await reload();
      // 官方账号才有额度可查，第三方账号后端会直接跳过，不用前端判断
      try {
        setAccounts(await api.refreshQuotas([id]));
      } catch {
        /* 额度查询失败不影响账号已添加这件事 */
      }
    },
    [reload, showToast],
  );

  // ------------------------------------------------------------ 派生数据

  const officialCount = accounts.filter((a) => a.kind === "official").length;
  const bestId = useMemo(() => pickBestAccount(accounts), [accounts]);
  const current = accounts.find((a) => a.is_current) ?? null;

  /** 5 小时窗口从未启动、需要点一下的账号 */
  const dormantAccounts = useMemo(
    () => accounts.filter((a) => a.kind === "official" && isDormant5h(a.quota)),
    [accounts],
  );

  const updatedAt = useMemo(() => {
    const stamps = accounts
      .map((a) => a.quota?.fetched_at ?? 0)
      .filter((n) => n > 0);
    return stamps.length ? Math.max(...stamps) : 0;
  }, [accounts]);

  const banners = useMemo(() => {
    const out: { tone: "warn" | "info"; text: string }[] = [];
    if (current && isExhausted(current.quota)) {
      const best = bestId ? accounts.find((a) => a.id === bestId) : null;
      out.push({
        tone: "warn",
        text: best
          ? `「${current.name}」额度已耗尽，建议切换到「${best.name}」`
          : `「${current.name}」额度已耗尽`,
      });
    }
    return out;
  }, [accounts, bestId, current]);

  // ------------------------------------------------------------ 渲染

  return (
    <div className="app">
      <div className="topbar">
        <div className="brand">
          <h1>Codex Hub</h1>
          <span className="count">
            {accounts.length} 个账号 · {officialCount} 个可查额度
          </span>
        </div>

        <div className="topbar-actions">
          <span className="updated">
            {refreshing
              ? "刷新中…"
              : updatedAt
                ? agoText(Math.max(0, Math.floor(Date.now() / 1000) - updatedAt))
                : "尚未查询"}
          </span>
          <button disabled={refreshing} onClick={doRefresh}>
            {refreshing ? <span className="spin">↻</span> : "刷新额度"}
          </button>
          <button onClick={() => setDialog({ mode: "create" })}>添加账号</button>
          <button onClick={() => setShowStats(true)}>统计</button>
          <button className="ghost" onClick={() => setShowSettings(true)}>
            设置
          </button>
        </div>
      </div>

      <div className="scroll">
        {dormantAccounts.length > 0 &&
          refreshingIds.size === 0 &&
          !warmingAll &&
          warmingId === null && (
          <div className="banners">
            <div className="banner info">
              <span className="dot" />
              <span>
                {dormantAccounts.length} 个账号的 5 小时窗口未启动 ——
                没用过就不会重置，点一下把它点着
              </span>
              <button
                className="ghost banner-action"
                disabled={warmingAll}
                onClick={doWarmupAll}
              >
                {warmingAll ? "激活中…" : "一键激活"}
              </button>
            </div>
          </div>
        )}

        {banners.length > 0 && (
          <div className="banners">
            {banners.map((b, i) => (
              <div key={i} className={`banner ${b.tone}`}>
                <span className="dot" />
                <span>{b.text}</span>
              </div>
            ))}
          </div>
        )}

        {!ready ? (
          <div className="empty">
            <p>正在读取账号库…</p>
          </div>
        ) : accounts.length === 0 ? (
          <div className="empty">
            <h2>还没有任何账号</h2>
            <p>
              Codex 的 5 小时与每周额度是按账号计的。把多个账号存进来，
              就能在一个界面里看到谁的额度还剩多少，并一键切换。
            </p>
            <div className="empty-actions">
              <button className="primary" onClick={() => setDialog({ mode: "create" })}>
                添加账号
              </button>
            </div>
          </div>
        ) : (
          <div className="grid">
            {accounts.map((a) => (
              <AccountCard
                key={a.id}
                account={a}
                isBest={a.id === bestId}
                busy={switchingId === a.id}
                requesting={refreshingIds.has(a.id) || warmingId === a.id}
                warming={warmingId === a.id}
                warmupLocked={
                  refreshingIds.size > 0 || warmingAll || warmingId !== null
                }
                onSwitch={requestSwitch}
                onRename={doRename}
                onEdit={doEdit}
                onWarmup={doWarmup}
                onToggleHidden={doToggleHidden}
                onDelete={doDelete}
              />
            ))}
          </div>
        )}
      </div>

      {showSettings && settings && (
        <SettingsDrawer
          settings={settings}
          paths={paths}
          onClose={() => setShowSettings(false)}
          onSaved={setSettings}
          onError={showToast}
        />
      )}

      {dialog && (
        <AddAccountDialog
          mode={dialog.mode}
          account={dialog.mode === "edit" ? dialog.account : null}
          onClose={() => setDialog(null)}
          onSaved={onDialogSaved}
          onError={showToast}
        />
      )}

      {showStats && <StatsPage onClose={() => setShowStats(false)} />}

      {pendingSwitch && (
        <SwitchConfirm
          account={pendingSwitch.account}
          status={pendingSwitch.status}
          statusError={pendingSwitch.statusError}
          busy={switchingId === pendingSwitch.account.id}
          onConfirm={doSwitch}
          onCancel={() => setPendingSwitch(null)}
        />
      )}

      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}
