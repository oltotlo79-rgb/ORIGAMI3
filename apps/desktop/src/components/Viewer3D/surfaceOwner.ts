// 1画素につき最前面の紙を1枚だけ選ぶための、Three.jsに依存する表示用データ。
// WebGLRendererやDOMには触れず、面IDの符号化・owner用geometry・描画順だけを扱う。
//
// 既定視点の24-bit depthは紙の位置で約1.58269e-5を区別でき、層間隔0.0002は
// その12.64倍ある。従って深度の精度は足りており、根本原因ではなかった。
// 角度操作ではto_frame3dが全ての面をlayer=0で返し得るため、重なった面の実際の
// 位置差が0になることが原因である。同値はlayer→面の鏡映偶奇→決定的fallbackの順で
// 決め、線だけでなく紙の色も同じownerへ絞らなければ、同一深度の面同士の縞模様は
// 消えない。

import * as THREE from "three";

/** owner render targetの0は、紙が無い背景として予約する。 */
export const SURFACE_OWNER_BACKGROUND_CODE = 0;

/** 共平面のtriangleが共有する、NDC xy→window depthの平面係数。 */
export const SURFACE_OWNER_DEPTH_PLANE_ATTRIBUTE = "surfaceOwnerDepthPlane";

/**
 * 支持平面ごとに付ける組の符号。0は「平面が定まらず組を作れなかった」。
 *
 * 共平面の面どうしは深度が完全に一致するはずだが、実機のGPUでは三角形分割ごとの
 * 補間で数code分ずれ、2 codeのtie窓から外れた面が捨てられていた。深度の一致に
 * 頼らず整数の一致で「同じ平面の仲間」を判定できるよう、面ID符号と同じ
 * RGBA8 tokenの仕組みで組符号を持たせる。
 */
export const SURFACE_OWNER_GROUP_ATTRIBUTE = "surfaceOwnerGroupToken";

/** owner最前深度targetと同じ固定精度。 */
export const SURFACE_OWNER_DEPTH_BITS = 24;
/** 共平面の補間丸めだけをtieとして扱う既存の深度code数。 */
export const SURFACE_OWNER_DEPTH_TOLERANCE_CODES = 2;
export const SURFACE_OWNER_DEPTH_TOLERANCE =
  SURFACE_OWNER_DEPTH_TOLERANCE_CODES / (2 ** SURFACE_OWNER_DEPTH_BITS - 1);

/** 剛体face自身を平面と認めるworld座標誤差。既存rigidity契約と同じ。 */
export const SURFACE_OWNER_PLANARITY_EPSILON = 1e-6;
/** 別faceを同じ支持平面へまとめるworld座標誤差。 */
export const SURFACE_OWNER_COPLANAR_EPSILON = 1e-6;
/** 単位法線どうしを同じ向きと認める無次元誤差。 */
export const SURFACE_OWNER_NORMAL_EPSILON = 1e-6;

const MAX_OWNER_CODE = 0xffff_ffff;

/** 同じ深度候補を描く順、またはpickerで選ぶ順を決める純粋な比較入力。 */
export interface SurfaceOwnerPriority {
  readonly faceId: number;
  readonly surfaceRank: number;
  readonly side: 1 | -1;
  readonly materialOrientation: 1 | -1;
}

/**
 * 昇順の所有者優先順位。正ならaをbより後に描く／aをpickerで選ぶ。
 * front/backは順位へ入れず、物理的な層順と既存fallbackを保つ。
 */
export function compareSurfaceOwnerPriority(
  a: SurfaceOwnerPriority,
  b: SurfaceOwnerPriority,
): number {
  const signedRank = a.side * a.surfaceRank - b.side * b.surfaceRank;
  if (signedRank !== 0) return signedRank;
  const exactRank = a.surfaceRank - b.surfaceRank;
  if (exactRank !== 0) return exactRank;
  const material = a.materialOrientation - b.materialOrientation;
  if (material !== 0) return material;
  return a.side * a.faceId - b.side * b.faceId;
}

