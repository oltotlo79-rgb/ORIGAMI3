// 3D表示の視点ドラッグ回転の検査。
//
// 利用者の指摘(2026-08-16)「3D図で折り紙を回していたら回らなくなるところがある。
// 90度以上回らないなどある?」を数値で押さえる。
// 「視点を戻す」直後の向き(CAMERA_DIR)から10度刻みで36段回し、
// どの段でも同じだけ向きが変わり、途中で止まる段が1つも無いことを確かめる。
//
// 画面高さいっぱいのドラッグで1回転(2π)というのが回転量の決め方なので、
// 10度は画面高さの 10/360 にあたる。

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  CAMERA_DIR,
  applyCameraDragRotation,
  cameraScreenUp,
  viewRotationStarts,
} from "./sceneBuilder";
import {
  cameraPositionForTarget,
  cameraQuaternionLookingAt,
  cameraUpForTarget,
} from "./viewCube";

/** 検査に使う3D表示の高さ(px)。実機の高さでも比は変わらない。 */
const CANVAS_HEIGHT = 720;
/** 1段の回す量(度)。 */
const STEP_DEG = 10;
/** 何段回すか。36段でちょうど1周する。 */
const STEP_COUNT = 36;
/** 1段分のドラッグ量(px)。 */
const STEP_PX = (CANVAS_HEIGHT * STEP_DEG) / 360;
/** 合格とする角度の食い違い(度)。 */
const TOLERANCE_DEG = 0.5;
/** 紙の中心(注視点)。 */
const TARGET = new THREE.Vector3(0.5, 0.5, 0);
/** 注視点からカメラまでの距離。 */
const DISTANCE = 2;
/** 世界の真上。極を通り越せるかの判定に使う。 */
const WORLD_TOP = new THREE.Vector3(0, 1, 0);

function defaultCamera(): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
  camera.position.copy(TARGET).addScaledVector(CAMERA_DIR, DISTANCE);
  camera.up.set(0, 1, 0);
  camera.lookAt(TARGET);
  camera.updateMatrixWorld(true);
  return camera;
}

/** 視点立方体で「上」を選んだ直後の姿勢。真上ちょうどから見下ろす。 */
function viewCubeTopCamera(): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
  const position = cameraPositionForTarget("top", TARGET, DISTANCE);
  const up = cameraUpForTarget("top");
  camera.position.copy(position);
  camera.up.copy(up);
  camera.quaternion.copy(cameraQuaternionLookingAt(position, TARGET, up));
  camera.updateMatrixWorld(true);
  return camera;
}

function viewDirection(camera: THREE.PerspectiveCamera): THREE.Vector3 {
  return camera.getWorldDirection(new THREE.Vector3());
}

function angleDeg(a: THREE.Vector3, b: THREE.Vector3): number {
  return THREE.MathUtils.radToDeg(a.angleTo(b));
}

interface SweepStep {
  /** 1つ前の段からのカメラの向きの変化(度)。 */
  turnedDeg: number;
  /** その段でのカメラの向き。 */
  direction: THREE.Vector3;
  /** その段でのカメラ位置(世界の真上との角度を測るのに使う)。 */
  offsetDirection: THREE.Vector3;
}

function sweep(
  camera: THREE.PerspectiveCamera,
  dragX: number,
  dragY: number,
  count = STEP_COUNT,
): { start: THREE.Vector3; steps: SweepStep[] } {
  const start = viewDirection(camera);
  let previous = start.clone();
  const steps: SweepStep[] = [];
  for (let i = 0; i < count; i += 1) {
    applyCameraDragRotation(camera, TARGET, dragX, dragY, CANVAS_HEIGHT);
    const direction = viewDirection(camera);
    steps.push({
      turnedDeg: angleDeg(previous, direction),
      direction: direction.clone(),
      offsetDirection: camera.position.clone().sub(TARGET).normalize(),
    });
    previous = direction.clone();
  }
  return { start, steps };
}

/** 期待の1段分から離れすぎた段(=止まった段)を数える。 */
function stalledSteps(steps: readonly SweepStep[]): number[] {
  return steps
    .map((step, index) => ({ step, index }))
    .filter(({ step }) => Math.abs(step.turnedDeg - STEP_DEG) >= TOLERANCE_DEG)
    .map(({ index }) => index);
}

