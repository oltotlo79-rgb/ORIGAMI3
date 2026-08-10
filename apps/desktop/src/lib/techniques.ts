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
 * Simpleは通常の単純折りではなく、汎用の「層操作」入口として再利用する。
 * 仕上げの角度は技法ツールの対象外。
 * Rust側 `ori3-layers::techniques` の実装と対応する。
 */
export const SUPPORTED_TECHNIQUES: {
  kind: TechniqueKind;
  /** ツールレールのサブメニュー用の短い名前 */
  short: string;
  title: string;
}[] = [
  {
    kind: "Simple",
    short: "層",
    title:
      "層操作: 既存折り目をクリックして選択層を開閉する、紙を動かさず重なり順だけ変える、選択層だけ山谷を反転する操作を組み合わせます。複数の部分を1手として同時に適用できます",
  },
  {
    kind: "Pleat",
    short: "段",
    title:
      "段折り: 平行な2本の折り線で山・谷を交互に折ります。紙の上をドラッグして1本目の折り線を引き、段の幅を指定してください。Ctrl+クリックで段の行き先を直接指定できます",
  },
  {
    kind: "InsideReverse",
    short: "中割り",
    title:
      "中割り折り: フラップの先端を層の間へ折り込みます。重なった層をクリックして選び、先端を折り返す線をドラッグしてください。Ctrl+クリックで先端の行き先を指定できます",
  },
  {
    kind: "OutsideReverse",
    short: "かぶせ",
    title:
      "かぶせ折り: フラップの先端を外側からかぶせます。重なった層をクリックして選び、先端を折り返す線をドラッグしてください。Ctrl+クリックで先端の行き先を指定できます",
  },
  {
    kind: "Squash",
    short: "つぶす",
    title:
      "開いてつぶす: フラップを開いて平らにつぶします。重なった層をクリックして選び、開く折り目(背)に重なる中心線をドラッグしてください。Ctrl+クリックの基準点でつぶす方向を直接指定できます",
  },
  {
    kind: "Petal",
    short: "花弁",
    title:
      "花弁折り: フラップの先端を持ち上げ、両側の縁を中心線に沿わせます。重なった層をクリックして選び、先端と行き先を通る中心線をドラッグしてください。Ctrl+クリックの基準点で持ち上げる先端を直接指定できます",
  },
  {
    kind: "OpenSink",
    short: "沈め",
    title:
      "沈め折り: フラップの先端(角)を袋の内側へ押し込みます。沈める折り線をドラッグし、Ctrl+クリックの基準点で押し込む先端側を指定できます。層を選ばなければ先端側の全ての層が沈みます",
  },
  {
    kind: "Swivel",
    short: "ひだ寄せ",
    title:
      "ひだ寄せ: 紙の縁を支点のまわりに寄せ、余った紙をひだにします。寄せたい縁が乗っている基準線をドラッグし、Ctrl+クリックの基準点で寄せる先を直接指定できます",
  },
  {
    kind: "Twist",
    short: "ねじり",
    title:
      "ねじり折り: 中央の多角形を回し、周りにひだを作ります。中央の形の角を順にクリックしてください(3つ以上。辺の数も長さも自由)。Ctrl+クリックで中心、Shift+クリックで対象層、Backspaceで1つ戻す。1辺をドラッグすると正多角形になります",
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
 * 複数の出どころの警告を1つにまとめ(同じ文言は1回だけ残す)、
 * 画面へ出す形にする。展開図の検査結果には自動再生の警告も合流しているため、
 * 途中の手順を選んで再生し直したときに同じ文言が二重に出るのを防ぐ。
 * まとめたあと [`withFixHint`] で「どう直せばよいか」を書き添える。
 */
export function uniqueWarnings(...lists: string[][]): string[] {
  return withFixHint([...new Set(lists.flat())]);
}

/**
 * 警告に「どう直せばよいか」を短く足す(設計原則3b: できない理由を短い日本語で)。
 *
 * 手順の途中へ折りを挟んだとき、後続の手順は飛ばされない(挟んだ折りは展開図へ
 * 辺を足すだけなので折り線は必ず見つかる)。代わりに、矛盾した指定は
 * 「形が求まりませんでした/収束していません」として最後の手順に付く。
 * 何を直せばよいかが分からないままだったので、直し方をここで添える。
 * アプリが勝手に手順を書き換えることはしない(利用者の意図と違う形になるため。
 * 設計原則: 止めずに警告)。
 */
const FIX_HINTS: [RegExp, string][] = [
  [
    /折り線が見つからないため/,
    "展開図にその折り線を引き直すか、この手順を削除してください",
  ],
  [
    /折り線の一部が見つかりません/,
    "消えた折り線を展開図に引き直すと、この手順が元どおり折れます",
  ],
  [
    /求まりませんでした|収束していません/,
    "前後の手順の指定が食い違っています。挟んだ折りを取り消すか、合わなくなった手順を選んで削除・移動してください",
  ],
  [
    /この層を飛ばしました/,
    "重なり順の目印が今の紙に見つかりません。この手順を折り直すと重なり順が付け直されます",
  ],
];

export function withFixHint(warnings: string[]): string[] {
  return warnings.map((w) => {
    const hint = FIX_HINTS.find(([re]) => re.test(w))?.[1];
    return hint === undefined || w.includes(hint) ? w : `${w}。${hint}`;
  });
}
