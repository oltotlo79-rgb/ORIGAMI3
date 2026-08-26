// 「紙をつかんで動かす」折り操作の幾何(UI-007 / UI-008)。
// つかんだ点Aから離した点Bへのドラッグを、
//   折り線 = AとBの垂直二等分線(近くに既存の折り目・紙の縁があればそちらへ吸着)
//   動かさない側 = 離した点B(つかんだ紙がBの位置へ倒れてくる)
// という折る指示に翻訳する。Three.jsには依存しない純粋な計算。

import type { Face, Vec2 } from "../../lib/types";
import {
  clipToMovingSide,
  facesAtPoint,
  movingLayers,
  type FoldLayer,
} from "./foldDraw";

const EPS = 1e-9;

/** これ未満のドラッグは折る指示とみなさない(正規化座標=紙の長辺1.0) */
export const MIN_DRAG = 0.02;
/** 既存の折り目・紙の縁へ吸着させる向きのずれの許容(ラジアン) */
const SNAP_ANGLE = (12 * Math.PI) / 180;
/** 吸着の位置ずれ許容(AとBの距離に対する割合。0なら二等分線ちょうど) */
const SNAP_BALANCE = 0.3;

/** 何枚の紙を動かすか。flap=つかんだひとかたまり / all=重なった紙を全部 / single=1枚だけ */
export type LegacyGrabMode = "flap" | "all" | "single";

/**
 * Future selection contract for fold lines created by an alignment operation.
 * `topPleatCount` is resolved by Rust from the document; never derive target
 * faces from `selectedLayerCount`, `2 * K`, face-id order, or `surface_rank`.
 */
export type GrabSelection =
  | { readonly mode: LegacyGrabMode; readonly topPleatCount?: never }
  | { readonly mode: "topPleats"; readonly topPleatCount: number };

/** Raw 3D dragging intentionally keeps its existing three runtime modes. */
export type GrabMode = LegacyGrabMode;

/** 単位ベクトル(長さ0ならnull) */
function unit(a: Vec2, b: Vec2): Vec2 | null {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len = Math.hypot(dx, dy);
  return len < EPS ? null : [dx / len, dy / len];
}

/** 向きuの直線(基点p0)から見た点の符号付き距離(正=進行方向の左側) */
function side(p0: Vec2, u: Vec2, p: Vec2): number {
  return u[0] * (p[1] - p0[1]) - u[1] * (p[0] - p0[0]);
}

/** 層全体を横切るのに十分な長さ(表示用。折り線は無限直線として扱われる) */
export function layerSpan(layers: FoldLayer[], atLeast = 0): number {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const l of layers) {
    for (const p of l.polygon) {
      minX = Math.min(minX, p[0]);
      minY = Math.min(minY, p[1]);
      maxX = Math.max(maxX, p[0]);
      maxY = Math.max(maxY, p[1]);
    }
  }
  const diag = Number.isFinite(minX) ? Math.hypot(maxX - minX, maxY - minY) : 1;
  return Math.max(diag, atLeast, 0.1);
}

/** AとBの垂直二等分線(中点から左右へspanずつ伸ばす) */
export function bisectorLine(a: Vec2, b: Vec2, span: number): [Vec2, Vec2] | null {
  const u = unit(a, b);
  if (!u) return null;
  const mid: Vec2 = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
  const perp: Vec2 = [-u[1], u[0]];
  return [
    [mid[0] - perp[0] * span, mid[1] - perp[1] * span],
    [mid[0] + perp[0] * span, mid[1] + perp[1] * span],
  ];
}

/**
 * 折り線を決める。基本は垂直二等分線で、既存の折り目・紙の縁が
 * 「向きも位置も近い」ときだけそちらへ吸着させる。
 * 候補はAとBを必ず隔てる線に限る(隔てない線で折ってもつかんだ紙は動かない)。
 */
