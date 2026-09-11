import { useCallback, useEffect, useMemo, useState } from "react";
import { api, errorText } from "../api";
import type { TokenDay, TokenStats } from "../types";
import { fmtPercent, fmtTokens } from "../format";

interface Props {
  onClose: () => void;
}

/** 时间范围选项：value = 0 表示全部历史 */
const RANGES: { label: string; value: number }[] = [
  { label: "今天", value: 1 },
  { label: "7 天", value: 7 },
  { label: "30 天", value: 30 },
];

/** 把最大值向上取到一个「好看」的刻度（1/2/5 × 10^n）。 */
function niceStep(max: number): number {
  if (max <= 0) return 1;
  const rough = max / 4;
  const mag = Math.pow(10, Math.floor(Math.log10(rough)));
  const norm = rough / mag;
  const snapped = norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 5 ? 5 : 10;
  return snapped * mag;
}

/**
 * 每日消耗柱状图。
 *
 * 纯 SVG 手写，不引第三方图表库 —— 只有一根柱子一种数据，
 * 引个库进来反而加重打包体积和启动时间。
 */
function DailyChart({ days }: { days: TokenDay[] }) {
  const [hover, setHover] = useState<number | null>(null);

  const W = 1000;
  const H = 250;
  const padL = 66;
  const padR = 14;
  const padT = 18;
  const padB = 34;
  const pw = W - padL - padR;
  const ph = H - padT - padB;

  const max = Math.max(1, ...days.map((d) => d.total));
  const step = niceStep(max);
  const top = Math.max(step, Math.ceil(max / step) * step);
  const y = (v: number) => padT + ph - (v / top) * ph;

  const slot = pw / Math.max(1, days.length);
  const barW = Math.max(2, Math.min(38, slot * 0.66));

  // x 轴最多标 8 个日期，免得挤成一团
  const labelEvery = Math.max(1, Math.ceil(days.length / 8));
  const gridLines = [0, 0.25, 0.5, 0.75, 1].map((f) => f * top);

  const hovered = hover !== null ? days[hover] : null;

  return (
    <div className="chart-wrap">
      <svg
        className="chart"
        viewBox={`0 0 ${W} ${H}`}
        onMouseLeave={() => setHover(null)}
      >
        {/* 横向刻度线 */}
        {gridLines.map((v, i) => (
          <g key={i}>
            <line
              x1={padL}
              x2={W - padR}
              y1={y(v)}
              y2={y(v)}
              stroke="var(--border)"
              strokeWidth={1}
              strokeDasharray={i === 0 ? undefined : "3 4"}
            />
            <text
              x={padL - 8}
              y={y(v) + 4}
              textAnchor="end"
              fontSize={11}
              fill="var(--text-faint)"
            >
              {fmtTokens(v)}
            </text>
          </g>
        ))}

        {/* 柱子 */}
        {days.map((d, i) => {
          const cx = padL + slot * i + slot / 2;
          const h = Math.max(1, (d.total / top) * ph);
          const active = hover === i;
          return (
            <g key={d.date}>
              {/* 热区：整条竖带都能触发，不用精准对准细柱子 */}
              <rect
                x={padL + slot * i}
                y={padT}
                width={slot}
                height={ph}
                fill="transparent"
                onMouseEnter={() => setHover(i)}
              />
              <rect
                x={cx - barW / 2}
                y={padT + ph - h}
                width={barW}
                height={h}
                rx={Math.min(3, barW / 2)}
                fill={active ? "var(--accent)" : "#6f68c9"}
                opacity={hover === null || active ? 1 : 0.55}
                pointerEvents="none"
              />
            </g>
          );
        })}

        {/* x 轴日期 */}
        {days.map((d, i) =>
          i % labelEvery === 0 || i === days.length - 1 ? (
            <text
              key={d.date}
              x={padL + slot * i + slot / 2}
              y={H - 12}
              textAnchor="middle"
              fontSize={11}
              fill={hover === i ? "var(--text)" : "var(--text-faint)"}
            >
              {d.date.slice(5)}
            </text>
          ) : null,
        )}
      </svg>

      {/* 悬停读数放在图外，避免跟柱子打架 */}
      <div className="chart-readout">
        {hovered ? (
          <>
            <b>{hovered.date}</b>
            <span>{fmtTokens(hovered.total)} tokens</span>
            <span className="dim">
              {hovered.calls} 次调用 · {hovered.threads} 条会话
            </span>
          </>
        ) : (
          <span className="dim">把鼠标移到柱子上看当天明细</span>
        )}
      </div>
    </div>
  );
}

