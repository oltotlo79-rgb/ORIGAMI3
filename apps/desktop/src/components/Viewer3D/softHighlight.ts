// 選択辺をたわみ網へ正確に写すため、ori3-soft/subdivide.rs が作った
// 「元の紙の座標 ↔ 網頂点 ↔ 表示頂点」の対応を復元する純粋計算。

import * as THREE from "three";
import type { Document, EdgeKind, Face, SoftMesh, Vec2 } from "../../lib/types";
import type { HighlightSegment, SoftContent } from "./sceneBuilder";

/** ori3_model::EPS。Rustのear_clipと同じ比較に使う。 */
const EAR_EPS = 1e-9;
/** 正規化した展開図上の交点・区間端を同一視する許容差。 */
const MATERIAL_EPS = 1e-9;
/** 重心座標の分母として扱える三角形面積(外積の2倍)の下限。 */
const AREA_EPS = 1e-12;
const CANDIDATE_DIVISIONS = [1, 2, 4, 8, 16] as const;

export interface SoftHighlightTriangle {
  /** SoftMesh.positions上の3頂点。 */
  readonly sources: readonly [number, number, number];
  /** SoftContent.positions上の、面ごとに複製された3頂点。 */
  readonly display: readonly [number, number, number];
  /** 展開図上の同じ3頂点。 */
  readonly material: readonly [Vec2, Vec2, Vec2];
}

export interface SoftHighlightMap {
  readonly division: (typeof CANDIDATE_DIVISIONS)[number];
  /** updateSoftContentが層lift込みで更新する配列そのもの。コピーしない。 */
  readonly livePositions: Float32Array;
  readonly edgeMaterialEndpoints: ReadonlyMap<number, readonly [Vec2, Vec2]>;
  readonly edgeKinds: ReadonlyMap<number, EdgeKind>;
  /** 面ID → (SoftMesh頂点番号 → SoftContent表示頂点番号)。 */
  readonly displayByFaceSource: ReadonlyMap<number, ReadonlyMap<number, number>>;
  /** SoftMesh頂点番号 → 展開図上の位置。 */
  readonly materialPositions: readonly Vec2[];
  readonly trianglesByFace: ReadonlyMap<number, readonly SoftHighlightTriangle[]>;
}

interface ReconstructedMesh {
  readonly division: (typeof CANDIDATE_DIVISIONS)[number];
  readonly materialPositions: Vec2[];
  readonly triangles: [number, number, number][];
  readonly triangleFaces: number[];
}

interface PreparedFace {
  readonly face: Face;
  readonly polygon: Vec2[];
  readonly ears: [number, number, number][];
}

function finitePoint(point: Vec2 | undefined): point is Vec2 {
  return point !== undefined && Number.isFinite(point[0]) && Number.isFinite(point[1]);
}

function cross(a: Vec2, b: Vec2, c: Vec2): number {
  return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
}

/** crates/ori3-soft/src/subdivide.rs::ear_clip の走査・比較・fallbackを同じ順で行う。 */
function earClip(poly: readonly Vec2[]): [number, number, number][] {
  const out: [number, number, number][] = [];
  if (poly.length < 3) return out;
  const indices = Array.from({ length: poly.length }, (_, index) => index);
  while (indices.length > 3) {
    const count = indices.length;
    let cut = -1;
    for (let i = 0; i < count; i++) {
      const a = indices[(i + count - 1) % count];
      const b = indices[i];
      const c = indices[(i + 1) % count];
      if (cross(poly[a], poly[b], poly[c]) <= EAR_EPS) continue;
      const blocked = indices.some(
        (point) =>
          point !== a &&
          point !== b &&
          point !== c &&
          cross(poly[a], poly[b], poly[point]) >= 0 &&
          cross(poly[b], poly[c], poly[point]) >= 0 &&
          cross(poly[c], poly[a], poly[point]) >= 0,
      );
      if (!blocked) {
        cut = i;
        break;
      }
    }
    if (cut < 0) break;
    const currentCount = indices.length;
    out.push([
      indices[(cut + currentCount - 1) % currentCount],
      indices[cut],
      indices[(cut + 1) % currentCount],
    ]);
    indices.splice(cut, 1);
  }
  for (let i = 1; i < indices.length - 1; i++) {
    out.push([indices[0], indices[i], indices[i + 1]]);
  }
  return out;
}

