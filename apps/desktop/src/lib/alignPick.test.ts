// 3D画面のクリックから点・線を拾う処理のテスト。
// 材料は畳み平面の層(輪郭の多角形)だけなので、手で書いた正方形で確かめられる。

import { describe, expect, it } from "vitest";
import {
  alignLineCandidates,
  alignVertexCandidates,
  nearestAlignLine,
  nearestAlignPoint,
  segmentIntersection,
  type AlignPickLayer,
} from "./alignPick";
import type { Vec2 } from "./types";

/** 単位正方形1枚 */
const SQUARE: AlignPickLayer = {
  polygon: [
    [0, 0],
    [1, 0],
    [1, 1],
    [0, 1],
  ],
};

/** 正方形を対角線で半分にした2枚(対角線が重なった層の共有辺になる) */
const HALVES: AlignPickLayer[] = [
  {
    polygon: [
      [0, 0],
      [1, 0],
      [1, 1],
    ],
  },
  {
    polygon: [
      [0, 0],
      [1, 1],
      [0, 1],
    ],
  },
];

describe("候補の集め方", () => {
  it("正方形1枚からは辺4本・角4つ", () => {
    expect(alignLineCandidates([SQUARE])).toHaveLength(4);
    expect(alignVertexCandidates([SQUARE])).toHaveLength(4);
  });

  it("重なった層が共有する辺・角は1つにまとめる", () => {
    // 三角形2枚 = 辺6本のうち対角線が共有なので5本、角は4つ
    expect(alignLineCandidates(HALVES)).toHaveLength(5);
    expect(alignVertexCandidates(HALVES)).toHaveLength(4);
  });

  it("1e-7未満でも許容差より離れた別の端点・線を同一視しない", () => {
    const close: AlignPickLayer[] = [
      {
        polygon: [
          [0, 0],
          [1, 0],
        ],
      },
      {
        polygon: [
          [0, 4e-8],
          [1, 4e-8],
        ],
      },
    ];
    expect(alignVertexCandidates(close)).toHaveLength(4);
    expect(alignLineCandidates(close)).toHaveLength(2);
  });

  it("長さ0の辺は候補に入れない", () => {
    const degenerate: AlignPickLayer = {
      polygon: [
        [0, 0],
        [0, 0],
        [1, 0],
        [1, 1],
      ],
    };
    expect(alignLineCandidates([degenerate])).toHaveLength(3);
  });
});

describe("線分の交点", () => {
  it("十字に交わる2本の交点を返す", () => {
    const x = segmentIntersection(
      [
        [-1, 0],
        [1, 0],
      ],
      [
        [0, -1],
        [0, 1],
      ],
    );
    expect(x).toEqual([0, 0]);
  });

  it("平行な2本はnull", () => {
    expect(
      segmentIntersection(
        [
          [0, 0],
          [1, 0],
        ],
        [
          [0, 1],
          [1, 1],
        ],
      ),
    ).toBeNull();
  });

  it("短い線分でも、直交していれば長さに依存せず交点を返す", () => {
    const x = segmentIntersection(
      [
        [-5e-6, 0],
        [5e-6, 0],
      ],
      [
        [0, -5e-6],
        [0, 5e-6],
      ],
    );
    expect(x).not.toBeNull();
    expect(x![0]).toBeCloseTo(0, 15);
    expect(x![1]).toBeCloseTo(0, 15);
  });

  it("延長すれば交わるが線分としては交わらない組はnull", () => {
    expect(
      segmentIntersection(
        [
          [0, 0],
          [1, 0],
        ],
        [
          [2, -1],
          [2, 1],
        ],
      ),
    ).toBeNull();
  });
});

describe("いちばん近いものを拾う", () => {
  it("角の近くをクリックすると、その角ちょうどの座標を返す(吸着)", () => {
    const p = nearestAlignPoint([SQUARE], [0.98, 1.02], 0.1);
    expect(p).toEqual([1, 1]);
  });

  it("半径の外なら点は拾わない", () => {
    expect(nearestAlignPoint([SQUARE], [0.5, 0.5], 0.1)).toBeNull();
  });

  it("角が無い場所でも、辺どうしの交点は拾える", () => {
    // 正方形の中を横切る2本の線(層として与える)。交点は(0.5,0.5)
    const cross: AlignPickLayer[] = [
      {
        polygon: [
          [0, 0.5],
          [1, 0.5],
        ],
      },
      {
        polygon: [
          [0.5, 0],
          [0.5, 1],
        ],
      },
    ];
    const p = nearestAlignPoint(cross, [0.52, 0.53], 0.06);
    expect(p![0]).toBeCloseTo(0.5, 12);
    expect(p![1]).toBeCloseTo(0.5, 12);
  });

  it("半径内に別の端点があっても、クリック位置に近い暗黙の交点を選ぶ", () => {
    const crossingWithNearbyEndpoint: AlignPickLayer[] = [
      {
        polygon: [
          [0, 0.5],
          [1, 0.5],
        ],
      },
      {
        polygon: [
          [0.5, 0],
          [0.5, 1],
        ],
      },
      {
        polygon: [
          [0.54, 0.54],
          [0.8, 0.8],
        ],
      },
    ];
    const p = nearestAlignPoint(crossingWithNearbyEndpoint, [0.5, 0.5], 0.08);
    expect(p![0]).toBeCloseTo(0.5, 12);
    expect(p![1]).toBeCloseTo(0.5, 12);
  });

  it("辺の近くをクリックするとその辺を拾う", () => {
    const line = nearestAlignLine([SQUARE], [0.5, 0.02], 0.1);
    expect(line).toEqual([
      [0, 0],
      [1, 0],
    ] as [Vec2, Vec2]);
  });

  it("辺から遠ければ線は拾わない", () => {
    expect(nearestAlignLine([SQUARE], [0.5, 0.5], 0.1)).toBeNull();
  });

  it("重なった層の共有辺(折り目)も1本の線として拾える", () => {
    // 線の向き(どちらが始点か)は輪郭のたどり方で決まるので、乗っている直線で確かめる
    const line = nearestAlignLine(HALVES, [0.5, 0.5], 0.1)!;
    expect(new Set([line[0].join(), line[1].join()])).toEqual(
      new Set(["0,0", "1,1"]),
    );
  });
});
