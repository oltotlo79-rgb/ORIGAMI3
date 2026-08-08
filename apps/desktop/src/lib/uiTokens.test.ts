// 画面の見た目の土台(App.css の設計トークン)の検査。
// 各テーマは :root (ポップ)を継承して必要な値だけを上書きするため、
// 実際のカスケードと同じように両方をマージして評価する。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { EDGE_COLORS, contrastRatio, type Rgb } from "./cpColors";
import { hexToRgb } from "./displayPrefs";

// vitest は .css の取り込みを空にするため、App.css は文字として直に読む。
const css = readFileSync(new URL("../App.css", import.meta.url), "utf8");

const THEMES = [
  { id: "pop", label: "ポップ", selector: null },
  { id: "simple", label: "シンプル", selector: '.app[data-theme="simple"]' },
  { id: "japanese", label: "和風", selector: '.app[data-theme="japanese"]' },
  { id: "modern", label: "モダン", selector: '.app[data-theme="modern"]' },
  { id: "classic", label: "クラシック", selector: '.app[data-theme="classic"]' },
] as const;

type Theme = (typeof THEMES)[number];
type ThemeId = Theme["id"];

const EXPECTED_CANVAS_COLORS: Record<
  ThemeId,
  { "--color-canvas-2d": string; "--color-canvas-3d": string }
> = {
  pop: { "--color-canvas-2d": "#ddd8d0", "--color-canvas-3d": "#cfcbc2" },
  simple: { "--color-canvas-2d": "#f0f0f1", "--color-canvas-3d": "#e7e7e9" },
  japanese: { "--color-canvas-2d": "#efe8d8", "--color-canvas-3d": "#e5dcc6" },
  modern: { "--color-canvas-2d": "#f4f4f5", "--color-canvas-3d": "#e9e9eb" },
  classic: { "--color-canvas-2d": "#ede4d0", "--color-canvas-3d": "#e3d8bf" },
};

function declarationBlock(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`${escaped}\\s*\\{([\\s\\S]*?)\\}`).exec(css);
  if (match === null) throw new Error(`CSSブロックがありません: ${selector}`);
  return match[1];
}

function declarations(block: string): Map<string, string> {
  const result = new Map<string, string>();
  for (const match of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    result.set(match[1], match[2].trim());
  }
  return result;
}

/** :root とテーマ固有ブロックをマージし、実効トークンを返す。 */
function tokens(theme: Theme = THEMES[0]): Map<string, string> {
  const result = declarations(declarationBlock(":root"));
  if (theme.selector !== null) {
    for (const [name, value] of declarations(declarationBlock(theme.selector))) {
      result.set(name, value);
    }
  }
  return result;
}

/** 単純な var(--token) の参照をたどり、実効値を返す。 */
function valueOf(themeTokens: Map<string, string>, name: string): string {
  let value = themeTokens.get(name);
  if (value === undefined) throw new Error(`トークンがありません: ${name}`);

  const visited = new Set<string>();
  while (true) {
    const reference = /^var\(\s*(--[\w-]+)\s*\)$/.exec(value);
    if (reference === null) return value;
    if (visited.has(reference[1])) throw new Error(`トークン参照が循環しています: ${name}`);
    visited.add(reference[1]);
    value = themeTokens.get(reference[1]);
    if (value === undefined) throw new Error(`参照先トークンがありません: ${reference[1]}`);
  }
}

/** トークン名 → [r,g,b] (16進色でない値は例外にする)。 */
function rgbOf(themeTokens: Map<string, string>, name: string): Rgb {
  const raw = valueOf(themeTokens, name);
  const functional = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/.exec(raw);
  const rgb = functional
    ? ([Number(functional[1]), Number(functional[2]), Number(functional[3])] as Rgb)
    : hexToRgb(raw);
  if (rgb === null) throw new Error(`16進色のトークンではありません: ${name} = ${raw}`);
  return rgb;
}

/** gradient内を含めてvar参照を展開する。 */
function expandedValue(themeTokens: Map<string, string>, name: string): string {
  let value = themeTokens.get(name);
  if (value === undefined) throw new Error(`トークンがありません: ${name}`);
  for (let depth = 0; depth < 12; depth++) {
    let replaced = false;
    value = value.replace(/var\(\s*(--[\w-]+)\s*\)/g, (_match, reference: string) => {
      const next = themeTokens.get(reference);
      if (next === undefined) throw new Error(`参照先トークンがありません: ${reference}`);
      replaced = true;
      return next;
    });
    if (!replaced) return value;
  }
  throw new Error(`トークン参照が深すぎます: ${name}`);
}

