// 畳み平面(z=0)の上に折り線を引くための純粋な幾何計算。
// Three.jsには依存しない(3D区画・展開図区画・ストアのどこからでも使う)。
//
// 座標系: 畳み平面の(x, y) = 3D表示のxy。平らに畳んだ紙は全ての層がz=0に乗るので、
// 立体表示(Frame3D)の多角形をそのまま2Dの層として扱える。

import type { Document, Face, Frame3D, Vec2 } from "../../lib/types";

/** 折り線のどちら側かを判定する余裕(正規化座標) */
const SIDE_EPS = 1e-9;

/** 畳み平面上の1層(面1つ分) */
export interface FoldLayer {
  /** 面ID */
  face: number;
  /** 重なり順(0が下) */
  layer: number;
  /** 畳み平面上の境界多角形 */
  polygon: Vec2[];
}

/**
 * 畳み平面上の層を作る。立体形状(frame)があればそれを使い、
 * まだ折っていない(frame=null)ときは展開図をそのまま平らな1層群として扱う。
 * 高さを持つ面(平らに畳まれていない面)は含めない。
 */
export function foldLayers(
  frame: Frame3D | null,
  doc: Document,
  faces: Face[],
  flatEps = 1e-6,
): FoldLayer[] {
  if (frame) {
    return frame.faces
      .filter((f) => f.polygon.every((p) => Math.abs(p[2]) <= flatEps))
      .map((f) => ({
        face: f.face,
        layer: f.layer,
        polygon: f.polygon.map((p): Vec2 => [p[0], p[1]]),
      }));
  }
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const out: FoldLayer[] = [];
  // 折る前は重なり順=面ID昇順(FlatState::initialと同じ)
  const sorted = [...faces].sort((a, b) => a.id - b.id);
  for (const [i, f] of sorted.entries()) {
    const polygon: Vec2[] = [];
    for (const id of f.vertices) {
      const p = pos.get(id);
      if (!p) break;
      polygon.push(p);
    }
    if (polygon.length === f.vertices.length && polygon.length >= 3) {
      out.push({ face: f.id, layer: i, polygon });
    }
  }
  return out;
}

/** 折り線の方向(単位ベクトル)。長さ0ならnull */
function direction(line: [Vec2, Vec2]): Vec2 | null {
  const dx = line[1][0] - line[0][0];
  const dy = line[1][1] - line[0][1];
  const len = Math.hypot(dx, dy);
  return len < SIDE_EPS ? null : [dx / len, dy / len];
}

/** 折り線から見た点の符号付き距離(正=進行方向の左側) */
function signedSide(line: [Vec2, Vec2], u: Vec2, p: Vec2): number {
  return u[0] * (p[1] - line[0][1]) - u[1] * (p[0] - line[0][0]);
}

/**
 * 「どちら側を動かすか」から、fold_throughへ渡す `keep_side_point`
 * (動かさない側を示す点)を作る。線の進行方向(始点→終点)に対する左右で指定する。
 */
export function keepSidePoint(
  line: [Vec2, Vec2],
  movingSide: "left" | "right",
): Vec2 {
  const u = direction(line) ?? ([1, 0] as Vec2);
  const mid: Vec2 = [
    (line[0][0] + line[1][0]) / 2,
    (line[0][1] + line[1][1]) / 2,
  ];
  const len = Math.hypot(line[1][0] - line[0][0], line[1][1] - line[0][1]);
  // 折り線から十分離れた点にする(向きだけが意味を持つ)
  const off = Math.max(0.01, len * 0.25);
  // 左法線は(-uy, ux)。動かす側の反対へずらす
  const sign = movingSide === "left" ? -1 : 1;
  return [mid[0] - u[1] * off * sign, mid[1] + u[0] * off * sign];
}

/** 折り線の可動側(keep_side_pointの反対側)に一部でも掛かる層 */
export function movingLayers(
  layers: FoldLayer[],
  line: [Vec2, Vec2],
  keepPoint: Vec2,
): FoldLayer[] {
  const u = direction(line);
  if (!u) return [];
  const keep = signedSide(line, u, keepPoint);
  if (Math.abs(keep) <= SIDE_EPS) return [];
  const keepSign = Math.sign(keep);
  return layers.filter((l) =>
    l.polygon.some((p) => keepSign * signedSide(line, u, p) < -SIDE_EPS),
  );
}

