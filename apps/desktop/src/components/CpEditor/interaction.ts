// ツール別のマウス・キーボード操作。
// 山/谷/補助: 1クリック目で始点(スナップ適用)→プレビュー→2クリック目で確定、Escで中止。
// 選択: クリックで線/頂点を選択、Ctrl+クリックで追加/解除、ドラッグで矩形複数選択。
// 削除: クリックした線を削除。
// 共通: ホイールズーム(カーソル中心)、Deleteで選択線削除。
// 表示位置の移動(パン)は3通り: スペースキーを押しながら左ドラッグ / 右ドラッグ /
// 中ボタンドラッグ。中ボタンの無い機器でもつかんで動かせるようにするため。

import type { Document, EdgeKind, EditOp, Vec2 } from "../../lib/types";
import type { WheelBehavior } from "../../lib/displayPrefs";
import type { Selection, ToolId } from "../../store/appStore";
import { screenToWorld, type ViewTransform } from "./renderer";
import {
  paperExtent,
  snap,
  snapForMove,
  snapOnDirectionAxis,
  type SnapResult,
} from "./snap";
import { CONSTRUCT_STEPS, constructLines, type ConstructOptions } from "../../lib/construct";
import { CURVE_STEPS, curvePolyline, type CurveOptions } from "../../lib/curve";
import {
  snapLineDirection,
  type DirectionSnapResult,
} from "../../lib/directionSnap";

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
  /** 既存線の延長または角の二等分へ向きだけを合わせた終点。 */
  directionSnap: DirectionSnapResult | null;
  /** 矩形選択: ドラッグ開始点と現在点(正規化座標) */
  marqueeStart: Vec2 | null;
  marqueeEnd: Vec2 | null;
  /** 左ボタン押下位置(px)。クリックとドラッグの判別に使う */
  downScreen: Vec2 | null;
  /** 今の選択操作を既存選択への追加/解除として扱うか(Ctrl/Command押下時) */
  selectionToggle: boolean;
  /** パン中の直前カーソル位置(px) */
  panLast: Vec2 | null;
  /** スペースキーを押し下げている間だけtrue(左ドラッグを表示位置の移動に使う) */
  spaceHeld: boolean;
  /** Shiftを押し下げている間は方向吸着を一時的に解除する。 */
  shiftHeld: boolean;
  /** 作図補助でクリック済みの点(正規化座標) */
  constructPoints: Vec2[];
  /** 作図補助でクリック済みの線(両端の座標) */
  constructSeg: [Vec2, Vec2] | null;
  /** カーソルが乗っている「平らに畳めない点」のID(なければnull) */
  hoverViolation: number | null;
  /** ドラッグ中の点(CPE-006)。toは離したときに確定する位置(それまではプレビュー) */
  vertexDrag: { id: number; from: Vec2; to: Vec2 } | null;
  /** 曲線モードでクリック済みの点(CPE-011)。順は[始点, 終点, 形を決める点…] */
  curvePoints: Vec2[];
}

export function initialEphemeralState(): EphemeralState {
  return {
    pendingStart: null,
    cursorWorld: null,
    hoverSnap: null,
    directionSnap: null,
    marqueeStart: null,
    marqueeEnd: null,
    downScreen: null,
    selectionToggle: false,
    panLast: null,
    spaceHeld: false,
    shiftHeld: false,
    constructPoints: [],
    constructSeg: null,
    hoverViolation: null,
    vertexDrag: null,
    curvePoints: [],
  };
}

/**
 * 曲線モードのクリックを1回分受け取る(CPE-011)。
 * 必要な数がそろったら折れ線として引く。まだ足りなければ点を覚えるだけ。
 */
function onCurveClick(ctx: InteractionCtx, pos: Vec2, kind: EdgeKind): void {
  const st = ctx.state;
  const need = CURVE_STEPS[ctx.curve.shape];
  // 始点と同じところをもう一度押しても線にならないので受け付けない
  if (st.curvePoints.length === 1 && dist(st.curvePoints[0], pos) <= 1e-9) return;
  st.curvePoints.push(pos);
  if (st.curvePoints.length < need) return;
  const pts = curvePolyline(ctx.curve.shape, st.curvePoints, {
    segments: ctx.curve.segments,
  });
  st.curvePoints = [];
  if (pts && pts.length >= 2) ctx.drawCurve(pts, kind);
}

