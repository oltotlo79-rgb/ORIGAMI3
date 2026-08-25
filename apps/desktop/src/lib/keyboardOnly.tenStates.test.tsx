// @vitest-environment jsdom
// 5-Aで固定した10状態を、実製品の画面要素に対するキー操作で点検する。
// jsdomはTab移動、button/checkbox/rangeのキー既定動作、描画結果を実装しない。
// その部分だけをブラウザーと同じ順番で補い、製品のDOM handlerとストア更新を通す。

import { readFileSync } from "node:fs";
import { createRef, type ReactElement } from "react";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { save } from "@tauri-apps/plugin-dialog";

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));

vi.mock("../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  documentExport: vi.fn(),
  editApply: vi.fn(),
  editApplyBatch: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  foldAllPreview: vi.fn(),
  poseSolve: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  recoveryCheck: vi.fn(),
  recoveryRestore: vi.fn(),
  proposalGenerate: vi.fn(),
  proposalProgress: vi.fn(),
  proposalControl: vi.fn(),
  proposalApply: vi.fn(),
}));

// canvasの描画器だけはjsdomに無いため無害化する。CpEditor本体、canvas、
// focus/keydown handler、ストアactionは実製品のものを使う。
vi.mock("../components/CpEditor/renderer", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../components/CpEditor/renderer")>();
  return { ...actual, render: vi.fn() };
});

import * as ipc from "../ipc/client";
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
  addLimb,
  defaultSkeleton,
  leafNodes,
  LENGTH_RANGE,
  skeletonRows,
  WIDTH_RANGE,
} from "./skeleton";
import type {
  CreasePattern,
  Document,
  DocumentView,
  FoldStep,
  ProposalCandidate,
  Skeleton,
} from "./types";
import {
  DEFAULT_NEW_PAPER,
  useAppStore,
} from "../store/appStore";

const initialStoreState = useAppStore.getState();
const baseLayoutCss = readFileSync("src/styles/base-layout.css", "utf8");
const viewerCss = readFileSync("src/styles/viewer.css", "utf8");
const dialogsCss = readFileSync("src/styles/dialogs.css", "utf8");

const BASE_DOCUMENT: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: {
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
    next_edge_id: 4,
  },
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

function viewOf(doc: Document = BASE_DOCUMENT): DocumentView {
  return {
    doc: structuredClone(doc),
    faces: [],
    warnings: [],
    violations: [],
    frame: null,
    skipped: [],
    contact_detected: false,
  };
}

function cp(mark: number): CreasePattern {
  return {
    ...structuredClone(BASE_DOCUMENT.cp),
    next_edge_id: mark,
  };
}

function candidate(mark: number): ProposalCandidate {
  return {
    cp: cp(mark),
    scale: 0.4,
    violations: 0,
    warnings: [],
    fold_plan: null,
  };
}

