// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { useRef, type ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { CpEditor } from "../components/CpEditor/CpEditor";
import { RecoveryDialog } from "../components/RecoveryDialog";
import { ExportDialog } from "../components/dialogs/ExportDialog";
import { HelpCenter } from "../components/dialogs/HelpCenter";
import {
  focusableElements,
  type FocusTarget,
} from "../components/dialogs/ModalDialog";
import { NewDocumentDialog } from "../components/dialogs/NewDocumentDialog";
import { ProposalWizard } from "../components/dialogs/ProposalWizard";
import {
  LENGTH_RANGE,
  ROOT_ID,
  WIDTH_RANGE,
  addLimb,
  defaultSkeleton,
  leafNodes,
} from "./skeleton";
import type {
  CreasePattern,
  Document,
  FoldStep,
  ProposalCandidate,
  Skeleton,
} from "./types";
import { useAppStore } from "../store/appStore";

/*
 * jsdomには配置計算がない。この検査は次の証拠を混ぜない。
 *
 * - 各状態では製品の実コンポーネントをmountし、名前、見出し、実focus対象、
 *   keyboard eventの到達をDOMで確かめる。
 * - 500×350 CSS pxへ収める仕組みは、規則の持ち主であるCSSを直接読む。
 * - getBoundingClientRectやscrollWidthへ偽の値を入れて、重なりや切れが無いとは
 *   主張しない。最終的な見た目は同梱版の実画面確認で確かめる。
 */

vi.mock("../components/CpEditor/renderer", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../components/CpEditor/renderer")>();
  return { ...actual, render: vi.fn() };
});

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

const cssByOwner = {
  tokens: cssSource("tokens.css"),
  themes: cssSource("themes.css"),
  baseLayout: cssSource("base-layout.css"),
  viewer: cssSource("viewer.css"),
  context: cssSource("context.css"),
  dialogs: cssSource("dialogs.css"),
  responsive: cssSource("responsive.css"),
} as const;
// index.cssのlayer順。詳しさよりlayer優先度が先に効くため、連結順も同じにする。
const allOwnedCss = Object.values(cssByOwner).join("\n");
const dialogsCss = cssByOwner.dialogs;
const responsiveCss = cssByOwner.responsive;
const viewerCss = cssByOwner.viewer;

function escaped(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

function declarationBlock(selector: string, source: string): string {
  const match = new RegExp(
    `${escaped(selector)}\\s*\\{([\\s\\S]*?)\\}`,
    "u",
  ).exec(source);
  if (match === null) throw new Error(`CSSブロックがありません: ${selector}`);
  return match[1];
}

function lastDeclarationBlock(selector: string, source: string): string {
  const matches = Array.from(
    source.matchAll(
      new RegExp(`${escaped(selector)}\\s*\\{([\\s\\S]*?)\\}`, "gu"),
    ),
  );
  const block = matches[matches.length - 1]?.[1];
  if (block === undefined) throw new Error(`CSSブロックがありません: ${selector}`);
  return block;
}

function optionalLastDeclarationBlock(
  selector: string,
  source: string,
): string | null {
  const matches = Array.from(
    source.matchAll(
      new RegExp(`${escaped(selector)}\\s*\\{([\\s\\S]*?)\\}`, "gu"),
    ),
  );
  return matches[matches.length - 1]?.[1] ?? null;
}

function optionalDeclarationValue(
  block: string | null,
  property: string,
): string | null {
  if (block === null) return null;
  const match = new RegExp(
    `(?:^|[;\\n])\\s*${escaped(property)}\\s*:\\s*([^;]+)`,
    "u",
  ).exec(block);
  return match?.[1]?.trim() ?? null;
}

function atRuleBlock(prelude: string, source: string): string {
  const start = source.indexOf(prelude);
  if (start < 0) throw new Error(`CSS規則がありません: ${prelude}`);
  const opening = source.indexOf("{", start + prelude.length);
  if (opening < 0) throw new Error(`CSS規則の開始括弧がありません: ${prelude}`);
  let depth = 0;
  for (let index = opening; index < source.length; index += 1) {
    if (source[index] === "{") depth += 1;
    if (source[index] === "}") depth -= 1;
    if (depth === 0) return source.slice(opening + 1, index);
  }
  throw new Error(`CSS規則の終了括弧がありません: ${prelude}`);
}

function expectDeclarations(block: string, expected: readonly string[]): void {
  for (const declaration of expected) expect(block).toContain(declaration);
}

function makeCp(mark: number): CreasePattern {
  return {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [1, 0] },
      { id: 2, pos: [1, 1] },
      { id: 3, pos: [0, 1] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Border" },
      { id: 2, v0: 2, v1: 3, kind: "Border" },
      { id: 3, v0: 3, v1: 0, kind: "Border" },
    ],
    next_vertex_id: 4,
    next_edge_id: mark,
  };
}

