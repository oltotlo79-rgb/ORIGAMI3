// 「紙をつかんで引く」操作の幾何テスト。
// 紙は単位正方形を対角線(辺5)で2つに割ったもの。面0が根、面1が動く側。

import { describe, expect, it } from "vitest";
import {
  MAX_PULL_DEG,
  faceParents,
  hingeAnglesFromFrame,
  pathAxes,
  planPull,
  pullDeltaDeg,
} from "./grabDrive";
import type { Document, EdgeKind, Face, Frame3D } from "./types";

function makeDoc(kind: EdgeKind = "Mountain"): Document {
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
        { id: 5, v0: 0, v1: 2, kind },
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
}

const FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
  { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
];

/** 面1を対角線まわりに90°起こした立体(面0は平ら) */
const BENT: Frame3D = {
  faces: [
    { face: 0, polygon: [[0, 0, 0], [1, 0, 0], [1, 1, 0]], layer: 0 },
    { face: 1, polygon: [[0, 0, 0], [1, 1, 0], [0.5, 0.5, Math.SQRT1_2]], layer: 1 },
  ],
  warnings: [],
};

describe("faceParents", () => {
  it("最初に現れる面を根にして、折り線で親子をつなぐ", () => {
    const parents = faceParents(FACES);
    expect(parents.get(0)).toBeUndefined(); // 面0が根
    expect(parents.get(1)).toEqual({ parent: 0, hinge: 5 });
  });
});

describe("hingeAnglesFromFrame", () => {
  it("平ら(立体がまだ無い)なら全ての折り線が0度", () => {
    expect(hingeAnglesFromFrame(makeDoc(), FACES, null).get(5)).toBeCloseTo(0, 9);
  });

  it("折れている形からは折り角を読み取る", () => {
    // 面1は紙の表(+z)側へ90°起きている = 谷折り側なので負の角度
    expect(hingeAnglesFromFrame(makeDoc(), FACES, BENT).get(5)).toBeCloseTo(-90, 6);
  });

  it("平らに畳まれた所は展開図の線種で±180度を決める", () => {
    const flatFolded: Frame3D = {
      faces: [
        BENT.faces[0],
        { face: 1, polygon: [[0, 0, 0], [1, 1, 0], [1, 0, 0]], layer: 1 },
      ],
      warnings: [],
    };
    expect(hingeAnglesFromFrame(makeDoc("Mountain"), FACES, flatFolded).get(5)).toBe(180);
    expect(hingeAnglesFromFrame(makeDoc("Valley"), FACES, flatFolded).get(5)).toBe(-180);
  });
});

describe("planPull / pullDeltaDeg", () => {
  it("経路上の折り線と、つかんだ点の動く速度を返す", () => {
    // 面1の(0,1)付近をつかみ、+z方向へ引く
    const plan = planPull(makeDoc(), FACES, null, 1, [0, 1, 0], [0, 0, 1]);
    expect(plan?.hinge).toBe(5);
    expect(plan?.baseDeg).toBeCloseTo(0, 9);
    // 動きは軸に直交する向きだけ(紙の面内には逃げない)
    expect(plan!.velocity[0]).toBeCloseTo(0, 9);
    expect(plan!.velocity[1]).toBeCloseTo(0, 9);
    expect(Math.abs(plan!.velocity[2])).toBeCloseTo(Math.SQRT1_2, 9);
  });

  it("根の面をつかんだときは動かす折り線が無い", () => {
    expect(planPull(makeDoc(), FACES, null, 0, [0.5, 0.2, 0], [0, 0, 1])).toBeNull();
  });

  it("ドラッグ量は「つかんだ点が指に最も近づく回転量」に対応する", () => {
    const plan = planPull(makeDoc(), FACES, null, 1, [0, 1, 0], [0, 0, 1])!;
    const arm = Math.hypot(...plan.velocity);
    // 腕の長さarmの円弧を弧長dだけ動かす角度は d/arm ラジアン
    const deg = pullDeltaDeg(plan.velocity, [
      (plan.velocity[0] / arm) * 0.1,
      (plan.velocity[1] / arm) * 0.1,
      (plan.velocity[2] / arm) * 0.1,
    ]);
    expect(deg).toBeCloseTo(((0.1 / arm) * 180) / Math.PI, 6);
    // 逆向きに引けば符号が反転する
    const back = pullDeltaDeg(plan.velocity, [
      (-plan.velocity[0] / arm) * 0.1,
      (-plan.velocity[1] / arm) * 0.1,
      (-plan.velocity[2] / arm) * 0.1,
    ]);
    expect(back).toBeCloseTo(-deg, 9);
    // 紙の表(+z)側へ引くと谷折り側(負の角度)へ動く
    expect(pullDeltaDeg(plan.velocity, [0, 0, 0.1])).toBeLessThan(0);
  });

  it("行き過ぎたドラッグは上限で止める", () => {
    expect(pullDeltaDeg([0, 0, 1e-3], [0, 0, 100])).toBe(MAX_PULL_DEG);
    expect(pullDeltaDeg([0, 0, 1e-3], [0, 0, -100])).toBe(-MAX_PULL_DEG);
  });

  it("回転軸が定まらない(速度0)ときは角度を変えない", () => {
    expect(pullDeltaDeg([0, 0, 0], [1, 1, 1])).toBe(0);
  });
});

describe("pathAxes", () => {
  it("立体の形に合わせて回転軸の位置と向きを返す", () => {
    const axes = pathAxes(makeDoc(), FACES, BENT, 1);
    expect(axes).toHaveLength(1);
    expect(axes[0].hinge).toBe(5);
    // 軸は親(面0)の反時計回り境界の向き = 頂点2(1,1) → 頂点0(0,0)
    expect(axes[0].a).toEqual([1, 1, 0]);
    expect(axes[0].u[0]).toBeCloseTo(-Math.SQRT1_2, 9);
    expect(axes[0].u[1]).toBeCloseTo(-Math.SQRT1_2, 9);
    expect(axes[0].u[2]).toBeCloseTo(0, 9);
  });
});
