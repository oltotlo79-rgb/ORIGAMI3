// 非平坦な3D表示上の線を、保存可能な材料(CP)座標へ一意に戻す純粋境界の先行検査。
// Face ID・層数・表示順はwireへ出さず、曖昧な場合は推測しない。

import { describe, expect, it } from "vitest";
import type { Vec2 } from "../../lib/types";
import type { Vec3 } from "../../lib/layerOffset";
import {
  mapPoint,
  pointInPolygon,
  type FacePlacement,
} from "./edgeHighlight";
import { materialFoldLineFrom3D } from "./cpPick3d";

const MATERIAL_LINE: [Vec2, Vec2] = [
  [0.2, 0.3],
  [0.8, 0.7],
];
const MATERIAL_KEEP_SIDE_POINT: Vec2 = [0.25, 0.8];

function placement(
  faceId: number,
  q0: Vec3,
  f1: Vec3,
  f2: Vec3,
  materialOffset: Vec2 = [0, 0],
): FacePlacement {
  const [ox, oy] = materialOffset;
  return {
    faceId,
    polygon: [
      [ox, oy],
      [ox + 1, oy],
      [ox + 1, oy + 1],
      [ox, oy + 1],
    ],
    p0: [ox, oy],
    e1: [1, 0],
    e2: [0, 1],
    q0,
    f1,
    f2,
    det: 1,
  };
}

const TILTED = placement(17, [0.1, -0.2, 0.4], [1, 0, 0.6], [0, 1, 0.25]);
const VERTICAL = placement(29, [2, -0.5, 0.1], [0, 1, 0], [0, 0, 1]);

function world(placement: FacePlacement, material: Vec2): Vec3 {
  const mapped = mapPoint(placement, material);
  if (mapped === null) throw new Error("test placement did not map material point");
  return mapped;
}

function resolve(
  one: FacePlacement,
  options: {
    line?: [Vec2, Vec2];
    keep?: Vec2;
    placements?: readonly FacePlacement[];
    lineWorld?: [Vec3, Vec3];
    keepSideWorld?: Vec3;
  } = {},
) {
  const line = options.line ?? MATERIAL_LINE;
  return materialFoldLineFrom3D(
    options.placements ?? [one],
    options.lineWorld ?? [world(one, line[0]), world(one, line[1])],
    options.keepSideWorld ?? world(one, options.keep ?? MATERIAL_KEEP_SIDE_POINT),
  );
}

