// 選んだ候補だけを大きく表示し、紙の上の先端位置を直接動かす画面(作業12)。
// 小さい候補ボタンとは別の画面なので、ボタンの中へ操作要素を入れない。

import { useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { EDGE_COLORS } from "../CpEditor/renderer";
import {
  PAPER_POSITION_KEY_STEP,
  PAPER_POSITION_KEY_STEP_LARGE,
  clampPaperPosition,
  clientPointToPaperPosition,
  paperBounds,
  paperEditorViewBox,
  paperPositionLabelLayout,
  paperPositionLabelLayouts,
  paperPositionChanged,
  paperPositionToPoint,
  paperPositionsFromCandidate,
} from "../../lib/paperPosition";
import { paperPositionEditorWidthPx } from "../../lib/proposalLayout";
import { leafNodes, skeletonPathLabels } from "../../lib/skeleton";
import type {
  PaperPosition2d,
  PaperTipPosition,
  ProposalCandidate,
  Skeleton,
} from "../../lib/types";
import type { ProposalLeafPositionState } from "../../lib/proposalPosition";

export interface PaperPositionEditorProps {
  candidate: ProposalCandidate;
  skeleton: Skeleton;
  positions: readonly PaperTipPosition[];
  disabled?: boolean;
  positionStates?: readonly ProposalLeafPositionState[];
  onPositionChange: (leafId: number, position: PaperPosition2d) => void;
}

export function PaperPositionEditor({
  candidate,
  skeleton,
  positions,
  disabled = false,
  positionStates = [],
  onPositionChange,
}: PaperPositionEditorProps) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  // つまんでいる最中とキーボードで選んでいる印だけが表示専用の一時状態。
  // 紙の上の場所そのものはZustandストアに置く。
  const [draggingId, setDraggingId] = useState<number | null>(null);
  const [focusedId, setFocusedId] = useState<number | null>(null);
  const bounds = paperBounds(candidate.cp);
  const viewBox = paperEditorViewBox(bounds);
  const editorWidth = paperPositionEditorWidthPx(viewBox);
  const original = paperPositionsFromCandidate(candidate);
  const originalByLeaf = new Map(
    original.map((entry) => [entry.leaf_id, entry.position]),
  );
  const currentByLeaf = new Map(
    positions.map((entry) => [entry.leaf_id, entry.position]),
  );
  const labels = skeletonPathLabels(skeleton);
  const positionStateByLeaf = new Map(
    positionStates.map((state) => [state.leaf_id, state]),
  );
  const leafIds = new Set(leafNodes(skeleton).map((node) => node.id));
  // 候補側が壊れて余分な対応を返しても、骨格の先端だけを葉ID順で最大12件描く。
  const handles = original
    .filter((entry) => leafIds.has(entry.leaf_id))
    .slice(0, 12);
  const vertexById = new Map(
    candidate.cp.vertices.map((vertex) => [vertex.id, vertex.pos]),
  );
  const handleRadius = bounds.longSide * 0.028;
  const handleStroke = bounds.longSide * 0.008;
  const labelSize = bounds.longSide * 0.036;
  const lineStroke = bounds.longSide * 0.0045;
  const handleViews = handles.map((entry) => {
    const current = currentByLeaf.get(entry.leaf_id) ?? entry.position;
    const point = paperPositionToPoint(current, bounds);
    const name = (labels.get(entry.leaf_id) ?? [`出っぱり${entry.leaf_id}`]).join(
      "の",
    );
    return { entry, current, point, name };
  });
  const labelByLeaf = new Map(
    paperPositionLabelLayouts(
      handleViews.map(({ entry, point, name }) => ({
        id: entry.leaf_id,
        point,
        label: name,
      })),
      viewBox,
      labelSize,
      handleRadius,
      draggingId ?? focusedId,
    ).map((layout) => [layout.id, layout]),
  );

  const moveToClient = (leafId: number, clientX: number, clientY: number) => {
    if (disabled) return;
    const svg = svgRef.current;
    if (!svg) return;
    onPositionChange(
      leafId,
      clientPointToPaperPosition(
        [clientX, clientY],
        svg.getBoundingClientRect(),
        bounds,
      ),
    );
  };

  const nudge = (
    leafId: number,
    dx: number,
    dy: number,
  ) => {
    if (disabled) return;
    const base = currentByLeaf.get(leafId) ?? originalByLeaf.get(leafId);
    if (!base) return;
    onPositionChange(
      leafId,
      clampPaperPosition({ x: base.x + dx, y: base.y + dy }, bounds),
    );
  };

  const handleKeyDown = (
    leafId: number,
    event: ReactKeyboardEvent<SVGCircleElement>,
  ) => {
    const step = event.shiftKey
      ? PAPER_POSITION_KEY_STEP_LARGE
      : PAPER_POSITION_KEY_STEP;
    switch (event.key) {
      case "ArrowLeft":
        nudge(leafId, -step, 0);
        break;
      case "ArrowRight":
        nudge(leafId, step, 0);
        break;
      case "ArrowUp":
        nudge(leafId, 0, step);
        break;
      case "ArrowDown":
        nudge(leafId, 0, -step);
        break;
      case "Delete":
      case "Backspace": {
        const start = originalByLeaf.get(leafId);
        if (start) onPositionChange(leafId, start);
        break;
      }
      default:
        return;
    }
    event.preventDefault();
  };

  return (
    <svg
      ref={svgRef}
      className="paper-position-editor"
      data-paper-position-editor="large"
      data-disabled={disabled ? "true" : "false"}
      viewBox={`${viewBox.x} ${viewBox.y} ${viewBox.width} ${viewBox.height}`}
      role="group"
      aria-label="紙の上の場所を調整する大きな見本"
      style={{
        width: "100%",
        maxWidth: `${editorWidth}px`,
        aspectRatio: `${viewBox.width} / ${viewBox.height}`,
      }}
      onPointerMove={(event) => {
        if (draggingId === null) return;
        event.preventDefault();
        moveToClient(draggingId, event.clientX, event.clientY);
      }}
      onPointerUp={() => setDraggingId(null)}
      onPointerCancel={() => setDraggingId(null)}
      onLostPointerCapture={() => setDraggingId(null)}
    >
      <rect
        x={bounds.minX}
        y={-bounds.maxY}
        width={bounds.width}
        height={bounds.height}
        fill="#ffffff"
      />
      {candidate.cp.edges.map((edge) => {
        const a = vertexById.get(edge.v0);
        const b = vertexById.get(edge.v1);
        if (!a || !b) return null;
        return (
          <line
            key={edge.id}
            x1={a[0]}
            y1={-a[1]}
            x2={b[0]}
            y2={-b[1]}
            stroke={EDGE_COLORS[edge.kind]}
            strokeWidth={lineStroke}
          />
        );
      })}
      {/* 引き出し線を全件まとめて先に描き、別の先端のつまみや名前へ
          後から線が重ならないようにする。 */}
      <g className="paper-position-label-leaders" aria-hidden="true">
        {handleViews.map(({ entry, point }) => {
          const labelLayout = labelByLeaf.get(entry.leaf_id);
          if (!labelLayout?.visible || !labelLayout.shifted) return null;
          return (
            <line
              key={entry.leaf_id}
              className="paper-position-label-leader"
              x1={point[0]}
              y1={-point[1]}
              x2={labelLayout.x}
              y2={labelLayout.y - labelLayout.fontSize * 0.55}
              stroke="var(--color-text-muted)"
              strokeWidth={lineStroke}
              aria-hidden="true"
            />
          );
        })}
      </g>
      {handleViews.map(({ entry, current, point, name }) => {
        const moved = paperPositionChanged(current, entry.position);
        const positionState = positionStateByLeaf.get(entry.leaf_id);
        const different = positionState?.kind === "different";
        const usedText =
          positionState?.used === "completion"
            ? "完成形で動かした場所を使います"
            : "紙の上で動かした場所を使います";
        const labelLayout = labelByLeaf.get(entry.leaf_id) ?? {
          id: entry.leaf_id,
          ...paperPositionLabelLayout(point, viewBox, name, labelSize, handleRadius),
          shifted: false,
          visible: true,
        };
        return (
          <g key={entry.leaf_id}>
            {different && (
              <circle
                className="paper-position-different-ring"
                data-paper-position-different={entry.leaf_id}
                data-position-used={positionState?.used ?? "automatic"}
                cx={point[0]}
                cy={-point[1]}
                r={handleRadius * 1.45}
                fill="none"
                stroke="var(--color-danger)"
                strokeWidth={handleStroke * 1.5}
                strokeDasharray={`${handleStroke * 2.5} ${handleStroke * 1.8}`}
                aria-hidden="true"
              />
            )}
            {focusedId === entry.leaf_id && (
              <circle
                className="paper-position-focus-ring"
                data-paper-focus-ring={entry.leaf_id}
                cx={point[0]}
                cy={-point[1]}
                r={handleRadius * 1.75}
                fill="none"
                stroke="var(--color-accent)"
                strokeWidth={handleStroke * 1.5}
              />
            )}
            <circle
              className={
                moved
                  ? "paper-position-handle paper-position-handle-moved"
                  : "paper-position-handle"
              }
              data-paper-position-handle={entry.leaf_id}
              data-paper-position-changed={moved ? "true" : "false"}
              cx={point[0]}
              cy={-point[1]}
              r={moved ? handleRadius * 1.18 : handleRadius}
              fill={moved ? "var(--color-accent)" : "var(--color-surface)"}
              stroke={moved ? "var(--color-surface)" : "var(--color-accent)"}
              strokeWidth={handleStroke}
              role="button"
              tabIndex={disabled ? -1 : 0}
              aria-disabled={disabled}
              aria-label={`${name}の紙の上の場所（${moved ? "動かしました" : "この候補のまま"}）${different ? `。完成形と紙の上で場所が違います。${usedText}` : ""}`}
              onPointerDown={(event) => {
                if (disabled) return;
                event.preventDefault();
                event.stopPropagation();
                setDraggingId(entry.leaf_id);
                const handle = event.currentTarget;
                handle.focus();
                try {
                  handle.setPointerCapture(event.pointerId);
                } catch {
                  // 掴み続けられない環境でも、絵の上で動かせる範囲はそのまま使う。
                }
              }}
              onFocus={() => setFocusedId(entry.leaf_id)}
              onBlur={() =>
                setFocusedId((currentId) =>
                  currentId === entry.leaf_id ? null : currentId,
                )
              }
              onKeyDown={(event) => handleKeyDown(entry.leaf_id, event)}
            >
              <title>{name}</title>
            </circle>
            {labelLayout.visible && (
              <text
                className="paper-position-label"
                x={labelLayout.x}
                y={labelLayout.y}
                fill="var(--color-text)"
                stroke="var(--color-surface)"
                strokeWidth={labelLayout.strokeWidth}
                paintOrder="stroke"
                fontSize={labelLayout.fontSize}
                textAnchor={labelLayout.textAnchor}
              >
                {name}
              </text>
            )}
          </g>
        );
      })}
    </svg>
  );
}
