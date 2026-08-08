// 展開図のCanvas 2D描画。線幅はここの定数に、紙と線の色は lib/cpColors に集約する。
// 座標系: 正規化座標(長辺=1.0、y軸上向き)→ 画面座標(px、y軸下向き)。

import type { Document, EdgeKind, Vec2 } from "../../lib/types";
import type { Selection } from "../../store/appStore";
import {
  EDGE_COLORS,
  cssRgb,
  gridColor,
  haloColor,
  paperFill,
  type Rgb,
} from "../../lib/cpColors";
import { paperExtent, type SnapResult } from "./snap";

// 線種ごとの色(山=赤・谷=青の慣例)は lib/cpColors に集約した。
// 紙の塗りと同時に決めないと、赤い紙に赤い山折り線を描いて見えなくなるため。
export { EDGE_COLORS };

export const COLORS = {
  /** 紙の外側の余白。App.css の --color-canvas-2d と同じ色にして境目を作らない */
  background: "#ddd8d0",
  paper: "#ffffff",
  paperShadow: "rgba(0, 0, 0, 0.25)",
  selection: "#ff9500",
  /** 複数スライダーのうち、指している折り目を選択全体から見分ける色 */
  hingeHover: "#7040c9",
  snapMarker: "#2aa02a",
  /** 平らに畳めない点(CPE-009)。操作は止めず色で知らせるだけ */
  violation: "#ff8c00",
  /** 巻き込みのために追加される折り目。確定前なので橙色の太い破線で示す */
  foldSuggestion: "#d97706",
  hintBackground: "rgba(28, 26, 22, 0.78)",
  hintText: "#ffffff",
  /** 延長・二等分方向へ吸着中であることを示す薄いガイド線。 */
  directionGuide: "rgba(38, 97, 74, 0.48)",
  /** 左右対称に描くときの対称軸(CPE-010)。薄く出して邪魔をしない */
  mirrorAxis: "rgba(59, 111, 201, 0.45)",
  marqueeFill: "rgba(59, 111, 201, 0.12)",
  marqueeStroke: "#3b6fc9",
  /** 拡大中に、紙のどの部分を見ているか示す細い位置バー */
  positionBarTrack: "rgba(28, 26, 22, 0.14)",
  positionBarThumb: "rgba(47, 107, 91, 0.58)",
} as const;

/** 線幅(px) */
export const LINE_WIDTHS = {
  border: 2,
  crease: 1.5,
  aux: 1,
  grid: 1,
  selected: 6,
  hovered: 10,
  preview: 1.5,
} as const;

/** 線の下に敷く縁取りの太さ(線幅にこれだけ足す) */
export const HALO_EXTRA_WIDTH = 2;

/** 補助線・プレビュー線の破線パターン(px) */
export const DASH_AUX = [5, 4] as const;
export const DASH_PREVIEW = [6, 4] as const;
/** スナップ候補マーカーの半径(px) */
export const SNAP_MARKER_RADIUS = 6;
/** 選択頂点マーカーの半径(px) */
export const VERTEX_MARKER_RADIUS = 5;

/** 表示変換: scale=1正規化単位あたりのpx、offset=原点の画面位置(px) */
export interface ViewTransform {
  scale: number;
  offsetX: number;
  offsetY: number;
}

export function worldToScreen(view: ViewTransform, p: Vec2): Vec2 {
  return [view.offsetX + p[0] * view.scale, view.offsetY - p[1] * view.scale];
}

export function screenToWorld(view: ViewTransform, p: Vec2): Vec2 {
  return [(p[0] - view.offsetX) / view.scale, (view.offsetY - p[1]) / view.scale];
}

/** 紙全体が9割の余白率で収まる表示変換(全体表示) */
export function fitView(doc: Document, widthPx: number, heightPx: number): ViewTransform {
  const [w, h] = paperExtent(doc);
  const scale = Math.min(widthPx / w, heightPx / h) * 0.9;
  return {
    scale,
    offsetX: (widthPx - w * scale) / 2,
    offsetY: heightPx - (heightPx - h * scale) / 2,
  };
}

