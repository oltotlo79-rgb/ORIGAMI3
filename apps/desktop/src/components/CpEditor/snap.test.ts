// snap.tsのユニットテスト。Rust側 Document::new と同じ形の初期ドキュメントを
// テスト用に手組みして検証する(頂点優先・グリッド・線上・範囲外null)。

import { describe, expect, it } from "vitest";
import type { Document, Edge, Vertex } from "../../lib/types";
import { paperExtent, snap } from "./snap";

/** 150×150mm相当の正方形ドキュメント(輪郭4辺、グリッド8分割) */
function makeDoc(overrides?: {
  widthMm?: number;
  heightMm?: number;
  gridDivisions?: number;
  extraVertices?: Vertex[];
  extraEdges?: Edge[];
}): Document {
  const widthMm = overrides?.widthMm ?? 150;
  const heightMm = overrides?.heightMm ?? 150;
  const long = Math.max(widthMm, heightMm);
  const w = widthMm / long;
  const h = heightMm / long;
  const vertices: Vertex[] = [
    { id: 0, pos: [0, 0] },
    { id: 1, pos: [w, 0] },
    { id: 2, pos: [w, h] },
    { id: 3, pos: [0, h] },
    ...(overrides?.extraVertices ?? []),
  ];
  const edges: Edge[] = [
    { id: 0, v0: 0, v1: 1, kind: "Border" },
    { id: 1, v0: 1, v1: 2, kind: "Border" },
    { id: 2, v0: 2, v1: 3, kind: "Border" },
    { id: 3, v0: 3, v1: 0, kind: "Border" },
    ...(overrides?.extraEdges ?? []),
  ];
  return {
    schema_version: 1,
    paper: { width_mm: widthMm, height_mm: heightMm },
    cp: {
      vertices,
      edges,
      next_vertex_id: vertices.length,
      next_edge_id: edges.length,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: overrides?.gridDivisions ?? 8,
    },
  };
}

/** 正方形+対角線(0,0)-(1,1)の山折り線 */
function docWithDiagonal(): Document {
  return makeDoc({ extraEdges: [{ id: 4, v0: 0, v1: 2, kind: "Mountain" }] });
}

describe("snap", () => {
  it("半径内の既存頂点に吸着する", () => {
    const r = snap(makeDoc(), [0.01, 0.008], 0.05);
    expect(r).toEqual({ pos: [0, 0], kind: "vertex" });
  });

  it("グリッド交点より頂点を優先する", () => {
    // 頂点(0.26,0.25)とグリッド交点(0.25,0.25)がカーソルから等距離でも頂点が勝つ
    const doc = makeDoc({ extraVertices: [{ id: 4, pos: [0.26, 0.25] }] });
    const r = snap(doc, [0.255, 0.25], 0.02);
    expect(r).toEqual({ pos: [0.26, 0.25], kind: "vertex" });
  });

  it("頂点がなければグリッド交点に吸着する(8分割)", () => {
    const r = snap(makeDoc(), [0.372, 0.253], 0.05);
    expect(r?.kind).toBe("grid");
    expect(r?.pos[0]).toBeCloseTo(0.375, 12);
    expect(r?.pos[1]).toBeCloseTo(0.25, 12);
  });

  it("グリッドはgrid_divisionsと紙の縦横比を反映する", () => {
    // 150×100mm → 正規化サイズ1.0×(2/3)、4分割 → y方向の刻みは1/6
    const doc = makeDoc({ heightMm: 100, gridDivisions: 4 });
    expect(paperExtent(doc)).toEqual([1, 100 / 150]);
    const r = snap(doc, [0.5, 0.17], 0.02);
    expect(r?.kind).toBe("grid");
    expect(r?.pos[0]).toBeCloseTo(0.5, 12);
    expect(r?.pos[1]).toBeCloseTo(1 / 6, 12);
  });

  it("線分上の点より近いグリッド交点を優先する", () => {
    // 下辺(y=0)上のグリッド交点(0.5,0): 線上吸着ではなくgridとして返る
    const r = snap(makeDoc(), [0.5, 0.005], 0.02);
    expect(r?.kind).toBe("grid");
    expect(r?.pos[0]).toBeCloseTo(0.5, 12);
    expect(r?.pos[1]).toBeCloseTo(0, 12);
  });

  it("頂点・グリッドがなければ線分上に吸着する(最近傍点へ射影)", () => {
    // 対角線上の点(0.3,0.3)はグリッド交点ではない
    const r = snap(docWithDiagonal(), [0.31, 0.29], 0.02);
    expect(r?.kind).toBe("edge");
    expect(r?.pos[0]).toBeCloseTo(0.3, 12);
    expect(r?.pos[1]).toBeCloseTo(0.3, 12);
  });

  it("半径内に候補がなければnull", () => {
    expect(snap(docWithDiagonal(), [0.4, 0.15], 0.02)).toBeNull();
  });

  it("参照切れの線があっても落ちずに吸着できる", () => {
    const doc = makeDoc({
      extraEdges: [{ id: 4, v0: 0, v1: 999, kind: "Mountain" }],
    });
    expect(snap(doc, [0.4, 0.15], 0.02)).toBeNull();
    expect(snap(doc, [0.01, 0.01], 0.05)?.kind).toBe("vertex");
  });
});
