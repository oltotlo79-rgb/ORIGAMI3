import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  VIEW_CUBE_FACES,
  cameraPositionForFace,
  cameraQuaternionLookingAt,
  cameraUpForFace,
  cameraViewDirection,
  cubeCssMatrixElements,
  directionAngleDeg,
  horizontalOverflowPx,
  interpolateCameraOffset,
  interpolateCameraPose,
  orbitCameraOffset,
  overlayRectsOverlap,
  trackedOrbitTarget,
  transportCameraScreenUp,
  viewCubeOverlayRects,
  viewDirectionForFace,
  type ViewCubeFace,
} from "./viewCube";

const TARGET = new THREE.Vector3(0.5, 0.5, 0);

describe("視点立方体の6面", () => {
  it.each(VIEW_CUBE_FACES)(
    "$labelの面を選ぶと期待方向との差が0.5度未満になる",
    ({ id }) => {
      const position = cameraPositionForFace(id, TARGET, 3);
      const actual = cameraViewDirection(position, TARGET);
      expect(directionAngleDeg(actual, viewDirectionForFace(id))).toBeLessThan(0.5);
    },
  );

  it("6面の利用者向け表示は日本語だけで、内部の座標表記を含まない", () => {
    const labels = VIEW_CUBE_FACES.map((face) => face.label);
    expect(labels).toEqual(["前", "後", "左", "右", "上", "下"]);
    expect(new Set(labels).size).toBe(6);
    expect(labels.join("")).not.toMatch(/(?:[+-][XYZ]|[XYZ]軸|座標)/i);
  });

  it.each(VIEW_CUBE_FACES)("$labelが立方体の手前を向く姿勢になる", ({ id }) => {
    const position = cameraPositionForFace(id, TARGET, 3);
    const cameraQuaternion = cameraQuaternionLookingAt(
      position,
      TARGET,
      cameraUpForFace(id),
    );
    const matrix = new THREE.Matrix4().fromArray(cubeCssMatrixElements(cameraQuaternion));
    const cssNormalByFace: Record<ViewCubeFace, THREE.Vector3> = {
      front: new THREE.Vector3(0, 0, 1),
      back: new THREE.Vector3(0, 0, -1),
      left: new THREE.Vector3(-1, 0, 0),
      right: new THREE.Vector3(1, 0, 0),
      // CSSは下向きが+yなので、紙の上(+y)は立方体内では-y。
      top: new THREE.Vector3(0, -1, 0),
      bottom: new THREE.Vector3(0, 1, 0),
    };
    const shown = cssNormalByFace[id].clone().transformDirection(matrix);
    expect(directionAngleDeg(shown, new THREE.Vector3(0, 0, 1))).toBeLessThan(0.5);
  });
});

describe("視点立方体の滑らかな移動", () => {
  it("真反対の面へ移る途中も注視点を通り抜けず、距離を保つ", () => {
    const from = new THREE.Vector3(0, 0, 2);
    const to = new THREE.Vector3(0, 0, -2);
    const halfway = interpolateCameraOffset(from, to, 0.5);
    expect(halfway.length()).toBeCloseTo(2, 10);
    expect(halfway.distanceTo(new THREE.Vector3())).toBeGreaterThan(1.99);
    expect(interpolateCameraOffset(from, to, 0)).toEqual(from);
    expect(interpolateCameraOffset(from, to, 1).distanceTo(to)).toBeLessThan(1e-9);
  });

  it("6×6方向の移動中も、全時刻で注視点を正確に向く", () => {
    for (const fromFace of VIEW_CUBE_FACES) {
      for (const toFace of VIEW_CUBE_FACES) {
        const fromPosition = cameraPositionForFace(fromFace.id, TARGET, 3);
        const toPosition = cameraPositionForFace(toFace.id, TARGET, 3);
        const fromOffset = fromPosition.clone().sub(TARGET);
        const toOffset = toPosition.clone().sub(TARGET);
        for (const progress of [0, 0.25, 0.5, 0.75, 1]) {
          const pose = interpolateCameraPose(
            fromOffset,
            toOffset,
            cameraUpForFace(fromFace.id),
            cameraUpForFace(toFace.id),
            progress,
          );
          const position = TARGET.clone().add(pose.offset);
          const quaternion = cameraQuaternionLookingAt(position, TARGET, pose.screenUp);
          const actualView = new THREE.Vector3(0, 0, -1).applyQuaternion(quaternion);
          expect(
            directionAngleDeg(actualView, cameraViewDirection(position, TARGET)),
          ).toBeLessThan(0.5);
          expect(pose.offset.length()).toBeCloseTo(3, 10);
        }
      }
    }
  });
});