function makeSteps(count: number): FoldStep[] {
  return Array.from({ length: count }, (_, index) => ({
    id: index + 1,
    kind: "Simple",
    drivers: [],
    layer_order: null,
    note: "",
  }));
}

const BASE_DOCUMENT: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: makeCp(4),
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

const LONG_EXPORT_BASENAME =
  "折り鶴の展開図_利用者調査で文字切れを確認するための非常に長い作品名_最終版.png";
const LONG_EXPORT_ERROR =
  "選んだ場所へ保存できませんでした。空き容量と書き込みできる場所かを確かめてから、別の保存先でもう一度試してください。";

function makeCandidate(
  mark: number,
  leafIds: readonly number[] = [],
): ProposalCandidate {
  return {
    cp: makeCp(mark),
    scale: 0.4,
    violations: 0,
    warnings: [],
    fold_plan: null,
    sites: leafIds.map((leafId, index) => ({
      circle: {
        leaf_id: leafId,
        circle_index: index,
        center: [
          ((index % 4) + 0.5) / 4,
          (Math.floor(index / 4) + 0.5) / Math.max(1, Math.ceil(leafIds.length / 4)),
        ] as [number, number],
        radius: 0.04,
      },
      vertex: null,
      molecules: [],
    })),
  };
}

function withLargestParts(skeleton: Skeleton): Skeleton {
  return {
    nodes: skeleton.nodes.map((node) =>
      node.parent === null
        ? node
        : {
            ...node,
            length: LENGTH_RANGE.max,
            width_factor: WIDTH_RANGE.max,
          },
    ),
  };
}

/** 5-Aで固定した「深い形」: 深さ4、先端12本の最大標本。 */
function deepTwelveTipBranch(): Skeleton {
  let skeleton = defaultSkeleton();
  skeleton = addLimb(skeleton, 1);
  const second = skeleton.nodes.find((node) => node.parent === 1);
  if (second === undefined) throw new Error("深い形の2段目がありません");
  skeleton = addLimb(skeleton, second.id);
  const third = skeleton.nodes.find((node) => node.parent === second.id);
  if (third === undefined) throw new Error("深い形の3段目がありません");
  for (let index = 0; index < 9; index += 1) {
    skeleton = addLimb(skeleton, third.id);
  }
  expect(leafNodes(skeleton)).toHaveLength(12);
  return withLargestParts(skeleton);
}

function radialSkeleton(count: number): Skeleton {
  return {
    nodes: [
      { id: ROOT_ID, parent: null, length: 0, width_factor: 1 },
      ...Array.from({ length: count }, (_, index) => ({
        id: index + 1,
        parent: ROOT_ID,
        length: 1,
        width_factor: 1,
      })),
    ],
  };
}

function resetVisibleState(): void {
  useAppStore.setState({
    doc: structuredClone(BASE_DOCUMENT),
    recovery: null,
    newDialogOpen: false,
    proposalStep: null,
    proposalSkeleton: defaultSkeleton(),
    proposalCandidates: [],
    proposalSelected: null,
    proposalPaperSource: null,
    proposalPaperPositions: [],
    proposalPaperSpecified: [],
    proposalPositionLastMoved: [],
    proposalPositionUndoStack: [],
    proposalPositionRedoStack: [],
    proposalBusy: false,
    proposalJobId: null,
    proposalProgress: null,
    proposalProgressWarning: null,
    proposalError: null,
    exportOpen: false,
    exportBusy: false,
    exportError: null,
    exportSavedPath: null,
    helpOpen: false,
    helpQuery: "",
    guideOpen: false,
    activeTool: "select",
    operationStage: 0,
    lineInputStart: null,
  });
}

function CpLineStartState(): ReactElement {
  const fitRef = useRef<(() => void) | null>(null);
  return (
    <section className="pane pane-2d" data-ten-state="cp-line-start">
      <CpEditor fitRef={fitRef} />
    </section>
  );
}

interface TenState {
  id: string;
  label: string;
  floatingUi: string | null;
  dialogName: string | null;
  headingName: string;
  expectedOperationCount: number;
  prepare: () => void;
  view: () => ReactElement;
  verticalSelectors: readonly string[];
}

