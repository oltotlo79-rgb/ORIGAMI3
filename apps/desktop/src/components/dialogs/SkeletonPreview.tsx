// 形の2Dプレビュー(PRO-001)と、先端をつまんで動かす操作(PRO-006〜008)。
// 親から子へつながる出っぱりをそのまま描く。線の長さ=出っぱりの長さ、
// 線の太さ=太さ(膨らみ)。数値を読まなくても「今どんな形を頼もうとしているか」が
// 一目で分かるようにする(設計原則3b)。
//
// 先端には丸いつまみを出し、つまんで動かすと「その先を出したい場所」になる。
// つまみは押して動かす操作のほかに、Tabで選んで矢印キーでも動かせ、
// DeleteやBackSpaceで自動の場所へ戻せる。場所そのものの計算は
// `proposalLayout.ts` と `skeleton.ts` の純関数に置き、ここには描画と入力だけを置く。

import { useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import {
  PROPOSAL_PREVIEW_MAX_WIDTH_PX,
  TIP_KEY_STEP,
  TIP_KEY_STEP_LARGE,
  calculatePreviewFrame,
  clientPointToTipPos,
  previewPointToTipPos,
} from "../../lib/proposalLayout";
import type { PreviewPartLayout } from "../../lib/proposalLayout";
import { clampTipPos, skeletonPathLabels } from "../../lib/skeleton";
import type { Skeleton, TipPos2d } from "../../lib/types";
import type { ProposalLeafPositionState } from "../../lib/proposalPosition";

const BODY_COLOR = "#8a5a2b";
const LIMB_COLOR = "#3b6fc9";

export interface SkeletonPreviewProps {
  skeleton: Skeleton;
  disabled?: boolean;
  positionStates?: readonly ProposalLeafPositionState[];
  /**
   * 先端の場所が変わったときに呼ぶ。`null` は「自動の場所へ戻す」。
   * 渡さないときは見るだけの絵になる。
   */
  onTipPosChange?: (id: number, pos: TipPos2d | null) => void;
}

export function SkeletonPreview({
  skeleton,
  disabled = false,
  positionStates = [],
  onTipPosChange,
}: SkeletonPreviewProps) {
  const layout = calculatePreviewFrame(skeleton);
  const pathLabels = skeletonPathLabels(skeleton);
  const svgRef = useRef<SVGSVGElement | null>(null);
  // つまんでいる最中か、どの先端を選んでいるかだけの一時的な表示状態。
  // 保存する値(場所そのもの)はストアに置く。
  const [draggingId, setDraggingId] = useState<number | null>(null);
  const [focusedId, setFocusedId] = useState<number | null>(null);
  const canMove = onTipPosChange !== undefined && !disabled;
  const positionStateByLeaf = new Map(
    positionStates.map((state) => [state.leaf_id, state]),
  );

  const moveTipTo = (id: number, clientX: number, clientY: number) => {
    const svg = svgRef.current;
    if (!svg || !onTipPosChange) return;
    onTipPosChange(
      id,
      clientPointToTipPos(
        [clientX, clientY],
        svg.getBoundingClientRect(),
        layout,
      ),
    );
  };

  const nudgeTip = (part: PreviewPartLayout, dx: number, dy: number) => {
    if (!onTipPosChange) return;
    // まだ場所を決めていない先端は、いま描かれている場所から動かし始める。
    const base =
      part.tipPos ?? previewPointToTipPos(part.end, layout.positionRadius);
    onTipPosChange(part.id, clampTipPos({ x: base.x + dx, y: base.y + dy }));
  };

  const endDrag = () => setDraggingId(null);

  const handleKeyDown = (
    part: PreviewPartLayout,
    event: ReactKeyboardEvent<SVGCircleElement>,
  ) => {
    if (!onTipPosChange) return;
    const step = event.shiftKey ? TIP_KEY_STEP_LARGE : TIP_KEY_STEP;
    switch (event.key) {
      case "ArrowLeft":
        nudgeTip(part, -step, 0);
        break;
      case "ArrowRight":
        nudgeTip(part, step, 0);
        break;
      case "ArrowUp":
        nudgeTip(part, 0, step);
        break;
      case "ArrowDown":
        nudgeTip(part, 0, -step);
        break;
      case "Delete":
      case "Backspace":
        onTipPosChange(part.id, null);
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  // SVGはy軸が下向きなので、配置のyを反転して「上が上」に見えるようにする
  return (
    <svg
      ref={svgRef}
      className="skeleton-preview"
      viewBox={`${layout.viewBox.x} ${layout.viewBox.y} ${layout.viewBox.width} ${layout.viewBox.height}`}
      role="img"
      aria-label="形見本"
      style={{
        width: "100%",
        maxWidth: `${PROPOSAL_PREVIEW_MAX_WIDTH_PX}px`,
        height: "auto",
      }}
      onPointerMove={(event) => {
        if (draggingId === null) return;
        event.preventDefault();
        moveTipTo(draggingId, event.clientX, event.clientY);
      }}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onLostPointerCapture={endDrag}
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
      {layout.parts
        .filter((part) => part.isTip)
        .map((part) => {
          const decided = part.tipPos !== null;
          const name = (pathLabels.get(part.id) ?? [part.label]).join("の");
          const positionState = positionStateByLeaf.get(part.id);
          const different = positionState?.kind === "different";
          const usedText =
            positionState?.used === "completion"
              ? "完成形で動かした場所を使います"
              : "紙の上で動かした場所を使います";
          return (
            <g key={part.id}>
              {different && (
                <circle
                  className="tip-position-different-ring"
                  data-tip-position-different={part.id}
                  data-position-used={positionState?.used ?? "automatic"}
                  cx={part.end[0]}
                  cy={-part.end[1]}
                  r={part.handleRadius * 1.45}
                  fill="none"
                  stroke="var(--color-danger)"
                  strokeWidth={part.handleStrokeWidth * 1.5}
                  strokeDasharray={`${part.handleStrokeWidth * 2.5} ${part.handleStrokeWidth * 1.8}`}
                  aria-hidden="true"
                />
              )}
              {focusedId === part.id && (
                // 選んでいる先端の目印。絵の中の長さで描くので、見本の大きさが
                // 変わっても太さの見え方が変わらない。
                <circle
                  data-tip-focus-ring={part.id}
                  className="tip-focus-ring"
                  cx={part.end[0]}
                  cy={-part.end[1]}
                  r={part.handleRadius * 1.8}
                  fill="none"
                  stroke="var(--color-accent)"
                  strokeWidth={part.handleStrokeWidth * 2}
                />
              )}
              <circle
                className={
                  `${decided ? "tip-handle tip-handle-decided" : "tip-handle"}${different ? " tip-handle-different" : ""}`
                }
                data-tip-handle={part.id}
                data-tip-decided={decided ? "true" : "false"}
                cx={part.end[0]}
                cy={-part.end[1]}
                r={part.handleRadius}
                fill={decided ? "var(--color-accent)" : "var(--color-surface)"}
                stroke={decided ? "var(--color-surface)" : LIMB_COLOR}
                strokeWidth={part.handleStrokeWidth}
                role={canMove ? "button" : undefined}
                tabIndex={canMove ? 0 : undefined}
                aria-label={
                  canMove
                    ? `${name}を出したい場所（${decided ? "決めました" : "自動"}）${different ? `。完成形と紙の上で場所が違います。${usedText}` : ""}`
                    : undefined
                }
                onPointerDown={(event) => {
                  if (!canMove) return;
                  event.preventDefault();
                  event.stopPropagation();
                  setDraggingId(part.id);
                  const handle = event.currentTarget;
                  // つまんだ先をそのまま矢印キーでも動かせるようにする
                  handle.focus();
                  try {
                    handle.setPointerCapture(event.pointerId);
                  } catch {
                    // 掴み続けられない環境では、そのまま絵の上での操作として続ける
                  }
                }}
                onFocus={() => setFocusedId(part.id)}
                onBlur={() =>
                  setFocusedId((current) =>
                    current === part.id ? null : current,
                  )
                }
                onKeyDown={(event) => handleKeyDown(part, event)}
              />
            </g>
          );
        })}
    </svg>
  );
}
