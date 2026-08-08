// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import {
  canvasBackgroundColor,
  COLORS,
} from "./CpEditor/renderer";
import { canvas3dBackgroundColor } from "./Viewer3D/sceneBuilder";

afterEach(() => {
  document.body.replaceChildren();
});

function themedCanvas(token: string, color: string): HTMLCanvasElement {
  const app = document.createElement("div");
  app.style.setProperty(token, color);
  const canvas = document.createElement("canvas");
  app.append(canvas);
  document.body.append(app);
  return canvas;
}

describe("キャンバスのテーマ背景", () => {
  it("2Dは祖先から継承したCSS変数を読み、未定義ならポップへ戻る", () => {
    expect(canvasBackgroundColor(themedCanvas("--color-canvas-2d", "#f0f0f1"))).toBe(
      "#f0f0f1",
    );
    expect(canvasBackgroundColor(document.createElement("canvas"))).toBe(COLORS.background);
  });

  it("3Dは祖先から継承したCSS変数を読み、未定義ならポップへ戻る", () => {
    expect(canvas3dBackgroundColor(themedCanvas("--color-canvas-3d", "#e3d8bf"))).toBe(
      "#e3d8bf",
    );
    expect(canvas3dBackgroundColor(document.createElement("canvas"))).toBe("#cfcbc2");
  });
});
