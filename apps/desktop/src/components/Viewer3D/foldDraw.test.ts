// 畳み平面上の折り線描画の幾何計算(層の取り出し・吸着・動く側の判定)のテスト。

import { describe, expect, it } from "vitest";
import type { Document, Face, Frame3D, Vec2 } from "../../lib/types";
import {
  clipToMovingSide,
  foldLayers,
  foldPreviewSegments,
  keepSidePoint,
  movingLayers,
  snapFoldPoint,
  topMovingFace,
} from "./foldDraw";

/** 正方形1面の展開図 */
function makeDoc(): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [1, 1] },
        { id: 3, pos: [0, 1] },
      ],
      edges: [
        { id: 0, v0: 0, v1: 1, kind: "Border" },
        { id: 1, v0: 1, v1: 2, kind: "Border" },
        { id: 2, v0: 2, v1: 3, kind: "Border" },
        { id: 3, v0: 3, v1: 0, kind: "Border" },
      ],
      next_vertex_id: 4,
      next_edge_id: 4,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

const SQUARE_FACE: Face[] = [{ id: 7, vertices: [0, 1, 2, 3], edges: [0, 1, 2, 3] }];

/** 同じ位置に2枚重なった状態(下=面0、上=面1)のFrame3D */
function stackedFrame(z = 0): Frame3D {
  const quad = (h: number): [number, number, number][] => [
    [0, 0, h],
    [1, 0, h],
    [1, 1, h],
    [0, 1, h],
  ];
  return {
    faces: [
      { face: 0, polygon: quad(0), layer: 0 },
      { face: 1, polygon: quad(z), layer: 1 },
    ],
    warnings: [],
  };
}

/** 進行方向+yの折り線(x=0.5) */
const LINE_UP: [Vec2, Vec2] = [
  [0.5, 0],
  [0.5, 1],
];

describe("foldLayers", () => {
  it("立体形状があれば畳み平面上の層として取り出す", () => {
    const layers = foldLayers(stackedFrame(), makeDoc(), SQUARE_FACE);
    expect(layers.map((l) => l.face)).toEqual([0, 1]);
    expect(layers.map((l) => l.layer)).toEqual([0, 1]);
    expect(layers[0].polygon).toEqual([
      [0, 0],
      [1, 0],
      [1, 1],
      [0, 1],
    ]);
  });

  it("平らに畳まれていない面(高さのある面)は除く", () => {
    const layers = foldLayers(stackedFrame(0.3), makeDoc(), SQUARE_FACE);
    expect(layers.map((l) => l.face)).toEqual([0]);
  });

  it("まだ折っていない(立体形状がない)ときは展開図をそのまま層にする", () => {
    const layers = foldLayers(null, makeDoc(), SQUARE_FACE);
    expect(layers).toHaveLength(1);
    expect(layers[0].face).toBe(7);
    expect(layers[0].layer).toBe(0);
    expect(layers[0].polygon).toEqual([
      [0, 0],
      [1, 0],
      [1, 1],
      [0, 1],
    ]);
  });
});

describe("keepSidePoint", () => {
  it("動かす側の反対側の点を返す(線の進行方向に対する左右)", () => {
    // +y向きの線: 右=x大きい側、左=x小さい側
    const right = keepSidePoint(LINE_UP, "right");
    expect(right[0]).toBeLessThan(0.5); // 右を動かすなら残るのは左
    const left = keepSidePoint(LINE_UP, "left");
    expect(left[0]).toBeGreaterThan(0.5);
    // 折り線上には乗らない
    expect(Math.abs(right[0] - 0.5)).toBeGreaterThan(1e-3);
  });
});

describe("movingLayers / topMovingFace", () => {
  it("可動側に掛かる層を選び、いちばん上の1枚を返す", () => {
    const layers = foldLayers(stackedFrame(), makeDoc(), SQUARE_FACE);
    const keep = keepSidePoint(LINE_UP, "right");
    expect(movingLayers(layers, LINE_UP, keep).map((l) => l.face)).toEqual([0, 1]);
    expect(topMovingFace(layers, LINE_UP, keep)).toBe(1);
  });

  it("可動側に紙が無ければ空(いちばん上の1枚もnull)", () => {
    const layers = foldLayers(stackedFrame(), makeDoc(), SQUARE_FACE);
    // 紙の右外にある線(x=2)で、動く側をさらに右にすると対象が無い
    const line: [Vec2, Vec2] = [
      [2, 0],
      [2, 1],
    ];
    const keep = keepSidePoint(line, "right");
    expect(movingLayers(layers, line, keep)).toEqual([]);
    expect(topMovingFace(layers, line, keep)).toBeNull();
  });
});

describe("clipToMovingSide", () => {
  it("多角形の動く側だけを切り出す", () => {
    const square: Vec2[] = [
      [0, 0],
      [1, 0],
      [1, 1],
      [0, 1],
    ];
    // +y向きの線(x=0.5)で右側を動かす → 右半分だけが残る
    const clipped = clipToMovingSide(square, LINE_UP, keepSidePoint(LINE_UP, "right"));
    expect(clipped).toHaveLength(4);
    expect(clipped.every((p) => p[0] >= 0.5 - 1e-9)).toBe(true);
    expect(Math.max(...clipped.map((p) => p[0]))).toBeCloseTo(1, 9);
  });

  it("動く側に掛からない多角形では空になる", () => {
    const left: Vec2[] = [
      [0, 0],
      [0.2, 0],
      [0.2, 1],
      [0, 1],
    ];
    expect(clipToMovingSide(left, LINE_UP, keepSidePoint(LINE_UP, "right"))).toEqual([]);
  });
});

describe("foldPreviewSegments", () => {
  it("折り線と、動く側に入る部分の輪郭を返す(上の1枚だけの指定にも従う)", () => {
    const layers = foldLayers(stackedFrame(), makeDoc(), SQUARE_FACE);
    const keep = keepSidePoint(LINE_UP, "right");
    expect(foldPreviewSegments(layers, LINE_UP, null)).toEqual([[LINE_UP[0], LINE_UP[1]]]);
    // 折り線1本 + 2層×(切り取った四角形の4辺)
    const all = foldPreviewSegments(layers, LINE_UP, keep);
    expect(all).toHaveLength(9);
    // 動かさない側(x<0.5)へはみ出さない
    expect(all.slice(1).every(([a, b]) => a[0] >= 0.5 - 1e-9 && b[0] >= 0.5 - 1e-9)).toBe(true);
    // 折り線1本 + 上の1層×4辺
    expect(foldPreviewSegments(layers, LINE_UP, keep, true)).toHaveLength(5);
  });
});

describe("snapFoldPoint", () => {
  it("半径内の頂点へ吸着する", () => {
    const layers = foldLayers(stackedFrame(), makeDoc(), SQUARE_FACE);
    expect(snapFoldPoint(layers, [0.97, 1.02], 0.1)).toEqual([1, 1]);
  });

  it("頂点が遠ければ輪郭の上へ吸着する", () => {
    const layers = foldLayers(stackedFrame(), makeDoc(), SQUARE_FACE);
    expect(snapFoldPoint(layers, [0.5, 1.02], 0.1)).toEqual([0.5, 1]);
  });

  it("半径内に候補が無ければそのままの位置を返す", () => {
    const layers = foldLayers(stackedFrame(), makeDoc(), SQUARE_FACE);
    expect(snapFoldPoint(layers, [0.5, 0.5], 0.1)).toEqual([0.5, 0.5]);
  });
});
