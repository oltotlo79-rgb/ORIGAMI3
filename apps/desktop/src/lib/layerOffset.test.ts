// 層のずらし表示(UI-010 / SIM-004)の計算のテスト。
// (1) 層ごとのずらし量: 下から順に等間隔で、層が多くても全体の厚みが決めた上限を超えない
// (2) 平坦判定: 平らに畳んだ状態のときだけずらす(折り途中の形は歪ませない)

import { describe, expect, it } from "vitest";
import type { Frame3D } from "./types";
import {
  MAX_STACK_RATIO,
  frameLayerCount,
  isFlatFrame,
  layerOffsets,
  stackLifts,
} from "./layerOffset";

/** 平らな三角形1枚(層を指定できる) */
function flatFace(face: number, layer: number, z = 0) {
  return {
    face,
    polygon: [
      [0, 0, z],
      [1, 0, z],
      [0, 1, z],
    ] as [number, number, number][],
    layer,
  };
}

describe("layerOffsets(層ごとのずらし量)", () => {
  it("層が無い・1枚だけのときはずらさない", () => {
    expect(layerOffsets(0, 1)).toEqual([]);
    expect(layerOffsets(1, 1)).toEqual([0]);
  });

  it("下(層0)から順に等間隔で高くなる", () => {
    const offsets = layerOffsets(4, 1);
    expect(offsets).toHaveLength(4);
    expect(offsets[0]).toBe(0);
    const step = offsets[1];
    expect(step).toBeGreaterThan(0);
    for (let i = 1; i < offsets.length; i++) {
      expect(offsets[i] - offsets[i - 1]).toBeCloseTo(step, 12);
    }
  });

  it("層が多いときは全体の厚みが上限(紙の長辺の割合)を超えない", () => {
    for (const count of [2, 3, 8, 16, 64, 256]) {
      const offsets = layerOffsets(count, 1);
      expect(offsets).toHaveLength(count);
      expect(offsets[count - 1]).toBeLessThanOrEqual(MAX_STACK_RATIO + 1e-12);
    }
  });

  it("層が増えても厚みは増えるだけで減らない(上限までは間隔一定)", () => {
    const few = layerOffsets(3, 1);
    const many = layerOffsets(30, 1);
    expect(many[29]).toBeGreaterThanOrEqual(few[2]);
    // 上限に達した後は間隔が狭まる(不自然に分厚くならない)
    expect(many[1]).toBeLessThan(few[1]);
  });

  it("紙の大きさに比例する", () => {
    const unit = layerOffsets(5, 1);
    const half = layerOffsets(5, 0.5);
    for (let i = 0; i < unit.length; i++) {
      expect(half[i]).toBeCloseTo(unit[i] / 2, 12);
    }
  });

  it("おかしな値(負・小数)でもずらさないだけで壊れない", () => {
    expect(layerOffsets(-3, 1)).toEqual([]);
    expect(layerOffsets(3, 0)).toEqual([0, 0, 0]);
    expect(layerOffsets(3, -1)).toEqual([0, 0, 0]);
  });
});

describe("isFlatFrame(平らに畳んだ状態か)", () => {
  it("全ての面がz=0に乗っていれば平ら", () => {
    const frame: Frame3D = {
      faces: [flatFace(0, 0), flatFace(1, 1)],
      warnings: [],
    };
    expect(isFlatFrame(frame)).toBe(true);
  });

  it("高さのある面が1つでもあれば平らでない(折り途中は歪ませない)", () => {
    const frame: Frame3D = {
      faces: [flatFace(0, 0), flatFace(1, 1, 0.3)],
      warnings: [],
    };
    expect(isFlatFrame(frame)).toBe(false);
  });

  it("面が無いときは平らとみなさない(ずらす対象が無い)", () => {
    expect(isFlatFrame({ faces: [], warnings: [] })).toBe(false);
  });

  it("計算誤差ほどの高さは平らとみなす", () => {
    const frame: Frame3D = { faces: [flatFace(0, 0, 1e-9)], warnings: [] };
    expect(isFlatFrame(frame)).toBe(true);
  });
});

describe("frameLayerCount(重なりの枚数)", () => {
  it("層番号の最大+1を返す(層は下から0)", () => {
    const frame: Frame3D = {
      faces: [flatFace(0, 2), flatFace(1, 0), flatFace(2, 1)],
      warnings: [],
    };
    expect(frameLayerCount(frame)).toBe(3);
  });

  it("面が無いときは0", () => {
    expect(frameLayerCount({ faces: [], warnings: [] })).toBe(0);
  });
});

