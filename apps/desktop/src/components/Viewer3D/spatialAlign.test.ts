// 立体上で選んだ点・線を、一意な共通支持面のchartで既存solveAlignへ渡す境界。
// global XY・Face ID順へ退避せず、共面でない入力はunavailableにする。

import { describe, expect, it } from "vitest";
import {
  distanceToLine,
  solveAlign,
  type FoldLine,
} from "../../lib/alignFold";
import type { AlignMode, AlignTarget, Vec2 } from "../../lib/types";
import type { Vec3 } from "../../lib/layerOffset";
import {
  mapPoint,
  unmapPoint,
  type FacePlacement,
} from "./edgeHighlight";
import { materialFoldLineFrom3D } from "./cpPick3d";
import {
  SPATIAL_REPROJECTION_EPS,
  solveSpatialAlignOnCommonPlane,
} from "./spatialAlign";

const VERTICAL_PLACEMENT: FacePlacement = {
  faceId: 17,
  polygon: [
    [-4, -4],
    [4, -4],
    [4, 4],
    [-4, 4],
  ],
  p0: [-4, -4],
  e1: [8, 0],
  e2: [0, 8],
  q0: [2, -4, -4],
  f1: [0, 8, 0],
  f2: [0, 0, 8],
  det: 64,
};

const VERTICAL_PLANE = {
  point: [2, 0, 0] as Vec3,
  normal: [1, 0, 0] as Vec3,
};

function world(placement: FacePlacement, material: Vec2): Vec3 {
  const mapped = mapPoint(placement, material);
  if (mapped === null) throw new Error("test placement did not map material point");
  return mapped;
}

function spatialTarget(target: AlignTarget) {
  return target.kind === "point"
    ? {
        kind: "point" as const,
        world: world(VERTICAL_PLACEMENT, target.p),
        supportPlanes: [VERTICAL_PLANE],
        foldedPoint: null,
      }
    : {
        kind: "line" as const,
        aWorld: world(VERTICAL_PLACEMENT, target.a),
        bWorld: world(VERTICAL_PLACEMENT, target.b),
        supportPlanes: [VERTICAL_PLANE],
        foldedLine: null,
      };
}

interface ModeCase {
  name: string;
  mode: AlignMode;
  picks: AlignTarget[];
  cursor: Vec2 | null;
}

const MODE_CASES: ModeCase[] = [
  {
    name: "2点を通る",
    mode: "throughTwoPoints",
    picks: [
      { kind: "point", p: [0.1, 0.2] },
      { kind: "point", p: [0.8, 0.6] },
    ],
    cursor: null,
  },
  {
    name: "点と点を合わせる",
    mode: "pointPoint",
    picks: [
      { kind: "point", p: [0, 0] },
      { kind: "point", p: [1, 1] },
    ],
    cursor: null,
  },
  {
    name: "線と線を合わせる",
    mode: "lineLine",
    picks: [
      { kind: "line", a: [0, 0], b: [1, 0] },
      { kind: "line", a: [0, 0], b: [0, 1] },
    ],
    cursor: [1, 1],
  },
  {
    name: "点を通り線と垂直",
    mode: "pointPerpendicularLine",
    picks: [
      { kind: "point", p: [0.25, 0.75] },
      { kind: "line", a: [0, 0], b: [1, 0] },
    ],
    cursor: null,
  },
  {
    name: "点を線へ合わせて別の点を通る",
    mode: "pointLineThrough",
    picks: [
      { kind: "point", p: [0, 1] },
      { kind: "line", a: [0, 0], b: [1, 0] },
      { kind: "point", p: [0, 0.5] },
    ],
    cursor: null,
  },
  {
    name: "2組を同時に合わせる",
    mode: "pointToLinePointToLine",
    picks: [
      { kind: "point", p: [0, 0] },
      { kind: "line", a: [1, -1], b: [1, 2] },
      { kind: "point", p: [-1, 1] },
      { kind: "line", a: [1, 0], b: [2, 1] },
    ],
    cursor: null,
  },
  {
    name: "点を線へ合わせて別の線と垂直",
    mode: "pointLinePerpendicular",
    picks: [
      { kind: "point", p: [0, 2] },
      { kind: "line", a: [-1, 0], b: [1, 0] },
      { kind: "line", a: [0, -1], b: [0, 1] },
    ],
    cursor: null,
  },
  {
    name: "既存の線",
    mode: "existingLine",
    picks: [{ kind: "line", a: [0, 0.4], b: [1, 0.4] }],
    cursor: null,
  },
];

