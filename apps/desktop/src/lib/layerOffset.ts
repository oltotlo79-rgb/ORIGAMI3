// 層のずらし表示(UI-010 / SIM-004)の計算。Three.jsには依存しない純粋な計算。
//
// 平らに畳んだ紙は全ての層がz=0に重なるため、そのまま描くと紙が1枚に見えてしまい、
// 「どの層を選んでいるのか」が画面から読み取れない(要件の設計原則3b: 直感的に触れること)。
// そこで平らな状態のときだけ、層ごとに微小な高さを付けて重なりを見せる。
//
// ここで作る値は表示専用で、作品データ(Frame3D)そのものには一切反映しない。

import type { Face3D, Frame3D } from "./types";

/** 層1枚あたりのずらし量(紙の長辺に対する割合)
 *
 * 実際の折り紙の紙の厚さは0とみなす(利用者の指示)。重なった紙が離れて見えると
 * 実物と違うため、目で見て分からない大きさに抑える。0にはしない。完全に同じ位置に
 * 描くと、どちらの面が手前か決まらず、内側の面や折り目が表面に透けて見えるため。 */
export const LAYER_STEP_RATIO = 0.0002;

/** 重なり全体の厚みの上限(紙の長辺に対する割合)。層が多くても分厚く見せない */
export const MAX_STACK_RATIO = 0.001;

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
/** 面の重心(平面上の点で判定するのに使う)。 */
function centroid(poly: readonly Vec3[]): Vec3 {
  const s: Vec3 = [0, 0, 0];
  for (const p of poly) {
    s[0] += p[0];
    s[1] += p[1];
    s[2] += p[2];
  }
  const n = Math.max(poly.length, 1);
  return [s[0] / n, s[1] / n, s[2] / n];
}

/**
 * 同じ平面にある2つの面が重なっているか。
 *
 * 重心が相手の内側にあるかを、面の法線を軸から外した2方向へ落として調べる。
 * 重なった紙の層を見分けるための判定なので、厳密な多角形の交差までは要らない。
 */
function overlapsInPlane(a: Face3D, b: Face3D): boolean {
  const n = polygonNormal(a.polygon);
  if (n === null) return false;
  const axis = canonicalAxis(n);
  const u: Vec3 = [
    n[1] * axis[2] - n[2] * axis[1],
    n[2] * axis[0] - n[0] * axis[2],
    n[0] * axis[1] - n[1] * axis[0],
  ];
  const ul = Math.hypot(u[0], u[1], u[2]);
  if (ul < AREA_EPS) return false;
  const uu: Vec3 = [u[0] / ul, u[1] / ul, u[2] / ul];
  const vv: Vec3 = [
    n[1] * uu[2] - n[2] * uu[1],
    n[2] * uu[0] - n[0] * uu[2],
    n[0] * uu[1] - n[1] * uu[0],
  ];
  const to2 = (p: Vec3): [number, number] => [
    p[0] * uu[0] + p[1] * uu[1] + p[2] * uu[2],
    p[0] * vv[0] + p[1] * vv[1] + p[2] * vv[2],
  ];
  const inside = (poly: readonly Vec3[], q: [number, number]): boolean => {
    const pts = poly.map(to2);
    let hit = false;
    for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
      const [xi, yi] = pts[i];
      const [xj, yj] = pts[j];
      if (yi > q[1] !== yj > q[1]) {
        const x = xi + ((q[1] - yi) / (yj - yi)) * (xj - xi);
        if (q[0] < x) hit = !hit;
      }
    }
    return hit;
  };
  return (
    inside(a.polygon, to2(centroid(b.polygon))) ||
    inside(b.polygon, to2(centroid(a.polygon)))
  );
}

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
    // 手順を記録せず角度だけで折ると、全ての面が同じ段になり離す幅が0になる。
    // すると重なった紙が完全に同じ位置へ描かれ、内側の折り目が表面から透けて
    // 見える。同じ段でも「互いに重なっている」面は離す。展開した1枚の紙は
    // 面どうしが重ならないので、これまでどおりばらけない。
    if (sorted.length === 1 && g.members.length > 1) {
      const stacked = g.members.filter((i) =>
        g.members.some((j) => j !== i && overlapsInPlane(faces[i], faces[j])),
      );
      stacked.forEach((i, index) => {
        ranks[i] = index;
      });
      if (stacked.length > depth) depth = stacked.length;
    }
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