function radialSkeleton(count: number): Skeleton {
  return {
    nodes: [
      { id: 0, parent: null, length: 0, width_factor: 1 },
      ...Array.from({ length: count }, (_, index) => ({
        id: index + 1,
        parent: 0,
        length: 0.7 + (index % 3) * 0.1,
        width_factor: 1,
      })),
    ],
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

/** 5-Aの代表標本: 16節点、先端12本、最大深さ4、操作行15。 */
function deepTwelveTipBranch(): Skeleton {
  let skeleton = defaultSkeleton();
  skeleton = addLimb(skeleton, 1);
  const secondLevel = skeleton.nodes.find((node) => node.parent === 1);
  if (secondLevel === undefined) throw new Error("第2階層を作れませんでした");
  skeleton = addLimb(skeleton, secondLevel.id);
  const thirdLevel = skeleton.nodes.find(
    (node) => node.parent === secondLevel.id,
  );
  if (thirdLevel === undefined) throw new Error("第3階層を作れませんでした");
  for (let index = 0; index < 9; index += 1) {
    skeleton = addLimb(skeleton, thirdLevel.id);
  }
  return withLargestParts(skeleton);
}

function candidateWithSites(mark: number, count: number): ProposalCandidate {
  return {
    ...candidate(mark),
    sites: Array.from({ length: count }, (_, index) => ({
      circle: {
        leaf_id: index + 1,
        circle_index: index,
        center: [
          ((index % 4) + 0.5) / 4,
          (Math.floor(index / 4) + 0.5) / 3,
        ] as [number, number],
        radius: 0.04,
      },
      vertex: null,
      molecules: [],
    })),
  };
}

function steps(count: number): FoldStep[] {
  return Array.from({ length: count }, (_, index) => ({
    id: index + 1,
    kind: "Simple",
    drivers: [],
    layer_order: null,
    note: "",
  }));
}

interface InputAudit {
  lowLevelEvents: string[];
  clicks: Array<{ detail: number; pointerType: string }>;
  stop: () => void;
}

/**
 * Enter/Spaceの標準動作もclickを1回作る。そのためclick総数を0とはせず、
 * detail>0又はpointerType付きの「pointer由来click」が0であることを分けて測る。
 */
function auditInput(): InputAudit {
  const lowLevelEvents: string[] = [];
  const clicks: Array<{ detail: number; pointerType: string }> = [];
  const lowLevelTypes = [
    "pointerdown",
    "pointerup",
    "pointermove",
    "pointercancel",
    "mousedown",
    "mouseup",
    "mousemove",
    "wheel",
    "contextmenu",
  ] as const;
  const recordLowLevel = (event: Event) => lowLevelEvents.push(event.type);
  const recordClick = (event: Event) => {
    const mouse = event as MouseEvent & { pointerType?: string };
    clicks.push({
      detail: mouse.detail,
      pointerType: mouse.pointerType ?? "",
    });
  };
  for (const type of lowLevelTypes) {
    document.addEventListener(type, recordLowLevel, true);
  }
  document.addEventListener("click", recordClick, true);
  return {
    lowLevelEvents,
    clicks,
    stop: () => {
      for (const type of lowLevelTypes) {
        document.removeEventListener(type, recordLowLevel, true);
      }
      document.removeEventListener("click", recordClick, true);
    },
  };
}

function visibleName(element: Element): string {
  const labelledBy = element.getAttribute("aria-labelledby");
  const labelledText = labelledBy
    ?.split(/\s+/u)
    .map((id) => document.getElementById(id)?.textContent ?? "")
    .join(" ");
  const ownerLabel = element.closest("label")?.textContent;
  const id = element.getAttribute("id");
  const explicitLabel = id
    ? [...document.querySelectorAll<HTMLLabelElement>("label")].find(
        (label) => label.htmlFor === id,
      )?.textContent
    : null;
  return (
    [
      element.getAttribute("aria-label"),
      labelledText,
      explicitLabel,
      ownerLabel,
      element.getAttribute("data-tooltip"),
      element.getAttribute("title"),
      element.textContent,
    ].find(
      (value): value is string =>
        typeof value === "string" && value.trim() !== "",
    )?.trim() ?? ""
  );
}

function isDisplayedFocusTarget(element: Element): boolean {
  const hiddenOwner = element.closest(
    '[hidden], [aria-hidden="true"], [inert]',
  );
  const style = getComputedStyle(element);
  return (
    hiddenOwner === null &&
    !element.hasAttribute("hidden") &&
    style.display !== "none" &&
    style.visibility !== "hidden"
  );
}

function focusTarget(target: FocusTarget): void {
  if (document.activeElement !== target) {
    act(() => target.focus({ preventScroll: true }));
  }
  expect(document.activeElement, visibleName(target)).toBe(target);
  expect(target.matches(":focus"), visibleName(target)).toBe(true);
  expect(isDisplayedFocusTarget(target), visibleName(target)).toBe(true);
}

/** jsdomに無い通常のTab移動だけを補い、keydown/keyupは実DOMへ送る。 */
function pressTab(
  root: HTMLElement,
  shiftKey = false,
  ordered = focusableElements(root),
): FocusTarget {
  if (ordered.length === 0) throw new Error("Tabで辿る対象がありません");
  const active = document.activeElement as FocusTarget | null;
  const down = new KeyboardEvent("keydown", {
    key: "Tab",
    shiftKey,
    bubbles: true,
    cancelable: true,
  });
  fireEvent(active ?? root, down);
  if (!down.defaultPrevented) {
    const current = active === null ? -1 : ordered.indexOf(active);
    const nextIndex = shiftKey
      ? current <= 0
        ? ordered.length - 1
        : current - 1
      : current < 0 || current === ordered.length - 1
        ? 0
        : current + 1;
    act(() => ordered[nextIndex].focus({ preventScroll: true }));
  }
  const focused = document.activeElement as FocusTarget;
  fireEvent.keyUp(focused, { key: "Tab", shiftKey });
  return focused;
}

function tabTo(root: HTMLElement, target: FocusTarget): void {
  const ordered = focusableElements(root);
  if (ordered.length === 0) throw new Error("Tabで辿る対象がありません");
  focusTarget(ordered[0]);
  for (let index = 0; index <= ordered.length; index += 1) {
    if (document.activeElement === target) return;
    pressTab(root, false, ordered);
  }
  throw new Error(`Tabで対象へ到達できません: ${visibleName(target)}`);
}

function expectFocusPresentation(root: HTMLElement, target: FocusTarget): void {
  focusTarget(target);
  if (target.matches(".skeleton-preview .tip-handle")) {
    const id = target.getAttribute("data-tip-handle");
    expect(id).not.toBeNull();
    expect(
      root.querySelectorAll(`[data-tip-focus-ring="${id}"]`),
      `${visibleName(target)}の見える選択輪`,
    ).toHaveLength(1);
    // SVGの座標系では共通outlineが巨大化するため、製品はそれを明示的に消し、
    // 上で確認した同じSVG内の輪を代わりに描く。この対を一緒に契約化する。
    const tipFocusRule =
      /\.skeleton-preview\s+\.tip-handle:focus,\s*\.skeleton-preview\s+\.tip-handle:focus-visible\s*\{([\s\S]*?)\}/u.exec(
        dialogsCss,
      )?.[1];
    expect(tipFocusRule).toContain("outline: none");
    expect(tipFocusRule).toContain("box-shadow: none");
    return;
  }
  if (target.matches(".paper-position-handle")) {
    const id = target.getAttribute("data-paper-position-handle");
    expect(id).not.toBeNull();
    expect(root.querySelector(`[data-paper-focus-ring="${id}"]`)).not.toBeNull();
    return;
  }

  const globalRule = /:focus-visible\s*\{([\s\S]*?)\}/u.exec(
    baseLayoutCss,
  )?.[1];
  expect(globalRule).toContain("outline: 2px solid var(--color-accent)");
  expect(globalRule).toContain("box-shadow: var(--focus-ring)");
  if (target.matches(".cp-canvas")) {
    const canvasRule = /\.cp-canvas:focus-visible\s*\{([\s\S]*?)\}/u.exec(
      viewerCss,
    )?.[1];
    expect(canvasRule).toContain("outline-offset: -4px");
  }
}

/**
 * 期待側もfocusableElementsだけから作ると、製品側の列挙漏れを見逃す。
 * 別のsemantic selectorで有効な操作を数え、radio group以外は全てTab順にあることを照合する。
 */
function expectEnabledActionsInTabOrder(root: HTMLElement): void {
  const ordered = focusableElements(root);
  const actions = [
    ...root.querySelectorAll<FocusTarget>(
      [
        "a[href]",
        "button",
        "canvas",
        "input:not([type='hidden'])",
        "select",
        "summary",
        "textarea",
        "[role='button']",
        "[role='slider']",
        "[data-paper-position-handle]",
      ].join(","),
    ),
  ].filter((element) => {
    const disabled =
      element.getAttribute("aria-disabled") === "true" ||
      ("disabled" in element &&
        Boolean((element as HTMLButtonElement | HTMLInputElement).disabled));
    return !disabled && isDisplayedFocusTarget(element);
  });

  for (const action of actions) {
    expect(visibleName(action), `${action.tagName}の操作名`).not.toBe("");
    if (action instanceof HTMLInputElement && action.type === "radio") {
      const group = actions.filter(
        (candidate): candidate is HTMLInputElement =>
          candidate instanceof HTMLInputElement &&
          candidate.type === "radio" &&
          candidate.name === action.name,
      );
      const representative = group.find((radio) => radio.checked) ?? group[0];
      expect(ordered, `${action.name} radio groupのTab入口`).toContain(
        representative,
      );
      continue;
    }
    expect(action.tabIndex, `${visibleName(action)}のtabIndex`).toBeGreaterThanOrEqual(0);
    expect(ordered, `${visibleName(action)}のTab順`).toContain(action);
  }
}

/** 全Tab stopを正順・逆順で1周し、名前と現在位置の表示契約を各要素で確認する。 */
function expectTabTraversal(root: HTMLElement): void {
  const ordered = focusableElements(root);
  expect(ordered.length).toBeGreaterThan(0);
  expectEnabledActionsInTabOrder(root);
  for (const target of ordered) {
    expect(visibleName(target), `${target.tagName}の画面上の名前`).not.toBe("");
  }

  focusTarget(ordered[0]);
  for (const expected of [...ordered.slice(1), ordered[0]]) {
    const focused = pressTab(root, false, ordered);
    expect(focused, `Tab → ${visibleName(expected)}`).toBe(expected);
    expectFocusPresentation(root, expected);
  }

  focusTarget(ordered[0]);
  for (const expected of [...ordered].reverse()) {
    const focused = pressTab(root, true, ordered);
    expect(focused, `Shift+Tab → ${visibleName(expected)}`).toBe(expected);
    expect(focused.matches(":focus"), visibleName(expected)).toBe(true);
  }
}

/**
 * jsdomはEnterからbuttonのclickを作らない。keydownが取り消されなかった場合だけ、
 * ブラウザーが作るdetail=0のclickを補う。pointer/mouseイベントは作らない。
 */
function pressEnter(button: HTMLButtonElement): void {
  const down = new KeyboardEvent("keydown", {
    key: "Enter",
    bubbles: true,
    cancelable: true,
  });
  fireEvent(button, down);
  expect(down.defaultPrevented).toBe(false);
  if (!down.defaultPrevented) act(() => button.click());
  fireEvent.keyUp(button, { key: "Enter" });
}

/** checkboxのSpace既定動作だけを補い、製品のonChangeを実際に通す。 */
function pressSpace(checkbox: HTMLInputElement): void {
  const down = new KeyboardEvent("keydown", {
    key: " ",
    bubbles: true,
    cancelable: true,
  });
  fireEvent(checkbox, down);
  expect(down.defaultPrevented).toBe(false);
  if (!down.defaultPrevented) act(() => checkbox.click());
  fireEvent.keyUp(checkbox, { key: " " });
}

/** rangeの矢印キー既定値変更だけを補い、製品のonChangeを実際に通す。 */
function pressRangeArrow(
  range: HTMLInputElement,
  key: "ArrowLeft" | "ArrowRight",
): number {
  const before = Number(range.value);
  const step = Number(range.step || "1");
  const after =
    key === "ArrowRight"
      ? Math.min(Number(range.max), before + step)
      : Math.max(Number(range.min), before - step);
  const down = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
  });
  fireEvent(range, down);
  if (!down.defaultPrevented) {
    fireEvent.change(range, { target: { value: String(after) } });
  }
  fireEvent.keyUp(range, { key });
  return down.defaultPrevented ? before : after;
}