function sideKey(face: Face, p: number, q: number, k: number, n: number): string {
  const count = face.vertices.length;
  const va = face.vertices[p];
  const vb = face.vertices[q];
  let edge: number | undefined;
  if ((p + 1) % count === q) edge = face.edges[p];
  else if ((q + 1) % count === p) edge = face.edges[q];
  if (edge !== undefined && va !== vb) {
    return `e:${edge}:${va < vb ? k : n - k}`;
  }
  return p < q
    ? `d:${face.id}:${p}:${q}:${k}`
    : `d:${face.id}:${q}:${p}:${n - k}`;
}

function gridKey(
  face: Face,
  triangle: number,
  corners: readonly [number, number, number],
  a: number,
  b: number,
  n: number,
): string {
  if (a === 0 && b === 0) return `c:${face.vertices[corners[0]]}`;
  if (a === n) return `c:${face.vertices[corners[1]]}`;
  if (b === n) return `c:${face.vertices[corners[2]]}`;
  if (b === 0) return sideKey(face, corners[0], corners[1], a, n);
  if (a === 0) return sideKey(face, corners[0], corners[2], b, n);
  if (a + b === n) return sideKey(face, corners[1], corners[2], b, n);
  return `i:${face.id}:${triangle}:${a}:${b}`;
}

function prepareFaces(
  faces: readonly Face[],
  vertexPositions: ReadonlyMap<number, Vec2>,
): PreparedFace[] | null {
  const prepared: PreparedFace[] = [];
  for (const face of faces) {
    if (
      !Number.isSafeInteger(face.id) ||
      face.vertices.length < 3 ||
      face.edges.length !== face.vertices.length
    ) {
      return null;
    }
    const polygon: Vec2[] = [];
    for (const vertex of face.vertices) {
      const point = vertexPositions.get(vertex);
      if (!finitePoint(point)) return null;
      polygon.push(point);
    }
    prepared.push({ face, polygon, ears: earClip(polygon) });
  }
  return prepared;
}

function reconstruct(
  preparedFaces: readonly PreparedFace[],
  division: (typeof CANDIDATE_DIVISIONS)[number],
): ReconstructedMesh {
  const sourceByKey = new Map<string, number>();
  const sums: [number, number][] = [];
  const counts: number[] = [];
  const triangles: [number, number, number][] = [];
  const triangleFaces: number[] = [];

  for (const { face, polygon, ears } of preparedFaces) {
    for (const [ear, corners] of ears.entries()) {
      const grid = new Array<number>((division + 1) * (division + 1)).fill(-1);
      const p0 = polygon[corners[0]];
      const p1 = polygon[corners[1]];
      const p2 = polygon[corners[2]];
      for (let a = 0; a <= division; a++) {
        for (let b = 0; b <= division - a; b++) {
          const key = gridKey(face, ear, corners, a, b, division);
          const point: Vec2 = [
            p0[0] + ((p1[0] - p0[0]) * a) / division + ((p2[0] - p0[0]) * b) / division,
            p0[1] + ((p1[1] - p0[1]) * a) / division + ((p2[1] - p0[1]) * b) / division,
          ];
          let source = sourceByKey.get(key);
          if (source === undefined) {
            source = sums.length;
            sourceByKey.set(key, source);
            sums.push([0, 0]);
            counts.push(0);
          }
          sums[source][0] += point[0];
          sums[source][1] += point[1];
          counts[source] += 1;
          grid[a * (division + 1) + b] = source;
        }
      }
      const at = (a: number, b: number) => grid[a * (division + 1) + b];
      for (let a = 0; a < division; a++) {
        for (let b = 0; b < division - a; b++) {
          triangles.push([at(a, b), at(a + 1, b), at(a, b + 1)]);
          triangleFaces.push(face.id);
          if (a + b + 2 <= division) {
            triangles.push([at(a + 1, b), at(a + 1, b + 1), at(a, b + 1)]);
            triangleFaces.push(face.id);
          }
        }
      }
    }
  }

  return {
    division,
    materialPositions: sums.map((sum, source) => [
      sum[0] / counts[source],
      sum[1] / counts[source],
    ]),
    triangles,
    triangleFaces,
  };
}

function exactSoftTopology(candidate: ReconstructedMesh, soft: SoftMesh): boolean {
  if (
    candidate.materialPositions.length !== soft.positions.length ||
    candidate.triangles.length !== soft.triangles.length ||
    candidate.triangleFaces.length !== soft.triangle_faces.length
  ) {
    return false;
  }
  for (let triangle = 0; triangle < candidate.triangles.length; triangle++) {
    const expected = candidate.triangles[triangle];
    const actual = soft.triangles[triangle];
    if (
      actual === undefined ||
      actual.length !== 3 ||
      actual[0] !== expected[0] ||
      actual[1] !== expected[1] ||
      actual[2] !== expected[2] ||
      soft.triangle_faces[triangle] !== candidate.triangleFaces[triangle]
    ) {
      return false;
    }
  }
  return true;
}

