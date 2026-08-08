/**
 * ヘルプと取扱説明書PDFの共通入口。
 * PDF生成側はHELP_CHAPTERSを順に組版し、figureブロックのdiagramIdを
 * HELP_DIAGRAMSで解決する。UI固有のコンポーネントやCSSには依存しない。
 */
export {
  DEFAULT_HELP_CHAPTER_ID,
  HELP_CHAPTERS,
  helpBlockText,
  helpChapterSearchText,
} from "./helpContent";
export { HELP_DIAGRAMS } from "./helpDiagrams";
export type {
  HelpBlock,
  HelpChapter,
  HelpChapterId,
  HelpDiagram,
  HelpDiagramId,
  HelpStep,
} from "./helpTypes";
