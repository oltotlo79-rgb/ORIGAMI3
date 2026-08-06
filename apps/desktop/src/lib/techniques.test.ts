// 技法の日本語表示名と、手順ごとの警告の取り出しのテスト。

import { describe, expect, it } from "vitest";
import {
  SUPPORTED_TECHNIQUES,
  TECHNIQUE_KINDS,
  TECHNIQUE_LABEL,
  uniqueWarnings,
  warningsForStep,
} from "./techniques";
import type { TechniqueKind } from "./types";

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
  it("自動で折れる6種(段・中割り・かぶせ・開いてつぶす・花弁・沈め)が並ぶ", () => {
    expect(SUPPORTED_TECHNIQUES.map((t) => t.kind)).toEqual([
      "Pleat",
      "InsideReverse",
      "OutsideReverse",
      "Squash",
      "Petal",
      "OpenSink",
    ]);
    for (const t of SUPPORTED_TECHNIQUES) {
      expect(t.short).toMatch(/^[^A-Za-z]+$/);
      expect(t.title.length).toBeGreaterThan(10);
    }
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
