import type {
  HelpBlock,
  HelpChapter,
  HelpDiagram,
  HelpDiagramId,
  HelpScreenshotBlock,
} from "./helpTypes";

export const MANUAL_IMAGE_DIAGRAM_IDS = [
  "workspace-four-areas",
  "new-paper-settings",
  "crease-tools",
  "angle-controls",
  "three-dimensional-controls",
  "technique-cards",
  "timeline-flow",
] as const satisfies readonly HelpDiagramId[];

export type ManualExportDiagram = Pick<HelpDiagram, "id" | "title" | "alt" | "svg">;

export interface ManualExportContent {
  chapters: readonly HelpChapter[];
  diagrams: Readonly<Partial<Record<HelpDiagramId, ManualExportDiagram>>>;
}

const manualImageIds = new Set<HelpDiagramId>(MANUAL_IMAGE_DIAGRAM_IDS);

function assertManualImage(diagram: HelpDiagram): string {
  const expected = `figure-${diagram.id}.png`;
  if (diagram.manualImage !== expected || !/^figure-[a-z0-9-]+\.png$/.test(expected)) {
    throw new Error(
      `図 ${diagram.id} の説明書用画像は docs/manual/assets/ 直下の ${expected} にしてください。`,
    );
  }
  return expected;
}

/**
 * 共通ヘルプをPDF向けのブロック列へ変換する。
 * 実画面化した図は注釈付き画像、その直後に元の全画面を並べる。
 */
export function buildManualExportContent(
  chapters: readonly HelpChapter[],
  diagrams: Readonly<Record<HelpDiagramId, HelpDiagram>>,
): ManualExportContent {
  const convertedCounts = new Map<HelpDiagramId, number>();

  const manualChapters = chapters.map((chapter): HelpChapter => {
    const blocks = chapter.blocks.flatMap((block): readonly HelpBlock[] => {
      if (block.type !== "figure") return [block];

      const diagram = diagrams[block.diagramId];
      if (!diagram || diagram.id !== block.diagramId) {
        throw new Error(`図 ${block.diagramId} を解決できません。`);
      }

      if (!manualImageIds.has(diagram.id)) {
        if (diagram.manualImage !== undefined) {
          throw new Error(`対象外の図 ${diagram.id} に manualImage が指定されています。`);
        }
        return [block];
      }

      const annotatedImage = assertManualImage(diagram);
      if (!block.image || !/^screen-[a-z0-9-]+\.png$/.test(block.image)) {
        throw new Error(`図 ${diagram.id} には元の全画面PNGが必要です。`);
      }
      convertedCounts.set(diagram.id, (convertedCounts.get(diagram.id) ?? 0) + 1);

      const replacement: readonly HelpScreenshotBlock[] = [
        {
          type: "screenshot",
          image: annotatedImage,
          caption: `${diagram.title}：${diagram.alt}`,
        },
        {
          type: "screenshot",
          image: block.image,
          caption: "図で示した位置を画面全体で確認できます。",
        },
      ];
      return replacement;
    });
    return { ...chapter, blocks };
  });

  for (const id of MANUAL_IMAGE_DIAGRAM_IDS) {
    assertManualImage(diagrams[id]);
    if (convertedCounts.get(id) !== 1) {
      throw new Error(`図 ${id} はPDF用ブロックへ1回だけ変換してください。`);
    }
  }

  const manualDiagrams: Partial<Record<HelpDiagramId, ManualExportDiagram>> = {};
  for (const [key, diagram] of Object.entries(diagrams) as [HelpDiagramId, HelpDiagram][]) {
    if (key !== diagram.id) {
      throw new Error(`図のキー ${key} とID ${diagram.id} が一致しません。`);
    }
    if (manualImageIds.has(diagram.id)) continue;
    if (diagram.manualImage !== undefined) {
      throw new Error(`対象外の図 ${diagram.id} に manualImage が指定されています。`);
    }
    manualDiagrams[diagram.id] = {
      id: diagram.id,
      title: diagram.title,
      alt: diagram.alt,
      svg: diagram.svg,
    };
  }

  return { chapters: manualChapters, diagrams: manualDiagrams };
}
