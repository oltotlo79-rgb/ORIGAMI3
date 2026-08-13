// カーソル位置の吸着計算(純関数)。IPCを介さずフロント側で毎フレーム計算する。
// 優先順: 既存頂点 > グリッド交点 > 線分上(交点は挿入時に自動で頂点化されるため
// 「既存頂点」に含まれる)。

import type { Document, Vec2 } from "../../lib/types";
import {
  intersectInfiniteLines,
  intersectDirectionRayWithSegment,
  intersectSegments,
  projectPointToDirectionRay,
} from "../../lib/directionSnap";
import { mirrorPoint, type MirrorLine } from "../../lib/mirror";

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

interface MoveEdge {
  id: number;
  kind: Document["cp"]["edges"][number]["kind"];
  a: Vec2;
  b: Vec2;
}

interface MoveSnapCandidate {
  pos: Vec2;
  firstId: number;
  secondId: number;
}

interface MovePointIndex {
  root: MovePointNode | null;
}

interface MovePointNode {
  candidate: MoveSnapCandidate;
  axis: 0 | 1;
  left: MovePointNode | null;
  right: MovePointNode | null;
}

/** ドラッグ開始時に一度だけ作る、点移動用の正確な吸着候補。 */
export interface PreparedMoveSnap {
  movingId: number;
  doc: Document;
  mirrorAxis: MirrorLine | null;
  radiusNorm: number;
  segmentIntersections: MovePointIndex;
  mirroredVertices: MovePointIndex;
  extendedIntersections: MovePointIndex;
}

const MOVE_POINT_DEDUP_QUANTUM = 1e-12;

/** 同じ対称直線かを値で比べる。画面イベントごとに同値の新しいオブジェクトが作られるため。 */
export function sameMoveMirrorAxis(a: MirrorLine | null, b: MirrorLine | null): boolean {
  return (
    a === b ||
    (a !== null &&
      b !== null &&
      a.p[0] === b.p[0] &&
      a.p[1] === b.p[1] &&
      a.d[0] === b.d[0] &&
      a.d[1] === b.d[1])
  );
}

/** 点移動中も位置が変わらない線だけを、辺ID順で返す。 */
function fixedMoveEdges(doc: Document, movingId: number): MoveEdge[] {
  const byId = new Map(doc.cp.vertices.map((vertex) => [vertex.id, vertex.pos]));
  return doc.cp.edges
    .filter((edge) => edge.v0 !== movingId && edge.v1 !== movingId)
    .map((edge): MoveEdge | null => {
      const a = byId.get(edge.v0);
      const b = byId.get(edge.v1);
      return a && b ? { id: edge.id, kind: edge.kind, a, b } : null;
    })
    .filter((edge): edge is MoveEdge => edge !== null)
    .sort((a, b) => a.id - b.id);
}

function candidateComesFirst(a: MoveSnapCandidate, b: MoveSnapCandidate): boolean {
  return a.firstId < b.firstId || (a.firstId === b.firstId && a.secondId < b.secondId);
}

/** 同じ構成点へ集まる多数の線を、座標の丸め誤差内で1候補にまとめる。 */
function candidatePositionKey(pos: Vec2): string {
  return `${Math.round(pos[0] / MOVE_POINT_DEDUP_QUANTUM)},${Math.round(
    pos[1] / MOVE_POINT_DEDUP_QUANTUM,
  )}`;
}

/** 紙端の丸め誤差だけを端へ戻し、紙外の候補は除外する。 */
function pointOnPaper(pos: Vec2, extent: Vec2): Vec2 | null {
  const epsilon = SNAP_RADIUS_ROUNDING_EPSILON;
  if (
    pos[0] < -epsilon ||
    pos[0] > extent[0] + epsilon ||
    pos[1] < -epsilon ||
    pos[1] > extent[1] + epsilon
  ) {
    return null;
  }
  return [
    Math.max(0, Math.min(extent[0], pos[0])),
    Math.max(0, Math.min(extent[1], pos[1])),
  ];
}

function addMoveCandidate(
  candidates: Map<string, MoveSnapCandidate>,
  candidate: MoveSnapCandidate,
): void {
  const key = candidatePositionKey(candidate.pos);
  const existing = candidates.get(key);
  if (!existing || candidateComesFirst(candidate, existing)) candidates.set(key, candidate);
}

function buildMovePointIndex(
  candidates: Map<string, MoveSnapCandidate>,
): MovePointIndex {
  function build(points: MoveSnapCandidate[], depth: number): MovePointNode | null {
    if (points.length === 0) return null;
    const axis = (depth % 2) as 0 | 1;
    points.sort(
      (a, b) =>
        a.pos[axis] - b.pos[axis] ||
        a.pos[1 - axis] - b.pos[1 - axis] ||
        a.firstId - b.firstId ||
        a.secondId - b.secondId,
    );
    const middle = Math.floor(points.length / 2);
    return {
      candidate: points[middle],
      axis,
      left: build(points.slice(0, middle), depth + 1),
      right: build(points.slice(middle + 1), depth + 1),
    };
  }

  return { root: build([...candidates.values()], 0) };
}

