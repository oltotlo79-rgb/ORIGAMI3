// 形の2Dプレビュー(PRO-001)と、先端をつまんで動かす操作(PRO-006〜008)。
// 親から子へつながる出っぱりをそのまま描く。線の長さ=出っぱりの長さ、
// 線の太さ=太さ(膨らみ)。数値を読まなくても「今どんな形を頼もうとしているか」が
// 一目で分かるようにする(設計原則3b)。
//
// 先端には丸いつまみを出し、つまんで動かすと「その先を出したい場所」になる。
// つまみは押して動かす操作のほかに、Tabで選んで矢印キーでも動かせ、
// DeleteやBackSpaceで自動の場所へ戻せる。場所そのものの計算は
// `proposalLayout.ts` と `skeleton.ts` の純関数に置き、ここには描画と入力だけを置く。

import { useMemo, useRef, useState } from "react";
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
const TIP_FOCUS_RING_SCALE = 1.8;
/**
 * 先端の操作丸とfocus輪を、割り当てた領域の80%以内に収める。
 * 実機の最大重なりは13.582×17.106 CSS px、操作丸は17.640px、
 * focus輪は31.752pxだった。実測値を境目にせず、同時に12個存在する
 * 操作丸を割当枠の80%以内に収める。focus輪は同時に1個だけなので、
 * 丸同士の間隔へ12個分を足さず、外枠との間だけ輪の実寸を確保する。
 */
const TIP_HANDLE_SLOT_OCCUPANCY = 0.8;

type PreviewPoint = readonly [number, number];

interface PlacedTipHandle {
  center: PreviewPoint;
  reservedRadius: number;
}

function squaresOverlap(
  first: PlacedTipHandle,
  secondCenter: PreviewPoint,
  secondRadius: number,
): boolean {
  return (
    Math.abs(first.center[0] - secondCenter[0]) <
      first.reservedRadius + secondRadius &&
    Math.abs(first.center[1] - secondCenter[1]) <
      first.reservedRadius + secondRadius
  );
}

function candidateIntersectsLabel(
  center: PreviewPoint,
  radius: number,
  part: PreviewPartLayout,
): boolean {
  const labelTop = Math.max(part.labelBounds.top, part.labelBounds.bottom);
  const labelBottom = Math.min(part.labelBounds.top, part.labelBounds.bottom);
  return (
    center[0] + radius > part.labelBounds.left &&
    center[0] - radius < part.labelBounds.right &&
    center[1] + radius > labelBottom &&
    center[1] - radius < labelTop
  );
}

/**
 * 込み入った形でも、操作丸だけを決定的な空き位置へ並べる。
 * 線の実先端 `part.end` は変えず、空いている形では従来位置をそのまま使う。
 */
