// 展開図の色づくりのテスト。「紙の色を変えても折り線が読めること」を
// コントラスト比の数値で確かめる(実機で赤い紙の山折り線が見えなかった不具合)。

import { describe, expect, it } from "vitest";
import { DEFAULT_DISPLAY } from "./displayPrefs";
import {
  EDGE_COLORS,
  MIN_LINE_CONTRAST,
  contrastRatio,
  edgeRgb,
  gridColor,
  haloColor,
  mixWhite,
  paperFill,
  relativeLuminance,
  worstLineContrast,
  type Rgb,
} from "./cpColors";

/** 試す紙の色(既定の赤・白・黒・濃い青・黄緑・鮮やかな青) */
const PAPERS: Record<string, Rgb> = {
  既定の赤: DEFAULT_DISPLAY.front_color,
  白: [255, 255, 255],
  黒: [0, 0, 0],
  濃い青: [0, 0, 128],
  黄緑: [120, 220, 60],
  青: [0, 128, 255],
};

describe("相対輝度とコントラスト比", () => {
  it("黒は0、白は1の輝度になる", () => {
    expect(relativeLuminance([0, 0, 0])).toBeCloseTo(0, 5);
    expect(relativeLuminance([255, 255, 255])).toBeCloseTo(1, 5);
  });

  it("白と黒のコントラスト比は21、同じ色どうしは1になる", () => {
    expect(contrastRatio([0, 0, 0], [255, 255, 255])).toBeCloseTo(21, 3);
    expect(contrastRatio([237, 28, 36], [237, 28, 36])).toBeCloseTo(1, 5);
  });
});

describe("紙の塗り色", () => {
  it("赤い紙をそのまま塗ると山折り線がほぼ同じ色になってしまう(不具合の再現)", () => {
    const raw = DEFAULT_DISPLAY.front_color;
    expect(contrastRatio(raw, edgeRgb("Mountain"))).toBeLessThan(1.2);
  });

  it("既定の赤い紙でも山折り線が3:1以上で読める", () => {
    const fill = paperFill(DEFAULT_DISPLAY.front_color);
    expect(contrastRatio(fill, edgeRgb("Mountain"))).toBeGreaterThanOrEqual(3);
    // 白へ薄めても赤みは残る(紙の色が分かる)
    expect(fill[0]).toBeGreaterThan(fill[1] + 20);
  });

  it("どの紙の色でも全ての線種が読める", () => {
    for (const [name, rgb] of Object.entries(PAPERS)) {
      const fill = paperFill(rgb);
      for (const kind of Object.keys(EDGE_COLORS) as (keyof typeof EDGE_COLORS)[]) {
        const cr = contrastRatio(fill, edgeRgb(kind));
        expect(cr, `${name}の紙 × ${kind}`).toBeGreaterThanOrEqual(MIN_LINE_CONTRAST);
      }
    }
  });

  it("読める範囲でいちばん濃い候補を選ぶ(必要以上に白くしない)", () => {
    const fill = paperFill(DEFAULT_DISPLAY.front_color);
    // 1段濃くすると、どれかの線種が下限を割る
    expect(worstLineContrast(mixWhite(DEFAULT_DISPLAY.front_color, 0.25))).toBeLessThan(
      MIN_LINE_CONTRAST,
    );
    expect(fill).toEqual(mixWhite(DEFAULT_DISPLAY.front_color, 0.2));
  });

  it("白い紙は白のまま塗る", () => {
    expect(paperFill([255, 255, 255])).toEqual([255, 255, 255]);
  });
});

describe("方眼と縁取り", () => {
  it("方眼は紙より暗く、折り線ほどは目立たない", () => {
    for (const rgb of Object.values(PAPERS)) {
      const fill = paperFill(rgb);
      const grid = gridColor(fill);
      expect(relativeLuminance(grid)).toBeLessThan(relativeLuminance(fill));
      expect(contrastRatio(fill, grid)).toBeLessThan(
        contrastRatio(fill, edgeRgb("Mountain")),
      );
    }
  });

  it("縁取りは紙が明るければ白、暗ければ黒を敷く", () => {
    expect(haloColor([250, 210, 210])).toContain("255");
    expect(haloColor([10, 10, 10])).toContain("24");
  });

  it("選択中の橙色の帯の上でも、白い縁取りを挟めば線種の色が読める", () => {
    const halo: Rgb = [255, 255, 255];
    const selection: Rgb = [255, 149, 0];
    expect(contrastRatio(selection, edgeRgb("Mountain"))).toBeLessThan(
      MIN_LINE_CONTRAST,
    );
    expect(contrastRatio(halo, edgeRgb("Mountain"))).toBeGreaterThanOrEqual(
      MIN_LINE_CONTRAST,
    );
  });
});
