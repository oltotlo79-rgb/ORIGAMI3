import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { HELP_CHAPTERS, HELP_DIAGRAMS } from "../src/help/index.ts";

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

const manualContent = {
  // screenshotブロックを扱う共通内容形式。
  schemaVersion: 2,
  application: {
    name: "ORIGAMI3",
    version: packageJson.version,
  },
  chapters: HELP_CHAPTERS,
  diagrams: HELP_DIAGRAMS,
};

await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(manualContent, null, 2)}\n`, "utf8");

console.log(
  `取扱説明書の内容を書き出しました: ${outputPath} (${HELP_CHAPTERS.length}章 / ${Object.keys(HELP_DIAGRAMS).length}図)`,
);
