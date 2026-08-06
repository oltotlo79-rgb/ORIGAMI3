// 展開図のCanvas 2D描画。色・線幅はここの定数に集約する。
// 座標系: 正規化座標(長辺=1.0、y軸上向き)→ 画面座標(px、y軸下向き)。

import type { Document, EdgeKind, Vec2 } from "../../lib/types";
import type { Selection } from "../../store/appStore";
import { paperExtent, type SnapResult } from "./snap";

/** 線種ごとの色 */
export const EDGE_COLORS: Record<EdgeKind, string> = {
  Border: "#000000",
  Mountain: "#d43c3c",
  Valley: "#3b6fc9",
  Aux: "#909090",
};

export const COLORS = {
  background: "#d8d8dc",
  paper: "#ffffff",
  paperShadow: "rgba(0, 0, 0, 0.25)",
  grid: "#dcdcdc",
  selection: "#ff9500",
  snapMarker: "#2aa02a",
  /** 平らに畳めない点(CPE-009)。操作は止めず色で知らせるだけ */
  violation: "#ff8c00",
  hintBackground: "rgba(20, 20, 24, 0.78)",
  hintText: "#ffffff",
  /** 左右対称に描くときの対称軸(CPE-010)。薄く出して邪魔をしない */
  mirrorAxis: "rgba(59, 111, 201, 0.45)",
  marqueeFill: "rgba(59, 111, 201, 0.12)",
  marqueeStroke: "#3b6fc9",
} as const;

/** 線幅(px) */
export const LINE_WIDTHS = {
  border: 2,
  crease: 1.5,
  aux: 1,
  grid: 1,
  selected: 4,
  preview: 1.5,
} as const;

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

/** 描画に必要な一時状態(ホバー・プレビュー等、ストア外の表示専用状態) */
export interface RenderOverlay {
  hoverSnap: SnapResult | null;
  /** 描画中のプレビュー線(始点確定後) */
  preview: { a: Vec2; b: Vec2; kind: EdgeKind } | null;
  /** 左右対称に描いているときの対称軸のx座標(正規化座標)。使わないならnull */
  mirrorAxis: number | null;
  /** 対称軸の反対側に出るプレビュー線(左右対称のときだけ) */
  mirrorPreview: { a: Vec2; b: Vec2; kind: EdgeKind } | null;
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
}

/** 紙の色([r,g,b])をcanvasの色文字列にする */
export function paperColor(rgb: [number, number, number]): string {
  return `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
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
): void {
  const n = doc.display.grid_divisions;
  if (n <= 0) return;
  const [w, h] = paperExtent(doc);
  ctx.strokeStyle = COLORS.grid;
  ctx.lineWidth = LINE_WIDTHS.grid;
  // x方向・y方向とも同じ分割数(輪郭と重なる両端は描かない)
  for (let i = 1; i < n; i++) {
    strokeSegment(ctx, view, [(i * w) / n, 0], [(i * w) / n, h]);
    strokeSegment(ctx, view, [0, (i * h) / n], [w, (i * h) / n]);
  }
}

function drawEdges(
  ctx: CanvasRenderingContext2D,
  doc: Document,
  view: ViewTransform,
  selection: Selection,
): void {
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const selected = new Set(selection.edgeIds);
  for (const e of doc.cp.edges) {
    const a = byId.get(e.v0);
    const b = byId.get(e.v1);
    if (!a || !b) continue; // 参照切れの壊れた線は描かない(検査の警告で知らせる)
    if (selected.has(e.id)) {
      // 選択強調: 下に太いハイライトを敷く
      ctx.save();
      ctx.strokeStyle = COLORS.selection;
      ctx.lineWidth = LINE_WIDTHS.selected;
      ctx.setLineDash([]);
      strokeSegment(ctx, view, a, b);
      ctx.restore();
    }
    ctx.strokeStyle = EDGE_COLORS[e.kind];
    ctx.lineWidth =
      e.kind === "Border"
        ? LINE_WIDTHS.border
        : e.kind === "Aux"
          ? LINE_WIDTHS.aux
          : LINE_WIDTHS.crease;
    ctx.setLineDash(e.kind === "Aux" ? [...DASH_AUX] : []);
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
  for (const line of [overlay.preview, overlay.mirrorPreview]) {
    if (!line) continue;
    ctx.strokeStyle = EDGE_COLORS[line.kind];
    ctx.lineWidth = LINE_WIDTHS.preview;
    ctx.setLineDash([...DASH_PREVIEW]);
    strokeSegment(ctx, view, line.a, line.b);
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
  // 展開図は紙の表を見ている面なので、表の色で塗る(PAP-003の見た目確認)
  ctx.fillStyle = paperColor(doc.display.front_color);
  ctx.fillRect(tl[0], tl[1], w * view.scale, h * view.scale);
  ctx.restore();

  drawGrid(ctx, doc, view);
  if (overlay.mirrorAxis !== null) drawMirrorAxis(ctx, doc, view, overlay.mirrorAxis);
  drawEdges(ctx, doc, view, selection);
  drawSelectedVertices(ctx, doc, view, selection);
  drawViolations(ctx, doc, view, overlay.violations);
  if (overlay.vertexDrag) drawVertexDrag(ctx, doc, view, overlay.vertexDrag);
  drawOverlay(ctx, view, overlay);
}
