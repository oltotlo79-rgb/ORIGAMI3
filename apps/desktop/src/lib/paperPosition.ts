// 選んだ提案候補について、紙の上で使う先端の場所を直接動かすための純関数。
// 完成形の位置とは別の一時入力として扱い、候補の小さな見本には混ぜない。

import { svgPointFromClient } from "./proposalLayout";
import type { ClientRect } from "./proposalLayout";
import type {
  CreasePattern,
  PaperPosition2d,
  PaperTipPosition,
  ProposalCandidate,
  Skeleton,
  Vec2,
} from "./types";

export const PAPER_POSITION_KEY_STEP = 0.02;
export const PAPER_POSITION_KEY_STEP_LARGE = 0.1;

export interface PaperBounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
  width: number;
  height: number;
  longSide: number;
}

export interface PaperEditorViewBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PaperPositionLabelLayout {
  x: number;
  y: number;
  fontSize: number;
  strokeWidth: number;
  textAnchor: "start" | "middle" | "end";
}

export interface PaperPositionLabelInput {
  id: number;
  point: Vec2;
  label: string;
}

export interface PaperPositionLabelPlacement extends PaperPositionLabelLayout {
  id: number;
  shifted: boolean;
  visible: boolean;
}

export interface PaperPositionLabelBounds {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** 展開図の紙全体を囲む範囲。壊れた入力でも有限な1×1を返す。 */
export function paperBounds(cp: CreasePattern): PaperBounds {
  const points = cp.vertices
    .map((vertex) => vertex.pos)
    .filter((pos) => Number.isFinite(pos[0]) && Number.isFinite(pos[1]));
  if (points.length === 0) {
    return {
      minX: 0,
      minY: 0,
      maxX: 1,
      maxY: 1,
      width: 1,
      height: 1,
      longSide: 1,
    };
  }

  const minX = Math.min(...points.map((pos) => pos[0]));
  const minY = Math.min(...points.map((pos) => pos[1]));
  const rawWidth = Math.max(...points.map((pos) => pos[0])) - minX;
  const rawHeight = Math.max(...points.map((pos) => pos[1])) - minY;
  const width = rawWidth > 0 && Number.isFinite(rawWidth) ? rawWidth : 1;
  const height = rawHeight > 0 && Number.isFinite(rawHeight) ? rawHeight : 1;
  return {
    minX,
    minY,
    maxX: minX + width,
    maxY: minY + height,
    width,
    height,
    longSide: Math.max(width, height),
  };
}

/** 端のつまみも切れないよう、紙の外へ長辺の6%だけ余白を取る。 */
export function paperEditorViewBox(bounds: PaperBounds): PaperEditorViewBox {
  const padding = bounds.longSide * 0.06;
  return {
    x: bounds.minX - padding,
    // 紙の座標は上が正、SVGは下が正なので上下を反転した範囲にする。
    y: -bounds.maxY - padding,
    width: bounds.width + 2 * padding,
    height: bounds.height + 2 * padding,
  };
}

/**
 * 端のつまみの呼び名を、紙の内側へ向けて置く。上端は下へ、左右端は中央へ向け、
 * 深い枝の長い名前はviewBox幅へ収まる分だけ文字を縮める。
 */
export function paperPositionLabelLayout(
  point: Vec2,
  viewBox: PaperEditorViewBox,
  label: string,
  desiredFontSize: number,
  handleRadius: number,
): PaperPositionLabelLayout {
  const glyphCount = Math.max(1, [...label].length);
  const safeDesiredFont =
    Number.isFinite(desiredFontSize) && desiredFontSize > 0 ? desiredFontSize : 1;
  const safeHandleRadius =
    Number.isFinite(handleRadius) && handleRadius > 0 ? handleRadius : 1;
  const desiredStroke = safeDesiredFont * 0.15;
  const availableWidth = Math.max(0, viewBox.width - desiredStroke * 2);
  const fontSize = Math.min(safeDesiredFont, availableWidth / glyphCount);
  const strokeWidth = fontSize * 0.15;
  const textWidth = fontSize * glyphCount;
  const left = viewBox.x;
  const right = viewBox.x + viewBox.width;
  const top = viewBox.y;
  const bottom = viewBox.y + viewBox.height;
  const offset = safeHandleRadius * 1.8;
  const cy = -point[1];

  let textAnchor: PaperPositionLabelLayout["textAnchor"] = "middle";
  let x = point[0];
  if (x - textWidth / 2 - strokeWidth / 2 < left) {
    textAnchor = "start";
    x = Math.min(
      Math.max(left + strokeWidth / 2, x + offset),
      right - textWidth - strokeWidth / 2,
    );
  } else if (x + textWidth / 2 + strokeWidth / 2 > right) {
    textAnchor = "end";
    x = Math.max(
      Math.min(right - strokeWidth / 2, x - offset),
      left + textWidth + strokeWidth / 2,
    );
  }

  // SVG textのyは基準線。上側は約1em、下側は約0.25emを字面として予約する。
  let y = cy - offset;
  if (y - fontSize - strokeWidth / 2 < top) {
    y = cy + offset + fontSize;
  }
  y = Math.min(
    Math.max(y, top + fontSize + strokeWidth / 2),
    bottom - fontSize * 0.25 - strokeWidth / 2,
  );

  return { x, y, fontSize, strokeWidth, textAnchor };
}

/** SVG文字の字面と白い縁取りを、実フォント差を吸収する1文字=1emで保守的に囲む。 */
export function paperPositionLabelBounds(
  layout: PaperPositionLabelLayout,
  label: string,
): PaperPositionLabelBounds {
  const width = Math.max(1, [...label].length) * layout.fontSize;
  const left =
    layout.textAnchor === "start"
      ? layout.x
      : layout.textAnchor === "end"
        ? layout.x - width
        : layout.x - width / 2;
  return {
    left: left - layout.strokeWidth / 2,
    top: layout.y - layout.fontSize - layout.strokeWidth / 2,
    right: left + width + layout.strokeWidth / 2,
    bottom: layout.y + layout.fontSize * 0.25 + layout.strokeWidth / 2,
  };
}

function labelBoundsOverlap(
  a: PaperPositionLabelBounds,
  b: PaperPositionLabelBounds,
  gap: number,
): boolean {
  return !(
    a.right + gap <= b.left ||
    b.right + gap <= a.left ||
    a.bottom + gap <= b.top ||
    b.bottom + gap <= a.top
  );
}

/**
 * 最大12本の呼び名を順にずらし、同じ場所へつまみを集めても文字を重ねない。
 * 置く余地が物理的にない名前は重ねず隠し、選択中の1件を必ず先に全名表示する。
 * 円のaria-labelと補足には全件の名前を残す。
 */
export function paperPositionLabelLayouts(
  inputs: readonly PaperPositionLabelInput[],
  viewBox: PaperEditorViewBox,
  desiredFontSize: number,
  handleRadius: number,
  priorityId: number | null = null,
): PaperPositionLabelPlacement[] {
  const placed: PaperPositionLabelBounds[] = [];
  const handleObstacleById = new Map(inputs.map(({ id, point }) => {
    const radius = handleRadius * 1.8;
    const cy = -point[1];
    return [id, {
      left: point[0] - radius,
      top: cy - radius,
      right: point[0] + radius,
      bottom: cy + radius,
    }] as const;
  }));
  const handleObstacles = [...handleObstacleById.values()];
  const top = viewBox.y;
  const bottom = viewBox.y + viewBox.height;
  const centerY = top + viewBox.height / 2;
  const capped = inputs.slice(0, 12);
  const ordered = [...capped].sort((a, b) => {
    if (a.id === priorityId) return -1;
    if (b.id === priorityId) return 1;
    return a.id - b.id;
  });
  const placementById = new Map<number, PaperPositionLabelPlacement>();

  for (const input of ordered) {
    const glyphCount = Math.max(1, [...input.label].length);
    // 9本以上は左右2列を使える幅へそろえる。560px表示で深さ4の名前も約14px以上を保つ。
    const columnFontSize =
      inputs.length > 8
        ? (viewBox.width / 2 - desiredFontSize * 0.2) / glyphCount
        : desiredFontSize;
    const base = paperPositionLabelLayout(
      input.point,
      viewBox,
      input.label,
      Math.min(desiredFontSize, columnFontSize),
      handleRadius,
    );
    const minimumY = top + base.fontSize + base.strokeWidth / 2;
    const maximumY = bottom - base.fontSize * 0.25 - base.strokeWidth / 2;
    // 字面は上1em・下0.25em、縁取りと間隔も要るため、1.75emずつ送る。
    const step = Math.max(
      base.fontSize * 1.75 + base.strokeWidth,
      handleRadius * 1.1,
    );
    const towardCenter = base.y <= centerY ? 1 : -1;
    let chosen: PaperPositionLabelLayout | null = null;
    const gap = base.fontSize * 0.12;
    const boundaryGap = gap + Math.max(1e-12, base.fontSize * 1e-6);
    const verticalCandidates: number[] = [];

    // 64段は最小文字でもviewBoxの高さを十分に走査し、同一点12件の余裕を持つ。
    for (let attempt = 0; attempt < 64; attempt += 1) {
      const distance = attempt === 0 ? 0 : Math.ceil(attempt / 2) * step;
      const direction = attempt === 0 || attempt % 2 === 1 ? towardCenter : -towardCenter;
      verticalCandidates.push(
        Math.min(maximumY, Math.max(minimumY, base.y + direction * distance)),
      );
    }
    // 一定刻みだけでは、つまみ同士の狭い空き帯を飛び越えることがある。
    // そこで全つまみと配置済み文字の上下端から、字面とgapがちょうど離れる基準線も作る。
    // 560px表示の通常4×3では刻み34.2pxに対して空き帯20.08pxだが、この境界候補なら12/12を置ける。
    for (const obstacle of [...handleObstacles, ...placed]) {
      verticalCandidates.push(
        Math.min(
          maximumY,
          Math.max(
            minimumY,
            obstacle.top - boundaryGap - base.fontSize * 0.25 - base.strokeWidth / 2,
          ),
        ),
        Math.min(
          maximumY,
          Math.max(
            minimumY,
            obstacle.bottom + boundaryGap + base.fontSize + base.strokeWidth / 2,
          ),
        ),
      );
    }

    // 境界候補を足しても、元の呼び名の場所に近いものから試す。
    // 同じ候補は端でのclampにより繰り返されるため、先に除いて引き出し線を短く保つ。
    const orderedVerticalCandidates = [...new Set(verticalCandidates)].sort(
      (left, right) => Math.abs(left - base.y) - Math.abs(right - base.y),
    );

    for (const y of orderedVerticalCandidates) {
      if (chosen !== null) break;
      const horizontalCandidates: PaperPositionLabelLayout[] = [
        { ...base, y },
        {
          ...base,
          x: viewBox.x + base.strokeWidth / 2,
          y,
          textAnchor: "start",
        },
        {
          ...base,
          x: viewBox.x + viewBox.width - base.strokeWidth / 2,
          y,
          textAnchor: "end",
        },
      ];
      for (const candidate of horizontalCandidates) {
        const candidateBounds = paperPositionLabelBounds(candidate, input.label);
        const blocked = [...handleObstacles, ...placed].some((obstacle) =>
          labelBoundsOverlap(candidateBounds, obstacle, gap),
        );
        if (!blocked) {
          chosen = candidate;
          placed.push(candidateBounds);
          break;
        }
      }
    }

    // 選択中だけは必ず先に表示する。ほかは置き場がなければ重ねず非表示にする。
    if (chosen === null && input.id === priorityId) {
      chosen = base;
      placed.push(paperPositionLabelBounds(base, input.label));
    }
    const layout = chosen ?? base;
    placementById.set(input.id, {
      id: input.id,
      ...layout,
      visible: chosen !== null,
      shifted:
        Math.abs(layout.x - base.x) > Number.EPSILON ||
        Math.abs(layout.y - base.y) > Number.EPSILON ||
        layout.textAnchor !== base.textAnchor,
    });
  }
  return capped.map((input) => placementById.get(input.id)!);
}

function finiteClamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return 0;
  if (value === Number.POSITIVE_INFINITY) return max;
  if (value === Number.NEGATIVE_INFINITY) return min;
  return Math.min(max, Math.max(min, value));
}

