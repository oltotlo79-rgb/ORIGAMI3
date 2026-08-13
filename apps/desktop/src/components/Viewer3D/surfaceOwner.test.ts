import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import {
  SURFACE_OWNER_BACKGROUND_CODE,
  createSurfaceOwnerBinding,
  createSurfaceOwnerCodes,
  createSurfaceOwnerSurface,
  disposeSurfaceOwnerSurface,
  orderSurfaceOwner,
  ownerCodeBytes,
  ownerCodeVector,
  updateSurfaceOwnerFaceLayers,
  updateSurfaceOwnerTriangleLayers,
} from "./surfaceOwner";

function positionOfTwoOverlappingFaces(): THREE.BufferAttribute {
  return new THREE.BufferAttribute(
    new Float32Array([
      0, 0, 0,
      1, 0, 0,
      0, 1, 0,
      0, 0, 0,
      1, 0, 0,
      0, 1, 0,
    ]),
    3,
  );
}

function cameraAt(z: number): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
  camera.position.set(0.25, 0.25, z);
  camera.lookAt(0.25, 0.25, 0);
  camera.updateMatrixWorld(true);
  return camera;
}

function indicesOf(surface: ReturnType<typeof createSurfaceOwnerSurface>): number[] {
  const index = surface.geometry.getIndex();
  if (!index) return [];
  return Array.from({ length: index.count }, (_, at) => index.getX(at));
}

function overlappingSurface(layers: [number, number] = [0, 0]) {
  return createSurfaceOwnerSurface({
    position: positionOfTwoOverlappingFaces(),
    vertexFaces: [10, 10, 10, 20, 20, 20],
    indices: [0, 1, 2, 3, 4, 5],
    triangleFaces: [10, 20],
    triangleLayers: layers,
  });
}

describe("surface owner code", () => {
  it("背景を0に予約し、重複した面IDを数値順の1始まりcodeへ詰める", () => {
    expect(SURFACE_OWNER_BACKGROUND_CODE).toBe(0);
    expect([...createSurfaceOwnerCodes([20, 3, 20, 9])]).toEqual([
      [3, 1],
      [9, 2],
      [20, 3],
    ]);
  });

  it("32-bit codeをlittle-endian RGBA8と正規化Vector4へ直す", () => {
    expect(ownerCodeBytes(0)).toEqual([0, 0, 0, 0]);
    expect(ownerCodeBytes(0x7856_3412)).toEqual([0x12, 0x34, 0x56, 0x78]);
    expect(ownerCodeBytes(0xffff_ffff)).toEqual([255, 255, 255, 255]);
    expect(ownerCodeVector(0x0403_0201).toArray()).toEqual([
      1 / 255,
      2 / 255,
      3 / 255,
      4 / 255,
    ]);
  });

  it("texture未接続のbindingは無効で、安全な解像度から始まる", () => {
    const binding = createSurfaceOwnerBinding();
    expect(binding.enabled.value).toBe(0);
    expect(binding.map.value).toBeNull();
    expect(binding.resolution.value).toBeInstanceOf(THREE.Vector2);
    expect(binding.resolution.value.toArray()).toEqual([1, 1]);
  });
});

