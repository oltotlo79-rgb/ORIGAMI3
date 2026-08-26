// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

// CSS importはvitestでは空になるため、製品で使う実ファイルをDOMへ入れて
// body portalの吹き出しを含む実際のselector/cascadeを確かめる。
const viewerCss = readFileSync(
  resolve(process.cwd(), "src/styles/viewer.css"),
  "utf8",
);
const baseLayoutCss = readFileSync(
  resolve(process.cwd(), "src/styles/base-layout.css"),
  "utf8",
);
const tooltipSource = readFileSync(
  resolve(process.cwd(), "src/components/Tooltip.tsx"),
  "utf8",
);
// jsdomはCSS cascade layerを解釈しないため、製品の単一viewer layerの外枠だけを
// 外して中の実selectorを検査する。selectorや宣言そのものは書き換えない。
const withoutLayer = (css: string, layer: string): string => css
  .replace(new RegExp(`^\\s*@layer\\s+${layer}\\s*\\{`), "")
  .replace(/\}\s*$/, "");
const globalFocusRule = baseLayoutCss.match(
  /(?:^|\n):focus-visible\s*\{[^}]+\}/u,
)?.[0];
if (globalFocusRule === undefined) {
  throw new Error("styles/base-layout.cssの共通focus輪がありません");
}
const captureCss = [
  ":root { --color-accent: #00867d; --focus-ring: 0 0 0 4px rgba(0, 122, 112, 0.2); }",
  globalFocusRule,
  withoutLayer(viewerCss, "viewer"),
].join("\n");

function mountActiveTooltip(): HTMLDivElement {
  const tooltip = document.createElement("div");
  tooltip.id = "origami3-active-tooltip";
  tooltip.dataset.floatingUi = "tooltip";
  tooltip.setAttribute("role", "tooltip");
  // TooltipHostが位置確定後に作る表示中の状態を再現する。
  tooltip.style.position = "fixed";
  tooltip.style.visibility = "visible";
  tooltip.textContent = "3D表示の操作方法";
  document.body.append(tooltip);
  return tooltip;
}

function mountViewerCanvas(): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.className = "viewer3d-canvas";
  canvas.tabIndex = 0;
  document.body.append(canvas);
  canvas.focus();
  return canvas;
}

function visibleBodySurfaceIds(): string[] {
  return Array.from(document.body.children)
    .filter((element) => getComputedStyle(element).display !== "none")
    .map((element) => element.id);
}

beforeEach(() => {
  const style = document.createElement("style");
  style.dataset.testSource = "styles/viewer.css";
  style.textContent = captureCss;
  document.head.append(style);

  const paper = document.createElement("main");
  paper.id = "capture-paper";
  document.body.append(paper);
});

afterEach(() => {
  document.documentElement.removeAttribute("data-origami3-capture-view");
  document.head.replaceChildren();
  document.body.replaceChildren();
});

describe("撮影中のbody portal吹き出し", () => {
  it("製品のtooltip marker 1件を撮影selector 1件で覆う", () => {
    expect(tooltipSource.match(/data-floating-ui="tooltip"/gu)).toHaveLength(1);
    expect(viewerCss.match(/\[data-floating-ui="tooltip"\]/gu)).toHaveLength(1);
  });

  it("通常画面では見え、撮影中だけ消え、撮影属性を外すと再び見える", () => {
    const tooltip = mountActiveTooltip();

    expect(getComputedStyle(tooltip).display).not.toBe("none");
    expect(getComputedStyle(tooltip).visibility).toBe("visible");

    document.documentElement.setAttribute("data-origami3-capture-view", "3d");
    expect(getComputedStyle(tooltip).display).toBe("none");

    document.documentElement.removeAttribute("data-origami3-capture-view");
    expect(getComputedStyle(tooltip).display).not.toBe("none");
    expect(getComputedStyle(tooltip).visibility).toBe("visible");
  });

  it("撮影中は吹き出しがmount済みでも可視面を増やさない", () => {
    document.documentElement.setAttribute("data-origami3-capture-view", "both");
    const withoutTooltip = visibleBodySurfaceIds();

    mountActiveTooltip();
    const withTooltip = visibleBodySurfaceIds();

    expect(withoutTooltip).toEqual(["capture-paper"]);
    expect(withTooltip).toEqual(withoutTooltip);
  });

  it("3D表示のfocus輪は通常画面に残り、撮影中だけ消え、撮影属性を外すと戻る", () => {
    const canvas = mountViewerCanvas();

    expect(document.activeElement).toBe(canvas);
    const normalOutline = getComputedStyle(canvas).outline;
    const normalShadow = getComputedStyle(canvas).boxShadow;
    expect(normalOutline).toContain("2px solid");
    expect(normalShadow).not.toBe("none");

    document.documentElement.setAttribute("data-origami3-capture-view", "3d");
    expect(getComputedStyle(canvas).outline).toBe("none");
    expect(getComputedStyle(canvas).boxShadow).toBe("none");

    document.documentElement.removeAttribute("data-origami3-capture-view");
    expect(getComputedStyle(canvas).outline).toBe(normalOutline);
    expect(getComputedStyle(canvas).boxShadow).toBe(normalShadow);
  });
});
