import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import * as builder from "./sceneBuilder";
import * as camera from "./sceneCamera";
import * as content from "./sceneContent";
import * as facade from "./sceneFacade";
import * as layers from "./sceneLayers";

/** 4-Aで固定した外向け入口を保ったまま、実装の所有先だけを分ける。 */
describe("scene C1〜C4 の公開境界", () => {
  it("C1 topology/content の公開実装を互換facadeからそのまま公開する", () => {
    expect([
      builder.buildTopology,
      builder.contentBoundingBox,
      builder.createContent,
      builder.updateFrame,
      builder.createSoftContent,
      builder.updateSoftContent,
    ]).toEqual([
      content.buildTopology,
      content.contentBoundingBox,
      content.createContent,
      content.updateFrame,
      content.createSoftContent,
      content.updateSoftContent,
    ]);
  });

  it("C2 camera/framing の公開実装を互換facadeからそのまま公開する", () => {
    expect([
      builder.CAMERA_DIR,
      builder.cameraScreenUp,
      builder.rotateCameraByDrag,
      builder.applyCameraDragRotation,
      builder.viewRotationStarts,
      builder.legacyPaperDistance,
      builder.boxCorners,
      builder.paperCorners,
      builder.boxFraming,
      builder.paperFraming,
      builder.applyPaperFraming,
    ]).toEqual([
      camera.CAMERA_DIR,
      camera.cameraScreenUp,
      camera.rotateCameraByDrag,
      camera.applyCameraDragRotation,
      camera.viewRotationStarts,
      camera.legacyPaperDistance,
      camera.boxCorners,
      camera.paperCorners,
      camera.boxFraming,
      camera.paperFraming,
      camera.applyPaperFraming,
    ]);
  });

  it("C3 highlight/layers の公開実装を互換facadeからそのまま公開する", () => {
    expect([
      builder.HIGHLIGHT_WIDTH_PX,
      builder.FOCUS_HIGHLIGHT_WIDTH_PX,
      builder.SUSPECT_HIGHLIGHT_WIDTH_PX,
      builder.PIN_MARK_WIDTH_PX,
      builder.PIN_MARK_RATIO,
      builder.PIN_MARK_MIN_LENGTH,
      builder.PIN_MARK_MAX_LENGTH,
      builder.createHighlightMaterials,
      builder.createHighlightGeometry,
      builder.highlightAppearance,
      builder.withPinMarks,
      builder.createHighlightLayer,
      builder.createSupplementalEdgeLayer,
      builder.clearGroup,
      builder.createPreviewMaterial,
    ]).toEqual([
      layers.HIGHLIGHT_WIDTH_PX,
      layers.FOCUS_HIGHLIGHT_WIDTH_PX,
      layers.SUSPECT_HIGHLIGHT_WIDTH_PX,
      layers.PIN_MARK_WIDTH_PX,
      layers.PIN_MARK_RATIO,
      layers.PIN_MARK_MIN_LENGTH,
      layers.PIN_MARK_MAX_LENGTH,
      layers.createHighlightMaterials,
      layers.createHighlightGeometry,
      layers.highlightAppearance,
      layers.withPinMarks,
      layers.createHighlightLayer,
      layers.createSupplementalEdgeLayer,
      layers.clearGroup,
      layers.createPreviewMaterial,
    ]);
  });

  it("C4 scene facade が外向けscene APIの唯一の実装所有者になる", () => {
    expect([
      builder.canvas3dBackgroundColor,
      builder.captureViewer3DReadback,
      builder.createScene,
    ]).toEqual([
      facade.canvas3dBackgroundColor,
      facade.captureViewer3DReadback,
      facade.createScene,
    ]);
  });

  it("分割した製品ファイルは全て1,500行以下に収まる", () => {
    const files = [
      "sceneBuilder.ts",
      "sceneContent.ts",
      "sceneCamera.ts",
      "sceneLayers.ts",
      "sceneFacade.ts",
    ];
    const counts = Object.fromEntries(
      files.map((name) => {
        const path = fileURLToPath(new URL(name, import.meta.url).href);
        return [name, readFileSync(path, "utf8").split(/\r?\n/).length];
      }),
    );
    expect(counts).toEqual(
      Object.fromEntries(files.map((name) => [name, expect.any(Number)])),
    );
    expect(Math.max(...Object.values(counts))).toBeLessThanOrEqual(1_500);
  });
});