/** GLSLへそのまま渡せる共有uniform。値だけを書き換え、入れ物は使い回す。 */
export interface SurfaceOwnerBinding {
  enabled: { value: number };
  map: { value: THREE.Texture | null };
  resolution: { value: THREE.Vector2 };
}

/** owner textureをまだ持たない、無効状態のuniformを作る。 */
export function createSurfaceOwnerBinding(): SurfaceOwnerBinding {
  return {
    enabled: { value: 0 },
    map: { value: null },
    // 無効中でもshader内の除算が不定値にならない大きさから始める。
    resolution: { value: new THREE.Vector2(1, 1) },
  };
}

function validFaceId(faceId: number): boolean {
  return Number.isSafeInteger(faceId) && faceId >= 0;
}

/** 面IDを数値順に詰め、背景0と衝突しない1始まりのowner codeへ対応付ける。 */
export function createSurfaceOwnerCodes(faceIds: Iterable<number>): Map<number, number> {
  const unique = new Set<number>();
  for (const faceId of faceIds) {
    if (!validFaceId(faceId)) {
      throw new RangeError(`invalid surface owner face ID: ${faceId}`);
    }
    unique.add(faceId);
  }
  const sorted = [...unique].sort((a, b) => a - b);
  if (sorted.length > MAX_OWNER_CODE) {
    throw new RangeError("too many surface owner faces");
  }
  return new Map(sorted.map((faceId, index) => [faceId, index + 1]));
}

function checkedOwnerCode(code: number): number {
  if (!Number.isInteger(code) || code < 0 || code > MAX_OWNER_CODE) {
    throw new RangeError(`invalid surface owner code: ${code}`);
  }
  return code;
}

/** RGBA8 render targetへ書くlittle-endianの4 byte。 */
export function ownerCodeBytes(code: number): [number, number, number, number] {
  const value = checkedOwnerCode(code);
  return [
    value & 0xff,
    (value >>> 8) & 0xff,
    (value >>> 16) & 0xff,
    (value >>> 24) & 0xff,
  ];
}

/** 正規化Uint8 attributeやtextureの値と比較する0〜1のRGBA。 */
export function ownerCodeVector(code: number): THREE.Vector4 {
  const bytes = ownerCodeBytes(code);
  return new THREE.Vector4(
    bytes[0] / 255,
    bytes[1] / 255,
    bytes[2] / 255,
    bytes[3] / 255,
  );
}

/** 同じ面に属する三角形をまとめた、owner passの並べ替え単位。 */
export interface SurfaceOwnerBatch {
  readonly faceId: number;
  readonly ownerCode: number;
  /** 元のtriangleFaces/triangleLayersにおける三角形番号。 */
  readonly triangles: number[];
  /** この面に属する三角形の頂点index。3個ずつ並ぶ。 */
  readonly indices: number[];
  /** layer更新APIで書き換える現在の層番号。 */
  layer: number;
  /** 計算側が返す、同一深度専用の下→上の重なり順位。 */
  surfaceRank: number;
}

/** owner pass専用geometryと、視点ごとの並べ替えに必要な純粋データ。 */
export interface SurfaceOwnerSurface {
  readonly geometry: THREE.BufferGeometry;
  readonly position: THREE.BufferAttribute;
  /** xyz=depth plane係数、w=共通平面を使うとき1。 */
  readonly depthPlanes: THREE.BufferAttribute;
  /** 同じ支持平面に乗る面の組を表すRGBA8符号。全て0なら組無し。 */
  readonly groupTokens: THREE.BufferAttribute;
  readonly ownerCodes: Map<number, number>;
  readonly triangleFaces: number[];
  readonly triangleLayers: number[];
  /** pickerと同じ実体を共有する、面ID→重なり順位。 */
  readonly faceSurfaceRanks: Map<number, number>;
  readonly batches: SurfaceOwnerBatch[];
}

