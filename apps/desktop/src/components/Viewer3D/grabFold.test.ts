import { describe, expect, it } from "vitest";
import type { Face, Vec2 } from "../../lib/types";
import type { FoldLayer } from "./foldDraw";
import {
  bisectorLine,
  flapFaces,
  planGrabFold,
  reflectPoint,
  snapFoldLine,
} from "./grabFold";

/** 1枚の正方形(長辺1.0) */
const SQUARE: FoldLayer[] = [
  { face: 0, layer: 0, polygon: [[0, 0], [1, 0], [1, 1], [0, 1]] },
];
const SQUARE_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2, 3], edges: [10, 11, 12, 13] },
];

/** x=0.5で半分に折った状態。折り目(辺11)で2枚がつながっている */
const HALF: FoldLayer[] = [
  { face: 0, layer: 0, polygon: [[0, 0], [0.5, 0], [0.5, 1], [0, 1]] },
  { face: 1, layer: 1, polygon: [[0.5, 0], [0, 0], [0, 1], [0.5, 1]] },
];
const HALF_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2, 3], edges: [10, 11, 12, 13] },
  { id: 1, vertices: [4, 5, 6, 7], edges: [21, 22, 23, 11] },
];

/** 折り線から見た点の符号(左右) */
function sideSign(line: [Vec2, Vec2], p: Vec2): number {
  const ux = line[1][0] - line[0][0];
  const uy = line[1][1] - line[0][1];
  return Math.sign(ux * (p[1] - line[0][1]) - uy * (p[0] - line[0][0]));
}

describe("bisectorLine", () => {
  it("2点の中点を通り、2点を結ぶ向きと直交する", () => {
    const line = bisectorLine([0.2, 0.5], [0.8, 0.5], 1)!;
    expect(line[0][0]).toBeCloseTo(0.5);
    expect(line[1][0]).toBeCloseTo(0.5);
    expect(sideSign(line, [0.2, 0.5])).toBe(-sideSign(line, [0.8, 0.5]));
  });

  it("同じ点ならnull", () => {
    expect(bisectorLine([0.5, 0.5], [0.5, 0.5], 1)).toBeNull();
  });
});

describe("snapFoldLine", () => {
  it("近くに向きの合う縁があれば、その線へ吸着する", () => {
    // 二等分線はわずかに傾くが、紙の縁x=0.5が向きも位置も近いので吸着する
    const line = snapFoldLine(HALF, [0.44, 0.5], [0.58, 0.52], 1)!;
    expect(line[0][0]).toBeCloseTo(0.5);
    expect(line[1][0]).toBeCloseTo(0.5);
  });

  it("向きの合う線が無ければ垂直二等分線のまま", () => {
    const line = snapFoldLine(SQUARE, [0.1, 0.1], [0.9, 0.9], 1)!;
    // x+y=1 の線(45度)。中点(0.5,0.5)を通る
    expect(line[0][0] + line[0][1]).toBeCloseTo(1);
    expect(line[1][0] + line[1][1]).toBeCloseTo(1);
  });
});

describe("reflectPoint", () => {
  it("折り線で鏡映する", () => {
    const line: [Vec2, Vec2] = [[0.5, 0], [0.5, 1]];
    expect(reflectPoint([0.2, 0.3], line)[0]).toBeCloseTo(0.8);
    expect(reflectPoint([0.2, 0.3], line)[1]).toBeCloseTo(0.3);
  });
});

describe("flapFaces", () => {
  it("折り線の動く側でつながっている層は一緒に動く", () => {
    const line: [Vec2, Vec2] = [[0, 0.5], [1, 0.5]];
    expect(flapFaces(HALF, HALF_FACES, 1, line, [0.25, 0.9])).toEqual([0, 1]);
  });

  it("動かさない側でつながっている層は付いてこない(そこが蝶番になる)", () => {
    const line: [Vec2, Vec2] = [[0.25, 0], [0.25, 1]];
    expect(flapFaces(HALF, HALF_FACES, 1, line, [0.4, 0.5])).toEqual([1]);
  });
});

describe("planGrabFold", () => {
  it("つかんだ紙が離した位置へ倒れる指示になる", () => {
    const r = planGrabFold(SQUARE, SQUARE_FACES, [0.25, 0.5], [0.75, 0.5], "flap");
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    // 動かさない側は離した点。折り線はx=0.5
    expect(r.plan.keepSidePoint).toEqual([0.75, 0.5]);
    expect(r.plan.line[0][0]).toBeCloseTo(0.5);
    expect(r.plan.targetLayers).toEqual([0]);
    expect(r.plan.selectedLayerCount).toBe(1);
    expect(r.plan.preview).toHaveLength(r.plan.selectedLayerCount);
    // プレビューはつかんだ側を鏡映した形(右半分に着地する)
    expect(r.plan.preview).toHaveLength(1);
    for (const p of r.plan.preview[0]) expect(p[0]).toBeGreaterThan(0.49);
  });

  it("Shift(全部)は対象層を指定しない(動く側の全ての層)", () => {
    const r = planGrabFold(HALF, HALF_FACES, [0.1, 0.5], [0.4, 0.5], "all");
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.plan.targetLayers).toBeNull();
    // targetLayers=nullでも、実際に選ばれた2層を数える。これは「ひだ数」ではない。
    expect(r.plan.selectedLayerCount).toBe(2);
    expect(r.plan.preview).toHaveLength(r.plan.selectedLayerCount);
  });

  it("Alt(1枚だけ)はいちばん上の層だけを折る", () => {
    const r = planGrabFold(HALF, HALF_FACES, [0.1, 0.5], [0.4, 0.5], "single");
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.plan.targetLayers).toEqual([1]);
    expect(r.plan.selectedLayerCount).toBe(1);
    expect(r.plan.preview).toHaveLength(r.plan.selectedLayerCount);
  });

  it("つかんだ面を渡すとその層から範囲を広げる", () => {
    const r = planGrabFold(HALF, HALF_FACES, [0.1, 0.5], [0.4, 0.5], "single", 0);
    expect(r.ok && r.plan.targetLayers).toEqual([0]);
  });

  it("短すぎるドラッグは折らずに理由を返す", () => {
    const r = planGrabFold(SQUARE, SQUARE_FACES, [0.5, 0.5], [0.5, 0.505], "flap");
    expect(r.ok).toBe(false);
    if (r.ok) return;
    expect(r.error).toContain("ドラッグ");
  });
});
