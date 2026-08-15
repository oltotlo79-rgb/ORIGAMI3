// ツール別のマウス・キーボード操作。
// 山/谷/補助: 1クリック目で始点(スナップ適用)→プレビュー→2クリック目で確定、Escで中止。
// 選択: クリックで線/頂点を選択、Ctrl+クリックで追加/解除、ドラッグで矩形複数選択。
// 削除: クリックした線を削除。
// 共通: ホイールズーム(カーソル中心)、Deleteで選択線削除。
// 表示位置の移動(パン)は3通り: スペースキーを押しながら左ドラッグ / 右ドラッグ /
// 中ボタンドラッグ。中ボタンの無い機器でもつかんで動かせるようにするため。

import type { Document, EdgeKind, EditOp, Face, Frame3D, Vec2 } from "../../lib/types";
import { ALIGN_STEPS, type AlignMode, type AlignTarget } from "../../lib/alignFold";
import type { WheelBehavior } from "../../lib/displayPrefs";
import type { MirrorLine } from "../../lib/mirror";
import type { AlignCpPick, Selection, ToolId } from "../../store/appStore";
import { screenToWorld, type ViewTransform } from "./renderer";
import {
  paperExtent,
  prepareMoveSnap,
  sameMoveMirrorAxis,
  snap,
  snapForMove,
  snapOnDirectionAxis,
  type PreparedMoveSnap,
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
// 画面座標との相互変換で紙端をわずかに越える丸めだけを許容する。
// これを超える紙外入力は、12pxの吸着範囲内でも紙上へ引き込まない。
const PAPER_BOUNDS_ROUNDING_EPSILON = 1e-12;
const OUTSIDE_PAPER_LINE_HINT =
  "紙の外には線を引けません。紙の上をクリックしてください";

/** 描画・操作の一時状態(ストアに入れない表示専用状態) */
export interface EphemeralState {
  /** 線ツールの始点(確定済み1クリック目) */
  pendingStart: Vec2 | null;
  /** 紙外を押したときにCanvasへ出す一時案内。 */
  lineInputHint: string | null;
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
  vertexDrag: { id: number; from: Vec2; to: Vec2; snap: PreparedMoveSnap | null } | null;
  /** 曲線モードでクリック済みの点(CPE-011)。順は[始点, 終点, 形を決める点…] */
  curvePoints: Vec2[];
}

export function initialEphemeralState(): EphemeralState {
  return {
    pendingStart: null,
    lineInputHint: null,
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
  // 確定時のクリックと同じ吸着後座標を使い、押した瞬間に形が跳ばないようにする。
  const pts = [...state.curvePoints, state.hoverSnap?.pos ?? state.cursorWorld];
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
    paper: paperExtent(ctx.finalDoc),
  });
  // 角度線のように何本もまとめて引く作図でも、元に戻す1回で作る前へ戻れるよう
  // 1回の要求として渡す(不具合D05)
  if (lines.length > 0) {
    ctx.applyEdit(
      lines.map(([a, b]) => ({ type: "AddSegment", a, b, kind: "Aux" }) as const),
    );
  }
  st.constructPoints = [];
  st.constructSeg = null;
}

