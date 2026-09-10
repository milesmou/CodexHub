import type { ReactNode } from "react";

interface Props {
  /** 剩余百分比 0-100 */
  percent: number;
  color: string;
  size?: number;
  stroke?: number;
  /** 环心内容，默认显示百分比 */
  children?: ReactNode;
}

/** 额度环形图：填充比例表示「还剩多少」，越满越健康。 */
export function QuotaRing({
  percent,
  color,
  size = 62,
  stroke = 6,
  children,
}: Props) {
  const r = (size - stroke) / 2;
  const circumference = 2 * Math.PI * r;
  const clamped = Math.max(0, Math.min(100, percent));
  const dash = (clamped / 100) * circumference;

  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke="var(--border)"
        strokeWidth={stroke}
      />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke={color}
        strokeWidth={stroke}
        strokeLinecap="round"
        strokeDasharray={`${dash} ${circumference}`}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
      />
      {children ?? (
        <text
          x={size / 2}
          y={size / 2}
          textAnchor="middle"
          dominantBaseline="central"
          fontSize={size * 0.26}
          fontWeight={500}
          fill="var(--text)"
        >
          {Math.round(clamped)}%
        </text>
      )}
    </svg>
  );
}