function expectPositiveWorldLine(line: readonly [Vec3, Vec3]): void {
  expect(
    Math.hypot(
      line[1][0] - line[0][0],
      line[1][1] - line[0][1],
      line[1][2] - line[0][2],
    ),
  ).toBeGreaterThan(1e-9);
}

function expectSameInfiniteLine(actual: readonly [Vec3, Vec3], expected: FoldLine): void {
  const a = unmapPoint(VERTICAL_PLACEMENT, actual[0]);
  const b = unmapPoint(VERTICAL_PLACEMENT, actual[1]);
  expect(a).not.toBeNull();
  expect(b).not.toBeNull();
  expect(distanceToLine(expected, a ?? expected[0])).toBeLessThanOrEqual(1e-9);
  expect(distanceToLine(expected, b ?? expected[1])).toBeLessThanOrEqual(1e-9);
}

describe("一意な共通3D平面での合わせ折り", () => {
  it.each(MODE_CASES)("$nameを同じchartのsolveAlignと一致させる", ({
    mode,
    picks,
    cursor,
  }) => {
    const expected = solveAlign(mode, picks, cursor);
    expect(expected.reason).toBeNull();
    expect(expected.lines.length).toBeGreaterThan(0);

    const actual = solveSpatialAlignOnCommonPlane({
      mode,
      picks: picks.map(spatialTarget),
      cursorWorld: cursor === null ? null : world(VERTICAL_PLACEMENT, cursor),
      placements: [VERTICAL_PLACEMENT],
    });

    expect(actual.status).toBe("ready");
    expect(actual.reason).toBeNull();
    expect(actual.solutions).toHaveLength(expected.lines.length);
    for (const [index, solution] of actual.solutions.entries()) {
      expect(solution, `solution ${index}`).not.toBeNull();
      if (solution === null) continue;
      expectPositiveWorldLine(solution.lineWorld);
      expectSameInfiniteLine(solution.lineWorld, expected.lines[index]);
      expect(solution.foldedPlane).toBeNull();
    }
  });

  it("表示層offsetがあってもraw z=0証拠があるときだけFoldThrough companionと初期sideを返す", () => {
    const displayLift = 0.0002;
    const placement: FacePlacement = {
      faceId: 201,
      polygon: [[0, 0], [1, 0], [1, 1], [0, 1]],
      p0: [0, 0],
      e1: [1, 0],
      e2: [0, 1],
      q0: [0, 0, displayLift],
      f1: [1, 0, 0],
      f2: [0, 1, 0],
      det: 1,
    };
    const plane = {
      point: [0, 0, displayLift] as Vec3,
      normal: [0, 0, 1] as Vec3,
    };
    const result = solveSpatialAlignOnCommonPlane({
      mode: "lineLine",
      picks: [
        {
          kind: "line",
          aWorld: [0, 1, displayLift],
          bWorld: [1, 1, displayLift],
          supportPlanes: [plane],
          foldedLine: [[0, 1], [1, 1]],
        },
        {
          kind: "line",
          aWorld: [0, 0, displayLift],
          bWorld: [1, 0, displayLift],
          supportPlanes: [plane],
          foldedLine: [[0, 0], [1, 0]],
        },
      ],
      cursorWorld: null,
      placements: [placement],
    });

    expect(result.status).toBe("ready");
    const target = result.solutions[0];
    expect(target).not.toBeNull();
    if (target === null) return;
    expect(target.foldedPlane?.line).toEqual(target.lineWorld.map(([x, y]) => [x, y]));
    expect(target.foldedPlane?.keepPointForMovingSide).toEqual({
      left: target.keepWorldForMovingSide.left?.slice(0, 2) ?? null,
      right: target.keepWorldForMovingSide.right?.slice(0, 2) ?? null,
    });
    expect(target.sideForFirstPick).toEqual({ automatic: "left", initial: "left" });

    const shifted = solveSpatialAlignOnCommonPlane({
      mode: "existingLine",
      picks: [
        {
          kind: "line",
          aWorld: [0, 0.5, 0.01],
          bWorld: [1, 0.5, 0.01],
          supportPlanes: [{ point: [0, 0, 0.01], normal: [0, 0, 1] }],
          foldedLine: null,
        },
      ],
      cursorWorld: null,
      placements: [{ ...placement, q0: [0, 0, 0.01] }],
    });
    expect(shifted.status).toBe("ready");
    expect(shifted.solutions[0]?.foldedPlane).toBeNull();
  });

  it("clip後の正長線分と厳密内部点をmaterialFoldLineFrom3Dへ渡せる", () => {
    const result = solveSpatialAlignOnCommonPlane({
      mode: "throughTwoPoints",
      picks: [
        spatialTarget({ kind: "point", p: [-1, -0.5] }),
        spatialTarget({ kind: "point", p: [1, 0.5] }),
      ],
      cursorWorld: null,
      placements: [VERTICAL_PLACEMENT],
    });
    const solution = result.solutions[0];
    expect(solution).not.toBeNull();
    if (solution === null) return;
    expectPositiveWorldLine(solution.lineWorld);

    const mapped = Object.values(solution.keepWorldForMovingSide)
      .filter((point): point is Vec3 => point !== null)
      .map((keep) =>
        materialFoldLineFrom3D(
          [VERTICAL_PLACEMENT],
          solution.lineWorld,
          keep,
        ),
      );
    expect(mapped.length).toBeGreaterThan(0);
    expect(mapped.every((one) => one !== null)).toBe(true);
  });

  it.each([
    ["傾斜面", {
      ...VERTICAL_PLACEMENT,
      q0: [0.1, -0.2, 0.4] as Vec3,
      f1: [1, 0, 0.6] as Vec3,
      f2: [0, 1, 0.25] as Vec3,
    }],
    ["垂直面", VERTICAL_PLACEMENT],
  ] as const)("%sでworld解と左右の材料companionを同じraw indexへ返す", (_name, placement) => {
    const point = world(placement, [0.75, 0.25]);
    const lineA = world(placement, [0.625, 0.125]);
    const lineB = world(placement, [0.625, 0.875]);
    const normal = (() => {
      const a = placement.f1;
      const b = placement.f2;
      const cross: Vec3 = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
      ];
      const length = Math.hypot(...cross);
      return cross.map((value) => value / length) as Vec3;
    })();
    const result = solveSpatialAlignOnCommonPlane({
      mode: "pointPerpendicularLine",
      picks: [
        {
          kind: "point",
          world: point,
          supportPlanes: [{ point, normal }],
          foldedPoint: null,
        },
        {
          kind: "line",
          aWorld: lineA,
          bWorld: lineB,
          supportPlanes: [{ point: lineA, normal }],
          foldedLine: null,
        },
      ],
      cursorWorld: lineA,
      placements: [placement],
    });

    expect(result.status).toBe("ready");
    expect(result.materialSolutions).toHaveLength(result.solutions.length);
    result.solutions.forEach((solution, index) => {
      const material = result.materialSolutions[index];
      expect(solution).not.toBeNull();
      expect(material).not.toBeNull();
      if (solution === null || material === null) return;
      expect(solution.foldedPlane).toBeNull();
      for (const side of ["left", "right"] as const) {
        const keep = solution.keepWorldForMovingSide[side];
        const expected =
          keep === null
            ? null
            : materialFoldLineFrom3D([placement], solution.lineWorld, keep);
        expect(material[side]).toEqual(
          expected === null
            ? null
            : {
                materialLine: expected.material_line,
                materialKeepSidePoint: expected.material_keep_side_point,
              },
        );
      }
    });
  });

  it("非共面ならglobal XYへ落とさずunavailableにする", () => {
    const result = solveSpatialAlignOnCommonPlane({
      mode: "throughTwoPoints",
      picks: [
        {
          kind: "point",
          world: [0, 0, 0],
          supportPlanes: [{ point: [0, 0, 0], normal: [0, 0, 1] }],
          foldedPoint: null,
        },
        {
          kind: "point",
          world: [1, 0, 0.25],
          supportPlanes: [{ point: [0, 0, 0.25], normal: [0, 0, 1] }],
          foldedPoint: null,
        },
      ],
      cursorWorld: null,
      placements: [],
    });

    expect(result.status).toBe("unavailable");
    expect(result.solutions).toEqual([]);
    expect(result.reason).not.toBeNull();
  });

  it("支持平面が2つ共通ならFace ID順で選ばずunavailableにする", () => {
    const supportPlanes = [
      { point: [0, 0, 0] as Vec3, normal: [0, 0, 1] as Vec3 },
      { point: [0, 0, 0] as Vec3, normal: [0, 1, 0] as Vec3 },
    ];
    const result = solveSpatialAlignOnCommonPlane({
      mode: "throughTwoPoints",
      picks: [
        { kind: "point", world: [0, 0, 0], supportPlanes, foldedPoint: null },
        { kind: "point", world: [1, 0, 0], supportPlanes, foldedPoint: null },
      ],
      cursorWorld: null,
      placements: [],
    });

    expect(result.status).toBe("unavailable");
    expect(result.solutions).toEqual([]);
    expect(result.reason).not.toBeNull();
  });

  it("許容差内の支持面が連鎖しても配列順で1面へ畳まずunavailableにする", () => {
    const middleZ = SPATIAL_REPROJECTION_EPS * 0.75;
    const planes = [
      { point: [0, 0, 0] as Vec3, normal: [0, 0, 1] as Vec3 },
      { point: [0, 0, middleZ] as Vec3, normal: [0, 0, 1] as Vec3 },
      {
        point: [0, 0, SPATIAL_REPROJECTION_EPS * 1.5] as Vec3,
        normal: [0, 0, 1] as Vec3,
      },
    ];
    const placement: FacePlacement = {
      faceId: 99,
      polygon: [
        [0, 0],
        [1, 0],
        [1, 1],
        [0, 1],
      ],
      p0: [0, 0],
      e1: [1, 0],
      e2: [0, 1],
      q0: [0, 0, middleZ],
      f1: [1, 0, 0],
      f2: [0, 1, 0],
      det: 1,
    };
    const solve = (supportPlanes: typeof planes) =>
      solveSpatialAlignOnCommonPlane({
        mode: "existingLine",
        picks: [
          {
            kind: "line",
            aWorld: [0.2, 0.5, middleZ],
            bWorld: [0.8, 0.5, middleZ],
            supportPlanes,
            foldedLine: null,
          },
        ],
        cursorWorld: null,
        placements: [placement],
      });

    const endpointFirst = solve(planes);
    const bridgeFirst = solve([planes[1], planes[0], planes[2]]);
    expect(endpointFirst.status).toBe("unavailable");
    expect(bridgeFirst.status).toBe("unavailable");
    expect(bridgeFirst).toEqual(endpointFirst);
  });

  it("target固有面の連鎖に埋もれた一意な共通面は拒否しない", () => {
    const eps = SPATIAL_REPROJECTION_EPS;
    const commonZ = eps * 1.35;
    const ownPlaneZ = [0, eps * 0.9, eps * 1.8, eps * 2.7];
    const targetZ = [eps * 0.675, eps * 1.125, eps * 1.575, eps * 2.025];
    const placement: FacePlacement = {
      faceId: 100,
      polygon: [
        [-4, -4],
        [4, -4],
        [4, 4],
        [-4, 4],
      ],
      p0: [-4, -4],
      e1: [8, 0],
      e2: [0, 8],
      q0: [-4, -4, commonZ],
      f1: [8, 0, 0],
      f2: [0, 8, 0],
      det: 64,
    };
    const chartPicks = MODE_CASES[5].picks;
    const picks = chartPicks.map((target, index) => {
      const supportPlanes = [
        { point: [0, 0, commonZ] as Vec3, normal: [0, 0, 1] as Vec3 },
        {
          point: [0, 0, ownPlaneZ[index]] as Vec3,
          normal: [0, 0, 1] as Vec3,
        },
      ];
      return target.kind === "point"
        ? {
            kind: "point" as const,
            world: [target.p[0], target.p[1], targetZ[index]] as Vec3,
            supportPlanes,
            foldedPoint: null,
          }
        : {
            kind: "line" as const,
            aWorld: [target.a[0], target.a[1], targetZ[index]] as Vec3,
            bWorld: [target.b[0], target.b[1], targetZ[index]] as Vec3,
            supportPlanes,
            foldedLine: null,
          };
    });

    const result = solveSpatialAlignOnCommonPlane({
      mode: "pointToLinePointToLine",
      picks,
      cursorWorld: null,
      placements: [placement],
    });
    expect(result.status).toBe("ready");
    expect(result.solutions.some((solution) => solution !== null)).toBe(true);
    expect(result.maxReprojectionResidual).toBeLessThanOrEqual(eps);
  });

  it("採用しない支持面の大残差を共通面の成功判定へ混ぜない", () => {
    const placement: FacePlacement = {
      faceId: 101,
      polygon: [
        [0, 0],
        [1, 0],
        [1, 1],
        [0, 1],
      ],
      p0: [0, 0],
      e1: [1, 0],
      e2: [0, 1],
      q0: [0, 0, 0],
      f1: [1, 0, 0],
      f2: [0, 1, 0],
      det: 1,
    };
    const result = solveSpatialAlignOnCommonPlane({
      mode: "existingLine",
      picks: [
        {
          kind: "line",
          aWorld: [0.2, 0.5, 0],
          bWorld: [0.8, 0.5, 0],
          supportPlanes: [
            { point: [0, 0, 0], normal: [0, 0, 1] },
            { point: [0, 0, 1], normal: [0, 0, 1] },
          ],
          foldedLine: null,
        },
      ],
      cursorWorld: null,
      placements: [placement],
    });
    expect(result.status).toBe("ready");
    expect(result.maxReprojectionResidual).toBeLessThanOrEqual(
      SPATIAL_REPROJECTION_EPS,
    );
  });

  it("同じ入力を100回解いて完全一致する", () => {
    const input = {
      mode: "lineLine" as const,
      picks: MODE_CASES[2].picks.map(spatialTarget),
      cursorWorld: world(VERTICAL_PLACEMENT, [1, 1]),
      placements: [VERTICAL_PLACEMENT],
    };
    const expected = solveSpatialAlignOnCommonPlane(input);
    expect(expected.status).toBe("ready");

    for (let i = 0; i < 100; i++) {
      expect(solveSpatialAlignOnCommonPlane(input)).toEqual(expected);
    }
  });
});

