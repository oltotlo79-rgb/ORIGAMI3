// 「紙をつかんで引く」操作の幾何テスト。
// 紙は単位正方形を対角線(辺5)で2つに割ったもの。面0が根、面1が動く側。

import { describe, expect, it } from "vitest";
import {
  MAX_PULL_DEG,
  faceParents,
  hingeAnglesFromFrame,
  mirrorHingeOf,
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

  // 実際の紙はどこをつかんでも動かせる。ソルバーが根の面をその場に固定する都合で
  // 「根をつかむと動かない」となっていたのを、相手側を逆に動かす形で解消した
  it("根の面をつかんでも、接する折り線を逆向きに駆動して動かせる", () => {
    const grab: [number, number, number] = [0.5, 0.2, 0];
    const root = planPull(makeDoc(), FACES, null, 0, grab, [0, 0, 1]);
    expect(root?.hinge).toBe(5);
    expect(root?.baseDeg).toBeCloseTo(0, 9);
    expect(Math.hypot(...root!.velocity)).toBeGreaterThan(0.1);
    // 同じ点を同じ向きへ引いても、根の側と相手側では角度の動く向きが逆になる
    const child = planPull(makeDoc(), FACES, null, 1, grab, [0, 0, 1])!;
    expect(pullDeltaDeg(root!.velocity, [0, 0, 0.1])).toBeCloseTo(
      -pullDeltaDeg(child.velocity, [0, 0, 0.1]),
      9,
    );
  });

  it("折り線がまったく無い1枚きりの紙は動かしようがない", () => {
    const solo = [{ id: 0, vertices: [0, 1, 2, 3], edges: [0, 1, 2, 3] }];
    expect(planPull(makeDoc(), solo, null, 0, [0.5, 0.2, 0], [0, 0, 1])).toBeNull();
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

// --- 左右対称の相手を一緒に動かす(UI-007) ---------------------------------
// 鶴の羽のように、胴(根の面)の左右に対の折り線がある展開図で確かめる。
// 実物の鶴の展開図はRust側のテスト(acceptance_crane.rs)でしか組み立てられない
// ため、ここでは「胴+左右の羽」という同じつながり方の紙で代用する。

/** 胴(中央)の左右に羽が付いた紙。rightXを動かすと左右対称でなくなる */
function makeWingDoc(rightX = 0.7): Document {
  return {
    ...makeDoc(),
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [0.3, 0] },
        { id: 2, pos: [rightX, 0] },
        { id: 3, pos: [1, 0] },
        { id: 4, pos: [1, 1] },
        { id: 5, pos: [rightX, 1] },
        { id: 6, pos: [0.3, 1] },
        { id: 7, pos: [0, 1] },
      ],
      edges: [
        { id: 0, v0: 0, v1: 1, kind: "Border" },
        { id: 1, v0: 1, v1: 2, kind: "Border" },
        { id: 2, v0: 2, v1: 3, kind: "Border" },
        { id: 3, v0: 3, v1: 4, kind: "Border" },
        { id: 4, v0: 4, v1: 5, kind: "Border" },
        { id: 5, v0: 5, v1: 6, kind: "Border" },
        { id: 6, v0: 6, v1: 7, kind: "Border" },
        { id: 7, v0: 7, v1: 0, kind: "Border" },
        { id: 8, v0: 1, v1: 6, kind: "Mountain" }, // 左の羽の付け根
        { id: 9, v0: 2, v1: 5, kind: "Mountain" }, // 右の羽の付け根
      ],
      next_vertex_id: 8,
      next_edge_id: 10,
    },
  };
}

/** 面0が胴(根)、面1が左の羽、面2が右の羽 */
const WING_FACES: Face[] = [
  { id: 0, vertices: [1, 2, 5, 6], edges: [1, 9, 5, 8] },
  { id: 1, vertices: [0, 1, 6, 7], edges: [0, 8, 6, 7] },
  { id: 2, vertices: [2, 3, 4, 5], edges: [2, 3, 4, 9] },
];

/** 中心線(x=0.5)そのものが折り線になっている紙 */
function makeCenterDoc(): Document {
  return {
    ...makeDoc(),
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [0.5, 0] },
        { id: 2, pos: [1, 0] },
        { id: 3, pos: [1, 1] },
        { id: 4, pos: [0.5, 1] },
        { id: 5, pos: [0, 1] },
      ],
      edges: [
        { id: 0, v0: 0, v1: 1, kind: "Border" },
        { id: 1, v0: 1, v1: 2, kind: "Border" },
        { id: 2, v0: 2, v1: 3, kind: "Border" },
        { id: 3, v0: 3, v1: 4, kind: "Border" },
        { id: 4, v0: 4, v1: 5, kind: "Border" },
        { id: 5, v0: 5, v1: 0, kind: "Border" },
        { id: 6, v0: 1, v1: 4, kind: "Mountain" },
      ],
      next_vertex_id: 6,
      next_edge_id: 7,
    },
  };
}

const CENTER_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 4, 5], edges: [0, 6, 4, 5] },
  { id: 1, vertices: [1, 2, 3, 4], edges: [1, 2, 3, 6] },
];