/** NumberStepperが実装する矢印キー経路を、そのまま製品inputへ送る。 */
function pressNumberArrow(
  input: HTMLInputElement,
  key: "ArrowDown" | "ArrowUp",
): number {
  fireEvent.keyDown(input, { key });
  fireEvent.keyUp(input, { key });
  return Number(input.value);
}

/**
 * jsdomに無いradio groupの矢印キー既定動作だけを補う。
 * disabledは飛ばし、focus・checked・input/changeの順を製品DOMへ通す。
 */
function pressRadioArrow(
  radio: HTMLInputElement,
  key: "ArrowLeft" | "ArrowRight",
): HTMLInputElement {
  const group = Array.from(
    document.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
  ).filter(
    (candidate) => candidate.name === radio.name && !candidate.disabled,
  );
  const currentIndex = group.indexOf(radio);
  if (currentIndex < 0 || group.length === 0) {
    throw new Error(`${visibleName(radio)}の選択肢群がありません`);
  }
  const down = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
  });
  fireEvent(radio, down);
  const offset = key === "ArrowRight" ? 1 : -1;
  const next = group[(currentIndex + offset + group.length) % group.length];
  if (!down.defaultPrevented) {
    act(() => next.focus({ preventScroll: true }));
    act(() => next.click());
  }
  fireEvent.keyUp(next, { key });
  return down.defaultPrevented ? radio : next;
}

