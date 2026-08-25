import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import * as camera from "./viewerCamera";
import * as highlight from "./viewerHighlight";
import * as lifecycle from "./viewerLifecycle";
import * as picking from "./viewerPicking";
import * as pointer from "./viewerPointer";

const PRODUCT_FILES = [
  "Viewer3D.tsx",
  "viewerLifecycle.ts",
  "viewerCamera.ts",
  "viewerPicking.ts",
  "viewerPointer.ts",
  "viewerHighlight.ts",
] as const;

function productSource(name: (typeof PRODUCT_FILES)[number]): string {
  const path = fileURLToPath(new URL(name, import.meta.url).href);
  return readFileSync(path, "utf8");
}

describe("Viewer C5〜C9 の公開境界", () => {
  it("C5〜C9をそれぞれ固有の実装moduleとして公開する", () => {
    expect([
      lifecycle,
      camera,
      picking,
      pointer,
      highlight,
    ].map((module) => Object.keys(module).length)).toEqual([
      expect.any(Number),
      expect.any(Number),
      expect.any(Number),
      expect.any(Number),
      expect.any(Number),
    ]);
    for (const count of [
      Object.keys(lifecycle).length,
      Object.keys(camera).length,
      Object.keys(picking).length,
      Object.keys(pointer).length,
      Object.keys(highlight).length,
    ]) {
      expect(count).toBeGreaterThan(0);
    }
  });

  it("Viewer3DはC5〜C9の5境界を明示して組み立てる", () => {
    const source = productSource("Viewer3D.tsx");
    for (const moduleName of [
      "viewerLifecycle",
      "viewerCamera",
      "viewerPicking",
      "viewerPointer",
      "viewerHighlight",
    ]) {
      expect(source).toMatch(
        new RegExp(`from\\s+["']\\./${moduleName}["']`),
      );
    }
  });

  it("分割した製品ファイルは全て1,500行以下で、useStateを増やさない", () => {
    const sources = PRODUCT_FILES.map((name) => [name, productSource(name)] as const);
    const lineCounts = Object.fromEntries(
      sources.map(([name, source]) => [name, source.split(/\r?\n/).length]),
    );
    expect(Math.max(...Object.values(lineCounts))).toBeLessThanOrEqual(1_500);
    expect(
      sources.flatMap(([name, source]) =>
        source.match(/\buseState\b/g)?.map(() => name) ?? [],
      ),
    ).toEqual([]);
  });

  it("既存overlayを各1回だけ再利用し、常設の表示区画を増やさない", () => {
    const source = productSource("Viewer3D.tsx");
    const openingTags = [
      "ViewerOverlayStack",
      "ViewerOperationHint",
      "PaperActionTip",
      "FoldDirectionTip",
      "ViewCube",
    ];
    for (const tag of openingTags) {
      expect(source.match(new RegExp(`<${tag}(?:\\s|>)`, "g")) ?? []).toHaveLength(1);
    }
  });
});
