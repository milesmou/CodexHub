import { useState } from "react";
import { api, errorText } from "../api";
import type { AccountView, WarmupSchedule } from "../types";

export function minuteText(minute: number): string {
  return `${Math.floor(minute / 60).toString().padStart(2, "0")}:${(minute % 60).toString().padStart(2, "0")}`;
}

const DEFAULT_SCHEDULE: WarmupSchedule = {
  enabled: false, quiet_start: 0, quiet_end: 540, activate_at: 480,
};

function toMinute(value: string): number {
  if (!/^\d{2}:\d{2}$/.test(value)) return NaN;
  const [hour, minute] = value.split(":").map(Number);
  return hour * 60 + minute;
}

export function WarmupScheduleDialog({ account, onClose, onSaved }: {
  account: AccountView;
  onClose: () => void;
  onSaved: () => void;
}) {
  const schedule = account.warmup_schedule ?? DEFAULT_SCHEDULE;
  const [enabled, setEnabled] = useState(schedule.enabled);
  const [start, setStart] = useState(minuteText(schedule.quiet_start));
  const [end, setEnd] = useState(minuteText(schedule.quiet_end));
  const [at, setAt] = useState(minuteText(schedule.activate_at));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save(event: React.FormEvent) {
    event.preventDefault();
    const quiet_start = toMinute(start), quiet_end = toMinute(end), activate_at = toMinute(at);
    if (![quiet_start, quiet_end, activate_at].every(m => Number.isInteger(m) && m >= 0 && m < 1440)) {
      setError("请填写完整的开始、结束和激活时间");
      return;
    }
    const within = quiet_start < quiet_end
      ? activate_at >= quiet_start && activate_at < quiet_end
      : activate_at >= quiet_start || activate_at < quiet_end;
    if (enabled && !within) {
      setError("激活时刻必须位于管控时间段内（不含结束时刻）");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.setWarmupSchedule(account.id, { enabled, quiet_start, quiet_end, activate_at });
      onSaved();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  }

  return (
    <div className="modal-mask" onClick={busy ? undefined : onClose}>
      <form className="modal warmup-schedule-modal" onClick={e => e.stopPropagation()} onSubmit={save}>
        <div className="modal-head">
          <h2>定时激活额度 · {account.name}</h2>
          <button type="button" className="ghost" disabled={busy} onClick={onClose}>关闭</button>
        </div>
        <div className="modal-body">
          <label className="checkbox">
            <input type="checkbox" checked={enabled} disabled={busy} onChange={e => setEnabled(e.target.checked)} />
            开启每日定时激活
          </label>
          <fieldset className="schedule-fields" disabled={busy || !enabled}>
            <div className="field">
              <label>普通自动激活的管控时间段</label>
              <div className="schedule-time-range">
                <label>开始<input aria-label="管控开始时间" type="time" step="60" required value={start} onChange={e => setStart(e.target.value)} /></label>
                <span>至</span>
                <label>结束<input aria-label="管控结束时间" type="time" step="60" required value={end} onChange={e => setEnd(e.target.value)} /></label>
              </div>
              <p className="hint">每天重复，支持跨午夜；开始与结束相同表示全天。</p>
            </div>
            <div className="field">
              <label htmlFor="schedule-activate-time">指定激活时刻</label>
              <input id="schedule-activate-time" type="time" step="60" required value={at} onChange={e => setAt(e.target.value)} />
            </div>
          </fieldset>
          <p className="schedule-explanation">
            管控时间段内跳过普通自动激活，在指定时刻启动定时任务，失败后在本次区间内重试；时间段外沿用全局自动激活设置。手动激活仍可使用。
          </p>
          <p className="hint">按电脑本地时间执行，需保持 Codex Hub 后台运行。仅激活尚未启动且额度可用的窗口；多个账号按顺序处理。错过首次触发时刻不补跑；已触发任务失败会重试，重启后仍可恢复，离开本次管控区间即停止。</p>
          {error && <p className="schedule-error" role="alert">{error}</p>}
        </div>
        <div className="modal-foot">
          <button type="submit" className="primary" disabled={busy}>{busy ? "保存中…" : "保存定时设置"}</button>
          <button type="button" disabled={busy} onClick={onClose}>取消</button>
        </div>
      </form>
    </div>
  );
}
