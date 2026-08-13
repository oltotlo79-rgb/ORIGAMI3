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

// 画面座標との相互変換と交点計算で生じる丸めだけを、12px境界で許容する。
// 最大表示倍率100000でも画面上1e-7pxなので、実際の吸着範囲は広げない。
const SNAP_RADIUS_ROUNDING_EPSILON = 1e-12;

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
 * 頂点を最優先にし、実交点が半径内にあるときだけグリッドより先にする。
 * それ以外は通常と同じくグリッドを線分より優先する。
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

  const byId = new Map(doc.cp.vertices.map((candidate) => [candidate.id, candidate.pos]));
  let nearbyIntersection: Vec2 | null = null;
  let nearbyIntersectionDistance = radiusNorm;
  let nearbyIntersectionEdgeId = Number.POSITIVE_INFINITY;
  let edge: Vec2 | null = null;
  let edgeDistance = radiusNorm;
  for (const candidate of doc.cp.edges) {
    const a = byId.get(candidate.v0);
    const b = byId.get(candidate.v1);
    if (!a || !b) continue;
    const intersection = intersectDirectionRayWithSegment(start, direction, a, b);
    if (!intersection) continue;

    // 方向合わせ中だけの例外: 鶴の基本形に必要な x=(√2-1)/2 は無理数で、
    // 方眼座標i/nは有理数なので2〜1024の整数等分では正確に表せない。
    // 実測では0.25へのずれにより、
    // 8本を180°にしたときの指定角が最大105.526°ずれ、実交点なら0°になった。
    // 通常のsnap順は変えず、実交点そのものが画面由来の半径(12px相当)内に
    // ある場合だけ方眼より優先する。
    const intersectionDistance = dist(cursor, intersection);
    if (
      intersectionDistance <= radiusNorm + SNAP_RADIUS_ROUNDING_EPSILON &&
      (nearbyIntersection === null ||
        intersectionDistance < nearbyIntersectionDistance ||
        (intersectionDistance === nearbyIntersectionDistance &&
          candidate.id < nearbyIntersectionEdgeId))
    ) {
      nearbyIntersection = intersection;
      nearbyIntersectionDistance = intersectionDistance;
      nearbyIntersectionEdgeId = candidate.id;
    }

    const cursorDistance = dist(cursor, closestPointOnSegment(cursor, a, b));
    if (cursorDistance <= edgeDistance) {
      edge = intersection;
      edgeDistance = cursorDistance;
    }
  }

  if (nearbyIntersection) return { pos: nearbyIntersection, kind: "edge" };

  const grid = snapGrid(doc, cursor, radiusNorm);
  if (grid) {
    const projected = projectPointToDirectionRay(start, direction, grid.pos);
    if (projected && dist(grid.pos, projected) <= radiusNorm) {
      return { pos: projected, kind: "grid" };
    }
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
