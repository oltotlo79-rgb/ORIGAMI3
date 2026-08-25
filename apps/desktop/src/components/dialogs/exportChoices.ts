import type { ExportKind } from "../../lib/types";

/** 種類ごとの表示名・拡張子・ひとこと説明。needsStepsは折り手順が要るもの */
export type ExportChoice = {
  kind: ExportKind;
  label: string;
  ext: string;
  hint: string;
  needsSteps?: boolean;
};

export const EXPORT_CHOICES: ExportChoice[] = [
  {
    kind: "CpSvg",
    label: "展開図(SVG)",
    ext: "svg",
    hint: "紙の実物大(mm)で保存します。いくら拡大してもぼやけず、印刷向きです。",
  },
  {
    kind: "CpPng",
    label: "展開図(PNG)",
    ext: "png",
    hint: "写真と同じ形式です。そのまま画面で見たり貼り付けたりできます。",
  },
  {
    kind: "DiagramPdf",
    label: "折り図(PDF)",
    ext: "pdf",
    hint:
      "折る手順を1コマずつ絵にして、A4の紙に1ページ6コマ並べます。" +
      "1ページ目は表紙(できあがりの形)です。そのまま印刷して使えます。",
    needsSteps: true,
  },
  {
    kind: "DiagramSvg",
    label: "折り図(ページごとのSVG)",
    ext: "svg",
    hint:
      "折り図をページごとの画像にします。選んだ場所に「-01」「-02」…と" +
      "番号を足したファイルがページの数だけ並びます。",
    needsSteps: true,
  },
];