/** 位置バーの1軸分。値はすべてCanvas上のCSSピクセル。 */
export interface AxisPositionBar {
  trackStart: number;
  trackLength: number;
  thumbStart: number;
  thumbLength: number;
}

/** 下端の横位置バーと右端の縦位置バー。 */
export interface ViewportPositionBars {
  horizontal: AxisPositionBar;
  vertical: AxisPositionBar;
}

/** 位置バーをCanvas端から離す量・太さ・最小のつまみ長(px)。 */
const POSITION_BAR_MARGIN = 6;
const POSITION_BAR_THICKNESS = 3;
const POSITION_BAR_MIN_THUMB = 20;

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * 紙の画面上の開始位置・長さから、位置バー1軸のつまみを求める純関数。
 * 紙が表示区画より大きいときは表示割合を長さ、スクロール進捗を位置にする。
 * 紙がこの軸に収まるときは、全範囲が見えていることをトラック全長で表す。
 */
export function deriveAxisPositionBar(
  paperStart: number,
  paperLength: number,
  viewportLength: number,
  trackStart: number,
  trackLength: number,
): AxisPositionBar {
  const safeTrackLength = Math.max(0, trackLength);
  if (safeTrackLength === 0) {
    return { trackStart, trackLength: 0, thumbStart: trackStart, thumbLength: 0 };
  }
  const visibleRatio =
    paperLength > 0 ? clamp(viewportLength / paperLength, 0, 1) : 1;
  const thumbLength =
    visibleRatio >= 1
      ? safeTrackLength
      : clamp(
          safeTrackLength * visibleRatio,
          Math.min(POSITION_BAR_MIN_THUMB, safeTrackLength),
          safeTrackLength,
        );
  const scrollable = Math.max(0, paperLength - viewportLength);
  const progress = scrollable === 0 ? 0 : clamp(-paperStart / scrollable, 0, 1);
  const thumbStart = trackStart + (safeTrackLength - thumbLength) * progress;
  return { trackStart, trackLength: safeTrackLength, thumbStart, thumbLength };
}

/**
 * 拡大中の紙と表示区画から、右端・下端の位置バーを求める純関数。
 * fitViewの倍率を1.0とし、それ以下ではバーを出さない。
 */
export function deriveViewportPositionBars(
  doc: Document,
  view: ViewTransform,
  widthPx: number,
  heightPx: number,
): ViewportPositionBars | null {
  if (widthPx <= 0 || heightPx <= 0) return null;
  const fitScale = fitView(doc, widthPx, heightPx).scale;
  if (!Number.isFinite(view.scale) || view.scale <= fitScale) return null;

  const [paperWidth, paperHeight] = paperExtent(doc);
  const paperWidthPx = paperWidth * view.scale;
  const paperHeightPx = paperHeight * view.scale;
  if (paperWidthPx <= 0 || paperHeightPx <= 0) return null;

  // 右下で2本が重ならないよう、互いの太さと余白の分だけトラックを短くする。
  const horizontalTrackLength = Math.max(
    0,
    widthPx - POSITION_BAR_MARGIN * 3 - POSITION_BAR_THICKNESS,
  );
  const verticalTrackLength = Math.max(
    0,
    heightPx - POSITION_BAR_MARGIN * 3 - POSITION_BAR_THICKNESS,
  );
  // 紙の原点は左下なので、縦軸だけ画面上端の位置へ直す。
  const paperTop = view.offsetY - paperHeightPx;
  return {
    horizontal: deriveAxisPositionBar(
      view.offsetX,
      paperWidthPx,
      widthPx,
      POSITION_BAR_MARGIN,
      horizontalTrackLength,
    ),
    vertical: deriveAxisPositionBar(
      paperTop,
      paperHeightPx,
      heightPx,
      POSITION_BAR_MARGIN,
      verticalTrackLength,
    ),
  };
}

