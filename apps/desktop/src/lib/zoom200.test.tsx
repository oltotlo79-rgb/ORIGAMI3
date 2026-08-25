// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";
import App from "../App";
import { useAppStore } from "../store/appStore";

/*
 * jsdom has no layout engine: getBoundingClientRect(), clientWidth and
 * scrollWidth are all zero.  This test therefore separates the evidence:
 *
 * - the mounted product App supplies the real permanent-zone topology and
 *   the real toolbar children/text;
 * - the shipped CSS supplies the grid/flex/overflow and intrinsic-width
 *   contracts;
 * - no synthetic rectangles are assigned to product elements.
 */

vi.mock("../components/CpEditor/CpEditor", () => ({
  CpEditor: () => <div className="cp-editor" aria-label="展開図" />,
}));
vi.mock("../components/Viewer3D/Viewer3D", () => ({
  Viewer3D: () => <div className="viewer-3d" aria-label="3D表示" />,
}));
vi.mock("../components/RecoveryDialog", () => ({ RecoveryDialog: () => null }));
vi.mock("../components/dialogs/NewDocumentDialog", () => ({
  NewDocumentDialog: () => null,
}));
vi.mock("../components/dialogs/ProposalWizard", () => ({
  ProposalWizard: () => null,
}));
vi.mock("../components/dialogs/ExportDialog", () => ({ default: () => null }));
vi.mock("../components/dialogs/HelpCenter", () => ({ HelpCenter: () => null }));
vi.mock("../components/HistoryShortcuts", () => ({ HistoryShortcuts: () => null }));
vi.mock("../components/FirstRunGuide", () => ({ FirstRunGuide: () => null }));
vi.mock("../components/Tooltip", () => ({ TooltipHost: () => null }));
vi.mock("../captureApi", () => ({ installCaptureApi: vi.fn() }));

const PHYSICAL_VIEWPORT = { width: 1000, height: 700 } as const;
const ZOOM = 2;
const CSS_VIEWPORT = {
  width: PHYSICAL_VIEWPORT.width / ZOOM,
  height: PHYSICAL_VIEWPORT.height / ZOOM,
} as const;

function cssSource(fileName: string): string {
  return readFileSync(
    join(dirname(fileURLToPath(import.meta.url)), "..", "styles", fileName),
    "utf8",
  );
}

const tokensCss = cssSource("tokens.css");
const baseLayoutCss = cssSource("base-layout.css");
const viewerCss = cssSource("viewer.css");
const contextCss = cssSource("context.css");
const responsiveCss = cssSource("responsive.css");
const viewerComponentSource = readFileSync(
  join(
    dirname(fileURLToPath(import.meta.url)),
    "..",
    "components",
    "Viewer3D",
    "Viewer3D.tsx",
  ),
  "utf8",
);

function escaped(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

function declarationBlock(selector: string, source: string): string {
  const match = new RegExp(`${escaped(selector)}\\s*\\{([\\s\\S]*?)\\}`, "u").exec(
    source,
  );
  if (match === null) throw new Error(`CSSブロックがありません: ${selector}`);
  return match[1];
}

function atRuleBlocks(prelude: string, source: string): string[] {
  const blocks: string[] = [];
  let searchFrom = 0;
  while (searchFrom < source.length) {
    const start = source.indexOf(prelude, searchFrom);
    if (start < 0) break;
    const openingBrace = source.indexOf("{", start + prelude.length);
    if (openingBrace < 0) throw new Error(`CSS規則の開始括弧がありません: ${prelude}`);
    let depth = 0;
    let closingBrace = -1;
    for (let index = openingBrace; index < source.length; index += 1) {
      if (source[index] === "{") depth += 1;
      if (source[index] === "}") depth -= 1;
      if (depth === 0) {
        closingBrace = index;
        break;
      }
    }
    if (closingBrace < 0) throw new Error(`CSS規則の終了括弧がありません: ${prelude}`);
    blocks.push(source.slice(openingBrace + 1, closingBrace));
    searchFrom = closingBrace + 1;
  }
  if (blocks.length === 0) throw new Error(`CSS規則がありません: ${prelude}`);
  return blocks;
}

function atRuleBlock(prelude: string, source: string): string {
  return atRuleBlocks(prelude, source)[0];
}

function declarationValue(block: string, property: string): string {
  const match = new RegExp(`(?:^|[;\\n])\\s*${escaped(property)}\\s*:\\s*([^;]+)`, "u").exec(
    block,
  );
  if (match === null) throw new Error(`CSS宣言がありません: ${property}`);
  return match[1].trim();
}

function tokenPx(name: string): number {
  const match = new RegExp(`${escaped(name)}\\s*:\\s*([0-9.]+)px`, "u").exec(
    tokensCss,
  );
  if (match === null) throw new Error(`pxトークンがありません: ${name}`);
  return Number(match[1]);
}

function pxValue(value: string): number {
  const direct = /^([0-9.]+)px$/u.exec(value);
  if (direct !== null) return Number(direct[1]);
  const variable = /^var\((--[a-z0-9-]+)\)$/u.exec(value);
  if (variable !== null) return tokenPx(variable[1]);
  throw new Error(`単純なpx値ではありません: ${value}`);
}

function expectDeclarations(block: string, expected: readonly string[]): void {
  for (const declaration of expected) expect(block).toContain(declaration);
}

function fullWidthJapaneseCount(text: string): number {
  return [...text].filter((character) =>
    /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}ー]/u.test(character),
  ).length;
}