describe("createSurfaceOwnerSurface", () => {
  it("positionを共有し、正規化owner属性・動的index・面batchを別geometryへ作る", () => {
    const position = positionOfTwoOverlappingFaces();
    const indices = [0, 1, 2, 3, 4, 5];
    const triangleFaces = [20, 10];
    const triangleLayers = [4, 2];
    const surface = createSurfaceOwnerSurface({
      position,
      vertexFaces: [20, 20, 20, 10, 10, 10],
      indices,
      triangleFaces,
      triangleLayers,
    });

    expect(surface.geometry.getAttribute("position")).toBe(position);
    expect(surface.position).toBe(position);
    const token = surface.geometry.getAttribute("surfaceOwnerToken");
    expect(token).toBeInstanceOf(THREE.Uint8BufferAttribute);
    expect(token.itemSize).toBe(4);
    expect(token.normalized).toBe(true);
    // 面ID 10→code 1、面ID 20→code 2。最初の3頂点は面20。
    expect(Array.from(token.array)).toEqual([
      2, 0, 0, 0,
      2, 0, 0, 0,
      2, 0, 0, 0,
      1, 0, 0, 0,
      1, 0, 0, 0,
      1, 0, 0, 0,
    ]);
    expect(surface.geometry.getIndex()?.usage).toBe(THREE.DynamicDrawUsage);
    expect(surface.batches.map((batch) => batch.faceId)).toEqual([10, 20]);
    expect(surface.batches.map((batch) => batch.layer)).toEqual([2, 4]);
    expect(surface.triangleFaces).toEqual([20, 10]);
    expect(surface.triangleLayers).toEqual([4, 2]);

    // 入力配列を書き換えてもowner側の記録・indexは変わらない。
    indices[0] = 99;
    triangleFaces[0] = 99;
    triangleLayers[0] = 99;
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(surface.triangleFaces).toEqual([20, 10]);
    expect(surface.triangleLayers).toEqual([4, 2]);
  });

  it("渡されたowner codeをrigid/soft間で共有できる", () => {
    const codes = createSurfaceOwnerCodes([10, 20, 30]);
    const surface = createSurfaceOwnerSurface({
      position: positionOfTwoOverlappingFaces(),
      vertexFaces: [10, 10, 10, 20, 20, 20],
      indices: [0, 1, 2, 3, 4, 5],
      triangleFaces: [10, 20],
      ownerCodes: codes,
    });
    expect([...surface.ownerCodes]).toEqual([...codes]);
  });
});

describe("orderSurfaceOwner", () => {
  it("同じlayerなら+側は大きいfaceIdを後勝ち、-側は逆にする", () => {
    const surface = overlappingSurface();
    orderSurfaceOwner(surface, cameraAt(2));
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);

    orderSurfaceOwner(surface, cameraAt(-2));
    expect(indicesOf(surface)).toEqual([3, 4, 5, 0, 1, 2]);
  });

  it("異なるlayerは+側で大きい層、-側で小さい層を後勝ちにする", () => {
    const surface = overlappingSurface([3, 1]);
    orderSurfaceOwner(surface, cameraAt(2));
    expect(indicesOf(surface)).toEqual([3, 4, 5, 0, 1, 2]);

    orderSurfaceOwner(surface, cameraAt(-2));
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);
  });

  it("面layer・三角形layerの更新を次の並べ替えへ反映する", () => {
    const surface = overlappingSurface();
    updateSurfaceOwnerFaceLayers(surface, new Map([[10, 5], [20, 1]]));
    expect(surface.triangleLayers).toEqual([5, 1]);
    expect(surface.batches.map((batch) => batch.layer)).toEqual([5, 1]);
    orderSurfaceOwner(surface, cameraAt(2));
    expect(indicesOf(surface)).toEqual([3, 4, 5, 0, 1, 2]);

    updateSurfaceOwnerTriangleLayers(surface, [0, 7]);
    expect(surface.triangleLayers).toEqual([0, 7]);
    expect(surface.batches.map((batch) => batch.layer)).toEqual([0, 7]);
    orderSurfaceOwner(surface, cameraAt(2));
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);
  });

  it("positionの現座標から法線を取り直す", () => {
    const surface = overlappingSurface();
    // 両面を+y法線の壁へ動かす。canonical normalも+yとなり、+y視点で昇順になる。
    const moved = surface.position.array as Float32Array;
    const wall = [
      0, 0, 0,
      0, 0, 1,
      1, 0, 0,
      0, 0, 0,
      0, 0, 1,
      1, 0, 0,
    ];
    moved.set(wall);
    surface.position.needsUpdate = true;
    const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
    camera.position.set(0.25, 2, 0.25);
    camera.updateMatrixWorld(true);
    orderSurfaceOwner(surface, camera);
    expect(indicesOf(surface)).toEqual([0, 1, 2, 3, 4, 5]);
  });
});

describe("disposeSurfaceOwnerSurface", () => {
  it("owner geometryだけを破棄し、共有positionを置き換えない", () => {
    const position = positionOfTwoOverlappingFaces();
    const surface = createSurfaceOwnerSurface({
      position,
      vertexFaces: [10, 10, 10, 20, 20, 20],
      indices: [0, 1, 2, 3, 4, 5],
      triangleFaces: [10, 20],
    });
    const dispose = vi.spyOn(surface.geometry, "dispose");

    disposeSurfaceOwnerSurface(surface);

    expect(dispose).toHaveBeenCalledTimes(1);
    expect(surface.position).toBe(position);
    expect(surface.geometry.getAttribute("position")).toBe(position);
  });
});