export interface CreateSurfaceOwnerSurfaceInput {
  /** 表示中の紙面と同じ、毎フレーム更新されるposition attribute。 */
  position: THREE.BufferAttribute;
  /** 頂点番号→面ID。position.count件必要。 */
  vertexFaces: ArrayLike<number>;
  /** 三角形index。3個で1枚。 */
  indices: ArrayLike<number>;
  /** 三角形番号→面ID。 */
  triangleFaces: ArrayLike<number>;
  /** 三角形番号→層。省略時は全て0。 */
  triangleLayers?: ArrayLike<number>;
  /** rigid/softで同じ符号を共有するときに渡す。 */
  ownerCodes?: ReadonlyMap<number, number>;
}

function copyNumbers(values: ArrayLike<number>): number[] {
  return Array.from({ length: values.length }, (_, index) => values[index]);
}

function finiteLayer(layer: number | undefined): number {
  return layer !== undefined && Number.isFinite(layer) ? layer : 0;
}

function makeIndexArray(indices: readonly number[]): Uint16Array | Uint32Array {
  let max = 0;
  for (const index of indices) {
    if (!Number.isSafeInteger(index) || index < 0) {
      throw new RangeError(`invalid surface owner vertex index: ${index}`);
    }
    if (index > max) max = index;
  }
  return max > 0xffff ? Uint32Array.from(indices) : Uint16Array.from(indices);
}

function copyOwnerCodes(
  triangleFaces: readonly number[],
  vertexFaces: ArrayLike<number>,
  supplied: ReadonlyMap<number, number> | undefined,
): Map<number, number> {
  const allFaces = [...triangleFaces, ...copyNumbers(vertexFaces)];
  if (supplied === undefined) return createSurfaceOwnerCodes(allFaces);

  const result = new Map<number, number>();
  const usedCodes = new Set<number>();
  for (const [faceId, code] of supplied) {
    if (!validFaceId(faceId)) {
      throw new RangeError(`invalid surface owner face ID: ${faceId}`);
    }
    const checked = checkedOwnerCode(code);
    if (checked === SURFACE_OWNER_BACKGROUND_CODE) {
      throw new RangeError("surface owner code 0 is reserved for the background");
    }
    if (usedCodes.has(checked)) {
      throw new RangeError(`duplicate surface owner code: ${checked}`);
    }
    result.set(faceId, checked);
    usedCodes.add(checked);
  }
  for (const faceId of allFaces) {
    if (!result.has(faceId)) {
      throw new RangeError(`missing surface owner code for face ${faceId}`);
    }
  }
  return result;
}

function refreshBatchLayers(surface: SurfaceOwnerSurface): void {
  for (const batch of surface.batches) {
    const triangle = batch.triangles[0];
    batch.layer = triangle === undefined ? 0 : finiteLayer(surface.triangleLayers[triangle]);
  }
}

/**
 * 表示geometryとは別のowner用geometryを作る。positionだけは同じBufferAttributeを
 * 共有し、紙の形が更新されたときにコピー無しでowner passへ反映される。
 */