function placeTipHandles(
  parts: readonly PreviewPartLayout[],
  frameRadius: number,
): Map<number, PreviewPoint> {
  const hasDecidedPosition = (part: PreviewPartLayout) =>
    part.tipPos !== null && part.tipPos !== undefined;
  // 決めた場所は保存値と同じ点へ表示する既存契約を守る。先にその場所を
  // 予約し、まだ自動配置の丸だけを空き位置へ逃がす。
  const tips = parts
    .filter((part) => part.isTip)
    .sort((first, second) =>
      !hasDecidedPosition(first) && hasDecidedPosition(second)
        ? 1
        : hasDecidedPosition(first) && !hasDecidedPosition(second)
          ? -1
          : first.id - second.id,
    );
  const originalCentersDoNotOverlap = tips.every((part, index) =>
    tips.slice(index + 1).every((other) => {
      const partRadius = part.handleRadius + part.handleStrokeWidth / 2;
      const otherRadius = other.handleRadius + other.handleStrokeWidth / 2;
      return (
        Math.abs(part.end[0] - other.end[0]) >= partRadius + otherRadius ||
        Math.abs(part.end[1] - other.end[1]) >= partRadius + otherRadius
      );
    }),
  );
  if (originalCentersDoNotOverlap) {
    return new Map(tips.map((part) => [part.id, part.end]));
  }
  const placed: PlacedTipHandle[] = [];
  const result = new Map<number, PreviewPoint>();
  const largestReservedRadius = Math.max(
    ...tips.map(
      (part) =>
        (part.handleRadius + part.handleStrokeWidth / 2) /
        TIP_HANDLE_SLOT_OCCUPANCY,
    ),
    0,
  );
  const largestVisibleRadius = Math.max(
    ...tips.map(
      (part) =>
        part.handleRadius * TIP_FOCUS_RING_SCALE + part.handleStrokeWidth,
    ),
    0,
  );
  const gridStep = Math.max(largestReservedRadius * 2, frameRadius * 0.1);

  for (const part of tips) {
    const visibleRadius =
      part.handleRadius * TIP_FOCUS_RING_SCALE + part.handleStrokeWidth;
    const reservedRadius =
      (part.handleRadius + part.handleStrokeWidth / 2) /
      TIP_HANDLE_SLOT_OCCUPANCY;
    if (hasDecidedPosition(part)) {
      placed.push({ center: part.end, reservedRadius });
      result.set(part.id, part.end);
      continue;
    }
    const gridCandidates: PreviewPoint[] = [];
    for (
      let y = -frameRadius + largestVisibleRadius;
      y <= frameRadius - largestVisibleRadius + 1e-9;
      y += gridStep
    ) {
      for (
        let x = -frameRadius + largestVisibleRadius;
        x <= frameRadius - largestVisibleRadius + 1e-9;
        x += gridStep
      ) {
        gridCandidates.push([x, y]);
      }
    }
    gridCandidates.sort(
      (first, second) =>
        Math.hypot(first[0] - part.end[0], first[1] - part.end[1]) -
          Math.hypot(second[0] - part.end[0], second[1] - part.end[1]) ||
        first[1] - second[1] ||
        first[0] - second[0],
    );
    const candidates = gridCandidates;

    const insideFrame = (center: PreviewPoint) =>
      Math.abs(center[0]) + visibleRadius <= frameRadius &&
      Math.abs(center[1]) + visibleRadius <= frameRadius;
    const hasHandleSpace = (center: PreviewPoint) =>
      !placed.some((other) =>
        squaresOverlap(other, center, reservedRadius),
      );
    const hasLabelSpace = (center: PreviewPoint) =>
      !parts.some((other) =>
        candidateIntersectsLabel(center, visibleRadius, other),
      );
    const center =
      candidates.find(
        (candidate) =>
          insideFrame(candidate) &&
          hasHandleSpace(candidate) &&
          hasLabelSpace(candidate),
      ) ??
      candidates.find(
        (candidate) => insideFrame(candidate) && hasHandleSpace(candidate),
      ) ??
      part.end;

    placed.push({ center, reservedRadius });
    result.set(part.id, center);
  }

  return result;
}

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
  const layout = useMemo(() => calculatePreviewFrame(skeleton), [skeleton]);
  const pathLabels = skeletonPathLabels(skeleton);
  const svgRef = useRef<SVGSVGElement | null>(null);
  // つまんでいる最中か、どの先端を選んでいるかだけの一時的な表示状態。
  // 保存する値(場所そのもの)はストアに置く。
  const [draggingId, setDraggingId] = useState<number | null>(null);
  const [focusedId, setFocusedId] = useState<number | null>(null);
  const canMove = onTipPosChange !== undefined && !disabled;
  const tipHandleCenters = useMemo(
    () => placeTipHandles(layout.parts, layout.frameRadius),
    [layout],
  );
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
      role="group"
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
          const handleCenter = tipHandleCenters.get(part.id) ?? part.end;
          const handleWasMoved =
            Math.abs(handleCenter[0] - part.end[0]) > 1e-9 ||
            Math.abs(handleCenter[1] - part.end[1]) > 1e-9;
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
              {handleWasMoved && (
                <>
                  <line
                    data-tip-handle-leader={part.id}
                    x1={part.end[0]}
                    y1={-part.end[1]}
                    x2={handleCenter[0]}
                    y2={-handleCenter[1]}
                    stroke="var(--color-text-muted)"
                    strokeWidth={part.handleStrokeWidth}
                    strokeDasharray={`${part.handleStrokeWidth * 2} ${part.handleStrokeWidth * 2}`}
                    pointerEvents="none"
                    aria-hidden="true"
                  />
                  <circle
                    data-tip-handle-origin={part.id}
                    cx={part.end[0]}
                    cy={-part.end[1]}
                    r={part.handleStrokeWidth * 1.5}
                    fill={LIMB_COLOR}
                    pointerEvents="none"
                    aria-hidden="true"
                  />
                </>
              )}
              {different && (
                <circle
                  className="tip-position-different-ring"
                  data-tip-position-different={part.id}
                  data-position-used={positionState?.used ?? "automatic"}
                  cx={handleCenter[0]}
                  cy={-handleCenter[1]}
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
                  cx={handleCenter[0]}
                  cy={-handleCenter[1]}
                  r={part.handleRadius * TIP_FOCUS_RING_SCALE}
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
                cx={handleCenter[0]}
                cy={-handleCenter[1]}
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
