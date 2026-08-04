// カーソル位置の吸着計算(純関数)。IPCを介さずフロント側で毎フレーム計算する。
// 優先順: 既存頂点 > グリッド交点 > 線分上(交点は挿入時に自動で頂点化されるため
// 「既存頂点」に含まれる)。

import type { Document, Vec2 } from "../../lib/types";

export type SnapKind = "vertex" | "grid" | "edge";

export interface SnapResult {
  pos: Vec2;
  kind: SnapKind;
}

/** 紙の正規化サイズ [幅, 高さ](長辺=1.0) */
export function paperExtent(doc: Document): Vec2 {
  const long = Math.max(doc.paper.width_mm, doc.paper.height_mm);
  return [doc.paper.width_mm / long, doc.paper.height_mm / long];
}

function dist(a: Vec2, b: Vec2): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1]);
}

/** 線分ab上でpに最も近い点 */
function closestPointOnSegment(p: Vec2, a: Vec2, b: Vec2): Vec2 {
  const ab: Vec2 = [b[0] - a[0], b[1] - a[1]];
  const len2 = ab[0] * ab[0] + ab[1] * ab[1];
  if (len2 === 0) return [a[0], a[1]];
  const t = ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2;
  const tc = Math.max(0, Math.min(1, t));
  return [a[0] + ab[0] * tc, a[1] + ab[1] * tc];
}

/** 半径内で最も近い既存頂点 */
function snapVertex(doc: Document, cursor: Vec2, radius: number): SnapResult | null {
  let best: Vec2 | null = null;
  let bestDist = radius;
  for (const v of doc.cp.vertices) {
    const d = dist(cursor, v.pos);
    if (d <= bestDist) {
      bestDist = d;
      best = v.pos;
    }
  }
  return best ? { pos: [best[0], best[1]], kind: "vertex" } : null;
}

/** 半径内で最も近いグリッド交点(紙の範囲内、grid_divisions分割) */
function snapGrid(doc: Document, cursor: Vec2, radius: number): SnapResult | null {
  const n = doc.display.grid_divisions;
  if (n <= 0) return null;
  const [w, h] = paperExtent(doc);
  const clamp = (v: number, max: number) => Math.max(0, Math.min(max, v));
  const i = clamp(Math.round((cursor[0] / w) * n), n);
  const j = clamp(Math.round((cursor[1] / h) * n), n);
  const pos: Vec2 = [(i * w) / n, (j * h) / n];
  return dist(cursor, pos) <= radius ? { pos, kind: "grid" } : null;
}

/** 半径内で最も近い線分上の点 */
function snapEdge(doc: Document, cursor: Vec2, radius: number): SnapResult | null {
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  let best: Vec2 | null = null;
  let bestDist = radius;
  for (const e of doc.cp.edges) {
    const a = byId.get(e.v0);
    const b = byId.get(e.v1);
    if (!a || !b) continue; // 参照切れの壊れた線は無視
    const p = closestPointOnSegment(cursor, a, b);
    const d = dist(cursor, p);
    if (d <= bestDist) {
      bestDist = d;
      best = p;
    }
  }
  return best ? { pos: best, kind: "edge" } : null;
}

/**
 * カーソル位置を吸着させる。radiusNormは正規化座標系での吸着半径。
 * 半径内に候補がなければnull(吸着なし)。
 */
export function snap(doc: Document, cursor: Vec2, radiusNorm: number): SnapResult | null {
  return (
    snapVertex(doc, cursor, radiusNorm) ??
    snapGrid(doc, cursor, radiusNorm) ??
    snapEdge(doc, cursor, radiusNorm)
  );
}
