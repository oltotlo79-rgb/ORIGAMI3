// 見た目の設定(紙の色・方眼の分割数)を扱う小道具と、2D/3Dの分割比の保管。
//
// 紙の色・方眼の分割数は「作品ごとの設定」なので、EditOp::SetDisplay を通じて
// Document.display に保存する(.ori3ファイルに入り、渡した相手にも同じ見た目で
// 伝わる)。ここでlocalStorageへ覚えることはしない。覚えてしまうと、人から
// もらった作品を開いたときにその作品の色を黙って上書きしてしまうため。
// localStorageに残すのは2D/3Dの分割比だけ(作品の中身ではなく画面の使い方の好み)。

import type { DisplaySettings, SoftSettings } from "./types";

/** Rust側 Document::new と同じ初期値(赤い表・白い裏・8分割・たわみはオフ)。
 * 作品をまだ開いていない間の表示に使う */
export const DEFAULT_DISPLAY: DisplaySettings = {
  front_color: [237, 28, 36],
  back_color: [255, 255, 255],
  grid_divisions: 8,
  soft_enabled: false,
  soft_stiffness: 0.5,
  soft_pressure: 0,
};

/** 面の分割の細かさ(1辺 2^2 = 4等分)。細かすぎると1コマ16msに入らないので
 * 画面からは変えられない固定値にし、大きな展開図ではRust側が自動で落とす */
export const SOFT_SUBDIVISION = 2;
/** たわみの反復回数(決定性のため固定) */
export const SOFT_ITERATIONS = 20;

/** 0.0〜1.0に丸める(入力が数でなければ既定値) */
export function clampUnit(v: number, fallback: number): number {
  if (!Number.isFinite(v)) return fallback;
  return Math.max(0, Math.min(1, v));
}

/**
 * 作品の見た目の設定から、たわみ計算へ渡す指定を組み立てる(SIM-015)。
 * 古い作品ファイルには項目が無いので既定値で埋める。
 */
export function softOf(display: DisplaySettings): SoftSettings {
  return {
    enabled: display.soft_enabled === true,
    subdivision: SOFT_SUBDIVISION,
    stiffness: clampUnit(display.soft_stiffness ?? 0.5, 0.5),
    pressure: clampUnit(display.soft_pressure ?? 0, 0),
    iterations: SOFT_ITERATIONS,
  };
}

/** 方眼の分割数の下限・上限(CPE-003) */
export const MIN_DIVISIONS = 2;
export const MAX_DIVISIONS = 64;

/** 2D区画の幅の割合の既定値と可動範囲(UI-004) */
export const DEFAULT_SPLIT_RATIO = 0.5;
export const MIN_SPLIT_RATIO = 0.2;
export const MAX_SPLIT_RATIO = 0.8;

const STORAGE_KEY = "origami3.prefs";

/** localStorageへ覚えておく画面の好み。作品の中身(紙の色・方眼)は
 * ここには入らない(作品ファイル側に保存する) */
export interface Prefs {
  splitRatio: number;
  /** 左右対称に線を引くか(CPE-010)。次に起動しても同じ描き方に戻る */
  mirrorDraw: boolean;
  /** 3Dで紙を引くとき、対称の相手の折り線も同時に動かすか(UI-007)。
   * 折り紙の作品はほとんどが左右対称で、鶴の羽のように両側を一緒に開くのが
   * 自然なので既定はオン。対称の相手が無い折り線では自動的に1本だけになる */
  pullMirror: boolean;
}

export const DEFAULT_PREFS: Prefs = {
  splitRatio: DEFAULT_SPLIT_RATIO,
  mirrorDraw: false,
  pullMirror: true,
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
    return {
      splitRatio: clampSplitRatio(saved.splitRatio ?? DEFAULT_SPLIT_RATIO),
      mirrorDraw: saved.mirrorDraw === true,
      // 保存が無い(初めての起動・古い保存)ときは既定のオンのままにする
      pullMirror: saved.pullMirror !== false,
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
