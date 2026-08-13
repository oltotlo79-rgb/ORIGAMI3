// 3Dビューのヒンジ拾い上げ(純関数)のテスト。
// クリック位置のしきい値、重なったときの手前優先を確かめる。
// (線分そのものの作り方はsceneBuilder.test.tsのbuildTopologyで確認する)

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  pickFace,
  pickHinge,
  pickHingeSegment,
  pickPaper,
  type HingeSegment,
  type PaperPickSurface,
} from "./hingePicker";

/** 原点を正面から見るカメラ(画面200×200px) */
function makeCamera(): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
  camera.position.set(0, 0, 2);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  return camera;
}

/** 原点を裏面から見るカメラ */
function makeBackCamera(): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
  camera.position.set(0, 0, -2);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  return camera;
}

function segment(
  edgeId: number,
  a: number[],
  b: number[],
  ownerFace?: number,
  layer?: number,
): HingeSegment {
  return {
    edgeId,
    a: new THREE.Vector3(a[0], a[1], a[2]),
    b: new THREE.Vector3(b[0], b[1], b[2]),
    ownerFace,
    layer,
  };
}

/** 画面中央で完全に重なる三角形を面ごとに1枚ずつ作る */
function makeOverlappingSurface(
  triangleFaceIds: number[],
  triangleLayers: number[],
): PaperPickSurface {
  const frontWinding = [-1, -1, 0, 1, -1, 0, 0, 1, 0];
  const backWinding = [-1, -1, 0, 0, 1, 0, 1, -1, 0];
  const positions = triangleFaceIds.flatMap((_, index) =>
    index % 2 === 0 ? backWinding : frontWinding,
  );
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(positions, 3),
  );
  const mesh = new THREE.Mesh(
    geometry,
    new THREE.MeshBasicMaterial({ side: THREE.DoubleSide }),
  );
  mesh.updateMatrixWorld(true);
  return { mesh, triangleFaceIds, triangleLayers };
}

function disposeSurface(surface: PaperPickSurface): void {
  surface.mesh.geometry.dispose();
  const materials = Array.isArray(surface.mesh.material)
    ? surface.mesh.material
    : [surface.mesh.material];
  for (const material of materials) material.dispose();
}

