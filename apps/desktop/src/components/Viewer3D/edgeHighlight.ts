// 2Dで選んだ辺を、現在3Dビューに表示している位置へ写す純粋な幾何計算。
// Three.jsのオブジェクトは受け取らず、sceneBuilderが更新済みの頂点バッファだけを読む。

import type { Vec3 } from "../../lib/layerOffset";
import type { Document, Face, Vec2 } from "../../lib/types";

/** 面IDから、表示中の頂点バッファ上の範囲を引くための最小インターフェース。 */
export interface FacePositionSlot {
  readonly offset: number;
  readonly count: number;
}

/** ヒンジは従来色、それ以外は「選択中だが操作対象外」と分かる別色で描く。 */
export type EdgeHighlightRole = "hinge" | "reference";

/** Three.jsに依存しない、選択辺1本分の強調表示対象。 */
export interface EdgeHighlightTarget {
  edgeId: number;
  role: EdgeHighlightRole;
  a: Vec3;
  b: Vec3;
}

interface Segment3D {
  a: Vec3;
  b: Vec3;
}

interface FacePlacement {
  polygon: Vec2[];
  p0: Vec2;
  e1: Vec2;
  e2: Vec2;
  q0: Vec3;
  f1: Vec3;
  f2: Vec3;
  det: number;
}

/** 面内判定で境界も含める余裕。展開図は長辺=1の正規化座標。 */
const POINT_EPS = 1e-9;
/** 2Dの基底として使えない、ほぼ一直線な3点を除くしきい値。 */
const AFFINE_EPS = 1e-12;

function finite2(p: Vec2): boolean {
  return Number.isFinite(p[0]) && Number.isFinite(p[1]);
}

function finite3(p: Vec3): boolean {
  return Number.isFinite(p[0]) && Number.isFinite(p[1]) && Number.isFinite(p[2]);
}

function readPosition(positions: ArrayLike<number>, index: number): Vec3 | null {
  if (!Number.isInteger(index) || index < 0) return null;
  const at = index * 3;
  if (at + 2 >= positions.length) return null;
  const p: Vec3 = [positions[at], positions[at + 1], positions[at + 2]];
  return finite3(p) ? p : null;
}

function validSlot(slot: FacePositionSlot | undefined, face: Face): slot is FacePositionSlot {
  return (
    slot !== undefined &&
    Number.isInteger(slot.offset) &&
    slot.offset >= 0 &&
    Number.isInteger(slot.count) &&
    slot.count >= 3 &&
    slot.count === face.vertices.length &&
    slot.count === face.edges.length
  );
}

/**
 * Face.edgesに現れる辺は、頂点バッファから両端を直接読む。
 * 共有辺やスリットで複数回現れる場合は、facesの走査順で最初の有効なものを使う。
 */
function boundarySegment(
  edgeId: number,
  faces: readonly Face[],
  slots: ReadonlyMap<number, FacePositionSlot>,
  positions: ArrayLike<number>,
): Segment3D | null {
  for (const face of faces) {
    const slot = slots.get(face.id);
    if (!validSlot(slot, face)) continue;
    const i = face.edges.indexOf(edgeId);
    if (i < 0) continue;
    const a = readPosition(positions, slot.offset + i);
    const b = readPosition(positions, slot.offset + ((i + 1) % slot.count));
    if (a && b) return { a, b };
  }
  return null;
}

function cross2(a: Vec2, b: Vec2): number {
  return a[0] * b[1] - a[1] * b[0];
}

function sub2(a: Vec2, b: Vec2): Vec2 {
  return [a[0] - b[0], a[1] - b[1]];
}

function sub3(a: Vec3, b: Vec3): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}

/** 多角形の内部。境界からeps以内もその面に含める。 */
function pointInPolygon(poly: readonly Vec2[], p: Vec2, eps = POINT_EPS): boolean {
  if (poly.length < 3) return false;
  const eps2 = eps * eps;
  for (let i = 0; i < poly.length; i++) {
    const a = poly[i];
    const b = poly[(i + 1) % poly.length];
    const dx = b[0] - a[0];
    const dy = b[1] - a[1];
    const len2 = dx * dx + dy * dy;
    const t =
      len2 === 0
        ? 0
        : Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2));
    const qx = a[0] + dx * t;
    const qy = a[1] + dy * t;
    if ((p[0] - qx) ** 2 + (p[1] - qy) ** 2 <= eps2) return true;
  }

  let inside = false;
  for (let i = 0; i < poly.length; i++) {
    const a = poly[i];
    const b = poly[(i + 1) % poly.length];
    if (a[1] > p[1] !== b[1] > p[1]) {
      const t = (p[1] - a[1]) / (b[1] - a[1]);
      if (p[0] < a[0] + t * (b[0] - a[0])) inside = !inside;
    }
  }
  return inside;
}

