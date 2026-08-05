// 3Dビューのヒンジ拾い上げ(純関数)のテスト。
// クリック位置のしきい値、重なったときの手前優先を確かめる。
// (線分そのものの作り方はsceneBuilder.test.tsのbuildTopologyで確認する)

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { pickHinge, type HingeSegment } from "./hingePicker";

/** 原点を正面から見るカメラ(画面200×200px) */
function makeCamera(): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
  camera.position.set(0, 0, 2);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  return camera;
}

function segment(edgeId: number, a: number[], b: number[]): HingeSegment {
  return {
    edgeId,
    a: new THREE.Vector3(a[0], a[1], a[2]),
    b: new THREE.Vector3(b[0], b[1], b[2]),
  };
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
});
