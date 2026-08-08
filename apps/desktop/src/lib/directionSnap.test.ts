import { describe, expect, it } from "vitest";
import type { Document, Edge, Vec2, Vertex } from "./types";
import {
  DIRECTION_SNAP_ANGLE_DEG,
  directionCandidatesAt,
  intersectDirectionRayWithSegment,
  projectPointToDirectionRay,
  snapLineDirection,
  snapToDirection,
  type DirectionCandidate,
} from "./directionSnap";

function doc(vertices: Vertex[], edges: Edge[]): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices,
      edges,
      next_vertex_id: vertices.length,
      next_edge_id: edges.length,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

const axisDoc = () =>
  doc(
    [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [1, 0] },
      { id: 2, pos: [0, 1] },
    ],
    [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 0, v1: 2, kind: "Mountain" },
    ],
  );

function hasDirection(
  candidates: DirectionCandidate[],
  expected: Vec2,
  kind: DirectionCandidate["kind"],
): boolean {
  return candidates.some(
    (candidate) =>
      candidate.kind === kind &&
      Math.abs(candidate.direction[0] - expected[0]) < 1e-10 &&
      Math.abs(candidate.direction[1] - expected[1]) < 1e-10,
  );
}

function polar(deg: number, radius: number): Vec2 {
  const rad = (deg * Math.PI) / 180;
  return [Math.cos(rad) * radius, Math.sin(rad) * radius];
}

describe("directionCandidatesAt", () => {
  it("x軸とy軸の間に45°の二等分方向を両側へ列挙する", () => {
    const candidates = directionCandidatesAt(axisDoc(), [0, 0]);
    const d = Math.SQRT1_2;
    expect(hasDirection(candidates, [d, d], "bisector")).toBe(true);
    expect(hasDirection(candidates, [-d, -d], "bisector")).toBe(true);
  });

  it("接続線そのものの延長を両方向へ列挙する", () => {
    const candidates = directionCandidatesAt(axisDoc(), [0, 0]);
    for (const direction of [
      [1, 0],
      [-1, 0],
      [0, 1],
      [0, -1],
    ] as Vec2[]) {
      expect(hasDirection(candidates, direction, "extension")).toBe(true);
    }
  });

  it("線の途中を始点にしたときは延長だけを候補にする", () => {
    const line = doc(
      [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
      ],
      [{ id: 0, v0: 0, v1: 1, kind: "Aux" }],
    );
    const candidates = directionCandidatesAt(line, [0.4, 0]);
    expect(candidates).toHaveLength(2);
    expect(candidates.every((candidate) => candidate.kind === "extension")).toBe(true);
  });

  it("一直線につながる2本の180°の角は垂直方向で二等分する", () => {
    const line = doc(
      [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [-1, 0] },
      ],
      [
        { id: 0, v0: 0, v1: 1, kind: "Valley" },
        { id: 1, v0: 0, v1: 2, kind: "Valley" },
      ],
    );
    const candidates = directionCandidatesAt(line, [0, 0]);
    expect(hasDirection(candidates, [0, 1], "bisector")).toBe(true);
    expect(hasDirection(candidates, [0, -1], "bisector")).toBe(true);
  });

  it("壊れた参照・長さ0の線を無視し、重複方向を作らない", () => {
    const broken = doc(
      [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [0, 0] },
      ],
      [
        { id: 0, v0: 0, v1: 1, kind: "Border" },
        { id: 1, v0: 0, v1: 1, kind: "Mountain" },
        { id: 2, v0: 0, v1: 2, kind: "Valley" },
        { id: 3, v0: 0, v1: 999, kind: "Aux" },
      ],
    );
    expect(directionCandidatesAt(broken, [0, 0])).toEqual([
      { direction: [1, 0], kind: "extension" },
      { direction: [-1, 0], kind: "extension" },
    ]);
  });
});

describe("snapToDirection", () => {
  it("5°以内の最も近い方向へ吸着し、カーソルまでの長さを保つ", () => {
    const result = snapLineDirection(axisDoc(), [0, 0], polar(48, 2));
    expect(result?.kind).toBe("bisector");
    expect(result?.direction[0]).toBeCloseTo(Math.SQRT1_2, 10);
    expect(result?.direction[1]).toBeCloseTo(Math.SQRT1_2, 10);
    expect(Math.hypot(result?.pos[0] ?? 0, result?.pos[1] ?? 0)).toBeCloseTo(2, 10);
  });

  it("判定角を超える方向には吸着しない", () => {
    const candidates: DirectionCandidate[] = [
      { direction: [Math.SQRT1_2, Math.SQRT1_2], kind: "bisector" },
    ];
    expect(
      snapToDirection([0, 0], polar(45 + DIRECTION_SNAP_ANGLE_DEG, 1), candidates),
    ).not.toBeNull();
    expect(
      snapToDirection([0, 0], polar(45 + DIRECTION_SNAP_ANGLE_DEG + 0.1, 1), candidates),
    ).toBeNull();
  });

  it("始点とカーソルが同じなら吸着しない", () => {
    expect(snapLineDirection(axisDoc(), [0, 0], [0, 0])).toBeNull();
  });
});

describe("方向軸上の対応点", () => {
  it("点を半直線へ垂直投影し、単位長でない方向も扱う", () => {
    const projected = projectPointToDirectionRay([0.1, 0.2], [2, 0], [0.7, 0.5]);
    expect(projected?.[0]).toBeCloseTo(0.7, 12);
    expect(projected?.[1]).toBeCloseTo(0.2, 12);
  });

  it("投影先が始点より後ろか方向が長さ0なら投影しない", () => {
    expect(projectPointToDirectionRay([0, 0], [1, 0], [-0.1, 0.2])).toBeNull();
    expect(projectPointToDirectionRay([0, 0], [0, 0], [0.1, 0.2])).toBeNull();
  });

  it("半直線と線分の交点を求める", () => {
    const intersection = intersectDirectionRayWithSegment(
      [0, 0],
      [2, 2],
      [0.2, 0.5],
      [0.8, 0.5],
    );
    expect(intersection?.[0]).toBeCloseTo(0.5, 12);
    expect(intersection?.[1]).toBeCloseTo(0.5, 12);
  });

  it("後方・線分外・平行・長さ0方向には交点を作らない", () => {
    expect(
      intersectDirectionRayWithSegment([0, 0], [1, 0], [-1, -1], [-1, 1]),
    ).toBeNull();
    expect(
      intersectDirectionRayWithSegment([0, 0], [1, 0], [0.5, 1], [0.5, 2]),
    ).toBeNull();
    expect(
      intersectDirectionRayWithSegment([0, 0], [1, 0], [0, 1], [1, 1]),
    ).toBeNull();
    expect(
      intersectDirectionRayWithSegment([0, 0], [0, 0], [0, -1], [0, 1]),
    ).toBeNull();
  });
});
