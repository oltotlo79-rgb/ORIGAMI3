import { describe, expect, it } from "vitest";
import { mirrorEdgeOf, withMirrorEdges } from "./mirrorEdit";
import type { Document, Face } from "./types";

/**
 * 左右対称な展開図(縦の中心線が対称軸)。
 * 中心線(辺13)と、そこへ集まる斜めの折り線2本(辺10・辺11)が左右の対になる。
 * 横に折り返しても重ならない形なので、対称軸は縦の1本だけが見つかる。
 * 相手のいない補助線(辺12)も混ぜてある。
 */
function makeDoc(): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [1, 1] },
        { id: 3, pos: [0, 1] },
        { id: 4, pos: [0.5, 0] },
        { id: 5, pos: [0.5, 1] },
        { id: 8, pos: [0.2, 0.4] },
        { id: 9, pos: [0.2, 0.6] },
      ],
      edges: [
        { id: 0, v0: 0, v1: 4, kind: "Border" },
        { id: 1, v0: 4, v1: 1, kind: "Border" },
        { id: 2, v0: 1, v1: 2, kind: "Border" },
        { id: 3, v0: 2, v1: 5, kind: "Border" },
        { id: 4, v0: 5, v1: 3, kind: "Border" },
        { id: 5, v0: 3, v1: 0, kind: "Border" },
        { id: 10, v0: 0, v1: 5, kind: "Mountain" },
        { id: 11, v0: 1, v1: 5, kind: "Mountain" },
        { id: 12, v0: 8, v1: 9, kind: "Aux" },
        { id: 13, v0: 4, v1: 5, kind: "Valley" },
      ],
      next_vertex_id: 10,
      next_edge_id: 14,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

const FACES: Face[] = [
  { id: 0, vertices: [0, 4, 5], edges: [0, 13, 10] },
  { id: 1, vertices: [4, 1, 5], edges: [1, 11, 13] },
  { id: 2, vertices: [0, 5, 3], edges: [10, 4, 5] },
  { id: 3, vertices: [1, 2, 5], edges: [2, 3, 11] },
];

describe("左右対称の相手の線を探す(消す・種類を変えるときに使う)", () => {
  it("展開図から見つけた対称軸で、折り線の相手が見つかる", () => {
    const doc = makeDoc();
    expect(mirrorEdgeOf(doc, FACES, 10)).toBe(11);
    expect(mirrorEdgeOf(doc, FACES, 11)).toBe(10);
  });

  it("輪郭の線にも効く(折り線に限らない)", () => {
    const doc = makeDoc();
    expect(mirrorEdgeOf(doc, FACES, 0)).toBe(1);
  });

  it("対称軸の上に乗る線には相手がいない(二重に消さない)", () => {
    const doc = makeDoc();
    expect(mirrorEdgeOf(doc, FACES, 13)).toBeNull();
  });

  it("反対側に線が無ければ相手はいない", () => {
    const doc = makeDoc();
    expect(mirrorEdgeOf(doc, FACES, 12)).toBeNull();
  });

  it("選んだ線に相手を足す。相手のいない線はその線だけが残る", () => {
    const doc = makeDoc();
    expect(withMirrorEdges(doc, FACES, [10]).sort()).toEqual([10, 11]);
    expect(withMirrorEdges(doc, FACES, [12])).toEqual([12]);
    // すでに対になっている2本を選んでも増えない
    expect(withMirrorEdges(doc, FACES, [10, 11]).sort()).toEqual([10, 11]);
  });
});
