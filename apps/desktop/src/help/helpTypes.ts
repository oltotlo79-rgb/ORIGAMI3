/**
 * アプリ内ヘルプと取扱説明書PDFが共有する、表示方法に依存しない内容の型。
 * React要素やHTMLを本文へ入れず、PDF側でも同じ順序で機械的に組版できるようにする。
 */

export type HelpChapterId =
  | "overview"
  | "workspace"
  | "new-paper"
  | "crease-pattern"
  | "fold"
  | "angles"
  | "three-dimensional"
  | "techniques"
  | "timeline"
  | "proposal"
  | "save-export"
  | "troubleshooting"
  | "shortcuts";

export type HelpDiagramId =
  | "overview-flow"
  | "workspace-four-areas"
  | "new-paper-settings"
  | "crease-tools"
  | "fold-flow"
  | "angle-controls"
  | "three-dimensional-controls"
  | "technique-cards"
  | "timeline-flow"
  | "proposal-wizard"
  | "save-export-flow"
  | "troubleshooting-flow"
  | "shortcut-map";

export interface HelpParagraphBlock {
  type: "paragraph";
  text: string;
}

export interface HelpHeadingBlock {
  type: "heading";
  text: string;
}

export interface HelpBulletListBlock {
  type: "bulletList";
  title?: string;
  items: readonly string[];
}

export interface HelpStep {
  title: string;
  description: string;
}

export interface HelpStepsBlock {
  type: "steps";
  title: string;
  items: readonly HelpStep[];
}

export interface HelpCalloutBlock {
  type: "callout";
  tone: "tip" | "note" | "warning";
  title: string;
  text: string;
}

export interface HelpFigureBlock {
  type: "figure";
  diagramId: HelpDiagramId;
  /** 取扱説明書へ追加表示する docs/manual/assets/ 配下の画像ファイル名。 */
  image?: string;
}

/** PDFだけに載せる実画面。画像本体をアプリのビルドへ含めない。 */
export interface HelpScreenshotBlock {
  type: "screenshot";
  /** docs/manual/assets/ 直下のPNGファイル名。 */
  image: string;
  caption: string;
}

export interface HelpTableBlock {
  type: "table";
  title?: string;
  columns: readonly string[];
  rows: readonly (readonly string[])[];
}

export type HelpBlock =
  | HelpParagraphBlock
  | HelpHeadingBlock
  | HelpBulletListBlock
  | HelpStepsBlock
  | HelpCalloutBlock
  | HelpFigureBlock
  | HelpScreenshotBlock
  | HelpTableBlock;

export interface HelpChapter {
  id: HelpChapterId;
  number: number;
  title: string;
  summary: string;
  blocks: readonly HelpBlock[];
}

/** SVGは外部画像に依存しない完全な文字列として保持する。 */
export interface HelpDiagram {
  id: HelpDiagramId;
  title: string;
  alt: string;
  svg: string;
}
