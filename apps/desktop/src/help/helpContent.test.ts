import { describe, expect, it } from "vitest";
import { HELP_CHAPTERS, helpBlockText, helpChapterSearchText } from "./helpContent";
import { HELP_DIAGRAMS } from "./helpDiagrams";
import type { HelpBlock, HelpScreenshotBlock } from "./helpTypes";

function manualImage(block: HelpBlock): string | null {
  if (block.type === "figure") return block.image?.trim() || null;
  if (block.type === "screenshot") return block.image.trim() || null;
  return null;
}

describe("ヘルプと取扱説明書PDFの共通内容源", () => {
  it("13章が番号順に並び、章IDと題がそろっている", () => {
    expect(HELP_CHAPTERS).toHaveLength(13);
    expect(HELP_CHAPTERS.map((chapter) => chapter.number)).toEqual(
      Array.from({ length: 13 }, (_, index) => index + 1),
    );
    expect(new Set(HELP_CHAPTERS.map((chapter) => chapter.id)).size).toBe(13);

    for (const chapter of HELP_CHAPTERS) {
      expect(chapter.title.trim().length).toBeGreaterThan(0);
      expect(chapter.summary.trim().length).toBeGreaterThan(0);
      expect(chapter.blocks.some((block) => block.type === "paragraph")).toBe(true);
      expect(chapter.blocks.some((block) => block.type === "steps")).toBe(true);
      expect(chapter.blocks.some((block) => block.type === "callout")).toBe(true);
      expect(chapter.blocks.map(helpBlockText).join(" ").trim().length).toBeGreaterThan(0);
    }
  });

  it("全章に解決できる固有のSVG図解が1点以上ある", () => {
    const figureIds = HELP_CHAPTERS.flatMap((chapter) =>
      chapter.blocks
        .filter((block) => block.type === "figure")
        .map((block) => block.diagramId),
    );

    expect(figureIds).toHaveLength(13);
    expect(new Set(figureIds).size).toBe(13);
    expect(new Set(Object.keys(HELP_DIAGRAMS))).toEqual(new Set(figureIds));
    expect(new Set(Object.values(HELP_DIAGRAMS).map((diagram) => diagram.svg)).size).toBe(13);

    for (const id of figureIds) {
      const diagram = HELP_DIAGRAMS[id];
      expect(diagram.title.trim().length).toBeGreaterThan(0);
      expect(diagram.alt.trim().length).toBeGreaterThan(0);
      expect(diagram.svg).toContain("<svg");
      expect(diagram.svg).toContain('viewBox="0 0 720 280"');
      expect(diagram.svg).toContain(`<title>${diagram.title}</title>`);
      expect(diagram.svg).toContain(`<desc>${diagram.alt}</desc>`);
      expect(diagram.svg.length).toBeGreaterThan(500);
    }
  });

  it("全章に実画面が1点以上あり、独立した画面例も直列化できる", () => {
    const blocks: HelpBlock[] = HELP_CHAPTERS.flatMap((chapter) => [...chapter.blocks]);
    const screenshots = blocks.filter(
      (block): block is HelpScreenshotBlock => block.type === "screenshot",
    );
    const images = blocks.map(manualImage).filter((image): image is string => image !== null);

    for (const chapter of HELP_CHAPTERS) {
      expect(chapter.blocks.some(manualImage), `第${chapter.number}章`).toBe(true);
    }
    expect(screenshots).toHaveLength(4);
    for (const screenshot of screenshots) {
      expect(screenshot.image).toMatch(/^screen-[a-z0-9-]+\.png$/);
      expect(screenshot.caption.trim().length).toBeGreaterThan(0);
    }
    expect(images).toHaveLength(17);
    expect(new Set(images).size).toBe(17);
  });

  it("本文は表示部品を含まない直列化可能なデータで、題と本文を検索できる", () => {
    expect(JSON.parse(JSON.stringify(HELP_CHAPTERS))).toEqual(HELP_CHAPTERS);
    const proposal = HELP_CHAPTERS.find((chapter) => chapter.id === "proposal");
    const crease = HELP_CHAPTERS.find((chapter) => chapter.id === "crease-pattern");
    expect(proposal && helpChapterSearchText(proposal)).toContain("骨格から展開図を提案");
    expect(crease && helpChapterSearchText(crease)).toContain("ベジェ曲線");
  });
});
