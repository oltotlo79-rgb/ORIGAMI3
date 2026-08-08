// カーソル位置の吸着計算(純関数)。IPCを介さずフロント側で毎フレーム計算する。
// 優先順: 既存頂点 > グリッド交点 > 線分上(交点は挿入時に自動で頂点化されるため
// 「既存頂点」に含まれる)。

import type { Document, Vec2 } from "../../lib/types";
import {
  intersectDirectionRayWithSegment,
  projectPointToDirectionRay,
} from "../../lib/directionSnap";

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

/** 半径内で最も近い既存頂点(excludeIdの点は無視する) */
function snapVertex(
  doc: Document,
  cursor: Vec2,
  radius: number,
  excludeId?: number,
): SnapResult | null {
  let best: Vec2 | null = null;
  let bestDist = radius;
  for (const v of doc.cp.vertices) {
    if (v.id === excludeId) continue;
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

/**
 * 方向吸着を保ったまま、近くの頂点・グリッド交点・線分へ吸着する。
 * 点は半直線へ垂直投影し、線分は半直線との交点を使うため、結果は必ず軸上にある。
 * 候補の優先順位は通常の吸着と同じく、頂点 > グリッド > 線分とする。
 */
export function snapOnDirectionAxis(
  doc: Document,
  start: Vec2,
  direction: Vec2,
  cursor: Vec2,
  radiusNorm: number,
): SnapResult | null {
  let vertex: Vec2 | null = null;
  let vertexDistance = radiusNorm;
  for (const candidate of doc.cp.vertices) {
    const cursorDistance = dist(cursor, candidate.pos);
    if (cursorDistance > vertexDistance) continue;
    const projected = projectPointToDirectionRay(start, direction, candidate.pos);
    if (!projected || dist(candidate.pos, projected) > radiusNorm) continue;
    vertex = projected;
    vertexDistance = cursorDistance;
  }
  if (vertex) return { pos: vertex, kind: "vertex" };

  const grid = snapGrid(doc, cursor, radiusNorm);
  if (grid) {
    const projected = projectPointToDirectionRay(start, direction, grid.pos);
    if (projected && dist(grid.pos, projected) <= radiusNorm) {
      return { pos: projected, kind: "grid" };
    }
  }

  const byId = new Map(doc.cp.vertices.map((candidate) => [candidate.id, candidate.pos]));
  let edge: Vec2 | null = null;
  let edgeDistance = radiusNorm;
  for (const candidate of doc.cp.edges) {
    const a = byId.get(candidate.v0);
    const b = byId.get(candidate.v1);
    if (!a || !b) continue;
    const cursorDistance = dist(cursor, closestPointOnSegment(cursor, a, b));
    if (cursorDistance > edgeDistance) continue;
    const intersection = intersectDirectionRayWithSegment(start, direction, a, b);
    if (!intersection) continue;
    edge = intersection;
    edgeDistance = cursorDistance;
  }
  return edge ? { pos: edge, kind: "edge" } : null;
}

/**
 * 点を動かしているときの吸着(CPE-006)。動かしている点そのものと、
 * その点につながる線は一緒に動くので吸着先にしない(自分自身に吸い付かない)。
 * 他の点 > グリッド交点の順に見る。
 */
export function snapForMove(
  doc: Document,
  cursor: Vec2,
  radiusNorm: number,
  movingId: number,
): SnapResult | null {
  return (
    snapVertex(doc, cursor, radiusNorm, movingId) ??
    snapGrid(doc, cursor, radiusNorm)
  );
}