const TEN_STATES: readonly TenState[] = [
  {
    id: "new-square",
    label: "新規作成・既定の正方形",
    floatingUi: "new-document-dialog",
    dialogName: "新しい紙を用意する",
    headingName: "新しい紙を用意する",
    expectedOperationCount: 9,
    prepare: () => {
      useAppStore.setState({
        newDialogOpen: true,
        newPaperDraft: { widthMm: 150, heightMm: 150, square: true },
      });
    },
    view: () => <NewDocumentDialog />,
    verticalSelectors: [".dialog"],
  },
  {
    id: "export-png-long-messages",
    label: "書き出し・PNGと長い成功／失敗文",
    floatingUi: "export-dialog",
    dialogName: "展開図・折り図を書き出す",
    headingName: "展開図・折り図を書き出す",
    expectedOperationCount: 7,
    prepare: () => {
      useAppStore.setState({
        doc: { ...structuredClone(BASE_DOCUMENT), sequence: makeSteps(1) },
        exportOpen: true,
        exportKind: "CpPng",
        exportIncludeAux: true,
        exportLongSide: 2048,
        exportSavedPath:
          `C:\\利用者\\折り鶴の作品\\とても長い名前の展開図を保存した場所\\${LONG_EXPORT_BASENAME}`,
        exportError: LONG_EXPORT_ERROR,
      });
    },
    view: () => <ExportDialog />,
    verticalSelectors: [".dialog"],
  },
  {
    id: "proposal-skeleton-deep",
    label: "提案・形を決める（深い形）",
    floatingUi: "proposal-dialog",
    dialogName: "形を決めて展開図を作ってもらう",
    headingName: "形を決めて展開図を作ってもらう",
    expectedOperationCount: 68,
    prepare: () => {
      useAppStore.setState({
        proposalStep: "skeleton",
        proposalSkeleton: deepTwelveTipBranch(),
      });
    },
    view: () => <ProposalWizard />,
    verticalSelectors: [".dialog-wide"],
  },
  {
    id: "proposal-four-candidates",
    label: "提案・4候補",
    floatingUi: "proposal-dialog",
    dialogName: "形を決めて展開図を作ってもらう",
    headingName: "形を決めて展開図を作ってもらう",
    expectedOperationCount: 8,
    prepare: () => {
      const skeleton = radialSkeleton(4);
      const leaves = leafNodes(skeleton).map((node) => node.id);
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: skeleton,
        proposalCandidates: Array.from({ length: 4 }, (_, index) =>
          makeCandidate(20 + index, leaves),
        ),
        proposalSelected: 0,
      });
    },
    view: () => <ProposalWizard />,
    verticalSelectors: [".dialog-wide"],
  },
  {
    id: "proposal-paper-twelve-handles",
    label: "提案・紙上の12個の場所",
    floatingUi: "proposal-dialog",
    dialogName: "紙の上の場所を調整",
    headingName: "紙の上の場所を調整",
    expectedOperationCount: 14,
    prepare: () => {
      const skeleton = radialSkeleton(12);
      const leaves = leafNodes(skeleton).map((node) => node.id);
      const candidate = makeCandidate(30, leaves);
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: skeleton,
        proposalCandidates: [candidate],
        proposalSelected: 0,
      });
      useAppStore.getState().openProposalPaperPositionEditor();
    },
    view: () => <ProposalWizard />,
    verticalSelectors: [],
  },
  {
    id: "proposal-confirm-warning",
    label: "提案・確認と既存手順消去警告",
    floatingUi: "proposal-dialog",
    dialogName: "形を決めて展開図を作ってもらう",
    headingName: "形を決めて展開図を作ってもらう",
    expectedOperationCount: 2,
    prepare: () => {
      useAppStore.setState({
        doc: { ...structuredClone(BASE_DOCUMENT), sequence: makeSteps(3) },
        proposalStep: "confirm",
        proposalCandidates: [makeCandidate(40)],
        proposalSelected: 0,
      });
    },
    view: () => <ProposalWizard />,
    verticalSelectors: [".dialog-wide"],
  },
  {
    id: "proposal-busy",
    label: "提案・処理中",
    floatingUi: "proposal-dialog",
    dialogName: "形を決めて展開図を作ってもらう",
    headingName: "形を決めて展開図を作ってもらう",
    expectedOperationCount: 18,
    prepare: () => {
      useAppStore.setState({
        proposalStep: "skeleton",
        proposalSkeleton: defaultSkeleton(),
        proposalBusy: true,
        proposalJobId: "ten-state-busy",
        proposalProgress: {
          job_id: "ten-state-busy",
          done: 2,
          total: 4,
          phase: "Generating",
        },
      });
    },
    view: () => <ProposalWizard />,
    verticalSelectors: [".dialog-wide"],
  },
  {
    id: "recovery-long-path",
    label: "復旧・長い実パス",
    floatingUi: "recovery-dialog",
    dialogName: "前回の終了が正常に行われませんでした",
    headingName: "前回の終了が正常に行われませんでした",
    expectedOperationCount: 2,
    prepare: () => {
      useAppStore.setState({
        recovery: {
          autosave_path:
            "C:\\Users\\利用者\\AppData\\Local\\ORIGAMI3\\控え\\非常に長い控えの名前.ori3",
          document_path:
            "C:\\Users\\利用者\\Documents\\折り紙作品\\2026年の展示会\\利用者調査で確認する非常に長い作品名\\折り鶴の最終作品.ori3",
          saved_at_ms: Date.UTC(2026, 7, 24, 12, 34, 56),
        },
      });
    },
    view: () => <RecoveryDialog />,
    verticalSelectors: [".dialog"],
  },
  {
    id: "help-all-thirteen-chapters",
    label: "ヘルプ・全13章",
    floatingUi: "help-dialog",
    dialogName: "ヘルプセンター",
    headingName: "ヘルプセンター",
    expectedOperationCount: 16,
    prepare: () => {
      useAppStore.setState({
        helpOpen: true,
        helpChapterId: "overview",
        helpQuery: "",
      });
    },
    view: () => <HelpCenter />,
    verticalSelectors: [".help-toc", ".help-content"],
  },
  {
    id: "cp-keyboard-line-start",
    label: "展開図・キーボードで始点を決めた途中状態",
    floatingUi: null,
    dialogName: null,
    headingName: "展開図",
    expectedOperationCount: 3,
    prepare: () => {
      useAppStore.setState({
        doc: structuredClone(BASE_DOCUMENT),
        activeTool: "mountain",
        operationStage: 0,
        lineInputStart: null,
      });
    },
    view: () => <CpLineStartState />,
    verticalSelectors: [],
  },
] as const;

