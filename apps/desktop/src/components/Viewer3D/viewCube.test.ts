import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  VIEW_CUBE_FACES,
  VIEW_CUBE_PLATES,
  VIEW_CUBE_TARGETS,
  VIEW_CUBE_ZONE_COLUMNS,
  VIEW_CUBE_ZONE_ROWS,
  attitudeAngleDeg,
  cameraAttitude,
  cameraPositionForTarget,
  cameraQuaternionLookingAt,
  cameraUpForTarget,
  cameraViewDirection,
  cubeCssMatrixElements,
  directionAngleDeg,
  horizontalOverflowPx,
  interpolateCameraPose,
  orbitCameraOffset,
  overlayRectsOverlap,
  smoothViewProgress,
  trackedOrbitTarget,
  transportCameraScreenUp,
  viewCubeOverlayRects,
  viewCubeZoneKind,
  viewCubeZoneTarget,
  viewDirectionForTarget,
  type ViewCubeFace,
  type ViewCubeTarget,
} from "./viewCube";

const TARGET = new THREE.Vector3(0.5, 0.5, 0);
const DISTANCE = 3;

/** 立方体の中で使う内部の座標表記。利用者向けの文言には出さない。 */
const INTERNAL_WORDS = /(?:[+-][XYZ]|[XYZ]軸|座標|camera|OrbitControls|solver|facet|hinge)/i;

function offsetFor(id: ViewCubeTarget): THREE.Vector3 {
  return cameraPositionForTarget(id, TARGET, DISTANCE).sub(TARGET);
}

/**
 * 移動の全過程を細かく刻み、通った回転量の合計・上下の反転回数を測る。
 * 合計が始点と終点の角度差に等しければ、遠回りしていない。
 */
function pathMeasurement(
  fromOffset: THREE.Vector3,
  fromScreenUp: THREE.Vector3,
  toOffset: THREE.Vector3,
  toScreenUp: THREE.Vector3,
  steps = 720,
) {
  const poses = [];
  for (let i = 0; i <= steps; i += 1) {
    poses.push(
      interpolateCameraPose(
        fromOffset,
        toOffset,
        fromScreenUp,
        toScreenUp,
        smoothViewProgress(i / steps),
      ),
    );
  }
  let arcDeg = 0;
  let upFlips = 0;
  let distanceError = 0;
  for (let i = 1; i < poses.length; i += 1) {
    arcDeg += attitudeAngleDeg(poses[i - 1].attitude, poses[i].attitude);
    if (poses[i - 1].screenUp.dot(poses[i].screenUp) < 0) upFlips += 1;
    distanceError = Math.max(
      distanceError,
      Math.abs(poses[i].offset.length() - DISTANCE),
    );
  }
  return {
    arcDeg,
    gapDeg: attitudeAngleDeg(poses[0].attitude, poses[poses.length - 1].attitude),
    upFlips,
    distanceError,
  };
}

