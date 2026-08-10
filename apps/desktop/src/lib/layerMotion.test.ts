import { describe, expect, it } from "vitest";
import {
  buildLayerMotionPart,
  describeLayerMotionPart,
  hasLayerMotionInput,
  type LayerMotionPartDraft,
} from "./layerMotion";

function draft(patch: Partial<LayerMotionPartDraft> = {}): LayerMotionPartDraft {
  return {
    layers: [4, 2, 4],
    line: null,
    mode: "stay",
    turn: "Keep",
    direction: "Up",
    anchor: 0,
    reverseLayers: false,
    ...patch,
  };
}

describe("汎用層操作の入力", () => {
  it("既存折り目のReflectをregionなし・Keepへ変換する", () => {
    const result = buildLayerMotionPart(
      draft({
        mode: "reflect",
        line: [[0.5, 0], [0.5, 1]],
        reverseLayers: true,
      }),
    );
    expect(result).toEqual({
      ok: true,
      part: {
        layers: [4, 2],
        region: [],
        transform: { Reflect: [[[0.5, 0], [0.5, 1]]] },
        turn: "Keep",
        reverse_layers: true,
      },
    });
  });

  it("Stayの4種類の重ね方をRustの外部タグ形へ変換する", () => {
    expect(buildLayerMotionPart(draft({ turn: "Outside", direction: "Down" }))).toMatchObject({
      ok: true,
      part: { transform: "Stay", turn: { Outside: "Down" } },
    });
    expect(buildLayerMotionPart(draft({ turn: "Inside" }))).toMatchObject({
      ok: true,
      part: { turn: { Inside: "Up" } },
    });
    expect(buildLayerMotionPart(draft({ turn: "Beside", anchor: 9 }))).toMatchObject({
      ok: true,
      part: { turn: { Beside: { anchor: 9, direction: "Up" } } },
    });
    expect(buildLayerMotionPart(draft({ reverseLayers: true }))).toMatchObject({
      ok: true,
      part: { turn: "Keep", reverse_layers: true },
    });
  });

  it("軸なしReflectと無変更Stayを弾く", () => {
    expect(buildLayerMotionPart(draft({ mode: "reflect" }))).toMatchObject({ ok: false });
    expect(buildLayerMotionPart(draft())).toMatchObject({ ok: false });
    expect(hasLayerMotionInput(draft({ layers: [], mode: "reflect" }))).toBe(false);
    expect(hasLayerMotionInput(draft({ layers: [], turn: "Outside" }))).toBe(true);
  });

  it("追加済みpartを日本語で説明する", () => {
    const result = buildLayerMotionPart(draft({ turn: "Beside", anchor: 7 }));
    expect(result.ok && describeLayerMotionPart(result.part)).toContain("面7の手前隣");
  });
});
