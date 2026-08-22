// 提案画面の形見本と横方向の配置。DOMに依存しない純関数として検査できる。

import { clampTipPos, previewLayout, skeletonRows } from "./skeleton";
import type { LimbLayout } from "./skeleton";
import type { Skeleton, TipPos2d, Vec2 } from "./types";

export const PROPOSAL_DIALOG_VIEWPORT_GUTTER_PX = 48;
export const PROPOSAL_DIALOG_MAX_WIDTH_PX = 720;
/** 紙全体と操作を最小画面でも並べる、紙位置画面だけの最大幅。 */
export const PROPOSAL_PAPER_DIALOG_MAX_WIDTH_PX = 960;
export const PROPOSAL_PREVIEW_MAX_WIDTH_PX = 200;
/** 小候補(150px)と混同しない、紙の上の場所を動かす大きな編集面。 */
export const PAPER_POSITION_EDITOR_MAX_WIDTH_PX = 560;
/** 5テーマで最も広い外側余白(18px×2)を残す。 */
export const PAPER_POSITION_DIALOG_VIEWPORT_GUTTER_PX = 36;
/** 紙と説明・操作の列の間隔。 */
export const PAPER_POSITION_LAYOUT_GAP_PX = 16;
/** 12本の通知はここまで表示し、残りは通知欄自身を送る。App.cssと同じ。 */
export const PAPER_POSITION_NOTICE_MAX_HEIGHT_PX = 112;
export const PROPOSAL_LIST_BASIS_PX = 360;
export const PROPOSAL_ROW_INDENT_STEP_PX = 16;
export const PROPOSAL_ROW_INDENT_MAX_PX = 48;

/** 紙の長辺を最大560pxへそろえる。縦長の紙を横560pxへ拡大してつまみを巨大化しない。 */
export function paperPositionEditorWidthPx(viewBox: {
  width: number;
  height: number;
}): number {
  if (
    !Number.isFinite(viewBox.width) ||
    !Number.isFinite(viewBox.height) ||
    viewBox.width <= 0 ||
    viewBox.height <= 0
  ) {
    return PAPER_POSITION_EDITOR_MAX_WIDTH_PX;
  }
  return (
    PAPER_POSITION_EDITOR_MAX_WIDTH_PX *
    (viewBox.width / Math.max(viewBox.width, viewBox.height))
  );
}

/** 全テーマのborderとpadding、および通常の縦スクロールバーを保守的に見積もる。 */
const DIALOG_HORIZONTAL_INSET_PX = 26;
const VERTICAL_SCROLLBAR_RESERVE_PX = 17;
const PROPOSAL_BODY_GAP_PX = 14;
const PREVIEW_STROKE_RATIO = 0.5;

/**
 * 見本の枠は、形が届いている一番遠いところ(`positionRadius`)の何倍かで決める。
 * 場所指定の `1.0` はちょうどその「一番遠いところ」にあたる。
 *
 * この倍率は1.25以上でなければならない。先端の線の太さは自動配置での届く距離の
 * 高々1/2(太さ係数の上限が2.0で、半径=長さ×太さのため)で、枠の角へ寄せた先端でも
 * 線の縁が枠から出ないためには `1/倍率 + 1/4 <= 1` が要る。
 */
export const PREVIEW_FRAME_MARGIN = 1.25;
/**
 * つまみの大きさと輪郭。枠の大きさに対する割合。
 * 出っぱりの線は太いところで枠の 0.4倍(太さ2.0のとき)になるので、
 * つまみが線の中に埋もれない大きさにする。
 */
const TIP_HANDLE_RATIO = 0.09;
const TIP_HANDLE_STROKE_RATIO = 0.02;
const PREVIEW_LABEL_FONT_RATIO = 0.11;
const PREVIEW_LABEL_STROKE_RATIO = 0.025;
const PREVIEW_LABEL_GAP_RATIO = 0.012;
/** 場所を決めた先端は、色だけでなく大きさでも分かるようにする。 */
export const TIP_HANDLE_DECIDED_SCALE = 1.25;
/** 矢印キー1回で動く量(Shiftを押していれば大きいほう) */
export const TIP_KEY_STEP = 0.05;
export const TIP_KEY_STEP_LARGE = 0.2;