describe("視点立方体のドラッグ", () => {
  it("横方向のドラッグで距離を保ったまま視点が回る", () => {
    const start = new THREE.Vector3(0, 0, 2);
    const moved = orbitCameraOffset(start, 100, 0, 400);
    expect(moved.length()).toBeCloseTo(start.length(), 10);
    expect(directionAngleDeg(start, moved)).toBeCloseTo(90, 8);
    expect(moved.x).toBeLessThan(-1.99);
  });

  it("縦方向のドラッグで距離を保ったまま視点が回る", () => {
    const start = new THREE.Vector3(0, 0, 2);
    const moved = orbitCameraOffset(start, 0, 100, 400);
    expect(moved.length()).toBeCloseTo(start.length(), 10);
    expect(directionAngleDeg(start, moved)).toBeGreaterThan(89.99);
    expect(moved.y).toBeGreaterThan(1.99);
  });

  it("面移動の途中からドラッグしても画面の上下が反転しない", () => {
    const topOffset = new THREE.Vector3(0, 3, 0);
    const bottomOffset = new THREE.Vector3(0, -3, 0);
    const halfway = interpolateCameraPose(
      topOffset,
      bottomOffset,
      cameraUpForFace("top"),
      cameraUpForFace("bottom"),
      0.5,
    );
    const draggedOffset = orbitCameraOffset(halfway.offset, 5, 0, 400);
    const draggedUp = transportCameraScreenUp(
      halfway.offset,
      draggedOffset,
      halfway.screenUp,
    );
    expect(directionAngleDeg(halfway.screenUp, draggedUp)).toBeLessThan(5);
    const quaternion = cameraQuaternionLookingAt(
      TARGET.clone().add(draggedOffset),
      TARGET,
      draggedUp,
    );
    const actualScreenUp = new THREE.Vector3(0, 1, 0).applyQuaternion(quaternion);
    expect(directionAngleDeg(actualScreenUp, draggedUp)).toBeLessThan(0.5);
  });

  it("回転・拡大縮小・画面内移動後の注視点を公開cameraだけで追跡できる", () => {
    const original = new THREE.Vector3(0, 0, 0);
    expect(
      trackedOrbitTarget(
        original,
        new THREE.Vector3(5, 0, 0),
        new THREE.Vector3(-1, 0, 0),
      ).distanceTo(original),
    ).toBeLessThan(1e-10);

    const pan = new THREE.Vector3(1.25, -0.75, 0);
    const afterPan = trackedOrbitTarget(
      original,
      new THREE.Vector3(0, 0, 5).add(pan),
      new THREE.Vector3(0, 0, -1),
    );
    expect(afterPan.distanceTo(pan)).toBeLessThan(1e-10);
  });
});

describe("視点立方体の配置", () => {
  it.each([
    {
      window: "1000×700",
      viewer: { left: 535, top: 56, width: 465, height: 335.12 },
      expectedCubeOnScreen: [892, 68, 988, 164],
    },
    {
      window: "1920×1080",
      viewer: { left: 995, top: 56, width: 925, height: 593.52 },
      expectedCubeOnScreen: [1812, 68, 1908, 164],
    },
  ])("$windowで既存表示と重ならず、横にはみ出さない", ({ viewer, expectedCubeOnScreen }) => {
    const rects = viewCubeOverlayRects(viewer.width, viewer.height);
    const screenCube = [
      viewer.left + rects.cube.left,
      viewer.top + rects.cube.top,
      viewer.left + rects.cube.right,
      viewer.top + rects.cube.bottom,
    ];
    expect(screenCube).toEqual(expectedCubeOnScreen);
    expect(overlayRectsOverlap(rects.cube, rects.modeHint)).toBe(false);
    expect(overlayRectsOverlap(rects.cube, rects.resetButton)).toBe(false);
    expect(horizontalOverflowPx(rects.cube, viewer.width)).toBe(0);
    expect(rects.cube.right - viewer.width).toBe(-12);
  });
});
