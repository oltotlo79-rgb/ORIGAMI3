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