describe("pickHinge", () => {
  const camera = makeCamera();
  // 画面中央を通る横向きの線分(z=0)
  const segments = [segment(1, [-0.5, 0, 0], [0.5, 0, 0])];

  it("しきい値の内側をクリックすると拾い、外側なら拾わない", () => {
    expect(pickHinge(segments, camera, 200, 200, 100, 104)).toBe(1);
    expect(pickHinge(segments, camera, 200, 200, 100, 130)).toBeNull();
  });

  it("線分の外側(端点より先)は距離が離れるので拾わない", () => {
    // 線分は画面のごく一部にしか映らないため、右端は端点から遠い
    expect(pickHinge(segments, camera, 200, 200, 199, 100)).toBeNull();
  });

  it("重なって見える折り線は手前(カメラに近い方)を選ぶ", () => {
    const far = segment(1, [-0.5, 0, 0], [0.5, 0, 0]);
    const near = segment(2, [-0.5, 0, 0.5], [0.5, 0, 0.5]); // カメラ側
    expect(pickHinge([far, near], camera, 200, 200, 100, 100)).toBe(2);
    // 並び順に関わらず同じ結果になる
    expect(pickHinge([near, far], camera, 200, 200, 100, 100)).toBe(2);
  });

  it("層操作用にはIDだけでなく、選んだ既存折り目の正確な線分を返す", () => {
    const far = segment(1, [-0.5, 0, 0], [0.5, 0, 0]);
    const near = segment(2, [-0.5, 0, 0.5], [0.5, 0, 0.5]);

    expect(pickHingeSegment([far, near], camera, 200, 200, 100, 100)).toBe(near);
  });

  it("わずかに手前の方が遠い場合でも、0.5px刻みで同程度なら手前を選ぶ", () => {
    // 奥の線はクリック位置ちょうど、手前の線は0.2pxだけ下にずれている。
    // 1本ずつ「今より近いか」で比べると、並び順によって奥が残ってしまう
    const far = segment(1, [-0.5, 0, 0], [0.5, 0, 0]);
    const nearOffset = 0.2 * (2 * Math.tan((45 * Math.PI) / 360) * 1.5) / 200;
    const near = segment(
      2,
      [-0.5, -nearOffset, 0.5],
      [0.5, -nearOffset, 0.5],
    ); // カメラ側で0.2pxほど下
    expect(pickHinge([far, near], camera, 200, 200, 100, 100)).toBe(2);
    expect(pickHinge([near, far], camera, 200, 200, 100, 100)).toBe(2);
  });

  it("カメラの後ろにある折り線は拾わない", () => {
    const behind = [segment(3, [-0.5, 0, 5], [0.5, 0, 5])];
    expect(pickHinge(behind, camera, 200, 200, 100, 100)).toBeNull();
  });

  it("同深度の紙面は正面で大きい層と面ID、裏面で小さい層と面IDを選ぶ", () => {
    const surface = makeOverlappingSurface([90, 3, 4, 9], [1, 1, 2, 2]);
    const backCamera = makeBackCamera();

    try {
      expect(
        pickFace(
          surface.mesh,
          surface.triangleFaceIds,
          camera,
          200,
          200,
          100,
          100,
          surface.triangleLayers,
        ),
      ).toBe(9);
      expect(
        pickPaper(
          surface.mesh,
          surface.triangleFaceIds,
          camera,
          200,
          200,
          100,
          100,
          surface.triangleLayers,
        )?.face,
      ).toBe(9);
      expect(
        pickFace(
          surface.mesh,
          surface.triangleFaceIds,
          backCamera,
          200,
          200,
          100,
          100,
          surface.triangleLayers,
        ),
      ).toBe(3);
      expect(
        pickPaper(
          surface.mesh,
          surface.triangleFaceIds,
          backCamera,
          200,
          200,
          100,
          100,
          surface.triangleLayers,
        )?.face,
      ).toBe(3);
    } finally {
      disposeSurface(surface);
    }
  });

  it("可視面と所有面が違う隠れ線を拾わない", () => {
    const surface = makeOverlappingSurface([10, 20], [1, 2]);
    const hidden = segment(1, [-0.5, 0, 0], [0.5, 0, 0], 10, 1);
    const visible = segment(2, [-0.5, 0, 0], [0.5, 0, 0], 20, 2);

    try {
      expect(
        pickHinge(
          [hidden, visible],
          camera,
          200,
          200,
          100,
          100,
          10,
          surface,
        ),
      ).toBe(2);
    } finally {
      disposeSurface(surface);
    }
  });

  it("同じedgeの面別コピーから可視面の線分を返す", () => {
    const surface = makeOverlappingSurface([10, 20], [1, 2]);
    const hiddenCopy = segment(7, [-0.5, 0, 0], [0.5, 0, 0], 10, 1);
    const visibleCopy = segment(7, [-0.5, 0, 0], [0.5, 0, 0], 20, 2);

    try {
      expect(
        pickHingeSegment(
          [hiddenCopy, visibleCopy],
          camera,
          200,
          200,
          100,
          100,
          10,
          surface,
        ),
      ).toBe(visibleCopy);
    } finally {
      disposeSurface(surface);
    }
  });

  it("surfaceを渡さなければownerFace付きでも従来の手前優先を保つ", () => {
    const far = segment(1, [-0.5, 0, 0], [0.5, 0, 0], 20, 2);
    const near = segment(2, [-0.5, 0, 0.5], [0.5, 0, 0.5], 10, 1);

    expect(
      pickHingeSegment([far, near], camera, 200, 200, 100, 100),
    ).toBe(near);
  });

  it("surfaceに当たらなければownerFace付きでも従来の手前優先を保つ", () => {
    const surface = makeOverlappingSurface([20], [2]);
    surface.mesh.position.set(5, 0, 0);
    surface.mesh.updateMatrixWorld(true);
    const far = segment(1, [-0.5, 0, 0], [0.5, 0, 0], 20, 2);
    const near = segment(2, [-0.5, 0, 0.5], [0.5, 0, 0.5], 10, 1);

    try {
      expect(
        pickHingeSegment(
          [far, near],
          camera,
          200,
          200,
          100,
          100,
          10,
          surface,
        ),
      ).toBe(near);
    } finally {
      disposeSurface(surface);
    }
  });

  it("ownerFaceのない従来線はsurfaceがあっても従来どおり候補に残す", () => {
    const surface = makeOverlappingSurface([20], [2]);
    const owned = segment(1, [-0.5, 0, 0], [0.5, 0, 0], 20, 2);
    const legacy = segment(2, [-0.5, 0, 0.5], [0.5, 0, 0.5]);

    try {
      expect(
        pickHingeSegment(
          [owned, legacy],
          camera,
          200,
          200,
          100,
          100,
          10,
          surface,
        ),
      ).toBe(legacy);
    } finally {
      disposeSurface(surface);
    }
  });
});
