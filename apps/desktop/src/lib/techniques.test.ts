// 技法の日本語表示名と、手順ごとの警告の取り出しのテスト。

import { describe, expect, it } from "vitest";
import {
  DISPLAY_TECHNIQUE_LABEL,
  SUPPORTED_TECHNIQUES,
  stepDisplayLabel,
  TECHNIQUE_KINDS,
  TECHNIQUE_LABEL,
  uniqueWarnings,
  warningsForStep,
  withFixHint,
} from "./techniques";
import type { DisplayTechniqueKind, FoldStep, TechniqueKind } from "./types";

describe("技法の表示名", () => {
  it("10種類すべてに日本語名がある", () => {
    const kinds: TechniqueKind[] = [
      "Simple",
      "Pleat",
      "InsideReverse",
      "OutsideReverse",
      "Petal",
      "Squash",
      "OpenSink",
      "Swivel",
      "Twist",
      "Pose",
    ];
    expect(TECHNIQUE_KINDS).toEqual(kinds);
    for (const k of kinds) {
      expect(TECHNIQUE_LABEL[k]).toMatch(/^[^A-Za-z]+$/); // 英字が残っていない
    }
  });
});

describe("サブメニューに出す技法", () => {
  it("汎用層操作と、自動で折れる8種の名前付き技法が並ぶ", () => {
    expect(SUPPORTED_TECHNIQUES.map((t) => t.kind)).toEqual([
      "Simple",
      "Pleat",
      "InsideReverse",
      "OutsideReverse",
      "Squash",
      "Petal",
      "OpenSink",
      "Swivel",
      "Twist",
    ]);
    for (const t of SUPPORTED_TECHNIQUES) {
      expect(t.short).toMatch(/^[^A-Za-z]+$/);
      expect(t.title.length).toBeGreaterThan(10);
    }
  });
});

// 手順に記録された技法名(technique_classification)の表示。正本(scratchpad/
// self-intersection-report.md §6)どおり、無ければkindのTECHNIQUE_LABELへ戻す。
describe("stepDisplayLabel", () => {
  function baseStep(overrides: Partial<FoldStep> = {}): FoldStep {
    return {
      id: 1,
      kind: "Simple",
      drivers: [],
      layer_order: null,
      note: "",
      ...overrides,
    };
  }

  const DISPLAY_KINDS: DisplayTechniqueKind[] = [
    "LayerOperation",
    "Pleat",
    "InsideReverse",
    "OutsideReverse",
    "Squash",
    "Petal",
    "OpenSink",
    "Swivel",
    "Twist",
    "GrabMove",
  ];

  it("10種のtechnique_classification.kindそれぞれの表示名を返す", () => {
    expect(DISPLAY_KINDS.length).toBe(10);
    for (const kind of DISPLAY_KINDS) {
      const step = baseStep({
        technique_classification: { kind, origin: "Automatic" },
      });
      expect(stepDisplayLabel(step)).toBe(DISPLAY_TECHNIQUE_LABEL[kind]);
    }
  });

  it("LayerOperation・GrabMove以外の8つはTECHNIQUE_LABELと同じ文字列を使う(同じ折り方が場所で違う名前にならない)", () => {
    const shared = [
      "Pleat",
      "InsideReverse",
      "OutsideReverse",
      "Squash",
      "Petal",
      "OpenSink",
      "Swivel",
      "Twist",
    ] as const;
    for (const kind of shared) {
      expect(DISPLAY_TECHNIQUE_LABEL[kind]).toBe(TECHNIQUE_LABEL[kind]);
    }
  });

  it("項目が無い手順ではkindのTECHNIQUE_LABELへ戻す(旧作品・Pose・分類対象外)", () => {
    const step = baseStep({ kind: "Pleat" });
    expect("technique_classification" in step).toBe(false);
    expect(stepDisplayLabel(step)).toBe(TECHNIQUE_LABEL.Pleat);

    const pose = baseStep({ kind: "Pose" });
    expect(stepDisplayLabel(pose)).toBe(TECHNIQUE_LABEL.Pose);
  });

  it("technique_classificationがnullでもkindのTECHNIQUE_LABELへ戻す(推測で名前を付けない)", () => {
    const step = baseStep({ kind: "Squash", technique_classification: null });
    expect(stepDisplayLabel(step)).toBe(TECHNIQUE_LABEL.Squash);
  });
});

describe("warningsForStep", () => {
  const warnings = [
    "手順1の折り線が見つからないため、この手順を飛ばしました",
    "手順10の折り線の一部が見つかりません",
    "手順2までの形が展開図から求まりませんでした",
  ];

  it("手順1の警告に手順10のものを含めない", () => {
    expect(warningsForStep(warnings, 1)).toEqual([warnings[0]]);
  });

  it("手順10の警告を取り出せる", () => {
    expect(warningsForStep(warnings, 10)).toEqual([warnings[1]]);
  });

  it("該当が無ければ空になる", () => {
    expect(warningsForStep(warnings, 3)).toEqual([]);
  });
});

describe("uniqueWarnings", () => {
  it("出どころが違っても同じ文言は1回だけ残す", () => {
    const fromCp = ["手順3を飛ばしました", "頂点が孤立しています"];
    const fromReplay = ["手順3を飛ばしました"];
    expect(uniqueWarnings(fromCp, [], fromReplay)).toEqual([
      "手順3を飛ばしました",
      "頂点が孤立しています",
    ]);
  });
});

// 一般的な不収束では前の希望角を自動調整し、警告して操作を続ける。
// 折り線自体が見つからない場合だけ、原因に合った引き直し・削除を案内する。
describe("withFixHint", () => {
  it("一般的な不収束には、自動調整しながら操作を続けられると伝える", () => {
    const [out] = withFixHint([
      "手順3までの形が展開図から求まりませんでした(いちばん近い形で表示します)",
    ]);
    expect(out).toContain("求まりませんでした");
    expect(out).toContain("前の角度を自動調整しています。操作は続けられます");
    expect(out).not.toContain("削除・移動");
    // 手順番号で始まる形は保つ(タイムラインの手順ごとの絞り込みが効くように)
    expect(warningsForStep([out], 3)).toEqual([out]);
  });

  it("計算側の不収束という語を、画面では利用者の状態へ言い換える", () => {
    const [out] = withFixHint(["追従計算が収束していません"]);
    expect(out).toBe(
      "指定した角度に近い形を表示しています。前の角度を自動調整しています。操作は続けられます",
    );
    expect(out).not.toMatch(/追従計算|収束/u);
  });

  it("折り線が見つからない手順には、引き直すか削除するよう伝える", () => {
    const [out] = withFixHint([
      "手順2の折り線が見つからないため、この手順を飛ばしました",
    ]);
    expect(out).toContain("引き直す");
    expect(out).toContain("削除");
  });

  it("重なり順の警告と、心当たりの無い警告", () => {
    expect(withFixHint(["層順序の代表点 (0.1, 0.2) …この層を飛ばしました"])[0]).toContain(
      "折り直す",
    );
    expect(withFixHint(["よく分からない警告"])).toEqual(["よく分からない警告"]);
  });

  it("同じ案内を二重に足さない", () => {
    const once = withFixHint(["手順2の折り線が見つからないため飛ばしました"]);
    expect(withFixHint(once)).toEqual(once);
  });
});