/** 面の2D頂点と現在の3D頂点から、面内のアフィン写像を復元する。 */
function facePlacement(
  face: Face,
  vertexPositions: ReadonlyMap<number, Vec2>,
  slots: ReadonlyMap<number, FacePositionSlot>,
  positions: ArrayLike<number>,
): FacePlacement | null {
  const slot = slots.get(face.id);
  if (!validSlot(slot, face)) return null;

  const polygon: Vec2[] = [];
  const solid: Vec3[] = [];
  for (let i = 0; i < face.vertices.length; i++) {
    const p = vertexPositions.get(face.vertices[i]);
    const q = readPosition(positions, slot.offset + i);
    if (!p || !finite2(p) || !q) return null;
    polygon.push(p);
    solid.push(q);
  }

  const p0 = polygon[0];
  const q0 = solid[0];
  // 先頭点から見て一次独立な2点を選ぶ。途中に一直線の頂点があっても構わない。
  for (let i = 1; i < polygon.length; i++) {
    for (let j = i + 1; j < polygon.length; j++) {
      const e1 = sub2(polygon[i], p0);
      const e2 = sub2(polygon[j], p0);
      const det = cross2(e1, e2);
      if (Math.abs(det) <= AFFINE_EPS) continue;
      return {
        polygon,
        p0,
        e1,
        e2,
        q0,
        f1: sub3(solid[i], q0),
        f2: sub3(solid[j], q0),
        det,
      };
    }
  }
  return null;
}

function mapPoint(placement: FacePlacement, p: Vec2): Vec3 | null {
  const d = sub2(p, placement.p0);
  const a = cross2(d, placement.e2) / placement.det;
  const b = cross2(placement.e1, d) / placement.det;
  const q: Vec3 = [
    placement.q0[0] + placement.f1[0] * a + placement.f2[0] * b,
    placement.q0[1] + placement.f1[1] * a + placement.f2[1] * b,
    placement.q0[2] + placement.f1[2] * a + placement.f2[2] * b,
  ];
  return finite3(q) ? q : null;
}

/**
 * AuxはFace.edgesに含まれない。線分の中点を含む面を探し、その面の現在位置へ写す。
 * 通常の編集では交点で線分が分割されるため、Aux辺1本は1面の内側に収まる。
 */
function interiorSegment(
  a2: Vec2,
  b2: Vec2,
  placements: readonly FacePlacement[],
): Segment3D | null {
  const mid: Vec2 = [(a2[0] + b2[0]) / 2, (a2[1] + b2[1]) / 2];
  for (const placement of placements) {
    if (!pointInPolygon(placement.polygon, mid)) continue;
    const a = mapPoint(placement, a2);
    const b = mapPoint(placement, b2);
    if (a && b) return { a, b };
  }
  return null;
}

/**
 * 2Dで選択中の辺を、現在3Dビューに表示している線分へ導出する。
 *
 * 出力順はDocument内の辺順で一定。選択IDや壊れたDocumentに重複があっても
 * 同じedgeIdは最大1件だけ返す。面との対応が取れない辺は平面(z=0)へ戻して、
 * 選択したのに何も見えない状態を避ける。
 */
export function deriveSelectedEdgeHighlights(
  doc: Document,
  faces: readonly Face[],
  slots: ReadonlyMap<number, FacePositionSlot>,
  positions: ArrayLike<number>,
  hinges: ReadonlySet<number>,
  selectedEdgeIds: readonly number[],
): EdgeHighlightTarget[] {
  const selected = new Set(selectedEdgeIds);
  if (selected.size === 0) return [];

  const vertexPositions = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const placements = faces
    .map((face) => facePlacement(face, vertexPositions, slots, positions))
    .filter((placement): placement is FacePlacement => placement !== null);
  const emitted = new Set<number>();
  const out: EdgeHighlightTarget[] = [];

  for (const edge of doc.cp.edges) {
    if (!selected.has(edge.id) || emitted.has(edge.id)) continue;
    const a2 = vertexPositions.get(edge.v0);
    const b2 = vertexPositions.get(edge.v1);
    let segment = boundarySegment(edge.id, faces, slots, positions);
    if (!segment && a2 && b2 && finite2(a2) && finite2(b2)) {
      segment = interiorSegment(a2, b2, placements) ?? {
        a: [a2[0], a2[1], 0],
        b: [b2[0], b2[1], 0],
      };
    }
    if (!segment) continue;
    emitted.add(edge.id);
    out.push({
      edgeId: edge.id,
      role: hinges.has(edge.id) ? "hinge" : "reference",
      a: segment.a,
      b: segment.b,
    });
  }
  return out;
}