/** 操作ハンドラが必要とする文脈(CpEditorが毎イベント渡す) */
export interface InteractionCtx {
  /** 現在の手順位置で画面に見えている展開図。操作対象は常にこの中から選ぶ。 */
  doc: Document;
  /** 紙の寸法と、展開図から3Dへの面対応に使う最終作品。 */
  finalDoc: Document;
  view: ViewTransform;
  tool: ToolId;
  selection: Selection;
  /** 「合わせて折る」の選択途中。nullなら通常の折り線入力として扱う。 */
  alignDraft: { mode: AlignMode; picks: AlignTarget[] } | null;
  /** 展開図の頂点・辺を現在の畳み平面へ写すための面対応。 */
  faces: Face[];
  frame3d: Frame3D | null;
  /** 作図補助の選び方(どの作図か・等分数・角度の刻み) */
  construct: ConstructOptions;
  /** 曲線の折り目の選び方(直線/曲線・描き方・分割・曲がるための線) */
  curve: CurveOptions;
  /** 修飾キーを押していないときのホイール動作(端末ごとの設定)。 */
  wheelBehavior: WheelBehavior;
  /** 平らに畳めない点のID(Rust側の判定結果)。橙色の丸で知らせる */
  violations: number[];
  /** 点を動かすとき、既存頂点を折り返す現在の対称基準線。 */
  mirrorAxis: MirrorLine | null;
  state: EphemeralState; // その場で書き換える
  setView: (view: ViewTransform) => void;
  /** 展開図を変える要求を送る。複数渡すと画面での1回の入力として扱われ、
   * 元に戻す1回でまとめて戻る(不具合D05) */
  applyEdit: (op: EditOp | EditOp[]) => void;
  /** 線を1本引く(左右対称のときは反対側にも引かれる。CPE-010) */
  drawSegment: (a: Vec2, b: Vec2, kind: EdgeKind) => void;
  /** 曲線を折れ線として引く(曲がるための線も設定に応じて一緒に引かれる) */
  drawCurve: (points: Vec2[], kind: EdgeKind) => void;
  setSelection: (selection: Selection) => void;
  /** 折るツールで引いた線を確定前の状態としてストアへ渡す */
  beginFoldDraft: (line: [Vec2, Vec2], source: "2d" | "3d") => void;
  /** 「合わせて折る」で、次に必要な点または線をストアへ渡す。 */
  pickAlignTarget: (
    target: AlignTarget,
    cursor?: Vec2 | null,
    cpPick?: AlignCpPick | null,
  ) => void;
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

/** 紙端の丸め誤差だけを端へ戻し、それを超える紙外座標は受け付けない。 */
function pointOnPaper(doc: Document, pos: Vec2): Vec2 | null {
  const [width, height] = paperExtent(doc);
  const epsilon = PAPER_BOUNDS_ROUNDING_EPSILON;
  if (
    !Number.isFinite(pos[0]) ||
    !Number.isFinite(pos[1]) ||
    pos[0] < -epsilon ||
    pos[0] > width + epsilon ||
    pos[1] < -epsilon ||
    pos[1] > height + epsilon
  ) {
    return null;
  }
  return [
    Math.max(0, Math.min(width, pos[0])),
    Math.max(0, Math.min(height, pos[1])),
  ];
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

/** Borderの端点は座標に関係なく紙の外形なので、点移動の対象にしない。 */
function isBorderVertex(doc: Document, vertexId: number): boolean {
  return doc.cp.edges.some(
    (edge) =>
      edge.kind === "Border" &&
      (edge.v0 === vertexId || edge.v1 === vertexId),
  );
}

/** 現在の手順で見えている線に属する頂点だけを、合わせ対象の点として拾う。 */
function pickVisibleAlignVertex(
  doc: Document,
  world: Vec2,
  tolNorm: number,
): number | null {
  const visible = new Set<number>();
  for (const edge of doc.cp.edges) {
    visible.add(edge.v0);
    visible.add(edge.v1);
  }
  let best: number | null = null;
  let bestDist = tolNorm;
  for (const vertex of doc.cp.vertices) {
    if (!visible.has(vertex.id)) continue;
    const d = dist(world, vertex.pos);
    if (d <= bestDist) {
      bestDist = d;
      best = vertex.id;
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

/** 3Dの面が「合わせて折る」で使える平らな状態かを判定する余裕。 */
const ALIGN_FLAT_EPS = 1e-6;
const ALIGN_MAP_EPS = 1e-9;

interface FlatFaceMap {
  faceId: number;
  layer: number;
  source: Vec2[];
  target: Vec2[];
}

function flatFaceMaps(doc: Document, faces: Face[], frame3d: Frame3D): FlatFaceMap[] {
  const facesById = new Map(faces.map((face) => [face.id, face]));
  const positions = new Map(doc.cp.vertices.map((vertex) => [vertex.id, vertex.pos]));
  const maps: FlatFaceMap[] = [];
  for (const frameFace of frame3d.faces) {
    const face = facesById.get(frameFace.face);
    if (
      !face ||
      frameFace.polygon.length !== face.vertices.length ||
      frameFace.polygon.some(
        (p) =>
          !Number.isFinite(p[0]) ||
          !Number.isFinite(p[1]) ||
          !Number.isFinite(p[2]) ||
          Math.abs(p[2]) > ALIGN_FLAT_EPS,
      )
    ) {
      continue;
    }
    const source = face.vertices.map((id) => positions.get(id));
    if (source.some((point) => point === undefined)) continue;
    maps.push({
      faceId: face.id,
      layer: frameFace.layer,
      source: source as Vec2[],
      target: frameFace.polygon.map((p): Vec2 => [p[0], p[1]]),
    });
  }
  return maps;
}

function pointInPolygon(point: Vec2, polygon: Vec2[]): boolean {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[j];
    const b = polygon[i];
    if (distToSegment(point, a, b) <= ALIGN_MAP_EPS) return true;
    if (
      (a[1] > point[1]) !== (b[1] > point[1]) &&
      point[0] < ((b[0] - a[0]) * (point[1] - a[1])) / (b[1] - a[1]) + a[0]
    ) {
      inside = !inside;
    }
  }
  return inside;
}

/** 面の3点対応から、面内の点へ同じ回転・鏡映・平行移動を適用する。 */
function mapPointInFace(map: FlatFaceMap, point: Vec2): Vec2 | null {
  const sourceA = map.source[0];
  const targetA = map.target[0];
  if (!sourceA || !targetA) return null;
  for (let i = 1; i < map.source.length; i++) {
    for (let j = i + 1; j < map.source.length; j++) {
      const sourceB = map.source[i];
      const sourceC = map.source[j];
      const targetB = map.target[i];
      const targetC = map.target[j];
      const bx = sourceB[0] - sourceA[0];
      const by = sourceB[1] - sourceA[1];
      const cx = sourceC[0] - sourceA[0];
      const cy = sourceC[1] - sourceA[1];
      const det = bx * cy - by * cx;
      if (Math.abs(det) <= ALIGN_MAP_EPS) continue;
      const px = point[0] - sourceA[0];
      const py = point[1] - sourceA[1];
      const u = (px * cy - py * cx) / det;
      const v = (bx * py - by * px) / det;
      return [
        targetA[0] + u * (targetB[0] - targetA[0]) + v * (targetC[0] - targetA[0]),
        targetA[1] + u * (targetB[1] - targetA[1]) + v * (targetC[1] - targetA[1]),
      ];
    }
  }
  return null;
}

function preferMapped<T extends { layer: number; faceId: number }>(
  candidate: T,
  current: T | null,
): boolean {
  return (
    current === null ||
    candidate.layer > current.layer ||
    (candidate.layer === current.layer && candidate.faceId < current.faceId)
  );
}

/** 展開図の頂点を、現在の平らに畳んだ紙の座標へ写す。 */
export function foldedAlignPoint(
  doc: Document,
  faces: Face[],
  frame3d: Frame3D | null,
  vertexId: number,
): Vec2 | null {
  const source = doc.cp.vertices.find((v) => v.id === vertexId)?.pos ?? null;
  if (!source || !frame3d) return source;
  let best: { p: Vec2; layer: number; faceId: number } | null = null;
  for (const map of flatFaceMaps(doc, faces, frame3d)) {
    if (!pointInPolygon(source, map.source)) continue;
    const p = mapPointInFace(map, source);
    const candidate = p ? { p, layer: map.layer, faceId: map.faceId } : null;
    if (candidate && preferMapped(candidate, best)) best = candidate;
  }
  return best?.p ?? null;
}

/** 展開図の辺を、同じ面の頂点対応で現在の平らに畳んだ紙の線へ写す。 */
export function foldedAlignLine(
  doc: Document,
  faces: Face[],
  frame3d: Frame3D | null,
  edgeId: number,
): [Vec2, Vec2] | null {
  const edge = doc.cp.edges.find((candidate) => candidate.id === edgeId);
  if (!edge) return null;
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const sourceA = byId.get(edge.v0);
  const sourceB = byId.get(edge.v1);
  if (!sourceA || !sourceB) return null;
  if (!frame3d) {
    return [sourceA, sourceB];
  }
  let best: { line: [Vec2, Vec2]; layer: number; faceId: number } | null = null;
  const midpoint: Vec2 = [
    (sourceA[0] + sourceB[0]) / 2,
    (sourceA[1] + sourceB[1]) / 2,
  ];
  for (const map of flatFaceMaps(doc, faces, frame3d)) {
    if (
      !pointInPolygon(sourceA, map.source) ||
      !pointInPolygon(midpoint, map.source) ||
      !pointInPolygon(sourceB, map.source)
    ) {
      continue;
    }
    const a = mapPointInFace(map, sourceA);
    const b = mapPointInFace(map, sourceB);
    const candidate =
      a && b
        ? { line: [a, b] as [Vec2, Vec2], layer: map.layer, faceId: map.faceId }
        : null;
    if (candidate && preferMapped(candidate, best)) best = candidate;
  }
  return best?.line ?? null;
}

function foldedEdgeCursor(
  doc: Document,
  edgeId: number,
  world: Vec2,
  line: [Vec2, Vec2],
): Vec2 {
  const edge = doc.cp.edges.find((candidate) => candidate.id === edgeId);
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const a = edge ? byId.get(edge.v0) : null;
  const b = edge ? byId.get(edge.v1) : null;
  if (!a || !b) return [(line[0][0] + line[1][0]) / 2, (line[0][1] + line[1][1]) / 2];
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len2 = dx * dx + dy * dy;
  const t =
    len2 === 0
      ? 0.5
      : Math.max(0, Math.min(1, ((world[0] - a[0]) * dx + (world[1] - a[1]) * dy) / len2));
  return [
    line[0][0] + (line[1][0] - line[0][0]) * t,
    line[0][1] + (line[1][1] - line[0][1]) * t,
  ];
}

/**
 * 「合わせて折る」で次に必要な対象だけを展開図から拾う。
 * 当たらなかったクリックは通常の2点折りへ流さず、同じ選択段階を保つ。
 */
function pickAlignFromCp(ctx: InteractionCtx, world: Vec2, pickTol: number): void {
  const draft = ctx.alignDraft;
  if (!draft) return;
  const steps = ALIGN_STEPS[draft.mode];
  const need = steps[draft.picks.length % steps.length];
  if (need === "point") {
    const id = pickVisibleAlignVertex(ctx.doc, world, pickTol);
    const point =
      id === null ? null : foldedAlignPoint(ctx.finalDoc, ctx.faces, ctx.frame3d, id);
    if (id !== null && point) {
      ctx.pickAlignTarget(
        { kind: "point", p: point },
        point,
        { kind: "vertex", id },
      );
    }
    return;
  }

  const id = pickEdge(ctx.doc, world, pickTol);
  const line = id === null ? null : foldedAlignLine(ctx.finalDoc, ctx.faces, ctx.frame3d, id);
  if (id !== null && line) {
    ctx.pickAlignTarget(
      { kind: "line", a: line[0], b: line[1] },
      foldedEdgeCursor(ctx.finalDoc, id, world, line),
      { kind: "edge", id },
    );
  }
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
  const paperWorld = pointOnPaper(ctx.finalDoc, world);
  if (paperWorld) ctx.state.lineInputHint = null;
  const snapRadius = SNAP_RADIUS_PX / ctx.view.scale;
  const pickTol = PICK_TOLERANCE_PX / ctx.view.scale;

  const kind = TOOL_KIND[ctx.tool];
  if (kind && ctx.curve.enabled) {
    // 曲線モード(CPE-011): 始点・終点・形を決める点を順にクリックする
    onCurveClick(ctx, snap(ctx.doc, world, snapRadius)?.pos ?? world, kind);
    return;
  }
  if (ctx.tool === "fold" && ctx.alignDraft) {
    // 合わせモード中は、方式ごとの点・線選択を通常の2点折りより優先する。
    pickAlignFromCp(ctx, world, pickTol);
    ctx.state.pendingStart = null;
    ctx.state.directionSnap = null;
    return;
  }
  if (kind || ctx.tool === "fold") {
    // 線ツール・折るツール: 1クリック目=始点、2クリック目=確定
    // 紙外かどうかは吸着させた後の点で決める。角・方眼・輪郭は紙の端にあるため、
    // 生のカーソル位置で先に断ると、1px外れただけで角が選べなくなる(利用者報告)。
    let pos = refreshLineEndpoint(
      ctx,
      kind && paperWorld ? paperWorld : world,
      snapRadius,
    );
    if (kind) {
      // 吸着しても紙外に留まる点だけを断る。灰色の余白には線を引けないままにする。
      const bounded = pointOnPaper(ctx.finalDoc, pos);
      if (!bounded) {
        ctx.state.lineInputHint = OUTSIDE_PAPER_LINE_HINT;
        ctx.state.hoverSnap = null;
        ctx.state.directionSnap = null;
        return;
      }
      pos = bounded;
      ctx.state.lineInputHint = null;
    }
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
    if (hit !== null && pos && !isBorderVertex(ctx.doc, hit)) {
      ctx.state.vertexDrag = {
        id: hit,
        from: pos,
        to: pos,
        snap: null,
      };
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
    if (
      drag.snap === null ||
      drag.snap.doc !== ctx.doc ||
      !sameMoveMirrorAxis(drag.snap.mirrorAxis, ctx.mirrorAxis) ||
      drag.snap.radiusNorm !== radius
    ) {
      drag.snap = prepareMoveSnap(ctx.doc, drag.id, ctx.mirrorAxis, radius);
    }
    drag.to =
      snapForMove(ctx.doc, world, radius, drag.id, ctx.mirrorAxis, drag.snap)?.pos ?? world;
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
    ctx.state.lineInputHint = null;
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
    const visibleEdgeIds = new Set(ctx.doc.cp.edges.map((edge) => edge.id));
    const ids = ctx.selection.edgeIds.filter((id) => visibleEdgeIds.has(id));
    if (ids.length > 0) ctx.applyEdit({ type: "RemoveEdges", ids });
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
