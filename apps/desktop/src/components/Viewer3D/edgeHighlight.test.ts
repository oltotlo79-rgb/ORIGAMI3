import { describe, expect, it } from "vitest";
import type { Document, Edge, Face, Vertex } from "../../lib/types";
import {
  deriveSelectedEdgeHighlights,
  type FacePositionSlot,
} from "./edgeHighlight";

function documentOf(vertices: Vertex[], edges: Edge[]): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices,
      edges,
      next_vertex_id: Math.max(0, ...vertices.map((v) => v.id + 1)),
      next_edge_id: Math.max(0, ...edges.map((e) => e.id + 1)),
    },
    sequence: [],
    display: {
      front_color: [230, 90, 60],
      back_color: [245, 245, 245],
      grid_divisions: 8,
    },
  };
}

const SQUARE_VERTICES: Vertex[] = [
  { id: 0, pos: [0, 0] },
  { id: 1, pos: [1, 0] },
  { id: 2, pos: [1, 1] },
  { id: 3, pos: [0, 1] },
];

describe("deriveSelectedEdgeHighlights", () => {
  it("境界辺は表示中positionsから直接読み、重複選択をDocument順の1件にする", () => {
    const doc = documentOf(SQUARE_VERTICES, [
      { id: 10, v0: 0, v1: 1, kind: "Border" },
      { id: 11, v0: 1, v1: 2, kind: "Border" },
      { id: 12, v0: 2, v1: 3, kind: "Border" },
      { id: 13, v0: 3, v1: 0, kind: "Border" },
      { id: 20, v0: 0, v1: 2, kind: "Mountain" },
    ]);
    const faces: Face[] = [
      { id: 0, vertices: [0, 1, 2], edges: [10, 11, 20] },
      { id: 1, vertices: [0, 2, 3], edges: [20, 12, 13] },
    ];
    const slots = new Map<number, FacePositionSlot>([
      [0, { offset: 0, count: 3 }],
      [1, { offset: 3, count: 3 }],
    ]);
    // 面0と面1で別々に持つ表示座標。共有辺20は従来どおり最初の面0側を使う。
    const positions = new Float32Array([
      0, 0, 0,
      2, 0, 0,
      2, 0, 2,
      0, 1, 0,
      2, 1, 2,
      0, 2, 0,
    ]);

    const result = deriveSelectedEdgeHighlights(
      doc,
      faces,
      slots,
      positions,
      new Set([20]),
      [20, 12, 10, 10, 999, 20],
    );

    // 選択配列の順序ではなく、Document内の辺順で一定になる。
    expect(result.map((target) => target.edgeId)).toEqual([10, 12, 20]);
    expect(result.map((target) => target.role)).toEqual([
      "reference",
      "reference",
      "hinge",
    ]);
    expect(result[0]).toMatchObject({ a: [0, 0, 0], b: [2, 0, 0] });
    expect(result[1]).toMatchObject({ a: [2, 1, 2], b: [0, 2, 0] });
    expect(result[2]).toMatchObject({ a: [2, 0, 2], b: [0, 0, 0] });
  });

  it("山谷でもヒンジ集合に無い辺は操作対象外の色へ分類する", () => {
    const doc = documentOf(SQUARE_VERTICES, [
      { id: 20, v0: 0, v1: 2, kind: "Mountain" },
    ]);
    // 行き止まりの折り線を同じ面が両側から辿る場合を模し、辺20が2回現れる。
    const face: Face = {
      id: 7,
      vertices: [0, 2, 0, 1],
      edges: [20, 20, 10, 11],
    };
    const positions = new Float32Array([
      4, 0, 0,
      4, 1, 0,
      4, 0, 0,
      5, 0, 0,
    ]);

    const result = deriveSelectedEdgeHighlights(
      doc,
      [face],
      new Map([[7, { offset: 0, count: 4 }]]),
      positions,
      new Set(),
      [20],
    );

    expect(result).toEqual([
      {
        edgeId: 20,
        role: "reference",
        a: [4, 0, 0],
        b: [4, 1, 0],
      },
    ]);
  });

  it("Auxの中点を含む面を選び、その面の2D→3D写像で両端を配置する", () => {
    const vertices: Vertex[] = [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [0.5, 0] },
      { id: 2, pos: [1, 0] },
      { id: 3, pos: [1, 1] },
      { id: 4, pos: [0.5, 1] },
      { id: 5, pos: [0, 1] },
      { id: 6, pos: [0.6, 0.5] },
      { id: 7, pos: [0.9, 0.5] },
    ];
    const doc = documentOf(vertices, [
      { id: 30, v0: 1, v1: 4, kind: "Valley" },
      { id: 99, v0: 6, v1: 7, kind: "Aux" },
    ]);
    const faces: Face[] = [
      { id: 0, vertices: [0, 1, 4, 5], edges: [10, 30, 11, 12] },
      { id: 1, vertices: [1, 2, 3, 4], edges: [13, 14, 15, 30] },
    ];
    const slots = new Map<number, FacePositionSlot>([
      [0, { offset: 0, count: 4 }],
      [1, { offset: 4, count: 4 }],
    ]);
    const positions = new Float32Array([
      // 左面はz=0のまま。
      0, 0, 0,
      0.5, 0, 0,
      0.5, 1, 0,
      0, 1, 0,
      // 右面は x=0.5 の壁へ90度起こす: (x,y) -> (0.5, x-0.5, y)。
      0.5, 0, 0,
      0.5, 0.5, 0,
      0.5, 0.5, 1,
      0.5, 0, 1,
    ]);

    const result = deriveSelectedEdgeHighlights(
      doc,
      faces,
      slots,
      positions,
      new Set([30]),
      [99],
    );

    expect(result).toHaveLength(1);
    expect(result[0].role).toBe("reference");
    expect(result[0].a[0]).toBeCloseTo(0.5, 9);
    expect(result[0].a[1]).toBeCloseTo(0.1, 9);
    expect(result[0].a[2]).toBeCloseTo(0.5, 9);
    expect(result[0].b[0]).toBeCloseTo(0.5, 9);
    expect(result[0].b[1]).toBeCloseTo(0.4, 9);
    expect(result[0].b[2]).toBeCloseTo(0.5, 9);
  });

  it("面が無いAuxは展開図の平面位置へフォールバックする", () => {
    const vertices: Vertex[] = [
      { id: 4, pos: [0.2, 0.3] },
      { id: 5, pos: [0.8, 0.7] },
    ];
    const doc = documentOf(vertices, [
      { id: 40, v0: 4, v1: 5, kind: "Aux" },
    ]);

    expect(
      deriveSelectedEdgeHighlights(
        doc,
        [],
        new Map(),
        new Float32Array(),
        new Set(),
        [40],
      ),
    ).toEqual([
      {
        edgeId: 40,
        role: "reference",
        a: [0.2, 0.3, 0],
        b: [0.8, 0.7, 0],
      },
    ]);
  });

  it("対応頂点が欠けた辺と未選択辺は返さず、入力を書き換えない", () => {
    const vertices: Vertex[] = [{ id: 0, pos: [0, 0] }];
    const edges: Edge[] = [
      { id: 1, v0: 0, v1: 99, kind: "Aux" },
      { id: 2, v0: 0, v1: 0, kind: "Aux" },
    ];
    const doc = documentOf(vertices, edges);
    const selected = [1, 1];

    const result = deriveSelectedEdgeHighlights(
      doc,
      [],
      new Map(),
      new Float32Array(),
      new Set(),
      selected,
    );

    expect(result).toEqual([]);
    expect(selected).toEqual([1, 1]);
    expect(doc.cp.edges).toEqual(edges);
  });
});
