import { useEffect, useState } from "react";
import type { Paths, Settings } from "../types";
import { api, errorText } from "../api";

interface Props {
  settings: Settings;
  paths: Paths | null;
  onClose: () => void;
  onSaved: (s: Settings) => void;
  onError: (msg: string) => void;
}

const INTERVALS: { label: string; value: number }[] = [
  { label: "关闭自动刷新", value: 0 },
  { label: "每 1 分钟", value: 60 },
  { label: "每 5 分钟", value: 300 },
  { label: "每 15 分钟", value: 900 },
  { label: "每 30 分钟", value: 1800 },
  { label: "每 1 小时", value: 3600 },
];

export function SettingsDrawer({
  settings,
  paths,
  onClose,
  onSaved,
  onError,
}: Props) {
  const [draft, setDraft] = useState<Settings>(settings);
  const [saving, setSaving] = useState(false);
  // undefined = 还在查，null = 没找到
  const [cliPath, setCliPath] = useState<string | null | undefined>(undefined);

  // 外部设置变化时同步一次，避免打开面板时是旧值
  useEffect(() => setDraft(settings), [settings]);

  // 查一下 codex CLI 在哪，自动激活要靠它
  useEffect(() => {
    let alive = true;
    api
      .codexCliPath()
      .then((p) => alive && setCliPath(p))
      .catch(() => alive && setCliPath(null));
    return () => {
      alive = false;
    };
  }, []);

  function patch<K extends keyof Settings>(key: K, value: Settings[K]) {
    setDraft((prev) => ({ ...prev, [key]: value }));
  }

  async function save() {
    setSaving(true);
    try {
      await api.setSettings(draft);
      onSaved(draft);
      onClose();
    } catch (e) {
      onError(errorText(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="drawer-mask" onClick={onClose}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head">
          <h2>设置</h2>
          <button className="ghost" onClick={onClose}>
            关闭
          </button>
        </div>

        <div className="drawer-body">
          <div className="field">
            <label>后台刷新间隔</label>
            <select
              style={{ width: "100%" }}
              value={draft.refresh_interval_secs}
              onChange={(e) => patch("refresh_interval_secs", Number(e.target.value))}
            >
              {INTERVALS.map((i) => (
                <option key={i.value} value={i.value}>
                  {i.label}
                </option>
              ))}
            </select>
            <p className="hint">
              官方接口有频率限制，间隔太短可能被拒。5 分钟是比较稳妥的值。
            </p>
          </div>

          <div className="field">
            <label>全局快捷键</label>
            <input
              style={{ width: "100%" }}
              value={draft.shortcut}
              placeholder="例如 Ctrl+Alt+C，留空则禁用"
              onChange={(e) => patch("shortcut", e.target.value)}
            />
            <p className="hint">
              按下后呼出/收起主窗口。格式如 Ctrl+Alt+C、Alt+Shift+K。
            </p>
          </div>

          <div className="field">
            <label>行为</label>
            <label>后台常驻方式</label>
            <select
              style={{ width: "100%", marginBottom: 8 }}
              value={draft.status_mode}
              onChange={(e) =>
                patch("status_mode", e.target.value as Settings["status_mode"])
              }
            >
              <option value="tray">系统托盘图标</option>
              <option value="taskbar">任务栏状态浮层</option>
            </select>
            <p className="hint">
              状态浮层会在主窗口最小化或关闭后出现，右键菜单与托盘图标一致。
            </p>
            <label className="checkbox">
              <input
                type="checkbox"
                checked={draft.minimize_to_tray}
                onChange={(e) => patch("minimize_to_tray", e.target.checked)}
              />
              关闭主窗口时驻留后台，不退出
            </label>
            <label className="checkbox">
              <input
                type="checkbox"
                checked={draft.startup}
                onChange={(e) => patch("startup", e.target.checked)}
              />
              开机自动启动
            </label>
            <label className="checkbox">
              <input
                type="checkbox"
                checked={draft.warmup_auto}
                onChange={(e) => patch("warmup_auto", e.target.checked)}
              />
              自动激活未启动的 5 小时窗口
            </label>
            <p className="hint">
              Codex 的 5 小时窗口是「用一次才开始计时」的，账号一直不用就永远等不到重置。
              打开这项后，刷新额度时发现某个官方账号的窗口从未启动，就会用 codex CLI
              以该账号身份发一条极简会话把它点着。
              同一账号 30 分钟内只会点一次；不会改动你当前的登录态。
            </p>
          </div>

          <div className="field">
            <label>通知</label>
            <label className="checkbox">
              <input
                type="checkbox"
                checked={draft.notify_on_limit}
                onChange={(e) => patch("notify_on_limit", e.target.checked)}
              />
              某个账号额度耗尽时提醒
            </label>
            <label className="checkbox">
              <input
                type="checkbox"
                checked={draft.notify_on_reset}
                onChange={(e) => patch("notify_on_reset", e.target.checked)}
              />
              某个账号额度恢复时提醒
            </label>
          </div>

          <div className="field">
            <label>文件位置</label>
            <div className="paths">
              <div>
                <b>账号库</b>
                <br />
                {paths?.data_dir ?? "—"}
              </div>
              <div style={{ marginTop: 6 }}>
                <b>Codex 登录态</b>
                <br />
                {paths?.auth_path ?? "—"}
              </div>
              <div style={{ marginTop: 6 }}>
                <b>codex CLI</b>
                <br />
                {cliPath === undefined
                  ? "查询中…"
                  : cliPath ?? "未找到（自动激活不可用，可设 CODEX_CLI_PATH）"}
              </div>
            </div>
            <p className="hint">
              每次切换账号前，旧的 auth.json 都会备份到账号库目录下的 backups/。
            </p>
            <button
              style={{ marginTop: 8 }}
              onClick={() => api.openDataDir().catch(() => undefined)}
            >
              打开账号库目录
            </button>
          </div>
        </div>

        <div
          style={{
            display: "flex",
            gap: 8,
            padding: 16,
            borderTop: "1px solid var(--border)",
          }}
        >
          <button className="primary" style={{ flex: 1 }} disabled={saving} onClick={save}>
            {saving ? "保存中…" : "保存设置"}
          </button>
          <button onClick={onClose}>取消</button>
        </div>
      </div>
    </div>
  );
}
