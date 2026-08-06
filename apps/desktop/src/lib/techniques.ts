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
 * 「技法」ツールのサブメニューに出す、選ぶだけで折れる技法。
 * ここに無い技法(単純折り・仕上げの角度)は技法ツールの対象外。
 * Rust側 `ori3-layers::techniques` の実装と対応する。
 */
export const SUPPORTED_TECHNIQUES: {
  kind: TechniqueKind;
  /** ツールレールのサブメニュー用の短い名前 */
  short: string;
  title: string;
}[] = [
  {
    kind: "Pleat",
    short: "段",
    title:
      "段折り: 平行な2本の折り線で山・谷を交互に折ります。紙の上をドラッグして1本目の折り線を引き、段の幅を指定してください",
  },
  {
    kind: "InsideReverse",
    short: "中割り",
    title:
      "中割り折り: フラップの先端を層の間へ折り込みます。重なった層をクリックして選び、先端を折り返す線をドラッグしてください",
  },
  {
    kind: "OutsideReverse",
    short: "かぶせ",
    title:
      "かぶせ折り: フラップの先端を外側からかぶせます。重なった層をクリックして選び、先端を折り返す線をドラッグしてください",
  },
  {
    kind: "Squash",
    short: "つぶす",
    title:
      "開いてつぶす: フラップを開いて平らにつぶします。重なった層をクリックして選び、開く折り目(背)に重なる中心線をドラッグしてください。基準点はつぶす方向を指します",
  },
  {
    kind: "Petal",
    short: "花弁",
    title:
      "花弁折り: フラップの先端を持ち上げ、両側の縁を中心線に沿わせます。重なった層をクリックして選び、先端と行き先を通る中心線をドラッグしてください。基準点は持ち上げる先端の位置を指します",
  },
  {
    kind: "OpenSink",
    short: "沈め",
    title:
      "沈め折り: フラップの先端(角)を袋の内側へ押し込みます。沈める折り線をドラッグし、基準点で押し込む先端側を指してください。層を選ばなければ先端側の全ての層が沈みます",
  },
];

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
