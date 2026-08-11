// 画面の見た目の土台(App.css の設計トークン)の検査。
// 各テーマは :root (ポップ)を継承して必要な値だけを上書きするため、
// 実際のカスケードと同じように両方をマージして評価する。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { EDGE_COLORS, contrastRatio, type Rgb } from "./cpColors";
import { UI_THEMES, hexToRgb } from "./displayPrefs";

// vitest は .css の取り込みを空にするため、App.css は文字として直に読む。
const css = readFileSync(new URL("../App.css", import.meta.url), "utf8");
const rendererSource = readFileSync(
  new URL("../components/CpEditor/renderer.ts", import.meta.url),
  "utf8",
);
const sceneSource = readFileSync(
  new URL("../components/Viewer3D/sceneBuilder.ts", import.meta.url),
  "utf8",
);

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
  const escaped = selector
    .trim()
    .split(/\s+/)
    .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("\\s+");
  const match = new RegExp(`${escaped}\\s*\\{([\\s\\S]*?)\\}`).exec(css);
  if (match === null) throw new Error(`CSSブロックがありません: ${selector}`);
  return match[1];
}

function declarationBlocks(selector: string): string[] {
  const escaped = selector
    .trim()
    .split(/\s+/)
    .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("\\s+");
  return [...css.matchAll(new RegExp(`${escaped}\\s*\\{([\\s\\S]*?)\\}`, "g"))].map(
    (match) => match[1],
  );
}

function declarations(block: string): Map<string, string> {
  const result = new Map<string, string>();
  for (const match of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    result.set(match[1], match[2].trim());
  }
  return result;
}

