// 骨格の2Dプレビュー(PRO-001)。胴の中心から出っぱりが放射状に伸びる図を描く。
// 線の長さ=出っぱりの長さ、線の太さ=太さ(膨らみ)。数値を読まなくても
// 「今どんな形を頼もうとしているか」が一目で分かるようにする(設計原則3b)。

import { limbLabel, previewLayout } from "../../lib/skeleton";
import type { Skeleton } from "../../lib/types";

const BODY_COLOR = "#8a5a2b";
const LIMB_COLOR = "#3b6fc9";

/** 線の太さ = 先端の膨らみ半径 × この割合 */
const STROKE_RATIO = 0.5;

export function SkeletonPreview({ skeleton }: { skeleton: Skeleton }) {
  const layout = previewLayout(skeleton);
  const reach = Math.max(
    ...layout.map((l) => l.radius * STROKE_RATIO + Math.hypot(...l.end)),
    0.5,
  );
  const r = reach * 1.25;
  // SVGはy軸が下向きなので、骨格のyを反転して「上が上」に見えるようにする
  return (
    <svg
      className="skeleton-preview"
      viewBox={`${-r} ${-r} ${2 * r} ${2 * r}`}
      role="img"
      aria-label={`出っぱり${layout.length}本の骨格`}
    >
      {layout.map((l, i) => (
        <g key={l.id}>
          <line
            x1={0}
            y1={0}
            x2={l.end[0]}
            y2={-l.end[1]}
            stroke={LIMB_COLOR}
            strokeWidth={Math.max(l.radius * STROKE_RATIO, r * 0.01)}
            strokeLinecap="round"
          />
          <text
            x={l.end[0] * 1.12}
            y={-l.end[1] * 1.12}
            fill="#333"
            fontSize={r * 0.11}
            textAnchor="middle"
            dominantBaseline="middle"
          >
            {limbLabel(i)}
          </text>
        </g>
      ))}
      <circle cx={0} cy={0} r={r * 0.05} fill={BODY_COLOR} />
    </svg>
  );
}
