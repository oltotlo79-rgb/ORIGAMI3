// 「合わせて折る」で3D画面のクリックから点・線を拾う純関数。
// 畳み平面(3D表示のxy)の層の輪郭だけを材料にするので、Three.jsには依存しない。
// 拾える点: 紙の角・展開図の頂点(畳んだ位置)・辺どうしの交点。
// 拾える線: 紙の輪郭の辺・折り線(層の境界としてそのまま現れる)。

import { ALIGN_EPS, type FoldLine } from "./alignFold";
import type { Vec2 } from "./types";

/** 拾い上げの材料(foldDraw.tsのFoldLayerがそのまま渡せる形) */
export interface AlignPickLayer {
  polygon: Vec2[];
}

/** 同じ線分を1本にまとめるときの座標の丸め幅。幾何判定の許容差と揃える。 */
const DEDUPE = 1 / ALIGN_EPS;

function key(p: Vec2): string {
  return `${Math.round(p[0] * DEDUPE)},${Math.round(p[1] * DEDUPE)}`;
}

/** 層の輪郭を線分の集まりにする(重なった層の同じ辺は1本にまとめる) */
export function alignLineCandidates(layers: AlignPickLayer[]): FoldLine[] {
  const seen = new Set<string>();
  const out: FoldLine[] = [];
  for (const l of layers) {
    const poly = l.polygon;
    for (let i = 0; i < poly.length; i++) {
      const a = poly[i];
      const b = poly[(i + 1) % poly.length];
      if (Math.hypot(b[0] - a[0], b[1] - a[1]) < ALIGN_EPS) continue;
      const [k0, k1] = [key(a), key(b)].sort();
      const k = `${k0}|${k1}`;
      if (seen.has(k)) continue;
      seen.add(k);
      out.push([a, b]);
    }
  }
  return out;
}

/** 層の輪郭の頂点(重なった同じ点は1つにまとめる) */
export function alignVertexCandidates(layers: AlignPickLayer[]): Vec2[] {
  const seen = new Set<string>();
  const out: Vec2[] = [];
  for (const l of layers) {
    for (const p of l.polygon) {
      const k = key(p);
      if (seen.has(k)) continue;
      seen.add(k);
      out.push(p);
    }
  }
  return out;
}

/** 点pから線分abまでの距離 */
export function distanceToSegment(p: Vec2, a: Vec2, b: Vec2): number {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len2 = dx * dx + dy * dy;
  const t =
    len2 === 0
      ? 0
      : Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2));
  return Math.hypot(p[0] - (a[0] + dx * t), p[1] - (a[1] + dy * t));
}

/** 2つの線分の交点(平行・線分の外で交わるならnull)。端点で接する場合も拾う */
export function segmentIntersection(
  s1: FoldLine,
  s2: FoldLine,
  eps = 1e-9,
): Vec2 | null {
  const r: Vec2 = [s1[1][0] - s1[0][0], s1[1][1] - s1[0][1]];
  const s: Vec2 = [s2[1][0] - s2[0][0], s2[1][1] - s2[0][1]];
  const den = r[0] * s[1] - r[1] * s[0];
  const scale = Math.hypot(r[0], r[1]) * Math.hypot(s[0], s[1]);
  if (scale < ALIGN_EPS * ALIGN_EPS || Math.abs(den) <= ALIGN_EPS * scale) return null;
  const d: Vec2 = [s2[0][0] - s1[0][0], s2[0][1] - s1[0][1]];
  const t = (d[0] * s[1] - d[1] * s[0]) / den;
  const u = (d[0] * r[1] - d[1] * r[0]) / den;
  if (t < -eps || t > 1 + eps || u < -eps || u > 1 + eps) return null;
  return [s1[0][0] + r[0] * t, s1[0][1] + r[1] * t];
}

/**
 * カーソルにいちばん近い線(半径内に無ければnull)。
 * 折り線として使うのは無限直線なので、線分そのものをそのまま返す。
 */
export function nearestAlignLine(
  layers: AlignPickLayer[],
  cursor: Vec2,
  radius: number,
): FoldLine | null {
  let best: FoldLine | null = null;
  let bestDist = radius;
  for (const seg of alignLineCandidates(layers)) {
    const d = distanceToSegment(cursor, seg[0], seg[1]);
    if (d <= bestDist) {
      bestDist = d;
      best = seg;
    }
  }
  return best;
}

/**
 * カーソルにいちばん近い点(半径内に無ければnull)。
 * 頂点(紙の角・折り目の端)と辺どうしの交点を同じ距離基準で探す。
 * 交点はカーソル周りの辺だけで作るので、辺が何万本あっても重くならない。
 * 頂点を無条件に優先すると、クリックした交点の近くに別の端点がある場合に
 * 交点を選べなくなるため、必ず実距離が短い候補を返す。
 */
export function nearestAlignPoint(
  layers: AlignPickLayer[],
  cursor: Vec2,
  radius: number,
): Vec2 | null {
  let best: Vec2 | null = null;
  let bestDist = radius;
  for (const p of alignVertexCandidates(layers)) {
    const d = Math.hypot(p[0] - cursor[0], p[1] - cursor[1]);
    if (d <= bestDist) {
      bestDist = d;
      best = p;
    }
  }
  const near = alignLineCandidates(layers).filter(
    (seg) => distanceToSegment(cursor, seg[0], seg[1]) <= radius * 2,
  );
  for (let i = 0; i < near.length; i++) {
    for (let j = i + 1; j < near.length; j++) {
      const x = segmentIntersection(near[i], near[j]);
      if (!x) continue;
      const d = Math.hypot(x[0] - cursor[0], x[1] - cursor[1]);
      if (d <= bestDist) {
        bestDist = d;
        best = x;
      }
    }
  }
  return best;
}