export function createSurfaceOwnerSurface(
  input: CreateSurfaceOwnerSurfaceInput,
): SurfaceOwnerSurface {
  const indices = copyNumbers(input.indices);
  if (indices.length % 3 !== 0) {
    throw new RangeError("surface owner indices must contain complete triangles");
  }
  const triangleCount = indices.length / 3;
  if (input.triangleFaces.length !== triangleCount) {
    throw new RangeError("surface owner triangleFaces length does not match indices");
  }
  if (input.triangleLayers && input.triangleLayers.length !== triangleCount) {
    throw new RangeError("surface owner triangleLayers length does not match indices");
  }
  if (input.vertexFaces.length !== input.position.count) {
    throw new RangeError("surface owner vertexFaces length does not match position");
  }

  const triangleFaces = copyNumbers(input.triangleFaces);
  for (const faceId of triangleFaces) {
    if (!validFaceId(faceId)) {
      throw new RangeError(`invalid surface owner face ID: ${faceId}`);
    }
  }
  const triangleLayers = input.triangleLayers
    ? copyNumbers(input.triangleLayers).map(finiteLayer)
    : new Array<number>(triangleCount).fill(0);
  const ownerCodes = copyOwnerCodes(
    triangleFaces,
    input.vertexFaces,
    input.ownerCodes,
  );

  const tokens = new Uint8Array(input.position.count * 4);
  for (let vertex = 0; vertex < input.position.count; vertex++) {
    const faceId = input.vertexFaces[vertex];
    if (!validFaceId(faceId)) {
      throw new RangeError(`invalid surface owner face ID: ${faceId}`);
    }
    const code = ownerCodes.get(faceId);
    if (code === undefined) {
      throw new RangeError(`missing surface owner code for face ${faceId}`);
    }
    tokens.set(ownerCodeBytes(code), vertex * 4);
  }

  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", input.position);
  geometry.setAttribute(
    "surfaceOwnerToken",
    new THREE.Uint8BufferAttribute(tokens, 4, true),
  );
  const depthPlanes = new THREE.BufferAttribute(
    new Float32Array(input.position.count * 4),
    4,
  );
  depthPlanes.setUsage(THREE.DynamicDrawUsage);
  geometry.setAttribute(SURFACE_OWNER_DEPTH_PLANE_ATTRIBUTE, depthPlanes);
  const groupTokens = new THREE.Uint8BufferAttribute(
    new Uint8Array(input.position.count * 4),
    4,
    true,
  );
  groupTokens.setUsage(THREE.DynamicDrawUsage);
  geometry.setAttribute(SURFACE_OWNER_GROUP_ATTRIBUTE, groupTokens);
  const index = new THREE.BufferAttribute(makeIndexArray(indices), 1);
  index.setUsage(THREE.DynamicDrawUsage);
  geometry.setIndex(index);

  const byFace = new Map<number, SurfaceOwnerBatch>();
  for (let triangle = 0; triangle < triangleCount; triangle++) {
    const faceId = triangleFaces[triangle];
    let batch = byFace.get(faceId);
    if (!batch) {
      batch = {
        faceId,
        ownerCode: ownerCodes.get(faceId)!,
        triangles: [],
        indices: [],
        layer: triangleLayers[triangle],
        surfaceRank: 0,
      };
      byFace.set(faceId, batch);
    }
    batch.triangles.push(triangle);
    const at = triangle * 3;
    batch.indices.push(indices[at], indices[at + 1], indices[at + 2]);
  }

  const surface: SurfaceOwnerSurface = {
    geometry,
    position: input.position,
    depthPlanes,
    groupTokens,
    ownerCodes,
    triangleFaces,
    triangleLayers,
    faceSurfaceRanks: new Map([...byFace.keys()].map((faceId) => [faceId, 0])),
    batches: [...byFace.values()].sort((a, b) => a.faceId - b.faceId),
  };
  refreshBatchLayers(surface);
  return surface;
}

/** 面IDごとのlayerへ差し替える。Mapに無い面は古い値を残さず0へ戻す。 */
export function updateSurfaceOwnerFaceLayers(
  surface: SurfaceOwnerSurface,
  layers: ReadonlyMap<number, number>,
): void {
  for (let triangle = 0; triangle < surface.triangleFaces.length; triangle++) {
    surface.triangleLayers[triangle] = finiteLayer(
      layers.get(surface.triangleFaces[triangle]),
    );
  }
  refreshBatchLayers(surface);
}

/** soft meshなど、三角形と同じ並びで届くlayerへ差し替える。 */
export function updateSurfaceOwnerTriangleLayers(
  surface: SurfaceOwnerSurface,
  layers: ArrayLike<number>,
): void {
  if (layers.length !== surface.triangleLayers.length) {
    throw new RangeError("surface owner triangle layer update has a wrong length");
  }
  for (let triangle = 0; triangle < layers.length; triangle++) {
    surface.triangleLayers[triangle] = finiteLayer(layers[triangle]);
  }
  refreshBatchLayers(surface);
}

/** 面IDごとの重なり順位へ差し替える。Mapに無い面は0へ戻す。 */
export function updateSurfaceOwnerFaceRanks(
  surface: SurfaceOwnerSurface,
  ranks: ReadonlyMap<number, number>,
): void {
  // pickSurfaceはこのMap実体を保持するため、置換せず内容だけを更新する。
  surface.faceSurfaceRanks.clear();
  for (const batch of surface.batches) {
    batch.surfaceRank = finiteLayer(ranks.get(batch.faceId));
    surface.faceSurfaceRanks.set(batch.faceId, batch.surfaceRank);
  }
}

