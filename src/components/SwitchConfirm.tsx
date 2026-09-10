import type { AccountView, CodexAppStatus } from "../types";

interface Props {
  account: AccountView;
  /** null = 还在查进程 */
  status: CodexAppStatus | null;
  /** 已确认、正在切换 */
  busy: boolean;
  statusError?: string | null;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * 切换账号前的确认框。
 *
 * 切换会把 Codex 整个关掉重开（它在启动时把 auth.json 读进内存，
 * 不重启就换不了账号），所以必须先说清楚要关什么、会有什么后果。
 */
export function SwitchConfirm({
  account,
  status,
  busy,
  statusError,
  onConfirm,
  onCancel,
}: Props) {
  const hasProcs = !!status && status.count > 0;

  return (
    <div className="modal-mask" onClick={busy ? undefined : onCancel}>
      <div className="modal switch-confirm" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>切换账号</h2>
          <button className="ghost" disabled={busy} onClick={onCancel}>
            关闭
          </button>
        </div>

        <div className="modal-body">
          <p className="confirm-lead">
            确定切换到「<b>{account.name}</b>」？
          </p>

          {!status && !statusError && <p className="hint">正在检查 Codex 进程…</p>}

          {statusError && (
            <div className="preview-box err">
              <div className="preview-line err">检查 Codex 进程失败：{statusError}</div>
              <div className="preview-line">
                仍然可以继续，但可能关不掉正在运行的 Codex。
              </div>
            </div>
          )}

          {hasProcs && (
            <div className="confirm-warn">
              <span className="confirm-warn-title">
                切换账号会关闭 Codex，完成后自动重新打开。
              </span>
              <p className="confirm-warn-foot">
                <b>正在进行的对话会中断，未保存的内容可能丢失。</b>
              </p>
            </div>
          )}

          {status && !hasProcs && (
            <div className="preview-box ok">
              <div className="preview-line">
                Codex 当前没在运行，只会把账号切过去，不会启动它。
              </div>
            </div>
          )}
        </div>

        <div className="modal-foot">
          <button
            className="primary"
            style={{ flex: 1 }}
            disabled={busy || (!status && !statusError)}
            onClick={onConfirm}
          >
            {busy ? "切换中…" : hasProcs ? "切换并重启 Codex" : "仅切换账号"}
          </button>
          <button disabled={busy} onClick={onCancel}>
            取消
          </button>
        </div>
      </div>
    </div>
  );
}
