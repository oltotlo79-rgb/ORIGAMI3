import { hexToRgb, rgbToHex } from "./displayPrefs";

export interface HsvColor {
  /** 色相。整数の0〜359度。 */
  h: number;
  /** 彩度。整数の0〜100%。 */
  s: number;
  /** 明度。整数の0〜100%。 */
  v: number;
}

type RgbColor = [number, number, number];

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}

function finiteOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

function normalizeHue(hue: number): number {
  const finiteHue = finiteOr(hue, 0);
  return ((finiteHue % 360) + 360) % 360;
}

/** RGB各成分を0〜255へ収め、色選択UI用の整数HSVへ変換する。 */
export function rgbToHsv(rgb: RgbColor): HsvColor {
  const [red, green, blue] = rgb.map((channel) =>
    clamp(finiteOr(channel, 0), 0, 255) / 255,
  );
  const maximum = Math.max(red, green, blue);
  const minimum = Math.min(red, green, blue);
  const delta = maximum - minimum;

  let hue = 0;
  if (delta !== 0) {
    if (maximum === red) {
      hue = 60 * (((green - blue) / delta) % 6);
    } else if (maximum === green) {
      hue = 60 * ((blue - red) / delta + 2);
    } else {
      hue = 60 * ((red - green) / delta + 4);
    }
  }

  const saturation = maximum === 0 ? 0 : delta / maximum;
  return {
    h: Math.round(normalizeHue(hue)) % 360,
    s: Math.round(saturation * 100),
    v: Math.round(maximum * 100),
  };
}

/** HSVを入力範囲へ正規化し、描画・保存に使う整数RGBへ変換する。 */
export function hsvToRgb(hsv: HsvColor): RgbColor {
  const hue = normalizeHue(hsv.h);
  const saturation = clamp(finiteOr(hsv.s, 0), 0, 100) / 100;
  const value = clamp(finiteOr(hsv.v, 0), 0, 100) / 100;
  const chroma = value * saturation;
  const sector = hue / 60;
  const intermediate = chroma * (1 - Math.abs((sector % 2) - 1));

  let red = 0;
  let green = 0;
  let blue = 0;
  if (sector < 1) {
    red = chroma;
    green = intermediate;
  } else if (sector < 2) {
    red = intermediate;
    green = chroma;
  } else if (sector < 3) {
    green = chroma;
    blue = intermediate;
  } else if (sector < 4) {
    green = intermediate;
    blue = chroma;
  } else if (sector < 5) {
    red = intermediate;
    blue = chroma;
  } else {
    red = chroma;
    blue = intermediate;
  }

  const match = value - chroma;
  return [red, green, blue].map((channel) =>
    Math.round((channel + match) * 255),
  ) as RgbColor;
}

/** 6桁の16進色をHSVへ変換する。不正な文字列はnull。 */
export function hexToHsv(hex: string): HsvColor | null {
  const rgb = hexToRgb(hex);
  return rgb === null ? null : rgbToHsv(rgb);
}

/** HSVを小文字の `#rrggbb` へ変換する。 */
export function hsvToHex(hsv: HsvColor): string {
  return rgbToHex(hsvToRgb(hsv));
}