/** 描いている最中の曲線(カーソル位置を仮の点として補った形)。まだ描けなければnull */
export function curveDraft(state: EphemeralState, curve: CurveOptions): Vec2[] | null {
  if (state.curvePoints.length === 0 || !state.cursorWorld) return null;
  const pts = [...state.curvePoints, state.cursorWorld];
  return curvePolyline(curve.shape, pts, { segments: curve.segments });
}

/** 作図補助で集め終えたクリックの数 */
export function constructDone(state: EphemeralState): number {
  return state.constructPoints.length + (state.constructSeg ? 1 : 0);
}

/** 作図補助のクリックを1回分受け取る。必要な数がそろったら補助線を引く */
function onConstructClick(ctx: InteractionCtx, world: Vec2, snapRadius: number): void {
  const st = ctx.state;
  const steps = CONSTRUCT_STEPS[ctx.construct.kind];
  const need = steps[Math.min(constructDone(st), steps.length - 1)];
  if (need === "line") {
    const id = pickEdge(ctx.doc, world, PICK_TOLERANCE_PX / ctx.view.scale);
    const edge = ctx.doc.cp.edges.find((e) => e.id === id);
    const byId = new Map(ctx.doc.cp.vertices.map((v) => [v.id, v.pos]));
    const a = edge && byId.get(edge.v0);
    const b = edge && byId.get(edge.v1);
    if (!a || !b) return; // 線に当たらなければ何もしない(案内は出したまま)
    st.constructSeg = [a, b];
  } else {
    st.constructPoints.push(snap(ctx.doc, world, snapRadius)?.pos ?? world);
  }
  if (constructDone(st) < steps.length) return;
  const lines = constructLines(ctx.construct.kind, st.constructPoints, st.constructSeg, {
    divisions: ctx.construct.divisions,
    stepDeg: ctx.construct.stepDeg,
    paper: paperExtent(ctx.doc),
  });
  for (const [a, b] of lines) {
    ctx.applyEdit({ type: "AddSegment", a, b, kind: "Aux" });
  }
  st.constructPoints = [];
  st.constructSeg = null;
}

/** 操作ハンドラが必要とする文脈(CpEditorが毎イベント渡す) */
export interface InteractionCtx {
  doc: Document;
  view: ViewTransform;
  tool: ToolId;
  selection: Selection;
  /** 作図補助の選び方(どの作図か・等分数・角度の刻み) */
  construct: ConstructOptions;
  /** 曲線の折り目の選び方(直線/曲線・描き方・分割・曲がるための線) */
  curve: CurveOptions;
  /** 修飾キーを押していないときのホイール動作(端末ごとの設定)。 */
  wheelBehavior: WheelBehavior;
  /** 平らに畳めない点のID(Rust側の判定結果)。橙色の丸で知らせる */
  violations: number[];
  state: EphemeralState; // その場で書き換える
  setView: (view: ViewTransform) => void;
  applyEdit: (op: EditOp) => void;
  /** 線を1本引く(左右対称のときは反対側にも引かれる。CPE-010) */
  drawSegment: (a: Vec2, b: Vec2, kind: EdgeKind) => void;
  /** 曲線を折れ線として引く(曲がるための線も設定に応じて一緒に引かれる) */
  drawCurve: (points: Vec2[], kind: EdgeKind) => void;
  setSelection: (selection: Selection) => void;
  /** 折るツールで引いた線を確定前の状態としてストアへ渡す */
  beginFoldDraft: (line: [Vec2, Vec2], source: "2d" | "3d") => void;
}

/** 線ツール → 引く線の種類(それ以外のツールは未定義) */
export const TOOL_KIND: Partial<Record<ToolId, EdgeKind>> = {
  mountain: "Mountain",
  valley: "Valley",
  aux: "Aux",
};

/**
 * 2回のクリックで線を引くツールか(折るツールを含む)。
 * 折るツールのプレビューは谷折りの色で描く。
 */
