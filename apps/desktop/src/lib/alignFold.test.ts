// 「合わせて折る」の幾何計算のテスト。
// すべて手計算で確かめられる数値だけを使う(近似で誤魔化さないため)。

import { describe, expect, it } from "vitest";
import {
  angleBisectors,
  alignRefPoint,
  distanceToLine,
  extendLine,
  foldPointOntoLine,
  movingSideOf,
  perpendicularBisector,
  solveAlign,
  sortByCursor,
  type FoldLine,
} from "./alignFold";
import type { Vec2 } from "./types";

/** 線が直線 c0*x + c1*y = c2 の上に乗っているか(両端点で確かめる) */
function onLine(line: FoldLine, c0: number, c1: number, c2: number) {
  for (const p of line) {
    expect(c0 * p[0] + c1 * p[1]).toBeCloseTo(c2, 9);
  }
}

/** 線が点を通るか */
function passesThrough(line: FoldLine, p: Vec2) {
  expect(distanceToLine(line, p)).toBeCloseTo(0, 9);
}

describe("perpendicularBisector(点と点を合わせる)", () => {
  it("(0,0)と(1,1)の垂直二等分線は y = 1 - x", () => {
    const line = perpendicularBisector([0, 0], [1, 1]);
    expect(line).not.toBeNull();
    // 中点(0.5,0.5)から左右へ |pq|/2 = √2/2 ずつ = (1,0)と(0,1)
    expect(line![0][0]).toBeCloseTo(1, 12);
    expect(line![0][1]).toBeCloseTo(0, 12);
    expect(line![1][0]).toBeCloseTo(0, 12);
    expect(line![1][1]).toBeCloseTo(1, 12);
    onLine(line!, 1, 1, 1); // x + y = 1
  });

  it("(0,0)と(2,0)の垂直二等分線は x = 1(縦の線)", () => {
    const line = perpendicularBisector([0, 0], [2, 0])!;
    onLine(line, 1, 0, 1);
    passesThrough(line, [1, 0]);
  });

  it("同じ点を2つ選ぶとnull", () => {
    expect(perpendicularBisector([0.5, 0.5], [0.5, 0.5])).toBeNull();
  });
});

describe("angleBisectors(線と線を合わせる)", () => {
  it("x軸とy軸が交わるときは y=x と y=-x の2本", () => {
    const out = angleBisectors(
      [
        [0, 0],
        [1, 0],
      ],
      [
        [0, 0],
        [0, 1],
      ],
    );
    expect(out).toHaveLength(2);
    onLine(out[0], 1, -1, 0); // y = x
    onLine(out[1], 1, 1, 0); // y = -x
  });

  it("交点が原点でなくても、その交点を通る2本になる", () => {
    // 点(2,1)で交わる横線と縦線
    const out = angleBisectors(
      [
        [0, 1],
        [4, 1],
      ],
      [
        [2, -1],
        [2, 3],
      ],
    );
    expect(out).toHaveLength(2);
    for (const line of out) passesThrough(line, [2, 1]);
    // 傾きは ±1(横線と縦線の二等分)
    onLine(out[0], 1, -1, 1); // y = x - 1
    onLine(out[1], 1, 1, 3); // y = -x + 3
  });

  it("平行な2本(y=0とy=2)の解は中間線 y=1 の1本だけ", () => {
    const out = angleBisectors(
      [
        [0, 0],
        [1, 0],
      ],
      [
        [0, 2],
        [1, 2],
      ],
    );
    expect(out).toHaveLength(1);
    onLine(out[0], 0, 1, 1); // y = 1
  });

  it("向きを逆に選んでも解の集まりは同じ(2本とも出る)", () => {
    const out = angleBisectors(
      [
        [1, 0],
        [0, 0],
      ],
      [
        [0, 0],
        [0, 1],
      ],
    );
    expect(out).toHaveLength(2);
    const dists = out.map((l) => distanceToLine(l, [1, 1])).sort();
    // y=x は(1,1)を通り距離0、y=-x は距離√2
    expect(dists[0]).toBeCloseTo(0, 9);
    expect(dists[1]).toBeCloseTo(Math.SQRT2, 9);
  });
});