const workA = new THREE.Vector3();
const workB = new THREE.Vector3();
const workC = new THREE.Vector3();
const workAB = new THREE.Vector3();
const workAC = new THREE.Vector3();
const workNormal = new THREE.Vector3();
const workCenter = new THREE.Vector3();
const workCameraPosition = new THREE.Vector3();
const workProjectedA = new THREE.Vector3();

interface BatchViewOrder {
  batch: SurfaceOwnerBatch;
  priority: {
    faceId: number;
    surfaceRank: number;
    side: 1 | -1;
    materialOrientation: 1 | -1;
  };
  planeNormal: THREE.Vector3;
  planeDistance: number;
  planar: boolean;
}

// 面数分の並べ替え要素は生成時に一度だけ確保し、毎フレーム使い回す。
const viewOrders = new WeakMap<SurfaceOwnerSurface, BatchViewOrder[]>();

function canonicalize(normal: THREE.Vector3): boolean {
  const ax = Math.abs(normal.x);
  const ay = Math.abs(normal.y);
  const az = Math.abs(normal.z);
  const component = ax >= ay && ax >= az ? normal.x : ay >= az ? normal.y : normal.z;
  if (component < 0) {
    normal.multiplyScalar(-1);
    return true;
  }
  return false;
}

/** 現在のpositionからcanonical法線とカメラ側を毎回求める。 */
function updateBatchViewOrder(
  surface: SurfaceOwnerSurface,
  item: BatchViewOrder,
  cameraPosition: THREE.Vector3,
): void {
  const { batch } = item;
  workNormal.set(0, 0, 0);
  workCenter.set(0, 0, 0);
  let points = 0;
  for (let i = 0; i < batch.indices.length; i += 3) {
    workA.fromBufferAttribute(surface.position, batch.indices[i]);
    workB.fromBufferAttribute(surface.position, batch.indices[i + 1]);
    workC.fromBufferAttribute(surface.position, batch.indices[i + 2]);
    workAB.subVectors(workB, workA);
    workAC.subVectors(workC, workA);
    workNormal.add(workAB.cross(workAC));
    workCenter.add(workA).add(workB).add(workC);
    points += 3;
  }
  if (points > 0) workCenter.multiplyScalar(1 / points);
  const normalIsValid = workNormal.lengthSq() > Number.EPSILON;
  if (!normalIsValid) workNormal.set(0, 0, 1);
  else workNormal.normalize();
  // The paper is authored in the XY plane, so the rotated material front keeps
  // the sign of its current z component.  This is geometry, not Face3D.mirrored.
  item.priority.materialOrientation = workNormal.z < 0 ? -1 : 1;
  canonicalize(workNormal);
  item.priority.side =
    workNormal.dot(workA.subVectors(cameraPosition, workCenter)) >= 0 ? 1 : -1;
  item.priority.faceId = batch.faceId;
  item.priority.surfaceRank = batch.surfaceRank;
  item.planeNormal.copy(workNormal);
  item.planeDistance = workNormal.dot(workCenter);
  let maximumPlaneError = 0;
  for (const vertex of batch.indices) {
    workA.fromBufferAttribute(surface.position, vertex);
    maximumPlaneError = Math.max(
      maximumPlaneError,
      Math.abs(workNormal.dot(workA) - item.planeDistance),
    );
  }
  item.planar =
    normalIsValid && maximumPlaneError <= SURFACE_OWNER_PLANARITY_EPSILON;
}

interface CanonicalDepthPlane {
  readonly x: number;
  readonly y: number;
  readonly constant: number;
}

const workViewProjection = new THREE.Matrix4();
const workInverseTranspose = new THREE.Matrix4();
const workClipPlane = new THREE.Vector4();

