import { describe, expect, it } from "vitest";
import { HELP_CHAPTERS, helpBlockText, helpChapterSearchText } from "./helpContent";
import { HELP_DIAGRAMS } from "./helpDiagrams";
import type { HelpBlock, HelpScreenshotBlock } from "./helpTypes";

const REAL_SCREEN_DIAGRAM_IDS = [
  "workspace-four-areas",
  "new-paper-settings",
  "crease-tools",
  "angle-controls",
  "three-dimensional-controls",
  "technique-cards",
  "timeline-flow",
] as const;

const INTENTIONAL_HAND_DRAWN_DIAGRAM_IDS = [
  "overview-flow",
  "fold-flow",
  "save-export-flow",
  "troubleshooting-flow",
  "shortcut-map",
] as const;

const DEFERRED_DIAGRAM_ID = "proposal-wizard" as const;

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

  it("全章に解決できる固有の図が1点以上ある", () => {
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
    }
  });

  it("対象7図だけが追跡対象の実画面PNGを表示し、PDF用画像名もそろっている", () => {
    expect(REAL_SCREEN_DIAGRAM_IDS).toHaveLength(7);
    const imageUrls = new Set<string>();

    for (const id of REAL_SCREEN_DIAGRAM_IDS) {
      const diagram = HELP_DIAGRAMS[id];
      const expectedImage = `figure-${id}.png`;
      expect(diagram.manualImage).toBe(expectedImage);
      expect(diagram.svg).toContain("<image ");
      expect(diagram.svg).toContain(expectedImage);
      expect(diagram.svg).toContain('width="720" height="280"');
      expect(diagram.svg).toContain('preserveAspectRatio="none"');
      const href = diagram.svg.match(/<image href="([^"]+)"/)?.[1];
      expect(href).toBeTruthy();
      imageUrls.add(href!);
    }
    expect(imageUrls.size).toBe(7);
  });

  it("意図して手描きで残す5図と延期する提案図を区別する", () => {
    expect(INTENTIONAL_HAND_DRAWN_DIAGRAM_IDS).toHaveLength(5);
    for (const id of INTENTIONAL_HAND_DRAWN_DIAGRAM_IDS) {
      const diagram = HELP_DIAGRAMS[id];
      expect(diagram.manualImage).toBeUndefined();
      expect(diagram.svg).not.toContain("<image ");
      expect(diagram.svg.length).toBeGreaterThan(500);
    }

    const deferred = HELP_DIAGRAMS[DEFERRED_DIAGRAM_ID];
    expect(deferred.manualImage).toBeUndefined();
    expect(deferred.svg).not.toContain("<image ");
    expect(deferred.svg.length).toBeGreaterThan(500);
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
    expect(screenshots).toHaveLength(21);
    for (const screenshot of screenshots) {
      expect(screenshot.image).toMatch(/^screen-[a-z0-9-]+\.png$/);
      expect(screenshot.caption.trim().length).toBeGreaterThan(0);
    }
    expect(images).toHaveLength(34);
    expect(new Set(images).size).toBe(34);
  });

  it("本文は表示部品を含まない直列化可能なデータで、題と本文を検索できる", () => {
    expect(JSON.parse(JSON.stringify(HELP_CHAPTERS))).toEqual(HELP_CHAPTERS);
    const proposal = HELP_CHAPTERS.find((chapter) => chapter.id === "proposal");
    const crease = HELP_CHAPTERS.find((chapter) => chapter.id === "crease-pattern");
    expect(proposal && helpChapterSearchText(proposal)).toContain("骨格から展開図を提案");
    expect(crease && helpChapterSearchText(crease)).toContain("ベジェ曲線");
  });

  it("対称描画の目的・3つの基準・画面での選び方を利用者向けに説明する", () => {
    const crease = HELP_CHAPTERS.find((chapter) => chapter.id === "crease-pattern");
    expect(crease).toBeDefined();
    const text = helpChapterSearchText(crease!);

    expect(text).toContain("手間を半分");
    expect(text).toContain("紙の縦の中心線");
    expect(text).toContain("紙の横の中心線");
    expect(text).toContain("この線を基準にする");
    expect(text).toContain("紫の破線");
    expect(text).toContain("別の作品へ切り替えたときも紙の縦の中心線へ戻ります");
    expect(text).not.toContain("鏡映");
    expect(text).not.toContain("ヤコビアン");

    const angles = HELP_CHAPTERS.find((chapter) => chapter.id === "angles");
    expect(angles).toBeDefined();
    const angleText = helpChapterSearchText(angles!);
    expect(angleText).toContain("展開図から対になる折り目を自動で見つけ");
    expect(angleText).toContain("描画用の基準線とは別の動きです");
  });

  it("現行画面の折りたたみ・色・区画・テーマ・追従を利用者向けに網羅する", () => {
    const allText = HELP_CHAPTERS.map(helpChapterSearchText).join("\n");

    for (const label of [
      "この道具の詳しい操作方法 ▼",
      "展開図の詳しい操作方法 ▼",
      "詳しい3D操作方法 ▼",
      "丸みの詳しい操作方法 ▼",
      "紙の色 ▼",
    ]) {
      expect(allText).toContain(label);
    }
    expect(allText).toContain("初めて使うとき、長い説明はすべて閉じています");
    expect(allText).toContain("閉じたままでも表と裏の現在の色見本");
    expect(allText).toContain("Tabキーで枠を移したときも同じ吹き出し");
    expect(allText).toContain("文字の代わりに▼または▲のアイコン");

    for (const colorControl of [
      "色の面",
      "色相",
      "16進数",
      "取り消し",
      "この色にする",
      "Shiftを押しながら矢印キー",
      "Enterで確定",
      "Escで閉じます",
    ]) {
      expect(allText).toContain(colorControl);
    }

    expect(allText).toContain("展開図と立体表示は50%ずつ、下部パネルは32%");
    expect(allText).toContain("表示の広さを初期に戻す");
    expect(allText).toContain("和紙のような繊維と濃淡");
    expect(allText).toContain("ごく細かな粒");
    expect(allText).toContain("操作方法は共通です");

    expect(allText).toContain("動かしている折り目の角度を最優先");
    expect(allText).toContain("前の希望から譲った折り目は琥珀色");
    expect(allText).toContain("現在72.0°");
    expect(allText).toContain("希望どおりの形が見つからない場合も操作は止まりません");

    expect(allText).toContain("初めて使うときの基準は「紙の縦の中心線」");
    expect(allText).toContain("紙の横の中心線");
    expect(allText).toContain("この線を基準にする");
    expect(allText).toContain("紫の破線");

    for (const internalWord of [
      "ヤコビアン",
      "ソルバ",
      "アルゴリズム",
      "データ構造",
      "細かな点の位置",
      "細かな直線の集まり",
    ]) {
      expect(allText).not.toContain(internalWord);
    }
  });

  it("一般的不収束は自動調整を案内し、折り線欠落だけは引き直し・削除を残す", () => {
    const troubleshooting = HELP_CHAPTERS.find(
      (chapter) => chapter.id === "troubleshooting",
    );
    expect(troubleshooting).toBeDefined();
    const text = helpChapterSearchText(troubleshooting!);

    expect(text).toContain("前の角度を自動調整しています。操作は続けられます");
    expect(text).not.toContain("合わなくなった手順を移動・削除");
    expect(text).toContain(
      "「折り線が見つからない」: 展開図で消えた折り線を引き直すか、その手順を削除します",
    );
  });
});
