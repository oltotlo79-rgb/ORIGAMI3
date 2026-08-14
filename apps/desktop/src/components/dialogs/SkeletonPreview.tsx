// 形の2Dプレビュー(PRO-001)。親から子へつながる出っぱりをそのまま描く。
// 線の長さ=出っぱりの長さ、線の太さ=太さ(膨らみ)。数値を読まなくても
// 「今どんな形を頼もうとしているか」が一目で分かるようにする(設計原則3b)。

import { previewLayout } from "../../lib/skeleton";
import type { Skeleton } from "../../lib/types";

const BODY_COLOR = "#8a5a2b";
const LIMB_COLOR = "#3b6fc9";

/** 線の太さ = 先端の膨らみ半径 × この割合 */
const STROKE_RATIO = 0.5;

export function SkeletonPreview({ skeleton }: { skeleton: Skeleton }) {
  const layout = previewLayout(skeleton);
  const reach = Math.max(
    ...layout.flatMap((l) => [
      Math.hypot(...l.start),
      l.radius * STROKE_RATIO + Math.hypot(...l.end),
    ]),
    0.5,
  );
  const r = reach * 1.25;
  // SVGはy軸が下向きなので、配置のyを反転して「上が上」に見えるようにする
  return (
    <svg
      className="skeleton-preview"
      viewBox={`${-r} ${-r} ${2 * r} ${2 * r}`}
      role="img"
      aria-label="形見本"
      style={{ width: "100%", maxWidth: "200px", height: "auto" }}
    >
      {layout.map((l) => (
        <g key={l.id}>
          <line
            data-preview-part={l.id}
            x1={l.start[0]}
            y1={-l.start[1]}
            x2={l.end[0]}
            y2={-l.end[1]}
            stroke={LIMB_COLOR}
            strokeWidth={Math.max(l.radius * STROKE_RATIO, r * 0.01)}
            strokeLinecap="round"
          />
          <circle
            data-preview-connection={l.id}
            cx={l.start[0]}
            cy={-l.start[1]}
            r={r * 0.022}
            fill={BODY_COLOR}
          />
        </g>
      ))}
      {layout.map((l) => {
        const dx = l.end[0] - l.start[0];
        const dy = l.end[1] - l.start[1];
        const length = Math.hypot(dx, dy) || 1;
        const offset = r * 0.08;
        // 次の線の延長上を避け、各線の横へ呼び名を置く。
        const labelX = l.end[0] - (dy / length) * offset;
        const labelY = l.end[1] + (dx / length) * offset;
        return (
          <text
            key={l.id}
            data-preview-label={l.id}
            x={labelX}
            y={-labelY}
            fill="#333"
            stroke="var(--color-surface)"
            strokeWidth={r * 0.025}
            paintOrder="stroke"
            fontSize={r * 0.11}
            textAnchor="middle"
            dominantBaseline="middle"
          >
            {l.label}
          </text>
        );
      })}
      <circle cx={0} cy={0} r={r * 0.05} fill={BODY_COLOR} />
    </svg>
  );
}
