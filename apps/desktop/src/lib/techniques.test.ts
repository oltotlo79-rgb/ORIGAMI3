// 技法の日本語表示名と、手順ごとの警告の取り出しのテスト。

import { describe, expect, it } from "vitest";
import {
  SUPPORTED_TECHNIQUES,
  TECHNIQUE_KINDS,
  TECHNIQUE_LABEL,
  uniqueWarnings,
  warningsForStep,
  withFixHint,
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
  it("自動で折れる8種(段・中割り・かぶせ・つぶす・花弁・沈め・ひだ寄せ・ねじり)が並ぶ", () => {
    expect(SUPPORTED_TECHNIQUES.map((t) => t.kind)).toEqual([
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

// 途中に折りを挟んで後続と矛盾したとき、アプリは勝手に直さず警告して続ける
// (設計原則: 止めずに警告)。代わりに「どう直せばよいか」を書き添える。
describe("withFixHint", () => {
  it("矛盾した手順の警告に、直し方を足す", () => {
    const [out] = withFixHint([
      "手順3までの形が展開図から求まりませんでした(いちばん近い形で表示します)",
    ]);
    expect(out).toContain("求まりませんでした");
    expect(out).toContain("削除・移動");
    // 手順番号で始まる形は保つ(タイムラインの手順ごとの絞り込みが効くように)
    expect(warningsForStep([out], 3)).toEqual([out]);
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