/** 紙の短辺では、その端が1.0未満になることを含めて紙の内側へ収める。 */
export function clampPaperPosition(
  position: PaperPosition2d,
  bounds: PaperBounds,
): PaperPosition2d {
  const xLimit = bounds.width / bounds.longSide;
  const yLimit = bounds.height / bounds.longSide;
  return {
    x: finiteClamp(position.x, -xLimit, xLimit),
    y: finiteClamp(position.y, -yLimit, yLimit),
  };
}

/** 紙の実座標を、紙の中心・長辺基準の一時入力へ直す。 */
export function paperPointToPosition(
  point: Vec2,
  bounds: PaperBounds,
): PaperPosition2d {
  const centerX = (bounds.minX + bounds.maxX) * 0.5;
  const centerY = (bounds.minY + bounds.maxY) * 0.5;
  return clampPaperPosition(
    {
      x: (2 * (point[0] - centerX)) / bounds.longSide,
      y: (2 * (point[1] - centerY)) / bounds.longSide,
    },
    bounds,
  );
}

/** 紙の中心・長辺基準の一時入力を、紙の実座標へ戻す。 */
export function paperPositionToPoint(
  position: PaperPosition2d,
  bounds: PaperBounds,
): Vec2 {
  const fit = clampPaperPosition(position, bounds);
  return [
    (bounds.minX + bounds.maxX) * 0.5 + (fit.x * bounds.longSide) / 2,
    (bounds.minY + bounds.maxY) * 0.5 + (fit.y * bounds.longSide) / 2,
  ];
}