/** WebPのコンテナから画像寸法とbyte数だけを読む。画素QAは受入検査で行う。 */
function webpInfo(file: URL): { width: number; height: number; bytes: number } {
  const readBinary = readFileSync as unknown as (path: string | URL) => Uint8Array;
  const data = readBinary(file);
  const ascii = (start: number, end: number) =>
    String.fromCharCode(...data.subarray(start, end));
  const uint16 = (start: number) => data[start] | (data[start + 1] << 8);
  const uint24 = (start: number) =>
    data[start] | (data[start + 1] << 8) | (data[start + 2] << 16);
  const uint32 = (start: number) =>
    (data[start] |
      (data[start + 1] << 8) |
      (data[start + 2] << 16) |
      (data[start + 3] << 24)) >>>
    0;

  if (ascii(0, 4) !== "RIFF" || ascii(8, 12) !== "WEBP") {
    throw new Error(`WebPではありません: ${file.pathname}`);
  }

  for (let cursor = 12; cursor + 8 <= data.length; ) {
    const kind = ascii(cursor, cursor + 4);
    const size = uint32(cursor + 4);
    const start = cursor + 8;
    if (kind === "VP8X" && start + 10 <= data.length) {
      return {
        width: 1 + uint24(start + 4),
        height: 1 + uint24(start + 7),
        bytes: data.byteLength,
      };
    }
    if (kind === "VP8 " && start + 10 <= data.length) {
      return {
        width: uint16(start + 6) & 0x3fff,
        height: uint16(start + 8) & 0x3fff,
        bytes: data.byteLength,
      };
    }
    if (kind === "VP8L" && start + 5 <= data.length && data[start] === 0x2f) {
      const width = 1 + data[start + 1] + ((data[start + 2] & 0x3f) << 8);
      const height =
        1 + ((data[start + 2] & 0xc0) >> 6) + (data[start + 3] << 2) + ((data[start + 4] & 0x0f) << 10);
      return { width, height, bytes: data.byteLength };
    }
    cursor += 8 + size + (size % 2);
  }
  throw new Error(`WebPの画像チャンクがありません: ${file.pathname}`);
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

const QUALITY_SIGNATURES = {
  japanese: {
    "--color-bg": "#f4efe2",
    "--color-surface": "#fbf8ef",
    "--color-surface-sunken": "#e9e1d0",
    "--color-text": "#2b2723",
    "--color-text-muted": "#625b50",
    "--color-accent": "#1f3f5e",
    "--color-accent-strong": "#17334d",
    "--color-accent-soft": "#dce6ec",
    "--color-secondary": "#61713e",
    "--color-pop-coral": "#b8433a",
    "--radius-sm": "4px",
    "--radius-md": "5px",
    "--radius-lg": "6px",
    "--splitter-background": "#877860",
    "--toolbar-separator-width": "2px",
    "--shadow-button": "0 1px 2px rgba(43, 39, 35, 0.1)",
  },
  modern: {
    "--color-bg": "#f7f8fa",
    "--color-surface": "#ffffff",
    "--color-surface-sunken": "#f1f3f5",
    "--color-text": "#181b20",
    "--color-text-muted": "#56616d",
    "--color-accent": "#2563eb",
    "--color-accent-strong": "#1d4ed8",
    "--color-accent-soft": "#eff6ff",
    "--color-accent-border": "rgba(59, 130, 246, 0.22)",
    "--color-secondary": "#374151",
    "--radius-sm": "8px",
    "--radius-md": "8px",
    "--radius-lg": "8px",
    "--toolbar-separator-width": "1px",
    "--shadow-button": "none",
    "--shadow-panel": "none",
    "--t-fast": "100ms ease-out",
  },
} as const;

describe("設計トークン", () => {
  it("選択可能な5テーマとCSSテーマ定義が1対1で対応する", () => {
    expect(THEMES.map((theme) => theme.id)).toEqual([...UI_THEMES]);
    for (const id of UI_THEMES.filter((theme) => theme !== "pop")) {
      expect(declarationBlocks(`.app[data-theme="${id}"]`), id).toHaveLength(1);
    }
  });

  it("和紙・粒子背景はローカルWebPで、寸法と容量の上限を守る", () => {
    const assets = {
      japanese: "japanese-washi.webp",
      modern: "modern-grain.webp",
    } as const;

    for (const [id, fileName] of Object.entries(assets)) {
      const theme = THEMES.find((candidate) => candidate.id === id);
      expect(theme, id).toBeDefined();
      expect(theme!.selector, id).not.toBeNull();
      const ownTokens = declarations(declarationBlock(theme!.selector!));
      const background = ownTokens.get("--app-background-image") ?? "";
      expect(background, id).toContain(`url("./assets/themes/${fileName}")`);
      expect(background, id).not.toContain("data:image");

      const file = new URL(`../assets/themes/${fileName}`, import.meta.url);
      const info = webpInfo(file);
      expect(info.width, `${id}: width`).toBeGreaterThanOrEqual(1024);
      expect(info.height, `${id}: height`).toBeGreaterThanOrEqual(1024);
      expect(info.bytes, `${id}: bytes`).toBeLessThanOrEqual(500_000);
    }

    expect(Object.values(assets).every((name) => name.endsWith(".webp"))).toBe(true);
    expect(css).not.toMatch(/assets\/themes\/[^"')]*(?:\.png|\.jpe?g)/i);
  });

  it("和風とモダンの品質を決める色・形・影・速さを固定する", () => {
    for (const [id, signature] of Object.entries(QUALITY_SIGNATURES)) {
      const theme = THEMES.find((candidate) => candidate.id === id)!;
      const themeTokens = tokens(theme);
      for (const [name, expected] of Object.entries(signature)) {
        expect(valueOf(themeTokens, name), `${theme.label}: ${name}`).toBe(expected);
      }
    }
  });

  it("背景のむらを含む最暗部でも本文と補助文字が4.5:1以上", () => {
    const cases = [
      { id: "japanese", background: "#e6ded0" },
      { id: "modern", background: "#f2f3f5" },
    ] as const;
    for (const sample of cases) {
      const theme = THEMES.find((candidate) => candidate.id === sample.id)!;
      const background = hexToRgb(sample.background) as Rgb;
      for (const foreground of ["--color-text", "--color-text-muted"] as const) {
        expect(
          contrastRatio(rgbOf(tokens(theme), foreground), background),
          `${theme.label}: ${foreground} / texture dark edge`,
        ).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("動く部品へSVGフィルタを掛けず、背景は静的なローカル画像だけを使う", () => {
    expect(css).not.toMatch(/filter\s*:\s*url\(/i);
    expect(css).not.toMatch(/--app-background-image\s*:[^;]*https?:/i);
    expect(css).not.toMatch(/animation(?:-name)?\s*:[^;]*app-background/i);
  });

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
      "--color-relaxation",
      "--app-background-image",
      "--app-background-size",
      "--app-background-repeat",
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

  it("全ボタンがテーマ共通の土台を通り、OS既定の立体枠へ戻らない", () => {
    const buttonBase = declarationBlock(".app :where(button)");
    expect(buttonBase).toMatch(/appearance\s*:\s*none\s*;/);
    expect(buttonBase).toMatch(
      /border\s*:[^;]*var\(--color-border\)[^;]*;/,
    );
    expect(buttonBase).toMatch(
      /background\s*:\s*var\(--color-control\)\s*;/,
    );
    expect(buttonBase).toMatch(/border-radius\s*:\s*var\(--radius-md\)\s*;/);
    expect(buttonBase).toMatch(/box-shadow\s*:\s*var\(--shadow-button\)\s*;/);
    expect(css).not.toMatch(/\b(?:outset|inset-button)\b/i);
  });

  it("選択肢・チェック・スライダー・色入力はOS既定appearanceを使わない", () => {
    for (const selector of [
      ".app select",
      '.app input[type="checkbox"]',
      '.app input[type="radio"]',
      '.app input[type="range"]',
      '.app input[type="color"]',
    ]) {
      expect(declarationBlock(selector), selector).toMatch(/appearance\s*:\s*none\s*;/);
    }
  });

  it("吹き出しのインライン固定色をテーマトークンで上書きする", () => {
    const tooltip = declarationBlock('[data-floating-ui="tooltip"]');
    expect(tooltip).toMatch(/background\s*:\s*var\(--color-tooltip-background\)\s*!important/);
    expect(tooltip).toMatch(/border[^;]*var\(--color-tooltip-border\)[^;]*!important/);
    expect(tooltip).toMatch(/color\s*:\s*var\(--color-tooltip-text\)\s*!important/);
    expect(tooltip).toMatch(/box-shadow\s*:\s*var\(--shadow-tooltip\)\s*!important/);
    expect(tooltip).toMatch(/padding[^;]*var\(--tooltip-padding-inline\)[^;]*!important/);
  });

  it("選択中ツールのhover規則はテーマ指定を後勝ちで潰さない", () => {
    const hoverRules = declarationBlocks(".tool-button.active:hover:not(:disabled)");
    expect(hoverRules).toHaveLength(1);
    expect(hoverRules[0]).toMatch(
      /background\s*:\s*var\(--tool-active-hover-background\)\s*;/,
    );
  });

  it("rootで先に解決される別名tokenは各テーマ自身が再定義する", () => {
    const aliases = [
      "--color-on-gold",
      "--tool-active-background",
      "--tool-active-color",
      "--tool-active-border",
      "--tool-active-hover-background",
      "--tool-active-hover-color",
      "--tool-active-hover-border",
      "--shadow-collision-guide",
      "--shadow-collision-guide-hover",
      "--toolbar-separator-radius",
      "--toolbar-separator-background",
    ] as const;
    for (const theme of THEMES.filter((candidate) => candidate.selector !== null)) {
      const ownTokens = declarations(declarationBlock(theme.selector!));
      for (const alias of aliases) {
        expect(ownTokens.has(alias), `${theme.label}: ${alias}`).toBe(true);
      }
    }
  });

  it("無効ボタンの文字コントラストを要素opacityで薄めない", () => {
    const disabled = declarationBlock(".app :where(button):disabled");
    expect(disabled).not.toMatch(/\bopacity\s*:/);
    expect(disabled).toMatch(/color\s*:\s*var\(--color-text-muted\)/);
    expect(disabled).toMatch(/background\s*:\s*var\(--color-surface-sunken\)/);
  });

  it("濃色の操作面はhover時も白文字を維持する", () => {
    const viewerReset = declarationBlock(".viewer-reset:hover,\n.viewer-reset:focus-visible");
    expect(viewerReset).toMatch(/color\s*:\s*var\(--color-on-solid\)/);
    const collision = declarationBlock(
      ".suspect-hinge-guide:hover,\n.suspect-hinge-guide:focus-visible",
    );
    expect(collision).toMatch(/color\s*:\s*var\(--color-collision-text\)/);
  });

  it("初回案内の主操作はhoverとactiveでも主ボタン配色を保つ", () => {
    const rules = declarationBlock(
      ".first-run-guide-prepare:hover:not(:disabled),\n.first-run-guide-done:hover:not(:disabled),\n.first-run-guide-prepare:active:not(:disabled),\n.first-run-guide-done:active:not(:disabled)",
    );
    expect(rules).toMatch(/background\s*:\s*var\(--color-accent-strong\)/);
    expect(rules).toMatch(/color\s*:\s*var\(--button-primary-color\)/);
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

  it("作品の選択・食い込み・追従・操作中の意味色をテーマ変更で変えない", () => {
    expect(rendererSource).toMatch(/selection:\s*"#ff9500"/);
    expect(rendererSource).toMatch(/suspect:\s*"#ff2438"/);
    expect(rendererSource).toMatch(/relaxed:\s*"#d97706"/);
    expect(rendererSource).toMatch(/active:\s*"#40cfff"/);
    expect(rendererSource).toMatch(/foldSuggestion:\s*"#d97706"/);
    expect(sceneSource).toMatch(/HIGHLIGHT_COLOR\s*=\s*0xffd400/);
    expect(sceneSource).toMatch(/REFERENCE_HIGHLIGHT_COLOR\s*=\s*0x40cfff/);
    expect(sceneSource).toMatch(/SUSPECT_HIGHLIGHT_COLOR\s*=\s*0xff2038/);
    expect(sceneSource).toMatch(/RELAXED_HIGHLIGHT_COLOR\s*=\s*0xd97706/);
    expect(sceneSource).toMatch(/ACTIVE_HIGHLIGHT_COLOR\s*=\s*0x40cfff/);
    expect(sceneSource).toMatch(/PREVIEW_COLOR\s*=\s*0x2f8fff/);

    for (const theme of THEMES) {
      expect(valueOf(tokens(theme), "--color-relaxation")).toBe("#d97706");
    }

    const semanticColors = ["#d97706", "#40cfff", "#ff2038"].map(
      (value) => hexToRgb(value) as Rgb,
    );
    for (let left = 0; left < semanticColors.length; left += 1) {
      for (let right = left + 1; right < semanticColors.length; right += 1) {
        const distance = Math.hypot(
          ...semanticColors[left].map(
            (value, index) => value - semanticColors[right][index],
          ),
        );
        expect(distance, `${left}/${right}`).toBeGreaterThan(100);
      }
    }
  });

  it("シンプルの太字は600を超えず、モダンの選択ボタンは青1色で階層化する", () => {
    const simple = tokens(THEMES[1]);
    expect(Number(valueOf(simple, "--fw-bold"))).toBeLessThanOrEqual(600);
    expect(Number(valueOf(simple, "--fw-black"))).toBeLessThanOrEqual(600);

    const modern = tokens(THEMES[3]);
    expect(valueOf(modern, "--tool-active-background")).toBe("#2563eb");
    expect(valueOf(modern, "--tool-active-color")).toBe("#ffffff");
    expect(valueOf(modern, "--tool-active-hover-background")).toBe("#1d4ed8");
    expect(valueOf(modern, "--tool-active-hover-color")).toBe("#ffffff");
  });
});
