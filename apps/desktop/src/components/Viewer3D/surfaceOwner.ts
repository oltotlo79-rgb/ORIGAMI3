// 1画素につき最前面の紙を1枚だけ選ぶための、Three.jsに依存する表示用データ。
// WebGLRendererやDOMには触れず、面IDの符号化・owner用geometry・描画順だけを扱う。
//
// 既定視点の24-bit depthは紙の位置で約1.58269e-5を区別でき、層間隔0.0002は
// その12.64倍ある。従って深度の精度は足りており、根本原因ではなかった。
// 角度操作ではto_frame3dが全ての面をlayer=0で返し得るため、重なった面の実際の
// 位置差が0になることが原因である。同値はlayer→面IDの順で決め、線だけでなく
// 紙の色も同じownerへ絞らなければ、同一深度の面同士の縞模様は消えない。

import * as THREE from "three";

/** owner render targetの0は、紙が無い背景として予約する。 */
export const SURFACE_OWNER_BACKGROUND_CODE = 0;

const MAX_OWNER_CODE = 0xffff_ffff;

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
}

/** owner pass専用geometryと、視点ごとの並べ替えに必要な純粋データ。 */
export interface SurfaceOwnerSurface {
  readonly geometry: THREE.BufferGeometry;
  readonly position: THREE.BufferAttribute;
  readonly ownerCodes: Map<number, number>;
  readonly triangleFaces: number[];
  readonly triangleLayers: number[];
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
    ownerCodes,
    triangleFaces,
    triangleLayers,
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

const workA = new THREE.Vector3();
const workB = new THREE.Vector3();
const workC = new THREE.Vector3();
const workAB = new THREE.Vector3();
const workAC = new THREE.Vector3();
const workNormal = new THREE.Vector3();
const workCenter = new THREE.Vector3();
const workCameraPosition = new THREE.Vector3();

interface BatchViewOrder {
  batch: SurfaceOwnerBatch;
  side: 1 | -1;
}

// 面数分の並べ替え要素は生成時に一度だけ確保し、毎フレーム使い回す。
const viewOrders = new WeakMap<SurfaceOwnerSurface, BatchViewOrder[]>();

function canonicalize(normal: THREE.Vector3): void {
  const ax = Math.abs(normal.x);
  const ay = Math.abs(normal.y);
  const az = Math.abs(normal.z);
  const component = ax >= ay && ax >= az ? normal.x : ay >= az ? normal.y : normal.z;
  if (component < 0) normal.multiplyScalar(-1);
}

/** 現在のpositionから面の法線と中心を毎回求め、カメラが平面のどちら側か返す。 */
function batchSide(
  surface: SurfaceOwnerSurface,
  batch: SurfaceOwnerBatch,
  cameraPosition: THREE.Vector3,
): 1 | -1 {
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
  if (workNormal.lengthSq() <= Number.EPSILON) workNormal.set(0, 0, 1);
  else workNormal.normalize();
  canonicalize(workNormal);
  return workNormal.dot(workA.subVectors(cameraPosition, workCenter)) >= 0 ? 1 : -1;
}

/**
 * owner passのindexを視点側の優先順へ並べ直す。同じ深度はLEQUALの後描きが勝つため、
 * +側ではlayer/faceId/codeの大きい面、-側では小さい面が最後になる。
 */
export function orderSurfaceOwner(
  surface: SurfaceOwnerSurface,
  camera: THREE.Camera,
): void {
  camera.getWorldPosition(workCameraPosition);
  let ordered = viewOrders.get(surface);
  if (!ordered) {
    ordered = surface.batches.map((batch) => ({ batch, side: 1 }));
    viewOrders.set(surface, ordered);
  }
  for (const item of ordered) {
    item.side = batchSide(surface, item.batch, workCameraPosition);
  }
  ordered.sort((a, b) => {
    const layer = a.side * a.batch.layer - b.side * b.batch.layer;
    if (layer !== 0) return layer;
    const face = a.side * a.batch.faceId - b.side * b.batch.faceId;
    if (face !== 0) return face;
    const code = a.side * a.batch.ownerCode - b.side * b.batch.ownerCode;
    return code;
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