const narrowRules = atRuleBlock("@media (max-width: 1100px)", responsiveCss);
const originalNewDocument = useAppStore.getState().newDocument;
const originalCheckRecovery = useAppStore.getState().checkRecovery;
const originalUiTheme = useAppStore.getState().uiTheme;
const originalViewport = {
  innerWidth: Object.getOwnPropertyDescriptor(window, "innerWidth"),
  innerHeight: Object.getOwnPropertyDescriptor(window, "innerHeight"),
  devicePixelRatio: Object.getOwnPropertyDescriptor(window, "devicePixelRatio"),
};

beforeEach(() => {
  Object.defineProperties(window, {
    innerWidth: { configurable: true, value: CSS_VIEWPORT.width },
    innerHeight: { configurable: true, value: CSS_VIEWPORT.height },
    devicePixelRatio: { configurable: true, value: ZOOM },
  });
  useAppStore.setState({
    uiTheme: "pop",
    exportOpen: false,
    helpOpen: false,
    proposalStep: null,
    newDocument: vi.fn().mockResolvedValue(undefined),
    checkRecovery: vi.fn().mockResolvedValue(undefined),
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    uiTheme: originalUiTheme,
    exportOpen: false,
    helpOpen: false,
    proposalStep: null,
    newDocument: originalNewDocument,
    checkRecovery: originalCheckRecovery,
  });
  for (const [property, descriptor] of Object.entries(originalViewport)) {
    if (descriptor !== undefined) Object.defineProperty(window, property, descriptor);
  }
});

function renderProductApp(): HTMLElement {
  const { container } = render(<App />);
  const app = container.querySelector<HTMLElement>(":scope > .app");
  if (app === null) throw new Error("製品Appのルートがありません");
  expect(window.innerWidth).toBe(500);
  expect(window.innerHeight).toBe(350);
  expect(window.devicePixelRatio).toBe(2);
  return app;
}

interface WorkspaceMetrics {
  mainHeight: number;
  contextHeight: number;
  pane2dWidth: number;
  pane3dWidth: number;
  timelineHeight: number;
  viewerHeight: number;
}

