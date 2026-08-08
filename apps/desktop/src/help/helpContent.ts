import { overviewChapter } from "./chapters/overview";
import { workspaceChapter } from "./chapters/workspace";
import { newPaperChapter } from "./chapters/newPaper";
import { creasePatternChapter } from "./chapters/creasePattern";
import { foldChapter } from "./chapters/fold";
import { anglesChapter } from "./chapters/angles";
import { threeDimensionalChapter } from "./chapters/threeDimensional";
import { techniquesChapter } from "./chapters/techniques";
import { timelineChapter } from "./chapters/timeline";
import { proposalChapter } from "./chapters/proposal";
import { saveExportChapter } from "./chapters/saveExport";
import { troubleshootingChapter } from "./chapters/troubleshooting";
import { shortcutsChapter } from "./chapters/shortcuts";
import type { HelpBlock, HelpChapter, HelpChapterId } from "./helpTypes";

/**
 * ヘルプの唯一の章配列。PDF側はこの配列を順に走査し、block.typeごとに組版する。
 * figureはhelpDiagrams.tsのHELP_DIAGRAMSをdiagramIdで引いて配置する。
 */
export const HELP_CHAPTERS = [
  overviewChapter,
  workspaceChapter,
  newPaperChapter,
  creasePatternChapter,
  foldChapter,
  anglesChapter,
  threeDimensionalChapter,
  techniquesChapter,
  timelineChapter,
  proposalChapter,
  saveExportChapter,
  troubleshootingChapter,
  shortcutsChapter,
] as const satisfies readonly HelpChapter[];

export const DEFAULT_HELP_CHAPTER_ID: HelpChapterId = HELP_CHAPTERS[0].id;

export function helpBlockText(block: HelpBlock): string {
  switch (block.type) {
    case "paragraph":
    case "heading":
      return block.text;
    case "bulletList":
      return [block.title, ...block.items].filter(Boolean).join(" ");
    case "steps":
      return [
        block.title,
        ...block.items.flatMap((item) => [item.title, item.description]),
      ].join(" ");
    case "callout":
      return `${block.title} ${block.text}`;
    case "table":
      return [block.title, ...block.columns, ...block.rows.flat()].filter(Boolean).join(" ");
    case "figure":
      return "";
  }
}

/** 章題と本文を単純な文字列一致で探すための、表示に依存しない検索文字列。 */
export function helpChapterSearchText(chapter: HelpChapter): string {
  return [chapter.title, chapter.summary, ...chapter.blocks.map(helpBlockText)].join(" ");
}
