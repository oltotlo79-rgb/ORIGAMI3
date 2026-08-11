// 見た目の好み(紙の色・方眼の分割数・分割比)の丸めと保管のテスト。

import { beforeEach, describe, expect, it } from "vitest";
import {
  DEFAULT_CONTEXT_PANEL_RATIO,
  DEFAULT_DISPLAY,
  DEFAULT_PREFS,
  MAX_CONTEXT_PANEL_RATIO,
  MIN_CONTEXT_PANEL_RATIO,
  UI_THEMES,
  clampContextPanelRatio,
  clampDivisions,
  clampSplitRatio,
  hexToRgb,
  loadPrefs,
  overlapPreventionOf,
  penetrationPreventionOf,
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
  it("方眼の分割数は2〜1024の整数に丸める", () => {
    expect(clampDivisions(1)).toBe(2);
    expect(clampDivisions(100)).toBe(100);
    expect(clampDivisions(1024)).toBe(1024);
    expect(clampDivisions(2048)).toBe(1024);
    expect(clampDivisions(8.4)).toBe(8);
    expect(clampDivisions(Number.NaN)).toBe(DEFAULT_DISPLAY.grid_divisions);
  });

  it("分割比は2割〜8割に収める", () => {
    expect(clampSplitRatio(0.05)).toBeCloseTo(0.2);
    expect(clampSplitRatio(0.95)).toBeCloseTo(0.8);
    expect(clampSplitRatio(0.35)).toBeCloseTo(0.35);
    expect(clampSplitRatio(Number.NaN)).toBeCloseTo(0.5);
  });

  it("下部パネルの比率は25%〜55%に収め、不正値は32%へ戻す", () => {
    expect(DEFAULT_CONTEXT_PANEL_RATIO).toBeCloseTo(0.32);
    expect(MIN_CONTEXT_PANEL_RATIO).toBeCloseTo(0.25);
    expect(MAX_CONTEXT_PANEL_RATIO).toBeCloseTo(0.55);
    expect(clampContextPanelRatio(0.1)).toBeCloseTo(MIN_CONTEXT_PANEL_RATIO);
    expect(clampContextPanelRatio(0.8)).toBeCloseTo(MAX_CONTEXT_PANEL_RATIO);
    expect(clampContextPanelRatio(0.4)).toBeCloseTo(0.4);
    expect(clampContextPanelRatio(Number.NaN)).toBeCloseTo(
      DEFAULT_CONTEXT_PANEL_RATIO,
    );
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

  it("食い込み検出は既定でオンで、項目の無い古い作品もオンとして扱う", () => {
    expect(penetrationPreventionOf(DEFAULT_DISPLAY)).toBe(true);
    const oldDisplay = { ...DEFAULT_DISPLAY };
    delete oldDisplay.penetration_prevention_enabled;
    expect(penetrationPreventionOf(oldDisplay)).toBe(true);
    expect(
      penetrationPreventionOf({
        ...DEFAULT_DISPLAY,
        penetration_prevention_enabled: false,
      }),
    ).toBe(false);
  });

  it("保存した好みは次に読むとそのまま戻る", () => {
    savePrefs(
      {
        splitRatio: 0.3,
        contextPanelRatio: 0.44,
        mirrorDraw: true,
        mirrorAxis: "paperHorizontal",
        pullMirror: false,
        wheelBehavior: "zoom",
        uiTheme: "classic",
        contextHelpExpanded: false,
        viewerHintExpanded: true,
        cpHelpExpanded: false,
        paperHelpExpanded: true,
        paperColorExpanded: true,
      },
      storage,
    );
    const loaded = loadPrefs(storage);
    expect(loaded.splitRatio).toBeCloseTo(0.3);
    expect(loaded.contextPanelRatio).toBeCloseTo(0.44);
    // 左右対称に描く指定も端末に覚えておく(CPE-010)
    expect(loaded.mirrorDraw).toBe(true);
    expect(loaded.mirrorAxis).toBe("paperHorizontal");
    // 3Dで引くときの左右同時の指定も覚えておく(UI-007)
    expect(loaded.pullMirror).toBe(false);
    // 2D展開図のホイール動作も端末ごとに戻る
    expect(loaded.wheelBehavior).toBe("zoom");
    // 画面デザインも作品とは分けて端末へ覚える
    expect(loaded.uiTheme).toBe("classic");
    // 操作説明の開閉も作品ではなく端末ごとの選択として戻る
    expect(loaded.contextHelpExpanded).toBe(false);
    expect(loaded.viewerHintExpanded).toBe(true);
    expect(loaded.cpHelpExpanded).toBe(false);
    expect(loaded.paperHelpExpanded).toBe(true);
    expect(loaded.paperColorExpanded).toBe(true);
  });

  it("操作説明は初回と旧版では畳み、一度開いた選択は次回も保つ", () => {
    expect(loadPrefs(storage)).toMatchObject({
      contextHelpExpanded: false,
      viewerHintExpanded: false,
      cpHelpExpanded: false,
      paperHelpExpanded: false,
      paperColorExpanded: false,
    });

    // 開閉項目をまだ持たない旧版の保存も、文字を増やさないよう畳んで補う
    storage.setItem(
      "origami3.prefs",
      JSON.stringify({ splitRatio: 0.4, uiTheme: "modern" }),
    );
    expect(loadPrefs(storage)).toMatchObject({
      contextHelpExpanded: false,
      viewerHintExpanded: false,
      cpHelpExpanded: false,
      paperHelpExpanded: false,
      paperColorExpanded: false,
    });

    savePrefs(
      {
        ...DEFAULT_PREFS,
        contextHelpExpanded: true,
        viewerHintExpanded: true,
        cpHelpExpanded: true,
        paperHelpExpanded: true,
        paperColorExpanded: true,
      },
      storage,
    );
    expect(loadPrefs(storage)).toMatchObject({
      contextHelpExpanded: true,
      viewerHintExpanded: true,
      cpHelpExpanded: true,
      paperHelpExpanded: true,
      paperColorExpanded: true,
    });
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

  it("対称操作の基準は縦中心が既定で、横中心だけを端末へ保存できる", () => {
    expect(DEFAULT_PREFS.mirrorAxis).toBe("paperVertical");
    expect(loadPrefs(storage).mirrorAxis).toBe("paperVertical");
    savePrefs({ ...DEFAULT_PREFS, mirrorAxis: "paperHorizontal" }, storage);
    expect(loadPrefs(storage).mirrorAxis).toBe("paperHorizontal");
    storage.setItem(
      "origami3.prefs",
      JSON.stringify({ ...DEFAULT_PREFS, mirrorAxis: "selectedLine" }),
    );
    expect(loadPrefs(storage).mirrorAxis).toBe("paperVertical");
  });

  it("画面デザインは5テーマから選び、未保存・不正値はポップへ戻す", () => {
    expect(UI_THEMES).toEqual(["pop", "simple", "japanese", "modern", "classic"]);
    expect(DEFAULT_PREFS.uiTheme).toBe("pop");
    expect(loadPrefs(storage).uiTheme).toBe("pop");

    for (const uiTheme of UI_THEMES) {
      savePrefs({ ...DEFAULT_PREFS, uiTheme }, storage);
      expect(loadPrefs(storage).uiTheme).toBe(uiTheme);
    }

    storage.setItem(
      "origami3.prefs",
      JSON.stringify({ ...DEFAULT_PREFS, uiTheme: "unknown" }),
    );
    expect(loadPrefs(storage).uiTheme).toBe("pop");
  });

  it("3Dで引くときの左右同時は、保存が無ければ既定のオン(UI-007)", () => {
    expect(DEFAULT_PREFS.pullMirror).toBe(true);
    expect(loadPrefs(storage).pullMirror).toBe(true);
    // 古い版が書いた好み(pullMirrorが無い)を読んでもオンのまま
    storage.setItem("origami3.prefs", JSON.stringify({ splitRatio: 0.4 }));
    expect(loadPrefs(storage).pullMirror).toBe(true);
  });

  it("下部パネルの比率が無い旧版の保存は32%で補う", () => {
    storage.setItem(
      "origami3.prefs",
      JSON.stringify({
        splitRatio: 0.4,
        mirrorDraw: true,
        pullMirror: false,
        wheelBehavior: "zoom",
        uiTheme: "modern",
      }),
    );
    expect(loadPrefs(storage).contextPanelRatio).toBeCloseTo(
      DEFAULT_CONTEXT_PANEL_RATIO,
    );
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
      contextPanelRatio: DEFAULT_CONTEXT_PANEL_RATIO,
      mirrorDraw: false,
      mirrorAxis: "paperVertical",
      pullMirror: true,
      wheelBehavior: "scroll",
      uiTheme: "pop",
      contextHelpExpanded: false,
      viewerHintExpanded: false,
      cpHelpExpanded: false,
      paperHelpExpanded: false,
      paperColorExpanded: false,
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