/** 描画に必要な一時状態(ホバー・プレビュー等、ストア外の表示専用状態) */
export interface RenderOverlay {
  hoverSnap: SnapResult | null;
  /** 描画中のプレビュー線(始点確定後) */
  preview: { a: Vec2; b: Vec2; kind: EdgeKind } | null;
  /** 方向吸着中に紙を横切って示す補助ガイド。 */
  directionGuide: [Vec2, Vec2] | null;
  /** 左右対称に描いているときの対称軸のx座標(正規化座標)。使わないならnull */
  mirrorAxis: number | null;
  /** 対称軸の反対側に出るプレビュー線(左右対称のときだけ) */
  mirrorPreview: { a: Vec2; b: Vec2; kind: EdgeKind } | null;
  /** 描いている最中の曲線と、それに付く「曲がるための線」(CPE-011)。
   * 確定すると細かい折れ線として展開図に入るので、その形をそのまま見せる */
  previewPaths: { points: Vec2[]; kind: EdgeKind }[];
  /** 矩形選択ドラッグ中の範囲(正規化座標) */
  marquee: { a: Vec2; b: Vec2 } | null;
  /** 平らに畳めない点のID(Rust側の判定結果。橙色の丸で知らせる) */
  violations: number[];
  /** 作図補助でクリック済みの点 */
  constructPoints: Vec2[];
  /** 画面の上に出す案内(次に何をすればよいか) */
  hint: string | null;
  /** カーソルの近くに出す説明(平らに畳めない理由) */
  tooltip: { pos: Vec2; text: string } | null;
  /** ドラッグ中の点と、離したら移る位置(CPE-006のプレビュー) */
  vertexDrag: { id: number; to: Vec2 } | null;
  /** 巻き込み折りで追加されるCP上の線分。候補が無ければ省略する */
  suggestedCreases?: [Vec2, Vec2][];
  /** 複数角度スライダーで指している折り目。選択色より太い縁で個別に示す */
  hoveredHinge?: number | null;
}

/** 点を動かしている途中のプレビュー: つながる線を破線で新しい位置へ引き直す */
function drawVertexDrag(
  ctx: CanvasRenderingContext2D,
  doc: Document,
  view: ViewTransform,
  drag: { id: number; to: Vec2 },
): void {
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  ctx.setLineDash([...DASH_PREVIEW]);
  ctx.lineWidth = LINE_WIDTHS.preview;
  for (const e of doc.cp.edges) {
    if (e.v0 !== drag.id && e.v1 !== drag.id) continue;
    const other = byId.get(e.v0 === drag.id ? e.v1 : e.v0);
    if (!other) continue;
    ctx.strokeStyle = EDGE_COLORS[e.kind];
    strokeSegment(ctx, view, drag.to, other);
  }
  ctx.setLineDash([]);
  const [sx, sy] = worldToScreen(view, drag.to);
  ctx.fillStyle = COLORS.selection;
  ctx.beginPath();
  ctx.arc(sx, sy, VERTEX_MARKER_RADIUS, 0, Math.PI * 2);
  ctx.fill();
}

/** 案内・説明の文字サイズ(px) */
const HINT_FONT = "13px sans-serif";
/** 平らに畳めない点の丸の半径(px) */
const VIOLATION_RADIUS = 8;

/** 平らに畳めない点を橙色の丸で示す(操作は止めない) */
function drawViolations(
  ctx: CanvasRenderingContext2D,
  doc: Document,
  view: ViewTransform,
  violations: number[],
): void {
  if (violations.length === 0) return;
  const ids = new Set(violations);
  ctx.strokeStyle = COLORS.violation;
  ctx.lineWidth = 2.5;
  ctx.setLineDash([]);
  for (const v of doc.cp.vertices) {
    if (!ids.has(v.id)) continue;
    const [sx, sy] = worldToScreen(view, v.pos);
    ctx.beginPath();
    ctx.arc(sx, sy, VIOLATION_RADIUS, 0, Math.PI * 2);
    ctx.stroke();
  }
}

/** 黒地に白文字の小さな札を描く(左上が(x, y)) */
function drawLabel(ctx: CanvasRenderingContext2D, x: number, y: number, text: string): void {
  ctx.font = HINT_FONT;
  ctx.textBaseline = "top";
  const w = ctx.measureText(text).width;
  ctx.fillStyle = COLORS.hintBackground;
  ctx.fillRect(x, y, w + 12, 22);
  ctx.fillStyle = COLORS.hintText;
  ctx.fillText(text, x + 6, y + 5);
}