describe("視点立方体の26箇所", () => {
  it("面6・辺12・角8の合計26箇所があり、呼び名が重ならない", () => {
    const byKind = {
      face: VIEW_CUBE_TARGETS.filter((t) => t.kind === "face"),
      edge: VIEW_CUBE_TARGETS.filter((t) => t.kind === "edge"),
      corner: VIEW_CUBE_TARGETS.filter((t) => t.kind === "corner"),
    };
    expect(byKind.face).toHaveLength(6);
    expect(byKind.edge).toHaveLength(12);
    expect(byKind.corner).toHaveLength(8);
    expect(VIEW_CUBE_TARGETS).toHaveLength(26);
    expect(new Set(VIEW_CUBE_TARGETS.map((t) => t.id)).size).toBe(26);
    expect(new Set(VIEW_CUBE_TARGETS.map((t) => t.actionLabel)).size).toBe(26);
  });

  it.each(VIEW_CUBE_TARGETS)(
    "$labelを選ぶと期待方向との差が0.5度未満になる",
    ({ id }) => {
      const position = cameraPositionForTarget(id, TARGET, DISTANCE);
      const actual = cameraViewDirection(position, TARGET);
      expect(directionAngleDeg(actual, viewDirectionForTarget(id))).toBeLessThan(0.5);
    },
  );

  it("辺は隣り合う2面の中間、角は隣り合う3面の中間を向く", () => {
    for (const target of VIEW_CUBE_TARGETS) {
      const direction = cameraPositionForTarget(target.id, TARGET, 1).sub(TARGET);
      const neighbours = VIEW_CUBE_FACES.filter((face) =>
        target.id.split("-").includes(face.id),
      );
      expect(neighbours).toHaveLength(
        target.kind === "face" ? 1 : target.kind === "edge" ? 2 : 3,
      );
      const middle = neighbours
        .reduce(
          (sum, face) =>
            sum.add(cameraPositionForTarget(face.id, TARGET, 1).sub(TARGET)),
          new THREE.Vector3(),
        )
        .normalize();
      expect(directionAngleDeg(direction, middle)).toBeLessThan(0.5);
    }
  });

  it("6面の利用者向け表示は日本語だけで、内部の座標表記を含まない", () => {
    const labels = VIEW_CUBE_FACES.map((face) => face.label);
    expect(labels).toEqual(["前", "後", "左", "右", "上", "下"]);
    expect(new Set(labels).size).toBe(6);
    expect(VIEW_CUBE_TARGETS.map((t) => `${t.label} ${t.actionLabel}`).join(" ")).not.toMatch(
      INTERNAL_WORDS,
    );
  });

  it.each(VIEW_CUBE_FACES)("$labelが立方体の手前を向く姿勢になる", ({ id }) => {
    const position = cameraPositionForTarget(id, TARGET, DISTANCE);
    const cameraQuaternion = cameraQuaternionLookingAt(
      position,
      TARGET,
      cameraUpForTarget(id),
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
    const shown = cssNormalByFace[id as ViewCubeFace].clone().transformDirection(matrix);
    expect(directionAngleDeg(shown, new THREE.Vector3(0, 0, 1))).toBeLessThan(0.5);
  });
});

describe("面を3×3へ割った押し場所", () => {
  it("6面×9区画が26箇所をすべて覆い、面9・辺24・角24の内訳になる", () => {
    const seen = new Map<ViewCubeTarget, number>();
    const kinds = { face: 0, edge: 0, corner: 0 };
    for (const plate of VIEW_CUBE_PLATES) {
      for (const row of VIEW_CUBE_ZONE_ROWS) {
        for (const column of VIEW_CUBE_ZONE_COLUMNS) {
          const id = viewCubeZoneTarget(plate, column, row);
          const kind = viewCubeZoneKind(column, row);
          kinds[kind] += 1;
          seen.set(id, (seen.get(id) ?? 0) + 1);
        }
      }
    }
    expect(kinds).toEqual({ face: 6, edge: 24, corner: 24 });
    expect(seen.size).toBe(26);
    for (const target of VIEW_CUBE_TARGETS) {
      // 面は1枚、辺は2枚、角は3枚の板から押せる。
      expect(seen.get(target.id)).toBe(
        target.kind === "face" ? 1 : target.kind === "edge" ? 2 : 3,
      );
    }
  });

  it.each(VIEW_CUBE_PLATES)(
    "$labelの面を正面から見たとき、区画の上下左右が画面の上下左右と揃う",
    (plate) => {
      const position = cameraPositionForTarget(plate.id, TARGET, DISTANCE);
      const quaternion = cameraQuaternionLookingAt(
        position,
        TARGET,
        cameraUpForTarget(plate.id),
      );
      const matrix = new THREE.Matrix4().fromArray(cubeCssMatrixElements(quaternion));
      // CSSは下向きが+yなので、世界の上向きは立方体の中では符号が逆になる。
      const onScreen = (v: readonly [number, number, number]) =>
        new THREE.Vector3(v[0], -v[1], v[2]).transformDirection(matrix);
      expect(
        directionAngleDeg(onScreen(plate.up), new THREE.Vector3(0, -1, 0)),
      ).toBeLessThan(0.5);
      expect(
        directionAngleDeg(onScreen(plate.right), new THREE.Vector3(1, 0, 0)),
      ).toBeLessThan(0.5);
    },
  );

  it("区画の位置と、そこが指す向きが食い違わない", () => {
    for (const plate of VIEW_CUBE_PLATES) {
      const normal = new THREE.Vector3(...plate.normal);
      const right = new THREE.Vector3(...plate.right);
      const up = new THREE.Vector3(...plate.up);
      // 板の3方向は互いに直角で、右×上が外向きになる(裏返っていない)。
      expect(right.dot(up)).toBeCloseTo(0, 10);
      expect(right.clone().cross(up).distanceTo(normal)).toBeLessThan(1e-10);
      for (const row of VIEW_CUBE_ZONE_ROWS) {
        for (const column of VIEW_CUBE_ZONE_COLUMNS) {
          const id = viewCubeZoneTarget(plate, column, row);
          const direction = cameraPositionForTarget(id, TARGET, 1).sub(TARGET);
          const alongRight = Math.round(direction.dot(right) * 1e6);
          const alongUp = Math.round(direction.dot(up) * 1e6);
          expect(direction.dot(normal)).toBeGreaterThan(0);
          expect(alongRight === 0).toBe(column === 0);
          expect(alongUp === 0).toBe(row === 0);
          if (column !== 0) expect(alongRight > 0).toBe(column > 0);
          if (row !== 0) expect(alongUp > 0).toBe(row > 0);
        }
      }
    }
  });
});

describe("視点立方体の移動が最短経路であること", () => {
  const startPoses = [
    { name: "前", offset: offsetFor("front"), up: cameraUpForTarget("front") },
    { name: "上", offset: offsetFor("top"), up: cameraUpForTarget("top") },
    { name: "後", offset: offsetFor("back"), up: cameraUpForTarget("back") },
    {
      name: "ななめ",
      offset: offsetFor("top-front-right"),
      up: cameraUpForTarget("top-front-right"),
    },
  ];

  it("代表的な10通り以上で、通った回転量が角度差と1度未満で一致する", () => {
    const cases: readonly (readonly [ViewCubeTarget, ViewCubeTarget])[] = [
      ["top", "bottom"],
      ["bottom", "top"],
      ["back", "top"],
      ["top", "back"],
      ["front", "back"],
      ["left", "right"],
      ["front", "top"],
      ["top", "left"],
      ["front", "top-front-right"],
      ["top-front-right", "bottom-back-left"],
      ["top-front", "bottom-back"],
      ["front-left", "back-right"],
      ["top-front-right", "front"],
      ["bottom", "top-back-left"],
    ];
    expect(cases.length).toBeGreaterThanOrEqual(10);
    for (const [from, to] of cases) {
      const measured = pathMeasurement(
        offsetFor(from),
        cameraUpForTarget(from),
        offsetFor(to),
        cameraUpForTarget(to),
      );
      expect(Math.abs(measured.arcDeg - measured.gapDeg)).toBeLessThan(1);
      expect(measured.upFlips).toBe(0);
      expect(measured.distanceError).toBeLessThan(1e-9);
    }
  });

  it("どの始点から26箇所のどこへ移っても、遠回りせず上下も反転しない", () => {
    let worstExtra = 0;
    let flips = 0;
    for (const start of startPoses) {
      for (const target of VIEW_CUBE_TARGETS) {
        const measured = pathMeasurement(
          start.offset,
          start.up,
          offsetFor(target.id),
          cameraUpForTarget(target.id),
          180,
        );
        worstExtra = Math.max(worstExtra, Math.abs(measured.arcDeg - measured.gapDeg));
        flips += measured.upFlips;
      }
    }
    expect(worstExtra).toBeLessThan(1);
    expect(flips).toBe(0);
  });

  it("移動の全時刻で注視点をまっすぐ向き、距離が変わらない", () => {
    for (const from of VIEW_CUBE_TARGETS) {
      for (const to of ["front", "top", "bottom", "top-front-right", "back-left"] as const) {
        for (const progress of [0, 0.25, 0.5, 0.75, 1]) {
          const pose = interpolateCameraPose(
            offsetFor(from.id),
            offsetFor(to),
            cameraUpForTarget(from.id),
            cameraUpForTarget(to),
            progress,
          );
          const position = TARGET.clone().add(pose.offset);
          const actualView = new THREE.Vector3(0, 0, -1).applyQuaternion(pose.attitude);
          expect(
            directionAngleDeg(actualView, cameraViewDirection(position, TARGET)),
          ).toBeLessThan(0.5);
          expect(pose.offset.length()).toBeCloseTo(DISTANCE, 10);
        }
      }
    }
  });

  it("真反対へ移る途中も注視点を通り抜けない", () => {
    const halfway = interpolateCameraPose(
      offsetFor("front"),
      offsetFor("back"),
      cameraUpForTarget("front"),
      cameraUpForTarget("back"),
      0.5,
    );
    expect(halfway.offset.length()).toBeCloseTo(DISTANCE, 10);
    expect(
      interpolateCameraPose(
        offsetFor("front"),
        offsetFor("back"),
        cameraUpForTarget("front"),
        cameraUpForTarget("back"),
        1,
      ).offset.distanceTo(offsetFor("back")),
    ).toBeLessThan(1e-9);
  });

  it("始点と終点では、指定した位置と上向きをそのまま返す", () => {
    const from = offsetFor("top-front-right");
    const to = offsetFor("bottom-back");
    const start = interpolateCameraPose(
      from,
      to,
      cameraUpForTarget("top-front-right"),
      cameraUpForTarget("bottom-back"),
      0,
    );
    const end = interpolateCameraPose(
      from,
      to,
      cameraUpForTarget("top-front-right"),
      cameraUpForTarget("bottom-back"),
      1,
    );
    expect(start.offset.distanceTo(from)).toBeLessThan(1e-9);
    expect(start.screenUp.distanceTo(cameraUpForTarget("top-front-right"))).toBeLessThan(1e-9);
    expect(end.offset.distanceTo(to)).toBeLessThan(1e-9);
    expect(end.screenUp.distanceTo(cameraUpForTarget("bottom-back"))).toBeLessThan(1e-9);
  });

  it("終端の上向きは、視点をドラッグで動かした後の上向きと同じ決め方になる", () => {
    // 真上・真下以外は世界の上向きを視線と直角へ落とすため、着いた後に姿勢が跳ねない。
    for (const target of VIEW_CUBE_TARGETS) {
      if (target.id === "top" || target.id === "bottom") continue;
      const offset = offsetFor(target.id);
      const settled = new THREE.Vector3(0, 1, 0).applyQuaternion(
        cameraAttitude(offset, new THREE.Vector3(0, 1, 0)),
      );
      expect(directionAngleDeg(settled, cameraUpForTarget(target.id))).toBeLessThan(0.5);
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

  it("移動の途中からドラッグしても画面の上下が反転しない", () => {
    const halfway = interpolateCameraPose(
      new THREE.Vector3(0, 3, 0),
      new THREE.Vector3(0, -3, 0),
      cameraUpForTarget("top"),
      cameraUpForTarget("bottom"),
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
      expectedCubeOnScreen: [860, 68, 988, 196],
    },
    {
      window: "1920×1080",
      viewer: { left: 995, top: 56, width: 925, height: 593.52 },
      expectedCubeOnScreen: [1780, 68, 1908, 196],
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
