// ねじり折りの「中央多角形」を3D画面のクリックで指定するための純粋な計算。
// 画面もIPCも触らない(ストア・Viewer3D・コンテキストパネルから使う)。
//
// エンジン(ori3-layers::twist)は `polygon` に頂点を順に並べれば、辺の数も
// 長さも仮定せず折れる。ここでは「順にクリックした点の並び」を作る操作と、
// 折る前に見せる下見(多角形と、そこから出るひだの折り線)を用意する。

import type { Vec2 } from "./types";

/** 多角形として使える最少の頂点数(Rust側 polygon_vertices と同じ) */
export const MIN_TWIST_VERTICES = 3;

/** 同じ場所を2度クリックしたとみなす距離(正規化座標。紙の長辺=1.0) */
export const TWIST_POINT_EPS = 1e-6;

/** ねじる角の既定値(度)。正多角形の指し方で出る角(四角形で約27°)に近い値 */
export const DEFAULT_TWIST_DEG = 30;

/** 頂点を1つ足す(すでに同じ場所にある点は足さない) */
export function addTwistVertex(
  poly: readonly Vec2[],
  p: Vec2,
  eps = TWIST_POINT_EPS,
): Vec2[] {
  if (poly.some((q) => Math.hypot(q[0] - p[0], q[1] - p[1]) <= eps)) {
    return [...poly];
  }
  return [...poly, p];
}

/** 直前に足した頂点を取り消す(無ければそのまま) */
export function undoTwistVertex(poly: readonly Vec2[]): Vec2[] {
  return poly.slice(0, Math.max(0, poly.length - 1));
}

/** 中央多角形として使える頂点数がそろったか */
export function isTwistPolygonReady(poly: readonly Vec2[]): boolean {
  return poly.length >= MIN_TWIST_VERTICES;
}

/**
 * 多角形の重心(面積の重心)。頂点が一直線に並ぶなど面積が0のときは
 * 頂点の平均で代用する。頂点が無ければnull。
 */
export function polygonCentroid(poly: readonly Vec2[]): Vec2 | null {
  const n = poly.length;
  if (n === 0) return null;
  let a2 = 0;
  let cx = 0;
  let cy = 0;
  for (let i = 0; i < n; i++) {
    const p = poly[i];
    const q = poly[(i + 1) % n];
    const cross = p[0] * q[1] - q[0] * p[1];
    a2 += cross;
    cx += (p[0] + q[0]) * cross;
    cy += (p[1] + q[1]) * cross;
  }
  if (Math.abs(a2) > 1e-12) return [cx / (3 * a2), cy / (3 * a2)];
  return [
    poly.reduce((s, p) => s + p[0], 0) / n,
    poly.reduce((s, p) => s + p[1], 0) / n,
  ];
}

/** 点cのまわりにpをrad回した点 */
function rotateAbout(p: Vec2, c: Vec2, rad: number): Vec2 {
  const [dx, dy] = [p[0] - c[0], p[1] - c[1]];
  const [cs, sn] = [Math.cos(rad), Math.sin(rad)];
  return [c[0] + dx * cs - dy * sn, c[1] + dx * sn + dy * cs];
}

/**
 * ねじり折りへ渡す「回転量を示す点」。エンジンは中心から見た
 * 「1辺目の中点の向き」から「この点の向き」までの角をねじる角とする。
 * 頂点が2つに満たなければnull。
 */
export function twistReferencePoint(
  poly: readonly Vec2[],
  center: Vec2,
  deg: number,
): Vec2 | null {
  if (poly.length < 2) return null;
  const mid: Vec2 = [
    (poly[0][0] + poly[1][0]) / 2,
    (poly[0][1] + poly[1][1]) / 2,
  ];
  return rotateAbout(mid, center, (deg * Math.PI) / 180);
}

/**
 * 折る前に見せる線分列: 指定した多角形の辺と、各頂点から外へ出るひだの折り線2本
 * (中心から外への放射方向 p と、それを頂点の外角だけ回した向き q。Rust側の
 * `twist_parts` と同じ決め方)。頂点が3つ未満なら、いま置いた点をつなぐ線だけ。
 */
export function twistPreviewSegments(
  poly: readonly Vec2[],
  center: Vec2 | null,
  armScale = 1,
): [Vec2, Vec2][] {
  const n = poly.length;
  const out: [Vec2, Vec2][] = [];
  for (let i = 0; i + 1 < n; i++) out.push([poly[i], poly[i + 1]]);
  if (n < MIN_TWIST_VERTICES) return out;
  out.push([poly[n - 1], poly[0]]);
  const c = center ?? polygonCentroid(poly);
  if (!c) return out;
  const arm =
    armScale * Math.max(...poly.map((p) => Math.hypot(p[0] - c[0], p[1] - c[1])));
  if (!(arm > 0)) return out;
  for (let j = 0; j < n; j++) {
    const v = poly[j];
    const len = Math.hypot(v[0] - c[0], v[1] - c[1]);
    if (len <= 0) continue;
    const p: Vec2 = [(v[0] - c[0]) / len, (v[1] - c[1]) / len];
    const before: Vec2 = [v[0] - poly[(j + n - 1) % n][0], v[1] - poly[(j + n - 1) % n][1]];
    const after: Vec2 = [poly[(j + 1) % n][0] - v[0], poly[(j + 1) % n][1] - v[1]];
    const ext = Math.atan2(
      before[0] * after[1] - before[1] * after[0],
      before[0] * after[0] + before[1] * after[1],
    );
    const q = rotateAbout(p, [0, 0], ext);
    out.push([v, [v[0] + p[0] * arm, v[1] + p[1] * arm]]);
    out.push([v, [v[0] + q[0] * arm, v[1] + q[1] * arm]]);
  }
  return out;
}