const FLOAT32_Q0: Vec3 = [
  0.8965008854866028,
  0.4158884286880493,
  0.9286432266235352,
];
const FLOAT32_Q1: Vec3 = [
  1.2746968269348145,
  -0.06158852577209473,
  1.7217280864715576,
];
const FLOAT32_Q2: Vec3 = [
  1.531247615814209,
  0.8156320452690125,
  2.12751841545105,
];
const FLOAT32_Q3: Vec3 = [
  1.1530518531799316,
  1.2931089401245117,
  1.3344334363937378,
];
const FLOAT32_NORMAL: Vec3 = [
  -0.8894658681871654,
  0.04999829005051116,
  0.45425834094937334,
];
const FLOAT32_PLANE = { point: FLOAT32_Q0, normal: FLOAT32_NORMAL };
const FLOAT32_PLACEMENT: FacePlacement = {
  faceId: 71,
  polygon: [
    [0, 0],
    [1, 0],
    [1, 1],
    [0, 1],
  ],
  p0: [0, 0],
  e1: [1, 0],
  e2: [1, 1],
  q0: FLOAT32_Q0,
  f1: FLOAT32_Q1.map((value, index) => value - FLOAT32_Q0[index]) as Vec3,
  f2: FLOAT32_Q2.map((value, index) => value - FLOAT32_Q0[index]) as Vec3,
  det: 1,
};

