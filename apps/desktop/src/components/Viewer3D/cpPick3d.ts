// 3Dビューでクリックした画素を、展開図の頂点・辺・面内座標へ戻す1本の逆写像。
//
// これがある前に3Dでできたのは「面を当てる」「辺を当てる」だけで、点は道具ごとに
// 別々の当て方(合わせて折る専用の層の輪郭あたり判定)しか無く、しかも平らに畳んだ
// 表示でしか働かなかった。ここでは道具ごとの分岐を作らず、
//
//     クリック画素 → 見えている面ID → その面内の展開図座標 → 頂点ID / 辺ID
//
// という1本の道だけを用意し、選ぶ・動かす・線を引く・作図するのすべてが同じ道を通る。
//
// 面の現在位置は3Dの頂点バッファから読む(edgeHighlight.ts の facePlacement)。
// 面が傾いていても倒れていても同じ式なので、平らな姿勢と立体姿勢を区別しない。
// 「高さのある面を捨てる」処理はここには無い。

import * as THREE from "three";
import type { Document, Face, Vec2 } from "../../lib/types";
import type { Vec3 } from "../../lib/layerOffset";
import {
  facePlacement,
  facePlaneNormal,
  mapPoint,
  pointInPolygon,
  unmapPoint,
  type FacePlacement,
  type FacePositionSlot,
} from "./edgeHighlight";
import {
  pickPaper,
  projectToScreenPx,
  screenDistanceToSegment,
  type PaperPickSurface,
} from "./hingePicker";

/**
 * 点を拾う許容距離(px)。辺の10px(`PICK_THRESHOLD_PX`)より狭くしてある。
 * 点は辺より優先して選ぶので、同じ広さにすると辺の端に近い場所で辺を選べなくなる。
 */
export const CP_POINT_PICK_PX = 8;

/** 面の上に載っている展開図の頂点 */
export interface CpFaceVertex {
  id: number;
  /** 展開図座標 */
  pos: Vec2;
}

/** 面の上に載っている展開図の辺 */
export interface CpFaceEdge {
  id: number;
  a: Vec2;
  b: Vec2;
}

/**
 * 「どの展開図の頂点・辺が、どの面の上に載っているか」の対応。
 * 3Dの形には依存しないので、展開図が変わるまで作り直さない。
 */
export interface CpFaceIndex {
  /** この対応を作った元。使い回してよいかの照合に使う */
  readonly doc: Document;
  readonly faces: readonly Face[];
  readonly facesById: ReadonlyMap<number, Face>;
  /** 展開図の頂点ID → 座標(面の位置合わせで毎回作り直さないよう持っておく) */
  readonly vertexPositions: ReadonlyMap<number, Vec2>;
  readonly vertices: ReadonlyMap<number, readonly CpFaceVertex[]>;
  readonly edges: ReadonlyMap<number, readonly CpFaceEdge[]>;
  /** 面ID → 展開図側の多角形 */
  readonly polygons: ReadonlyMap<number, readonly Vec2[]>;
}

/** 3Dの画素から拾った、展開図側の対応 */
export interface CpPick3D {
  /** その画素で実際に見えている面のID */
  faceId: number;
  /** その面内の展開図座標(頂点に当たっていればその頂点のちょうどの座標) */
  cp: Vec2;
  /** 3D上の当たった位置。平らに畳んだ表示では xy がそのまま畳み平面の座標になる */
  world: Vec3;
  /** 許容距離内にある展開図の頂点。無ければnull */
  vertexId: number | null;
  /** 許容距離内にある展開図の辺。無ければnull */
  edgeId: number | null;
  /**
   * クリックした画素そのものが紙の上だったか。
   * 紙の角ちょうど・紙のすぐ外を押したときは false になり、そばの点だけが拾える。
   */
  onPaper: boolean;
}

/** 面の多角形を包む長方形。総当たりを避けるための粗いふるい。 */
interface Bounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/** 境界からこの距離以内は「面の上」に含める(展開図は長辺=1の正規化座標)。 */
const ON_FACE_EPS = 1e-9;

/**
 * 材料座標と3D座標はいずれも紙の長辺を1とする正規化座標。
 * Float64の逆写像で面上かを確かめるため、表示用Float32頂点の丸めより十分小さく、
 * かつ演算誤差より大きい距離を許す。
 */
