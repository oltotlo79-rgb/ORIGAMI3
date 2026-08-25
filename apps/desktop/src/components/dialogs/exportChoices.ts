import { FOLD_FILE_EXCHANGE_READY } from "../../lib/foldFileExchange";
import type {
  DocumentExportKind,
  ExportKind,
  FoldExportKind,
} from "../../lib/types";

/** 種類ごとの表示名・拡張子・ひとこと説明。needsStepsは折り手順が要るもの */
export type ExportChoice<Kind extends DocumentExportKind = DocumentExportKind> = {
  kind: Kind;
  label: string;
  ext: string;
  hint: string;
  needsSteps?: boolean;
};

const BASE_EXPORT_CHOICES: ExportChoice<ExportKind>[] = [
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

export const FOLD_EXPORT_CHOICE: ExportChoice<FoldExportKind> = {
  kind: "FoldJson",
  label: "ほかの折り紙ソフトのファイル",
  ext: "fold",
  hint:
    "折り目や折る手順を、対応しているほかの折り紙ソフトで使える形にします。" +
    "書き出せない内容があるときは、理由をお知らせします。",
};

export function exportChoicesForReadiness(
  ready: false,
): ExportChoice<ExportKind>[];
export function exportChoicesForReadiness(
  ready: true,
): ExportChoice<DocumentExportKind>[];
export function exportChoicesForReadiness(
  ready: boolean,
): ExportChoice<DocumentExportKind>[];
export function exportChoicesForReadiness(
  ready: boolean,
): ExportChoice<DocumentExportKind>[] {
  return ready
    ? [...BASE_EXPORT_CHOICES, FOLD_EXPORT_CHOICE]
    : [...BASE_EXPORT_CHOICES];
}

export function exportDialogTitleForReadiness(ready: boolean): string {
  return ready ? "作品を書き出す" : "展開図・折り図を書き出す";
}

// falseの間はkindが既存ExportKindへ狭まり、store接続なしにFoldJsonを選べない。
export const EXPORT_CHOICES = exportChoicesForReadiness(
  FOLD_FILE_EXCHANGE_READY,
);
export const EXPORT_DIALOG_TITLE = exportDialogTitleForReadiness(
  FOLD_FILE_EXCHANGE_READY,
);
