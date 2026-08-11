// 「仕上げの角度」(Poseステップ)の組み立てテスト。
// 角度は丸めずに記録する: 頂点まわりのループが閉じる関係は角度どうしを厳密に
// 結んでいて、丸めると再生のたびに「追従計算が収束していません」が出てしまう。

import { describe, expect, it } from "vitest";
import { buildPoseStep, currentAngles, hasPoseAngle, nextStepId } from "./poseStep";
import type { Document } from "./types";

const DOC: Document = {
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
      { id: 5, v0: 0, v1: 2, kind: "Mountain" },
    ],
    next_vertex_id: 4,
    next_edge_id: 6,
  },
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

/** 小数3桁にも9桁にも収まらない、ソルバーが返すような角度(端数のある実数) */
const RAW = -Math.PI * 43.7;

describe("buildPoseStep", () => {
  it("角度を丸めずにそのまま記録する", () => {
    const step = buildPoseStep(DOC, new Map([[5, RAW]]));
    expect(step.drivers).toHaveLength(1);
    const driver = step.drivers[0];
    expect(driver.a).toEqual([0, 0]);
    expect(driver.b).toEqual([1, 1]);
    expect(driver.target_angle_deg).toBe(RAW);
    // JSONへ書き出して読み直しても1ビットも変わらない(.ori3はJSON)
    expect(JSON.parse(JSON.stringify(driver)).target_angle_deg).toBe(RAW);
  });

  it("丸めていたころの桁数(小数3桁・9桁)より細かい値が残る", () => {
    const deg = buildPoseStep(DOC, new Map([[5, RAW]])).drivers[0].target_angle_deg;
    expect(deg).not.toBe(Number(RAW.toFixed(3)));
    expect(deg).not.toBe(Number(RAW.toFixed(9)));
  });

  it("角度の無い折り線と、値が有限でないものは書き出さない", () => {
    expect(buildPoseStep(DOC, new Map()).drivers).toHaveLength(0);
    expect(buildPoseStep(DOC, new Map([[5, NaN]])).drivers).toHaveLength(0);
  });

  it("手順IDは既存の最大+1", () => {
    expect(nextStepId(DOC)).toBe(0);
    expect(buildPoseStep(DOC, new Map([[5, RAW]])).kind).toBe("Pose");
  });
});

describe("currentAngles / hasPoseAngle", () => {
  it("追従計算の実角 → 利用者の希望値 → 0度 の順に決める", () => {
    const angles = currentAngles(
      new Set([0, 5, 9]),
      new Map([[5, 90]]),
      new Map([
        [5, 12],
        [9, RAW],
      ]),
    );
    expect(angles.get(5)).toBe(12);
    expect(angles.get(9)).toBe(RAW);
    expect(angles.get(0)).toBe(0);
  });

  it("希望90度より追従後の実角72.123456789度を丸めずに保存する", () => {
    const actual = 72.123456789;
    const angles = currentAngles(
      new Set([5]),
      new Map([[5, 90]]),
      new Map([[5, actual]]),
    );
    const step = buildPoseStep(DOC, angles);

    expect(step.drivers).toHaveLength(1);
    expect(step.drivers[0].target_angle_deg).toBe(actual);
    expect(step.drivers[0].target_angle_deg).not.toBe(90);
    expect(JSON.parse(JSON.stringify(step)).drivers[0].target_angle_deg).toBe(actual);
  });

  it("ほぼ平らなら残す意味がない", () => {
    expect(hasPoseAngle(new Map([[5, 0.1]]))).toBe(false);
    expect(hasPoseAngle(new Map([[5, RAW]]))).toBe(true);
  });
});