const initialStoreState = useAppStore.getState();
const originalViewport = {
  innerWidth: Object.getOwnPropertyDescriptor(window, "innerWidth"),
  innerHeight: Object.getOwnPropertyDescriptor(window, "innerHeight"),
  devicePixelRatio: Object.getOwnPropertyDescriptor(window, "devicePixelRatio"),
};
const originalResizeObserver = globalThis.ResizeObserver;
const originalCanvasContext = HTMLCanvasElement.prototype.getContext;
const originalCanvasWidth = Object.getOwnPropertyDescriptor(
  HTMLCanvasElement.prototype,
  "clientWidth",
);
const originalCanvasHeight = Object.getOwnPropertyDescriptor(
  HTMLCanvasElement.prototype,
  "clientHeight",
);

beforeEach(() => {
  Object.defineProperties(window, {
    innerWidth: { configurable: true, value: CSS_VIEWPORT.width },
    innerHeight: { configurable: true, value: CSS_VIEWPORT.height },
    devicePixelRatio: { configurable: true, value: ZOOM },
  });
  Object.defineProperties(HTMLCanvasElement.prototype, {
    clientWidth: { configurable: true, value: CSS_VIEWPORT.width },
    clientHeight: { configurable: true, value: CSS_VIEWPORT.height },
  });
  HTMLCanvasElement.prototype.getContext = vi.fn(() => ({})) as never;
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  resetVisibleState();
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
  for (const [property, descriptor] of Object.entries(originalViewport)) {
    if (descriptor !== undefined) Object.defineProperty(window, property, descriptor);
  }
  if (originalCanvasWidth !== undefined) {
    Object.defineProperty(
      HTMLCanvasElement.prototype,
      "clientWidth",
      originalCanvasWidth,
    );
  }
  if (originalCanvasHeight !== undefined) {
    Object.defineProperty(
      HTMLCanvasElement.prototype,
      "clientHeight",
      originalCanvasHeight,
    );
  }
  HTMLCanvasElement.prototype.getContext = originalCanvasContext;
  globalThis.ResizeObserver = originalResizeObserver;
});

function mountState(state: TenState): HTMLElement {
  state.prepare();
  render(state.view());
  expect(window.innerWidth).toBe(500);
  expect(window.innerHeight).toBe(350);
  expect(window.devicePixelRatio).toBe(2);

  if (state.floatingUi !== null) {
    const root = document.querySelector<HTMLElement>(
      `[data-floating-ui="${state.floatingUi}"]`,
    );
    if (root === null) throw new Error(`${state.label}の製品画面がありません`);
    return root;
  }
  const cp = document.querySelector<HTMLElement>(
    '[data-ten-state="cp-line-start"] .cp-editor',
  );
  if (cp === null) throw new Error("展開図の製品画面がありません");
  return cp;
}

function visibleName(element: Element): string {
  const labelledBy = element.getAttribute("aria-labelledby");
  const labelledText = labelledBy
    ?.split(/\s+/u)
    .map((id) => document.getElementById(id)?.textContent ?? "")
    .join(" ");
  const ownerLabel = element.closest("label")?.textContent;
  const explicitLabel =
    element.id === ""
      ? null
      : [...document.querySelectorAll<HTMLLabelElement>("label")].find(
          (label) => label.htmlFor === element.id,
        )?.textContent;
  return (
    [
      element.getAttribute("aria-label"),
      labelledText,
      explicitLabel,
      ownerLabel,
      element.getAttribute("data-tooltip"),
      element.textContent,
    ]
      .find(
        (value): value is string =>
          typeof value === "string" && value.trim() !== "",
      )
      ?.replace(/\s+/gu, " ")
      .trim() ?? ""
  );
}