function canonicalDepthPlane(
  normal: THREE.Vector3,
  distance: number,
  camera: THREE.Camera,
): CanonicalDepthPlane | null {
  workViewProjection.multiplyMatrices(
    camera.projectionMatrix,
    camera.matrixWorldInverse,
  );
  workInverseTranspose.copy(workViewProjection).invert().transpose();
  workClipPlane
    .set(normal.x, normal.y, normal.z, -distance)
    .applyMatrix4(workInverseTranspose);
  const scale = Math.max(
    Math.abs(workClipPlane.x),
    Math.abs(workClipPlane.y),
    Math.abs(workClipPlane.z),
    Math.abs(workClipPlane.w),
  );
  if (
    !Number.isFinite(scale) ||
    scale === 0 ||
    Math.abs(workClipPlane.z) <= Number.EPSILON * scale * 16
  ) {
    return null;
  }
  // world planeをinverse-transposeでclip spaceへ移し、
  // z_ndcをx/yの式へ解いて[0,1]のwindow depthへ直す。
  const x = Math.fround((-0.5 * workClipPlane.x) / workClipPlane.z);
  const y = Math.fround((-0.5 * workClipPlane.y) / workClipPlane.z);
  const constant = Math.fround(0.5 - (0.5 * workClipPlane.w) / workClipPlane.z);
  if (![x, y, constant].every(Number.isFinite)) return null;
  return { x, y, constant };
}

function batchFitsPlane(
  surface: SurfaceOwnerSurface,
  batch: SurfaceOwnerBatch,
  normal: THREE.Vector3,
  distance: number,
): boolean {
  return batch.indices.every((vertex) => {
    workA.fromBufferAttribute(surface.position, vertex);
    return (
      Math.abs(normal.dot(workA) - distance) <=
      SURFACE_OWNER_COPLANAR_EPSILON
    );
  });
}

function depthPlaneFitsGroup(
  surface: SurfaceOwnerSurface,
  group: readonly BatchViewOrder[],
  plane: CanonicalDepthPlane,
  camera: THREE.Camera,
): boolean {
  for (const { batch } of group) {
    for (const vertex of batch.indices) {
      workProjectedA.fromBufferAttribute(surface.position, vertex).project(camera);
      const originalDepth = (workProjectedA.z + 1) * 0.5;
      const sharedDepth =
        plane.x * workProjectedA.x +
        plane.y * workProjectedA.y +
        plane.constant;
      if (
        !Number.isFinite(originalDepth) ||
        !Number.isFinite(sharedDepth) ||
        Math.abs(sharedDepth - originalDepth) >
          SURFACE_OWNER_DEPTH_TOLERANCE
      ) {
        return false;
      }
    }
  }
  return true;
}

/**
 * 共平面の別triangle分割へ、同じNDC深度平面と同じ組符号を割り当てる。
 *
 * 深度平面はshaderの深度を一致させるための最善手だが、実機のGPUでは一致を
 * 前提にできないことが分かったため、平面がまとまった面には必ず組符号も配る。
 * 組符号は整数の一致だけで「同じ支持平面の仲間」を表すので、補間の丸めに左右されない。
 * 非平面batchはw=0・組符号0のままにし、shaderで従来のgl_FragCoord.zへ戻す。
 */