function operationKind(target: FocusTarget): string {
  if (target instanceof HTMLInputElement) return `input:${target.type}`;
  return target.getAttribute("role") ?? target.tagName.toLocaleLowerCase("en-US");
}

function operationLedgerId(target: FocusTarget): string {
  return `${operationKind(target)}:${visibleName(target).replace(/\s+/gu, " ").trim()}`;
}

function dialog(): HTMLElement {
  return screen.getByRole("dialog");
}

interface TenStateCase {
  id: number;
  label: string;
  arrange: () => void;
  node: () => ReactElement;
  root: () => HTMLElement;
  assertState: (root: HTMLElement) => void;
  target: (root: HTMLElement) => FocusTarget;
  activateAndVerify: (target: FocusTarget, root: HTMLElement) => Promise<void>;
}

const LONG_NAME = `${"長い保存先".repeat(24)}.png`;
const LONG_ERROR = `保存できませんでした。${"別の場所を選んでください。".repeat(20)}`;

const TEN_STATES: readonly TenStateCase[] = [
  {
    id: 1,
    label: "New・既定の正方形",
    arrange: () => {
      useAppStore.setState({
        newDialogOpen: true,
        newPaperDraft: { ...DEFAULT_NEW_PAPER },
      });
    },
    node: () => <NewDocumentDialog />,
    root: dialog,
    assertState: () => {
      expect(screen.getByRole("heading", { name: "新しい紙を用意する" })).toBeTruthy();
      expect(
        (screen.getByLabelText("正方形(たて・よこが同じ)") as HTMLInputElement)
          .checked,
      ).toBe(true);
    },
    target: () => screen.getByRole("button", { name: "折り紙 24cm角" }) as FocusTarget,
    activateAndVerify: async (target) => {
      pressEnter(target as HTMLButtonElement);
      expect(useAppStore.getState().newPaperDraft).toEqual({
        widthMm: 240,
        heightMm: 240,
        square: true,
      });
    },
  },
  {
    id: 2,
    label: "Export・PNGと長い成功/失敗文",
    arrange: () => {
      useAppStore.setState({
        doc: structuredClone(BASE_DOCUMENT),
        exportOpen: true,
        exportKind: "CpPng",
        exportIncludeAux: false,
        exportLongSide: 16384,
        exportBusy: false,
        exportSavedPath: `C:\\非常に長い保存先\\${LONG_NAME}`,
        exportError: LONG_ERROR,
      });
    },
    node: () => <ExportDialog />,
    root: dialog,
    assertState: () => {
      expect(screen.getByRole("heading", { name: "展開図・折り図を書き出す" })).toBeTruthy();
      expect(screen.getByText(`保存しました:${LONG_NAME}`)).toBeTruthy();
      expect(screen.getByText(`保存できませんでした:${LONG_ERROR}`)).toBeTruthy();
      expect((screen.getByLabelText("展開図(PNG)") as HTMLInputElement).checked).toBe(true);
    },
    target: () => screen.getByLabelText("補助線(下書きの線)も含める") as FocusTarget,
    activateAndVerify: async (target) => {
      pressSpace(target as HTMLInputElement);
      expect(useAppStore.getState().exportIncludeAux).toBe(true);
      expect((target as HTMLInputElement).checked).toBe(true);
    },
  },
  {
    id: 3,
    label: "Proposal・skeleton（深い形）",
    arrange: () => {
      useAppStore.setState({
        proposalStep: "skeleton",
        proposalSkeleton: deepTwelveTipBranch(),
        proposalCandidates: [],
        proposalSelected: null,
        proposalBusy: false,
        proposalError: null,
      });
    },
    node: () => <ProposalWizard />,
    root: dialog,
    assertState: (root) => {
      expect(screen.getByRole("heading", { name: "形を決めて展開図を作ってもらう" })).toBeTruthy();
      const skeleton = useAppStore.getState().proposalSkeleton;
      expect(skeleton.nodes).toHaveLength(16);
      expect(leafNodes(skeleton)).toHaveLength(12);
      expect(Math.max(...skeletonRows(skeleton).map((row) => row.depth))).toBe(4);
      expect(root.querySelectorAll("[data-shape-row]")).toHaveLength(15);
    },
    target: () => screen.getAllByRole("slider", { name: /の長さ$/u })[0] as FocusTarget,
    activateAndVerify: async (target) => {
      const range = target as HTMLInputElement;
      const before = Number(range.value);
      const expected = pressRangeArrow(range, "ArrowLeft");
      expect(expected).not.toBe(before);
      expect(useAppStore.getState().proposalSkeleton.nodes[1].length).toBe(expected);
      expect(Number(range.value)).toBe(expected);
    },
  },
  {
    id: 4,
    label: "Proposal・candidates（4候補）",
    arrange: () => {
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(4),
        proposalCandidates: Array.from({ length: 4 }, (_, index) =>
          candidateWithSites(20 + index, 4),
        ),
        proposalSelected: 0,
        proposalBusy: false,
        proposalError: null,
      });
    },
    node: () => <ProposalWizard />,
    root: dialog,
    assertState: () => {
      expect(screen.getAllByRole("button", { name: /候補[1-4]$/u })).toHaveLength(4);
    },
    target: () => screen.getByRole("button", { name: "候補4" }) as FocusTarget,
    activateAndVerify: async (target) => {
      pressEnter(target as HTMLButtonElement);
      expect(useAppStore.getState().proposalSelected).toBe(3);
      expect(target.getAttribute("aria-pressed")).toBe("true");
    },
  },
  {
    id: 5,
    label: "Proposal・paper-position（12個の丸印）",
    arrange: () => {
      useAppStore.setState({
        proposalStep: "candidates",
        proposalSkeleton: radialSkeleton(12),
        proposalCandidates: [candidateWithSites(30, 12)],
        proposalSelected: 0,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalPaperSpecified: [],
        proposalPositionLastMoved: [],
        proposalPositionUndoStack: [],
        proposalPositionRedoStack: [],
        proposalBusy: false,
        proposalError: null,
      });
      useAppStore.getState().openProposalPaperPositionEditor();
    },
    node: () => <ProposalWizard />,
    root: dialog,
    assertState: (root) => {
      expect(screen.getByRole("heading", { name: "紙の上の場所を調整" })).toBeTruthy();
      expect(root.querySelectorAll("[data-paper-position-handle]")).toHaveLength(12);
    },
    target: (root) => {
      const handle = root.querySelector<FocusTarget>(
        '[data-paper-position-handle="1"]',
      );
      if (handle === null) throw new Error("1番目の丸い印がありません");
      return handle;
    },
    activateAndVerify: async (target) => {
      const before = useAppStore.getState().proposalPaperPositions[0].position.x;
      fireEvent.keyDown(target, { key: "ArrowRight" });
      fireEvent.keyUp(target, { key: "ArrowRight" });
      const after = useAppStore.getState().proposalPaperPositions[0].position.x;
      expect(after).toBeGreaterThan(before);
      expect(target.getAttribute("data-paper-position-changed")).toBe("true");
    },
  },
  {
    id: 6,
    label: "Proposal・confirm（既存手順を消す警告あり）",
    arrange: () => {
      useAppStore.setState({
        doc: { ...structuredClone(BASE_DOCUMENT), sequence: steps(3) },
        proposalStep: "confirm",
        proposalSkeleton: radialSkeleton(4),
        proposalCandidates: [candidate(40)],
        proposalSelected: 0,
        proposalBusy: false,
        proposalError: null,
      });
    },
    node: () => <ProposalWizard />,
    root: dialog,
    assertState: () => {
      expect(
        screen.getByText("この展開図を使うと、今ある折り手順3件はすべて消えます。"),
      ).toBeTruthy();
    },
    target: () => screen.getByRole("button", { name: "選び直す" }) as FocusTarget,
    activateAndVerify: async (target) => {
      pressEnter(target as HTMLButtonElement);
      expect(useAppStore.getState().proposalStep).toBe("candidates");
      expect(screen.getByRole("button", { name: "候補1" })).toBeTruthy();
    },
  },
  {
    id: 7,
    label: "Proposal・busy（進行表示あり）",
    arrange: () => {
      useAppStore.setState({
        proposalStep: "skeleton",
        proposalSkeleton: radialSkeleton(4),
        proposalCandidates: [],
        proposalSelected: null,
        proposalBusy: true,
        proposalProgress: {
          job_id: "keyboard-ten-states",
          done: 2,
          total: 4,
          phase: "Generating",
        },
        proposalProgressWarning: null,
        proposalError: null,
      });
    },
    node: () => <ProposalWizard />,
    root: dialog,
    assertState: () => {
      expect(screen.getByRole("status").textContent).toContain("4件中2件め");
      expect(screen.getByText("計算中…")).toBeTruthy();
    },
    target: () => screen.getByRole("button", { name: "やめる" }) as FocusTarget,
    activateAndVerify: async (target) => {
      pressEnter(target as HTMLButtonElement);
      expect(useAppStore.getState().proposalStep).toBeNull();
      expect(screen.queryByRole("dialog")).toBeNull();
    },
  },
  {
    id: 8,
    label: "Recovery・長い実パス",
    arrange: () => {
      useAppStore.setState({
        recovery: {
          autosave_path: `C:\\自動保存\\${"深い場所\\".repeat(18)}控え.ori3`,
          document_path: `C:\\作品\\${"深い場所\\".repeat(18)}折り鶴.ori3`,
          saved_at_ms: Date.UTC(2026, 7, 26, 0, 0, 0),
        },
      });
    },
    node: () => <RecoveryDialog />,
    root: dialog,
    assertState: () => {
      expect(
        screen.getByRole("heading", {
          name: "前回の終了が正常に行われませんでした",
        }),
      ).toBeTruthy();
      expect(screen.getByText(/元の作品:折り鶴\.ori3/u)).toBeTruthy();
    },
    target: () => screen.getByRole("button", { name: "復元する" }) as FocusTarget,
    activateAndVerify: async (target) => {
      pressEnter(target as HTMLButtonElement);
      await waitFor(() => expect(useAppStore.getState().recovery).toBeNull());
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(ipc.recoveryRestore).toHaveBeenCalledWith(true);
    },
  },
  {
    id: 9,
    label: "Help・全13章",
    arrange: () => {
      useAppStore.setState({
        helpOpen: true,
        helpChapterId: "overview",
        helpQuery: "",
      });
    },
    node: () => <HelpCenter />,
    root: dialog,
    assertState: () => {
      expect(screen.getByText("全13章")).toBeTruthy();
      expect(screen.getByRole("navigation", { name: "ヘルプの目次" }).querySelectorAll("button")).toHaveLength(13);
    },
    target: () => screen.getByRole("button", { name: /ショートカット一覧/u }) as FocusTarget,
    activateAndVerify: async (target) => {
      pressEnter(target as HTMLButtonElement);
      expect(useAppStore.getState().helpChapterId).toBe("shortcuts");
      expect(target.getAttribute("aria-current")).toBe("page");
      expect(screen.getByRole("heading", { name: "ショートカット一覧" })).toBeTruthy();
    },
  },
  {
    id: 10,
    label: "CpEditor・キーボードで始点を決めた途中状態",
    arrange: () => {
      useAppStore.setState({
        doc: structuredClone(BASE_DOCUMENT),
        docEpoch: 1,
        faces: [],
        hinges: new Set(),
        selection: { edgeIds: [], vertexIds: [] },
        activeTool: "mountain",
        operationStage: 0,
        lineInputStart: null,
        currentStep: null,
      });
    },
    node: () => <CpEditor fitRef={createRef<(() => void) | null>()} />,
    root: () => {
      const root = document.querySelector<HTMLElement>(".cp-editor");
      if (root === null) throw new Error("展開図の区画がありません");
      // 初回mount時に古い描画途中を消す製品effectがあるため、到達不能な値を
      // 直接注入しない。実canvasを選び、Enterで始点を決めて固定状態10を作る。
      const canvas = screen.getByTestId("cp-canvas") as FocusTarget;
      focusTarget(canvas);
      fireEvent.keyDown(canvas, { key: "Enter" });
      fireEvent.keyUp(canvas, { key: "Enter" });
      return root;
    },
    assertState: () => {
      const canvas = screen.getByTestId("cp-canvas");
      expect(canvas.getAttribute("aria-label")).toContain("Enterを2回");
      expect(useAppStore.getState().lineInputStart).toEqual([0.5, 0.5]);
      expect(useAppStore.getState().operationStage).toBe(1);
    },
    target: () => screen.getByTestId("cp-canvas") as FocusTarget,
    activateAndVerify: async (target) => {
      fireEvent.keyDown(target, { key: "Escape" });
      fireEvent.keyUp(target, { key: "Escape" });
      expect(useAppStore.getState().lineInputStart).toBeNull();
      expect(useAppStore.getState().operationStage).toBe(0);
    },
  },
] as const;