function actualOperations(root: HTMLElement): FocusTarget[] {
  if (root.classList.contains("dialog")) return focusableElements(root);
  return Array.from(
    root.querySelectorAll<FocusTarget>(
      "button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), canvas[tabindex], [role='button'][tabindex], [role='slider'][tabindex], [tabindex='0']",
    ),
  ).filter((element, index, all) => all.indexOf(element) === index);
}

const INDEPENDENT_SEMANTIC_SELECTOR = [
  "a[href]",
  "area[href]",
  "button",
  "input:not([type='hidden'])",
  "select",
  "textarea",
  "iframe",
  "object",
  "embed",
  "summary",
  "audio[controls]",
  "video[controls]",
  "[contenteditable]",
  "canvas",
  "[role='button']",
  "[role='slider']",
].join(",");

/** 共通Modal実装を自己参照せず、tabIndexより先にDOMの意味から操作を列挙する。 */
function semanticActions(root: HTMLElement): Element[] {
  const candidates = Array.from(
    root.querySelectorAll<Element>(INDEPENDENT_SEMANTIC_SELECTOR),
  ).filter((element) => {
    if (element.getAttribute("aria-disabled") === "true") return false;
    if (
      "disabled" in element &&
      (element as Element & { disabled?: boolean }).disabled === true
    ) {
      return false;
    }
    if (element.closest("[hidden], [aria-hidden='true'], [inert]")) return false;
    return true;
  });

  const radioStops = new Map<string, HTMLInputElement>();
  for (const element of candidates) {
    if (!(element instanceof HTMLInputElement) || element.type !== "radio") continue;
    const key = `${element.form?.id ?? ""}\u0000${element.name}`;
    const current = radioStops.get(key);
    if (current === undefined || (!current.checked && element.checked)) {
      radioStops.set(key, element);
    }
  }
  return candidates.filter(
    (element) =>
      !(element instanceof HTMLInputElement) ||
      element.type !== "radio" ||
      radioStops.get(`${element.form?.id ?? ""}\u0000${element.name}`) === element,
  );
}

/**
 * 配送だけを検査する。製品actionが動いた証拠とはせず、各状態の結果検査は
 * keyboard-only側へ分離する（展開図の始点だけはこのファイルでも実結果を見る）。
 */
function dispatchKeyboardEvent(target: FocusTarget): void {
  const isSlider =
    target.getAttribute("role") === "slider" ||
    (target instanceof HTMLInputElement && target.type === "range");
  const key = isSlider ? "ArrowRight" : target instanceof HTMLCanvasElement ? "ArrowRight" : "Enter";
  const down = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
  });
  const up = new KeyboardEvent("keyup", { key, bubbles: true });
  fireEvent(target, down);
  fireEvent(target, up);
  expect(down.target, `${visibleName(target)}へのkeydown配送`).toBe(target);
  expect(up.target, `${visibleName(target)}へのkeyup配送`).toBe(target);
}

