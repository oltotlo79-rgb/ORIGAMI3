// 形の2Dプレビュー(PRO-001)。親から子へつながる出っぱりをそのまま描く。
// 線の長さ=出っぱりの長さ、線の太さ=太さ(膨らみ)。数値を読まなくても
// 「今どんな形を頼もうとしているか」が一目で分かるようにする(設計原則3b)。

import {
  PROPOSAL_PREVIEW_MAX_WIDTH_PX,
  calculatePreviewFrame,
} from "../../lib/proposalLayout";
import type { Skeleton } from "../../lib/types";

const BODY_COLOR = "#8a5a2b";
const LIMB_COLOR = "#3b6fc9";

export function SkeletonPreview({ skeleton }: { skeleton: Skeleton }) {
  const layout = calculatePreviewFrame(skeleton);
  // SVGはy軸が下向きなので、配置のyを反転して「上が上」に見えるようにする
  return (
    <svg
      className="skeleton-preview"
      viewBox={`${layout.viewBox.x} ${layout.viewBox.y} ${layout.viewBox.width} ${layout.viewBox.height}`}
      role="img"
      aria-label="形見本"
      style={{
        width: "100%",
        maxWidth: `${PROPOSAL_PREVIEW_MAX_WIDTH_PX}px`,
        height: "auto",
      }}
    >
      {layout.parts.map((part) => (
        <g key={part.id}>
          <line
            data-preview-part={part.id}
            x1={part.start[0]}
            y1={-part.start[1]}
            x2={part.end[0]}
            y2={-part.end[1]}
            stroke={LIMB_COLOR}
            strokeWidth={part.strokeWidth}
            strokeLinecap="round"
          />
          <circle
            data-preview-connection={part.id}
            cx={part.start[0]}
            cy={-part.start[1]}
            r={part.connectionRadius}
            fill={BODY_COLOR}
          />
        </g>
      ))}
      {layout.parts.map((part) => (
        <text
          key={part.id}
          data-preview-label={part.id}
          x={part.labelPosition[0]}
          y={-part.labelPosition[1]}
          fill="#333"
          stroke="var(--color-surface)"
          strokeWidth={part.labelStrokeWidth}
          paintOrder="stroke"
          fontSize={part.labelFontSize}
          textAnchor="middle"
          dominantBaseline="middle"
        >
          {part.label}
        </text>
      ))}
      <circle cx={0} cy={0} r={layout.bodyRadius} fill={BODY_COLOR} />
    </svg>
  );
}
