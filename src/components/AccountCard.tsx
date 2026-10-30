import { useState } from "react";
import type { DragEvent } from "react";
import type { AccountView, QuotaWindow } from "../types";
import { QuotaRing } from "./QuotaRing";
import { minuteText } from "./WarmupScheduleDialog";
import {
  initial,
  isDormant5h,
  isDormant5hBlocked,
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
  resetting: boolean;
  resetLocked: boolean;
  /** 有任何激活任务在跑（含批量），此时所有激活按钮都要禁用，避免重复发请求 */
  warmupLocked: boolean;
  dragging: boolean;
  dragTarget: boolean;
  onDragStart: (id: string) => void;
  onDragOver: (id: string) => void;
  onDrop: (id: string) => void;
  onDragEnd: () => void;
  onSwitch: (id: string) => void;
  onRename: (id: string, name: string) => void;
  onEdit: (id: string) => void;
  onWarmup: (id: string) => void;
  onUseResetCredit: (id: string) => void;
  onSchedule: (id: string) => void;
  onToggleHidden: (id: string, hidden: boolean) => void;
  onDelete: (id: string) => void;
}

/** 单个额度窗口的展示块 */
function WindowBlock({
  window: w,
  dormant = false,
  warmupBlocked = false,
  fetchedAt = 0,
}: {
  window: QuotaWindow | null | undefined;
  /** 该窗口是否从未启动（此时不显示倒计时，因为根本没有在倒计时） */
  dormant?: boolean;
  /** 窗口未启动，但周额度已耗尽，当前无法用会话激活 */
  warmupBlocked?: boolean;
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
      {warmupBlocked ? (
        <p className="reset dormant-hint">等待周额度恢复</p>
      ) : dormant ? (
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
  resetting,
  resetLocked,
  warmupLocked,
  dragging,
  dragTarget,
  onDragStart,
  onDragOver,
  onDrop,
  onDragEnd,
  onSwitch,
  onRename,
  onEdit,
  onWarmup,
  onUseResetCredit,
  onSchedule,
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
  const dormantBlocked = dormant && isDormant5hBlocked(quota);
  const resetCredits = quota?.reset_credits;
  const nextResetCredit = resetCredits?.credits.find(
    (credit) => credit.expires_at === null || credit.expires_at > Date.now() / 1000,
  );
  const noResetCredits = resetCredits != null && (
    resetCredits.available_count === 0 || !nextResetCredit
  );
  const resetExpiry = nextResetCredit?.expires_at
    ? `${new Intl.DateTimeFormat("zh-CN", {
        month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false,
      }).format(new Date(nextResetCredit.expires_at * 1000))} 到期`
    : nextResetCredit ? "优先使用最先到期的卡" : "自动选择最先到期的卡";

  function commitRename() {
    const next = draft.trim();
    if (next && next !== account.name) {
      onRename(account.id, next);
    } else {
      setDraft(account.name);
    }
    setEditing(false);
  }

  function startDragging(event: DragEvent<HTMLDivElement>) {
    const target = event.target as HTMLElement;
    if (target.closest("button, input, select, textarea")) {
      event.preventDefault();
      return;
    }
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", account.id);
    onDragStart(account.id);
  }

  return (
    <div
      className={[
        "card",
        account.is_current ? "current" : "",
        exhausted && !account.is_current ? "offline" : "",
        dragging ? "dragging" : "",
        dragTarget ? "drag-target" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      draggable={!editing}
      onDragStart={startDragging}
      onDragOver={(event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        onDragOver(account.id);
      }}
      onDrop={(event) => {
        event.preventDefault();
        onDrop(account.id);
      }}
      onDragEnd={onDragEnd}
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
        {dormant && !dormantBlocked && !requesting && (
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
              warmupBlocked={dormantBlocked}
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
      {isOfficial && (
        <button
          className="reset-credit-action"
          disabled={requesting || resetLocked || (noResetCredits && !account.reset_pending)}
          title={account.reset_pending
            ? "上次兑换结果尚未确认，重试时继续使用原卡片和请求编号"
            : quota?.reset_credits_error
              ? `重置卡查询失败：${quota.reset_credits_error}\n点击重新查询并使用最先到期的可用卡`
              : "使用一张当前套餐支持且未过期的重置卡，优先使用最先到期的卡"}
          onClick={() => onUseResetCredit(account.id)}
        >
          <span>{resetting ? "重置中…" : account.reset_pending ? "重试重置" : "使用重置卡"}</span>
          <span className="reset-credit-detail">
            {account.reset_pending ? "继续确认上次兑换"
              : noResetCredits ? "暂无可用卡"
              : resetCredits ? `${resetCredits.available_count} 张 · ${resetExpiry}`
              : quota?.reset_credits_error ? "查询失败，点击重试" : resetExpiry}
          </span>
        </button>
      )}
      {isOfficial && account.warmup_schedule?.enabled && (
        <button className="schedule-summary" onClick={() => onSchedule(account.id)}>
          每日 {minuteText(account.warmup_schedule.activate_at)} 激活
          <span>
            {account.warmup_schedule.quiet_start === account.warmup_schedule.quiet_end
              ? "全天管控自动激活"
              : `${minuteText(account.warmup_schedule.quiet_start)}–${minuteText(account.warmup_schedule.quiet_end)} 管控自动激活`}
          </span>
        </button>
      )}
      <div className="card-foot">
        {account.is_current ? (
          <button disabled>正在使用中</button>
        ) : (
          <button
            className="primary"
            disabled={busy || resetting}
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
        {isOfficial && (
          <button className="ghost" title="设置该账号的每日激活时刻与自动激活管控时间段" onClick={() => onSchedule(account.id)}>
            定时
          </button>
        )}
        <button
          className="ghost"
          title="编辑这个账号的授权文件 / 配置片段"
          disabled={resetting}
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
            disabled={resetting}
            onClick={() => onDelete(account.id)}
            onMouseLeave={() => setConfirmDelete(false)}
          >
            确认删除
          </button>
        ) : (
          <button
            className="ghost"
            title="从列表移除（不会影响磁盘上的文件）"
            disabled={resetting}
            onClick={() => setConfirmDelete(true)}
          >
            删除
          </button>
        )}
      </div>
    </div>
  );
}