describe("視点のドラッグ回転(上下・左右とも止まらない)", () => {
  it("上下方向へ10度刻みで36段回すと、毎段10度ずつ変わり1周して戻る", () => {
    const camera = defaultCamera();
    const { start, steps } = sweep(camera, 0, STEP_PX);

    expect(steps).toHaveLength(STEP_COUNT);
    expect(stalledSteps(steps)).toEqual([]);
    for (const step of steps) {
      expect(Math.abs(step.turnedDeg - STEP_DEG)).toBeLessThan(TOLERANCE_DEG);
    }
    // 36段で1周し、始めの向きへ戻る。
    expect(angleDeg(steps[STEP_COUNT - 1].direction, start)).toBeLessThan(
      TOLERANCE_DEG,
    );
  });

  it("左右方向へ10度刻みで36段回すと、毎段10度ずつ変わり1周して戻る", () => {
    const camera = defaultCamera();
    const { start, steps } = sweep(camera, STEP_PX, 0);

    expect(steps).toHaveLength(STEP_COUNT);
    expect(stalledSteps(steps)).toEqual([]);
    for (const step of steps) {
      expect(Math.abs(step.turnedDeg - STEP_DEG)).toBeLessThan(TOLERANCE_DEG);
    }
    expect(angleDeg(steps[STEP_COUNT - 1].direction, start)).toBeLessThan(
      TOLERANCE_DEG,
    );
  });

  it("上下方向の1周は世界の真上と真下をどちらも通り越す", () => {
    const camera = defaultCamera();
    const { steps } = sweep(camera, 0, STEP_PX);
    const toTop = steps.map((step) => angleDeg(step.offsetDirection, WORLD_TOP));

    // 通り越した証拠: 真上のすぐそば(1度未満)と真下のすぐそば(179度超)を両方通る。
    expect(Math.min(...toTop)).toBeLessThan(1);
    expect(Math.max(...toTop)).toBeGreaterThan(179);
  });

  it("世界の真上を通り越す前後の3段が、どれも1段分のまま跳ねない", () => {
    const camera = defaultCamera();
    const { steps } = sweep(camera, 0, STEP_PX);
    const toTop = steps.map((step) => angleDeg(step.offsetDirection, WORLD_TOP));
    let nearest = 0;
    for (let i = 1; i < toTop.length; i += 1) {
      if (toTop[i] < toTop[nearest]) nearest = i;
    }

    expect(nearest).toBeGreaterThan(0);
    expect(nearest).toBeLessThan(steps.length - 1);
    for (const index of [nearest - 1, nearest, nearest + 1]) {
      expect(Math.abs(steps[index].turnedDeg - STEP_DEG)).toBeLessThan(
        TOLERANCE_DEG,
      );
    }
  });

  it("視点立方体で真上へ行った直後にドラッグしても跳ねない", () => {
    const camera = viewCubeTopCamera();
    // 立方体の「上」は真上ちょうど。ここが回転の行き止まりにならないことを見る。
    expect(angleDeg(camera.position.clone().sub(TARGET), WORLD_TOP)).toBeLessThan(
      1e-6,
    );

    const { steps } = sweep(camera, 0, STEP_PX, 3);
    expect(stalledSteps(steps)).toEqual([]);
    for (const step of steps) {
      expect(Math.abs(step.turnedDeg - STEP_DEG)).toBeLessThan(TOLERANCE_DEG);
    }
  });

  it("回しても注視点までの距離と画面の上向きの直角は保たれる", () => {
    const camera = defaultCamera();
    const { steps } = sweep(camera, STEP_PX * 0.7, STEP_PX * 0.7);

    expect(steps).toHaveLength(STEP_COUNT);
    expect(camera.position.distanceTo(TARGET)).toBeCloseTo(DISTANCE, 9);
    const up = cameraScreenUp(camera);
    expect(up.length()).toBeCloseTo(1, 9);
    expect(Math.abs(up.dot(viewDirection(camera)))).toBeLessThan(1e-9);
    // カメラのupも姿勢と同じ向きを指し続ける(寄る・平行移動で傾きが戻らない)。
    expect(angleDeg(camera.up, up)).toBeLessThan(1e-6);
  });

  it("注視点は回転で動かない", () => {
    const camera = defaultCamera();
    const before = TARGET.clone();
    sweep(camera, STEP_PX, STEP_PX, 5);
    expect(TARGET.distanceTo(before)).toBe(0);
  });
});

describe("視点回転を始めるボタン(ツールごとの割り当てを変えない)", () => {
  const selectTool = {
    LEFT: THREE.MOUSE.ROTATE,
    MIDDLE: THREE.MOUSE.DOLLY,
    RIGHT: THREE.MOUSE.PAN,
  } as const;
  const drawTool = {
    LEFT: null,
    MIDDLE: THREE.MOUSE.DOLLY,
    RIGHT: THREE.MOUSE.PAN,
  } as const;
  const pullTool = {
    LEFT: null,
    MIDDLE: THREE.MOUSE.PAN,
    RIGHT: THREE.MOUSE.ROTATE,
  } as const;

  it("選択ツールでは左ドラッグで回り、右・中ボタンでは回らない", () => {
    expect(viewRotationStarts(selectTool, 0, false)).toBe(true);
    expect(viewRotationStarts(selectTool, 1, false)).toBe(false);
    expect(viewRotationStarts(selectTool, 2, false)).toBe(false);
  });

  it("折る・技法では左ドラッグで回らない", () => {
    expect(viewRotationStarts(drawTool, 0, false)).toBe(false);
    expect(viewRotationStarts(drawTool, 2, false)).toBe(false);
  });

  it("引くツールでは右ドラッグで回り、左では回らない", () => {
    expect(viewRotationStarts(pullTool, 0, false)).toBe(false);
    expect(viewRotationStarts(pullTool, 2, false)).toBe(true);
  });

  it("修飾キーを押すと回転と平行移動が入れ替わる", () => {
    // 回転ボタン+修飾キーは平行移動になるので回らない。
    expect(viewRotationStarts(selectTool, 0, true)).toBe(false);
    // 平行移動ボタン+修飾キーは回転になる。
    expect(viewRotationStarts(selectTool, 2, true)).toBe(true);
  });

  it("知らないボタンでは回らない", () => {
    expect(viewRotationStarts(selectTool, 3, false)).toBe(false);
    expect(viewRotationStarts(selectTool, 4, true)).toBe(false);
  });
});
