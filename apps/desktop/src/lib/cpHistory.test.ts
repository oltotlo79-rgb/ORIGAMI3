import { describe, expect, it } from "vitest";
import type { Document, FoldStep, TechniqueKind, Vec2 } from "./types";
import { documentForCpStep } from "./cpHistory";

function step(
  id: number,
  line: [Vec2, Vec2],
  targetAngleDeg: number,
  kind: TechniqueKind = "Simple",
): FoldStep {
  return {
    id,
    kind,
    drivers: [{ a: line[0], b: line[1], target_angle_deg: targetAngleDeg }],
    layer_order: null,
    note: "",
  };
}

function documentWithHistory(): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 100, height_mm: 100 },
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [1, 1] },
        { id: 3, pos: [0, 1] },
        { id: 4, pos: [0.25, 0] },
        { id: 5, pos: [0.25, 1] },
        { id: 6, pos: [0.75, 0] },
        { id: 7, pos: [0.75, 1] },
        { id: 8, pos: [0, 0.4] },
        { id: 9, pos: [1, 0.4] },
      ],
      edges: [
        { id: 10, v0: 0, v1: 1, kind: "Border" },
        { id: 11, v0: 0, v1: 2, kind: "Aux" },
        { id: 12, v0: 8, v1: 9, kind: "Valley" },
        { id: 13, v0: 4, v1: 5, kind: "Mountain" },
        { id: 14, v0: 6, v1: 7, kind: "Valley" },
      ],
      next_vertex_id: 10,
      next_edge_id: 15,
    },
    sequence: [
      step(1, [[0.25, 0], [0.25, 1]], 180),
      step(2, [[0.75, 0], [0.75, 1]], -180),
    ],
    display: {
      front_color: [255, 255, 255],
      back_color: [220, 220, 220],
      grid_divisions: 8,
    },
  };
}

function edgeIds(doc: Document): number[] {
  return doc.cp.edges.map((edge) => edge.id);
}

describe("documentForCpStep", () => {
  it("折る前にもBorder・Aux・どのdriverにも属さない事前折り線を表示する", () => {
    const doc = documentWithHistory();

    expect(edgeIds(documentForCpStep(doc, 0))).toEqual([10, 11, 12]);
  });

  it("手順1と手順2で追加された折り線を順に表示する", () => {
    const doc = documentWithHistory();

    expect(edgeIds(documentForCpStep(doc, 1))).toEqual([10, 11, 12, 13]);
    expect(documentForCpStep(doc, 2)).toBe(doc);
    expect(edgeIds(documentForCpStep(doc, 2))).toEqual([10, 11, 12, 13, 14]);
  });

  it("後から分割されたdriver線分上の断片も同じ追加手順に含める", () => {
    const doc = documentWithHistory();
    doc.cp.vertices.push(
      { id: 20, pos: [0.3, 0.6] },
      { id: 21, pos: [0.7, 0.6] },
    );
    doc.cp.edges.push({ id: 20, v0: 20, v1: 21, kind: "Mountain" });
    doc.sequence[0] = step(1, [[0, 0.6], [1, 0.6]], 135);

    expect(edgeIds(documentForCpStep(doc, 0))).not.toContain(20);
    expect(edgeIds(documentForCpStep(doc, 1))).toContain(20);
  });

  it("Poseと0度driverは折り線の追加時点として扱わない", () => {
    const doc = documentWithHistory();
    doc.sequence = [
      step(1, [[0.25, 0], [0.25, 1]], 180, "Pose"),
      step(2, [[0.75, 0], [0.75, 1]], 0),
    ];

    expect(edgeIds(documentForCpStep(doc, 0))).toEqual([10, 11, 12, 13, 14]);
  });

  it("0→1→2→0と往復しても元Documentを変更しない", () => {
    const doc = documentWithHistory();
    const originalEdges = doc.cp.edges.slice();

    const atZero = documentForCpStep(doc, 0);
    const atOne = documentForCpStep(doc, 1);
    const atTwo = documentForCpStep(doc, 2);
    const atZeroAgain = documentForCpStep(doc, 0);

    expect(edgeIds(atZero)).toEqual([10, 11, 12]);
    expect(edgeIds(atOne)).toEqual([10, 11, 12, 13]);
    expect(atTwo).toBe(doc);
    expect(atZeroAgain).toBe(atZero);
    expect(doc.cp.edges).toEqual(originalEdges);
    expect(doc.cp.edges).toBeDefined();
  });

  it("同じDocumentと正規化後の手順ではキャッシュした参照を返す", () => {
    const doc = documentWithHistory();
    const another = documentWithHistory();

    expect(documentForCpStep(doc, 1)).toBe(documentForCpStep(doc, 1));
    expect(documentForCpStep(doc, -1)).toBe(documentForCpStep(doc, 0));
    expect(documentForCpStep(another, 1)).not.toBe(documentForCpStep(doc, 1));
  });

  it("nullと最終以降は元Documentを返し、負の手順は0へ丸める", () => {
    const doc = documentWithHistory();

    expect(documentForCpStep(doc, null)).toBe(doc);
    expect(documentForCpStep(doc, 2)).toBe(doc);
    expect(documentForCpStep(doc, 99)).toBe(doc);
    expect(documentForCpStep(doc, -99)).toBe(documentForCpStep(doc, 0));
  });

  it("壊れた頂点参照と退化driverに一致しうる辺は隠さない", () => {
    const doc = documentWithHistory();
    doc.cp.edges.push({ id: 30, v0: 999, v1: 0, kind: "Mountain" });
    doc.sequence = [step(1, [[0, 0], [0, 0]], 180)];

    expect(edgeIds(documentForCpStep(doc, 0))).toContain(30);
    expect(edgeIds(documentForCpStep(doc, 0))).toContain(13);
  });
});
