// 層のずらし表示(UI-010 / SIM-004)の計算。Three.jsには依存しない純粋な計算。
//
// 平らに畳んだ紙は全ての層がz=0に重なるため、そのまま描くと紙が1枚に見えてしまい、
// 「どの層を選んでいるのか」が画面から読み取れない(要件の設計原則3b: 直感的に触れること)。
// そこで平らな状態のときだけ、層ごとに微小な高さを付けて重なりを見せる。
//
// ここで作る値は表示専用で、作品データ(Frame3D)そのものには一切反映しない。

import type { Frame3D } from "./types";

/** 層1枚あたりのずらし量(紙の長辺に対する割合) */
export const LAYER_STEP_RATIO = 0.01;

/** 重なり全体の厚みの上限(紙の長辺に対する割合)。層が多くても分厚く見せない */
export const MAX_STACK_RATIO = 0.03;

/** これ以下の高さは計算誤差とみなして「平ら」と扱う(foldDrawの判定と揃える) */
const FLAT_EPS = 1e-6;

/**
 * 層ごとのずらし量(下から0番)を返す。
 * 間隔は「紙の長辺 × LAYER_STEP_RATIO」を基本とし、層が多いときは重なり全体の
 * 厚みが MAX_STACK_RATIO を超えないよう間隔を詰める。
 */
export function layerOffsets(layerCount: number, paperScale: number): number[] {
  const count = Math.floor(layerCount);
  if (!Number.isFinite(count) || count <= 0) return [];
  const scale = Number.isFinite(paperScale) && paperScale > 0 ? paperScale : 0;
  const gaps = count - 1;
  const step =
    gaps === 0
      ? 0
      : Math.min(LAYER_STEP_RATIO, MAX_STACK_RATIO / gaps) * scale;
  const out = new Array<number>(count);
  for (let i = 0; i < count; i++) out[i] = i * step;
  return out;
}

/**
 * 平らに畳んだ状態か(全ての面が同じ平面=z≒0に乗っているか)。
 * 折り途中や立体的な形にずらしを掛けると形が歪むので、この判定を通ったときだけ使う。
 */
export function isFlatFrame(frame: Frame3D, eps = FLAT_EPS): boolean {
  if (frame.faces.length === 0) return false;
  return frame.faces.every((f) => f.polygon.every((p) => Math.abs(p[2]) <= eps));
}

/** 重なりの枚数(層番号の最大+1。層は下から0) */
export function frameLayerCount(frame: Frame3D): number {
  let max = -1;
  for (const f of frame.faces) {
    if (f.layer > max) max = f.layer;
  }
  return max + 1;
}

// ---------------------------------------------------------------------------
// 重なった面のずらし(平らな状態に限らない)
// ---------------------------------------------------------------------------

export type Vec3 = [number, number, number];

/** 同じ平面に乗っているとみなす向き・位置のずれ */
const PLANE_EPS = 1e-4;
/** 面積がこれ以下の面は向きが決まらないので仲間分けから外す */
const AREA_EPS = 1e-12;

/** 多角形の法線(Newell法)。長さは面積の2倍。単位ベクトルに直して返す */
function polygonNormal(poly: readonly Vec3[]): Vec3 | null {
  let x = 0;
  let y = 0;
  let z = 0;
  for (let i = 0; i < poly.length; i++) {
    const a = poly[i];
    const b = poly[(i + 1) % poly.length];
    x += (a[1] - b[1]) * (a[2] + b[2]);
    y += (a[2] - b[2]) * (a[0] + b[0]);
    z += (a[0] - b[0]) * (a[1] + b[1]);
  }
  const len = Math.hypot(x, y, z);
  if (!Number.isFinite(len) || len <= AREA_EPS) return null;
  return [x / len, y / len, z / len];
}

/**
 * 最大成分が正になるよう符号をそろえる。裏返った面も同じ平面の仲間として扱え、
 * 積み上げる向きが表裏に左右されなくなる(Rust側 ori3-soft::canonical_axis と同じ規約)。
 */
function canonicalAxis(n: Vec3): Vec3 {
  const a = [Math.abs(n[0]), Math.abs(n[1]), Math.abs(n[2])];
  const k = a[0] >= a[1] && a[0] >= a[2] ? 0 : a[1] >= a[2] ? 1 : 2;
  return n[k] < 0 ? [-n[0], -n[1], -n[2]] : n;
}

/** 面が乗っている平面(そろえた法線と、原点からの符号付き距離) */
function facePlane(poly: readonly Vec3[]): { n: Vec3; d: number } | null {
  const raw = polygonNormal(poly);
  if (raw === null) return null;
  const n = canonicalAxis(raw);
  let d = 0;
  for (const p of poly) d += n[0] * p[0] + n[1] * p[1] + n[2] * p[2];
  return { n, d: d / poly.length };
}

/**
 * 面ごとのずらしベクトル(表示専用。frame.faces と同じ並び)を返す。
 *
 * 平らに畳んだ状態だけでなく、折り途中・立体・引っ張った状態でも
 * 「同じ平面に重なった面のかたまり」ごとに層番号の順でその平面の法線方向へ離す。
 * 完全に重なった面の深度が同値のままだと、後から描かれる裏面が表の色を
 * 塗りつぶしてしまうため(実機で見つかった不具合)。
 *
 * 層番号が同じ面は離さない(展開した1枚の紙がばらばらに浮かないように)。
 */
export function stackLifts(frame: Frame3D, paperScale: number): Vec3[] {
  const faces = frame.faces;
  const lifts: Vec3[] = faces.map(() => [0, 0, 0]);
  const planes = faces.map((f) => facePlane(f.polygon));

  // 同じ平面どうしを貪欲に仲間分けする(面の枚数は多くないので総当たりで足りる)
  const groups: { n: Vec3; d: number; members: number[] }[] = [];
  for (let i = 0; i < faces.length; i++) {
    const p = planes[i];
    if (p === null) continue;
    const g = groups.find(
      (q) =>
        Math.abs(q.d - p.d) <= PLANE_EPS &&
        Math.abs(q.n[0] - p.n[0]) <= PLANE_EPS &&
        Math.abs(q.n[1] - p.n[1]) <= PLANE_EPS &&
        Math.abs(q.n[2] - p.n[2]) <= PLANE_EPS,
    );
    if (g) g.members.push(i);
    else groups.push({ n: p.n, d: p.d, members: [i] });
  }

  // かたまりごとに層番号を下から詰め直す(番号が飛んでいても等間隔に見せる)
  const ranks = new Array<number>(faces.length).fill(0);
  let depth = 1;
  for (const g of groups) {
    const sorted = [...new Set(g.members.map((i) => faces[i].layer))].sort(
      (a, b) => a - b,
    );
    for (const i of g.members) ranks[i] = sorted.indexOf(faces[i].layer);
    if (sorted.length > depth) depth = sorted.length;
  }

  // 間隔は最も厚い重なりに合わせて全体でそろえる
  const steps = layerOffsets(depth, paperScale);
  for (const g of groups) {
    for (const i of g.members) {
      const s = steps[ranks[i]] ?? 0;
      lifts[i] = [g.n[0] * s, g.n[1] * s, g.n[2] * s];
    }
  }
  return lifts;
}