/** 単色またはgradientに含まれる不透明色をすべて取り出す。 */
function colorsOf(themeTokens: Map<string, string>, name: string): Rgb[] {
  const value = expandedValue(themeTokens, name);
  const colors = [
    ...value.matchAll(/#[0-9a-fA-F]{6}\b|rgba?\(\s*\d+\s*,\s*\d+\s*,\s*\d+[^)]*\)/g),
  ].map((match) => {
    const raw = match[0];
    const functional = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/.exec(raw);
    const rgb = functional
      ? ([Number(functional[1]), Number(functional[2]), Number(functional[3])] as Rgb)
      : hexToRgb(raw);
    if (rgb === null) throw new Error(`色を読めません: ${name} = ${raw}`);
    return rgb;
  });
  if (colors.length === 0) throw new Error(`背景色を読めません: ${name} = ${value}`);
  return colors;
}

/** 文字色と背景色の組(すべて4.5:1以上が必要)。 */
const TEXT_PAIRS: readonly (readonly [string, string])[] = [
  ["--color-text", "--color-surface"],
  ["--color-text", "--color-bg"],
  ["--color-text", "--color-control"],
  ["--color-text", "--color-surface-sunken"],
  ["--color-text-muted", "--color-surface"],
  ["--color-text-muted", "--color-bg"],
  ["--color-text-muted", "--color-surface-sunken"],
  ["--color-danger", "--color-surface"],
  ["--color-warn", "--color-surface"],
  ["--color-accent-strong", "--color-accent-soft"],
  ["--color-secondary-strong", "--color-surface"],
  ["--heading-color", "--color-surface"],
  ["--button-hover-color", "--button-hover-background"],
  ["--color-text", "--color-warning-surface"],
  ["--color-warn", "--color-warning-surface"],
  ["--color-danger", "--color-danger-soft"],
  ["--color-on-gold", "--color-pop-yellow"],
  ["--color-on-coral", "--color-pop-coral"],
];

/** --color-on-solid (白)を文字色として使う単色面。 */
const SOLID_SURFACES = [
  "--color-accent",
  "--color-accent-strong",
  "--color-secondary",
  "--color-danger",
  "--color-warn-badge",
] as const;

describe("設計トークン", () => {
  it("色・余白・角丸・影・文字の大きさの段階が定義されている", () => {
    const themeTokens = tokens();
    const required = [
      "--color-bg",
      "--color-surface",
      "--color-border",
      "--color-accent",
      "--color-canvas-2d",
      "--color-canvas-3d",
      "--color-crease-mountain",
      "--color-crease-valley",
      "--sp-2",
      "--sp-4",
      "--radius-sm",
      "--radius-md",
      "--shadow-1",
      "--shadow-dialog",
      "--fs-xs",
      "--fs-sm",
      "--fs-md",
      "--fs-lg",
      "--lh-body",
    ];
    for (const name of required) expect(themeTokens.has(name), name).toBe(true);
  });

  it("使っているトークンはすべて :root で定義されている", () => {
    const rootTokens = tokens();
    for (const match of css.matchAll(/var\((--[\w-]+)\)/g)) {
      expect(rootTokens.has(match[1]), match[1]).toBe(true);
    }
  });

  for (const theme of THEMES) {
    describe(theme.label, () => {
      const themeTokens = tokens(theme);

      it("文字色は指定された背景色に対してコントラスト比4.5:1以上", () => {
        for (const [foreground, background] of TEXT_PAIRS) {
          expect(
            contrastRatio(rgbOf(themeTokens, foreground), rgbOf(themeTokens, background)),
            `${theme.label}: ${foreground} / ${background}`,
          ).toBeGreaterThanOrEqual(4.5);
        }
      });

      it("単色面の上の白文字はコントラスト比4.5:1以上", () => {
        const onSolid = rgbOf(themeTokens, "--color-on-solid");
        for (const surface of SOLID_SURFACES) {
          expect(
            contrastRatio(onSolid, rgbOf(themeTokens, surface)),
            `${theme.label}: --color-on-solid / ${surface}`,
          ).toBeGreaterThanOrEqual(4.5);
        }
      });

      it("主ボタンと選択中ボタンはgradient終端を含め文字コントラスト比4.5:1以上", () => {
        const buttonPairs = [
          ["--button-primary-color", "--button-primary-background"],
          ["--tool-active-color", "--tool-active-background"],
          ["--tool-active-hover-color", "--tool-active-hover-background"],
        ] as const;
        for (const [foreground, background] of buttonPairs) {
          const text = rgbOf(themeTokens, foreground);
          for (const color of colorsOf(themeTokens, background)) {
            expect(
              contrastRatio(text, color),
              `${theme.label}: ${foreground} / ${background}`,
            ).toBeGreaterThanOrEqual(4.5);
          }
        }
      });

      it("2D/3Dキャンバス色がテーマ仕様どおり", () => {
        for (const [name, expected] of Object.entries(EXPECTED_CANVAS_COLORS[theme.id])) {
          expect(valueOf(themeTokens, name), `${theme.label}: ${name}`).toBe(expected);
        }
      });

      it("山折り・谷折り線の色は全テーマ共通でEDGE_COLORSと一致する", () => {
        expect(valueOf(themeTokens, "--color-crease-mountain").toLowerCase()).toBe(
          EDGE_COLORS.Mountain.toLowerCase(),
        );
        expect(valueOf(themeTokens, "--color-crease-valley").toLowerCase()).toBe(
          EDGE_COLORS.Valley.toLowerCase(),
        );
      });
    });
  }

  it("ポップの効き色は展開図の山=赤・谷=青と取り違えない色にする", () => {
    const accent = rgbOf(tokens(), "--color-accent");
    for (const kind of ["Mountain", "Valley"] as const) {
      const line = hexToRgb(EDGE_COLORS[kind]) as Rgb;
      const distance = Math.hypot(...accent.map((value, index) => value - line[index]));
      expect(distance, kind).toBeGreaterThan(80);
    }
  });

  it("シンプルの太字は600を超えず、モダンの墨ボタンはhoverで白地へ反転する", () => {
    const simple = tokens(THEMES[1]);
    expect(Number(valueOf(simple, "--fw-bold"))).toBeLessThanOrEqual(600);
    expect(Number(valueOf(simple, "--fw-black"))).toBeLessThanOrEqual(600);

    const modern = tokens(THEMES[3]);
    expect(valueOf(modern, "--tool-active-background")).toBe("#18181b");
    expect(valueOf(modern, "--tool-active-color")).toBe("#ffffff");
    expect(valueOf(modern, "--tool-active-hover-background")).toBe("#ffffff");
    expect(valueOf(modern, "--tool-active-hover-color")).toBe("#18181b");
  });
});
