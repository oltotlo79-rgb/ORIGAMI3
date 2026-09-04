import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { HELP_CHAPTERS } from "./helpContent";
import { HELP_DIAGRAMS } from "./helpDiagrams";
import {
  MANUAL_IMAGE_DIAGRAM_IDS,
  buildManualExportContent,
} from "./manualExport";
import type { HelpBlock, HelpDiagram, HelpDiagramId } from "./helpTypes";

function imageReferences(blocks: readonly HelpBlock[]): string[] {
  return blocks.flatMap((block) => {
    if (block.type === "screenshot") return [block.image];
    if (block.type === "figure" && block.image) return [block.image];
    return [];
  });
}

function pngSize(bytes: Uint8Array): [number, number] {
  expect(Array.from(bytes.subarray(0, 8))).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
  expect(String.fromCharCode(...bytes.subarray(12, 16))).toBe("IHDR");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return [view.getUint32(16), view.getUint32(20)];
}

function expectSameBytes(helpBytes: Uint8Array, manualBytes: Uint8Array, image: string): void {
  expect(
    helpBytes.byteLength,
    `${image}: ヘルプ側と取扱説明書側でバイト数が異なります`,
  ).toBe(manualBytes.byteLength);

  let differentIndex = -1;
  for (let index = 0; index < helpBytes.byteLength; index += 1) {
    if (helpBytes[index] !== manualBytes[index]) {
      differentIndex = index;
      break;
    }
  }

  const detail =
    differentIndex < 0
      ? ""
      : ` (offset ${differentIndex}: ヘルプ=${helpBytes[differentIndex]}, 取扱説明書=${manualBytes[differentIndex]})`;
  expect(differentIndex, `${image}: 2か所のPNGのバイト列が一致しません${detail}`).toBe(-1);
}

describe("取扱説明書向けの実画面図", () => {
  it("7図を派生PNGと元の全画面へ変換し、残る6図だけをSVGで渡す", () => {
    const exported = buildManualExportContent(HELP_CHAPTERS, HELP_DIAGRAMS);
    const blocks = exported.chapters.flatMap((chapter) => chapter.blocks);
    const figures = blocks.filter((block) => block.type === "figure");
    const screenshots = blocks.filter((block) => block.type === "screenshot");
    const images = imageReferences(blocks);
    const derivedImages = images.filter((image) => image.startsWith("figure-"));
    const screenImages = images.filter((image) => image.startsWith("screen-"));

    expect(exported.chapters).toHaveLength(13);
    expect(Object.keys(exported.diagrams)).toHaveLength(6);
    expect(figures).toHaveLength(6);
    // 旧39/45/38→新47/53/46。2026-09-04に本文へ加えた8画面ぶんの増加で、
    // 手描きで残す6図・派生PNG 7図の区別と重複0件の条件は変えていない。
    expect(screenshots).toHaveLength(47);
    expect(images).toHaveLength(53);
    expect(derivedImages).toHaveLength(7);
    expect(new Set(derivedImages).size).toBe(7);
    expect(screenImages).toHaveLength(46);
    expect(new Set(screenImages).size).toBe(46);

    for (const id of MANUAL_IMAGE_DIAGRAM_IDS) {
      expect(figures.some((block) => block.diagramId === id)).toBe(false);
      expect(exported.diagrams[id]).toBeUndefined();
      expect(derivedImages).toContain(`figure-${id}.png`);
    }
    for (const diagram of Object.values(exported.diagrams)) {
      expect(diagram).not.toHaveProperty("manualImage");
    }
  });

  it("対象7組は正しいPNGで、1800x700かつ2か所のバイト列が完全一致する", () => {
    for (const id of MANUAL_IMAGE_DIAGRAM_IDS) {
      const image = `figure-${id}.png`;
      const helpPath = new URL(`./diagram-assets/${image}`, import.meta.url);
      const manualPath = new URL(`../../../../docs/manual/assets/${image}`, import.meta.url);
      const helpBytes = readFileSync(helpPath);
      const manualBytes = readFileSync(manualPath);

      expectSameBytes(helpBytes, manualBytes, image);
      expect(helpBytes.byteLength).toBeGreaterThanOrEqual(4096);
      expect(pngSize(helpBytes)).toEqual([1800, 700]);
      expect(pngSize(manualBytes)).toEqual([1800, 700]);
    }
  });

  it("直下ファイル名でないmanualImageと変換回数のずれを拒否する", () => {
    const wrongName = {
      ...HELP_DIAGRAMS,
      "workspace-four-areas": {
        ...HELP_DIAGRAMS["workspace-four-areas"],
        manualImage: "../figure-workspace-four-areas.png",
      },
    } satisfies Record<HelpDiagramId, HelpDiagram>;
    expect(() => buildManualExportContent(HELP_CHAPTERS, wrongName)).toThrow(
      "docs/manual/assets/ 直下",
    );

    const duplicateFigure = {
      ...HELP_CHAPTERS[0],
      blocks: [...HELP_CHAPTERS[0].blocks, HELP_CHAPTERS[1].blocks[1]],
    };
    expect(() =>
      buildManualExportContent([duplicateFigure, ...HELP_CHAPTERS.slice(1)], HELP_DIAGRAMS),
    ).toThrow("1回だけ変換");
  });
});
