// 見た目の好み(紙の色・方眼の分割数・分割比)の丸めと保管のテスト。

import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_DISPLAY,
  DEFAULT_PREFS,
  clampDivisions,
  clampSplitRatio,
  hexToRgb,
  loadPrefs,
  overlapPreventionOf,
  rgbToHex,
  savePrefs,
  type StorageLike,
} from "./displayPrefs";

/** localStorageの代わり(テスト用の覚え書き) */
let store: Record<string, string>;
const storage: StorageLike = {
  getItem: (k) => store[k] ?? null,
  setItem: (k, v) => {
    store[k] = v;
  },
};

beforeEach(() => {
  store = {};
});

describe("見た目の好み", () => {
  it("方眼の分割数は2〜64の整数に丸める", () => {
    expect(clampDivisions(1)).toBe(2);
    expect(clampDivisions(100)).toBe(64);
    expect(clampDivisions(8.4)).toBe(8);
    expect(clampDivisions(Number.NaN)).toBe(DEFAULT_DISPLAY.grid_divisions);
  });

  it("分割比は2割〜8割に収める", () => {
    expect(clampSplitRatio(0.05)).toBeCloseTo(0.2);
    expect(clampSplitRatio(0.95)).toBeCloseTo(0.8);
    expect(clampSplitRatio(0.35)).toBeCloseTo(0.35);
    expect(clampSplitRatio(Number.NaN)).toBeCloseTo(0.5);
  });

  it("色は色見本の形と行き来できる", () => {
    expect(rgbToHex([237, 28, 36])).toBe("#ed1c24");
    expect(hexToRgb("#ED1C24")).toEqual([237, 28, 36]);
    expect(hexToRgb("あか")).toBeNull();
  });

  it("重なり防止は既定でオンで、項目の無い古い作品もオンとして扱う", () => {
    expect(overlapPreventionOf(DEFAULT_DISPLAY)).toBe(true);
    const oldDisplay = { ...DEFAULT_DISPLAY };
    delete oldDisplay.overlap_prevention_enabled;
    expect(overlapPreventionOf(oldDisplay)).toBe(true);
    expect(
      overlapPreventionOf({ ...DEFAULT_DISPLAY, overlap_prevention_enabled: false }),
    ).toBe(false);
  });

  it("保存した好みは次に読むとそのまま戻る", () => {
    savePrefs(
      {
        splitRatio: 0.3,
        mirrorDraw: true,
        pullMirror: false,
        wheelBehavior: "zoom",
      },
      storage,
    );
    const loaded = loadPrefs(storage);
    expect(loaded.splitRatio).toBeCloseTo(0.3);
    // 左右対称に描く指定も端末に覚えておく(CPE-010)
    expect(loaded.mirrorDraw).toBe(true);
    // 3Dで引くときの左右同時の指定も覚えておく(UI-007)
    expect(loaded.pullMirror).toBe(false);
    // 2D展開図のホイール動作も端末ごとに戻る
    expect(loaded.wheelBehavior).toBe("zoom");
  });

  it("ホイールは既定でスクロールし、古い保存や不正値もスクロールに戻す", () => {
    expect(DEFAULT_PREFS.wheelBehavior).toBe("scroll");
    expect(loadPrefs(storage).wheelBehavior).toBe("scroll");
    storage.setItem(
      "origami3.prefs",
      JSON.stringify({ splitRatio: 0.4, wheelBehavior: "unknown" }),
    );
    expect(loadPrefs(storage).wheelBehavior).toBe("scroll");
  });

  it("3Dで引くときの左右同時は、保存が無ければ既定のオン(UI-007)", () => {
    expect(DEFAULT_PREFS.pullMirror).toBe(true);
    expect(loadPrefs(storage).pullMirror).toBe(true);
    // 古い版が書いた好み(pullMirrorが無い)を読んでもオンのまま
    storage.setItem("origami3.prefs", JSON.stringify({ splitRatio: 0.4 }));
    expect(loadPrefs(storage).pullMirror).toBe(true);
  });

  it("紙の色・方眼は端末に覚えない(作品ファイル側の設定なので)", () => {
    // 古い版が書いた好み(displayを含む)を読んでも、色は引き継がない。
    // 引き継ぐと、人からもらった作品を開いたときにその色を黙って上書きしてしまう
    store["origami3.prefs"] = JSON.stringify({
      display: { ...DEFAULT_DISPLAY, front_color: [0, 128, 255] },
      splitRatio: 0.3,
    });
    expect(loadPrefs(storage)).toEqual({
      splitRatio: 0.3,
      mirrorDraw: false,
      pullMirror: true,
      wheelBehavior: "scroll",
    });
  });

  it("何も保存されていない・壊れている場合は既定値に戻す", () => {
    expect(loadPrefs(storage)).toEqual(DEFAULT_PREFS);
    store["origami3.prefs"] = "{壊れた";
    expect(loadPrefs(storage)).toEqual(DEFAULT_PREFS);
    // 覚えておく先が無い環境(保存できない)でも既定値で動く
    expect(loadPrefs(null)).toEqual(DEFAULT_PREFS);
    expect(() => savePrefs(DEFAULT_PREFS, null)).not.toThrow();
  });
});