export function previewKind(tool: ToolId): EdgeKind | undefined {
  return tool === "fold" ? "Valley" : TOOL_KIND[tool];
}

function dist(a: Vec2, b: Vec2): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1]);
}

/**
 * 通常の直線描画の終点を決める。方向吸着中はその方向を軸として保ち、
 * 近くの頂点・グリッド交点は軸への投影点、線分は軸との交点へ吸着する。
 * 方向吸着が無いときは従来どおり頂点・グリッド・線分の順で吸着する。
 * 曲線モードと「折る」ツールには方向吸着を適用しない。
 */
function resolveLineEndpoint(
  ctx: InteractionCtx,
  world: Vec2,
  snapRadius: number,
): {
  pos: Vec2;
  pointSnap: SnapResult | null;
  directionSnap: DirectionSnapResult | null;
} {
  const start = ctx.state.pendingStart;
  const straightKind = TOOL_KIND[ctx.tool];
  const directionSnap =
    start && straightKind && !ctx.curve.enabled && !ctx.state.shiftHeld
      ? snapLineDirection(ctx.doc, start, world)
      : null;
  if (directionSnap && start) {
    const pointSnap = snapOnDirectionAxis(
      ctx.doc,
      start,
      directionSnap.direction,
      world,
      snapRadius,
    );
    const pos = pointSnap?.pos ?? directionSnap.pos;
    return {
      pos,
      pointSnap,
      directionSnap: { ...directionSnap, pos },
    };
  }

  const pointSnap = snap(ctx.doc, world, snapRadius);
  return {
    pos: pointSnap?.pos ?? world,
    pointSnap,
    directionSnap: null,
  };
}

/** カーソル移動・Shift解除のどちらからも同じ吸着結果へ更新する。 */
function refreshLineEndpoint(ctx: InteractionCtx, world: Vec2, snapRadius: number): Vec2 {
  const resolved = resolveLineEndpoint(ctx, world, snapRadius);
  ctx.state.hoverSnap = resolved.pointSnap;
  ctx.state.directionSnap = resolved.directionSnap;
  return resolved.pos;
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

/** ホイール操作1回に含まれる移動量と修飾キー。移動量はCSS pxへ換算済み。 */
export interface WheelGesture {
  deltaX: number;
  deltaY: number;
  shiftKey: boolean;
  ctrlKey: boolean;
}

/** 設定と修飾キーから決まる、今回のホイールの役割。 */
export type WheelAction = "zoom" | "scroll-x" | "scroll-y";

/**
 * 通常ホイールとCtrl+ホイールの役割を入れ替える。
 * スクロールになった場合だけShiftで横方向へ切り替える。
 */
export function wheelAction(
  behavior: WheelBehavior,
  modifiers: Pick<WheelGesture, "shiftKey" | "ctrlKey">,
): WheelAction {
  const zoom = (behavior === "zoom") !== modifiers.ctrlKey;
  if (zoom) return "zoom";
  return modifiers.shiftKey ? "scroll-x" : "scroll-y";
}

/** 現在の設定に合う、Canvas上の短い操作案内。 */
export function wheelHint(behavior: WheelBehavior): string {
  return behavior === "scroll"
    ? "ホイール: 上下 / Shift+ホイール: 左右 / Ctrl+ホイール: 拡大縮小"
    : "ホイール: 拡大縮小 / Ctrl+ホイール: 上下 / Ctrl+Shift+ホイール: 左右";
}

/** ホイールの移動量だけ表示位置を動かす(正の量なら紙は上・左へ動く)。 */
export function scrollView(
  view: ViewTransform,
  action: "scroll-x" | "scroll-y",
  deltaX: number,
  deltaY: number,
): ViewTransform {
  // Shift+ホイールをOSがdeltaXへ移し替える場合にも同じ量で動かす。
  const delta = deltaY !== 0 ? deltaY : deltaX;
  return {
    scale: view.scale,
    offsetX: view.offsetX - (action === "scroll-x" ? delta : 0),
    offsetY: view.offsetY - (action === "scroll-y" ? delta : 0),
  };
}

/** カーソル位置を動かさずに拡大縮小した表示変換を返す。 */
export function zoomView(view: ViewTransform, screen: Vec2, delta: number): ViewTransform {
  if (delta === 0) return view;
  const factor = delta < 0 ? ZOOM_STEP : 1 / ZOOM_STEP;
  const scale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, view.scale * factor));
  const f = scale / view.scale;
  return {
    scale,
    offsetX: screen[0] - (screen[0] - view.offsetX) * f,
    offsetY: screen[1] - (screen[1] - view.offsetY) * f,
  };
}

