// 平らに畳めない点の説明文のテスト。

import { describe, expect, it } from "vitest";
import { REASON_ANGLES, REASON_COUNTS, violationReason } from "./flatFoldHint";
import type { Document, EdgeKind, Vec2 } from "./types";

/** 中心(id=0)から放射状に折り目を出した展開図を作る(角度は度) */
function radial(spokes: [number, EdgeKind][]): Document {
  const vertices = [{ id: 0, pos: [0.5, 0.5] as Vec2 }];
  const edges = spokes.map(([deg, kind], i) => {
    const rad = (deg * Math.PI) / 180;
    vertices.push({
      id: i + 1,
      pos: [0.5 + 0.4 * Math.cos(rad), 0.5 + 0.4 * Math.sin(rad)] as Vec2,
    });
    return { id: i, v0: 0, v1: i + 1, kind };
  });
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: { vertices, edges, next_vertex_id: vertices.length, next_edge_id: edges.length },
    sequence: [],
    display: { front_color: [237, 28, 36], back_color: [255, 255, 255], grid_divisions: 8 },
  };
}

describe("平らに畳めない点の説明", () => {
  it("山と谷の本数が合わないときはその理由を出す", () => {
    const doc = radial([
      [0, "Mountain"],
      [90, "Mountain"],
      [180, "Mountain"],
      [270, "Mountain"],
    ]);
    const text = violationReason(doc, 0);
    expect(text).toContain(REASON_COUNTS);
    expect(text).not.toContain(REASON_ANGLES);
  });

  it("角の和が合わないときはその理由を出す", () => {
    const doc = radial([
      [0, "Mountain"],
      [90, "Mountain"],
      [180, "Mountain"],
      [240, "Valley"],
    ]);
    const text = violationReason(doc, 0);
    expect(text).toContain(REASON_ANGLES);
    expect(text).not.toContain(REASON_COUNTS);
  });

  it("補助線は数えない(本数の理由が出る)", () => {
    const doc = radial([
      [0, "Mountain"],
      [90, "Mountain"],
      [180, "Mountain"],
      [270, "Valley"],
      [45, "Aux"],
    ]);
    // 折り目だけを見れば畳める形なので、説明は断定しない言い方になる
    expect(violationReason(doc, 0)).toBe("平らに畳めないかもしれません");
  });

  it("英字の専門用語を画面に出さない", () => {
    const doc = radial([
      [0, "Mountain"],
      [120, "Mountain"],
      [240, "Valley"],
    ]);
    expect(violationReason(doc, 0)).not.toMatch(/[A-Za-z]/);
  });
});