/**
 * Rustが実際に選んだ細分数を、出力topologyとの完全一致だけから一意に復元する。
 * 座標の最近傍照合は使わず、候補が0個または複数なら誤対応を避けてnullを返す。
 */
export function buildSoftHighlightMap(
  doc: Document,
  faces: readonly Face[],
  soft: SoftMesh,
  softContent: SoftContent,
): SoftHighlightMap | null {
  const vertexPositions = new Map<number, Vec2>();
  for (const vertex of doc.cp.vertices) {
    if (!Number.isSafeInteger(vertex.id) || !finitePoint(vertex.pos)) return null;
    vertexPositions.set(vertex.id, vertex.pos);
  }
  const faceIds = new Set<number>();
  for (const face of faces) {
    if (faceIds.has(face.id)) return null;
    faceIds.add(face.id);
  }

  const preparedFaces = prepareFaces(faces, vertexPositions);
  if (!preparedFaces) return null;
  const baseTriangleCount = preparedFaces.reduce(
    (count, prepared) => count + prepared.ears.length,
    0,
  );
  if (baseTriangleCount <= 0) return null;
  // Rustではear 1枚がdiv²枚になる。400面・div2なら、これだけで候補5個を
  // 1個へ絞れ、外れたdivのMeshKey/gridを構築しない。完全なindex/face/source
  // 照合はこの後も行うため、速さのために検証を緩めてはいない。
  const divisions = CANDIDATE_DIVISIONS.filter(
    (division) => baseTriangleCount * division * division === soft.triangles.length,
  );
  if (divisions.length !== 1) return null;
  const match = reconstruct(preparedFaces, divisions[0]);
  if (!exactSoftTopology(match, soft)) return null;

  const layout = softContent.layout;
  if (
    layout.vertexCount !== layout.source.length ||
    layout.vertexCount !== layout.faceOf.length ||
    softContent.positions.length !== layout.vertexCount * 3 ||
    layout.indices.length !== soft.triangles.length * 3 ||
    layout.triangleSources.length !== soft.triangles.length ||
    layout.triangleFaceIds.length !== soft.triangles.length
  ) {
    return null;
  }
  const displayByFaceSource = new Map<number, Map<number, number>>();
  for (let display = 0; display < layout.vertexCount; display++) {
    const source = layout.source[display];
    const face = layout.faceOf[display];
    if (source < 0 || source >= soft.positions.length || !faceIds.has(face)) return null;
    let bySource = displayByFaceSource.get(face);
    if (!bySource) {
      bySource = new Map<number, number>();
      displayByFaceSource.set(face, bySource);
    }
    if (bySource.has(source)) return null;
    bySource.set(source, display);
  }

  const trianglesByFace = new Map<number, SoftHighlightTriangle[]>();
  for (let triangle = 0; triangle < match.triangles.length; triangle++) {
    const face = match.triangleFaces[triangle];
    const sources = match.triangles[triangle];
    const bySource = displayByFaceSource.get(face);
    if (
      layout.triangleSources[triangle] !== triangle ||
      layout.triangleFaceIds[triangle] !== face ||
      bySource === undefined
    ) {
      return null;
    }
    const d0 = bySource.get(sources[0]);
    const d1 = bySource.get(sources[1]);
    const d2 = bySource.get(sources[2]);
    if (
      d0 === undefined ||
      d1 === undefined ||
      d2 === undefined ||
      layout.indices[triangle * 3] !== d0 ||
      layout.indices[triangle * 3 + 1] !== d1 ||
      layout.indices[triangle * 3 + 2] !== d2
    ) {
      return null;
    }
    let list = trianglesByFace.get(face);
    if (!list) {
      list = [];
      trianglesByFace.set(face, list);
    }
    list.push({
      sources,
      display: [d0, d1, d2],
      material: [
        match.materialPositions[sources[0]],
        match.materialPositions[sources[1]],
        match.materialPositions[sources[2]],
      ],
    });
  }

  const edgeMaterialEndpoints = new Map<number, readonly [Vec2, Vec2]>();
  const edgeKinds = new Map<number, EdgeKind>();
  for (const edge of doc.cp.edges) {
    if (!Number.isSafeInteger(edge.id) || edgeMaterialEndpoints.has(edge.id)) return null;
    const a = vertexPositions.get(edge.v0);
    const b = vertexPositions.get(edge.v1);
    if (!finitePoint(a) || !finitePoint(b)) return null;
    edgeMaterialEndpoints.set(edge.id, [a, b]);
    edgeKinds.set(edge.id, edge.kind);
  }

  return {
    division: match.division,
    livePositions: softContent.positions,
    edgeMaterialEndpoints,
    edgeKinds,
    displayByFaceSource,
    materialPositions: match.materialPositions,
    trianglesByFace,
  };
}