describe("3D折り線から材料座標への一意な逆写像", () => {
  it("3D線上の2点をmaterial_lineへ戻す", () => {
    const result = resolve(TILTED);

    expect(result?.material_line[0][0]).toBeCloseTo(MATERIAL_LINE[0][0], 12);
    expect(result?.material_line[0][1]).toBeCloseTo(MATERIAL_LINE[0][1], 12);
    expect(result?.material_line[1][0]).toBeCloseTo(MATERIAL_LINE[1][0], 12);
    expect(result?.material_line[1][1]).toBeCloseTo(MATERIAL_LINE[1][1], 12);
  });

  it("material_keep_side_pointは材料面の厳密な内部点になる", () => {
    const result = resolve(TILTED);

    expect(result?.material_keep_side_point[0]).toBeCloseTo(
      MATERIAL_KEEP_SIDE_POINT[0],
      12,
    );
    expect(result?.material_keep_side_point[1]).toBeCloseTo(
      MATERIAL_KEEP_SIDE_POINT[1],
      12,
    );
    expect(
      result === null
        ? false
        : pointInPolygon(TILTED.polygon, result.material_keep_side_point, 0),
    ).toBe(true);
    const [a, b] = result?.material_line ?? MATERIAL_LINE;
    const keep = result?.material_keep_side_point ?? a;
    const side =
      (b[0] - a[0]) * (keep[1] - a[1]) -
      (b[1] - a[1]) * (keep[0] - a[0]);
    expect(Math.abs(side)).toBeGreaterThan(1e-9);
  });

  it.each([
    ["傾斜面", TILTED],
    ["垂直面", VERTICAL],
  ] as const)("%sでもglobal XYへ潰さず一意に戻す", (_name, one) => {
    const result = resolve(one);

    expect(result).not.toBeNull();
    expect(result?.material_line[0][0]).toBeCloseTo(MATERIAL_LINE[0][0], 12);
    expect(result?.material_line[0][1]).toBeCloseTo(MATERIAL_LINE[0][1], 12);
    expect(result?.material_line[1][0]).toBeCloseTo(MATERIAL_LINE[1][0], 12);
    expect(result?.material_line[1][1]).toBeCloseTo(MATERIAL_LINE[1][1], 12);
  });

  it("内部点が境界・紙外・面外の3D点なら推測せずnullにする", () => {
    expect(resolve(TILTED, { keep: [0, 0.5] })).toBeNull();
    expect(resolve(TILTED, { keep: [1.2, 0.5] })).toBeNull();
    const above = world(TILTED, MATERIAL_KEEP_SIDE_POINT);
    expect(
      resolve(TILTED, {
        keepSideWorld: [above[0], above[1], above[2] + 0.25],
      }),
    ).toBeNull();
  });

  it("境界許容差の内側でも紙外のkeep点は拒否し、厳密な内部点だけを返す", () => {
    // pointInPolygonの許容差(1e-9)より小さい紙外量でも、wireへ紙外点を出さない。
    expect(resolve(TILTED, { keep: [-1e-12, 0.5] })).toBeNull();
    expect(resolve(TILTED, { keep: [1 + 1e-12, 0.5] })).toBeNull();

    const result = resolve(TILTED, { keep: [1e-7, 0.5] });
    expect(result).not.toBeNull();
    expect(result?.material_keep_side_point[0]).toBeGreaterThan(0);
    expect(result?.material_keep_side_point[0]).toBeLessThan(1);
  });

  it("同じ3D線は100回同じ材料直線になり、無効候補の配列順にも依存しない", () => {
    const lineWorld: [Vec3, Vec3] = [
      world(TILTED, MATERIAL_LINE[0]),
      world(TILTED, MATERIAL_LINE[1]),
    ];
    const keepSideWorld = world(TILTED, MATERIAL_KEEP_SIDE_POINT);
    const expected = materialFoldLineFrom3D([TILTED], lineWorld, keepSideWorld);
    expect(expected).not.toBeNull();

    for (let i = 0; i < 100; i++) {
      expect(materialFoldLineFrom3D([TILTED], lineWorld, keepSideWorld)).toEqual(expected);
    }
    expect(materialFoldLineFrom3D([VERTICAL, TILTED], lineWorld, keepSideWorld)).toEqual(
      expected,
    );
    expect(materialFoldLineFrom3D([TILTED, VERTICAL], lineWorld, keepSideWorld)).toEqual(
      expected,
    );
  });

  it("同じ3D点に材料座標の複数候補があればFace ID順で選ばずnullにする", () => {
    const secondMaterialChart = placement(
      3,
      TILTED.q0,
      TILTED.f1,
      TILTED.f2,
      [10, 20],
    );

    expect(
      resolve(TILTED, {
        placements: [TILTED, secondMaterialChart],
      }),
    ).toBeNull();
  });

  it("長さ0は拒否するが、同じ面の境界上にある一意な線は戻せる", () => {
    const same = world(TILTED, [0.4, 0.4]);
    expect(resolve(TILTED, { lineWorld: [same, same] })).toBeNull();
    const boundaryLine: [Vec2, Vec2] = [
      [0, 0.2],
      [0, 0.8],
    ];
    const result = resolve(TILTED, { line: boundaryLine });
    expect(result?.material_line[0][0]).toBeCloseTo(0, 12);
    expect(result?.material_line[0][1]).toBeCloseTo(0.2, 12);
    expect(result?.material_line[1][0]).toBeCloseTo(0, 12);
    expect(result?.material_line[1][1]).toBeCloseTo(0.8, 12);
  });

  it("wire材料は2座標だけで、Face ID・2*K・層数・surface_rankを含まない", () => {
    const result = resolve(VERTICAL);
    if (result === null) throw new Error("unique vertical material line was rejected");

    expect(Object.keys(result).sort()).toEqual([
      "material_keep_side_point",
      "material_line",
    ]);
    const wire = result as unknown as Record<string, unknown>;
    for (const forbidden of [
      "face",
      "faceId",
      "face_id",
      "faces",
      "target_layers",
      "topPleatCount",
      "selectedLayerCount",
      "layerCount",
      "surface_rank",
    ]) {
      expect(Object.prototype.hasOwnProperty.call(wire, forbidden), forbidden).toBe(false);
    }
  });
});