/** 可動側に掛かる層のうち、いちばん上の1枚の面ID(無ければnull) */
export function topMovingFace(
  layers: FoldLayer[],
  line: [Vec2, Vec2],
  keepPoint: Vec2,
): number | null {
  let best: FoldLayer | null = null;
  for (const l of movingLayers(layers, line, keepPoint)) {
    if (!best || l.layer > best.layer) best = l;
  }
  return best ? best.face : null;
}

/**
 * 多角形を折り線で切り、可動側に残る部分を返す(Sutherland-Hodgmanの半平面切り取り)。
 * 可動側に何も残らなければ空。凹んだ多角形(スリットのある面)では切り口が
 * つながって見えることがあるが、動く範囲の目安としては十分。
 */
export function clipToMovingSide(
  polygon: Vec2[],
  line: [Vec2, Vec2],
  keepPoint: Vec2,
): Vec2[] {
  const u = direction(line);
  if (!u || polygon.length < 3) return [];
  const keep = signedSide(line, u, keepPoint);
  if (Math.abs(keep) <= SIDE_EPS) return [];
  const keepSign = Math.sign(keep);
  // 可動側を「負」にそろえた符号付き距離
  const d = (p: Vec2) => keepSign * signedSide(line, u, p);
  const out: Vec2[] = [];
  for (let i = 0; i < polygon.length; i++) {
    const a = polygon[i];
    const b = polygon[(i + 1) % polygon.length];
    const da = d(a);
    const db = d(b);
    if (da <= 0) out.push(a);
    if ((da < 0 && db > 0) || (da > 0 && db < 0)) {
      const t = da / (da - db);
      out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
  }
  return out;
}

/**
 * 3Dプレビュー用の線分列: 折り線そのものと、動く側に入る部分の輪郭。
 * `topOnly` ならいちばん上の1枚だけを動く層として示す。
 */
export function foldPreviewSegments(
  layers: FoldLayer[],
  line: [Vec2, Vec2],
  keepPoint: Vec2 | null,
  topOnly = false,
): [Vec2, Vec2][] {
  const out: [Vec2, Vec2][] = [[line[0], line[1]]];
  if (!keepPoint) return out;
  let moving = movingLayers(layers, line, keepPoint);
  if (topOnly) {
    const top = moving.reduce<FoldLayer | null>(
      (best, l) => (!best || l.layer > best.layer ? l : best),
      null,
    );
    moving = top ? [top] : [];
  }
  for (const l of moving) {
    const clipped = clipToMovingSide(l.polygon, line, keepPoint);
    for (let i = 0; i < clipped.length; i++) {
      out.push([clipped[i], clipped[(i + 1) % clipped.length]]);
    }
  }
  return out;
}

/** 線分ab上でpに最も近い点 */
function closestOnSegment(p: Vec2, a: Vec2, b: Vec2): Vec2 {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len2 = dx * dx + dy * dy;
  if (len2 === 0) return a;
  const t = Math.max(
    0,
    Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2),
  );
  return [a[0] + dx * t, a[1] + dy * t];
}

/**
 * 折り線の端点を畳み平面上の紙へ吸着させる。
 * 優先順は展開図エディタと同じ考え方で「既存の点 > 紙の輪郭・折り目の上」。
 * 半径内に候補が無ければ元の位置をそのまま返す。
 */
export function snapFoldPoint(
  layers: FoldLayer[],
  p: Vec2,
  radius: number,
): Vec2 {
  let bestPoint: Vec2 | null = null;
  let bestDist = radius;
  for (const l of layers) {
    for (const q of l.polygon) {
      const d = Math.hypot(q[0] - p[0], q[1] - p[1]);
      if (d <= bestDist) {
        bestDist = d;
        bestPoint = q;
      }
    }
  }
  if (bestPoint) return bestPoint;

  bestDist = radius;
  for (const l of layers) {
    for (let i = 0; i < l.polygon.length; i++) {
      const q = closestOnSegment(p, l.polygon[i], l.polygon[(i + 1) % l.polygon.length]);
      const d = Math.hypot(q[0] - p[0], q[1] - p[1]);
      if (d <= bestDist) {
        bestDist = d;
        bestPoint = q;
      }
    }
  }
  return bestPoint ?? p;
}