function reprojectionResidual(point: Vec3): number {
  const material = unmapPoint(FLOAT32_PLACEMENT, point);
  const remapped = material && mapPoint(FLOAT32_PLACEMENT, material);
  if (remapped === null) return Number.POSITIVE_INFINITY;
  return Math.hypot(
    remapped[0] - point[0],
    remapped[1] - point[1],
    remapped[2] - point[2],
  );
}

describe("共通面への再写像残差", () => {
  it("Float32表示面の実測残差までは許し、境目を超えた点は拒否する", () => {
    // fixed seedで20万姿勢のf64単位正方形をFloat32表示頂点へした実測最大は
    // 2.161808357136193e-7。境目2.7e-7の80.07%で、余裕0にはしない。
    // 最小表示層間隔2e-4は境目の約741倍なので、別の支持面とは区別できる。
    expect(reprojectionResidual(FLOAT32_Q3)).toBeCloseTo(
      2.161808357136193e-7,
      15,
    );
    expect(SPATIAL_REPROJECTION_EPS).toBe(2.7e-7);
    expect(reprojectionResidual(FLOAT32_Q3)).toBeLessThanOrEqual(
      SPATIAL_REPROJECTION_EPS,
    );

    const base = world(FLOAT32_PLACEMENT, [0.25, 0.75]);
    const otherPoint = world(FLOAT32_PLACEMENT, [0.75, 0.25]);
    const acceptedPoint: Vec3 = [
      base[0] + FLOAT32_NORMAL[0] * 2.161808357136193e-7,
      base[1] + FLOAT32_NORMAL[1] * 2.161808357136193e-7,
      base[2] + FLOAT32_NORMAL[2] * 2.161808357136193e-7,
    ];
    expect(reprojectionResidual(acceptedPoint)).toBeCloseTo(
      2.1618083559610095e-7,
      15,
    );
    const accepted = solveSpatialAlignOnCommonPlane({
      mode: "pointPoint",
      picks: [
        {
          kind: "point",
          world: acceptedPoint,
          supportPlanes: [FLOAT32_PLANE],
          foldedPoint: null,
        },
        {
          kind: "point",
          world: otherPoint,
          supportPlanes: [FLOAT32_PLANE],
          foldedPoint: null,
        },
      ],
      cursorWorld: null,
      placements: [FLOAT32_PLACEMENT],
    });
    expect(accepted.status).toBe("ready");
    expect(accepted.maxReprojectionResidual).toBeCloseTo(
      2.1618083559610095e-7,
      15,
    );
    expect(accepted.solutions.some((solution) => solution !== null)).toBe(true);

    const rejectedPoint: Vec3 = [
      base[0] + FLOAT32_NORMAL[0] * 3.161808357136193e-7,
      base[1] + FLOAT32_NORMAL[1] * 3.161808357136193e-7,
      base[2] + FLOAT32_NORMAL[2] * 3.161808357136193e-7,
    ];
    expect(reprojectionResidual(rejectedPoint)).toBeCloseTo(
      3.1618083579334924e-7,
      15,
    );
    expect(reprojectionResidual(rejectedPoint)).toBeGreaterThan(
      SPATIAL_REPROJECTION_EPS,
    );

    const rejected = solveSpatialAlignOnCommonPlane({
      mode: "pointPoint",
      picks: [
        {
          kind: "point",
          world: rejectedPoint,
          supportPlanes: [FLOAT32_PLANE],
          foldedPoint: null,
        },
        {
          kind: "point",
          world: otherPoint,
          supportPlanes: [FLOAT32_PLANE],
          foldedPoint: null,
        },
      ],
      cursorWorld: null,
      placements: [FLOAT32_PLACEMENT],
    });
    expect(rejected.status).toBe("unavailable");
    expect(rejected.solutions).toEqual([]);
  });
});
