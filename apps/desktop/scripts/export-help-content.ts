import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { HELP_CHAPTERS, HELP_DIAGRAMS } from "../src/help/index.ts";
import {
  MANUAL_IMAGE_DIAGRAM_IDS,
  buildManualExportContent,
} from "../src/help/manualExport.ts";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const packageJsonPath = resolve(scriptDirectory, "../package.json");
const packageJson = JSON.parse(await readFile(packageJsonPath, "utf8")) as {
  name: string;
  version: string;
};

const defaultOutput = resolve(scriptDirectory, "../../../docs/manual/help-content.json");
const outputPath = process.argv[2]
  ? resolve(process.cwd(), process.argv[2])
  : defaultOutput;

const manualAssetsDirectory = resolve(scriptDirectory, "../../../docs/manual/assets");
const helpAssetsDirectory = resolve(scriptDirectory, "../src/help/diagram-assets");
const exported = buildManualExportContent(HELP_CHAPTERS, HELP_DIAGRAMS);

function assertPng(bytes: Uint8Array, path: string): void {
  const signature = [137, 80, 78, 71, 13, 10, 26, 10];
  if (
    bytes.length < 24 ||
    !signature.every((value, index) => bytes[index] === value) ||
    String.fromCharCode(...bytes.subarray(12, 16)) !== "IHDR"
  ) {
    throw new Error(`PNGとして読めません: ${path}`);
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = view.getUint32(16);
  const height = view.getUint32(20);
  if (width !== 1800 || height !== 700) {
    throw new Error(`派生画像は1800x700にしてください: ${path} (${width}x${height})`);
  }
  if (bytes.length < 4096) {
    throw new Error(`派生画像が小さすぎるため配布物へ確実に含められません: ${path}`);
  }
}

function bytesEqual(left: Uint8Array, right: Uint8Array): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

const manualImageNames = MANUAL_IMAGE_DIAGRAM_IDS.map((id) => {
  const image = HELP_DIAGRAMS[id].manualImage;
  if (!image) throw new Error(`図 ${id} に manualImage がありません。`);
  return image;
});
if (new Set(manualImageNames).size !== MANUAL_IMAGE_DIAGRAM_IDS.length) {
  throw new Error("説明書用の派生画像名が重複しています。");
}

await Promise.all(
  manualImageNames.map(async (image) => {
    const manualPath = resolve(manualAssetsDirectory, image);
    const helpPath = resolve(helpAssetsDirectory, image);
    const [manualBytes, helpBytes] = await Promise.all([readFile(manualPath), readFile(helpPath)]);
    assertPng(manualBytes, manualPath);
    assertPng(helpBytes, helpPath);
    if (!bytesEqual(manualBytes, helpBytes)) {
      throw new Error(`ヘルプ用と説明書用の派生画像が一致しません: ${image}`);
    }
  }),
);

const exportedBlocks = exported.chapters.flatMap((chapter) => chapter.blocks);
const figureCount = exportedBlocks.filter((block) => block.type === "figure").length;
const screenshotCount = exportedBlocks.filter((block) => block.type === "screenshot").length;
const referencedImages = exportedBlocks.flatMap((block) => {
  if (block.type === "screenshot") return [block.image];
  if (block.type === "figure" && block.image) return [block.image];
  return [];
});
const derivedImageCount = referencedImages.filter((image) => image.startsWith("figure-")).length;
const screenImageCount = referencedImages.filter((image) => image.startsWith("screen-")).length;
if (
  exported.chapters.length !== 13 ||
  Object.keys(exported.diagrams).length !== 6 ||
  figureCount !== 6 ||
  screenshotCount !== 35 ||
  derivedImageCount !== 7 ||
  screenImageCount !== 34 ||
  referencedImages.length !== 41
) {
  throw new Error(
    `PDF用内容の件数が不正です: ${exported.chapters.length}章 / ${Object.keys(exported.diagrams).length} SVG図 / ${figureCount} figure / ${screenshotCount} screenshot / ${derivedImageCount}派生PNG / ${screenImageCount}既存PNG`,
  );
}

const manualContent = {
  // screenshotブロックを扱う共通内容形式。
  schemaVersion: 2,
  application: {
    name: "ORIGAMI3",
    version: packageJson.version,
  },
  chapters: exported.chapters,
  diagrams: exported.diagrams,
};

await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(manualContent, null, 2)}\n`, "utf8");

console.log(
  `取扱説明書の内容を書き出しました: ${outputPath} (${exported.chapters.length}章 / ${Object.keys(exported.diagrams).length} SVG図 / ${derivedImageCount}派生PNG / ${screenImageCount}既存PNG)`,
);
