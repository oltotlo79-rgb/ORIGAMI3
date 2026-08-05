// 技法の日本語表示名と、手順の警告表示の共有ロジック
// (タイムラインのチップ・コンテキストパネル・警告バッジで使う)。
// 画面に出る文言は日本語で統一する(要件§2)。

import type { TechniqueKind } from "./types";

export const TECHNIQUE_LABEL: Record<TechniqueKind, string> = {
  Simple: "単純折り",
  Pleat: "段折り",
  InsideReverse: "中割り折り",
  OutsideReverse: "かぶせ折り",
  Petal: "花弁折り",
  Squash: "開いてつぶす",
  OpenSink: "沈め折り",
  Swivel: "ひだ寄せ",
  Twist: "ねじり折り",
  Pose: "仕上げの角度",
};

/** セレクトに並べる順(TECHNIQUE_LABELの定義順) */
export const TECHNIQUE_KINDS = Object.keys(TECHNIQUE_LABEL) as TechniqueKind[];

/**
 * 再生の警告文から、手順number(1始まり)に関するものだけを取り出す。
 * Rust側の警告は「手順3の折り線が…」のように手順番号で始まる。
 * 手順1と手順10を取り違えないよう、数字の直後が数字でないことを確かめる。
 */
export function warningsForStep(warnings: string[], number: number): string[] {
  const head = new RegExp(`^手順${number}(?![0-9])`);
  return warnings.filter((w) => head.test(w));
}

/**
 * 複数の出どころの警告を1つにまとめる(同じ文言は1回だけ残す)。
 * 展開図の検査結果には自動再生の警告も合流しているため、途中の手順を選んで
 * 再生し直したときに同じ文言が二重に出るのを防ぐ。
 */
export function uniqueWarnings(...lists: string[][]): string[] {
  return [...new Set(lists.flat())];
}
