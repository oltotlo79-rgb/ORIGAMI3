// 画面の見た目の土台(App.css の設計トークン)の検査。
// 1) 使うトークンが定義されていること、2) 文字色が背景に対して読めること、
// 3) 効き色が展開図の慣例色(山=赤・谷=青)と取り違えられないこと を確かめる。
// 数値の出どころは lib/cpColors のコントラスト計算(展開図の線と同じ物差し)。

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { EDGE_COLORS, contrastRatio, type Rgb } from "./cpColors";
import { hexToRgb } from "./displayPrefs";

// vitest は .css の取り込みを空にするため、App.css は文字として直に読む
const css = readFileSync(new URL("../App.css", import.meta.url), "utf8");

/** :root に書いた「--名前: 値」を集める */
function tokens(): Map<string, string> {
  const root = /:root\s*\{([\s\S]*?)\}/.exec(css);
  expect(root).not.toBeNull();
  const map = new Map<string, string>();
  for (const m of (root as RegExpExecArray)[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    map.set(m[1], m[2].trim());
  }
  return map;
}

/** トークン名 → [r,g,b](色でない値は例外にする) */
function rgbOf(name: string): Rgb {
  const raw = tokens().get(name);
  const rgb = raw === undefined ? null : hexToRgb(raw);
  if (rgb === null) throw new Error(`色のトークンではありません: ${name} = ${raw}`);
  return rgb;
}

/** 文字色と背景色の組(すべて4.5:1以上が必要) */
const TEXT_PAIRS: [string, string][] = [
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
];

describe("設計トークン", () => {
  it("色・余白・角丸・影・文字の大きさの段階が定義されている", () => {
    const t = tokens();
    const required = [
      "--color-bg",
      "--color-surface",
      "--color-border",
      "--color-accent",
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
    for (const name of required) expect(t.has(name), name).toBe(true);
  });

  it("使っているトークンはすべて :root で定義されている", () => {
    const t = tokens();
    for (const m of css.matchAll(/var\((--[\w-]+)\)/g)) {
      expect(t.has(m[1]), m[1]).toBe(true);
    }
  });

  it("文字は背景に対してコントラスト比4.5:1以上", () => {
    for (const [fg, bg] of TEXT_PAIRS) {
      expect(contrastRatio(rgbOf(fg), rgbOf(bg)), `${fg} / ${bg}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("面で塗る効き色・警告色の上の白文字も4.5:1以上", () => {
    const white: Rgb = [255, 255, 255];
    for (const name of ["--color-accent", "--color-accent-strong", "--color-danger", "--color-warn-badge"]) {
      expect(contrastRatio(white, rgbOf(name)), name).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("効き色は展開図の山=赤・谷=青と取り違えない色にする", () => {
    const accent = rgbOf("--color-accent");
    for (const kind of ["Mountain", "Valley"] as const) {
      const line = hexToRgb(EDGE_COLORS[kind]) as Rgb;
      const dist = Math.hypot(...accent.map((v, i) => v - line[i]));
      expect(dist, kind).toBeGreaterThan(80);
    }
  });
});