function workspaceMetrics(app: HTMLElement): WorkspaceMetrics {
  const mainRow = app.querySelector<HTMLElement>(":scope > .main-row");
  if (mainRow === null) throw new Error("製品Appの中央行がありません");
  const columns = /^([0-9.]+)px\s+([0-9.]+)fr\s+([0-9.]+)px\s+([0-9.]+)fr$/u.exec(
    mainRow.style.gridTemplateColumns,
  );
  if (columns === null) throw new Error("製品Appの左右比率が読めません");
  const availableWidth = CSS_VIEWPORT.width - Number(columns[1]) - Number(columns[3]);
  const columnShare = Number(columns[2]) + Number(columns[4]);
  const pane2dWidth = (availableWidth * Number(columns[2])) / columnShare;
  const pane3dWidth = (availableWidth * Number(columns[4])) / columnShare;

  const toolbarHeight = pxValue(
    declarationValue(declarationBlock(".toolbar", baseLayoutCss), "min-height"),
  );
  const contextSplitterHeight = pxValue(
    declarationValue(
      declarationBlock(".context-panel-splitter", baseLayoutCss),
      "min-height",
    ),
  );
  const mainShare = Number(app.style.getPropertyValue("--main-row-share").replace("fr", ""));
  const contextShare = Number(
    app.style.getPropertyValue("--context-panel-share").replace("fr", ""),
  );
  const verticalShare = mainShare + contextShare;
  const availableHeight = CSS_VIEWPORT.height - toolbarHeight - contextSplitterHeight;
  const mainHeight = (availableHeight * mainShare) / verticalShare;
  const contextHeight = (availableHeight * contextShare) / verticalShare;
  const pane3dRows = declarationValue(
    declarationBlock(".pane-3d", viewerCss),
    "grid-template-rows",
  );
  const timelineMatch = /([0-9.]+)px$/u.exec(pane3dRows);
  if (timelineMatch === null) throw new Error("時間軸の固定高さが読めません");
  const timelineHeight = Number(timelineMatch[1]);
  return {
    mainHeight,
    contextHeight,
    pane2dWidth,
    pane3dWidth,
    timelineHeight,
    viewerHeight: mainHeight - timelineHeight,
  };
}

function toolbarIntrinsicWidth(toolbar: HTMLElement): number {
  const buttons = Array.from(toolbar.querySelectorAll<HTMLButtonElement>(":scope > button"));
  const separators = Array.from(
    toolbar.querySelectorAll<HTMLElement>(":scope > .toolbar-separator"),
  );
  const brand = toolbar.querySelector<HTMLElement>(":scope > .toolbar-brand");
  if (brand === null) throw new Error("製品ツールバーのブランドがありません");

  // These counts and labels come from the mounted AppToolbar, not a test fixture.
  expect(buttons.map((button) => button.textContent?.trim())).toEqual([
    "新規",
    "開く",
    "保存",
    "元に戻す",
    "やり直し",
    "提案",
    "書き出し",
    "ヘルプ",
  ]);
  expect(separators).toHaveLength(3);
  expect(Array.from(toolbar.children)).toHaveLength(12);
  expect(buttons.every((button) => button.querySelector(".toolbar-icon") !== null)).toBe(
    true,
  );

  const toolbarRule = declarationBlock(".toolbar", narrowRules);
  const buttonRule = declarationBlock(".toolbar button", narrowRules);
  const brandRule = declarationBlock(".toolbar-brand", narrowRules);
  const markRule = declarationBlock(".toolbar-brand-mark", narrowRules);
  const baseButtonRule = declarationBlock(".toolbar button", baseLayoutCss);
  const genericButtonRule = declarationBlock(".app :where(button)", baseLayoutCss);
  const iconRule = declarationBlock(".toolbar-icon", baseLayoutCss);
  const separatorRule = declarationBlock(".toolbar-separator", baseLayoutCss);
  const strongRule = declarationBlock(".toolbar-brand-copy strong", baseLayoutCss);

  const toolbarPadding = pxValue(declarationValue(toolbarRule, "padding-inline"));
  const toolbarGap = pxValue(declarationValue(toolbarRule, "gap"));
  const buttonPadding = pxValue(declarationValue(buttonRule, "padding-inline"));
  const buttonGap = pxValue(declarationValue(baseButtonRule, "gap"));
  const buttonBorder = tokenPx(
    /var\((--[a-z0-9-]+)\)/u.exec(declarationValue(genericButtonRule, "border"))?.[1] ??
      "",
  );
  const iconWidth = pxValue(declarationValue(iconRule, "width"));
  const fontSize = tokenPx("--fs-md");
  const labelWidth = buttons.reduce(
    (sum, button) => sum + fullWidthJapaneseCount(button.textContent?.trim() ?? "") * fontSize,
    0,
  );
  const buttonChrome =
    buttons.length *
    (buttonPadding * 2 + buttonBorder * 2 + iconWidth + buttonGap);

  const separatorWidth =
    separators.length *
    (pxValue(declarationValue(separatorRule, "width")) +
      pxValue(
        declarationValue(separatorRule, "margin").split(/\s+/u).slice(-1)[0] ?? "",
      ) *
        2);

  // The ASCII "ORIGAMI" width is deliberately omitted. The fixed mark, fixed
  // digit and padding alone are enough for a conservative lower bound.
  const brandWidth =
    pxValue(declarationValue(markRule, "width")) +
    pxValue(declarationValue(brandRule, "gap")) +
    pxValue(declarationValue(brandRule, "padding-right")) +
    tokenPx("--brand-digit-width") +
    tokenPx("--brand-digit-margin") +
    pxValue(declarationValue(strongRule, "padding-inline-end")) +
    tokenPx("--brand-divider");

  return (
    toolbarPadding * 2 +
    toolbarGap * (toolbar.children.length - 1) +
    buttonChrome +
    labelWidth +
    separatorWidth +
    brandWidth
  );
}

