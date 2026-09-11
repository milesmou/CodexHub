import { useState } from "react";
import type { AccountView, QuotaWindow } from "../types";
import { QuotaRing } from "./QuotaRing";
import {
  initial,
  isDormant5h,
  isExhausted,
  remainingPercent,
  toneFor,
  windowLabel,
} from "../format";

interface Props {
  account: AccountView;
  /** 是否是综合评分最高的账号 */
  isBest: boolean;
  busy: boolean;
  /** 正在查询额度或发送 5 小时窗口激活请求 */
  requesting: boolean;
  /** 该账号正在激活 5 小时窗口 */
  warming: boolean;
  /** 有任何激活任务在跑（含批量），此时所有激活按钮都要禁用，避免重复发请求 */
  warmupLocked: boolean;
  onSwitch: (id: string) => void;
  onRename: (id: string, name: string) => void;
  onEdit: (id: string) => void;
  onWarmup: (id: string) => void;
  onToggleHidden: (id: string, hidden: boolean) => void;
  onDelete: (id: string) => void;
}

/** 单个额度窗口的展示块 */
function WindowBlock({
  window: w,
  dormant = false,
  fetchedAt = 0,
}: {
  window: QuotaWindow | null | undefined;
  /** 该窗口是否从未启动（此时不显示倒计时，因为根本没有在倒计时） */
  dormant?: boolean;
  /** 查询时间，用于接口没有返回绝对重置时间时换算 */
  fetchedAt?: number;
}) {
  if (!w) {
    return (
      <div className="ring-box">
        <div className="ring-empty">—</div>
        <p className="label">无窗口</p>
      </div>
    );
  }

  const remaining = remainingPercent(w) ?? 0;
  const resetAt = w.reset_at > 0
    ? w.reset_at
    : fetchedAt > 0 && w.reset_after_seconds > 0
      ? fetchedAt + w.reset_after_seconds
      : 0;
  const resetText =
    resetAt > 0
      ? `${new Intl.DateTimeFormat("zh-CN", {
          year: "numeric",
          month: "2-digit",
          day: "2-digit",
          hour: "2-digit",
          minute: "2-digit",
          hour12: false,
        }).format(new Date(resetAt * 1000))} 重置`
      : "重置时间未知";

  return (
    <div className="ring-box">
      <QuotaRing percent={remaining} color={toneFor(remaining)} />
      <p className="label">{windowLabel(w.window_seconds)}</p>
      <p className="used">已用 {Math.round(w.used_percent)}%</p>
      {dormant ? (
        // 窗口没启动就没什么可倒计时的，直接说状态
        <p className="reset dormant-hint">
          {windowLabel(w.window_seconds)}窗口未启动
        </p>
      ) : (
        <p className="reset">{resetText}</p>
      )}
    </div>
  );
}

