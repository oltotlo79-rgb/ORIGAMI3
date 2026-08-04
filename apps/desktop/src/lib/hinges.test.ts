// hingeEdgeIds(角度を指定できる辺)の判定テスト。

import { describe, expect, it } from "vitest";
import { hingeEdgeIds } from "./hinges";
import type { Document, Edge, Face } from "./types";

/** 正方形(頂点0..3)+追加の辺で作る最小のDocument */
function makeDoc(extra: Edge[]): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [1, 1] },
        { id: 3, pos: [0, 1] },
        { id: 4, pos: [0.5, 0.5] },
      ],
      edges: [
        { id: 0, v0: 0, v1: 1, kind: "Border" },
        { id: 1, v0: 1, v1: 2, kind: "Border" },
        { id: 2, v0: 2, v1: 3, kind: "Border" },
        { id: 3, v0: 3, v1: 0, kind: "Border" },
        ...extra,
      ],
      next_vertex_id: 5,
      next_edge_id: 20,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

/** 対角線(辺5)で分けた2つの面 */
const SPLIT_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
  { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
];

describe("hingeEdgeIds", () => {
  it("2つの面に共有される山折り・谷折りの辺だけを折り線とみなす", () => {
    const doc = makeDoc([{ id: 5, v0: 0, v1: 2, kind: "Mountain" }]);
    expect([...hingeEdgeIds(doc, SPLIT_FACES)]).toEqual([5]);

    const valley = makeDoc([{ id: 5, v0: 0, v1: 2, kind: "Valley" }]);
    expect([...hingeEdgeIds(valley, SPLIT_FACES)]).toEqual([5]);
  });

  it("輪郭・補助線は折り線にならない", () => {
    const border = makeDoc([{ id: 5, v0: 0, v1: 2, kind: "Border" }]);
    expect(hingeEdgeIds(border, SPLIT_FACES).size).toBe(0);

    const aux = makeDoc([{ id: 5, v0: 0, v1: 2, kind: "Aux" }]);
    expect(hingeEdgeIds(aux, SPLIT_FACES).size).toBe(0);
  });

  it("行き止まりの折り線(同じ面の境界に2回現れる)は折り線にならない", () => {
    const doc = makeDoc([{ id: 6, v0: 0, v1: 4, kind: "Mountain" }]);
    const faces: Face[] = [
      // 中心へ入って戻る切れ込みを含む1枚の面
      { id: 0, vertices: [0, 1, 2, 3, 0, 4], edges: [0, 1, 2, 3, 6, 6] },
    ];
    expect(hingeEdgeIds(doc, faces).size).toBe(0);
  });

  it("面がまだ無い(展開図だけ)なら折り線は無い", () => {
    const doc = makeDoc([{ id: 5, v0: 0, v1: 2, kind: "Mountain" }]);
    expect(hingeEdgeIds(doc, []).size).toBe(0);
  });
});