export function snapFoldLine(
  layers: FoldLayer[],
  a: Vec2,
  b: Vec2,
  span: number,
): [Vec2, Vec2] | null {
  const base = bisectorLine(a, b, span);
  if (!base) return null;
  const baseDir = unit(base[0], base[1]);
  const drag = Math.hypot(b[0] - a[0], b[1] - a[1]);
  if (!baseDir || drag < EPS) return base;
  const mid: Vec2 = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
  let best: [Vec2, Vec2] | null = null;
  let bestScore = Infinity;
  for (const l of layers) {
    for (let i = 0; i < l.polygon.length; i++) {
      const p = l.polygon[i];
      const q = l.polygon[(i + 1) % l.polygon.length];
      const eu = unit(p, q);
      if (!eu) continue;
      const da = side(p, eu, a);
      const db = side(p, eu, b);
      if (da * db >= 0) continue; // AとBを隔てていない
      const angle = Math.acos(Math.min(1, Math.abs(eu[0] * baseDir[0] + eu[1] * baseDir[1])));
      if (angle > SNAP_ANGLE) continue;
      const balance = Math.abs(Math.abs(da) - Math.abs(db)) / drag;
      if (balance > SNAP_BALANCE) continue;
      const score = angle / SNAP_ANGLE + balance / SNAP_BALANCE;
      if (score >= bestScore) continue;
      bestScore = score;
      const t = (mid[0] - p[0]) * eu[0] + (mid[1] - p[1]) * eu[1];
      const foot: Vec2 = [p[0] + eu[0] * t, p[1] + eu[1] * t];
      best = [
        [foot[0] - eu[0] * span, foot[1] - eu[1] * span],
        [foot[0] + eu[0] * span, foot[1] + eu[1] * span],
      ];
    }
  }
  return best ?? base;
}

/**
 * つかんだ面と一緒に動く「ひとかたまり(フラップ)」の面ID。
 *
 * 一緒に動くのは、折り線の動く側に掛かっていて、かつ動く側にある折り目・縁で
 * つながっている面。折り線より手前(動かさない側)でつながっている面は、
 * そこが蝶番になるので付いてこない。実際の紙で「つまんだ紙をめくる」ときに
 * 一緒に持ち上がる範囲と同じ考え方。
 */
export function flapFaces(
  layers: FoldLayer[],
  faces: Face[],
  startFace: number,
  line: [Vec2, Vec2],
  keepPoint: Vec2,
): number[] {
  const u = unit(line[0], line[1]);
  if (!u) return [];
  const moving = new Set(movingLayers(layers, line, keepPoint).map((l) => l.face));
  if (!moving.has(startFace)) return [];
  const keepSign = Math.sign(side(line[0], u, keepPoint));
  /** 折り線の動く側に少しでも掛かっているか */
  const reachesMovingSide = (seg: [Vec2, Vec2]) =>
    seg.some((p) => keepSign * side(line[0], u, p) < -EPS);

  const polygons = new Map(layers.map((l) => [l.face, l.polygon]));
  // 辺ID → その辺を境界に持つ面ID(畳んだ平面での線分つき)
  const shared = new Map<number, { seg: [Vec2, Vec2]; faces: number[] }>();
  for (const f of faces) {
    const poly = polygons.get(f.id);
    if (!moving.has(f.id) || !poly || poly.length !== f.edges.length) continue;
    for (let i = 0; i < poly.length; i++) {
      const seg: [Vec2, Vec2] = [poly[i], poly[(i + 1) % poly.length]];
      const entry = shared.get(f.edges[i]);
      if (entry) entry.faces.push(f.id);
      else shared.set(f.edges[i], { seg, faces: [f.id] });
    }
  }
  const neighbors = new Map<number, number[]>();
  for (const { seg, faces: ids } of shared.values()) {
    if (ids.length < 2 || !reachesMovingSide(seg)) continue;
    for (const id of ids) {
      const list = neighbors.get(id) ?? [];
      list.push(...ids.filter((o) => o !== id));
      neighbors.set(id, list);
    }
  }
  const out = new Set<number>([startFace]);
  const queue = [startFace];
  while (queue.length > 0) {
    for (const next of neighbors.get(queue.pop()!) ?? []) {
      if (!out.has(next)) {
        out.add(next);
        queue.push(next);
      }
    }
  }
  return [...out].sort((x, y) => x - y);
}

