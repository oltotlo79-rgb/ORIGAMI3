import { describe, expect, it } from "vitest";
import {
  MIN_TWIST_VERTICES,
  addTwistVertex,
  isTwistPolygonReady,
  polygonCentroid,
  twistPreviewSegments,
  twistReferencePoint,
  undoTwistVertex,
} from "./twistPolygon";
import type { Vec2 } from "./types";

/** 一辺1の正方形(反時計回り) */
const SQUARE: Vec2[] = [
  [0, 0],
  [1, 0],
  [1, 1],
  [0, 1],
];

describe("ねじり折りの中央多角形(頂点を順に置く)", () => {
  it("クリックのたびに頂点が末尾へ足される", () => {
    let poly: Vec2[] = [];
    poly = addTwistVertex(poly, [0, 0]);
    poly = addTwistVertex(poly, [1, 0]);
    expect(poly).toEqual([
      [0, 0],
      [1, 0],
    ]);
  });

  it("同じ場所を2度クリックしても増えない(誤って重なった頂点を作らない)", () => {
    const poly = addTwistVertex([[0, 0]], [0, 1e-9]);
    expect(poly).toEqual([[0, 0]]);
  });

  it("直前の頂点を取り消せる。空でも壊れない", () => {
    expect(undoTwistVertex(SQUARE)).toEqual(SQUARE.slice(0, 3));
    expect(undoTwistVertex([])).toEqual([]);
  });

  it("3点そろうまでは多角形として使えない", () => {
    expect(MIN_TWIST_VERTICES).toBe(3);
    expect(isTwistPolygonReady([])).toBe(false);
    expect(isTwistPolygonReady(SQUARE.slice(0, 2))).toBe(false);
    expect(isTwistPolygonReady(SQUARE.slice(0, 3))).toBe(true);
    expect(isTwistPolygonReady(SQUARE)).toBe(true);
  });
});

describe("中心と回転量", () => {
  it("既定の中心は多角形の重心(面積の重心)", () => {
    expect(polygonCentroid(SQUARE)).toEqual([0.5, 0.5]);
    // 辺の長さが違う三角形でも重心が出る
    const c = polygonCentroid([
      [0, 0],
      [3, 0],
      [0, 3],
    ]);
    expect(c?.[0]).toBeCloseTo(1, 9);
    expect(c?.[1]).toBeCloseTo(1, 9);
  });

  it("面積0(一直線)なら頂点の平均、頂点なしならnull", () => {
    const c = polygonCentroid([
      [0, 0],
      [2, 0],
      [4, 0],
    ]);
    expect(c?.[0]).toBeCloseTo(2, 9);
    expect(polygonCentroid([])).toBeNull();
  });

  it("回転量を示す点は、1辺目の中点を中心のまわりに指定の角だけ回した点", () => {
    const rp = twistReferencePoint(SQUARE, [0.5, 0.5], 90);
    // 1辺目の中点(0.5, 0)を中心(0.5, 0.5)のまわりに90度回すと(1.0, 0.5)
    expect(rp?.[0]).toBeCloseTo(1, 9);
    expect(rp?.[1]).toBeCloseTo(0.5, 9);
    // 向きを逆にすると反対側へ回る
    const back = twistReferencePoint(SQUARE, [0.5, 0.5], -90);
    expect(back?.[0]).toBeCloseTo(0, 9);
    expect(back?.[1]).toBeCloseTo(0.5, 9);
  });

  it("頂点が2つに満たなければ回転量を決められない", () => {
    expect(twistReferencePoint([[0, 0]], [0, 0], 30)).toBeNull();
  });
});

describe("折る前の下見(多角形とひだの折り線)", () => {
  it("3点未満なら置いた点をつなぐ線だけ", () => {
    expect(twistPreviewSegments(SQUARE.slice(0, 2), null)).toEqual([
      [
        [0, 0],
        [1, 0],
      ],
    ]);
  });

  it("多角形の全ての辺と、頂点ごとに2本のひだの折り線が出る", () => {
    const segs = twistPreviewSegments(SQUARE, [0.5, 0.5]);
    // 辺4本 + 頂点4つ×2本
    expect(segs).toHaveLength(4 + 8);
    // 最初の4本は多角形の輪郭(閉じている)
    expect(segs[3]).toEqual([
      [0, 1],
      [0, 0],
    ]);
    // 頂点(0,0)から出る1本目は中心から外へ向かう放射方向
    const arm = segs[4];
    expect(arm[0]).toEqual([0, 0]);
    expect(arm[1][0]).toBeLessThan(0);
    expect(arm[1][1]).toBeLessThan(0);
  });

  it("中心を渡さなければ重心を使う(同じ形になる)", () => {
    expect(twistPreviewSegments(SQUARE, null)).toEqual(
      twistPreviewSegments(SQUARE, [0.5, 0.5]),
    );
  });
});
