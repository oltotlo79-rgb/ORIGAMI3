// 作図補助(CPE-005)の計算のテスト。Rust側(crates/ori3-cp/tests/construct.rs)と
// 同じ図形で同じ答えになることを確かめる。

import { describe, expect, it } from "vitest";
import {
  bisector,
  clipToPaper,
  constructHint,
  constructLines,
  directionLines,
  dividePoints,
  perpendicular,
} from "./construct";
import type { Vec2 } from "./types";

const near = (a: Vec2, b: Vec2) => Math.hypot(a[0] - b[0], a[1] - b[1]) < 1e-9;

describe("作図の計算", () => {
  it("直角の二等分線は45°方向へ伸びる", () => {
    const line = bisector([1, 0], [0, 0], [0, 1]);
    expect(line).not.toBeNull();
    const [a, b] = line as [Vec2, Vec2];
    expect(near(a, [0, 0])).toBe(true);
    expect(b[0] - b[1]).toBeCloseTo(0, 12);
    expect(Math.hypot(b[0], b[1])).toBeCloseTo(1, 12);
  });

  it("腕の長さがゼロなら二等分線は作らない", () => {
    expect(bisector([0, 0], [0, 0], [1, 0])).toBeNull();
  });

  it("垂線の足は線を延長した直線の上に落ちる", () => {
    const line = perpendicular([2, 1], [[0, 0], [1, 0]]);
    expect(near((line as [Vec2, Vec2])[1], [2, 0])).toBe(true);
  });

  it("等分点は両端を含まないn-1個", () => {
    const pts = dividePoints([[0, 0], [1, 0]], 4);
    expect(pts.length).toBe(3);
    expect(near(pts[1], [0.5, 0])).toBe(true);
    expect(dividePoints([[0, 0], [1, 0]], 1)).toEqual([]);
    expect(dividePoints([[0, 0], [1, 0]], 9)).toEqual([]);
  });

  it("22.5°刻みの方向線は8本で、どれも指定した点を通る", () => {
    const lines = directionLines([0.5, 0.5], 22.5);
    expect(lines.length).toBe(8);
    for (const [a, b] of lines) {
      expect(near([(a[0] + b[0]) / 2, (a[1] + b[1]) / 2], [0.5, 0.5])).toBe(true);
    }
    expect(directionLines([0, 0], 0)).toEqual([]);
  });

  it("紙の外へ出た線は紙の縁で切り取る", () => {
    const clipped = clipToPaper([[-1, 0.5], [2, 0.5]], 1, 1);
    expect(near((clipped as [Vec2, Vec2])[0], [0, 0.5])).toBe(true);
    expect(near((clipped as [Vec2, Vec2])[1], [1, 0.5])).toBe(true);
    expect(clipToPaper([[-1, 2], [2, 2]], 1, 1)).toBeNull();
  });
});

describe("引く補助線の組み立て", () => {
  const opts = { divisions: 4, stepDeg: 22.5, paper: [1, 1] as Vec2 };

  it("クリックが足りないうちは線を作らない", () => {
    expect(constructLines("bisector", [[0, 0], [1, 0]], null, opts)).toEqual([]);
    expect(constructLines("angle", [], null, opts)).toEqual([]);
  });

  it("角度線は紙の中に収まる8本になる", () => {
    const lines = constructLines("angle", [[0.5, 0.5]], null, opts);
    expect(lines.length).toBe(8);
    for (const [a, b] of [...lines].flatMap((l) => [l])) {
      for (const p of [a, b]) {
        expect(p[0]).toBeGreaterThanOrEqual(-1e-9);
        expect(p[0]).toBeLessThanOrEqual(1 + 1e-9);
        expect(p[1]).toBeGreaterThanOrEqual(-1e-9);
        expect(p[1]).toBeLessThanOrEqual(1 + 1e-9);
      }
    }
  });

  it("等分は各点を端点にした目印(n-1本)を作る", () => {
    const lines = constructLines("divide", [[0, 0.5], [1, 0.5]], null, opts);
    expect(lines.length).toBe(3);
    expect(near(lines[0][0], [0.25, 0.5])).toBe(true);
  });

  it("案内は日本語で、次にすることを伝える", () => {
    expect(constructHint("bisector", 0, 4)).toContain("角の1本目");
    expect(constructHint("divide", 0, 6)).toContain("6等分");
    expect(constructHint("angle", 0, 4)).toContain("方向線");
  });
});