describe("stackLifts(重なった面のずらし)", () => {
  const step = layerOffsets(2, 1)[1];

  it("平らに重なった面は層の順に+zへ離れる(これまでの平坦時と同じ)", () => {
    const frame: Frame3D = {
      faces: [flatFace(0, 0), flatFace(1, 1)],
      warnings: [],
    };
    const lifts = stackLifts(frame, 1);
    expect(lifts[0]).toEqual([0, 0, 0]);
    expect(lifts[1][2]).toBeCloseTo(step, 12);
    expect(lifts[1][0]).toBeCloseTo(0, 12);
  });

  it("有効なsurface rankがlayerと逆ならrankの順に離れる", () => {
    const frame: Frame3D = {
      faces: [
        { ...flatFace(0, 0), surface_rank: 1 },
        { ...flatFace(1, 1), surface_rank: 0 },
      ],
      warnings: [],
    };
    const lifts = stackLifts(frame, 1);
    expect(lifts[0][2]).toBeCloseTo(step, 12);
    expect(lifts[1]).toEqual([0, 0, 0]);
  });

  it("surface rankが欠落・重複・範囲外・非整数ならframe全体をlayer順へ戻す", () => {
    const cases: Frame3D["faces"][] = [
      [
        { ...flatFace(0, 0), surface_rank: 1 },
        flatFace(1, 1),
      ],
      [
        { ...flatFace(0, 0), surface_rank: 0 },
        { ...flatFace(1, 1), surface_rank: 0 },
      ],
      [
        { ...flatFace(0, 0), surface_rank: 0 },
        { ...flatFace(1, 1), surface_rank: 2 },
      ],
      [
        { ...flatFace(0, 0), surface_rank: 0 },
        { ...flatFace(1, 1), surface_rank: 0.5 },
      ],
    ];
    for (const faces of cases) {
      const lifts = stackLifts({ faces, warnings: [] }, 1);
      expect(lifts[0]).toEqual([0, 0, 0]);
      expect(lifts[1][2]).toBeCloseTo(step, 12);
    }
  });

  it("層が同じ面は離さない(展開した1枚の紙がばらけない)", () => {
    const frame: Frame3D = {
      faces: [flatFace(0, 0), flatFace(1, 0), flatFace(2, 0)],
      warnings: [],
    };
    expect(stackLifts(frame, 1)).toEqual([
      [0, 0, 0],
      [0, 0, 0],
      [0, 0, 0],
    ]);
  });

  it("裏返って重なった面(法線が逆向き)も同じ向きへ積み上げる", () => {
    // 面1は頂点の並びが逆=法線が-z。同じ平面の仲間として+z側へ積む
    const flipped = {
      face: 1,
      polygon: [
        [0, 1, 0],
        [1, 0, 0],
        [0, 0, 0],
      ] as [number, number, number][],
      layer: 1,
    };
    const lifts = stackLifts({ faces: [flatFace(0, 0), flipped], warnings: [] }, 1);
    expect(lifts[0]).toEqual([0, 0, 0]);
    expect(lifts[1][2]).toBeCloseTo(step, 12);
  });

  it("折り途中・立体でも、重なった面はその平面の法線方向へ離れる", () => {
    // x=0の平面(法線±x)に重なった2枚。zへ足しても離れないので法線方向へ離す
    const wall = (face: number, layer: number) => ({
      face,
      polygon: [
        [0, 0, 0],
        [0, 1, 0],
        [0, 1, 1],
      ] as [number, number, number][],
      layer,
    });
    const lifts = stackLifts({ faces: [wall(0, 0), wall(1, 1)], warnings: [] }, 1);
    expect(lifts[0]).toEqual([0, 0, 0]);
    expect(Math.abs(lifts[1][0])).toBeCloseTo(step, 12);
    expect(lifts[1][1]).toBeCloseTo(0, 12);
    expect(lifts[1][2]).toBeCloseTo(0, 12);
  });

  it("平面が違う面どうしは離さない(立体の形を歪ませない)", () => {
    const frame: Frame3D = {
      faces: [flatFace(0, 0), flatFace(1, 1, 0.5)],
      warnings: [],
    };
    expect(stackLifts(frame, 1)).toEqual([
      [0, 0, 0],
      [0, 0, 0],
    ]);
  });

  it("面積の無い面・面が無いフレームでも壊れない", () => {
    const degenerate = {
      face: 0,
      polygon: [
        [0, 0, 0],
        [1, 1, 1],
        [2, 2, 2],
      ] as [number, number, number][],
      layer: 3,
    };
    expect(stackLifts({ faces: [degenerate], warnings: [] }, 1)).toEqual([[0, 0, 0]]);
    expect(stackLifts({ faces: [], warnings: [] }, 1)).toEqual([]);
  });

  it("重なりが厚くても全体の厚みは上限を超えない", () => {
    const faces = Array.from({ length: 20 }, (_, i) => flatFace(i, i));
    const lifts = stackLifts({ faces, warnings: [] }, 1);
    expect(lifts[19][2]).toBeLessThanOrEqual(MAX_STACK_RATIO + 1e-12);
    expect(lifts[19][2]).toBeGreaterThan(0);
  });
});