describe("mirrorHingeOf", () => {
  it("紙の縦の中心線をはさんで対になる折り線を見つける", () => {
    const doc = makeWingDoc();
    expect(mirrorHingeOf(doc, WING_FACES, 8)).toBe(9);
    expect(mirrorHingeOf(doc, WING_FACES, 9)).toBe(8); // 逆向きでも対になる
  });

  it("中心線の上にある折り線は相手なし(1本だけ動かす)", () => {
    expect(mirrorHingeOf(makeCenterDoc(), CENTER_FACES, 6)).toBeNull();
    // 対角線も中心線で折り返すともう1本の対角線になるが、そこには線が無い
    expect(mirrorHingeOf(makeDoc(), FACES, 5)).toBeNull();
  });

  it("左右対称でない展開図では相手なし", () => {
    // 右の羽の付け根だけ x=0.8 にずらすと、x=0.3 の鏡映(x=0.7)に線が無い
    expect(mirrorHingeOf(makeWingDoc(0.8), WING_FACES, 8)).toBeNull();
  });

  it("輪郭の辺は相手に選ばない(動かしても形が変わらないため)", () => {
    // 辺0(左下の輪郭)の鏡映は辺2(右下の輪郭)だが、折り線ではないのでnull
    expect(mirrorHingeOf(makeWingDoc(), WING_FACES, 0)).toBeNull();
  });

  it("紙が縦長でも中心線は紙の幅の真ん中で決まる", () => {
    const doc = makeWingDoc();
    // 幅75mm・高さ150mm(長辺=150)なら中心線はx=0.25。x=0.3の鏡映はx=0.2
    const narrow = { ...doc, paper: { width_mm: 75, height_mm: 150 } };
    expect(mirrorHingeOf(narrow, WING_FACES, 8)).toBeNull();
  });
});

describe("planPull(左右同時)", () => {
  it("左の羽をつかむと、右の羽の折り線も一緒に動かす相手として返る", () => {
    const doc = makeWingDoc();
    const plan = planPull(doc, WING_FACES, null, 1, [0, 0.5, 0], [0, 0, 0], true);
    expect(plan?.hinge).toBe(8);
    expect(plan?.mirrorHinge).toBe(9);
  });

  it("左右同時を切っていれば、従来どおり1本だけ", () => {
    const doc = makeWingDoc();
    const plan = planPull(doc, WING_FACES, null, 1, [0, 0.5, 0]);
    expect(plan?.hinge).toBe(8);
    expect(plan?.mirrorHinge).toBeNull();
  });

  it("左右同時でも、対称な相手が無い展開図では1本だけ", () => {
    const doc = makeWingDoc(0.8);
    const plan = planPull(doc, WING_FACES, null, 1, [0, 0.5, 0], [0, 0, 0], true);
    expect(plan?.hinge).toBe(8);
    expect(plan?.mirrorHinge).toBeNull();
  });
});

// 折り鶴のように「体の軸が紙の対角線」になる作品の左右(UI-007)。
// このアプリの折り操作で折った鶴は、正方形を半分に2回折って組み立てるので
// 首と尾が向かい合う2隅、羽が残りの2隅から出る。つまり展開図の上での左右の
// 折り返し軸は紙の対角線であって、縦の中心線ではない。
// ここでは鶴の肩にあたる形(体の左右に対の折り線が1本ずつ)で確かめる。

/** 対角線(0,0)-(1,1)を体の軸にして、左右に羽の付け根がある正方形の紙 */
function makeDiagonalDoc(): Document {
  return {
    ...makeDoc(),
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [1, 1] },
        { id: 3, pos: [0, 1] },
        { id: 4, pos: [0.5, 0] },
        { id: 5, pos: [1, 0.5] },
        { id: 6, pos: [0.5, 1] },
        { id: 7, pos: [0, 0.5] },
      ],
      edges: [
        { id: 0, v0: 0, v1: 4, kind: "Border" },
        { id: 1, v0: 4, v1: 1, kind: "Border" },
        { id: 2, v0: 1, v1: 5, kind: "Border" },
        { id: 3, v0: 5, v1: 2, kind: "Border" },
        { id: 4, v0: 2, v1: 6, kind: "Border" },
        { id: 5, v0: 6, v1: 3, kind: "Border" },
        { id: 6, v0: 3, v1: 7, kind: "Border" },
        { id: 7, v0: 7, v1: 0, kind: "Border" },
        { id: 8, v0: 4, v1: 5, kind: "Mountain" }, // 一方の羽の付け根
        { id: 9, v0: 7, v1: 6, kind: "Mountain" }, // もう一方の羽の付け根
      ],
      next_vertex_id: 8,
      next_edge_id: 10,
    },
  };
}

/** 面0が体(根)、面1と面2が左右の羽 */
const DIAGONAL_FACES: Face[] = [
  { id: 0, vertices: [0, 4, 5, 2, 6, 7], edges: [0, 8, 3, 4, 9, 7] },
  { id: 1, vertices: [4, 1, 5], edges: [1, 2, 8] },
  { id: 2, vertices: [7, 6, 3], edges: [9, 5, 6] },
];

describe("体の軸が紙の対角線のとき(折り鶴と同じ並び)", () => {
  it("縦の中心線では対にならない折り線も、対角線をはさんで対になる", () => {
    const doc = makeDiagonalDoc();
    // 縦の中心線(x=0.5)で折り返すと、辺8は折り線の無い所へ移る
    expect(mirrorHingeOf(doc, DIAGONAL_FACES, 8)).toBe(9);
    expect(mirrorHingeOf(doc, DIAGONAL_FACES, 9)).toBe(8);
  });

  it("片方の羽をつかむと、もう片方の羽の折り線も一緒に動く", () => {
    const doc = makeDiagonalDoc();
    const plan = planPull(
      doc,
      DIAGONAL_FACES,
      null,
      1,
      [0.9, 0.1, 0],
      [0, 0, 0],
      true,
    );
    expect(plan?.hinge).toBe(8);
    expect(plan?.mirrorHinge).toBe(9);
  });

  it("長方形の紙では対角線を軸にしない(左右の意味が変わってしまうため)", () => {
    const doc = makeDiagonalDoc();
    const oblong = { ...doc, paper: { width_mm: 100, height_mm: 150 } };
    expect(mirrorHingeOf(oblong, DIAGONAL_FACES, 8)).toBeNull();
  });
});