function strokeSegment(
  ctx: CanvasRenderingContext2D,
  view: ViewTransform,
  a: Vec2,
  b: Vec2,
): void {
  const sa = worldToScreen(view, a);
  const sb = worldToScreen(view, b);
  ctx.beginPath();
  ctx.moveTo(sa[0], sa[1]);
  ctx.lineTo(sb[0], sb[1]);
  ctx.stroke();
}

function drawGrid(
  ctx: CanvasRenderingContext2D,
  doc: Document,
  view: ViewTransform,
  fill: Rgb,
): void {
  const n = doc.display.grid_divisions;
  if (n <= 0) return;
  const [w, h] = paperExtent(doc);
  // 方眼は紙の塗りを少し暗くした色。紙の色を変えても消えず、折り線より控えめになる
  ctx.strokeStyle = cssRgb(gridColor(fill));
  ctx.lineWidth = LINE_WIDTHS.grid;
  ctx.setLineDash([]);
  // x方向・y方向とも同じ分割数(輪郭と重なる両端は描かない)。
  // 128等分でも254線分を1つのPath/1回のstrokeへまとめ、描画呼び出しを増やさない。
  ctx.beginPath();
  for (let i = 1; i < n; i++) {
    const x0 = worldToScreen(view, [(i * w) / n, 0]);
    const x1 = worldToScreen(view, [(i * w) / n, h]);
    const y0 = worldToScreen(view, [0, (i * h) / n]);
    const y1 = worldToScreen(view, [w, (i * h) / n]);
    ctx.moveTo(x0[0], x0[1]);
    ctx.lineTo(x1[0], x1[1]);
    ctx.moveTo(y0[0], y0[1]);
    ctx.lineTo(y1[0], y1[1]);
  }
  ctx.stroke();
}

function drawEdges(
  ctx: CanvasRenderingContext2D,
  doc: Document,
  view: ViewTransform,
  selection: Selection,
  fill: Rgb,
  hoveredHinge: number | null,
): void {
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const selected = new Set(selection.edgeIds);
  const halo = haloColor(fill);
  for (const e of doc.cp.edges) {
    const a = byId.get(e.v0);
    const b = byId.get(e.v1);
    if (!a || !b) continue; // 参照切れの壊れた線は描かない(検査の警告で知らせる)
    if (e.id === hoveredHinge) {
      // 全選択の橙色より外側へ紫の縁を敷き、どのスライダーの線かを示す。
      ctx.save();
      ctx.strokeStyle = COLORS.hingeHover;
      ctx.lineWidth = LINE_WIDTHS.hovered;
      ctx.setLineDash([]);
      strokeSegment(ctx, view, a, b);
      ctx.restore();
    }
    if (selected.has(e.id)) {
      // 選択強調: 下に太いハイライトを敷く
      ctx.save();
      ctx.strokeStyle = COLORS.selection;
      ctx.lineWidth = LINE_WIDTHS.selected;
      ctx.setLineDash([]);
      strokeSegment(ctx, view, a, b);
      ctx.restore();
    }
    const width =
      e.kind === "Border"
        ? LINE_WIDTHS.border
        : e.kind === "Aux"
          ? LINE_WIDTHS.aux
          : LINE_WIDTHS.crease;
    ctx.setLineDash(e.kind === "Aux" ? [...DASH_AUX] : []);
    // 縁取りを先に敷く: 方眼や選択の橙色の帯に重なっても線種の色が読める。
    // 輪郭は紙の外の背景と接するので敷かない(紙の縁が白く光って見えるため)
    if (e.kind !== "Border") {
      ctx.strokeStyle = halo;
      ctx.lineWidth = width + HALO_EXTRA_WIDTH;
      strokeSegment(ctx, view, a, b);
    }
    ctx.strokeStyle = EDGE_COLORS[e.kind];
    ctx.lineWidth = width;
    strokeSegment(ctx, view, a, b);
  }
  ctx.setLineDash([]);
}