/** 設定に従ってスクロールまたはカーソル中心ズームを行う。 */
export function onWheel(ctx: InteractionCtx, screen: Vec2, gesture: WheelGesture): void {
  const action = wheelAction(ctx.wheelBehavior, gesture);
  ctx.setView(
    action === "zoom"
      ? zoomView(ctx.view, screen, gesture.deltaY !== 0 ? gesture.deltaY : gesture.deltaX)
      : scrollView(ctx.view, action, gesture.deltaX, gesture.deltaY),
  );
}

/** このボタン(と今のキー状態)は「展開図をつかんで動かす」操作か */
export function isPanStart(state: EphemeralState, button: number): boolean {
  return button === 1 || button === 2 || (button === 0 && state.spaceHeld);
}

export function onMouseDown(
  ctx: InteractionCtx,
  screen: Vec2,
  button: number,
  shiftHeld?: boolean,
  selectionToggle = false,
): void {
  if (shiftHeld !== undefined) ctx.state.shiftHeld = shiftHeld;
  // 表示位置の移動を最優先で拾う(線引き・選択より先に判定する)
  if (isPanStart(ctx.state, button)) {
    ctx.state.panLast = screen;
    ctx.state.directionSnap = null;
    return;
  }
  if (button !== 0) return;
  const world = screenToWorld(ctx.view, screen);
  const snapRadius = SNAP_RADIUS_PX / ctx.view.scale;
  const pickTol = PICK_TOLERANCE_PX / ctx.view.scale;

  const kind = TOOL_KIND[ctx.tool];
  if (kind && ctx.curve.enabled) {
    // 曲線モード(CPE-011): 始点・終点・形を決める点を順にクリックする
    onCurveClick(ctx, snap(ctx.doc, world, snapRadius)?.pos ?? world, kind);
    return;
  }
  if (kind || ctx.tool === "fold") {
    // 線ツール・折るツール: 1クリック目=始点、2クリック目=確定
    const pos = refreshLineEndpoint(ctx, world, snapRadius);
    const start = ctx.state.pendingStart;
    if (start === null) {
      ctx.state.pendingStart = pos;
    } else {
      if (dist(start, pos) > 1e-9) {
        if (kind) {
          ctx.drawSegment(start, pos, kind);
        } else {
          ctx.beginFoldDraft([start, pos], "2d");
        }
      }
      ctx.state.pendingStart = null;
      ctx.state.directionSnap = null;
    }
    return;
  }
  if (ctx.tool === "construct") {
    onConstructClick(ctx, world, snapRadius);
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
    ctx.state.selectionToggle = selectionToggle;
    ctx.state.marqueeEnd = null;
    // 点の上で押したときは、その点を動かす操作として始める(CPE-006)。
    // 動かさずに離せばただの選択になる
    const hit = pickVertex(ctx.doc, world, pickTol);
    const pos = hit === null ? null : ctx.doc.cp.vertices.find((v) => v.id === hit)?.pos;
    if (hit !== null && pos) {
      ctx.state.vertexDrag = { id: hit, from: pos, to: pos };
      ctx.state.marqueeStart = null;
      // Ctrl/Commandクリックは離したときに既存選択へ追加/解除する。
      // 通常の点ドラッグは従来どおり押した時点でその点を選ぶ。
      if (!selectionToggle) ctx.setSelection({ edgeIds: [], vertexIds: [hit] });
      return;
    }
    ctx.state.marqueeStart = world;
  }
}