describe("5-Aで固定した200%の10状態", () => {
  it("状態を増減させず、Help全13章とProposal 5状態を個別rowに固定する", () => {
    expect(TEN_STATES).toHaveLength(10);
    expect(TEN_STATES.map((state) => state.id)).toEqual([
      "new-square",
      "export-png-long-messages",
      "proposal-skeleton-deep",
      "proposal-four-candidates",
      "proposal-paper-twelve-handles",
      "proposal-confirm-warning",
      "proposal-busy",
      "recovery-long-path",
      "help-all-thirteen-chapters",
      "cp-keyboard-line-start",
    ]);
    expect(TEN_STATES.filter((state) => state.id.startsWith("proposal-"))).toHaveLength(5);
  });

  it.each(TEN_STATES)(
    "$label: 名前付き画面・見出し・実操作を欠かさない",
    (state) => {
      const root = mountState(state);
      if (state.dialogName !== null) {
        expect(
          screen.getByRole("dialog", { name: state.dialogName }),
        ).toBe(root);
      } else {
        expect(
          screen.getByLabelText(/展開図。矢印キーで位置を動かし/u),
        ).toBeInstanceOf(HTMLCanvasElement);
      }
      if (state.id === "cp-keyboard-line-start") {
        // 常設の展開図はdialogではない。画面名は操作案内の先頭へ常時表示する。
        expect(root.querySelector(".cp-operation-current > span")?.textContent).toBe(
          state.headingName,
        );
      } else {
        expect(
          screen.getByRole("heading", { name: state.headingName }),
          `${state.label}の見出し`,
        ).not.toBeNull();
      }

      if (state.id === "help-all-thirteen-chapters") {
        expect(screen.getByText("全13章")).not.toBeNull();
        expect(root.querySelectorAll(".help-toc > button")).toHaveLength(13);
      }
      if (state.id === "export-png-long-messages") {
        expect(LONG_EXPORT_BASENAME.length).toBeGreaterThan(40);
        expect(root.textContent).toContain(
          `保存しました:${LONG_EXPORT_BASENAME}`,
        );
        expect(root.textContent).toContain(
          `保存できませんでした:${LONG_EXPORT_ERROR}`,
        );
      }
      if (state.id === "proposal-paper-twelve-handles") {
        expect(
          root.querySelectorAll("[data-paper-position-handle]"),
        ).toHaveLength(12);
      }
      if (state.id === "proposal-confirm-warning") {
        expect(root.textContent).toContain("今ある折り手順3件はすべて消えます");
      }
      if (state.id === "proposal-busy") {
        expect(root.querySelector("[data-proposal-progress]")).not.toBeNull();
        expect(root.textContent).toContain("4件中2件め");
      }
      if (state.id === "recovery-long-path") {
        const path = useAppStore.getState().recovery?.document_path ?? "";
        expect(path.length).toBeGreaterThan(60);
        expect(root.textContent).toContain("元の作品:折り鶴の最終作品.ori3");
      }
      if (state.id === "cp-keyboard-line-start") {
        const canvas = root.querySelector<HTMLCanvasElement>("canvas.cp-canvas");
        if (canvas === null) throw new Error("展開図の操作面がありません");
        act(() => canvas.focus());
        fireEvent.keyDown(canvas, { key: "Enter" });
        expect(useAppStore.getState().lineInputStart).not.toBeNull();
        expect(useAppStore.getState().operationStage).toBe(1);
      }

      const operations = actualOperations(root);
      const independentActions = semanticActions(root);
      for (const action of independentActions) {
        const tabIndex =
          "tabIndex" in action
            ? (action as Element & { tabIndex?: unknown }).tabIndex
            : undefined;
        expect(
          tabIndex,
          `${state.label}: ${visibleName(action)}のtabIndex`,
        ).toBeTypeOf("number");
        expect(
          tabIndex as number,
          `${state.label}: ${visibleName(action)}がTab順から外れています`,
        ).toBeGreaterThanOrEqual(0);
        expect(
          operations.includes(action as FocusTarget),
          `${state.label}: ${visibleName(action)}が製品focus列挙から欠けています`,
        ).toBe(true);
      }
      expect(
        operations.map(visibleName).filter((name) => name === ""),
        `${state.label}で画面上の名前が無い実操作`,
      ).toEqual([]);
      expect(
        operations,
        `${state.label}の実操作数（背景のinert操作や重複操作で水増ししない）`,
      ).toHaveLength(state.expectedOperationCount);

      for (const operation of operations) {
        act(() => operation.focus());
        expect(document.activeElement, `${visibleName(operation)}のfocus`).toBe(
          operation,
        );
        expect(operation.tabIndex).toBeGreaterThanOrEqual(0);
        dispatchKeyboardEvent(operation);
      }
    },
  );

  it("実操作は状態別の実数で固定し、存在しない50件へ水増ししない", () => {
    expect(
      TEN_STATES.map(({ label, expectedOperationCount }) => ({
        label,
        count: expectedOperationCount,
      })),
    ).toEqual([
      { label: "新規作成・既定の正方形", count: 9 },
      { label: "書き出し・PNGと長い成功／失敗文", count: 7 },
      { label: "提案・形を決める（深い形）", count: 68 },
      { label: "提案・4候補", count: 8 },
      { label: "提案・紙上の12個の場所", count: 14 },
      { label: "提案・確認と既存手順消去警告", count: 2 },
      { label: "提案・処理中", count: 18 },
      { label: "復旧・長い実パス", count: 2 },
      { label: "ヘルプ・全13章", count: 16 },
      { label: "展開図・キーボードで始点を決めた途中状態", count: 3 },
    ]);
    expect(
      TEN_STATES.reduce(
        (sum, state) => sum + state.expectedOperationCount,
        0,
      ),
    ).toBe(147);
  });
});

const HORIZONTAL_ONLY_SCROLLER_SELECTORS = [
  ".timeline-controls",
  ".timeline-steps",
  ".help-table-wrap",
] as const;

function decisionButtons(root: HTMLElement): HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>("button")).filter(
    (button) =>
      button.classList.contains("button-primary") ||
      button.classList.contains("button-danger") ||
      /作りはじめる|書き出す|これにする|作り直す|この展開図を使う|復元する|破棄する/u.test(
        visibleName(button),
      ),
  );
}