const buttonId = (name: string) => `button:${name}`;
const rangeId = (name: string) => `input:range:${name}`;

const DEEP_LEAF_NAMES = [
  ...Array.from(
    { length: 9 },
    (_, index) => `頭のその先1のその先1のその先${index + 1}`,
  ),
  "尾",
  "右前足",
  "左前足",
] as const;
const DEEP_BRANCH_NAMES = [
  "頭",
  "頭のその先1",
  "頭のその先1のその先1",
] as const;
const RADIAL_FOUR_NAMES = ["頭", "尾", "右前足", "左前足"] as const;
const PAPER_TWELVE_NAMES = [
  "頭",
  "尾",
  "右前足",
  "左前足",
  "右後足",
  "左後足",
  "右の羽",
  "左の羽",
  "出っぱり9",
  "出っぱり10",
  "出っぱり11",
  "出っぱり12",
] as const;
const HELP_CHAPTER_LEDGER = [
  ["1ORIGAMI3でできること", "overview"],
  ["2画面の見かた", "workspace"],
  ["3新しい紙を用意する", "new-paper"],
  ["4展開図に線を引く", "crease-pattern"],
  ["5折る", "fold"],
  ["6角度を変える", "angles"],
  ["7立体にする", "three-dimensional"],
  ["8技法を使う", "techniques"],
  ["9手順の記録と再生", "timeline"],
  ["10形から展開図を提案", "proposal"],
  ["11保存と書き出し", "save-export"],
  ["12困ったときは", "troubleshooting"],
  ["13ショートカット一覧", "shortcuts"],
] as const;