export function onMouseMove(ctx: InteractionCtx, screen: Vec2, shiftHeld?: boolean): void {
  if (shiftHeld !== undefined) ctx.state.shiftHeld = shiftHeld;
  if (ctx.state.panLast) {
    const [lx, ly] = ctx.state.panLast;
    ctx.state.panLast = screen;
    ctx.setView({
      scale: ctx.view.scale,
      offsetX: ctx.view.offsetX + screen[0] - lx,
      offsetY: ctx.view.offsetY + screen[1] - ly,
    });
    ctx.state.directionSnap = null;
    return;
  }
  const world = screenToWorld(ctx.view, screen);
  ctx.state.cursorWorld = world;

  // 「平らに畳めない点」に近づいたら、その点を覚えて理由を出せるようにする
  const near = pickVertex(ctx.doc, world, SNAP_RADIUS_PX / ctx.view.scale);
  ctx.state.hoverViolation =
    near !== null && ctx.violations.includes(near) ? near : null;

  // 点を動かしている間は、離す前の位置をプレビューとして持つだけにする
  // (1ドラッグ=1回の編集。途中の位置は履歴に残さない)
  const drag = ctx.state.vertexDrag;
  if (drag && ctx.state.downScreen) {
    const radius = SNAP_RADIUS_PX / ctx.view.scale;
    drag.to = snapForMove(ctx.doc, world, radius, drag.id)?.pos ?? world;
    ctx.state.hoverSnap = null;
    ctx.state.directionSnap = null;
    return;
  }

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

  // 通常の直線は、点吸着が無いときだけ延長・二等分方向へ吸着する。
  const straightKind = TOOL_KIND[ctx.tool];
  if ((straightKind && !ctx.curve.enabled) || ctx.tool === "fold") {
    refreshLineEndpoint(ctx, world, SNAP_RADIUS_PX / ctx.view.scale);
  } else if (previewKind(ctx.tool) || ctx.tool === "construct") {
    ctx.state.hoverSnap = snap(ctx.doc, world, SNAP_RADIUS_PX / ctx.view.scale);
    ctx.state.directionSnap = null;
  } else {
    ctx.state.hoverSnap = null;
    ctx.state.directionSnap = null;
  }
}

function mergeSelection(a: Selection, b: Selection): Selection {
  return {
    edgeIds: [...new Set([...a.edgeIds, ...b.edgeIds])],
    vertexIds: [...new Set([...a.vertexIds, ...b.vertexIds])],
  };
}

function toggleSelectionId(
  selection: Selection,
  kind: "edge" | "vertex",
  id: number,
): Selection {
  const key = kind === "edge" ? "edgeIds" : "vertexIds";
  const ids = selection[key];
  return {
    ...selection,
    [key]: ids.includes(id) ? ids.filter((value) => value !== id) : [...ids, id],
  };
}

export function onMouseUp(
  ctx: InteractionCtx,
  screen: Vec2,
  button: number,
  selectionToggle?: boolean,
): void {
  // どのボタンで動かし始めていても、離したところで移動を終える
  if (ctx.state.panLast) {
    ctx.state.panLast = null;
    return;
  }
  if (button !== 0 || ctx.tool !== "select" || !ctx.state.downScreen) return;
  // クリック途中でCtrl/Commandを離しても、押し始めた時点の複数選択意図を保つ。
  const toggle = ctx.state.selectionToggle || selectionToggle === true;
  ctx.state.selectionToggle = false;
  // 点を離したところで動かし方を確定する(CPE-006)。動いていなければ選択のまま
  const drag = ctx.state.vertexDrag;
  if (drag) {
    ctx.state.vertexDrag = null;
    ctx.state.downScreen = null;
    if (dist(drag.from, drag.to) > 1e-9) {
      ctx.applyEdit({ type: "MoveVertex", id: drag.id, to: drag.to });
    } else if (toggle) {
      ctx.setSelection(toggleSelectionId(ctx.selection, "vertex", drag.id));
    }
    return;
  }
  const start = ctx.state.marqueeStart;
  const end = ctx.state.marqueeEnd;
  ctx.state.downScreen = null;
  ctx.state.marqueeStart = null;
  ctx.state.marqueeEnd = null;
  if (start && end) {
    // 矩形ドラッグ: 範囲内の線・頂点を複数選択
    const inRect = selectInRect(ctx.doc, start, end);
    ctx.setSelection(toggle ? mergeSelection(ctx.selection, inRect) : inRect);
    return;
  }
  // クリック: 頂点優先で近傍の1つを選択(何もなければ選択解除)
  const world = screenToWorld(ctx.view, screen);
  const pickTol = PICK_TOLERANCE_PX / ctx.view.scale;
  const vertexId = pickVertex(ctx.doc, world, pickTol);
  if (vertexId !== null) {
    ctx.setSelection(
      toggle
        ? toggleSelectionId(ctx.selection, "vertex", vertexId)
        : { edgeIds: [], vertexIds: [vertexId] },
    );
    return;
  }
  const edgeId = pickEdge(ctx.doc, world, pickTol);
  if (toggle) {
    // Ctrl/Command+空白は現在の複数選択を保つ。
    if (edgeId !== null) {
      ctx.setSelection(toggleSelectionId(ctx.selection, "edge", edgeId));
    }
    return;
  }
  ctx.setSelection({ edgeIds: edgeId !== null ? [edgeId] : [], vertexIds: [] });
}