function subtract(a: Vec2, b: Vec2): Vec2 {
  return [a[0] - b[0], a[1] - b[1]];
}

function crossVector(a: Vec2, b: Vec2): number {
  return a[0] * b[1] - a[1] * b[0];
}

function dot(a: Vec2, b: Vec2): number {
  return a[0] * b[0] + a[1] * b[1];
}

function clampUnit(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function addTriangleEdgeIntersections(
  out: number[],
  start: Vec2,
  direction: Vec2,
  directionLength2: number,
  a: Vec2,
  b: Vec2,
): void {
  const edge = subtract(b, a);
  const fromStart = subtract(a, start);
  const denominator = crossVector(direction, edge);
  if (Math.abs(denominator) > AREA_EPS) {
    const t = crossVector(fromStart, edge) / denominator;
    const u = crossVector(fromStart, direction) / denominator;
    if (
      Number.isFinite(t) &&
      Number.isFinite(u) &&
      t >= -MATERIAL_EPS &&
      t <= 1 + MATERIAL_EPS &&
      u >= -MATERIAL_EPS &&
      u <= 1 + MATERIAL_EPS
    ) {
      out.push(clampUnit(t));
    }
    return;
  }

  // 平行でも同一直線なら、三角形辺の両端を物理辺のparameterへ射影する。
  // 最近傍点は使わず、重なっている範囲の端だけを分割点にする。
  if (Math.abs(crossVector(fromStart, direction)) > MATERIAL_EPS) return;
  for (const point of [a, b]) {
    const t = dot(subtract(point, start), direction) / directionLength2;
    if (Number.isFinite(t) && t >= -MATERIAL_EPS && t <= 1 + MATERIAL_EPS) {
      out.push(clampUnit(t));
    }
  }
}

function barycentric(
  point: Vec2,
  triangle: SoftHighlightTriangle,
): [number, number, number] | null {
  const [a, b, c] = triangle.material;
  const ab = subtract(b, a);
  const ac = subtract(c, a);
  const ap = subtract(point, a);
  const denominator = crossVector(ab, ac);
  if (!Number.isFinite(denominator) || Math.abs(denominator) <= AREA_EPS) return null;
  const wb = crossVector(ap, ac) / denominator;
  const wc = crossVector(ab, ap) / denominator;
  const wa = 1 - wb - wc;
  if (
    !Number.isFinite(wa) ||
    !Number.isFinite(wb) ||
    !Number.isFinite(wc) ||
    wa < -MATERIAL_EPS ||
    wb < -MATERIAL_EPS ||
    wc < -MATERIAL_EPS ||
    wa > 1 + MATERIAL_EPS ||
    wb > 1 + MATERIAL_EPS ||
    wc > 1 + MATERIAL_EPS
  ) {
    return null;
  }
  // 交点の丸めで境界から1e-9以内だけ外れた重みを境界へ戻す。
  const clamped = [Math.max(0, wa), Math.max(0, wb), Math.max(0, wc)] as const;
  const sum = clamped[0] + clamped[1] + clamped[2];
  if (!Number.isFinite(sum) || sum <= AREA_EPS) return null;
  return [clamped[0] / sum, clamped[1] / sum, clamped[2] / sum];
}

function materialPoint(start: Vec2, direction: Vec2, t: number): Vec2 {
  return [start[0] + direction[0] * t, start[1] + direction[1] * t];
}

function displayPoint(
  triangle: SoftHighlightTriangle,
  weights: readonly [number, number, number],
  positions: Float32Array,
): THREE.Vector3 | null {
  const result = new THREE.Vector3();
  for (let corner = 0; corner < 3; corner++) {
    const display = triangle.display[corner];
    const at = display * 3;
    const x = positions[at];
    const y = positions[at + 1];
    const z = positions[at + 2];
    if (
      !Number.isSafeInteger(display) ||
      display < 0 ||
      at + 2 >= positions.length ||
      !Number.isFinite(x) ||
      !Number.isFinite(y) ||
      !Number.isFinite(z)
    ) {
      return null;
    }
    result.x += x * weights[corner];
    result.y += y * weights[corner];
    result.z += z * weights[corner];
  }
  return Number.isFinite(result.x) && Number.isFinite(result.y) && Number.isFinite(result.z)
    ? result
    : null;
}

function projectOneSegment(
  segment: HighlightSegment,
  map: SoftHighlightMap,
): HighlightSegment[] | null {
  const ownerFace = segment.ownerFace;
  if (ownerFace === undefined) return null;
  const endpoints = map.edgeMaterialEndpoints.get(segment.edgeId);
  const triangles = map.trianglesByFace.get(ownerFace);
  if (!endpoints || !triangles || triangles.length === 0) return null;
  const [start, end] = endpoints;
  const direction = subtract(end, start);
  const directionLength2 = dot(direction, direction);
  if (!Number.isFinite(directionLength2) || directionLength2 <= AREA_EPS) return null;

  const cuts = [0, 1];
  for (const triangle of triangles) {
    for (let edge = 0; edge < 3; edge++) {
      addTriangleEdgeIntersections(
        cuts,
        start,
        direction,
        directionLength2,
        triangle.material[edge],
        triangle.material[(edge + 1) % 3],
      );
    }
  }
  cuts.sort((a, b) => a - b);
  const uniqueCuts: number[] = [];
  for (const cut of cuts) {
    if (!Number.isFinite(cut)) return null;
    if (
      uniqueCuts.length === 0 ||
      Math.abs(cut - uniqueCuts[uniqueCuts.length - 1]) > MATERIAL_EPS
    ) {
      uniqueCuts.push(cut);
    }
  }

  const pieces: HighlightSegment[] = [];
  let outsideLength = 0;
  let insideLength = 0;
  for (let interval = 0; interval + 1 < uniqueCuts.length; interval++) {
    const from = uniqueCuts[interval];
    const to = uniqueCuts[interval + 1];
    if (to - from <= MATERIAL_EPS) continue;
    const midpoint = materialPoint(start, direction, (from + to) / 2);
    let selected: SoftHighlightTriangle | undefined;
    for (const triangle of triangles) {
      if (barycentric(midpoint, triangle) !== null) {
        selected = triangle;
        break;
      }
    }
    if (!selected) {
      outsideLength += to - from;
      continue;
    }
    const fromWeights = barycentric(materialPoint(start, direction, from), selected);
    const toWeights = barycentric(materialPoint(start, direction, to), selected);
    if (!fromWeights || !toWeights) return null;
    const a = displayPoint(selected, fromWeights, map.livePositions);
    const b = displayPoint(selected, toWeights, map.livePositions);
    // 各pieceが実際に載るfine triangleの重心を内向きprobeにする。rigid側の
    // probeをそのまま残すと、膨らんだ紙では中心線の可視判定だけが旧平面へ戻る。
    const surfaceProbe = displayPoint(
      selected,
      [1 / 3, 1 / 3, 1 / 3],
      map.livePositions,
    );
    if (!a || !b || !surfaceProbe) return null;
    pieces.push({ ...segment, a, b, surfaceProbe });
    insideLength += to - from;
  }

  if (pieces.length === 0 || insideLength <= MATERIAL_EPS) return null;
  // Face.edgesに属する物理辺は全区間がその面に含まれる。Auxだけは面の外を横切る
  // ことがあるので、含まれる区間を全て写せていれば外側のgapを許す。
  if (map.edgeKinds.get(segment.edgeId) !== "Aux" && outsideLength > MATERIAL_EPS) return null;
  if (insideLength + outsideLength < 1 - MATERIAL_EPS) return null;
  return pieces;
}

/**
 * 強調線をCP材質座標で細分網へ写す。近い頂点を探す近似は行わない。
 * 対応が欠ける線分は、見えなくするのではなく入力オブジェクトそのものを返す。
 */
export function projectHighlightSegmentsToSoftSurface(
  segments: HighlightSegment[],
  map: SoftHighlightMap | null,
): HighlightSegment[] {
  if (map === null) return segments;
  const projected: HighlightSegment[] = [];
  for (const segment of segments) {
    const pieces = projectOneSegment(segment, map);
    if (pieces === null) projected.push(segment);
    else projected.push(...pieces);
  }
  return projected;
}