/** 点を折り線で鏡映する(折った後に紙が着地する位置) */
export function reflectPoint(p: Vec2, line: [Vec2, Vec2]): Vec2 {
  const u = unit(line[0], line[1]);
  if (!u) return p;
  const d = side(line[0], u, p);
  return [p[0] + u[1] * 2 * d, p[1] - u[0] * 2 * d];
}

/** ドラッグから組み立てた折る指示と、その見た目(実行前プレビュー) */
export interface GrabFoldPlan {
  line: [Vec2, Vec2];
  /** 動かさない側を示す点(=離した点) */
  keepSidePoint: Vec2;
  /** 折る対象の面ID。nullは「動く側に掛かる全ての層」 */
  targetLayers: number[] | null;
  /** 通常grabで実際に選ばれた層数。完全折りの「ひだ数」ではない。 */
  selectedLayerCount: number;
  /** 折った後に紙が着地する多角形(半透明で重ねて見せる) */
  preview: Vec2[][];
  /** 折り線と、これから動く範囲の輪郭 */
  segments: [Vec2, Vec2][];
}

export type GrabFoldResult =
  | { ok: true; plan: GrabFoldPlan }
  | { ok: false; error: string };

/**
 * つかんだ点Aから離した点Bへのドラッグを折る指示に翻訳する。
 * `grabFace` は立体表示のraycastで拾った面ID(取れなければAの位置のいちばん上の層)。
 * 動く層の決め方はバックエンド(fold_through)と同じ「折り線の動く側に掛かる層」。
 */
export function planGrabFold(
  layers: FoldLayer[],
  faces: Face[],
  a: Vec2,
  b: Vec2,
  mode: GrabMode,
  grabFace: number | null = null,
): GrabFoldResult {
  if (layers.length === 0) return { ok: false, error: "折れる紙がありません" };
  if (Math.hypot(b[0] - a[0], b[1] - a[1]) < MIN_DRAG) {
    return { ok: false, error: "動かす向きへもう少し長くドラッグしてください" };
  }
  const line = snapFoldLine(layers, a, b, layerSpan(layers));
  if (!line) return { ok: false, error: "折り線を決められませんでした" };
  const keepSidePoint = b;
  const moving = movingLayers(layers, line, keepSidePoint);
  if (moving.length === 0) {
    return { ok: false, error: "その向きへ動かせる紙がありません" };
  }
  let targetLayers: number[] | null = null;
  if (mode !== "all") {
    const movingIds = new Set(moving.map((l) => l.face));
    const stack = facesAtPoint(layers, a, 1e-3).filter((id) => movingIds.has(id));
    const start =
      grabFace !== null && movingIds.has(grabFace)
        ? grabFace
        : (stack[stack.length - 1] ?? null);
    if (start === null) {
      return { ok: false, error: "つかんだ場所に動かせる紙がありません" };
    }
    const flap = mode === "single" ? [start] : flapFaces(layers, faces, start, line, keepSidePoint);
    targetLayers = flap.length > 0 ? flap : [start];
  }
  const targets = targetLayers;
  const shown = targets === null ? moving : moving.filter((l) => targets.includes(l.face));
  const preview: Vec2[][] = [];
  const segments: [Vec2, Vec2][] = [[line[0], line[1]]];
  for (const l of shown) {
    const clipped = clipToMovingSide(l.polygon, line, keepSidePoint);
    if (clipped.length < 3) continue;
    for (let i = 0; i < clipped.length; i++) {
      segments.push([clipped[i], clipped[(i + 1) % clipped.length]]);
    }
    preview.push(clipped.map((p) => reflectPoint(p, line)));
  }
  return {
    ok: true,
    plan: {
      line,
      keepSidePoint,
      targetLayers,
      selectedLayerCount: shown.length,
      preview,
      segments,
    },
  };
}