function drawSelectedVertices(
  ctx: CanvasRenderingContext2D,
  doc: Document,
  view: ViewTransform,
  selection: Selection,
): void {
  const selected = new Set(selection.vertexIds);
  ctx.fillStyle = COLORS.selection;
  for (const v of doc.cp.vertices) {
    if (!selected.has(v.id)) continue;
    const [sx, sy] = worldToScreen(view, v.pos);
    ctx.beginPath();
    ctx.arc(sx, sy, VERTEX_MARKER_RADIUS, 0, Math.PI * 2);
    ctx.fill();
  }
}

/** 左右対称に描いているときの対称軸(紙の縦の中心線)を薄い破線で示す */
function drawMirrorAxis(
  ctx: CanvasRenderingContext2D,
  doc: Document,
  view: ViewTransform,
  axisX: number,
): void {
  const [, h] = paperExtent(doc);
  ctx.strokeStyle = COLORS.mirrorAxis;
  ctx.lineWidth = 1;
  ctx.setLineDash([...DASH_AUX]);
  strokeSegment(ctx, view, [axisX, 0], [axisX, h]);
  ctx.setLineDash([]);
}

function drawOverlay(
  ctx: CanvasRenderingContext2D,
  view: ViewTransform,
  overlay: RenderOverlay,
): void {
  if (overlay.directionGuide) {
    ctx.strokeStyle = COLORS.directionGuide;
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 5]);
    strokeSegment(ctx, view, overlay.directionGuide[0], overlay.directionGuide[1]);
    ctx.setLineDash([]);
  }
  for (const line of [overlay.preview, overlay.mirrorPreview]) {
    if (!line) continue;
    ctx.strokeStyle = EDGE_COLORS[line.kind];
    ctx.lineWidth = LINE_WIDTHS.preview;
    ctx.setLineDash([...DASH_PREVIEW]);
    strokeSegment(ctx, view, line.a, line.b);
    ctx.setLineDash([]);
  }
  for (const path of overlay.previewPaths) {
    if (path.points.length < 2) continue;
    ctx.strokeStyle = EDGE_COLORS[path.kind];
    ctx.lineWidth = LINE_WIDTHS.preview;
    ctx.setLineDash([...DASH_PREVIEW]);
    ctx.beginPath();
    path.points.forEach((p, i) => {
      const [sx, sy] = worldToScreen(view, p);
      if (i === 0) ctx.moveTo(sx, sy);
      else ctx.lineTo(sx, sy);
    });
    ctx.stroke();
    ctx.setLineDash([]);
  }
  for (const [a, b] of overlay.suggestedCreases ?? []) {
    // 既存線や方眼に重なっても候補位置が読めるよう、白い縁取りの上へ描く。
    ctx.setLineDash([...DASH_PREVIEW]);
    ctx.strokeStyle = "rgba(255, 255, 255, 0.9)";
    ctx.lineWidth = 5;
    strokeSegment(ctx, view, a, b);
    ctx.strokeStyle = COLORS.foldSuggestion;
    ctx.lineWidth = 3;
    strokeSegment(ctx, view, a, b);
    ctx.setLineDash([]);
  }
  if (overlay.marquee) {
    const a = worldToScreen(view, overlay.marquee.a);
    const b = worldToScreen(view, overlay.marquee.b);
    const x = Math.min(a[0], b[0]);
    const y = Math.min(a[1], b[1]);
    const w = Math.abs(a[0] - b[0]);
    const h = Math.abs(a[1] - b[1]);
    ctx.fillStyle = COLORS.marqueeFill;
    ctx.fillRect(x, y, w, h);
    ctx.strokeStyle = COLORS.marqueeStroke;
    ctx.lineWidth = 1;
    ctx.strokeRect(x, y, w, h);
  }
  if (overlay.hoverSnap) {
    const [sx, sy] = worldToScreen(view, overlay.hoverSnap.pos);
    ctx.beginPath();
    ctx.arc(sx, sy, SNAP_MARKER_RADIUS, 0, Math.PI * 2);
    ctx.strokeStyle = COLORS.snapMarker;
    ctx.lineWidth = 2;
    ctx.stroke();
  }
  // 作図補助でクリック済みの点(あと何点必要かが見て分かる)
  ctx.fillStyle = COLORS.snapMarker;
  for (const p of overlay.constructPoints) {
    const [sx, sy] = worldToScreen(view, p);
    ctx.beginPath();
    ctx.arc(sx, sy, 4, 0, Math.PI * 2);
    ctx.fill();
  }
  if (overlay.tooltip) {
    const [sx, sy] = worldToScreen(view, overlay.tooltip.pos);
    drawLabel(ctx, sx + 12, sy + 12, overlay.tooltip.text);
  }
  if (overlay.hint) {
    drawLabel(ctx, 8, 8, overlay.hint);
  }
}

