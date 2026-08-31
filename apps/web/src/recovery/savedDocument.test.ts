import { describe, expect, it } from "vitest";
import type { Document } from "../../../desktop/src/lib/types";
import {
  parseSavedDocument,
  projectSavedDocument,
  serializeSavedDocument,
  type SavedDocumentSource,
} from "./savedDocument";

function source(): SavedDocumentSource {
  const doc: Document & SavedDocumentSource["doc"] = {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
      ],
      edges: [{ id: 0, v0: 0, v1: 1, kind: "Mountain" }],
      next_vertex_id: 2,
      next_edge_id: 1,
    },
    sequence: [
      {
        id: 4,
        kind: "Pose",
        drivers: [
          { a: [0, 0], b: [1, 0], target_angle_deg: 90 },
        ],
        layer_order: null,
        alignment: {
          mode: "throughTwoPoints",
          picks: [
            { kind: "point", p: [0, 0] },
            { kind: "line", a: [0, 0], b: [1, 1] },
          ],
        },
        finish_soft: { enabled: true, stiffness: 0.6, pressure: 0.2 },
        note: "仕上げ",
      },
    ],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
      overlap_prevention_enabled: true,
      penetration_prevention_enabled: false,
      soft_enabled: true,
      soft_stiffness: 0.6,
      soft_pressure: 0.2,
    },
  };
  return {
    doc,
    step_creases: [{ step: 4, lines: [[[0, 0], [1, 0]]] }],
  };
}

describe("SavedDocumentのWeb永続化境界", () => {
  it("Rust SavedDocument相当の確定済みデータだけを明示投影する", () => {
    const input = source();
    const polluted = {
      ...input.doc,
      frame: { vertices: [[0, 0, 1]] },
      warm_start: [{ hinge: 1, target_angle_deg: 45 }],
      fold_all_preview: { percent: 50 },
      display: { ...input.doc.display, temporary_pose: [1, 2, 3] },
      sequence: [
        {
          ...input.doc.sequence[0],
          angles: { 0: 90 },
          next_warm_seed: [{ hinge: 0, target_angle_deg: 90 }],
        },
      ],
    } as unknown as SavedDocumentSource["doc"];

    const payload = serializeSavedDocument({
      doc: polluted,
      step_creases: input.step_creases,
    });

    expect(payload).not.toContain("frame");
    expect(payload).not.toContain("warm_start");
    expect(payload).not.toContain("next_warm_seed");
    expect(payload).not.toContain("fold_all_preview");
    expect(payload).not.toContain("temporary_pose");
    expect(payload).not.toContain("angles");
    expect(JSON.parse(payload)).toEqual(projectSavedDocument(input));
  });

  it("読み出し時にも余分な実行時状態を落とし、復元だけでは入力を書き換えない", () => {
    const payload = JSON.stringify({
      ...projectSavedDocument(source()),
      frame: { forbidden: true },
      warm_seed: [1, 2, 3],
    });

    const restored = parseSavedDocument(payload);

    expect(restored).toEqual(projectSavedDocument(source()));
    expect(restored).not.toHaveProperty("frame");
    expect(restored).not.toHaveProperty("warm_seed");
  });

  it("壊れたJSONと必須項目不足を日本語エラーにする", () => {
    expect(() => parseSavedDocument("{"))
      .toThrow("復旧データのJSONが壊れているため、作品を復元できません。");
    expect(() => parseSavedDocument(JSON.stringify({ schema_version: 1 })))
      .toThrow("復旧データに作品として必要な項目がありません。");
  });
});
