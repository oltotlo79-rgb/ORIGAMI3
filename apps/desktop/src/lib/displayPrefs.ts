// 見た目の設定(紙の色・方眼の分割数)を扱う小道具と、画面操作の好みの保管。
//
// 紙の色・方眼の分割数は「作品ごとの設定」なので、EditOp::SetDisplay を通じて
// Document.display に保存する(.ori3ファイルに入り、渡した相手にも同じ見た目で
// 伝わる)。ここでlocalStorageへ覚えることはしない。覚えてしまうと、人から
// もらった作品を開いたときにその作品の色を黙って上書きしてしまうため。
// localStorageに残すのは2D/3Dの分割比・下部パネルの高さ・対称描画・
// ホイール・操作説明の開閉など画面の使い方の好みだけ。

import type { DisplaySettings, SoftSettings } from "./types";
import type { MirrorAxisPreset } from "./mirror";

/** Rust側 Document::new と同じ初期値(赤い表・白い裏・8分割・各防止はオン・たわみはオフ)。
 * 作品をまだ開いていない間の表示に使う */
export const DEFAULT_DISPLAY: DisplaySettings = {
  front_color: [237, 28, 36],
  back_color: [255, 255, 255],
  grid_divisions: 8,
  overlap_prevention_enabled: true,
  penetration_prevention_enabled: true,
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

/** 折り動作中の重なり防止を使うか。項目の無い古い作品も既定のオンで扱う。 */
export function overlapPreventionOf(display: DisplaySettings): boolean {
  return display.overlap_prevention_enabled !== false;
}

/** 角度操作中の食い込み検出を使うか。項目の無い古い作品も既定のオンで扱う。 */
export function penetrationPreventionOf(display: DisplaySettings): boolean {
  return display.penetration_prevention_enabled !== false;
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
export const MAX_DIVISIONS = 1024;

/** 2D区画の幅の割合の既定値と可動範囲(UI-004) */
export const DEFAULT_SPLIT_RATIO = 0.5;
export const MIN_SPLIT_RATIO = 0.2;
export const MAX_SPLIT_RATIO = 0.8;

/**
 * 画面下部の「今できる操作」が、ツールバーを除いた作業領域に占める割合。
 * 1080px級では既定値で約325pxとなり、従来の160pxより説明と主操作が見やすい。
 * 最小ウィンドウ(700px高)でも、下限は従来と同程度の約160px、上限でも
 * 2D/3D側へ約285pxを残す。
 */
export const DEFAULT_CONTEXT_PANEL_RATIO = 0.32;
export const MIN_CONTEXT_PANEL_RATIO = 0.25;
export const MAX_CONTEXT_PANEL_RATIO = 0.55;

const STORAGE_KEY = "origami3.prefs";

/** 2D展開図で修飾キーを押していないときのホイール動作。 */
export type WheelBehavior = "scroll" | "zoom";

/** 端末ごとに選べる画面デザイン。作品ファイルには含めない。 */
export const UI_THEMES = ["pop", "simple", "japanese", "modern", "classic"] as const;
export type UiTheme = (typeof UI_THEMES)[number];

/** 古い保存値や手編集されたlocalStorageから、安全なテーマだけを受け入れる。 */
export function isUiTheme(value: unknown): value is UiTheme {
  return typeof value === "string" && (UI_THEMES as readonly string[]).includes(value);
}

/** localStorageへ覚えておく画面の好み。作品の中身(紙の色・方眼)は
 * ここには入らない(作品ファイル側に保存する) */
export interface Prefs {
  splitRatio: number;
  /** 下部の「今できる操作」の高さ。作品ではなく画面の使い方として覚える。 */
  contextPanelRatio: number;
  /** 左右対称に線を引くか(CPE-010)。次に起動しても同じ描き方に戻る */
  mirrorDraw: boolean;
  /** 紙の中心線のどちらを対称操作の基準にするか。作品内で選んだ線は保存しない。 */
  mirrorAxis: MirrorAxisPreset;
  /** 3Dで紙を引くとき、対称の相手の折り線も同時に動かすか(UI-007)。
   * 折り紙の作品はほとんどが左右対称で、鶴の羽のように両側を一緒に開くのが
   * 自然なので既定はオン。対称の相手が無い折り線では自動的に1本だけになる */
  pullMirror: boolean;
  /** 2D展開図のホイール操作。作品ではなく端末ごとの操作の好みとして覚える。 */
  wheelBehavior: WheelBehavior;
  /** 画面全体のデザイン。作品には保存せず、この端末だけに覚える。 */
  uiTheme: UiTheme;
  /** 下部の詳しい操作方法を開いているか。初回は文字を増やさないよう畳む。 */
  contextHelpExpanded: boolean;
  /** 3Dビューの詳しいマウス操作を開いているか。初回は畳む。 */
  viewerHintExpanded: boolean;
  /** 2D展開図の詳しいホイール操作を開いているか。初回は畳む。 */
  cpHelpExpanded: boolean;
  /** 紙の丸み・膨らみについての詳しい説明を開いているか。初回は畳む。 */
  paperHelpExpanded: boolean;
  /** 紙の表裏の色見本を開いているか。初回は畳み、現在色だけ小さく残す。 */
  paperColorExpanded: boolean;
}

export const DEFAULT_PREFS: Prefs = {
  splitRatio: DEFAULT_SPLIT_RATIO,
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

/** 下部パネルの割合を、上下どちらの区画も使える範囲へ収める。 */
export function clampContextPanelRatio(r: number): number {
  if (!Number.isFinite(r)) return DEFAULT_CONTEXT_PANEL_RATIO;
  return Math.max(MIN_CONTEXT_PANEL_RATIO, Math.min(MAX_CONTEXT_PANEL_RATIO, r));
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
      contextPanelRatio: clampContextPanelRatio(
        saved.contextPanelRatio ?? DEFAULT_CONTEXT_PANEL_RATIO,
      ),
      mirrorDraw: saved.mirrorDraw === true,
      // 選んだ作品内の線や不正な値は保存対象外。旧版も縦の中心線へ戻す。
      mirrorAxis:
        saved.mirrorAxis === "paperHorizontal"
          ? "paperHorizontal"
          : "paperVertical",
      // 保存が無い(初めての起動・古い保存)ときは既定のオンのままにする
      pullMirror: saved.pullMirror !== false,
      // 古い保存には項目が無いので、一般的な描画ソフトと同じスクロールを既定にする
      wheelBehavior: saved.wheelBehavior === "zoom" ? "zoom" : "scroll",
      // テーマ未保存の旧版・未知の値は、従来デザインのポップへ戻す
      uiTheme: isUiTheme(saved.uiTheme) ? saved.uiTheme : "pop",
      // 初回・開閉項目を持たない旧版では文字を増やさないよう畳む。
      // 利用者が明示的に開いた項目だけ、次回も開いたままにする。
      contextHelpExpanded: saved.contextHelpExpanded === true,
      viewerHintExpanded: saved.viewerHintExpanded === true,
      cpHelpExpanded: saved.cpHelpExpanded === true,
      paperHelpExpanded: saved.paperHelpExpanded === true,
      paperColorExpanded: saved.paperColorExpanded === true,
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