/**
 * 147は上限ではなく、この固定標本で2026-08-26に実測した全Tab停止位置の台帳。
 * 画面へ操作を足した／替えた場合は、同数の入替でもこの名前付き台帳が赤くなり、
 * 新しい操作結果を検査してから台帳を更新する。存在しない操作は足さない。
 */
const EXPECTED_OPERATION_LEDGER: Readonly<Record<number, readonly string[]>> = {
  1: [
    "input:radio:正方形(たて・よこが同じ)",
    "input:number:紙の横の長さ（mm）",
    buttonId("紙の横の長さ（mm）を増やす"),
    buttonId("紙の横の長さ（mm）を減らす"),
    buttonId("折り紙 15cm角の大きさを使います"),
    buttonId("折り紙 24cm角の大きさを使います"),
    buttonId("A4の紙の大きさを使います"),
    buttonId("入力した大きさで新しい作品を始めます"),
    buttonId("新規作成をやめます"),
  ],
  2: [
    "input:radio:展開図(PNG)",
    "input:checkbox:補助線(下書きの線)も含める",
    "input:number:画像の大きさ（長辺の点数）",
    buttonId("画像の大きさ（長辺の点数）を増やす"),
    buttonId("画像の大きさ（長辺の点数）を減らす"),
    buttonId("保存先を選んで書き出す"),
    buttonId("閉じる"),
  ],
  3: [
    ...DEEP_LEAF_NAMES.map((name) =>
      buttonId(`${name}を出したい場所（自動）`),
    ),
    ...DEEP_BRANCH_NAMES.flatMap((name) => [
      rangeId(`${name}の長さ`),
      buttonId(`${name}とその先を消す`),
    ]),
    ...DEEP_LEAF_NAMES.flatMap((name) => [
      rangeId(`${name}の長さ`),
      rangeId(`${name}の太さ`),
      buttonId(`${name}のこの先に足す`),
      buttonId(`${name}とその先を消す`),
    ]),
    buttonId("展開図を作ってもらう"),
    buttonId("やめる"),
  ],
  4: [
    ...Array.from({ length: 4 }, (_, index) => buttonId(`候補${index + 1}`)),
    buttonId("形を直す"),
    buttonId("別の置き方も見る"),
    buttonId("紙の上の場所も調整"),
    buttonId("これにする"),
  ],
  5: [
    ...PAPER_TWELVE_NAMES.map((name) =>
      buttonId(`${name}の紙の上の場所（この候補のまま）`),
    ),
    buttonId("候補へ戻る"),
    buttonId("この場所で作り直す"),
  ],
  6: [buttonId("選び直す"), buttonId("この展開図を使う")],
  7: [
    ...RADIAL_FOUR_NAMES.flatMap((name) => [
      rangeId(`${name}の長さ`),
      rangeId(`${name}の太さ`),
      buttonId(`${name}のこの先に足す`),
      buttonId(`${name}とその先を消す`),
    ]),
    buttonId("出っぱりを増やす"),
    buttonId("やめる"),
  ],
  8: [buttonId("復元する"), buttonId("破棄する")],
  9: [
    buttonId("ヘルプセンターを閉じる"),
    "input:search:章題・本文を検索",
    ...HELP_CHAPTER_LEDGER.map(([name]) => buttonId(name)),
    buttonId("基本操作ガイドをもう一度"),
  ],
  10: [
    buttonId("展開図の詳しい操作方法 ▼"),
    "canvas:展開図。矢印キーで位置を動かし、Enterを2回押すと線を引けます。Escapeでやめます",
    "div:展開図に表示している手順",
  ],
};

