// ツール別のマウス・キーボード操作。
// 山/谷/補助: 1クリック目で始点(スナップ適用)→プレビュー→2クリック目で確定、Escで中止。
// 選択: クリックで線/頂点を選択、ドラッグで矩形複数選択。削除: クリックした線を削除。
// 共通: ホイールズーム(カーソル中心)、中ボタンドラッグでパン、Deleteで選択線削除。

import type { Document, EdgeKind, EditOp, Vec2 } from "../../lib/types";
import type { Selection, ToolId } from "../../store/appStore";
import { screenToWorld, type ViewTransform } from "./renderer";
import { snap, type SnapResult } from "./snap";

/** 吸着半径(px) */
export const SNAP_RADIUS_PX = 12;
/** クリック選択の許容距離(px) */
export const PICK_TOLERANCE_PX = 6;
/** これ以上動いたらクリックではなくドラッグとみなす距離(px) */
const DRAG_THRESHOLD_PX = 4;
/** ホイール1ノッチあたりのズーム倍率 */
const ZOOM_STEP = 1.1;
const MIN_SCALE = 20;
const MAX_SCALE = 100000;

/** 描画・操作の一時状態(ストアに入れない表示専用状態) */
export interface EphemeralState {
  /** 線ツールの始点(確定済み1クリック目) */
  pendingStart: Vec2 | null;
  cursorWorld: Vec2 | null;
  hoverSnap: SnapResult | null;
  /** 矩形選択: ドラッグ開始点と現在点(正規化座標) */
  marqueeStart: Vec2 | null;
  marqueeEnd: Vec2 | null;
  /** 左ボタン押下位置(px)。クリックとドラッグの判別に使う */
  downScreen: Vec2 | null;
  /** パン中の直前カーソル位置(px) */
  panLast: Vec2 | null;
}

export function initialEphemeralState(): EphemeralState {
  return {
    pendingStart: null,
    cursorWorld: null,
    hoverSnap: null,
    marqueeStart: null,
    marqueeEnd: null,
    downScreen: null,
    panLast: null,
  };
}

/** 操作ハンドラが必要とする文脈(CpEditorが毎イベント渡す) */
export interface InteractionCtx {
  doc: Document;
  view: ViewTransform;
  tool: ToolId;
  selection: Selection;
  state: EphemeralState; // その場で書き換える
  setView: (view: ViewTransform) => void;
  applyEdit: (op: EditOp) => void;
  setSelection: (selection: Selection) => void;
}

/** 線ツール → 引く線の種類(それ以外のツールは未定義) */
export const TOOL_KIND: Partial<Record<ToolId, EdgeKind>> = {
  mountain: "Mountain",
  valley: "Valley",
  aux: "Aux",
};

function dist(a: Vec2, b: Vec2): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1]);
}

function distToSegment(p: Vec2, a: Vec2, b: Vec2): number {
  const ab: Vec2 = [b[0] - a[0], b[1] - a[1]];
  const len2 = ab[0] * ab[0] + ab[1] * ab[1];
  if (len2 === 0) return dist(p, a);
  const t = Math.max(0, Math.min(1, ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2));
  return dist(p, [a[0] + ab[0] * t, a[1] + ab[1] * t]);
}

/** 許容距離内で最も近い頂点のID */
export function pickVertex(doc: Document, world: Vec2, tolNorm: number): number | null {
  let best: number | null = null;
  let bestDist = tolNorm;
  for (const v of doc.cp.vertices) {
    const d = dist(world, v.pos);
    if (d <= bestDist) {
      bestDist = d;
      best = v.id;
    }
  }
  return best;
}

/** 許容距離内で最も近い線のID */
export function pickEdge(doc: Document, world: Vec2, tolNorm: number): number | null {
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  let best: number | null = null;
  let bestDist = tolNorm;
  for (const e of doc.cp.edges) {
    const a = byId.get(e.v0);
    const b = byId.get(e.v1);
    if (!a || !b) continue;
    const d = distToSegment(world, a, b);
    if (d <= bestDist) {
      bestDist = d;
      best = e.id;
    }
  }
  return best;
}

/** 矩形(対角2点)に両端点が入る線と、矩形内の頂点を返す */
export function selectInRect(doc: Document, a: Vec2, b: Vec2): Selection {
  const x0 = Math.min(a[0], b[0]);
  const x1 = Math.max(a[0], b[0]);
  const y0 = Math.min(a[1], b[1]);
  const y1 = Math.max(a[1], b[1]);
  const inside = (p: Vec2) => p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1;
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const edgeIds = doc.cp.edges
    .filter((e) => {
      const va = byId.get(e.v0);
      const vb = byId.get(e.v1);
      return va !== undefined && vb !== undefined && inside(va) && inside(vb);
    })
    .map((e) => e.id);
  const vertexIds = doc.cp.vertices.filter((v) => inside(v.pos)).map((v) => v.id);
  return { edgeIds, vertexIds };
}