/** スペースキーとみなす入力(環境によっては"Spacebar"が来る) */
export function isSpaceKey(key: string): boolean {
  return key === " " || key === "Spacebar";
}

/** 今の状態に合ったカーソルの形(つかんで動かしている間は「つかんだ手」) */
export function cursorFor(tool: ToolId, state: EphemeralState): string {
  if (state.panLast) return "grabbing";
  if (state.spaceHeld) return "grab";
  if (previewKind(tool) !== undefined || tool === "delete" || tool === "construct") {
    return "crosshair";
  }
  return "default";
}

/** 表示位置を動かしている(動かせる)ときの案内。それ以外はnull(設計原則3b) */
export function panHint(state: EphemeralState): string | null {
  if (state.panLast) return "表示位置を動かしています(離すと決まります)";
  if (state.spaceHeld) return "スペースを押している間はドラッグで表示位置を動かせます";
  return null;
}

/** Esc: 描画・選択操作の中止 / Delete: 選択中の線を削除 / スペース: つかんで動かす */
export function onKeyDown(ctx: InteractionCtx, key: string): void {
  if (key === "Shift") {
    ctx.state.shiftHeld = true;
    if (ctx.state.cursorWorld) {
      refreshLineEndpoint(
        ctx,
        ctx.state.cursorWorld,
        SNAP_RADIUS_PX / ctx.view.scale,
      );
    } else {
      ctx.state.directionSnap = null;
    }
    return;
  }
  if (isSpaceKey(key)) {
    ctx.state.spaceHeld = true;
    return;
  }
  if (key === "Escape") {
    ctx.state.spaceHeld = false;
    ctx.state.shiftHeld = false;
    ctx.state.panLast = null;
    ctx.state.pendingStart = null;
    ctx.state.directionSnap = null;
    ctx.state.downScreen = null;
    ctx.state.marqueeStart = null;
    ctx.state.marqueeEnd = null;
    ctx.state.selectionToggle = false;
    ctx.state.constructPoints = [];
    ctx.state.constructSeg = null;
    ctx.state.curvePoints = [];
    // 動かしかけの点は元の位置に戻す(まだ確定していない)
    ctx.state.vertexDrag = null;
    return;
  }
  if (key === "Delete" && ctx.selection.edgeIds.length > 0) {
    ctx.applyEdit({ type: "RemoveEdges", ids: ctx.selection.edgeIds });
  }
}

/** スペースキーを離したら、つかんで動かす状態を解く */
export function onKeyUp(ctx: InteractionCtx, key: string): void {
  if (key === "Shift") {
    ctx.state.shiftHeld = false;
    if (ctx.state.cursorWorld) {
      refreshLineEndpoint(
        ctx,
        ctx.state.cursorWorld,
        SNAP_RADIUS_PX / ctx.view.scale,
      );
    }
    return;
  }
  if (!isSpaceKey(key)) return;
  ctx.state.spaceHeld = false;
  ctx.state.panLast = null;
}
