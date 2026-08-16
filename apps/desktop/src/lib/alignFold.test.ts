// 「合わせて折る」の幾何計算のテスト。
// すべて手計算で確かめられる数値だけを使う(近似で誤魔化さないため)。

import { describe, expect, it } from "vitest";
import {
  angleBisectors,
  alignRefPoint,
  distanceToLine,
  extendLine,
  foldPointOntoLine,
  foldPointOntoLinePerpendicular,
  foldTwoPointsOntoTwoLines,
  lineThroughPoints,
  movingSideOf,
  perpendicularThroughPoint,
  perpendicularBisector,
  reflectPointAcrossFold,
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

describe("藤田・羽鳥1と4(通過点・垂線)", () => {
  it("2点(0,0),(2,1)を通る折り目は、その2点の座標を丸めずに使う", () => {
    const line = lineThroughPoints([0, 0], [2, 1]);
    expect(line).toEqual([
      [0, 0],
      [2, 1],
    ]);
    passesThrough(line!, [0, 0]);
    passesThrough(line!, [2, 1]);
  });

  it("同じ点を2回選ぶと折り線は決まらない", () => {
    expect(lineThroughPoints([0.25, 0.75], [0.25, 0.75])).toBeNull();
  });

  it("点(0.25,0.5)を通り横線に垂直な折り目は x=0.25", () => {
    const line = perpendicularThroughPoint(
      [0.25, 0.5],
      [
        [0, 0],
        [1, 0],
      ],
    )!;
    onLine(line, 1, 0, 0.25);
    passesThrough(line, [0.25, 0.5]);
  });

  it("斜線 y=x に垂直な折り目も指定点を厳密に通る", () => {
    const p: Vec2 = [0.2, 0.3];
    const source: FoldLine = [
      [-1, -1],
      [2, 2],
    ];
    const line = perpendicularThroughPoint(p, source)!;
    passesThrough(line, p);
    const a: Vec2 = [source[1][0] - source[0][0], source[1][1] - source[0][1]];
    const b: Vec2 = [line[1][0] - line[0][0], line[1][1] - line[0][1]];
    expect(a[0] * b[0] + a[1] * b[1]).toBeCloseTo(0, 12);
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

describe("藤田・羽鳥6(2組の点→線を同時に合わせる)", () => {
  it("既知の折り目 x=0.5 を全実数解の中に含み、全候補が両方の反射条件を満たす", () => {
    const p1: Vec2 = [0, 0];
    const l1: FoldLine = [
      [1, -1],
      [1, 2],
    ]; // p1はx=0.5で折ると(1,0)へ移る
    const p2: Vec2 = [-1, 1];
    const l2: FoldLine = [
      [1, 0],
      [2, 1],
    ]; // p2はx=0.5で折ると(2,1)へ移る

    const out = foldTwoPointsOntoTwoLines(p1, l1, p2, l2);
    expect(out.length).toBeGreaterThan(0);
    expect(out.length).toBeLessThanOrEqual(3);
    expect(out.some((line) => distanceToLine(line, [0.5, 0]) <= 1e-9)).toBe(true);
    for (const fold of out) {
      const q1 = reflectPointAcrossFold(p1, fold)!;
      const q2 = reflectPointAcrossFold(p2, fold)!;
      expect(distanceToLine(l1, q1)).toBeLessThan(1e-8);
      expect(distanceToLine(l2, q2)).toBeLessThan(1e-8);
    }
  });

  it("n=(0,1)となる水平折り(三次方程式の無限遠の根)も落とさない", () => {
    const out = foldTwoPointsOntoTwoLines(
      [0, 1],
      [
        [-1, -1],
        [1, -1],
      ],
      [2, 2],
      [
        [2, -3],
        [2, 3],
      ],
    );
    // y=0で折ると(0,-1)と(2,-2)へ移り、それぞれの線に乗る。
    expect(out.some((line) => distanceToLine(line, [0, 0]) <= 1e-9)).toBe(true);
  });

  it("判別式が0に近い3実根でも、近接する2解を重根として落とさない", () => {
    const p1: Vec2 = [0.16620390651305184, 0.05606810980014021];
    const l1: FoldLine = [
      [0.7233129972797999, 0.08141874087277112],
      [0.7403485898762333, 0.08278827472852669],
    ];
    const p2: Vec2 = [0.15681505806280727, 0.9080062872025711];
    const l2: FoldLine = [
      [0.6552902502134852, 0.9306888477719542],
      [0.6689570877774842, 0.9317875760888786],
    ];
    const known: FoldLine = [
      [0.49241791908431437, -0.9786278355761696],
      [0.4015042125210822, 1.019304770384035],
    ];

    const out = foldTwoPointsOntoTwoLines(p1, l1, p2, l2);
    expect(out).toHaveLength(3);
    expect(
      out.some(
        (line) =>
          distanceToLine(line, known[0]) < 1e-9 &&
          distanceToLine(line, known[1]) < 1e-9,
      ),
    ).toBe(true);
    for (const fold of out) {
      const q1 = reflectPointAcrossFold(p1, fold)!;
      const q2 = reflectPointAcrossFold(p2, fold)!;
      expect(distanceToLine(l1, q1)).toBeLessThan(1e-8);
      expect(distanceToLine(l2, q2)).toBeLessThan(1e-8);
    }
  });
});

describe("藤田・羽鳥7(点→線と別の線への垂直を同時指定)", () => {
  it("P(0,2)をy=0へ重ね、x=0に垂直な折り目は y=1", () => {
    const line = foldPointOntoLinePerpendicular(
      [0, 2],
      [
        [-1, 0],
        [1, 0],
      ],
      [
        [0, -1],
        [0, 1],
      ],
    )!;
    onLine(line, 0, 1, 1);
    const reflected = reflectPointAcrossFold([0, 2], line)!;
    expect(reflected[0]).toBeCloseTo(0, 12);
    expect(reflected[1]).toBeCloseTo(0, 12);
  });

  it("斜線 y=x に垂直な折り目 x+y=1 で、点を縦線 x=1へ重ねる", () => {
    const source: Vec2 = [0, 0];
    const target: FoldLine = [
      [1, -1],
      [1, 2],
    ];
    const perpendicularTo: FoldLine = [
      [-1, -1],
      [2, 2],
    ];
    const line = foldPointOntoLinePerpendicular(source, target, perpendicularTo)!;
    onLine(line, 1, 1, 1);
    const reflected = reflectPointAcrossFold(source, line)!;
    expect(distanceToLine(target, reflected)).toBeLessThan(1e-12);
    const a: Vec2 = [
      perpendicularTo[1][0] - perpendicularTo[0][0],
      perpendicularTo[1][1] - perpendicularTo[0][1],
    ];
    const b: Vec2 = [line[1][0] - line[0][0], line[1][1] - line[0][1]];
    expect(a[0] * b[0] + a[1] * b[1]).toBeCloseTo(0, 12);
  });

  it("移動方向と合わせ先が平行で交わらなければ解なし", () => {
    expect(
      foldPointOntoLinePerpendicular(
        [0, 1],
        [
          [0, 0],
          [1, 0],
        ],
        [
          [0, 2],
          [1, 2],
        ],
      ),
    ).toBeNull();
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

  it("線と線: 45°でない角でも、二等分線の値が独立に求めた線と一致する", () => {
    // 紙の下辺(y=0)と対角線(y=x)がなす90°ではなく45°の角。
    // その二等分線は原点を通る22.5°の線で、傾きは tan22.5° = √2 - 1(無理数)。
    // 既存の検査は直交する2本(答えが y=±x)しか見ていないため、
    // 傾きが有理数でない場合に値が正しいかは、ここで初めて照合する。
    const bottom = [
      [0, 0],
      [1, 0],
    ] as FoldLine;
    const diagonal = [
      [0, 0],
      [1, 1],
    ] as FoldLine;
    const out = solveAlign("lineLine", [
      { kind: "line", a: bottom[0], b: bottom[1] },
      { kind: "line", a: diagonal[0], b: diagonal[1] },
    ]);
    expect(out.reason).toBeNull();
    expect(out.lines).toHaveLength(2);

    const tan22p5 = Math.SQRT2 - 1; // = tan(22.5°)
    const tan112p5 = -1 / tan22p5; // 直交するもう1本 = tan(112.5°)
    // 2本の解は「22.5°の線」と「112.5°の線」。どちらが先頭かは問わない。
    const slopes = out.lines
      .map((line) => (line[1][1] - line[0][1]) / (line[1][0] - line[0][0]))
      .sort((a, b) => a - b);
    expect(slopes[0]).toBeCloseTo(tan112p5, 12);
    expect(slopes[1]).toBeCloseTo(tan22p5, 12);
    // どちらも角の頂点(原点)を通る。
    for (const line of out.lines) passesThrough(line, [0, 0]);
    // 22.5°の線は、両方の辺から等しい距離にある点を通る。
    const onBisector: Vec2 = [1, tan22p5];
    expect(distanceToLine(bottom, onBisector)).toBeCloseTo(
      distanceToLine(diagonal, onBisector),
      12,
    );
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

  it("2点を通る・点を通り垂直・既存線を、その座標どおり解く", () => {
    const through = solveAlign("throughTwoPoints", [
      { kind: "point", p: [0.1, 0.2] },
      { kind: "point", p: [0.8, 0.6] },
    ]);
    expect(through.lines).toHaveLength(1);
    passesThrough(through.lines[0], [0.1, 0.2]);
    passesThrough(through.lines[0], [0.8, 0.6]);

    const perpendicular = solveAlign("pointPerpendicularLine", [
      { kind: "point", p: [0.25, 0.75] },
      { kind: "line", a: [0, 0], b: [1, 0] },
    ]);
    expect(perpendicular.lines).toHaveLength(1);
    onLine(perpendicular.lines[0], 1, 0, 0.25);

    const existing = solveAlign("existingLine", [
      { kind: "line", a: [0, 0.4], b: [1, 0.4] },
    ]);
    expect(existing.lines).toHaveLength(1);
    onLine(existing.lines[0], 0, 1, 0.4);
  });

  it("2組同時と点→線+垂直も共通入口から解ける", () => {
    const simultaneous = solveAlign("pointToLinePointToLine", [
      { kind: "point", p: [0, 0] },
      { kind: "line", a: [1, -1], b: [1, 2] },
      { kind: "point", p: [-1, 1] },
      { kind: "line", a: [1, 0], b: [2, 1] },
    ]);
    expect(simultaneous.lines.length).toBeGreaterThan(0);
    expect(simultaneous.lines.length).toBeLessThanOrEqual(3);

    const perpendicular = solveAlign("pointLinePerpendicular", [
      { kind: "point", p: [0, 2] },
      { kind: "line", a: [-1, 0], b: [1, 0] },
      { kind: "line", a: [0, -1], b: [0, 1] },
    ]);
    expect(perpendicular.lines).toHaveLength(1);
    onLine(perpendicular.lines[0], 0, 1, 1);
  });
});