export interface PreviewPartLayout extends LimbLayout {
  strokeWidth: number;
  connectionRadius: number;
  labelPosition: Vec2;
  labelFontSize: number;
  labelStrokeWidth: number;
  /** 呼び名の文字と白い縁取りを含む、見本座標上の範囲。 */
  labelBounds: PreviewLabelBounds;
  /** 先端をつまむ丸の半径と輪郭の太さ */
  handleRadius: number;
  handleStrokeWidth: number;
}

export interface PreviewLabelBounds {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface PreviewFrameLayout {
  parts: PreviewPartLayout[];
  frameRadius: number;
  /**
   * 場所指定の `1.0` が対応する見本上の長さ。
   * 場所を決めても変わらない(自動配置だけから決まる)ので、
   * つまんで動かしている最中に倍率がずれない。
   */
  positionRadius: number;
  bodyRadius: number;
  viewBox: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
}

/** 場所の指定(-1.0〜1.0)を見本の座標へ直す。y軸は上向き。 */
export function tipPosToPreviewPoint(
  pos: TipPos2d,
  positionRadius: number,
): Vec2 {
  return [pos.x * positionRadius, pos.y * positionRadius];
}

/** 見本の座標(y軸は上向き)を場所の指定へ直す。範囲外は範囲内へ収める。 */
export function previewPointToTipPos(
  point: Vec2,
  positionRadius: number,
): TipPos2d {
  if (!(positionRadius > 0)) return { x: 0, y: 0 };
  return clampTipPos({
    x: point[0] / positionRadius,
    y: point[1] / positionRadius,
  });
}

export interface ClientRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * 画面上の位置を見本の座標へ直す。SVGは既定の`xMidYMid meet`で、
 * 余った側に等しい余白ができるので、その分をここで戻す。y軸はSVGのまま下向き。
 */
export function svgPointFromClient(
  client: Vec2,
  rect: ClientRect,
  viewBox: PreviewFrameLayout["viewBox"],
): Vec2 {
  const scale = Math.min(
    rect.width / viewBox.width,
    rect.height / viewBox.height,
  );
  if (!Number.isFinite(scale) || scale <= 0) return [viewBox.x, viewBox.y];
  const left = rect.left + (rect.width - viewBox.width * scale) / 2;
  const top = rect.top + (rect.height - viewBox.height * scale) / 2;
  return [
    viewBox.x + (client[0] - left) / scale,
    viewBox.y + (client[1] - top) / scale,
  ];
}

/** 画面上の位置を、そのまま先端の場所の指定へ直す(範囲外は範囲内へ収める)。 */
export function clientPointToTipPos(
  client: Vec2,
  rect: ClientRect,
  frame: PreviewFrameLayout,
): TipPos2d {
  const [x, y] = svgPointFromClient(client, rect, frame.viewBox);
  // 見本はy軸が上向き、SVGは下向きなので符号を戻す
  return previewPointToTipPos([x, -y], frame.positionRadius);
}

interface PreviewLabelSize {
  width: number;
  height: number;
}

/**
 * SVGの呼び名は日本語が中心なので、全角は1em、英数字は0.7emとして見積もる。
 * 実機で使うフォントの実測幅より広めに取り、白い縁取りも両側へ足す。
 */
function previewLabelSize(
  label: string,
  fontSize: number,
  strokeWidth: number,
): PreviewLabelSize {
  const em = Array.from(label).reduce(
    (sum, character) => sum + (/^[\x20-\x7e]$/u.test(character) ? 0.7 : 1),
    0,
  );
  return {
    width: Math.max(fontSize, em * fontSize) + strokeWidth * 2,
    height: fontSize * 1.2 + strokeWidth * 2,
  };
}

function labelBoundsAt(center: Vec2, size: PreviewLabelSize): PreviewLabelBounds {
  return {
    left: center[0] - size.width / 2,
    top: center[1] + size.height / 2,
    right: center[0] + size.width / 2,
    bottom: center[1] - size.height / 2,
  };
}

function labelIsInsideFrame(bounds: PreviewLabelBounds, radius: number): boolean {
  return (
    bounds.left >= -radius &&
    bounds.right <= radius &&
    bounds.bottom >= -radius &&
    bounds.top <= radius
  );
}

function labelBoundsIntersect(
  first: PreviewLabelBounds,
  second: PreviewLabelBounds,
  gap: number,
): boolean {
  return !(
    first.right + gap <= second.left ||
    second.right + gap <= first.left ||
    first.top + gap <= second.bottom ||
    second.top + gap <= first.bottom
  );
}

function labelIntersectsCircle(
  bounds: PreviewLabelBounds,
  center: Vec2,
  radius: number,
  gap: number,
): boolean {
  const nearestX = Math.max(bounds.left, Math.min(center[0], bounds.right));
  const nearestY = Math.max(bounds.bottom, Math.min(center[1], bounds.top));
  return Math.hypot(nearestX - center[0], nearestY - center[1]) < radius + gap;
}

function unitVector(vector: Vec2): Vec2 | null {
  const length = Math.hypot(vector[0], vector[1]);
  return length > 1e-9 ? [vector[0] / length, vector[1] / length] : null;
}

function uniqueDirections(vectors: readonly Vec2[]): Vec2[] {
  const found: Vec2[] = [];
  for (const vector of vectors) {
    const direction = unitVector(vector);
    if (
      direction &&
      !found.some(
        (current) =>
          Math.abs(current[0] - direction[0]) < 1e-6 &&
          Math.abs(current[1] - direction[1]) < 1e-6,
      )
    ) {
      found.push(direction);
    }
  }
  return found;
}

interface PreviewLabelPlacement {
  position: Vec2;
  bounds: PreviewLabelBounds;
}

function placePreviewLabels(
  parts: readonly PreviewPartLayout[],
  frameRadius: number,
): Map<number, PreviewLabelPlacement> {
  const gap = frameRadius * PREVIEW_LABEL_GAP_RATIO;
  const handleObstacles = parts
    .filter((part) => part.isTip)
    .map((part) => ({
      center: part.end,
      radius: part.handleRadius + part.handleStrokeWidth / 2,
    }));
  const placed = new Map<number, PreviewLabelPlacement>();
  const occupied: PreviewLabelBounds[] = [];

  // 長い呼び名から置くと、残った小さな隙間を短い呼び名に使える。
  const ordered = [...parts].sort((first, second) => {
    const widthDifference =
      previewLabelSize(
        second.label,
        second.labelFontSize,
        second.labelStrokeWidth,
      ).width -
      previewLabelSize(
        first.label,
        first.labelFontSize,
        first.labelStrokeWidth,
      ).width;
    return Math.abs(widthDifference) > 1e-9
      ? widthDifference
      : first.id - second.id;
  });

  for (const part of ordered) {
    const size = previewLabelSize(
      part.label,
      part.labelFontSize,
      part.labelStrokeWidth,
    );
    const halfDiagonal = Math.hypot(size.width / 2, size.height / 2);
    const ownRadius = part.isTip
      ? part.handleRadius + part.handleStrokeWidth / 2
      : part.connectionRadius;
    const lineDirection = unitVector([
      part.end[0] - part.start[0],
      part.end[1] - part.start[1],
    ]) ?? [1, 0];
    const radialDirection = unitVector(part.end) ?? lineDirection;
    const directions = uniqueDirections([
      [-lineDirection[1], lineDirection[0]],
      [lineDirection[1], -lineDirection[0]],
      radialDirection,
      [-radialDirection[0], -radialDirection[1]],
      [1, 0],
      [-1, 0],
      [0, 1],
      [0, -1],
      [1, 1],
      [-1, 1],
      [1, -1],
      [-1, -1],
    ]);
    const candidates: Vec2[] = [];
    const firstDistance = ownRadius + halfDiagonal + gap;
    for (let ring = 0; ring < 6; ring += 1) {
      const distance = firstDistance + ring * (size.height + gap);
      for (const direction of directions) {
        candidates.push([
          part.end[0] + direction[0] * distance,
          part.end[1] + direction[1] * distance,
        ]);
      }
    }

    // 込み入った12本立てでは、近くの候補を使い切った後に枠全体から探す。
    // 並び順は先端に近いものからなので、離れすぎる場所は最後の手段になる。
    const gridStep = Math.max(size.height + gap, frameRadius * 0.08);
    const grid: Vec2[] = [];
    for (
      let y = -frameRadius + size.height / 2;
      y <= frameRadius - size.height / 2 + 1e-9;
      y += gridStep
    ) {
      for (
        let x = -frameRadius + size.width / 2;
        x <= frameRadius - size.width / 2 + 1e-9;
        x += gridStep
      ) {
        grid.push([x, y]);
      }
    }
    grid.sort(
      (first, second) =>
        Math.hypot(first[0] - part.end[0], first[1] - part.end[1]) -
        Math.hypot(second[0] - part.end[0], second[1] - part.end[1]),
    );
    candidates.push(...grid);

    const candidate = candidates.find((position) => {
      const bounds = labelBoundsAt(position, size);
      return (
        labelIsInsideFrame(bounds, frameRadius) &&
        !handleObstacles.some((handle) =>
          labelIntersectsCircle(bounds, handle.center, handle.radius, gap),
        ) &&
        !occupied.some((boundsAlreadyUsed) =>
          labelBoundsIntersect(bounds, boundsAlreadyUsed, gap),
        )
      );
    });

    // 1〜12本と最長の標準名では必ず上で見つかる。壊れた入力でもNaNを描かないため、
    // 最後だけ枠内へ収めた従来位置を使う。
    const fallback: Vec2 = [
      Math.max(
        -frameRadius + size.width / 2,
        Math.min(frameRadius - size.width / 2, part.labelPosition[0]),
      ),
      Math.max(
        -frameRadius + size.height / 2,
        Math.min(frameRadius - size.height / 2, part.labelPosition[1]),
      ),
    ];
    const position = candidate ?? fallback;
    const bounds = labelBoundsAt(position, size);
    placed.set(part.id, { position, bounds });
    occupied.push(bounds);
  }

  return placed;
}

/** SVGに渡す全座標と寸法を求める。座標系はy軸が上向き。 */
export function calculatePreviewFrame(skeleton: Skeleton): PreviewFrameLayout {
  const rawParts = previewLayout(skeleton);
  // 届く距離は「場所を決めていないときの並べ方」だけから決める。
  // 決めた場所を入れて測ると、動かすたびに倍率が変わって手元がずれる。
  const reach = Math.max(
    ...rawParts.flatMap((part) => [
      Math.hypot(...part.start),
      part.radius * PREVIEW_STROKE_RATIO + Math.hypot(...part.end),
    ]),
    0.5,
  );
  const frameRadius = reach * PREVIEW_FRAME_MARGIN;
  const positionRadius = reach;
  const initialParts = rawParts.map((part): PreviewPartLayout => {
    const end = part.tipPos
      ? tipPosToPreviewPoint(part.tipPos, positionRadius)
      : part.end;
    const dx = end[0] - part.start[0];
    const dy = end[1] - part.start[1];
    const length = Math.hypot(dx, dy) || 1;
    const handleRadius = frameRadius * TIP_HANDLE_RATIO;
    // 呼び名がつまみと重ならないよう、つまみの分だけ外へずらす
    const labelOffset =
      frameRadius * 0.08 + (part.isTip ? handleRadius * TIP_HANDLE_DECIDED_SCALE : 0);
    return {
      ...part,
      end,
      strokeWidth: Math.max(
        part.radius * PREVIEW_STROKE_RATIO,
        frameRadius * 0.01,
      ),
      connectionRadius: frameRadius * 0.022,
      labelPosition: [
        end[0] - (dy / length) * labelOffset,
        end[1] + (dx / length) * labelOffset,
      ],
      labelFontSize: frameRadius * PREVIEW_LABEL_FONT_RATIO,
      labelStrokeWidth: frameRadius * PREVIEW_LABEL_STROKE_RATIO,
      labelBounds: {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
      },
      handleRadius: part.tipPos
        ? handleRadius * TIP_HANDLE_DECIDED_SCALE
        : handleRadius,
      handleStrokeWidth: frameRadius * TIP_HANDLE_STROKE_RATIO,
    };
  });
  const labelPlacements = placePreviewLabels(initialParts, frameRadius);
  const parts = initialParts.map((part) => {
    const placement = labelPlacements.get(part.id);
    return placement
      ? {
          ...part,
          labelPosition: placement.position,
          labelBounds: placement.bounds,
        }
      : part;
  });

  return {
    parts,
    frameRadius,
    positionRadius,
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

export interface VerticalSpan {
  top: number;
  height: number;
  bottom: number;
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

export interface PaperPositionEditorScreenLayout {
  dialog: HorizontalSpan;
  dialogVertical: VerticalSpan;
  editor: HorizontalSpan;
  editorVertical: VerticalSpan;
  controls: HorizontalSpan;
  dialogMaxHeight: number;
  noticeMaximumHeight: number;
  editorViewportHeight: number;
  squareEditorContentHeight: number;
  squareEditorScrollRange: number;
  /** 負なら画面内の余白、正なら画面外へ出た量。 */
  horizontalExcessPx: number;
  /** 実際に画面外へ出た量。画面内なら0。 */
  horizontalOverflowPx: number;
  /** 負なら画面内の余白、正なら画面外へ出た量。 */
  verticalExcessPx: number;
  /** ダイアログ外枠が実際に画面外へ出た量。内部の紙は専用領域で送る。 */
  verticalOverflowPx: number;
  /** 固定内容と最小操作域がダイアログ高を超える量。 */
  contentVerticalOverflowPx: number;
}

function horizontalSpan(left: number, width: number): HorizontalSpan {
  return { left, width, right: left + width };
}

function verticalSpan(top: number, height: number): VerticalSpan {
  return { top, height, bottom: top + height };
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

/** 紙位置の別画面を、同じ提案ダイアログ内で横にはみ出さず配置する。 */
export function calculatePaperPositionEditorLayout(
  viewport: ViewportSize,
): PaperPositionEditorScreenLayout {
  if (
    !Number.isFinite(viewport.width) ||
    !Number.isFinite(viewport.height) ||
    viewport.width < 0 ||
    viewport.height < 0
  ) {
    throw new Error("viewport dimensions must be finite and non-negative");
  }

  const dialogWidth = Math.min(
    PROPOSAL_PAPER_DIALOG_MAX_WIDTH_PX,
    Math.max(0, viewport.width - PROPOSAL_DIALOG_VIEWPORT_GUTTER_PX),
  );
  const dialog = horizontalSpan((viewport.width - dialogWidth) / 2, dialogWidth);
  // 紙画面はテーマに依存しない16px paddingと、最大2pxの枠を使う。
  const paperDialogContentInset = 18;
  const contentLeft = dialog.left + paperDialogContentInset;
  const contentWidth = Math.max(
    0,
    dialog.width - 2 * paperDialogContentInset,
  );
  const editorWidth = Math.min(
    PAPER_POSITION_EDITOR_MAX_WIDTH_PX,
    Math.max(0, contentWidth - PAPER_POSITION_LAYOUT_GAP_PX),
  );
  const editor = horizontalSpan(contentLeft, editorWidth);
  const controls = horizontalSpan(
    editor.right + PAPER_POSITION_LAYOUT_GAP_PX,
    Math.max(
      0,
      contentLeft + contentWidth - editor.right - PAPER_POSITION_LAYOUT_GAP_PX,
    ),
  );
  const occupied = [dialog, editor, controls];
  const leftmost = Math.min(...occupied.map((span) => span.left));
  const rightmost = Math.max(...occupied.map((span) => span.right));
  const horizontalExcessPx = Math.max(
    -leftmost,
    rightmost - viewport.width,
  );
  const dialogMaxHeight = Math.max(
    0,
    viewport.height - PAPER_POSITION_DIALOG_VIEWPORT_GUTTER_PX,
  );
  const dialogVertical = verticalSpan(
    (viewport.height - dialogMaxHeight) / 2,
    dialogMaxHeight,
  );
  const paperDialogContentHeight = Math.max(
    0,
    dialogMaxHeight - 2 * paperDialogContentInset,
  );
  const editorViewportHeight = Math.min(
    PAPER_POSITION_EDITOR_MAX_WIDTH_PX,
    paperDialogContentHeight,
  );
  const editorVertical = verticalSpan(
    dialogVertical.top +
      paperDialogContentInset +
      (paperDialogContentHeight - editorViewportHeight) / 2,
    editorViewportHeight,
  );
  // 紙全体を左列へ収める。紙自身のscrollは0で、つまみ直径は約28pxのまま。
  const squareEditorContentHeight = editorWidth;
  const squareEditorScrollRange = Math.max(
    0,
    squareEditorContentHeight - editorViewportHeight,
  );
  const verticalExcessPx = Math.max(
    -dialogVertical.top,
    dialogVertical.bottom - viewport.height,
  );
  const contentVerticalOverflowPx = Math.max(
    0,
    PAPER_POSITION_EDITOR_MAX_WIDTH_PX - paperDialogContentHeight,
  );

  return {
    dialog,
    dialogVertical,
    editor,
    editorVertical,
    controls,
    dialogMaxHeight,
    noticeMaximumHeight: PAPER_POSITION_NOTICE_MAX_HEIGHT_PX,
    editorViewportHeight,
    squareEditorContentHeight,
    squareEditorScrollRange,
    horizontalExcessPx,
    horizontalOverflowPx: Math.max(0, horizontalExcessPx),
    verticalExcessPx,
    verticalOverflowPx: Math.max(
      0,
      verticalExcessPx,
      contentVerticalOverflowPx,
    ),
    contentVerticalOverflowPx,
  };
}
