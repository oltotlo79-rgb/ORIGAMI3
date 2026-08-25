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
const ZOOM_200_MEDIA = "@media (max-width: 790px)";

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
const timelineComponentSource = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "..", "components", "Timeline.tsx"),
  "utf8",
);
const tauriConfig = JSON.parse(
  readFileSync(
    join(
      dirname(fileURLToPath(import.meta.url)),
      "..",
      "..",
      "src-tauri",
      "tauri.conf.json",
    ),
    "utf8",
  ),
) as {
  app: { windows: Array<{ minWidth: number; minHeight: number }> };
};
const normalWindowMinimum = tauriConfig.app.windows[0];
if (normalWindowMinimum === undefined) throw new Error("製品窓の設定がありません");
const TIMELINE_CONTROL_LABELS = [
  "⏮ 最初へ",
  "◀ 前へ",
  "▶ 再生",
  "次へ ▶",
  "⏸ 一時停止",
] as const;

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

function axisSpacing(value: string): { vertical: number; horizontal: number } {
  const parts = value.trim().split(/\s+/u);
  if (parts.length === 1) {
    const spacing = pxValue(parts[0] ?? "");
    return { vertical: spacing, horizontal: spacing };
  }
  if (parts.length === 2) {
    return {
      vertical: pxValue(parts[0] ?? ""),
      horizontal: pxValue(parts[1] ?? ""),
    };
  }
  throw new Error(`上下・左右の値ではありません: ${value}`);
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
const zoom200Rules = atRuleBlock(ZOOM_200_MEDIA, responsiveCss);
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

  const toolbar = app.querySelector<HTMLElement>(":scope > .toolbar");
  if (toolbar === null) throw new Error("製品Appの上部操作がありません");
  const toolbarHeight = toolbar200Metrics(toolbar).height;
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
    declarationBlock(".pane-3d", zoom200Rules),
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

interface Toolbar200Metrics {
  cellWidth: number;
  brandCellWidth: number;
  maxButtonWidth: number;
  brandWidth: number;
  height: number;
  horizontalOverflow: number;
}

function toolbar200Metrics(toolbar: HTMLElement): Toolbar200Metrics {
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

  const toolbarRule = declarationBlock(".toolbar", zoom200Rules);
  const buttonRule = declarationBlock(".toolbar button", zoom200Rules);
  const brandRule = declarationBlock(".toolbar-brand", zoom200Rules);
  const separatorRule200 = declarationBlock(".toolbar-separator", zoom200Rules);
  const markRule = declarationBlock(".toolbar-brand-mark", narrowRules);
  const baseButtonRule = declarationBlock(".toolbar button", baseLayoutCss);
  const genericButtonRule = declarationBlock(".app :where(button)", baseLayoutCss);
  const iconRule = declarationBlock(".toolbar-icon", baseLayoutCss);
  const strongRule = declarationBlock(".toolbar-brand-copy strong", narrowRules);

  expect(declarationValue(toolbarRule, "display")).toBe("grid");
  expect(declarationValue(separatorRule200, "display")).toBe("none");
  const columnsMatch = /^repeat\(([0-9]+),\s*minmax\(0,\s*1fr\)\)$/u.exec(
    declarationValue(toolbarRule, "grid-template-columns"),
  );
  if (columnsMatch === null) throw new Error("200%時の上部操作の列数が読めません");
  const columns = Number(columnsMatch[1]);
  const rowHeightMatch = /^minmax\(([0-9.]+px),\s*auto\)$/u.exec(
    declarationValue(toolbarRule, "grid-auto-rows"),
  );
  if (rowHeightMatch === null) throw new Error("200%時の上部操作の行高が読めません");
  const rowHeight = pxValue(rowHeightMatch[1]);
  const brandSpanMatch = /^span\s+([0-9]+)$/u.exec(
    declarationValue(brandRule, "grid-column"),
  );
  if (brandSpanMatch === null) throw new Error("200%時の作品名の列数が読めません");
  const brandSpan = Number(brandSpanMatch[1]);
  const toolbarPadding = axisSpacing(declarationValue(toolbarRule, "padding"));
  const toolbarGap = pxValue(declarationValue(toolbarRule, "gap"));
  const buttonPadding = pxValue(declarationValue(buttonRule, "padding-inline"));
  const buttonGap = pxValue(declarationValue(baseButtonRule, "gap"));
  const buttonBorder = tokenPx(
    /var\((--[a-z0-9-]+)\)/u.exec(declarationValue(genericButtonRule, "border"))?.[1] ??
      "",
  );
  const iconWidth = pxValue(declarationValue(iconRule, "width"));
  const fontSize = tokenPx("--fs-md");
  const buttonWidths = buttons.map(
    (button) =>
      [...(button.textContent?.trim() ?? "")].length * fontSize +
      buttonPadding * 2 +
      buttonBorder * 2 +
      iconWidth +
      buttonGap,
  );
  const cellWidth =
    (CSS_VIEWPORT.width -
      toolbarPadding.horizontal * 2 -
      toolbarGap * (columns - 1)) /
    columns;
  const maxButtonWidth = Math.max(...buttonWidths);

  const brandStrong = brand.querySelector<HTMLElement>("strong");
  if (brandStrong === null) throw new Error("製品名の文字がありません");
  const brandFontSize = pxValue(declarationValue(strongRule, "font-size"));
  const brandWidth =
    pxValue(declarationValue(markRule, "width")) +
    pxValue(declarationValue(brandRule, "gap")) +
    pxValue(declarationValue(brandRule, "padding-right")) +
    tokenPx("--brand-divider") +
    [...(brandStrong.textContent?.trim() ?? "")].length * brandFontSize;
  const brandCellWidth = cellWidth * brandSpan + toolbarGap * (brandSpan - 1);
  const rows = Math.ceil((brandSpan + buttons.length) / columns);
  const height =
    rowHeight * rows +
    toolbarGap * (rows - 1) +
    toolbarPadding.vertical * 2 +
    tokenPx("--panel-border-width");
  const horizontalOverflow = Math.max(
    0,
    maxButtonWidth - cellWidth,
    brandWidth - brandCellWidth,
  );

  return {
    cellWidth,
    brandCellWidth,
    maxButtonWidth,
    brandWidth,
    height,
    horizontalOverflow,
  };
}

interface Timeline200Metrics {
  cellWidth: number;
  maxControlWidth: number;
  horizontalOverflow: number;
}

function timeline200Metrics(app: HTMLElement): Timeline200Metrics {
  const workspace = workspaceMetrics(app);
  const timelineRule = declarationBlock(".timeline", zoom200Rules);
  const controlsRule = declarationBlock(".timeline-controls", zoom200Rules);
  const timelinePadding = axisSpacing(declarationValue(timelineRule, "padding"));
  const columnsMatch = /^repeat\(([0-9]+),\s*minmax\(0,\s*1fr\)\)$/u.exec(
    declarationValue(controlsRule, "grid-template-columns"),
  );
  if (columnsMatch === null) throw new Error("200%時の時間軸の列数が読めません");
  const columns = Number(columnsMatch[1]);
  const gap = pxValue(declarationValue(controlsRule, "gap"));
  const padding = pxValue(
    declarationValue(
      declarationBlock(".timeline-controls button", zoom200Rules),
      "padding-inline",
    ),
  );
  const cellWidth =
    (workspace.pane3dWidth -
      timelinePadding.horizontal * 2 -
      gap * (columns - 1)) /
    columns;
  for (const label of TIMELINE_CONTROL_LABELS) {
    expect(timelineComponentSource).toContain(label);
  }
  const maxControlWidth = Math.max(
    ...TIMELINE_CONTROL_LABELS.map(
      (label) =>
        [...label].length * tokenPx("--fs-sm") +
        padding * 2 +
        tokenPx("--control-border-width") * 2,
    ),
  );
  return {
    cellWidth,
    maxControlWidth,
    horizontalOverflow: Math.max(0, maxControlWidth - cellWidth),
  };
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
    expectDeclarations(declarationBlock(".toolbar", baseLayoutCss), [
      "display: flex",
      "min-height: 52px",
    ]);
    expect(declarationBlock(".toolbar button", baseLayoutCss)).toContain(
      "white-space: nowrap",
    );
    expect(PHYSICAL_VIEWPORT.width).toBeGreaterThan(CSS_VIEWPORT.width);
    expect(PHYSICAL_VIEWPORT.height).toBeGreaterThan(CSS_VIEWPORT.height);
    expect(normalWindowMinimum.minWidth).toBeGreaterThan(790);
    expect(normalWindowMinimum.minHeight).toBe(PHYSICAL_VIEWPORT.height);
    expect(zoom200Rules).not.toMatch(/(?:font-size|\bscale)\s*:/u);
    expect(zoom200Rules).not.toMatch(/transform\s*:[^;]*scale/u);
    expectDeclarations(declarationBlock(".toolbar", zoom200Rules), [
      "display: grid",
      "grid-template-columns: repeat(5, minmax(0, 1fr))",
      "grid-auto-rows: minmax(34px, auto)",
    ]);

    const metrics = toolbar200Metrics(toolbar);
    expect(
      {
        maxButtonWidth: metrics.maxButtonWidth,
        buttonCellWidth: metrics.cellWidth,
        brandWidth: metrics.brandWidth,
        brandCellWidth: metrics.brandCellWidth,
        horizontalOverflow: metrics.horizontalOverflow,
      },
      "200%時の5列×2段で、文字・アイコン・余白を含む各要素をセル内へ収めます。",
    ).toEqual({
      maxButtonWidth: expect.any(Number),
      buttonCellWidth: expect.any(Number),
      brandWidth: expect.any(Number),
      brandCellWidth: expect.any(Number),
      horizontalOverflow: 0,
    });
    expect(metrics.maxButtonWidth).toBeLessThanOrEqual(metrics.cellWidth);
    expect(metrics.brandWidth).toBeLessThanOrEqual(metrics.brandCellWidth);
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
    expectDeclarations(declarationBlock(".toolbar", zoom200Rules), [
      "display: grid",
      "grid-template-columns: repeat(5, minmax(0, 1fr))",
      "gap: var(--sp-1)",
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
    expectDeclarations(declarationBlock(".timeline-controls", zoom200Rules), [
      "display: grid",
      "grid-template-columns: repeat(2, minmax(0, 1fr))",
      "gap: var(--sp-1)",
      "overflow-x: hidden",
      "overflow-y: auto",
    ]);
    expectDeclarations(declarationBlock(".pane-3d", zoom200Rules), [
      "grid-template-rows: minmax(0, 1fr) 60px",
    ]);
    expectDeclarations(declarationBlock(".timeline", zoom200Rules), [
      "overflow-x: hidden",
      "overflow-y: auto",
    ]);
    expectDeclarations(declarationBlock(".timeline-steps", zoom200Rules), [
      "flex-wrap: wrap",
      "overflow-x: hidden",
      "overflow-y: auto",
    ]);
    expectDeclarations(declarationBlock(".timeline-slot", zoom200Rules), [
      "flex: 1 1 100%",
      "min-width: 0",
      "max-width: 100%",
    ]);
    expectDeclarations(
      declarationBlock(".timeline-slot > .timeline-chip", zoom200Rules),
      [
        "flex: 1 1 auto",
        "min-width: 0",
        "max-width: 100%",
        "white-space: normal",
        "overflow-wrap: anywhere",
      ],
    );
    const timelineMetrics = timeline200Metrics(app);
    expect(timelineMetrics.maxControlWidth).toBeLessThanOrEqual(
      timelineMetrics.cellWidth,
    );
    expect(timelineMetrics.horizontalOverflow).toBe(0);
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
    expect(viewerComponentSource).toContain("視点を戻す");
    const metrics = workspaceMetrics(app);
    expect(metrics.pane3dWidth).toBeLessThanOrEqual(360);
    const overlayRule = declarationBlock(".viewer-overlay-region", zoom200Rules);
    const overlayTop = pxValue(declarationValue(overlayRule, "top"));
    const overlayRight = pxValue(declarationValue(overlayRule, "right"));
    const overlayBottom = pxValue(declarationValue(overlayRule, "bottom"));
    const overlayLeft = pxValue(declarationValue(overlayRule, "left"));
    const missingOverlayHeight = Math.max(
      0,
      overlayTop + overlayBottom - metrics.viewerHeight,
    );

    const cubeRule = declarationBlock(".view-cube", zoom200Rules);
    const resetRule = declarationBlock(".viewer-reset", zoom200Rules);
    const resetBaseRule = declarationBlock(".viewer-reset", baseLayoutCss);
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
      fullWidthJapaneseCount("視点を戻す") * tokenPx("--fs-sm") +
      pxValue(declarationValue(resetRule, "padding-inline")) * 2 +
      tokenPx("--control-border-width") * 2;
    const reset = {
      left: pxValue(declarationValue(resetRule, "left")),
      top:
        metrics.viewerHeight -
        pxValue(declarationValue(resetRule, "bottom")) -
        pxValue(declarationValue(resetBaseRule, "min-height")),
      width: resetWidth,
      height: pxValue(declarationValue(resetBaseRule, "min-height")),
    };
    const overlay = {
      left: overlayLeft,
      top: overlayTop,
      width: metrics.pane3dWidth - overlayLeft - overlayRight,
      height: metrics.viewerHeight - overlayTop - overlayBottom,
    };
    const overlapDimensions = (
      first: { left: number; top: number; width: number; height: number },
      second: { left: number; top: number; width: number; height: number },
    ): [number, number] => [
      Math.max(
        0,
        Math.min(first.left + first.width, second.left + second.width) -
          Math.max(first.left, second.left),
      ),
      Math.max(
        0,
        Math.min(first.top + first.height, second.top + second.height) -
          Math.max(first.top, second.top),
      ),
    ];
    const cubeResetOverlap = overlapDimensions(cube, reset);
    const overlayCubeOverlap = overlapDimensions(overlay, cube);
    const overlayResetOverlap = overlapDimensions(overlay, reset);
    const overlapAreas = [
      cubeResetOverlap[0] * cubeResetOverlap[1],
      overlayCubeOverlap[0] * overlayCubeOverlap[1],
      overlayResetOverlap[0] * overlayResetOverlap[1],
    ];
    const safeGaps = {
      overlayToCube: cube.left - (overlay.left + overlay.width),
      overlayToReset: reset.top - (overlay.top + overlay.height),
      resetToCube: cube.left - (reset.left + reset.width),
    };
    expect(overlay.width).toBeGreaterThan(0);
    expect(overlay.height).toBeGreaterThan(0);
    for (const gap of Object.values(safeGaps)) {
      expect(gap).toBeGreaterThanOrEqual(tokenPx("--sp-2"));
    }
    expect(
      { overlapAreas, safeGaps, missingOverlayHeight },
      `3D本体は${metrics.viewerHeight.toFixed(2)}px高です。` +
        `視点立方体と「視点を戻す」の保守的な重なりは` +
        `${cubeResetOverlap[0].toFixed(2)}×${cubeResetOverlap[1].toFixed(2)}px、` +
        `札領域は高さが${missingOverlayHeight.toFixed(2)}px不足します。`,
    ).toEqual({
      overlapAreas: [0, 0, 0],
      safeGaps: {
        overlayToCube: expect.any(Number),
        overlayToReset: expect.any(Number),
        resetToCube: expect.any(Number),
      },
      missingOverlayHeight: 0,
    });
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
    const toolbarOverflow = toolbar200Metrics(toolbar).horizontalOverflow;
    const timelineOverflow = timeline200Metrics(app).horizontalOverflow;
    const horizontalOverflow = Math.max(toolbarOverflow, timelineOverflow);
    expectDeclarations(
      declarationBlock(".timeline-slot > .timeline-chip", zoom200Rules),
      ["min-width: 0", "max-width: 100%", "overflow-wrap: anywhere"],
    );
    expect(declarationValue(declarationBlock(".timeline-controls", viewerCss), "overflow-x"))
      .toBe("auto");
    expect(declarationValue(declarationBlock(".timeline-steps", viewerCss), "overflow-x"))
      .toBe("auto");
    const permanentHorizontalScrollers = [
      [".timeline-controls", declarationBlock(".timeline-controls", zoom200Rules)],
      [".timeline-steps", declarationBlock(".timeline-steps", zoom200Rules)],
    ]
      .filter(([, block]) =>
        /overflow-x:\s*(?:auto|scroll)/u.test(block),
      )
      .map(([selector]) => selector);
    expect(
      { horizontalOverflow, permanentHorizontalScrollers },
      `200%時の横方向の内容超過は ${horizontalOverflow}px です。` +
        `常設の横送り指定は ${permanentHorizontalScrollers.join("、")} です。` +
        "overflow:hiddenは内容を切るだけで、横超過0の代わりにはなりません。",
    ).toEqual({ horizontalOverflow: 0, permanentHorizontalScrollers: [] });
  });
});
