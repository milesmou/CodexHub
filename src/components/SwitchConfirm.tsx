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

/** 把进程列表压成「ChatGPT.exe × 9」这种形式，别把 11 行全铺出来。 */
function groupProcs(status: CodexAppStatus): { name: string; count: number; app: boolean }[] {
  const map = new Map<string, { name: string; count: number; app: boolean }>();
  for (const p of status.procs) {
    const hit = map.get(p.name);
    if (hit) hit.count += 1;
    else map.set(p.name, { name: p.name, count: 1, app: p.app });
  }
  // 桌面应用本体排前面
  return [...map.values()].sort((a, b) => Number(b.app) - Number(a.app));
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
  const groups = status ? groupProcs(status) : [];

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
            <>
              <div className="confirm-warn">
                <span className="confirm-warn-title">
                  将关闭 {status!.count} 个 Codex 进程，切换完成后自动重新打开
                </span>
                <ul className="proc-list">
                  {groups.map((g) => (
                    <li key={g.name}>
                      <span className={`proc-tag ${g.app ? "app" : ""}`}>
                        {g.app ? "桌面应用" : "命令行"}
                      </span>
                      <code>{g.name}</code>
                      <span className="proc-count">× {g.count}</span>
                    </li>
                  ))}
                </ul>
                <p className="confirm-warn-foot">
                  Codex 是在启动时读取账号的，不重启就换不了账号。
                  <b>正在进行的对话会中断，未保存的内容可能丢失。</b>
                </p>
              </div>
            </>
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