export function AccountCard({
  account,
  isBest,
  busy,
  requesting,
  warming,
  warmupLocked,
  onSwitch,
  onRename,
  onEdit,
  onWarmup,
  onToggleHidden,
  onDelete,
}: Props) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(account.name);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const quota = account.quota;
  const exhausted = isExhausted(quota);
  const isOfficial = account.kind === "official";
  // 5 小时窗口从未启动 —— 不点一下就永远不会重置
  const dormant = isOfficial && isDormant5h(quota);

  function commitRename() {
    const next = draft.trim();
    if (next && next !== account.name) {
      onRename(account.id, next);
    } else {
      setDraft(account.name);
    }
    setEditing(false);
  }

  return (
    <div
      className={[
        "card",
        account.is_current ? "current" : "",
        exhausted && !account.is_current ? "offline" : "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <div className="card-head">
        <div className={`avatar ${isOfficial ? "" : "third"}`}>
          {initial(account.name, account.email)}
        </div>

        <div className="card-title">
          {editing ? (
            <input
              autoFocus
              value={draft}
              style={{ width: "100%", padding: "2px 6px" }}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commitRename}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitRename();
                if (e.key === "Escape") {
                  setDraft(account.name);
                  setEditing(false);
                }
              }}
            />
          ) : (
            <p className="name" title={account.name}>
              {account.name}
            </p>
          )}
          <p className="sub" title={account.email ?? ""}>
            {account.email || (isOfficial ? "官方账号" : "第三方 · API Key")}
            {account.plan_type ? ` · ${account.plan_type}` : ""}
          </p>
        </div>

        {account.is_current && <span className="badge current">当前</span>}
        {!account.is_current && exhausted && (
          <span className="badge limited">已耗尽</span>
        )}
        {!account.is_current && isBest && !exhausted && (
          <span className="badge best">状态最佳</span>
        )}
        {!isOfficial && <span className="badge third">第三方</span>}

        {requesting && (
          <span
            className="card-request-spinner"
            title="请求处理中"
            aria-label="请求处理中"
          />
        )}

        {/* 请求运行时由统一的卡片转圈状态替代激活入口 */}
        {dormant && !requesting && (
          <button
            className="warmup-btn"
            disabled={warming || warmupLocked}
            title={
              "该账号的 5 小时窗口从未启动 —— 没用过就不会计时，也永远等不到重置。\n" +
              "点一下会用该账号发一条极简会话把窗口点着，之后 5 小时正常重置。\n" +
              "全程用隔离的临时环境，不会改动你当前的登录态。"
            }
            onClick={() => onWarmup(account.id)}
          >
            {warming ? "激活中" : "激活"}
          </button>
        )}
      </div>

      {!isOfficial ? (
        <div className="third-party-box">按量计费，没有订阅额度窗口</div>
      ) : !quota ? (
        <div className="rings">
          <WindowBlock window={undefined} />
          <WindowBlock window={undefined} />
        </div>
      ) : !quota.ok ? (
        <div className="error-box" title={quota.error ?? ""}>
          {quota.error || "查询失败"}
        </div>
      ) : (
        <>
          <div className="rings">
            <WindowBlock
              window={quota.primary}
              dormant={dormant}
              fetchedAt={quota.fetched_at}
            />
            <WindowBlock window={quota.secondary} fetchedAt={quota.fetched_at} />
          </div>
          {(quota.has_credits || quota.unlimited) && (
            <p className="credits">
              {quota.unlimited
                ? "积分额度：无限"
                : `附加积分余额：${quota.credits_balance ?? "0"}`}
            </p>
          )}
        </>
      )}

      {/* 第一行：主操作（切换）单独占一行，避免被下面一排小按钮挤扁 */}
      <div className="card-foot">
        {account.is_current ? (
          <button disabled>正在使用中</button>
        ) : (
          <button
            className="primary"
            disabled={busy}
            onClick={() => onSwitch(account.id)}
          >
            {busy ? "切换中…" : "切换到该账号"}
          </button>
        )}
      </div>

      {/* 第二行：次要操作 */}
      <div className="card-actions">
        <button
          className="ghost"
          title="就地修改备注名"
          onClick={() => {
            setConfirmDelete(false);
            setEditing((v) => !v);
          }}
        >
          {editing ? "完成" : "重命名"}
        </button>
        <button
          className="ghost"
          title="编辑这个账号的授权文件 / 配置片段"
          onClick={() => {
            setConfirmDelete(false);
            setEditing(false);
            onEdit(account.id);
          }}
        >
          编辑
        </button>
        <button
          className="ghost"
          title={account.hidden ? "在托盘里显示" : "在托盘里隐藏"}
          onClick={() => onToggleHidden(account.id, !account.hidden)}
        >
          {account.hidden ? "已隐藏" : "隐藏"}
        </button>
        {confirmDelete ? (
          <button
            className="ghost"
            style={{ color: "var(--danger)" }}
            onClick={() => onDelete(account.id)}
            onMouseLeave={() => setConfirmDelete(false)}
          >
            确认删除
          </button>
        ) : (
          <button
            className="ghost"
            title="从列表移除（不会影响磁盘上的文件）"
            onClick={() => setConfirmDelete(true)}
          >
            删除
          </button>
        )}
      </div>
    </div>
  );
}