/** 拡大中の紙の見えている範囲を、Canvasの右端・下端へ薄く重ねる。 */
function drawViewportPositionBars(
  ctx: CanvasRenderingContext2D,
  doc: Document,
  view: ViewTransform,
  widthPx: number,
  heightPx: number,
): void {
  const bars = deriveViewportPositionBars(doc, view, widthPx, heightPx);
  if (!bars) return;

  const horizontalY = heightPx - POSITION_BAR_MARGIN - POSITION_BAR_THICKNESS;
  const verticalX = widthPx - POSITION_BAR_MARGIN - POSITION_BAR_THICKNESS;
  ctx.save();
  ctx.fillStyle = COLORS.positionBarTrack;
  ctx.fillRect(
    bars.horizontal.trackStart,
    horizontalY,
    bars.horizontal.trackLength,
    POSITION_BAR_THICKNESS,
  );
  ctx.fillRect(
    verticalX,
    bars.vertical.trackStart,
    POSITION_BAR_THICKNESS,
    bars.vertical.trackLength,
  );
  ctx.fillStyle = COLORS.positionBarThumb;
  ctx.fillRect(
    bars.horizontal.thumbStart,
    horizontalY,
    bars.horizontal.thumbLength,
    POSITION_BAR_THICKNESS,
  );
  ctx.fillRect(
    verticalX,
    bars.vertical.thumbStart,
    POSITION_BAR_THICKNESS,
    bars.vertical.thumbLength,
  );
  ctx.restore();
}

/**
 * 展開図全体を描画する。widthPx/heightPxはCSSピクセルのキャンバスサイズ。
 * 呼び出し側でcanvas.width/heightをdpr倍に設定しておくこと。
 */
export function render(
  ctx: CanvasRenderingContext2D,
  widthPx: number,
  heightPx: number,
  dpr: number,
  doc: Document,
  view: ViewTransform,
  selection: Selection,
  overlay: RenderOverlay,
): void {
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.fillStyle = COLORS.background;
  ctx.fillRect(0, 0, widthPx, heightPx);

  // 紙(白地+うっすら影)
  const [w, h] = paperExtent(doc);
  const tl = worldToScreen(view, [0, h]);
  ctx.save();
  ctx.shadowColor = COLORS.paperShadow;
  ctx.shadowBlur = 8;
  // 展開図は紙の表を見ている面なので表の色で塗る(PAP-003の見た目確認)。
  // ただし赤い紙に赤い山折り線が埋もれるので、線が読める濃さまで白へ薄めて塗る
  const fill = paperFill(doc.display.front_color);
  ctx.fillStyle = cssRgb(fill);
  ctx.fillRect(tl[0], tl[1], w * view.scale, h * view.scale);
  ctx.restore();

  drawGrid(ctx, doc, view, fill);
  if (overlay.mirrorAxis !== null) drawMirrorAxis(ctx, doc, view, overlay.mirrorAxis);
  drawEdges(ctx, doc, view, selection, fill, overlay.hoveredHinge ?? null);
  drawSelectedVertices(ctx, doc, view, selection);
  drawViolations(ctx, doc, view, overlay.violations);
  if (overlay.vertexDrag) drawVertexDrag(ctx, doc, view, overlay.vertexDrag);
  drawOverlay(ctx, view, overlay);
  // 選択線や操作ヒントの後に描き、紙の位置を常に見失わないようにする。
  drawViewportPositionBars(ctx, doc, view, widthPx, heightPx);
}
