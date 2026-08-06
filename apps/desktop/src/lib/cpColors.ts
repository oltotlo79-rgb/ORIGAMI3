// 展開図の色づくり。紙の色をそのまま塗ると、赤い紙の上の赤い山折り線のように
// 折り線が紙に埋もれて見えなくなる(既定の表の色 #ED1C24 と山折り #d43c3c は
// コントラスト比 1.06:1 でほぼ同じ色)。
// そこで紙は「白へ薄めた色」で塗り、どの線種も読めるコントラスト比になるまで
// 薄める。折り紙の慣例(山=赤・谷=青)と3D表示の色は変えない。

import type { EdgeKind } from "./types";
import { hexToRgb } from "./displayPrefs";

export type Rgb = [number, number, number];

/** 線種ごとの色。山=赤・谷=青の慣例を守る(補助線は薄い紙の上でも読める濃さの灰) */
export const EDGE_COLORS: Record<EdgeKind, string> = {
  Border: "#000000",
  Mountain: "#d43c3c",
  Valley: "#3b6fc9",
  Aux: "#7a7a7a",
};

/** 紙の上で線が読めるとみなすコントラスト比の下限(WCAG 1.4.11 と同じ 3:1) */
export const MIN_LINE_CONTRAST = 3;

/** 紙の色の濃さの候補(濃い順)。線が読める範囲でいちばん濃いものを使う */
export const PAPER_TINTS = [0.25, 0.2, 0.15, 0.1] as const;

/** 方眼を紙より少しだけ暗くする割合(折り線より目立たせない) */
const GRID_DARKEN = 0.88;

function toLinear(c: number): number {
  const s = Math.max(0, Math.min(255, c)) / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}

/** 相対輝度(WCAG 2.xの定義。0=黒 〜 1=白) */
export function relativeLuminance(rgb: Rgb): number {
  return (
    0.2126 * toLinear(rgb[0]) + 0.7152 * toLinear(rgb[1]) + 0.0722 * toLinear(rgb[2])
  );
}

/** コントラスト比(1〜21。大きいほど見分けやすい) */
export function contrastRatio(a: Rgb, b: Rgb): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** 白へ薄める(strength=1で元の色のまま、0で白) */
export function mixWhite(rgb: Rgb, strength: number): Rgb {
  const s = Math.max(0, Math.min(1, strength));
  return rgb.map((v) => Math.round(v * s + 255 * (1 - s))) as Rgb;
}

/** 線種の色を[r,g,b]で返す(読めない指定は黒扱い) */
export function edgeRgb(kind: EdgeKind): Rgb {
  return hexToRgb(EDGE_COLORS[kind]) ?? [0, 0, 0];
}

const LINE_RGB: Rgb[] = (Object.keys(EDGE_COLORS) as EdgeKind[]).map(edgeRgb);

/** 紙の色に対して、いちばん読みにくい線種とのコントラスト比 */
export function worstLineContrast(fill: Rgb): number {
  return LINE_RGB.reduce((min, l) => Math.min(min, contrastRatio(fill, l)), Infinity);
}

/**
 * 紙の塗り色を決める。どの線種も MIN_LINE_CONTRAST 以上になる範囲で、
 * いちばん濃い(紙の色が分かる)候補を選ぶ。
 */
export function paperFill(rgb: Rgb): Rgb {
  for (const t of PAPER_TINTS) {
    const fill = mixWhite(rgb, t);
    if (worstLineContrast(fill) >= MIN_LINE_CONTRAST) return fill;
  }
  return mixWhite(rgb, PAPER_TINTS[PAPER_TINTS.length - 1]);
}

/** 方眼の色(紙の塗りを少し暗くしたもの) */
export function gridColor(fill: Rgb): Rgb {
  return fill.map((v) => Math.round(v * GRID_DARKEN)) as Rgb;
}

/**
 * 線の下に敷く縁取り(ハロー)の色。選択中の橙色の帯や方眼の上に線が重なっても
 * 線種の色が読めるようにする。紙が明るければ白、暗ければ黒。
 */
export function haloColor(fill: Rgb): string {
  return relativeLuminance(fill) >= 0.5
    ? "rgba(255, 255, 255, 0.92)"
    : "rgba(24, 24, 28, 0.92)";
}

/** [r,g,b] → canvasの色文字列 */
export function cssRgb(rgb: Rgb): string {
  return `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
}
