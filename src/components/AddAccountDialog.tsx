import { useEffect, useMemo, useRef, useState } from "react";
import type { AccountKind, AccountView, AuthPreview } from "../types";
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

const AUTH_PLACEHOLDER =
  "把官方 auth.json 的完整内容粘贴到这里（含 tokens.access_token / refresh_token）";

function parseModels(text: string): string[] {
  return [...new Set(text.split(/[\n,，]+/).map((item) => item.trim()).filter(Boolean))];
}

export function AddAccountDialog({
  mode,
  account,
  initialAuth,
  onClose,
  onSaved,
  onError,
}: Props) {
  const [name, setName] = useState(account?.name ?? "");
  const [kind, setKind] = useState<AccountKind | null>(
    mode === "edit" ? account?.kind ?? null : null,
  );
  const [authText, setAuthText] = useState(initialAuth ?? "");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [modelsText, setModelsText] = useState("");
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
        try {
          const parsed = JSON.parse(cred.auth) as { OPENAI_API_KEY?: unknown };
          setApiKey(typeof parsed.OPENAI_API_KEY === "string" ? parsed.OPENAI_API_KEY : "");
        } catch {
          setApiKey("");
        }
        setBaseUrl(cred.base_url ?? "");
        setModelsText(cred.models.join("\n"));
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

    const text =
      kind === "third_party"
        ? apiKey.trim()
          ? JSON.stringify({ OPENAI_API_KEY: apiKey.trim() })
          : ""
        : authText.trim();
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
  }, [apiKey, authText, kind]);

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

  async function save() {
    const trimmedAuth =
      kind === "third_party"
        ? JSON.stringify({ OPENAI_API_KEY: apiKey.trim() }, null, 2)
        : authText.trim();
    if (!trimmedAuth) {
      onError("请先填入授权内容");
      return;
    }
    if (preview && !preview.ok) {
      onError(preview.error || "授权内容校验没通过");
      return;
    }
    if (!kind) {
      onError("请先选择账号类型");
      return;
    }
    if (preview?.kind !== kind) {
      onError(kind === "official" ? "这不是官方 OAuth 登录态" : "这不是第三方 API Key 凭证");
      return;
    }

    const models = parseModels(modelsText);
    if (kind === "third_party" && !baseUrl.trim()) {
      onError("请填写第三方服务的 Base URL");
      return;
    }
    if (kind === "third_party" && models.length === 0) {
      onError("请至少填写一个模型");
      return;
    }

    setSaving(true);
    try {
      if (mode === "edit" && account) {
        const updated = await api.updateAccountCredentials(account.id, {
          auth: trimmedAuth,
          base_url: kind === "third_party" ? baseUrl.trim() : null,
          models: kind === "third_party" ? models : [],
          name: name.trim() || null,
        });
        onSaved(updated.name, updated.id);
      } else {
        const created = await api.createAccount({
          name: name.trim(),
          kind,
          auth: trimmedAuth,
          base_url: kind === "third_party" ? baseUrl.trim() : null,
          models: kind === "third_party" ? models : [],
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

  const models = parseModels(modelsText);
  const kindMatches = !!kind && preview?.kind === kind;
  const thirdPartyReady =
    kind !== "third_party" || (baseUrl.trim().length > 0 && models.length > 0);
  const credentialReady =
    kind === "third_party" ? apiKey.trim().length > 0 : authText.trim().length > 0;
  const canSave =
    credentialReady && preview?.ok === true && kindMatches && thirdPartyReady && !saving;

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
                <label>账号类型</label>
                <div className="account-kind-picker">
                  <button
                    type="button"
                    className={kind === "official" ? "kind-option selected" : "kind-option"}
                    disabled={mode === "edit"}
                    onClick={() => setKind("official")}
                  >
                    <b>官方账号</b>
                    <span>使用 OAuth auth.json</span>
                  </button>
                  <button
                    type="button"
                    className={kind === "third_party" ? "kind-option selected" : "kind-option"}
                    disabled={mode === "edit"}
                    onClick={() => setKind("third_party")}
                  >
                    <b>第三方账号</b>
                    <span>API Key + Base URL + 模型列表</span>
                  </button>
                </div>
              </div>

              {kind && (
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
                {kind === "official" ? (
                  <>
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
                    {pickedPath && <p className="hint">已读取：{pickedPath}</p>}
                  </>
                ) : (
                  <>
                    <label>API Key</label>
                    <input
                      style={{ width: "100%" }}
                      value={apiKey}
                      placeholder="sk-..."
                      spellCheck={false}
                      onChange={(e) => setApiKey(e.target.value)}
                    />
                  </>
                )}

                {preview && (
                  <div
                    className={`preview-box ${preview.ok && preview.kind === kind ? "ok" : "err"}`}
                  >
                    <div className="preview-head">
                      <span className={`badge ${preview.kind === "official" ? "" : "third"}`}>
                        {kindLabel ?? "无法识别"}
                      </span>
                      {preview.ok && preview.kind === kind ? (
                        <span className="preview-state ok">校验通过</span>
                      ) : (
                        <span className="preview-state err">
                          {preview.ok ? "与所选类型不一致" : "校验未通过"}
                        </span>
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

              {kind === "third_party" && (
                <>
                  <div className="field">
                    <label>Base URL</label>
                    <input
                      style={{ width: "100%" }}
                      value={baseUrl}
                      placeholder="https://your-relay.example.com/v1"
                      onChange={(e) => setBaseUrl(e.target.value)}
                    />
                  </div>

                  <div className="field">
                    <label>模型列表</label>
                    <textarea
                      className="code-area model-list-area"
                      spellCheck={false}
                      value={modelsText}
                      placeholder={"每行一个模型，例如：\ngpt-5.4\ngpt-5.3-codex"}
                      onChange={(e) => setModelsText(e.target.value)}
                    />
                    <p className="hint">
                      第一项作为默认模型；也支持用逗号分隔。项目和会话记录仍由所有账号共用。
                    </p>
                  </div>
                </>
              )}
                </>
              )}
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