const MATERIAL_COORD_EPS = 1e-9;
const WORLD_PLANE_EPS = 1e-8;

function distanceSquared2(a: Vec2, b: Vec2): number {
  return (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2;
}

function distanceSquared3(a: Vec3, b: Vec3): number {
  return (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2 + (a[2] - b[2]) ** 2;
}

function pointSegmentDistanceSquared(p: Vec2, a: Vec2, b: Vec2): number {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const lengthSquared = dx * dx + dy * dy;
  const t =
    lengthSquared === 0
      ? 0
      : Math.max(
          0,
          Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / lengthSquared),
        );
  const q: Vec2 = [a[0] + dx * t, a[1] + dy * t];
  return distanceSquared2(p, q);
}

function strictlyInsidePolygon(polygon: readonly Vec2[], point: Vec2): boolean {
  if (!pointInPolygon(polygon, point, MATERIAL_COORD_EPS)) return false;
  const epsSquared = MATERIAL_COORD_EPS * MATERIAL_COORD_EPS;
  for (let i = 0; i < polygon.length; i++) {
    if (
      pointSegmentDistanceSquared(point, polygon[i], polygon[(i + 1) % polygon.length]) <=
      epsSquared
    ) {
      return false;
    }
  }
  return true;
}

function boundsOf(polygon: readonly Vec2[]): Bounds {
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  for (const p of polygon) {
    minX = Math.min(minX, p[0]);
    minY = Math.min(minY, p[1]);
    maxX = Math.max(maxX, p[0]);
    maxY = Math.max(maxY, p[1]);
  }
  return { minX, minY, maxX, maxY };
}

function inBounds(bounds: Bounds, p: Vec2, eps: number): boolean {
  return (
    p[0] >= bounds.minX - eps &&
    p[0] <= bounds.maxX + eps &&
    p[1] >= bounds.minY - eps &&
    p[1] <= bounds.maxY + eps
  );
}

/**
 * 展開図の頂点・辺を面ごとに振り分ける。
 * 面の角だけでなく、面の内側に落ちている頂点(補助線の端など)も同じ面の候補にする。
 * 展開図が同じ間は結果が変わらないので、`doc`・`faces`が同じなら作り直さない。
 */
export function buildCpFaceIndex(doc: Document, faces: readonly Face[]): CpFaceIndex {
  const positions = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const vertices = new Map<number, CpFaceVertex[]>();
  const edges = new Map<number, CpFaceEdge[]>();
  const polygons = new Map<number, Vec2[]>();

  for (const face of faces) {
    const polygon: Vec2[] = [];
    for (const id of face.vertices) {
      const p = positions.get(id);
      if (!p) break;
      polygon.push(p);
    }
    if (polygon.length !== face.vertices.length || polygon.length < 3) continue;
    polygons.set(face.id, polygon);
    const bounds = boundsOf(polygon);

    const seen = new Set<number>();
    const faceVertices: CpFaceVertex[] = [];
    for (const vertex of doc.cp.vertices) {
      if (seen.has(vertex.id)) continue;
      if (!inBounds(bounds, vertex.pos, ON_FACE_EPS)) continue;
      if (!pointInPolygon(polygon, vertex.pos, ON_FACE_EPS)) continue;
      seen.add(vertex.id);
      faceVertices.push({ id: vertex.id, pos: vertex.pos });
    }
    vertices.set(face.id, faceVertices);

    const faceEdges: CpFaceEdge[] = [];
    for (const edge of doc.cp.edges) {
      const a = positions.get(edge.v0);
      const b = positions.get(edge.v1);
      if (!a || !b) continue;
      if (!seen.has(edge.v0) || !seen.has(edge.v1)) continue;
      // 両端が面の上にあっても、へこんだ面では途中が外へ出ることがある。
      // 中点も面の上にあることを確かめる(edgeHighlightのAux判定と同じ考え方)。
      const mid: Vec2 = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
      if (!pointInPolygon(polygon, mid, ON_FACE_EPS)) continue;
      faceEdges.push({ id: edge.id, a, b });
    }
    edges.set(face.id, faceEdges);
  }

  return {
    doc,
    faces,
    facesById: new Map(faces.map((face) => [face.id, face])),
    vertexPositions: positions,
    vertices,
    edges,
    polygons,
  };
}

/** 面の現在位置を復元する(3Dの頂点バッファから読む)。 */
export function placementOf(
  index: CpFaceIndex,
  faceId: number,
  slots: ReadonlyMap<number, FacePositionSlot>,
  positions: ArrayLike<number>,
): FacePlacement | null {
  const face = index.facesById.get(faceId);
  if (!face) return null;
  return facePlacement(face, index.vertexPositions, slots, positions);
}

/** Rustへ渡せる、材料座標だけで表した折り線と残す側の内部点。 */
export interface MaterialFoldLine {
  material_line: [Vec2, Vec2];
  material_keep_side_point: Vec2;
}

function materialPointOnPlacement(placement: FacePlacement, world: Vec3): Vec2 | null {
  const material = unmapPoint(placement, world);
  if (!material) return null;
  const remapped = mapPoint(placement, material);
  if (!remapped) return null;
  const worldScale = Math.max(
    1,
    Math.hypot(...placement.f1),
    Math.hypot(...placement.f2),
  );
  if (distanceSquared3(remapped, world) > (WORLD_PLANE_EPS * worldScale) ** 2) {
    return null;
  }
  return material;
}

/**
 * 表示中の同じ面にある折り線上の2点と、残す側の厳密な内部点を材料座標へ戻す。
 *
 * `unmapPoint` は3D基底2本を使うため、傾斜面・垂直面でもglobal XYへ潰さない。
 * 3点をすべて説明できる面が一意でない場合は、Face ID順などで推測せずnullを返す。
 * 返すのは材料座標だけであり、Face ID・surface_rank・selectedLayerCount・2 * Kから
 * 作った面数は絶対にwireへ運ばない。ひだ数と対象面はRustが同じ書類から再計算する。
 */
export function materialFoldLineFrom3D(
  placements: readonly FacePlacement[],
  lineWorld: readonly [Vec3, Vec3],
  keepSideWorld: Vec3,
): MaterialFoldLine | null {
  if (distanceSquared3(lineWorld[0], lineWorld[1]) <= MATERIAL_COORD_EPS ** 2) {
    return null;
  }

  const candidates: MaterialFoldLine[] = [];
  for (const placement of placements) {
    const a = materialPointOnPlacement(placement, lineWorld[0]);
    const b = materialPointOnPlacement(placement, lineWorld[1]);
    const keep = materialPointOnPlacement(placement, keepSideWorld);
    if (!a || !b || !keep) continue;
    if (
      !pointInPolygon(placement.polygon, a, MATERIAL_COORD_EPS) ||
      !pointInPolygon(placement.polygon, b, MATERIAL_COORD_EPS) ||
      !strictlyInsidePolygon(placement.polygon, keep)
    ) {
      continue;
    }

    const materialLengthSquared = distanceSquared2(a, b);
    if (materialLengthSquared <= MATERIAL_COORD_EPS ** 2) continue;
    const side =
      (b[0] - a[0]) * (keep[1] - a[1]) -
      (b[1] - a[1]) * (keep[0] - a[0]);
    if (Math.abs(side) <= MATERIAL_COORD_EPS * Math.sqrt(materialLengthSquared)) continue;

    candidates.push({
      material_line: [
        [a[0], a[1]],
        [b[0], b[1]],
      ],
      material_keep_side_point: [keep[0], keep[1]],
    });
  }

  // 同じ3D点を複数の材料面が説明できるときも、Face ID順では選ばない。
  return candidates.length === 1 ? candidates[0] : null;
}

/**
 * 立体姿勢でも数えられる、3Dで指せる展開図の点の候補。
 * 「平らな面しか候補にしない」作りになっていないことを検査で数えるために公開する。
 */
export function cpPointCandidates(
  index: CpFaceIndex,
  slots: ReadonlyMap<number, FacePositionSlot>,
  positions: ArrayLike<number>,
): { faceId: number; vertexId: number; cp: Vec2; world: Vec3 }[] {
  const out: { faceId: number; vertexId: number; cp: Vec2; world: Vec3 }[] = [];
  for (const face of index.faces) {
    const placement = placementOf(index, face.id, slots, positions);
    if (!placement) continue;
    for (const vertex of index.vertices.get(face.id) ?? []) {
      const world = mapPoint(placement, vertex.pos);
      if (!world) continue;
      out.push({ faceId: face.id, vertexId: vertex.id, cp: vertex.pos, world });
    }
  }
  return out;
}

/** `pickCpFromPixel` へ渡す材料 */
export interface CpPickArgs {
  index: CpFaceIndex;
  /** 面ID → 3D頂点バッファ上の範囲(sceneのtopology.slots) */
  slots: ReadonlyMap<number, FacePositionSlot>;
  /** 現在表示している3D頂点座標(sceneのpositions) */
  positions: ArrayLike<number>;
  /** どの画素にどの面が見えているかを決める紙面 */
  surface: PaperPickSurface;
  camera: THREE.Camera;
  widthPx: number;
  heightPx: number;
  x: number;
  y: number;
  /** 点を拾う許容距離(px) */
  thresholdPx?: number;
}

/**
 * 3Dのクリック画素から、展開図の頂点ID・辺ID・面内座標を1度に返す。
 *
 * 面は既存の紙面あたり判定(`pickPaper`)で決めるので、重なった紙の手前が選ばれる。
 * 点と辺は画面上の距離(px)で選ぶ。3Dの辺選択(`pickHingeSegment`)と同じ基準にすると、
 * 拡大率が変わっても拾いやすさが変わらない。
 */
export function pickCpFromPixel(args: CpPickArgs): CpPick3D | null {
  const { index, slots, positions, surface, camera, widthPx, heightPx, x, y } = args;
  const thresholdPx = args.thresholdPx ?? CP_POINT_PICK_PX;
  const probe = (px: number, py: number) =>
    pickPaper(
      surface.mesh,
      surface.triangleFaceIds,
      camera,
      widthPx,
      heightPx,
      px,
      py,
      surface.triangleLayers,
      surface.faceSurfaceRanks,
    );

  const center = probe(x, y);
  // 紙の角ちょうどを押すと、光線が三角形の縁に当たって外れることがある。
  // 展開図区画では紙の外を押しても近くの点を拾えるので、3Dでも同じにする。
  // 中心が外れたときだけ、許容距離の円周を8方向で探して面を見つける。
  const faceIds: number[] = [];
  if (center) faceIds.push(center.face);
  else {
    for (let i = 0; i < 8; i++) {
      const angle = (i * Math.PI) / 4;
      const hit = probe(x + Math.cos(angle) * thresholdPx, y + Math.sin(angle) * thresholdPx);
      if (hit && !faceIds.includes(hit.face)) faceIds.push(hit.face);
    }
  }
  if (faceIds.length === 0) return null;

  let best: {
    faceId: number;
    placement: FacePlacement;
    vertexId: number | null;
    vertexCp: Vec2 | null;
    vertexWorld: Vec3 | null;
    vertexPx: number;
    vertexDepth: number;
    edgeId: number | null;
  } | null = null;

  for (const faceId of faceIds) {
    const placement = placementOf(index, faceId, slots, positions);
    if (!placement) continue;
    let vertexId: number | null = null;
    let vertexCp: Vec2 | null = null;
    let vertexWorld: Vec3 | null = null;
    let vertexPx = thresholdPx;
    let vertexDepth = Number.POSITIVE_INFINITY;
    for (const vertex of index.vertices.get(faceId) ?? []) {
      const q = mapPoint(placement, vertex.pos);
      if (!q) continue;
      const screen = projectToScreenPx(new THREE.Vector3(...q), camera, widthPx, heightPx);
      if (!screen) continue;
      const d = Math.hypot(screen.x - x, screen.y - y);
      if (d <= vertexPx) {
        vertexPx = d;
        vertexDepth = screen.depth;
        vertexId = vertex.id;
        vertexCp = vertex.pos;
        vertexWorld = q;
      }
    }
    let edgeId: number | null = null;
    let bestEdge = thresholdPx;
    for (const edge of index.edges.get(faceId) ?? []) {
      const qa = mapPoint(placement, edge.a);
      const qb = mapPoint(placement, edge.b);
      if (!qa || !qb) continue;
      const sa = projectToScreenPx(new THREE.Vector3(...qa), camera, widthPx, heightPx);
      const sb = projectToScreenPx(new THREE.Vector3(...qb), camera, widthPx, heightPx);
      if (!sa || !sb) continue;
      const d = screenDistanceToSegment(x, y, sa.x, sa.y, sb.x, sb.y);
      if (d <= bestEdge) {
        bestEdge = d;
        edgeId = edge.id;
      }
    }
    const candidate = {
      faceId,
      placement,
      vertexId,
      vertexCp,
      vertexWorld,
      vertexPx,
      vertexDepth,
      edgeId,
    };
    // 面が複数見つかるのは中心が紙を外れたときだけ。画面上でいちばん近い点を持つ面、
    // 同じ近さなら手前の面を選ぶ(見えている紙の点を選んだことになる)。
    if (
      best === null ||
      candidate.vertexPx < best.vertexPx ||
      (candidate.vertexPx === best.vertexPx && candidate.vertexDepth < best.vertexDepth)
    ) {
      best = candidate;
    }
  }
  if (best === null) return null;

  const world: Vec3 | null = center
    ? [center.point.x, center.point.y, center.point.z]
    : null;
  const cp = world ? unmapPoint(best.placement, world) : null;
  // 頂点に当たったときは、丸めの入る逆写像の結果ではなく展開図の値をそのまま返す。
  const resolvedCp = best.vertexCp ?? cp;
  const resolvedWorld = best.vertexWorld ?? world;
  // 紙を外していて近くの点も無ければ、指しているものが何も無い。
  if (!resolvedCp || !resolvedWorld) return null;

  return {
    faceId: best.faceId,
    cp: resolvedCp,
    world: resolvedWorld,
    vertexId: best.vertexId,
    edgeId: best.edgeId,
    onPaper: center !== null,
  };
}

/**
 * 画素を、指定した面が乗っている平面まで延ばして展開図座標へ直す。
 * 点をつかんで動かしている間は、面の外へカーソルが出ても同じ面の座標系で追い続ける。
 */
export function cpPointOnFacePlane(
  placement: FacePlacement,
  camera: THREE.Camera,
  widthPx: number,
  heightPx: number,
  x: number,
  y: number,
): Vec2 | null {
  const normal = facePlaneNormal(placement);
  if (!normal) return null;
  const raycaster = new THREE.Raycaster();
  raycaster.setFromCamera(
    new THREE.Vector2((x / widthPx) * 2 - 1, 1 - (y / heightPx) * 2),
    camera,
  );
  const plane = new THREE.Plane().setFromNormalAndCoplanarPoint(
    new THREE.Vector3(...normal),
    new THREE.Vector3(...placement.q0),
  );
  const at = raycaster.ray.intersectPlane(plane, new THREE.Vector3());
  if (!at) return null;
  return unmapPoint(placement, [at.x, at.y, at.z]);
}

/**
 * 展開図の点を、その面の現在位置での小さな十字(3D)にする。選んだ点を見せるのに使う。
 * `lift` は紙の面から浮かせる高さ。面と同じ高さに置くと紙とちらつくので少しだけ浮かす。
 */
export function cpMarkSegments(
  placement: FacePlacement,
  cp: Vec2,
  size: number,
  lift = 0,
): [Vec3, Vec3][] {
  const normal = lift === 0 ? null : facePlaneNormal(placement);
  const raise = (p: Vec3): Vec3 =>
    normal === null
      ? p
      : [p[0] + normal[0] * lift, p[1] + normal[1] * lift, p[2] + normal[2] * lift];
  const out: [Vec3, Vec3][] = [];
  for (const [dx, dy] of [
    [size, 0],
    [0, size],
  ]) {
    const a = mapPoint(placement, [cp[0] - dx, cp[1] - dy]);
    const b = mapPoint(placement, [cp[0] + dx, cp[1] + dy]);
    if (a && b) out.push([raise(a), raise(b)]);
  }
  return out;
}

/** 紙の外形の端点は展開図の輪郭そのものなので、動かす対象にしない(展開図区画と同じ規則)。 */
export function isBorderVertex(doc: Document, vertexId: number): boolean {
  return doc.cp.edges.some(
    (edge) =>
      edge.kind === "Border" && (edge.v0 === vertexId || edge.v1 === vertexId),
  );
}