describe("foldPointOntoLine(点を線に合わせる+折り目が通る点)", () => {
  const xAxis: FoldLine = [
    [0, 0],
    [1, 0],
  ];

  it("解が2つ: P(0,1)をx軸へ、原点を通る折り → y=x と y=-x", () => {
    const out = foldPointOntoLine([0, 1], xAxis, [0, 0]);
    expect(out).toHaveLength(2);
    for (const line of out) passesThrough(line, [0, 0]);
    const slopes = out
      .map((l) => (l[1][1] - l[0][1]) / (l[1][0] - l[0][0]))
      .sort((a, b) => a - b);
    expect(slopes[0]).toBeCloseTo(-1, 9);
    expect(slopes[1]).toBeCloseTo(1, 9);
  });

  it("解が1つ(接する): P(0,1)をx軸へ、(0,0.5)を通る折り → y=0.5", () => {
    const out = foldPointOntoLine([0, 1], xAxis, [0, 0.5]);
    expect(out).toHaveLength(1);
    onLine(out[0], 0, 1, 0.5);
  });

  it("解が0つ: 折り目が通る点が線から遠すぎる(円が届かない)", () => {
    // Q=(0,3), P=(0,1) → 半径2、線までの距離3 なので交点なし
    expect(foldPointOntoLine([0, 1], xAxis, [0, 3])).toHaveLength(0);
  });

  it("点が既に線の上にあるときは、動かない解を除いて1本だけ返す", () => {
    // P=(1,0)はx軸上。Q=(0,0)を中心・半径1の円との交点は(1,0)と(-1,0)。
    // (1,0)はPと同じなので折りにならず、(-1,0)へ移す x=0 だけが残る
    const out = foldPointOntoLine([1, 0], xAxis, [0, 0]);
    expect(out).toHaveLength(1);
    onLine(out[0], 1, 0, 0); // x = 0
  });
});

describe("動かす側と解の並べ替え", () => {
  it("1つ目に選んだ点がある側を動かす側にする", () => {
    // 折り線は x軸(左→右)。上側(y>0)が左、下側が右
    const line: FoldLine = [
      [0, 0],
      [1, 0],
    ];
    expect(movingSideOf(line, [0.5, 1])).toBe("left");
    expect(movingSideOf(line, [0.5, -1])).toBe("right");
    // 線の上に乗っている点は判定できないので既定側
    expect(movingSideOf(line, [0.5, 0])).toBe("right");
  });

  it("線を選んだときの代表点は中点", () => {
    expect(alignRefPoint({ kind: "line", a: [0, 0], b: [2, 4] })).toEqual([1, 2]);
    expect(alignRefPoint({ kind: "point", p: [3, 5] })).toEqual([3, 5]);
  });

  it("カーソルに近い解が先頭に来る", () => {
    const a: FoldLine = [
      [0, 0],
      [1, 0],
    ]; // y = 0
    const b: FoldLine = [
      [0, 5],
      [1, 5],
    ]; // y = 5
    expect(sortByCursor([b, a], [0, 0.1])[0]).toBe(a);
    expect(sortByCursor([a, b], [0, 4.9])[0]).toBe(b);
  });

  it("伸ばしても乗っている直線は変わらない", () => {
    const line = extendLine(
      [
        [0, 1],
        [1, 1],
      ],
      3,
    );
    onLine(line, 0, 1, 1);
    expect(Math.hypot(line[1][0] - line[0][0], line[1][1] - line[0][1])).toBeCloseTo(6, 9);
  });
});

describe("solveAlign(合わせ方ごとの入口)", () => {
  it("選択が足りないうちは解も理由も出さない", () => {
    expect(solveAlign("pointPoint", [{ kind: "point", p: [0, 0] }])).toEqual({
      lines: [],
      reason: null,
    });
  });

  it("点と点: 解1本で、紙より長い線分になる(下見と可動側の判定用)", () => {
    const out = solveAlign("pointPoint", [
      { kind: "point", p: [0, 0] },
      { kind: "point", p: [1, 1] },
    ]);
    expect(out.lines).toHaveLength(1);
    onLine(out.lines[0], 1, 1, 1);
    const len = Math.hypot(
      out.lines[0][1][0] - out.lines[0][0][0],
      out.lines[0][1][1] - out.lines[0][0][1],
    );
    expect(len).toBeCloseTo(2, 9);
  });

  it("線と線: カーソルに近い解が既定になる", () => {
    const picks = [
      { kind: "line" as const, a: [0, 0] as Vec2, b: [1, 0] as Vec2 },
      { kind: "line" as const, a: [0, 0] as Vec2, b: [0, 1] as Vec2 },
    ];
    // (1,1)寄りのカーソルなら y=x が先頭
    const near = solveAlign("lineLine", picks, [1, 1]);
    expect(near.lines).toHaveLength(2);
    onLine(near.lines[0], 1, -1, 0);
    // (1,-1)寄りなら y=-x が先頭
    const far = solveAlign("lineLine", picks, [1, -1]);
    onLine(far.lines[0], 1, 1, 0);
  });

  it("点を線に合わせる: 届かないときは日本語の理由を返す", () => {
    const out = solveAlign("pointLineThrough", [
      { kind: "point", p: [0, 1] },
      { kind: "line", a: [0, 0], b: [1, 0] },
      { kind: "point", p: [0, 3] },
    ]);
    expect(out.lines).toHaveLength(0);
    expect(out.reason).toContain("届きません");
  });

  it("同じ点を2つ選ぶと理由を返す", () => {
    const out = solveAlign("pointPoint", [
      { kind: "point", p: [0.5, 0.5] },
      { kind: "point", p: [0.5, 0.5] },
    ]);
    expect(out.lines).toHaveLength(0);
    expect(out.reason).toContain("同じ位置");
  });
});
