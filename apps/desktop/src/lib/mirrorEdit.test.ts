import { describe, expect, it } from "vitest";
import { mirrorEdgeOf, withMirrorEdges } from "./mirrorEdit";
import { paperMirrorLine, type MirrorLine } from "./mirror";
import { DEFAULT_DISPLAY } from "./displayPrefs";
import type { Document, EdgeKind, Vec2 } from "./types";

function makeDoc(): Document {
  const vertices: Document["cp"]["vertices"] = [];
  const edges: Document["cp"]["edges"] = [];
  let nextVertex = 0;
  const add = (id: number, a: Vec2, b: Vec2, kind: EdgeKind = "Aux") => {
    const v0 = nextVertex++;
    const v1 = nextVertex++;
    vertices.push({ id: v0, pos: a }, { id: v1, pos: b });
    edges.push({ id, v0, v1, kind });
  };
  // 縦中心x=.5で対になる2本と、軸上の1本。
  add(10, [0.1, 0.2], [0.3, 0.4], "Mountain");
  add(11, [0.9, 0.2], [0.7, 0.4], "Mountain");
  add(12, [0.5, 0.1], [0.5, 0.9], "Valley");
  // 横中心y=.5で対になる2本。
  add(20, [0.15, 0.2], [0.35, 0.2]);
  add(21, [0.15, 0.8], [0.35, 0.8]);
  // 選んだ斜線y=xで対になる2本と、基準線そのもの。
  add(30, [0.1, 0.3], [0.2, 0.4]);
  add(31, [0.3, 0.1], [0.4, 0.2]);
  add(32, [0, 0], [1, 1]);
  // どの例でも相手にしない孤立線。
  add(40, [0.05, 0.65], [0.08, 0.72]);
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: { vertices, edges, next_vertex_id: nextVertex, next_edge_id: 41 },
    sequence: [],
    display: DEFAULT_DISPLAY,
  };
}

const DIAGONAL: MirrorLine = { p: [0, 0], d: [1, 1] };

describe("指定した基準線で、消す・線種変更の相手を探す", () => {
  it("紙の縦の中心線で左右の相手が見つかる", () => {
    const doc = makeDoc();
    const axis = paperMirrorLine(doc.paper, "paperVertical");
    expect(mirrorEdgeOf(doc, 10, axis)).toBe(11);
    expect(mirrorEdgeOf(doc, 11, axis)).toBe(10);
    expect(withMirrorEdges(doc, [10], axis).sort()).toEqual([10, 11]);
  });

  it("紙の横の中心線で上下の相手が見つかる", () => {
    const doc = makeDoc();
    const axis = paperMirrorLine(doc.paper, "paperHorizontal");
    expect(mirrorEdgeOf(doc, 20, axis)).toBe(21);
    expect(withMirrorEdges(doc, [21], axis).sort()).toEqual([20, 21]);
  });

  it("選んだ斜め線で対角位置の相手が見つかる", () => {
    const doc = makeDoc();
    expect(mirrorEdgeOf(doc, 30, DIAGONAL)).toBe(31);
    expect(withMirrorEdges(doc, [30], DIAGONAL).sort()).toEqual([30, 31]);
  });

  it("基準線上の線には相手を足さず、孤立線も選んだ1本だけにする", () => {
    const doc = makeDoc();
    expect(mirrorEdgeOf(doc, 12, paperMirrorLine(doc.paper, "paperVertical"))).toBeNull();
    expect(mirrorEdgeOf(doc, 32, DIAGONAL)).toBeNull();
    expect(withMirrorEdges(doc, [40], DIAGONAL)).toEqual([40]);
    expect(withMirrorEdges(doc, [30, 31], DIAGONAL).sort()).toEqual([30, 31]);
  });
});
