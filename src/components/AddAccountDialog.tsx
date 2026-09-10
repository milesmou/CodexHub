import { useEffect, useMemo, useRef, useState } from "react";
import type { AccountView, AuthPreview } from "../types";
import { api, errorText } from "../api";

interface Props {
  /** create = 新建账号；edit = 改已有账号的授权内容 */
  mode: "create" | "edit";
  /** edit 模式下要编辑的账号 */
  account?: AccountView | null;
  /** 打开弹窗时若带了预填的 auth 文本（例如从文件直接拖进来） */
  initialAuth?: string;
  onClose: () => void;
  /** 保存成功后回调（带上账号名与 id，便于刷新列表 / 额度） */
  onSaved: (name: string, id: string) => void;
  onError: (msg: string) => void;
}

/** config.toml 里指向第三方中转的最简模板，点「插入模板」时塞进编辑框。 */
const THIRD_PARTY_CONFIG_TEMPLATE = `[model_providers.custom]
name = "custom"
base_url = "https://your-relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
`;

const AUTH_PLACEHOLDER = `把 auth.json 的完整内容粘贴到这里，例如：

{
  "OPENAI_API_KEY": "sk-......"
}

或官方登录态（含 tokens.access_token / refresh_token）`;

export function AddAccountDialog({
  mode,
  account,
  initialAuth,
  onClose,
  onSaved,
  onError,
}: Props) {
  const [name, setName] = useState(account?.name ?? "");
  const [authText, setAuthText] = useState(initialAuth ?? "");
  const [configText, setConfigText] = useState("");
  const [preview, setPreview] = useState<AuthPreview | null>(null);
  const [loading, setLoading] = useState(mode === "edit");
  const [saving, setSaving] = useState(false);
  const [pickedPath, setPickedPath] = useState<string | null>(null);

  const previewTimer = useRef<number | null>(null);

  // ------------------------------------------------------------ 初始回填
  useEffect(() => {
    if (mode !== "edit" || !account) return;
    let alive = true;
    (async () => {
      try {
        const cred = await api.readAccountCredentials(account.id);
        if (!alive) return;
        setName(cred.name);
        setAuthText(cred.auth);
        setConfigText(cred.config ?? "");
      } catch (e) {
        onError(errorText(e));
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [mode, account, onError]);

  // ------------------------------------------------------------ 实时校验预览
  useEffect(() => {
    if (previewTimer.current) window.clearTimeout(previewTimer.current);

    const text = authText.trim();
    if (!text) {
      setPreview(null);
      return;
    }

    // 边打字边请求太浪费，停 300ms 再校验
    previewTimer.current = window.setTimeout(() => {
      api
        .previewAuth(text)
        .then(setPreview)
        .catch(() => setPreview(null));
    }, 300);

    return () => {
      if (previewTimer.current) window.clearTimeout(previewTimer.current);
    };
  }, [authText]);

  // ------------------------------------------------------------ 操作

  async function pickFile() {
    try {
      const picked = await api.pickAuthFile(
        mode === "edit" ? "选择新的授权文件" : "选择 Codex 授权文件",
      );
      if (!picked) return;
      setAuthText(picked.content);
      setPickedPath(picked.path);
    } catch (e) {
      onError(errorText(e));
    }
  }

  async function fillFromCurrent() {
    try {
      const text = await api.readCurrentAuthText();
      setAuthText(text);
      setPickedPath(null);
    } catch (e) {
      onError(errorText(e));
    }
  }

  async function fillConfigFromCurrent() {
    try {
      const text = await api.readCurrentConfigText();
      if (!text.trim()) {
        onError("当前没有 config.toml");
        return;
      }
      setConfigText(text);
    } catch (e) {
      onError(errorText(e));
    }
  }

  async function save() {
    const trimmedAuth = authText.trim();
    if (!trimmedAuth) {
      onError("请先填入授权内容");
      return;
    }
    if (preview && !preview.ok) {
      onError(preview.error || "授权内容校验没通过");
      return;
    }

    setSaving(true);
    try {
      if (mode === "edit" && account) {
        const updated = await api.updateAccountCredentials(account.id, {
          auth: trimmedAuth,
          config: configText,
          name: name.trim() || null,
        });
        onSaved(updated.name, updated.id);
      } else {
        const created = await api.createAccount({
          name: name.trim(),
          auth: trimmedAuth,
          config: configText.trim() ? configText : null,
          source: pickedPath ? `file:${pickedPath}` : "manual",
        });
        onSaved(created.name, created.id);
      }
    } catch (e) {
      onError(errorText(e));
      setSaving(false);
    }
  }

  // ------------------------------------------------------------ 派生

  const kindLabel = useMemo(() => {
    if (!preview?.kind) return null;
    return preview.kind === "official" ? "官方账号" : "第三方账号";
  }, [preview]);

  const canSave = authText.trim().length > 0 && (!preview || preview.ok) && !saving;

  return (
    <div className="modal-mask" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>{mode === "edit" ? "编辑授权" : "添加账号"}</h2>
          <button className="ghost" onClick={onClose}>
            关闭
          </button>
        </div>

        <div className="modal-body">
          {loading ? (
            <p className="hint">正在读取账号信息…</p>
          ) : (
            <>
              <div className="field">
                <label>备注名</label>
                <input
                  style={{ width: "100%" }}
                  value={name}
                  placeholder="留空则用邮箱 / 账号 ID 自动命名"
                  onChange={(e) => setName(e.target.value)}
                />
              </div>

              <div className="field">
                <div className="field-row">
                  <label>授权内容（auth.json）</label>
                  <div className="mini-actions">
                    <button className="ghost" onClick={pickFile}>
                      从文件导入
                    </button>
                    {mode === "create" && (
                      <button className="ghost" onClick={fillFromCurrent}>
                        用当前登录态
                      </button>
                    )}
                  </div>
                </div>
                <textarea
                  className="code-area"
                  spellCheck={false}
                  value={authText}
                  placeholder={AUTH_PLACEHOLDER}
                  onChange={(e) => {
                    setAuthText(e.target.value);
                    setPickedPath(null);
                  }}
                />
                {pickedPath && (
                  <p className="hint">已读取：{pickedPath}</p>
                )}

                {preview && (
                  <div className={`preview-box ${preview.ok ? "ok" : "err"}`}>
                    <div className="preview-head">
                      <span className={`badge ${preview.kind === "official" ? "" : "third"}`}>
                        {kindLabel ?? "无法识别"}
                      </span>
                      {preview.ok ? (
                        <span className="preview-state ok">校验通过</span>
                      ) : (
                        <span className="preview-state err">校验未通过</span>
                      )}
                    </div>
                    {preview.credential && (
                      <p className="preview-line">{preview.credential}</p>
                    )}
                    {preview.email && (
                      <p className="preview-line">邮箱：{preview.email}</p>
                    )}
                    {preview.account_id && (
                      <p className="preview-line">account_id：{preview.account_id}</p>
                    )}
                    {preview.error && (
                      <p className="preview-line err">{preview.error}</p>
                    )}
                  </div>
                )}
              </div>

              <div className="field">
                <div className="field-row">
                  <label>config.toml 片段（可选）</label>
                  <div className="mini-actions">
                    <button
                      className="ghost"
                      onClick={() => setConfigText(THIRD_PARTY_CONFIG_TEMPLATE)}
                    >
                      插入第三方模板
                    </button>
                    <button className="ghost" onClick={fillConfigFromCurrent}>
                      以当前配置为模板
                    </button>
                  </div>
                </div>
                <textarea
                  className="code-area"
                  spellCheck={false}
                  value={configText}
                  placeholder="切换到这个账号时会合并进 ~/.codex/config.toml（只覆盖这里写到的键）"
                  onChange={(e) => setConfigText(e.target.value)}
                />
                <p className="hint">
                  第三方中转账号需要在这里配上 <code>[model_providers.*]</code>，
                  指向中转地址。留空则切换时不动 config.toml。
                </p>
              </div>
            </>
          )}
        </div>

        <div className="modal-foot">
          <button className="primary" style={{ flex: 1 }} disabled={!canSave} onClick={save}>
            {saving ? "保存中…" : mode === "edit" ? "保存修改" : "添加账号"}
          </button>
          <button onClick={onClose}>取消</button>
        </div>
      </div>
    </div>
  );
}