interface OperationResultCase {
  state: TenStateCase;
  ledgerId: string;
}

const OPERATION_RESULT_CASES: readonly OperationResultCase[] = TEN_STATES.flatMap(
  (state) =>
    (EXPECTED_OPERATION_LEDGER[state.id] ?? []).map((ledgerId) => ({
      state,
      ledgerId,
    })),
);

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState(initialStoreState, true);
  vi.mocked(ipc.recoveryRestore).mockResolvedValue(viewOf());
  vi.mocked(ipc.proposalProgress).mockResolvedValue(null);
  Object.defineProperty(HTMLCanvasElement.prototype, "clientWidth", {
    configurable: true,
    value: 400,
  });
  Object.defineProperty(HTMLCanvasElement.prototype, "clientHeight", {
    configurable: true,
    value: 400,
  });
  HTMLCanvasElement.prototype.getContext = vi.fn(() => ({})) as never;
  HTMLCanvasElement.prototype.getBoundingClientRect = vi.fn(
    () =>
      ({
        x: 0,
        y: 0,
        width: 400,
        height: 400,
        top: 0,
        right: 400,
        bottom: 400,
        left: 0,
        toJSON: () => ({}),
      }) as DOMRect,
  );
  Element.prototype.setPointerCapture = vi.fn();
  Element.prototype.releasePointerCapture = vi.fn();
  Element.prototype.hasPointerCapture = vi.fn(() => false);
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("5-E: 固定10状態のキーボード検査", () => {
  it("DIAGNOSTIC: 10状態の操作台帳を採取する", () => {
    const ledger: Record<string, string[]> = {};
    for (const state of TEN_STATES) {
      cleanup();
      useAppStore.setState(initialStoreState, true);
      state.arrange();
      render(state.node());
      const root = state.root();
      ledger[String(state.id)] = focusableElements(root).map(operationLedgerId);
    }
    console.log(`TEN_STATE_LEDGER=${JSON.stringify(ledger, null, 2)}`);
  });

  it("5-Aで決めた10状態を増減せず、同じ順序と名前で固定する", () => {
    expect(TEN_STATES.map(({ id, label }) => ({ id, label }))).toEqual([
      { id: 1, label: "New・既定の正方形" },
      { id: 2, label: "Export・PNGと長い成功/失敗文" },
      { id: 3, label: "Proposal・skeleton（深い形）" },
      { id: 4, label: "Proposal・candidates（4候補）" },
      { id: 5, label: "Proposal・paper-position（12個の丸印）" },
      { id: 6, label: "Proposal・confirm（既存手順を消す警告あり）" },
      { id: 7, label: "Proposal・busy（進行表示あり）" },
      { id: 8, label: "Recovery・長い実パス" },
      { id: 9, label: "Help・全13章" },
      { id: 10, label: "CpEditor・キーボードで始点を決めた途中状態" },
    ]);

    const canceledRange = document.createElement("input");
    canceledRange.type = "range";
    canceledRange.min = "0";
    canceledRange.max = "10";
    canceledRange.step = "1";
    canceledRange.value = "5";
    const input = vi.fn();
    const change = vi.fn();
    canceledRange.addEventListener("keydown", (event) => {
      if (event.key === "ArrowRight") event.preventDefault();
    });
    canceledRange.addEventListener("input", input);
    canceledRange.addEventListener("change", change);

    expect(pressRangeArrow(canceledRange, "ArrowRight")).toBe(5);
    expect(canceledRange.value).toBe("5");
    expect(input).not.toHaveBeenCalled();
    expect(change).not.toHaveBeenCalled();
  });

  it.each(TEN_STATES)(
    "$id $label: Tab/Shift+Tab・見える現在位置・キー発火・pointer入力0",
    async (state) => {
      state.arrange();
      const audit = auditInput();
      try {
        render(state.node());
        const root = state.root();
        state.assertState(root);
        expectTabTraversal(root);

        const target = state.target(root);
        tabTo(root, target);
        expectFocusPresentation(root, target);
        await state.activateAndVerify(target, root);

        expect(audit.lowLevelEvents, "pointer/mouse/wheel/contextmenu").toEqual([]);
        expect(
          audit.clicks.filter(
            (click) => click.detail > 0 || click.pointerType !== "",
          ),
          "pointer由来click",
        ).toEqual([]);
      } finally {
        audit.stop();
      }
    },
  );
});