/** 半径内の候補だけを平衡2次元索引から調べ、同距離なら候補元IDで決める。 */
function queryMovePointIndex(
  index: MovePointIndex,
  cursor: Vec2,
  radiusNorm: number,
): Vec2 | null {
  if (radiusNorm < 0) return null;
  const radius = radiusNorm + SNAP_RADIUS_ROUNDING_EPSILON;
  const result: { best: (MoveSnapCandidate & { distance: number }) | null } = {
    best: null,
  };

  function search(node: MovePointNode | null): void {
    if (!node) return;
    const candidate = node.candidate;
    const distance = dist(cursor, candidate.pos);
    if (
      distance <= radius &&
      (result.best === null ||
        distance < result.best.distance ||
        (distance === result.best.distance && candidateComesFirst(candidate, result.best)))
    ) {
      result.best = { ...candidate, distance };
    }

    const delta = cursor[node.axis] - candidate.pos[node.axis];
    const near = delta < 0 ? node.left : node.right;
    const far = delta < 0 ? node.right : node.left;
    search(near);
    const limit = result.best ? Math.min(radius, result.best.distance) : radius;
    if (Math.abs(delta) <= limit) search(far);
  }

  search(index.root);
  return result.best ? [result.best.pos[0], result.best.pos[1]] : null;
}

/**
 * 点移動中に使う交点・対称位置を一度だけ列挙し、12px相当の空間索引へ入れる。
 * 毎回のpointermoveでは辺の全組合せを調べない。
 */
export function prepareMoveSnap(
  doc: Document,
  movingId: number,
  mirrorAxis: MirrorLine | null,
  radiusNorm: number,
): PreparedMoveSnap {
  const segmentCandidates = new Map<string, MoveSnapCandidate>();
  const mirroredCandidates = new Map<string, MoveSnapCandidate>();
  const extendedCandidates = new Map<string, MoveSnapCandidate>();
  const extent = paperExtent(doc);
  const edges = fixedMoveEdges(doc, movingId);

  for (let i = 0; i < edges.length; i += 1) {
    for (let j = i + 1; j < edges.length; j += 1) {
      const first = edges[i];
      const second = edges[j];
      const segment = intersectSegments(first.a, first.b, second.a, second.b);
      if (segment) {
        const pos = pointOnPaper(segment, extent);
        if (pos) {
          addMoveCandidate(segmentCandidates, {
            pos,
            firstId: first.id,
            secondId: second.id,
          });
        }
        continue;
      }
      if (first.kind === "Border" || second.kind === "Border") continue;
      const extended = intersectInfiniteLines(first.a, first.b, second.a, second.b);
      if (!extended) continue;
      const pos = pointOnPaper(extended, extent);
      if (pos) {
        addMoveCandidate(extendedCandidates, {
          pos,
          firstId: first.id,
          secondId: second.id,
        });
      }
    }
  }

  if (mirrorAxis) {
    for (const vertex of doc.cp.vertices) {
      if (vertex.id === movingId) continue;
      const pos = pointOnPaper(mirrorPoint(vertex.pos, mirrorAxis), extent);
      if (pos) {
        addMoveCandidate(mirroredCandidates, {
          pos,
          firstId: vertex.id,
          secondId: Number.POSITIVE_INFINITY,
        });
      }
    }
  }

  return {
    movingId,
    doc,
    mirrorAxis,
    radiusNorm,
    segmentIntersections: buildMovePointIndex(segmentCandidates),
    mirroredVertices: buildMovePointIndex(mirroredCandidates),
    extendedIntersections: buildMovePointIndex(extendedCandidates),
  };
}

/**
 * 点を動かしているときの吸着(CPE-006)。動かしている点そのものと、
 * その点につながる線は一緒に動くので吸着先にしない(自分自身に吸い付かない)。
 * 他の点 > 線分交点 > 対称位置 > 折り目の延長交点 > グリッド交点の順に見る。
 */
export function snapForMove(
  doc: Document,
  cursor: Vec2,
  radiusNorm: number,
  movingId: number,
  mirrorAxis: MirrorLine | null,
  prepared?: PreparedMoveSnap,
): SnapResult | null {
  const vertex = snapVertex(doc, cursor, radiusNorm, movingId);
  if (vertex) return vertex;
  const candidates =
    prepared?.movingId === movingId &&
    prepared.doc === doc &&
    sameMoveMirrorAxis(prepared.mirrorAxis, mirrorAxis) &&
    prepared.radiusNorm === radiusNorm
      ? prepared
      : prepareMoveSnap(doc, movingId, mirrorAxis, radiusNorm);
  const segment = queryMovePointIndex(candidates.segmentIntersections, cursor, radiusNorm);
  if (segment) return { pos: segment, kind: "edge" };
  const mirrored = queryMovePointIndex(candidates.mirroredVertices, cursor, radiusNorm);
  if (mirrored) return { pos: mirrored, kind: "vertex" };
  const extended = queryMovePointIndex(candidates.extendedIntersections, cursor, radiusNorm);
  if (extended) return { pos: extended, kind: "edge" };
  return snapGrid(doc, cursor, radiusNorm);
}