/** 单条横向占比条，用于模型分布。 */
function ShareBar({
  label,
  value,
  total,
  sub,
}: {
  label: string;
  value: number;
  total: number;
  sub: string;
}) {
  const pct = total > 0 ? value / total : 0;
  return (
    <div className="share-row">
      <div className="share-head">
        <span className="share-name" title={label}>
          {label}
        </span>
        <span className="share-val">
          {fmtTokens(value)} · {fmtPercent(pct)}
        </span>
      </div>
      <div className="share-track">
        <div className="share-fill" style={{ width: `${Math.max(1, pct * 100)}%` }} />
      </div>
      <span className="share-sub">{sub}</span>
    </div>
  );
}

export function StatsPage({ onClose }: Props) {
  const [range, setRange] = useState<number>(1);
  const [stats, setStats] = useState<TokenStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (days: number) => {
    setLoading(true);
    setError(null);
    try {
      setStats(await api.tokenStats(days === 0 ? undefined : days));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load(range);
  }, [load, range]);

  /** 日均：按真正有消耗的天数算，不然中间空掉的日子也会拉低平均值 */
  const dailyAvg = useMemo(() => {
    if (!stats || stats.days.length === 0) return 0;
    return stats.total / stats.days.length;
  }, [stats]);

  const peak = useMemo(() => {
    if (!stats || stats.days.length === 0) return null;
    return stats.days.reduce((a, b) => (b.total > a.total ? b : a));
  }, [stats]);

  return (
    <div className="page-mask" onClick={onClose}>
      <div className="page" onClick={(e) => e.stopPropagation()}>
        <div className="page-head">
          <div className="page-title">
            <h2>Token 消耗统计</h2>
            <p className="page-sub">
              {stats
                ? `${stats.first_day ?? "—"} ~ ${stats.last_day ?? "—"} · 扫描 ${stats.files} 份会话文件`
                : "统计中…"}
            </p>
          </div>

          <div className="page-actions">
            {RANGES.map((r) => (
              <button
                key={r.value}
                className={range === r.value ? "primary" : "ghost"}
                disabled={loading}
                onClick={() => setRange(r.value)}
              >
                {r.label}
              </button>
            ))}
            <button className="ghost" disabled={loading} onClick={() => void load(range)}>
              {loading ? <span className="spin">↻</span> : "重新统计"}
            </button>
            <button className="ghost" onClick={onClose}>
              关闭
            </button>
          </div>
        </div>

        <div className="page-body">
          {error && <div className="banner warn">{error}</div>}

          {!stats && loading && (
            <div className="empty">
              <p>正在扫描会话文件…</p>
            </div>
          )}

          {stats && (
            <>
              {/* ---- 概览数字 ---- */}
              <div className="stat-tiles">
                <div className="stat-tile">
                  <span className="stat-label">总消耗</span>
                  <b className="stat-value">{fmtTokens(stats.total)}</b>
                  <span className="stat-sub">
                    {stats.calls.toLocaleString("zh-CN")} 次调用 · {stats.threads} 条会话
                  </span>
                </div>
                <div className="stat-tile">
                  <span className="stat-label">日均</span>
                  <b className="stat-value">{fmtTokens(dailyAvg)}</b>
                  <span className="stat-sub">按 {stats.days.length} 个有消耗的天摊</span>
                </div>
                <div className="stat-tile">
                  <span className="stat-label">峰值</span>
                  <b className="stat-value">{peak ? fmtTokens(peak.total) : "—"}</b>
                  <span className="stat-sub">{peak ? peak.date : "—"}</span>
                </div>
                <div className="stat-tile">
                  <span className="stat-label">缓存命中率</span>
                  <b className="stat-value">
                    {stats.input > 0 ? fmtPercent(stats.cached / stats.input) : "—"}
                  </b>
                  <span className="stat-sub">输入里有多少是复用上下文</span>
                </div>
              </div>

              {/* ---- 用量构成 ---- */}
              <section className="panel">
                <h3>用量构成</h3>
                <div className="breakdown">
                  <div className="bd-row">
                    <span className="bd-name">输入总量</span>
                    <span className="bd-val">{fmtTokens(stats.input)}</span>
                    <span className="bd-note">每次调用都会把上下文重新投喂一遍</span>
                  </div>
                  <div className="bd-row sub">
                    <span className="bd-name">其中缓存命中</span>
                    <span className="bd-val">{fmtTokens(stats.cached)}</span>
                    <span className="bd-note">单价低，但不等于没消耗</span>
                  </div>
                  <div className="bd-row sub">
                    <span className="bd-name">其中新增输入</span>
                    <span className="bd-val">{fmtTokens(stats.input - stats.cached)}</span>
                    <span className="bd-note">真正新读进去的内容</span>
                  </div>
                  <div className="bd-row">
                    <span className="bd-name">输出</span>
                    <span className="bd-val">{fmtTokens(stats.output)}</span>
                    <span className="bd-note">模型写出来的</span>
                  </div>
                  <div className="bd-row">
                    <span className="bd-name">推理</span>
                    <span className="bd-val">{fmtTokens(stats.reasoning)}</span>
                    <span className="bd-note">思维链，通常已计在输出内</span>
                  </div>
                </div>
              </section>

              {/* ---- 每日柱状图 ---- */}
              <section className="panel">
                <h3>每日消耗</h3>
                {stats.days.length === 0 ? (
                  <p className="hint">这个区间里没有记录。</p>
                ) : (
                  <DailyChart days={stats.days} />
                )}
              </section>

              {/* ---- 模型分布 ---- */}
              {stats.models.length > 0 && (
                <section className="panel">
                  <h3>模型分布</h3>
                  {stats.models.map((m) => (
                    <ShareBar
                      key={m.model}
                      label={m.model}
                      value={m.total}
                      total={stats.total}
                      sub={`${m.calls.toLocaleString("zh-CN")} 次调用 · 输入 ${fmtTokens(m.input)} · 输出 ${fmtTokens(m.output)}`}
                    />
                  ))}
                  {stats.models.reduce((s, m) => s + m.calls, 0) < stats.calls && (
                    <p className="hint">
                      还有{" "}
                      {stats.calls - stats.models.reduce((s, m) => s + m.calls, 0)}{" "}
                      次调用来自旧版会话文件，那时没记模型名，因此没算进上面的分布。
                    </p>
                  )}
                </section>
              )}

              {/* ---- 每日明细表 ---- */}
              <section className="panel">
                <h3>每日明细</h3>
                {stats.days.length === 0 ? (
                  <p className="hint">暂无数据。</p>
                ) : (
                  <table className="stat-table">
                    <thead>
                      <tr>
                        <th>日期</th>
                        <th className="num">总量</th>
                        <th className="num">输入</th>
                        <th className="num">缓存</th>
                        <th className="num">输出</th>
                        <th className="num">调用</th>
                        <th className="num">会话</th>
                        <th className="bar-col">占比</th>
                      </tr>
                    </thead>
                    <tbody>
                      {[...stats.days].reverse().map((d) => {
                        const pct = stats.total > 0 ? d.total / stats.total : 0;
                        return (
                          <tr key={d.date}>
                            <td>{d.date}</td>
                            <td className="num strong">{fmtTokens(d.total)}</td>
                            <td className="num">{fmtTokens(d.input)}</td>
                            <td className="num dim">{fmtTokens(d.cached)}</td>
                            <td className="num">{fmtTokens(d.output)}</td>
                            <td className="num">{d.calls}</td>
                            <td className="num dim">{d.threads || "—"}</td>
                            <td className="bar-col">
                              <div className="mini-track">
                                <div
                                  className="mini-fill"
                                  style={{ width: `${Math.max(2, pct * 100)}%` }}
                                />
                              </div>
                              <span className="mini-pct">{fmtPercent(pct)}</span>
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
              </section>

              {/* ---- 口径说明 ---- */}
              <p className="stat-note">
                {stats.note}
                <br />
                数据来源：<code>~/.codex/sessions</code> 与{" "}
                <code>~/.codex/archived_sessions</code> 下的 rollout 文件。
              </p>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
