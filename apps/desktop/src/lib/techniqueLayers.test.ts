import { describe, expect, it } from "vitest";
import {
  minimumTechniqueFlap,
  techniqueFlapForPreset,
  techniqueUsesOpenToBack,
  toggleTechniqueFlap,
} from "./techniqueLayers";
import type { TechniqueKind } from "./types";

describe("技法ごとの層指定", () => {
  it("2層必須は中割り・かぶせだけ、つぶし・花弁は1層、ほかは空=全層", () => {
    expect(minimumTechniqueFlap("InsideReverse")).toBe(2);
    expect(minimumTechniqueFlap("OutsideReverse")).toBe(2);
    expect(minimumTechniqueFlap("Squash")).toBe(1);
    expect(minimumTechniqueFlap("Petal")).toBe(1);
    for (const kind of ["Pleat", "OpenSink", "Swivel", "Twist"] satisfies TechniqueKind[]) {
      expect(minimumTechniqueFlap(kind)).toBe(0);
    }
  });

  it("向こう側を選べるのはRust側で参照する4技法だけ", () => {
    for (const kind of ["Squash", "Petal", "Swivel", "Twist"] satisfies TechniqueKind[]) {
      expect(techniqueUsesOpenToBack(kind)).toBe(true);
    }
    for (const kind of [
      "Pleat",
      "InsideReverse",
      "OutsideReverse",
      "OpenSink",
    ] satisfies TechniqueKind[]) {
      expect(techniqueUsesOpenToBack(kind)).toBe(false);
    }
  });
});

describe("奥→手前の候補から部分集合を作る", () => {
  // 悪魔の手順51/55/98/128のように、枚数と奥行きを厳密に指定できることを
  // 最大128層の候補で固定する。
  const candidates = Array.from({ length: 128 }, (_, i) => i);

  it("全部・手前51枚・奥55枚・手前98枚・手前から128枚目を選べる", () => {
    expect(techniqueFlapForPreset(candidates, "all", 1)).toEqual(candidates);
    expect(techniqueFlapForPreset(candidates, "front", 51)).toEqual(
      candidates.slice(77),
    );
    expect(techniqueFlapForPreset(candidates, "back", 55)).toEqual(
      candidates.slice(0, 55),
    );
    expect(techniqueFlapForPreset(candidates, "front", 98)).toEqual(
      candidates.slice(30),
    );
    expect(techniqueFlapForPreset(candidates, "frontNth", 128)).toEqual([0]);
  });

  it("候補外を混ぜず、個別チェック後も奥→手前順を保つ", () => {
    expect(toggleTechniqueFlap([10, 20, 30, 40], [40, 999, 20], 30)).toEqual([
      20,
      30,
      40,
    ]);
    expect(toggleTechniqueFlap([10, 20, 30, 40], [20, 30, 40], 30)).toEqual([
      20,
      40,
    ]);
  });
});