describe("10状態を500×350 CSS pxへ収める所有CSS契約", () => {
  it("横方向だけの送り領域は所有CSSの3箇所に限定される", () => {
    const selectors = Array.from(
      allOwnedCss.matchAll(
        /([^{}]+)\{[^{}]*overflow-x:\s*(?:auto|scroll)\s*;[^{}]*\}/gu,
      ),
      (match) => match[1].trim(),
    );
    expect(selectors.sort()).toEqual(
      [...HORIZONTAL_ONLY_SCROLLER_SELECTORS].sort(),
    );
  });

  it("Helpの目次は52pxのfocus帯を確保し、狭幅ではsidebarだけが縦送りになる", () => {
    const state = TEN_STATES.find(
      (candidate) => candidate.id === "help-all-thirteen-chapters",
    );
    if (state === undefined) throw new Error("Help状態がありません");
    const root = mountState(state);
    expect(root.querySelectorAll(".help-toc > button")).toHaveLength(13);

    const baseSidebar = declarationBlock(".help-sidebar", dialogsCss);
    const narrowRules = atRuleBlock("@media (max-width: 790px)", responsiveCss);
    const narrowSidebar = optionalLastDeclarationBlock(
      ".help-sidebar",
      narrowRules,
    );
    const narrowToc = optionalLastDeclarationBlock(".help-toc", narrowRules);

    expect({
      baseFocusBand:
        /grid-template-rows:\s*auto\s+auto\s+minmax\(52px,\s*1fr\)\s+auto/u.test(
          baseSidebar,
        ),
      narrowSidebarScroller:
        narrowSidebar !== null &&
        /overflow-x:\s*hidden/u.test(narrowSidebar) &&
        /overflow-y:\s*auto/u.test(narrowSidebar),
      tocIsNotSecondScroller:
        narrowToc !== null && /overflow:\s*visible/u.test(narrowToc),
    }).toEqual({
      baseFocusBand: true,
      narrowSidebarScroller: true,
      tocIsNotSecondScroller: true,
    });
  });

  it.each(TEN_STATES)(
    "$label: 決定操作を横送りだけの領域へ置かず、操作自身を固定配置にしない",
    (state) => {
      const root = mountState(state);
      const decisions = decisionButtons(root);
      for (const button of decisions) {
        expect(
          button.closest(HORIZONTAL_ONLY_SCROLLER_SELECTORS.join(",")),
          `${visibleName(button)}が横送りだけの領域にあります`,
        ).toBeNull();
      }

      const operations = actualOperations(root);
      for (const operation of operations) {
        const inlinePosition =
          operation instanceof HTMLElement || operation instanceof SVGElement
            ? operation.style.position
            : "";
        expect(inlinePosition, `${visibleName(operation)}自身の固定配置`).not.toBe(
          "fixed",
        );
        expect(
          operation.matches(".first-run-guide, .dialog-backdrop"),
          `${visibleName(operation)}自身が所有CSSのfixed selectorに一致します`,
        ).toBe(false);
      }
    },
  );

  it.each(TEN_STATES.filter((state) => state.verticalSelectors.length > 0))(
    "$label: 内容を縮めず製品内で縦送りできる",
    (state) => {
      const root = mountState(state);
      for (const selector of state.verticalSelectors) {
        expect(root.matches(selector) || root.querySelector(selector) !== null).toBe(
          true,
        );
        const block = declarationBlock(selector, dialogsCss);
        expect(block, `${state.label}の${selector}`).toContain("overflow-y: auto");
      }
    },
  );

  it("展開図の現在操作は、要素の全文と200%で見せる文字を一致させる", () => {
    const state = TEN_STATES.find(
      (candidate) => candidate.id === "cp-keyboard-line-start",
    );
    if (state === undefined) throw new Error("展開図の固定状態がありません");
    const root = mountState(state);
    const summary = root.querySelector<HTMLElement>(".operation-summary-line");
    if (summary === null) throw new Error("展開図の現在操作の文字がありません");

    const content = summary.textContent?.trim() ?? "";
    const completeText = summary.dataset.tooltip?.trim() ?? "";
    const baseRule = lastDeclarationBlock(".operation-summary-line", viewerCss);
    const zoomRules = atRuleBlock("@media (max-width: 790px)", responsiveCss);
    const zoomRule = optionalLastDeclarationBlock(
      ".operation-summary-line",
      zoomRules,
    );
    const effective = (property: string) =>
      optionalDeclarationValue(zoomRule, property) ??
      optionalDeclarationValue(baseRule, property);
    const clippingDeclarations = {
      overflow: effective("overflow"),
      textOverflow: effective("text-overflow"),
      whiteSpace: effective("white-space"),
      overflowWrap: effective("overflow-wrap"),
    };

    // jsdomは字形を配置しないため、見えた文字数を捏造しない。実製品DOMの全文と
    // 実効CSSを照合し、省略記号を作る組合せそのものを200%では禁止する。
    expect(content.length).toBeGreaterThan(10);
    expect(
      { content, completeText, clippingDeclarations },
      "200%では要素の全文を折り返して見せ、ellipsisやhiddenで一部を捨てません。",
    ).toEqual({
      content: completeText,
      completeText,
      clippingDeclarations: {
        overflow: "visible",
        textOverflow: "clip",
        whiteSpace: "normal",
        overflowWrap: "anywhere",
      },
    });
  });

  it("紙上12個の場所は固定560pxを500×350へ収める縦横契約を持つ", () => {
    const state = TEN_STATES.find(
      (candidate) => candidate.id === "proposal-paper-twelve-handles",
    );
    if (state === undefined) throw new Error("紙上12個の固定状態がありません");
    const root = mountState(state);
    expect(root.getAttribute("data-proposal-step")).toBe("paper-position");

    const dialogRule = declarationBlock(
      '.dialog-wide[data-proposal-step="paper-position"]',
      dialogsCss,
    );
    const stepRule = lastDeclarationBlock(".paper-position-step", dialogsCss);
    const stageRule = lastDeclarationBlock(".paper-position-stage", dialogsCss);
    expectDeclarations(dialogRule, ["overflow: hidden"]);
    expectDeclarations(stepRule, ["overflow: hidden"]);
    expectDeclarations(stageRule, [
      "width: 560px",
      "height: 560px",
      "min-height: 560px",
    ]);

    const zoomRules = atRuleBlock("@media (max-width: 790px)", responsiveCss);
    const narrowDialogRule = optionalLastDeclarationBlock(
      '.dialog-wide[data-proposal-step="paper-position"]',
      zoomRules,
    );
    const narrowStepRule = optionalLastDeclarationBlock(
      ".paper-position-step",
      zoomRules,
    );
    const narrowStageRule = optionalLastDeclarationBlock(
      ".paper-position-stage",
      zoomRules,
    );
    const flexibleStageWidth =
      narrowStageRule !== null &&
      /width:\s*(?:100%|(?:min|max|clamp|calc)\()/u.test(narrowStageRule) &&
      /max-width:\s*100%/u.test(narrowStageRule) &&
      /min-width:\s*0/u.test(narrowStageRule);
    const narrowDialogScrollsVertically =
      narrowDialogRule !== null &&
      /overflow-x:\s*hidden/u.test(narrowDialogRule) &&
      /overflow-y:\s*auto/u.test(narrowDialogRule);
    const narrowStepScrollsVertically =
      narrowStepRule !== null &&
      /overflow-x:\s*hidden/u.test(narrowStepRule) &&
      /overflow-y:\s*auto/u.test(narrowStepRule);
    const narrowStepReservesFocusBand =
      narrowStepRule !== null &&
      /grid-template-rows:\s*max-content\s+max-content\s+max-content/u.test(
        narrowStepRule,
      ) &&
      /box-sizing:\s*border-box/u.test(narrowStepRule) &&
      /padding:\s*24px\s+10px\s+10px/u.test(narrowStepRule);
    const stage = root.querySelector(".paper-position-stage");
    const actions = Array.from(
      root.querySelectorAll<HTMLButtonElement>(".paper-position-actions > button"),
    );
    const enabledActions = actions.filter((action) => !action.disabled);
    const operations = actualOperations(root);
    const handles = Array.from(
      root.querySelectorAll<FocusTarget>("[data-paper-position-handle]"),
    );
    const lastHandleIndex = Math.max(
      ...handles.map((handle) => operations.indexOf(handle)),
    );
    const firstActionIndex = Math.min(
      ...enabledActions.map((action) =>
        operations.indexOf(action as FocusTarget),
      ),
    );
    expect(
      {
        baseStageSize: 560,
        cssViewportWidth: CSS_VIEWPORT.width,
        flexibleStageWidth,
        narrowDialogScrollsVertically,
        narrowStepScrollsVertically,
        narrowStepReservesFocusBand,
        actionCount: actions.length,
        actionsOutsideStage: actions.every((action) => !stage?.contains(action)),
        actionsFollowHandlesInTabOrder: firstActionIndex > lastHandleIndex,
      },
      "通常表示の560pxは保ち、500×350では幅だけを可変にして縦送りで全操作へ到達させます。",
    ).toEqual({
      baseStageSize: 560,
      cssViewportWidth: 500,
      flexibleStageWidth: true,
      narrowDialogScrollsVertically: true,
      narrowStepScrollsVertically: true,
      narrowStepReservesFocusBand: true,
      actionCount: 3,
      actionsOutsideStage: true,
      actionsFollowHandlesInTabOrder: true,
    });
  });

  it("展開図の操作面は固定pxではなく親区画の幅・高さへ追従する", () => {
    const state = TEN_STATES.find(
      (candidate) => candidate.id === "cp-keyboard-line-start",
    );
    if (state === undefined) throw new Error("展開図の固定状態がありません");
    const root = mountState(state);
    expect(root.querySelector("canvas.cp-canvas")).not.toBeNull();
    expectDeclarations(declarationBlock(".cp-editor", viewerCss), [
      "position: relative",
      "width: 100%",
      "height: 100%",
    ]);
    expectDeclarations(declarationBlock(".cp-canvas", viewerCss), [
      "display: block",
      "width: 100%",
      "height: 100%",
    ]);
  });
});