function updateSurfaceOwnerDepthPlanes(
  surface: SurfaceOwnerSurface,
  orders: readonly BatchViewOrder[],
  camera: THREE.Camera,
): void {
  (surface.depthPlanes.array as Float32Array).fill(0);
  (surface.groupTokens.array as Uint8Array).fill(0);
  const groups: {
    normal: THREE.Vector3;
    distance: number;
    members: BatchViewOrder[];
  }[] = [];
  const deterministicOrders = [...orders].sort(
    (left, right) => left.batch.faceId - right.batch.faceId,
  );
  for (const item of deterministicOrders) {
    if (!item.planar) continue;
    const group = groups.find((candidate) => {
      const sameDirection = candidate.normal.dot(item.planeNormal) >= 0;
      workNormal
        .copy(item.planeNormal)
        .multiplyScalar(sameDirection ? 1 : -1);
      const alignedDistance = sameDirection
        ? item.planeDistance
        : -item.planeDistance;
      return (
        candidate.normal.distanceToSquared(workNormal) <=
          SURFACE_OWNER_NORMAL_EPSILON ** 2 &&
        Math.abs(candidate.distance - alignedDistance) <=
          SURFACE_OWNER_COPLANAR_EPSILON &&
        batchFitsPlane(
          surface,
          item.batch,
          candidate.normal,
          candidate.distance,
        )
      );
    });
    if (group) group.members.push(item);
    else {
      groups.push({
        normal: item.planeNormal.clone(),
        distance: item.planeDistance,
        members: [item],
      });
    }
  }

  // 支持平面ごとに組符号を配る。同じ平面の面は同じ符号、別の平面の面は別の符号。
  // 深度平面が使えない場合(下の残差guardで却下された場合を含む)でも、
  // 「最前面と同じ平面の面だけが候補」という判定が整数の一致だけで成り立つ。
  const groupBytes = surface.groupTokens.array as Uint8Array;
  let groupCode = SURFACE_OWNER_BACKGROUND_CODE;
  for (const group of groups) {
    groupCode += 1;
    const bytes = ownerCodeBytes(groupCode);
    for (const { batch } of group.members) {
      // 正規化Uint8 attributeなので、面ID符号と同じく生のbyteをそのまま書く。
      for (const vertex of batch.indices) groupBytes.set(bytes, vertex * 4);
    }
  }
  surface.groupTokens.needsUpdate = true;

  for (const group of groups) {
    const triangleCount = group.members.reduce(
      (count, item) => count + item.batch.indices.length / 3,
      0,
    );
    if (triangleCount <= 1) continue;
    const representative = canonicalDepthPlane(
      group.normal,
      group.distance,
      camera,
    );
    if (
      !representative ||
      !depthPlaneFitsGroup(surface, group.members, representative, camera)
    ) {
      continue;
    }
    for (const { batch } of group.members) {
      for (const vertex of batch.indices) {
        surface.depthPlanes.setXYZW(
          vertex,
          representative.x,
          representative.y,
          representative.constant,
          1,
        );
      }
    }
  }
  surface.depthPlanes.needsUpdate = true;
}

/**
 * owner passのindexを視点側の優先順へ並べ直す。同じ深度はLEQUALの後描きが勝つため、
 * 実深度はGPUの第1・第2passで先に限定し、本当に同じ深度の候補だけを計算側の
 * surfaceRankで並べる。mirroredは参照しない。同rankは古いsnapshotや不正入力だけの
 * 防御経路で、現在の材質winding、面IDの順に決定的な全順序へ落とす。
 */
export function orderSurfaceOwner(
  surface: SurfaceOwnerSurface,
  camera: THREE.Camera,
): void {
  camera.getWorldPosition(workCameraPosition);
  let ordered = viewOrders.get(surface);
  if (!ordered) {
    ordered = surface.batches.map((batch) => ({
      batch,
      priority: {
        faceId: batch.faceId,
        surfaceRank: batch.surfaceRank,
        side: 1,
        materialOrientation: 1,
      },
      planeNormal: new THREE.Vector3(0, 0, 1),
      planeDistance: 0,
      planar: false,
    }));
    viewOrders.set(surface, ordered);
  }
  for (const item of ordered) {
    updateBatchViewOrder(surface, item, workCameraPosition);
  }
  updateSurfaceOwnerDepthPlanes(surface, ordered, camera);
  ordered.sort((a, b) => {
    const priority = compareSurfaceOwnerPriority(a.priority, b.priority);
    if (priority !== 0) return priority;
    return (
      a.priority.side * a.batch.ownerCode - b.priority.side * b.batch.ownerCode
    );
  });

  const index = surface.geometry.getIndex();
  if (!index) throw new Error("surface owner geometry has no index");
  let at = 0;
  for (const { batch } of ordered) {
    for (const vertex of batch.indices) index.setX(at++, vertex);
  }
  index.needsUpdate = true;
}

/** owner geometryだけを破棄する。共有position attribute自体は破棄・置換しない。 */
export function disposeSurfaceOwnerSurface(surface: SurfaceOwnerSurface): void {
  viewOrders.delete(surface);
  surface.geometry.dispose();
}
