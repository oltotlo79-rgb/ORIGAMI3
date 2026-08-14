// 提案画面の形見本と横方向の配置。DOMに依存しない純関数として検査できる。

import { previewLayout, skeletonRows } from "./skeleton";
import type { LimbLayout } from "./skeleton";
import type { Skeleton, Vec2 } from "./types";

export const PROPOSAL_DIALOG_VIEWPORT_GUTTER_PX = 48;
export const PROPOSAL_DIALOG_MAX_WIDTH_PX = 720;
export const PROPOSAL_PREVIEW_MAX_WIDTH_PX = 200;
export const PROPOSAL_LIST_BASIS_PX = 360;
export const PROPOSAL_ROW_INDENT_STEP_PX = 16;
export const PROPOSAL_ROW_INDENT_MAX_PX = 48;

/** 全テーマのborderとpadding、および通常の縦スクロールバーを保守的に見積もる。 */
const DIALOG_HORIZONTAL_INSET_PX = 26;
const VERTICAL_SCROLLBAR_RESERVE_PX = 17;
const PROPOSAL_BODY_GAP_PX = 14;
const PREVIEW_STROKE_RATIO = 0.5;

export interface PreviewPartLayout extends LimbLayout {
  strokeWidth: number;
  connectionRadius: number;
  labelPosition: Vec2;
  labelFontSize: number;
  labelStrokeWidth: number;
}

export interface PreviewFrameLayout {
  parts: PreviewPartLayout[];
  frameRadius: number;
  bodyRadius: number;
  viewBox: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
}

/** SVGに渡す全座標と寸法を求める。座標系はy軸が上向き。 */
export function calculatePreviewFrame(skeleton: Skeleton): PreviewFrameLayout {
  const rawParts = previewLayout(skeleton);
  const reach = Math.max(
    ...rawParts.flatMap((part) => [
      Math.hypot(...part.start),
      part.radius * PREVIEW_STROKE_RATIO + Math.hypot(...part.end),
    ]),
    0.5,
  );
  const frameRadius = reach * 1.25;
  const parts = rawParts.map((part): PreviewPartLayout => {
    const dx = part.end[0] - part.start[0];
    const dy = part.end[1] - part.start[1];
    const length = Math.hypot(dx, dy) || 1;
    const labelOffset = frameRadius * 0.08;
    return {
      ...part,
      strokeWidth: Math.max(
        part.radius * PREVIEW_STROKE_RATIO,
        frameRadius * 0.01,
      ),
      connectionRadius: frameRadius * 0.022,
      labelPosition: [
        part.end[0] - (dy / length) * labelOffset,
        part.end[1] + (dx / length) * labelOffset,
      ],
      labelFontSize: frameRadius * 0.11,
      labelStrokeWidth: frameRadius * 0.025,
    };
  });

  return {
    parts,
    frameRadius,
    bodyRadius: frameRadius * 0.05,
    viewBox: {
      x: -frameRadius,
      y: -frameRadius,
      width: 2 * frameRadius,
      height: 2 * frameRadius,
    },
  };
}

export interface ViewportSize {
  width: number;
  height: number;
}

export interface HorizontalSpan {
  left: number;
  width: number;
  right: number;
}

export interface ProposalRowSpan extends HorizontalSpan {
  id: number;
  indent: number;
}

export interface ProposalScreenLayout {
  dialog: HorizontalSpan;
  preview: HorizontalSpan;
  list: HorizontalSpan;
  rows: ProposalRowSpan[];
  dialogMaxHeight: number;
  previewFrame: PreviewFrameLayout;
  /** 負なら画面内の余白、正なら画面外へ出た量。 */
  horizontalExcessPx: number;
  /** 実際に画面外へ出た量。画面内なら0。 */
  horizontalOverflowPx: number;
}

function horizontalSpan(left: number, width: number): HorizontalSpan {
  return { left, width, right: left + width };
}

/**
 * 現行のflex規則を数値化する。高さは横幅へ効くスクロールバー予約にだけ使い、
 * 縦方向はダイアログ内のスクロールへ任せる。
 */
export function calculateProposalLayout(
  skeleton: Skeleton,
  viewport: ViewportSize,
): ProposalScreenLayout {
  if (
    !Number.isFinite(viewport.width) ||
    !Number.isFinite(viewport.height) ||
    viewport.width < 0 ||
    viewport.height < 0
  ) {
    throw new Error("viewport dimensions must be finite and non-negative");
  }

  const dialogWidth = Math.min(
    PROPOSAL_DIALOG_MAX_WIDTH_PX,
    Math.max(0, viewport.width - PROPOSAL_DIALOG_VIEWPORT_GUTTER_PX),
  );
  const dialog = horizontalSpan((viewport.width - dialogWidth) / 2, dialogWidth);
  const contentLeft = dialog.left + DIALOG_HORIZONTAL_INSET_PX;
  const contentWidth = Math.max(
    0,
    dialog.width -
      2 * DIALOG_HORIZONTAL_INSET_PX -
      VERTICAL_SCROLLBAR_RESERVE_PX,
  );
  const sideBySide =
    contentWidth >=
    PROPOSAL_PREVIEW_MAX_WIDTH_PX +
      PROPOSAL_BODY_GAP_PX +
      PROPOSAL_LIST_BASIS_PX;
  const previewWidth = Math.min(PROPOSAL_PREVIEW_MAX_WIDTH_PX, contentWidth);
  const preview = horizontalSpan(contentLeft, previewWidth);
  const listLeft = sideBySide
    ? preview.right + PROPOSAL_BODY_GAP_PX
    : contentLeft;
  const listWidth = sideBySide
    ? Math.max(0, contentWidth - previewWidth - PROPOSAL_BODY_GAP_PX)
    : contentWidth;
  const list = horizontalSpan(listLeft, listWidth);
  const rows = skeletonRows(skeleton).map(({ node, depth }) => {
    const indent = Math.min(
      Math.max(depth - 1, 0) * PROPOSAL_ROW_INDENT_STEP_PX,
      PROPOSAL_ROW_INDENT_MAX_PX,
    );
    return {
      id: node.id,
      indent,
      ...horizontalSpan(list.left + indent, Math.max(0, list.width - indent)),
    };
  });
  const occupied = [dialog, preview, list, ...rows];
  const leftmost = Math.min(...occupied.map((span) => span.left));
  const rightmost = Math.max(...occupied.map((span) => span.right));
  const horizontalExcessPx = Math.max(
    -leftmost,
    rightmost - viewport.width,
  );

  return {
    dialog,
    preview,
    list,
    rows,
    dialogMaxHeight: viewport.height * 0.88,
    previewFrame: calculatePreviewFrame(skeleton),
    horizontalExcessPx,
    horizontalOverflowPx: Math.max(0, horizontalExcessPx),
  };
}
