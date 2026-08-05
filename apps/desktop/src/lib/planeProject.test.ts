// 画面座標→畳み平面(z=0)投影のテスト。
// カメラで平面上の点を画面へ投影し、同じ画面位置から元の点が返ることを確かめる。

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { planeRadius, screenToPlane } from "./planeProject";

const WIDTH = 800;
const HEIGHT = 600;

/** 紙(1×1)を斜め上から見下ろすカメラ */
function makeCamera(target: [number, number, number] = [0.5, 0.5, 0]) {
  const camera = new THREE.PerspectiveCamera(45, WIDTH / HEIGHT, 0.01, 100);
  camera.position.set(0.5, -1.2, 1.4);
  camera.lookAt(...target);
  camera.updateMatrixWorld(true);
  camera.updateProjectionMatrix();
  return camera;
}

/** 世界座標を画面座標(canvas左上基準のpx)へ(hingePickerと同じ式) */
function toScreen(camera: THREE.Camera, p: THREE.Vector3): [number, number] {
  const v = p.clone().project(camera);
  return [((v.x + 1) / 2) * WIDTH, ((1 - v.y) / 2) * HEIGHT];
}

describe("screenToPlane", () => {
  it("平面上の点を画面へ投影してから戻すと元の位置になる", () => {
    const camera = makeCamera();
    for (const [x, y] of [
      [0.0, 0.0],
      [0.5, 0.5],
      [1.0, 0.25],
      [0.2, 0.9],
    ]) {
      const [sx, sy] = toScreen(camera, new THREE.Vector3(x, y, 0));
      const back = screenToPlane(camera, WIDTH, HEIGHT, sx, sy);
      expect(back).not.toBeNull();
      expect(back?.[0]).toBeCloseTo(x, 6);
      expect(back?.[1]).toBeCloseTo(y, 6);
    }
  });

  it("平面と交わらない向き(上を向いたカメラ)ではnullを返す", () => {
    const camera = makeCamera([0.5, 0.5, 8]);
    expect(screenToPlane(camera, WIDTH, HEIGHT, WIDTH / 2, HEIGHT / 2)).toBeNull();
  });

  it("大きさのない画面ではnullを返す", () => {
    const camera = makeCamera();
    expect(screenToPlane(camera, 0, 0, 0, 0)).toBeNull();
  });
});

describe("planeRadius", () => {
  it("画面上の距離を平面上の距離へ直す(px幅に比例して増える)", () => {
    const camera = makeCamera();
    const r10 = planeRadius(camera, WIDTH, HEIGHT, 400, 300, 10, 0.02);
    const r20 = planeRadius(camera, WIDTH, HEIGHT, 400, 300, 20, 0.02);
    expect(r10).toBeGreaterThan(0);
    expect(r10).toBeLessThan(0.2); // 紙(長辺1.0)に対して十分小さい
    expect(r20).toBeGreaterThan(r10);
  });

  it("平面へ投影できない位置では代わりの値を返す", () => {
    const camera = makeCamera([0.5, 0.5, 8]);
    expect(planeRadius(camera, WIDTH, HEIGHT, 400, 300, 10, 0.02)).toBe(0.02);
  });
});