describe("物理1000×700を200%表示した製品画面", () => {
  it("文字を切らず、実際の上部操作を500 CSS pxへ収める", () => {
    const app = renderProductApp();
    const toolbar = app.querySelector<HTMLElement>(":scope > .toolbar");
    if (toolbar === null) throw new Error("製品ツールバーがありません");

    const rootRule = /html,\s*body,\s*#root\s*\{([\s\S]*?)\}/u.exec(
      baseLayoutCss,
    )?.[1];
    if (rootRule === undefined) throw new Error("画面ルートのCSSがありません");
    expect(rootRule).toContain("overflow: hidden");
    expect(declarationBlock(".toolbar", baseLayoutCss)).not.toContain("flex-wrap");
    expect(declarationBlock(".toolbar button", baseLayoutCss)).toContain(
      "white-space: nowrap",
    );

    const requiredWidth = toolbarIntrinsicWidth(toolbar);
    expect(
      requiredWidth,
      `実DOMとCSSから求めた上部操作の保守的なintrinsic幅 ${requiredWidth}px が、` +
        `200%時の表示幅 ${CSS_VIEWPORT.width}px を超えています。` +
        "画面ルートで隠すと、右側の文字と押しどころが切れます。",
    ).toBeLessThanOrEqual(CSS_VIEWPORT.width);
  });

  it("押しどころを重ねず、各常設区画の中で折返し又は縦送りにする", () => {
    const app = renderProductApp();
    const buttons = Array.from(app.querySelectorAll<HTMLButtonElement>("button"));
    expect(buttons.length).toBeGreaterThanOrEqual(19);
    expect(buttons.every((button) => button.tabIndex === 0)).toBe(true);

    // jsdomの矩形0を合否に使わず、実際の押しどころを収める製品CSSを検査する。
    expect(
      buttons.every((button) => {
        const box = button.getBoundingClientRect();
        return box.width === 0 && box.height === 0;
      }),
    ).toBe(true);
    expectDeclarations(declarationBlock(".toolbar", baseLayoutCss), [
      "display: flex",
      "align-items: center",
    ]);
    expectDeclarations(declarationBlock(".toolbar", narrowRules), [
      "gap: var(--sp-2)",
    ]);
    expectDeclarations(declarationBlock(".tool-rail", baseLayoutCss), [
      "display: flex",
      "flex-direction: column",
      "gap: var(--sp-3)",
      "overflow-y: auto",
    ]);
    expectDeclarations(declarationBlock(".viewer-overlay-stack", viewerCss), [
      "display: flex",
      "flex-direction: column",
      "gap: var(--sp-3)",
      "overflow-y: auto",
    ]);
    expectDeclarations(declarationBlock(".timeline-controls", viewerCss), [
      "display: flex",
      "gap: var(--sp-3)",
      "overflow-x: auto",
    ]);
    expectDeclarations(declarationBlock(".paper-action-entrances", contextCss), [
      "display: flex",
      "flex-wrap: wrap",
      "gap: var(--sp-3)",
    ]);
    expectDeclarations(declarationBlock(".button-row", contextCss), [
      "display: flex",
      "gap: var(--sp-3)",
      "flex-wrap: wrap",
    ]);
    expectDeclarations(declarationBlock(".paper-color-swatches", contextCss), [
      "display: grid",
      "grid-template-columns: repeat(8, 26px)",
      "gap: var(--sp-2)",
    ]);

    // Viewer3D itself needs WebGL and is the one mocked leaf above. Prove that
    // its shipped source really mounts both controls, then compare their
    // shipped absolute-position contracts in the real 200% workspace.
    expect(viewerComponentSource).toContain("<ViewCube");
    expect(viewerComponentSource).toContain('className="viewer-reset"');
    const metrics = workspaceMetrics(app);
    expect(metrics.pane3dWidth).toBeLessThanOrEqual(360);
    const compactViewerBlocks = atRuleBlocks(
      "@container viewer-operation-help (max-width: 360px)",
      responsiveCss,
    );
    const overlayOwner = compactViewerBlocks.find((block) =>
      block.includes(".viewer-overlay-region"),
    );
    if (overlayOwner === undefined) throw new Error("狭い3D札のCSS規則がありません");
    const overlayRule = declarationBlock(".viewer-overlay-region", overlayOwner);
    const overlayTop = pxValue(declarationValue(overlayRule, "top"));
    const overlayBottom = pxValue(declarationValue(overlayRule, "bottom"));
    const missingOverlayHeight = Math.max(
      0,
      overlayTop + overlayBottom - metrics.viewerHeight,
    );

    const cubeRule = declarationBlock(".view-cube", viewerCss);
    const resetRule = declarationBlock(".viewer-reset", baseLayoutCss);
    const cube = {
      left:
        metrics.pane3dWidth -
        pxValue(declarationValue(cubeRule, "right")) -
        pxValue(declarationValue(cubeRule, "width")),
      top: pxValue(declarationValue(cubeRule, "top")),
      width: pxValue(declarationValue(cubeRule, "width")),
      height: pxValue(declarationValue(cubeRule, "height")),
    };
    const resetWidth =
      pxValue(declarationValue(resetRule, "padding").split(/\s+/u).slice(-1)[0] ?? "") *
        2 +
      tokenPx("--control-border-width") * 2;
    const reset = {
      left:
        metrics.pane3dWidth -
        pxValue(declarationValue(resetRule, "right")) -
        resetWidth,
      top:
        metrics.viewerHeight -
        pxValue(declarationValue(resetRule, "bottom")) -
        pxValue(declarationValue(resetRule, "min-height")),
      width: resetWidth,
      height: pxValue(declarationValue(resetRule, "min-height")),
    };
    const overlapWidth = Math.max(
      0,
      Math.min(cube.left + cube.width, reset.left + reset.width) -
        Math.max(cube.left, reset.left),
    );
    const overlapHeight = Math.max(
      0,
      Math.min(cube.top + cube.height, reset.top + reset.height) -
        Math.max(cube.top, reset.top),
    );
    expect(
      [overlapWidth, overlapHeight, missingOverlayHeight],
      `3D本体は${metrics.viewerHeight.toFixed(2)}px高です。` +
        `視点立方体と「視点を戻す」の保守的な重なりは` +
        `${overlapWidth.toFixed(2)}×${overlapHeight.toFixed(2)}px、` +
        `札領域は高さが${missingOverlayHeight.toFixed(2)}px不足します。`,
    ).toEqual([0, 0, 0]);
  });

  it("道具・展開図・3Dと時間軸・右の札の4区画を同時に残す", () => {
    const app = renderProductApp();
    const mainRow = app.querySelector<HTMLElement>(":scope > .main-row");
    const contextPanel = app.querySelector<HTMLElement>(":scope > .context-panel");
    if (mainRow === null || contextPanel === null) {
      throw new Error("製品Appの常設区画がありません");
    }
    const toolRail = mainRow.querySelector<HTMLElement>(":scope > .tool-rail");
    const creasePattern = mainRow.querySelector<HTMLElement>(":scope > .pane-2d");
    const viewerAndTimeline = mainRow.querySelector<HTMLElement>(":scope > .pane-3d");
    expect(toolRail).not.toBeNull();
    expect(creasePattern).not.toBeNull();
    expect(viewerAndTimeline).not.toBeNull();
    expect(viewerAndTimeline?.querySelector(":scope > .pane-3d-view")).not.toBeNull();
    expect(viewerAndTimeline?.querySelector(":scope > .timeline")).not.toBeNull();
    expect(contextPanel.id).toBe("context-panel");
    expect([toolRail, creasePattern, viewerAndTimeline, contextPanel]).toHaveLength(4);
    expect(
      [toolRail, creasePattern, viewerAndTimeline, contextPanel].every(
        (area) => area !== null && !area.hidden && area.getAttribute("aria-hidden") !== "true",
      ),
    ).toBe(true);

    expectDeclarations(declarationBlock(".app", baseLayoutCss), [
      "display: grid",
      "minmax(0, var(--main-row-share",
      "minmax(0, var(--context-panel-share",
      "height: 100%",
    ]);
    expectDeclarations(declarationBlock(".main-row", baseLayoutCss), [
      "display: grid",
      "grid-template-columns: 64px 1fr 6px 1fr",
      "min-height: 0",
    ]);
    expectDeclarations(declarationBlock(".pane", baseLayoutCss), [
      "min-width: 0",
      "min-height: 0",
      "overflow: hidden",
    ]);
    expectDeclarations(declarationBlock(".pane-3d", viewerCss), [
      "display: grid",
      "grid-template-rows: minmax(0, 1fr) 96px",
    ]);
    expectDeclarations(declarationBlock(".context-panel", contextCss), [
      "min-height: 0",
      "overflow: hidden",
    ]);

    const metrics = workspaceMetrics(app);
    expect(metrics.pane2dWidth).toBeGreaterThan(0);
    expect(metrics.pane3dWidth).toBeGreaterThan(0);
    expect(metrics.mainHeight).toBeGreaterThan(metrics.timelineHeight);
    expect(metrics.timelineHeight).toBeGreaterThan(0);
    expect(metrics.viewerHeight).toBeGreaterThan(0);
    expect(metrics.contextHeight).toBeGreaterThan(0);
  });

  it("画面全体の横送りを出さず、各区画で横方向のはみ出しを止める", () => {
    const app = renderProductApp();
    const rootRule = /html,\s*body,\s*#root\s*\{([\s\S]*?)\}/u.exec(
      baseLayoutCss,
    )?.[1];
    if (rootRule === undefined) throw new Error("画面ルートのCSSがありません");
    expect(rootRule).toContain("overflow: hidden");
    expect(rootRule).not.toMatch(/overflow-x:\s*(?:auto|scroll)/u);
    expectDeclarations(declarationBlock(".tool-rail", baseLayoutCss), [
      "overflow-x: hidden",
    ]);
    expectDeclarations(declarationBlock(".pane", baseLayoutCss), [
      "min-width: 0",
      "overflow: hidden",
    ]);
    expectDeclarations(declarationBlock(".timeline", viewerCss), [
      "min-width: 0",
      "overflow: hidden",
    ]);
    expectDeclarations(declarationBlock(".context-selection", contextCss), [
      "min-width: 0",
      "overflow-y: auto",
    ]);

    const toolbar = app.querySelector<HTMLElement>(":scope > .toolbar");
    if (toolbar === null) throw new Error("製品ツールバーがありません");
    const horizontalOverflow = Math.max(
      0,
      toolbarIntrinsicWidth(toolbar) - CSS_VIEWPORT.width,
    );
    const permanentHorizontalScrollers = [
      [".timeline-controls", declarationBlock(".timeline-controls", viewerCss)],
      [".timeline-steps", declarationBlock(".timeline-steps", viewerCss)],
    ]
      .filter(([, block]) => block.includes("overflow-x: auto"))
      .map(([selector]) => selector);
    expect(
      { horizontalOverflow, permanentHorizontalScrollers },
      `200%時の横方向の内容超過は ${horizontalOverflow}px です。` +
        `常設の横送り指定は ${permanentHorizontalScrollers.join("、")} です。` +
        "overflow:hiddenは内容を切るだけで、横超過0の代わりにはなりません。",
    ).toEqual({ horizontalOverflow: 0, permanentHorizontalScrollers: [] });
  });
});