/** ホイールズーム(カーソル位置を中心に拡大縮小) */
export function onWheel(ctx: InteractionCtx, screen: Vec2, deltaY: number): void {
  const factor = deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP;
  const scale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, ctx.view.scale * factor));
  const f = scale / ctx.view.scale;
  ctx.setView({
    scale,
    offsetX: screen[0] - (screen[0] - ctx.view.offsetX) * f,
    offsetY: screen[1] - (screen[1] - ctx.view.offsetY) * f,
  });
}

export function onMouseDown(ctx: InteractionCtx, screen: Vec2, button: number): void {
  if (button === 1) {
    ctx.state.panLast = screen;
    return;
  }
  if (button !== 0) return;
  const world = screenToWorld(ctx.view, screen);
  const snapRadius = SNAP_RADIUS_PX / ctx.view.scale;
  const pickTol = PICK_TOLERANCE_PX / ctx.view.scale;

  const kind = TOOL_KIND[ctx.tool];
  if (kind) {
    // 線ツール: 1クリック目=始点、2クリック目=確定
    const pos = snap(ctx.doc, world, snapRadius)?.pos ?? world;
    if (ctx.state.pendingStart === null) {
      ctx.state.pendingStart = pos;
    } else {
      if (dist(ctx.state.pendingStart, pos) > 1e-9) {
        ctx.applyEdit({ type: "AddSegment", a: ctx.state.pendingStart, b: pos, kind });
      }
      ctx.state.pendingStart = null;
    }
    return;
  }
  if (ctx.tool === "delete") {
    const id = pickEdge(ctx.doc, world, pickTol);
    if (id !== null) {
      ctx.applyEdit({ type: "RemoveEdges", ids: [id] });
    }
    return;
  }
  if (ctx.tool === "select") {
    // クリックかドラッグかはmouseupまで分からないので開始点だけ覚える
    ctx.state.downScreen = screen;
    ctx.state.marqueeStart = world;
    ctx.state.marqueeEnd = null;
  }
}

export function onMouseMove(ctx: InteractionCtx, screen: Vec2): void {
  if (ctx.state.panLast) {
    const [lx, ly] = ctx.state.panLast;
    ctx.state.panLast = screen;
    ctx.setView({
      scale: ctx.view.scale,
      offsetX: ctx.view.offsetX + screen[0] - lx,
      offsetY: ctx.view.offsetY + screen[1] - ly,
    });
    return;
  }
  const world = screenToWorld(ctx.view, screen);
  ctx.state.cursorWorld = world;

  // 矩形選択のドラッグ更新
  if (ctx.tool === "select" && ctx.state.downScreen) {
    if (
      ctx.state.marqueeEnd !== null ||
      dist(screen, ctx.state.downScreen) > DRAG_THRESHOLD_PX
    ) {
      ctx.state.marqueeEnd = world;
    }
    return;
  }

  // 線ツールのスナップ候補表示
  if (TOOL_KIND[ctx.tool]) {
    ctx.state.hoverSnap = snap(ctx.doc, world, SNAP_RADIUS_PX / ctx.view.scale);
  } else {
    ctx.state.hoverSnap = null;
  }
}

export function onMouseUp(ctx: InteractionCtx, screen: Vec2, button: number): void {
  if (button === 1) {
    ctx.state.panLast = null;
    return;
  }
  if (button !== 0 || ctx.tool !== "select" || !ctx.state.downScreen) return;
  const start = ctx.state.marqueeStart;
  const end = ctx.state.marqueeEnd;
  ctx.state.downScreen = null;
  ctx.state.marqueeStart = null;
  ctx.state.marqueeEnd = null;
  if (start && end) {
    // 矩形ドラッグ: 範囲内の線・頂点を複数選択
    ctx.setSelection(selectInRect(ctx.doc, start, end));
    return;
  }
  // クリック: 頂点優先で近傍の1つを選択(何もなければ選択解除)
  const world = screenToWorld(ctx.view, screen);
  const pickTol = PICK_TOLERANCE_PX / ctx.view.scale;
  const vertexId = pickVertex(ctx.doc, world, pickTol);
  if (vertexId !== null) {
    ctx.setSelection({ edgeIds: [], vertexIds: [vertexId] });
    return;
  }
  const edgeId = pickEdge(ctx.doc, world, pickTol);
  ctx.setSelection({ edgeIds: edgeId !== null ? [edgeId] : [], vertexIds: [] });
}

/** Esc: 描画・選択操作の中止 / Delete: 選択中の線を削除 */
export function onKeyDown(ctx: InteractionCtx, key: string): void {
  if (key === "Escape") {
    ctx.state.pendingStart = null;
    ctx.state.downScreen = null;
    ctx.state.marqueeStart = null;
    ctx.state.marqueeEnd = null;
    return;
  }
  if (key === "Delete" && ctx.selection.edgeIds.length > 0) {
    ctx.applyEdit({ type: "RemoveEdges", ids: ctx.selection.edgeIds });
  }
}
