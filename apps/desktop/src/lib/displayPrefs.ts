// 見た目の設定(紙の色・方眼の分割数)と2D/3Dの分割比の保管。
// Tauriコマンドは13個で打ち止めで、色・方眼を保存する口が無いため、
// これらは「その利用者の見え方の好み」として画面側だけで持ち、
// localStorageへ覚えておく(次に起動しても同じ見え方に戻る)。

import type { DisplaySettings } from "./types";

/** Rust側 Document::new と同じ初期値(赤い表・白い裏・8分割) */
export const DEFAULT_DISPLAY: DisplaySettings = {
  front_color: [237, 28, 36],
  back_color: [255, 255, 255],
  grid_divisions: 8,
};

/** 方眼の分割数の下限・上限(CPE-003) */
export const MIN_DIVISIONS = 2;
export const MAX_DIVISIONS = 64;

/** 2D区画の幅の割合の既定値と可動範囲(UI-004) */
export const DEFAULT_SPLIT_RATIO = 0.5;
export const MIN_SPLIT_RATIO = 0.2;
export const MAX_SPLIT_RATIO = 0.8;

const STORAGE_KEY = "origami3.prefs";

export interface Prefs {
  display: DisplaySettings;
  splitRatio: number;
}

export const DEFAULT_PREFS: Prefs = {
  display: DEFAULT_DISPLAY,
  splitRatio: DEFAULT_SPLIT_RATIO,
};

/** 方眼の分割数を範囲内の整数に丸める(入力が数でなければ既定値) */
export function clampDivisions(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_DISPLAY.grid_divisions;
  return Math.max(MIN_DIVISIONS, Math.min(MAX_DIVISIONS, Math.round(n)));
}

/** 分割比を範囲内に丸める(入力が数でなければ既定値) */
export function clampSplitRatio(r: number): number {
  if (!Number.isFinite(r)) return DEFAULT_SPLIT_RATIO;
  return Math.max(MIN_SPLIT_RATIO, Math.min(MAX_SPLIT_RATIO, r));
}

/** [r,g,b] → "#rrggbb"(色見本の入力欄が使う形) */
export function rgbToHex(rgb: [number, number, number]): string {
  const hex = (v: number) =>
    Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0");
  return `#${hex(rgb[0])}${hex(rgb[1])}${hex(rgb[2])}`;
}

/** "#rrggbb" → [r,g,b]。読めない文字列なら null */
export function hexToRgb(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = Number.parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** 覚えておく先(localStorage)。テストでは差し替える */
export interface StorageLike {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
}

/** 使える保管先を返す(無い環境ではnull。覚えられなくても操作は続けられる) */
export function defaultStorage(): StorageLike | null {
  const s = globalThis.localStorage as StorageLike | undefined;
  return typeof s?.getItem === "function" && typeof s.setItem === "function"
    ? s
    : null;
}

/** 保存済みの好みを読む(壊れていれば既定値。保管先が無い環境も可) */
export function loadPrefs(storage: StorageLike | null = defaultStorage()): Prefs {
  try {
    const raw = storage?.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_PREFS;
    const saved = JSON.parse(raw) as Partial<Prefs>;
    const d = saved.display;
    return {
      display: {
        front_color: d?.front_color ?? DEFAULT_DISPLAY.front_color,
        back_color: d?.back_color ?? DEFAULT_DISPLAY.back_color,
        grid_divisions: clampDivisions(
          d?.grid_divisions ?? DEFAULT_DISPLAY.grid_divisions,
        ),
      },
      splitRatio: clampSplitRatio(saved.splitRatio ?? DEFAULT_SPLIT_RATIO),
    };
  } catch {
    return DEFAULT_PREFS;
  }
}

/** 好みを覚えておく(保存できなくても操作は止めない) */
export function savePrefs(
  prefs: Prefs,
  storage: StorageLike | null = defaultStorage(),
): void {
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify(prefs));
  } catch {
    // 保存できない環境でも見た目の変更自体は効く
  }
}
