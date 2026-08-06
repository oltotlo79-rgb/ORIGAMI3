// 展開図から対称軸を見つける計算のテスト(UI-007)。

import { describe, expect, it } from "vitest";
import type { Segment } from "./mirror";
import {
  axisAt,
  buildSegmentIndex,
  candidateAngles,
  findMirrorAxes,
  findSegment,
  symmetryScore,
} from "./symmetry";
import type { Paper, Vec2 } from "./types";

const SQUARE: Paper = { width_mm: 150, height_mm: 150 };
const OBLONG: Paper = { width_mm: 100, height_mm: 200 };

/** 軸の角度(度。0以上180未満) */
function angleOf(d: Vec2): number {
  return (((Math.atan2(d[1], d[0]) * 180) / Math.PI) + 180) % 180;
}

function index(segs: Segment[]) {
  return buildSegmentIndex(segs.map((s, i) => [i, s] as [number, Segment]));
}

describe("findSegment", () => {
  it("同じ位置の線分を向きの違いも含めて見つける", () => {
    const ix = index([
      [
        [0.1, 0.2],
        [0.4, 0.6],
      ],
      [
        [0.7, 0.1],
        [0.9, 0.3],
      ],
    ]);
    expect(findSegment(ix, [[0.4, 0.6], [0.1, 0.2]])).toBe(0);
    expect(findSegment(ix, [[0.7, 0.1], [0.9, 0.3]])).toBe(1);
    expect(findSegment(ix, [[0.7, 0.1], [0.9, 0.4]])).toBeNull();
  });

  it("格子のマス目をまたいでも見つかる(誤差はマスよりずっと小さい)", () => {
    const ix = index([[[0.0005, 0.0005], [0.0015, 0.0015]]]);
    expect(findSegment(ix, [[0.0005, 0.0005], [0.0015 + 5e-7, 0.0015]])).toBe(0);
  });
});

describe("candidateAngles", () => {
  it("紙の縦・横・対角を必ず候補に入れる", () => {
    const got = candidateAngles(SQUARE, []);
    for (const want of [90, 0, 45, 135]) {
      expect(got.some((a) => Math.abs(a - want) < 1e-9), `${want}°`).toBe(true);
    }
  });

  it("長方形の紙では対角の角度が紙の縦横比で決まる", () => {
    const got = candidateAngles(OBLONG, []);
    expect(got.some((a) => Math.abs(a - 63.4349488) < 1e-6)).toBe(true);
  });

  it("折り線の向きの組から軸の候補を作る(有限個に抑える)", () => {
    // 30°と60°の折り線 → その平均45°(と直交する135°)が候補に入る
    const segs: Segment[] = [
      [[0, 0], [Math.cos(Math.PI / 6), Math.sin(Math.PI / 6)]],
      [[0, 0], [Math.cos(Math.PI / 3), Math.sin(Math.PI / 3)]],
    ];
    const got = candidateAngles(SQUARE, segs);
    expect(got.some((a) => Math.abs(a - 45) < 1e-6)).toBe(true);
    expect(got.length).toBeLessThanOrEqual(32);
  });
});

describe("findMirrorAxes", () => {
  /** x=0.5 をはさんで左右対称な折り線の集まり */
  const symmetric: Segment[] = [
    [[0.2, 0.1], [0.3, 0.4]],
    [[0.8, 0.1], [0.7, 0.4]],
    [[0.1, 0.6], [0.4, 0.9]],
    [[0.9, 0.6], [0.6, 0.9]],
  ];

  it("左右対称な展開図では紙の縦の中心線が軸になる", () => {
    const axes = findMirrorAxes(SQUARE, index(symmetric));
    expect(axes.length).toBeGreaterThan(0);
    expect(angleOf(axes[0].d)).toBeCloseTo(90);
    expect(symmetryScore(index(symmetric), axisAt(SQUARE, 90))).toBe(1);
  });

  it("非対称な展開図では軸が見つからない", () => {
    const asym: Segment[] = [
      [[0.2, 0.1], [0.3, 0.4]],
      [[0.15, 0.6], [0.42, 0.83]],
      [[0.31, 0.22], [0.47, 0.09]],
    ];
    expect(findMirrorAxes(SQUARE, index(asym))).toEqual([]);
  });

  it("軸が複数あるときは根の面を保つ軸を先に返す", () => {
    // 上の集まりは縦の中心線でも横の中心線でも対称(2本の軸がある)
    const both = [...symmetric, ...symmetric.map((s) => s.map((p) => [p[0], 1 - p[1]]) as Segment)];
    const ix = index(both);
    const found = findMirrorAxes(SQUARE, ix).map((a) => angleOf(a.d));
    expect(found).toContain(0);
    expect(found).toContain(90);
    expect(symmetryScore(ix, axisAt(SQUARE, 0))).toBe(1);
    expect(symmetryScore(ix, axisAt(SQUARE, 90))).toBe(1);
    // y=0.5 をまたぐ三角形が根の面なら、横の中心線(0°)だけがそれを保つ
    const root: Vec2[] = [[0.4, 0.4], [0.6, 0.5], [0.4, 0.6]];
    expect(angleOf(findMirrorAxes(SQUARE, ix, root)[0].d)).toBeCloseTo(0);
  });
});

describe("性能", () => {
  it("辺2万本の展開図でも引き始めの1回が十分速い", () => {
    // 100x100の格子(縦線・横線・斜め線)= 約2万本。x=0.5について左右対称
    const segs: Segment[] = [];
    const n = 100;
    for (let i = 0; i <= n; i++) {
      for (let j = 0; j < n; j++) {
        segs.push([[i / n, j / n], [i / n, (j + 1) / n]]);
        segs.push([[j / n, i / n], [(j + 1) / n, i / n]]);
      }
    }
    expect(segs.length).toBeGreaterThan(20000);
    const start = performance.now();
    const axes = findMirrorAxes(SQUARE, index(segs));
    const ms = performance.now() - start;
    expect(axes.length).toBeGreaterThan(0);
    expect(ms).toBeLessThan(500);
  });
});
