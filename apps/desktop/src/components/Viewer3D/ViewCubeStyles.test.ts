import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { contrastRatio, type Rgb } from "../../lib/cpColors";
import { UI_THEMES, hexToRgb } from "../../lib/displayPrefs";

const appCss = readFileSync(new URL("../../App.css", import.meta.url), "utf8").replace(
  /\r\n/g,
  "\n",
);

function cssDeclarations(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return appCss.match(new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`, "s"))?.[1] ?? "";
}

interface CssRule {
  selectors: string[];
  body: string;
}

// 注釈を外してから規則へ割る。選び手は複数行に分かれるため、各行の末尾だけを見る。
const CSS_RULES: CssRule[] = [
  ...appCss.replace(/\/\*[\s\S]*?\*\//g, "\n").matchAll(/([^{}]+)\{([^{}]*)\}/g),
].map((match) => ({
  selectors: match[1]
    .split(",")
    .map((part) => (part.split("\n").pop() ?? "").trim())
    .filter((part) => part.length > 0),
  body: match[2],
}));

function declarations(block: string): Map<string, string> {
  const result = new Map<string, string>();
  for (const match of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    result.set(match[1], match[2].trim());
  }
  return result;
}

/** テーマ用の宣言は :root を土台に必要な値だけ上書きする。実際のカスケードに合わせる。 */
function themeTokens(selector: string | null): Map<string, string> {
  const merged = declarations(cssDeclarations(":root"));
  if (selector !== null) {
    for (const [key, value] of declarations(cssDeclarations(selector))) {
      merged.set(key, value);
    }
  }
  return merged;
}

function resolveToken(tokens: Map<string, string>, name: string): string {
  let value = tokens.get(name);
  for (let step = 0; step < 8 && value !== undefined && value.startsWith("var("); step += 1) {
    const inner = /var\(\s*(--[\w-]+)\s*\)/.exec(value)?.[1];
    value = inner === undefined ? undefined : tokens.get(inner);
  }
  if (value === undefined) throw new Error(`未定義の色: ${name}`);
  return value;
}

function rgbOf(tokens: Map<string, string>, name: string): Rgb {
  const rgb = hexToRgb(resolveToken(tokens, name));
  if (rgb === null) throw new Error(`色として読めません: ${name}`);
  return rgb;
}

/** 白の薄い膜を重ねたあとの実際の見た目の色。辺・角の明るさの差を数値で見るのに使う。 */
function overWhite(base: Rgb, alpha: number): Rgb {
  return base.map((channel) => channel + (255 - channel) * alpha) as Rgb;
}

/** 辺・角に重ねる白い膜の濃さをCSSから読む(テストへ数値を写し取らない)。 */
function whiteVeilAlpha(name: string): number {
  const value = resolveToken(themeTokens(null), name);
  const alpha = /rgba\(\s*255\s*,\s*255\s*,\s*255\s*,\s*([\d.]+)\s*\)/.exec(value)?.[1];
  if (alpha === undefined) throw new Error(`白い膜として読めません: ${name}`);
  return Number(alpha);
}

function toLab([r, g, b]: Rgb): [number, number, number] {
  const linear = (value: number) => {
    const v = value / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  const [lr, lg, lb] = [linear(r), linear(g), linear(b)];
  const x = (0.4124 * lr + 0.3576 * lg + 0.1805 * lb) / 0.95047;
  const y = 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
  const z = (0.0193 * lr + 0.1192 * lg + 0.9505 * lb) / 1.08883;
  const f = (t: number) => (t > 0.008856 ? Math.cbrt(t) : 7.787 * t + 16 / 116);
  const [fx, fy, fz] = [f(x), f(y), f(z)];
  return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
}

/** 色の見た目の隔たり。10を超えると別の色として見分けられるとされる。 */
function colorDistance(a: Rgb, b: Rgb): number {
  const [la, aa, ba] = toLab(a);
  const [lb, ab, bb] = toLab(b);
  return Math.hypot(la - lb, aa - ab, ba - bb);
}

const THEME_SELECTOR: Record<(typeof UI_THEMES)[number], string | null> = {
  pop: null,
  simple: '.app[data-theme="simple"]',
  japanese: '.app[data-theme="japanese"]',
  modern: '.app[data-theme="modern"]',
  classic: '.app[data-theme="classic"]',
};

/** 向かい合う面は同じ色。色そのものは書かず、テーマの変数を指す。 */
const FACE_COLOR_GROUPS = [
  { name: "前と後", faces: ["front", "back"], token: "--color-accent" },
  { name: "左と右", faces: ["left", "right"], token: "--color-secondary" },
  { name: "上と下", faces: ["top", "bottom"], token: "--color-warn-badge" },
] as const;

function faceColorToken(face: string): string | null {
  for (const rule of CSS_RULES) {
    if (!rule.selectors.includes(`.view-cube-face-${face}`)) continue;
    const match = /--view-cube-face-color:\s*var\(\s*(--[\w-]+)\s*\)/.exec(rule.body);
    if (match) return match[1];
  }
  return null;
}

describe("視点立方体のCSS契約", () => {
  it("外接枠を3D区画の右上12px、96px四方に固定する", () => {
    const declarationText = cssDeclarations(".view-cube");
    expect(declarationText).toContain("top: 12px;");
    expect(declarationText).toContain("right: 12px;");
    expect(declarationText).toContain("width: 96px;");
    expect(declarationText).toContain("height: 96px;");
    expect(declarationText).toContain("position: absolute;");
  });

  it("面を3×3へ割り、縁13pxの帯を辺と角の押し場所にする", () => {
    const face = cssDeclarations(".view-cube-face");
    expect(face).toContain("grid-template-columns: 13px 1fr 13px;");
    expect(face).toContain("grid-template-rows: 13px 1fr 13px;");
    expect(face).toContain("box-sizing: border-box;");
    expect(face).toContain("width: 48px;");
    expect(face).toContain("height: 48px;");
    expect(face).toContain("background: var(--view-cube-face-color);");
  });

  it("押せることをカーソルで示し、指した場所を色と枠で知らせる", () => {
    const zone = cssDeclarations(".view-cube-zone");
    expect(zone).toContain("cursor: pointer;");
    expect(zone).toContain("color: var(--color-on-solid);");
    expect(appCss).toMatch(
      /\.view-cube-zone:hover,\n\.view-cube-zone:focus-visible,\n\.view-cube-zone\[data-pointed="true"\] \{[\s\S]*?background: var\(--color-accent-soft\);[\s\S]*?box-shadow: inset 0 0 0 2px var\(--color-accent\);/,
    );
  });

  it("辺と角は面より明るく塗り、押す前から押し場所が見分けられる", () => {
    expect(cssDeclarations('.view-cube-zone[data-view-cube-kind="edge"]')).toContain(
      "background: var(--color-swatch-highlight-soft);",
    );
    const corner = cssDeclarations('.view-cube-zone[data-view-cube-kind="corner"]');
    expect(corner).toContain("background-color: var(--color-swatch-highlight);");
    expect(corner).toContain("var(--color-swatch-highlight-soft)");
  });

  it("左上案内と条件付きの右上通知も120pxを予約する", () => {
    expect(cssDeclarations(".viewer-operation-hint")).toContain("right: 120px;");
    expect(cssDeclarations(".status-badge")).toContain("right: 120px;");
    expect(cssDeclarations(".suspect-hinge-guide")).toContain("right: 120px;");
  });
});

describe("視点立方体の面の色", () => {
  it("6面に色が付き、向かい合う3組は同じ色になる", () => {
    const tokens = new Map<string, string>();
    for (const group of FACE_COLOR_GROUPS) {
      for (const face of group.faces) {
        const token = faceColorToken(face);
        expect(token).toBe(group.token);
        tokens.set(face, token ?? "");
      }
    }
    expect(tokens.size).toBe(6);
    expect(new Set(tokens.values()).size).toBe(3);
    // 色は直に書かず、必ずテーマの変数を指す。
    expect(appCss).not.toMatch(/--view-cube-face-color:\s*#/);
  });

  it.each([...UI_THEMES])("%sのテーマで3色が互いに見分けられる", (theme) => {
    const tokens = themeTokens(THEME_SELECTOR[theme]);
    const colors = FACE_COLOR_GROUPS.map((group) => rgbOf(tokens, group.token));
    for (let a = 0; a < colors.length; a += 1) {
      for (let b = a + 1; b < colors.length; b += 1) {
        expect(colorDistance(colors[a], colors[b])).toBeGreaterThan(15);
      }
    }
  });

  it.each([...UI_THEMES])("%sのテーマで面の呼び名が色の上でも読める", (theme) => {
    const tokens = themeTokens(THEME_SELECTOR[theme]);
    const text = rgbOf(tokens, "--color-on-solid");
    for (const group of FACE_COLOR_GROUPS) {
      expect(contrastRatio(text, rgbOf(tokens, group.token))).toBeGreaterThanOrEqual(4.5);
    }
    // 指した場所の色でも読めること。
    const pointedText = rgbOf(tokens, "--color-accent-strong");
    const pointedBackground = rgbOf(tokens, "--color-accent-soft");
    expect(contrastRatio(pointedText, pointedBackground)).toBeGreaterThanOrEqual(4.5);
  });

  it.each([...UI_THEMES])("%sのテーマで、色を付けても面・辺・角の明るさの差が残る", (theme) => {
    const tokens = themeTokens(THEME_SELECTOR[theme]);
    const soft = whiteVeilAlpha("--color-swatch-highlight-soft");
    const strong = whiteVeilAlpha("--color-swatch-highlight");
    // 角は薄い膜を2枚重ねるため、辺よりさらに明るくなる。
    const cornerAlpha = strong + soft * (1 - strong);
    for (const group of FACE_COLOR_GROUPS) {
      const base = rgbOf(tokens, group.token);
      const edge = overWhite(base, soft);
      const corner = overWhite(base, cornerAlpha);
      expect(colorDistance(base, edge)).toBeGreaterThan(10);
      expect(colorDistance(edge, corner)).toBeGreaterThan(10);
    }
  });
});