/** 画面上でつまんだ点を、紙の上の場所へ直す。紙の外なら最寄りの縁で止める。 */
export function clientPointToPaperPosition(
  client: Vec2,
  rect: ClientRect,
  bounds: PaperBounds,
): PaperPosition2d {
  const [x, svgY] = svgPointFromClient(
    client,
    rect,
    paperEditorViewBox(bounds),
  );
  return paperPointToPosition([x, -svgY], bounds);
}

/** 選んだ候補の各先端を、紙の上の編集開始位置へ変換する。 */
export function paperPositionsFromCandidate(
  candidate: ProposalCandidate,
): PaperTipPosition[] {
  const bounds = paperBounds(candidate.cp);
  const byLeaf = new Map<number, PaperTipPosition>();
  for (const site of candidate.sites ?? []) {
    const center = site.circle.center;
    if (!Number.isFinite(center[0]) || !Number.isFinite(center[1])) continue;
    byLeaf.set(site.circle.leaf_id, {
      leaf_id: site.circle.leaf_id,
      position: paperPointToPosition(center, bounds),
    });
  }
  return [...byLeaf.values()].sort((a, b) => a.leaf_id - b.leaf_id);
}

export function paperPositionChanged(
  current: PaperPosition2d | undefined,
  original: PaperPosition2d | undefined,
  tolerance = 1e-9,
): boolean {
  if (!current || !original) return current !== original;
  return (
    Math.abs(current.x - original.x) > tolerance ||
    Math.abs(current.y - original.y) > tolerance
  );
}

/**
 * 紙の上の入力だけを使う再計算要求を1件組み立てる。
 *
 * 元の`proposalSkeleton`は完成形の位置を保ったまま残し、送信用の複製だけを作る。
 * 2種類を同時に持つ場合の優先規則は作業13の統合点で決めるため、この関数は
 * 紙の上だけを試す経路に限定する。
 */
export function skeletonForPaperPositions(
  skeleton: Skeleton,
  positions: readonly PaperTipPosition[],
): Skeleton {
  const byLeaf = new Map(positions.map((entry) => [entry.leaf_id, entry.position]));
  const parentIds = new Set(
    skeleton.nodes.flatMap((node) =>
      node.parent === null ? [] : [node.parent],
    ),
  );
  return {
    nodes: skeleton.nodes.map((node) => {
      const isLeaf = node.parent !== null && !parentIds.has(node.id);
      if (!isLeaf) return node;
      const copy = { ...node };
      delete copy.tip_pos_2d;
      const position = byLeaf.get(node.id);
      if (position) {
        copy.tip_pos_2d = {
          x: finiteClamp(position.x, -1, 1),
          y: finiteClamp(position.y, -1, 1),
        };
      }
      return copy;
    }),
  };
}
