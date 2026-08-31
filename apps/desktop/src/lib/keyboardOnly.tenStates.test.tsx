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

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
} {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
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

function expectSeparatedTipHandles(root: HTMLElement): void {
  const tipHandles = Array.from(
    root.querySelectorAll<SVGCircleElement>("[data-tip-handle]"),
  );
  expect(tipHandles).toHaveLength(12);
  for (let firstIndex = 0; firstIndex < tipHandles.length; firstIndex += 1) {
    const first = tipHandles[firstIndex];
    const firstRadius = Number(first.getAttribute("r"));
    const firstX = Number(first.getAttribute("cx"));
    const firstY = Number(first.getAttribute("cy"));
    for (
      let secondIndex = firstIndex + 1;
      secondIndex < tipHandles.length;
      secondIndex += 1
    ) {
      const second = tipHandles[secondIndex];
      const secondRadius = Number(second.getAttribute("r"));
      const requiredSeparation = (firstRadius + secondRadius) / 0.8;
      const separated =
        Math.abs(firstX - Number(second.getAttribute("cx"))) >=
          requiredSeparation ||
        Math.abs(firstY - Number(second.getAttribute("cy"))) >=
          requiredSeparation;
      expect(
        separated,
        `先端${first.dataset.tipHandle}と${second.dataset.tipHandle}の操作丸は、` +
          "実径を枠の80%以内に収める余裕を持って重ならない: " +
          `first=(${firstX},${firstY},r=${firstRadius}), ` +
          `second=(${second.getAttribute("cx")},${second.getAttribute("cy")},r=${secondRadius}), ` +
          `required=${requiredSeparation}`,
      ).toBe(true);
    }
  }
  expect(root.querySelectorAll("[data-tip-handle-leader]").length).toBeGreaterThan(0);
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
  if (target.matches('.help-search-control input[type="search"]')) {
    const searchRule =
      /\.help-search-control\s+input\[type="search"\]\s*\{([\s\S]*?)\}/u.exec(
        baseLayoutCss,
      )?.[1];
    expect(searchRule).toContain("outline: 0");
    expect(searchRule).toContain("box-shadow: none");

    const focusOwner = target.closest<HTMLElement>(".help-search-control");
    expect(focusOwner, "検索欄の代わりに選択輪を描く親要素").not.toBeNull();
    expect(focusOwner?.contains(target)).toBe(true);
    expect(
      focusOwner?.matches(":focus-within"),
      "検索欄を選ぶと親のfocus-withinが成立すること",
    ).toBe(true);
    const parentFocusRule =
      /\.help-search-control:focus-within\s*\{([\s\S]*?)\}/u.exec(dialogsCss)?.[1];
    expect(parentFocusRule).toContain("border-color: var(--color-accent)");
    expect(parentFocusRule).toContain("box-shadow: var(--focus-ring)");
    return;
  }
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

interface FocusViewportIssue {
  state: string;
  operation: string;
  reason: string;
  top?: number;
  bottom?: number;
  scrollTop?: number;
}

function modeledRect(top: number, height: number, width = 20): DOMRect {
  return {
    x: 0,
    y: top,
    left: 0,
    right: width,
    top,
    bottom: top + height,
    width,
    height,
    toJSON: () => ({}),
  } as DOMRect;
}

/**
 * 148件（旧147件）すべてが必ず通るviewport判定入口。「あとで確認する」追加で1件増えた。
 * jsdomは配置計算を持たないため、
 * HTML操作について座標0を合格根拠にせず、表示中・root内・fixedでないという
 * native focus scrollの前提を検査する。実機で外れた紙の12番目だけは、CDP実測
 * top=-32.16 / bottom=-12.31を回帰モデルにし、製品のfocus handlerが最寄りの
 * .paper-position-step.scrollTopを動かしてfocus輪ぶん8pxまで画面内へ戻すかを見る。
 */
function auditFocusViewport(
  state: TenStateCase,
  root: HTMLElement,
  target: FocusTarget,
): FocusViewportIssue | null {
  const operation = operationLedgerId(target);
  if (!root.contains(target)) {
    return { state: state.label, operation, reason: "状態rootの外にあります" };
  }
  if (!isDisplayedFocusTarget(target)) {
    return { state: state.label, operation, reason: "表示されていません" };
  }
  if (
    (target instanceof HTMLElement || target instanceof SVGElement) &&
    target.style.position === "fixed"
  ) {
    return { state: state.label, operation, reason: "fixed配置です" };
  }

  const isTwelfthPaperHandle =
    target.getAttribute("data-paper-position-handle") === "12";
  if (!isTwelfthPaperHandle) {
    focusTarget(target);
    return null;
  }

  const step = target.closest<HTMLElement>(".paper-position-step");
  if (step === null) {
    return { state: state.label, operation, reason: "縦送りの親がありません" };
  }
  const initialScrollTop = 100;
  const measuredTop = -32.16;
  const measuredHeight = 19.85;
  const focusRingInset = 8;
  step.scrollTop = initialScrollTop;
  Object.defineProperty(step, "getBoundingClientRect", {
    configurable: true,
    value: () => modeledRect(0, 350, 500),
  });
  Object.defineProperty(target, "getBoundingClientRect", {
    configurable: true,
    value: () =>
      modeledRect(
        measuredTop - (step.scrollTop - initialScrollTop),
        measuredHeight,
      ),
  });

  const active = document.activeElement as HTMLElement | SVGElement | null;
  if (active !== null && "blur" in active && typeof active.blur === "function") {
    act(() => active.blur());
  }
  focusTarget(target);
  const viewport = step.getBoundingClientRect();
  const focused = target.getBoundingClientRect();
  if (
    focused.top < viewport.top + focusRingInset ||
    focused.bottom > viewport.bottom - focusRingInset
  ) {
    return {
      state: state.label,
      operation,
      reason: "focus後もfocus輪を含めて表示範囲内へ戻りません",
      top: focused.top,
      bottom: focused.bottom,
      scrollTop: step.scrollTop,
    };
  }
  return null;
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
      expect(
        screen.getByRole("heading", {
          name: "作品を書き出す", // 旧「展開図・折り図を書き出す」→新「作品を書き出す」。5番目追加に伴う意図した照合値更新であり、緩和ではない。
        }),
      ).toBeTruthy();
      // ファイル名(利用者が選ぶ)は.user-textへ、前後の案内文(製品側の固定文言)は
      // 従来どおりで、文全体も1字も変わっていないことを両方検査する。
      const savedName = screen.getByText(LONG_NAME, { selector: ".user-text" });
      expect(savedName.closest(".hint")?.textContent).toBe(
        `保存しました:${LONG_NAME}`,
      );
      // 失敗理由は製品が組み立てる固定文言で利用者入力ではないため、.user-textの対象外
      // のまま文全体一致で検査する(据え置き、こちらは変更していない)。
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
      expectSeparatedTipHandles(root);
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
    assertState: (root) => {
      expect(screen.getByRole("status").textContent).toContain("4件中2件め");
      expect(screen.getByText("計算中…")).toBeTruthy();
      expect(root.querySelectorAll("[data-tip-handle-leader]")).toHaveLength(0);
      for (const handle of root.querySelectorAll<SVGCircleElement>(
        "[data-tip-handle]",
      )) {
        const id = handle.dataset.tipHandle;
        const line = root.querySelector<SVGLineElement>(
          `[data-preview-part="${id}"]`,
        );
        expect(line, `通常4本の先端${id}`).not.toBeNull();
        expect(handle.getAttribute("cx"), `通常4本の先端${id}の横位置`).toBe(
          line?.getAttribute("x2"),
        );
        expect(handle.getAttribute("cy"), `通常4本の先端${id}の縦位置`).toBe(
          line?.getAttribute("y2"),
        );
      }
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
          candidate_id: 8_002,
          autosave_path: `C:\\自動保存\\${"深い場所\\".repeat(18)}控え.ori3`,
          document_path: `C:\\作品\\${"深い場所\\".repeat(18)}折り鶴.ori3`,
          saved_at_ms: Date.UTC(2026, 7, 26, 0, 0, 0),
          step_count: null,
        },
      });
      const candidate = useAppStore.getState().recovery;
      if (candidate !== null) {
        useAppStore.setState({ recoveryChoices: [candidate] });
      }
    },
    node: () => <RecoveryDialog />,
    root: dialog,
    assertState: () => {
      expect(
        screen.getByRole("heading", {
          name: "前回の終了が正常に行われませんでした",
        }),
      ).toBeTruthy();
      // 旧値「元の作品:折り鶴…」→新値「元の作品: 折り鶴…」:
      // 候補一覧の日時・作品名・手順数を読み分ける区切り空白を加えたため。
      expect(screen.getByText(/元の作品: 折り鶴\.ori3/u)).toBeTruthy();
    },
    target: () => screen.getByRole("button", { name: "復元する" }) as FocusTarget,
    activateAndVerify: async (target) => {
      pressEnter(target as HTMLButtonElement);
      await waitFor(() => expect(useAppStore.getState().recovery).toBeNull());
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(ipc.recoveryRestore).toHaveBeenCalledWith(true, 8_002);
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
 * 148は上限ではなく、この固定標本で実測した全Tab停止位置の台帳。
 * 旧値147→新値148: 復旧画面へ「あとで確認する」が1停止位置増えたため。
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
  // 旧台帳2件→新台帳3件: 候補を保持して閉じる「あとで確認する」を追加したため。
  8: [
    buttonId("復元する"),
    buttonId("破棄する"),
    buttonId("あとで確認する"),
  ],
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

function buttonTarget(target: FocusTarget): HTMLButtonElement {
  expect(target).toBeInstanceOf(HTMLButtonElement);
  return target as HTMLButtonElement;
}

function inputTarget(target: FocusTarget, type: string): HTMLInputElement {
  expect(target).toBeInstanceOf(HTMLInputElement);
  const input = target as HTMLInputElement;
  expect(input.type).toBe(type);
  return input;
}

function shapeRowId(target: FocusTarget): number {
  const row = target.closest<HTMLElement>("[data-shape-row]");
  if (row === null) throw new Error(`${visibleName(target)}の形の行がありません`);
  const id = Number(row.dataset.shapeRow);
  if (!Number.isInteger(id)) throw new Error(`${visibleName(target)}の部位番号がありません`);
  return id;
}

function expectSkeletonRangeResult(
  target: FocusTarget,
  key: "ArrowLeft" | "ArrowRight",
): void {
  const range = inputTarget(target, "range");
  const nodeId = shapeRowId(range);
  const property = range.getAttribute("aria-label")?.endsWith("の太さ")
    ? "width_factor"
    : "length";
  const before = useAppStore
    .getState()
    .proposalSkeleton.nodes.find((node) => node.id === nodeId)?.[property];
  const expected = pressRangeArrow(range, key);
  const after = useAppStore
    .getState()
    .proposalSkeleton.nodes.find((node) => node.id === nodeId)?.[property];
  expect(expected, `${visibleName(range)}のDOM結果`).not.toBe(before);
  expect(after, `${visibleName(range)}の作品状態`).toBe(expected);
}

function expectSkeletonAddResult(target: FocusTarget): void {
  const parentId = shapeRowId(target);
  const before = useAppStore.getState().proposalSkeleton.nodes;
  const beforeIds = new Set(before.map((node) => node.id));
  pressEnter(buttonTarget(target));
  const after = useAppStore.getState().proposalSkeleton.nodes;
  const added = after.filter((node) => !beforeIds.has(node.id));
  expect(added, `${visibleName(target)}で足された部位`).toHaveLength(1);
  expect(added[0].parent).toBe(parentId);
}

function expectSkeletonRemoveResult(target: FocusTarget): void {
  const removedId = shapeRowId(target);
  const before = useAppStore.getState().proposalSkeleton.nodes;
  const removedIds = new Set([removedId]);
  let foundDescendant = true;
  while (foundDescendant) {
    foundDescendant = false;
    for (const node of before) {
      if (
        node.parent !== null &&
        removedIds.has(node.parent) &&
        !removedIds.has(node.id)
      ) {
        removedIds.add(node.id);
        foundDescendant = true;
      }
    }
  }
  pressEnter(buttonTarget(target));
  const after = useAppStore.getState().proposalSkeleton.nodes;
  for (const id of removedIds) {
    expect(
      after.some((node) => node.id === id),
      `${visibleName(target)}で対象とその先の部位${id}が消えること`,
    ).toBe(false);
  }
  expect(after).toHaveLength(before.length - removedIds.size);
}

async function expectProposalGeneration(target: FocusTarget): Promise<void> {
  pressEnter(buttonTarget(target));
  await waitFor(() => expect(ipc.proposalGenerate).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(useAppStore.getState().proposalBusy).toBe(false));
  expect(useAppStore.getState().proposalStep).toBe("candidates");
  expect(useAppStore.getState().proposalCandidates).toHaveLength(1);
  expect(useAppStore.getState().proposalCandidates[0].cp.next_edge_id).toBe(91);
}

async function expectNewStateOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  if (ledgerId.startsWith("input:radio:")) {
    const next = pressRadioArrow(inputTarget(target, "radio"), "ArrowRight");
    expect(visibleName(next)).toContain("長方形");
    expect(useAppStore.getState().newPaperDraft.square).toBe(false);
    return;
  }
  if (ledgerId.startsWith("input:number:")) {
    expect(pressNumberArrow(inputTarget(target, "number"), "ArrowUp")).toBe(151);
    expect(useAppStore.getState().newPaperDraft.widthMm).toBe(151);
    return;
  }
  if (ledgerId.includes("を増やす")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().newPaperDraft.widthMm).toBe(151);
    return;
  }
  if (ledgerId.includes("を減らす")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().newPaperDraft.widthMm).toBe(149);
    return;
  }
  if (ledgerId.includes("折り紙 15cm角")) {
    pressNumberArrow(
      screen.getByLabelText("紙の横の長さ（mm）") as HTMLInputElement,
      "ArrowUp",
    );
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().newPaperDraft).toEqual({
      widthMm: 150,
      heightMm: 150,
      square: true,
    });
    return;
  }
  if (ledgerId.includes("折り紙 24cm角")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().newPaperDraft).toEqual({
      widthMm: 240,
      heightMm: 240,
      square: true,
    });
    return;
  }
  if (ledgerId.includes("A4の紙")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().newPaperDraft).toEqual({
      widthMm: 297,
      heightMm: 210,
      square: false,
    });
    return;
  }
  if (ledgerId.includes("新しい作品を始めます")) {
    pressEnter(buttonTarget(target));
    await waitFor(() => expect(ipc.documentNew).toHaveBeenCalledTimes(1));
    expect(ipc.documentNew).toHaveBeenCalledWith({
      width_mm: 150,
      height_mm: 150,
    });
    expect(useAppStore.getState().newDialogOpen).toBe(false);
    return;
  }
  if (ledgerId.includes("新規作成をやめます")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().newDialogOpen).toBe(false);
    return;
  }
  throw new Error(`新規作成の結果が未定義です: ${ledgerId}`);
}

async function expectExportStateOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  if (ledgerId.startsWith("input:radio:")) {
    const next = pressRadioArrow(inputTarget(target, "radio"), "ArrowLeft");
    expect(visibleName(next)).toContain("展開図(SVG)");
    expect(useAppStore.getState().exportKind).toBe("CpSvg");
    return;
  }
  if (ledgerId.startsWith("input:checkbox:")) {
    pressSpace(inputTarget(target, "checkbox"));
    expect(useAppStore.getState().exportIncludeAux).toBe(true);
    return;
  }
  if (ledgerId.startsWith("input:number:")) {
    expect(pressNumberArrow(inputTarget(target, "number"), "ArrowDown")).toBe(
      16128,
    );
    expect(useAppStore.getState().exportLongSide).toBe(16128);
    return;
  }
  if (ledgerId.includes("を増やす")) {
    const number = screen.getByLabelText(
      "画像の大きさ（長辺の点数）",
    ) as HTMLInputElement;
    expect(pressNumberArrow(number, "ArrowDown")).toBe(16128);
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().exportLongSide).toBe(16384);
    return;
  }
  if (ledgerId.includes("を減らす")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().exportLongSide).toBe(16128);
    return;
  }
  if (ledgerId.includes("保存先を選んで書き出す")) {
    pressEnter(buttonTarget(target));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(ipc.documentExport).toHaveBeenCalledTimes(1));
    expect(ipc.documentExport).toHaveBeenCalledWith(
      "CpPng",
      "C:\\検査\\keyboard-ledger.png",
      { include_aux: false, png_long_side: 16384 },
    );
    await waitFor(() =>
      expect(useAppStore.getState().exportSavedPath).toBe(
        "C:\\検査\\keyboard-ledger.png",
      ),
    );
    return;
  }
  if (ledgerId === buttonId("閉じる")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().exportOpen).toBe(false);
    return;
  }
  throw new Error(`書き出しの結果が未定義です: ${ledgerId}`);
}

async function expectDeepProposalOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  if (target.hasAttribute("data-tip-handle")) {
    const leafId = Number(target.getAttribute("data-tip-handle"));
    expect(
      useAppStore.getState().proposalSkeleton.nodes.find((node) => node.id === leafId)
        ?.tip_pos_2d,
    ).toBeUndefined();
    fireEvent.keyDown(target, { key: "ArrowRight" });
    fireEvent.keyUp(target, { key: "ArrowRight" });
    expect(
      useAppStore.getState().proposalSkeleton.nodes.find((node) => node.id === leafId)
        ?.tip_pos_2d,
    ).toBeDefined();
    expect(useAppStore.getState().proposalPositionUndoStack).toHaveLength(1);
    return;
  }
  if (ledgerId.startsWith("input:range:")) {
    expectSkeletonRangeResult(target, "ArrowLeft");
    return;
  }
  if (ledgerId.endsWith("のこの先に足す")) {
    expectSkeletonAddResult(target);
    return;
  }
  if (ledgerId.endsWith("とその先を消す")) {
    expectSkeletonRemoveResult(target);
    return;
  }
  if (ledgerId === buttonId("展開図を作ってもらう")) {
    await expectProposalGeneration(target);
    return;
  }
  if (ledgerId === buttonId("やめる")) {
    const pending = deferred<Awaited<ReturnType<typeof ipc.proposalGenerate>>>();
    let requestedJobId = "";
    vi.mocked(ipc.proposalGenerate).mockImplementation(
      (_skeleton, _paper, _seed, jobId) => {
        requestedJobId = jobId;
        return pending.promise;
      },
    );
    vi.mocked(ipc.proposalControl).mockImplementation((operation) =>
      Promise.resolve({
        job_id: operation.job_id,
        done: 0,
        total: 0,
        phase: "Cancelled",
      }),
    );
    let generation!: Promise<void>;
    act(() => {
      useAppStore.setState({
        proposalBusy: false,
        proposalJobId: null,
        proposalProgress: null,
      });
      generation = useAppStore.getState().generateProposal();
    });
    await waitFor(() => expect(useAppStore.getState().proposalBusy).toBe(true));
    const close = screen.getByRole("button", { name: "やめる" });
    expect(close).toBeInstanceOf(HTMLButtonElement);
    focusTarget(close as HTMLButtonElement);
    pressEnter(close as HTMLButtonElement);
    await waitFor(() =>
      expect(ipc.proposalControl).toHaveBeenCalledWith({
        type: "Cancel",
        job_id: requestedJobId,
      }),
    );
    expect(useAppStore.getState().proposalStep).toBeNull();
    expect(useAppStore.getState().proposalCandidates).toEqual([]);
    await act(async () => {
      pending.resolve({
        job_id: requestedJobId,
        candidates: [candidate(778)],
      });
      await generation;
    });
    expect(useAppStore.getState().proposalStep).toBeNull();
    expect(useAppStore.getState().proposalCandidates).toEqual([]);
    return;
  }
  throw new Error(`形を決める画面の結果が未定義です: ${ledgerId}`);
}

async function expectCandidateOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  const candidateMatch = /^button:候補([1-4])$/u.exec(ledgerId);
  if (candidateMatch !== null) {
    const expected = Number(candidateMatch[1]) - 1;
    if (expected === 0) {
      pressEnter(screen.getByRole("button", { name: "候補2" }));
      expect(useAppStore.getState().proposalSelected).toBe(1);
    }
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().proposalSelected).toBe(expected);
    expect(target.getAttribute("aria-pressed")).toBe("true");
    return;
  }
  if (ledgerId === buttonId("形を直す")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().proposalStep).toBe("skeleton");
    return;
  }
  if (ledgerId === buttonId("別の置き方も見る")) {
    await expectProposalGeneration(target);
    return;
  }
  if (ledgerId === buttonId("紙の上の場所も調整")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().proposalStep).toBe("paper-position");
    expect(useAppStore.getState().proposalPaperPositions).toHaveLength(4);
    return;
  }
  if (ledgerId === buttonId("これにする")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().proposalStep).toBe("confirm");
    return;
  }
  throw new Error(`候補画面の結果が未定義です: ${ledgerId}`);
}

async function expectPaperPositionOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  if (target.hasAttribute("data-paper-position-handle")) {
    const leafId = Number(target.getAttribute("data-paper-position-handle"));
    const before = useAppStore
      .getState()
      .proposalPaperPositions.find((entry) => entry.leaf_id === leafId)?.position.x;
    fireEvent.keyDown(target, { key: "ArrowRight" });
    fireEvent.keyUp(target, { key: "ArrowRight" });
    const state = useAppStore.getState();
    const after = state.proposalPaperPositions.find(
      (entry) => entry.leaf_id === leafId,
    )?.position.x;
    expect(after).toBeGreaterThan(before ?? Number.POSITIVE_INFINITY);
    expect(
      state.proposalPaperSpecified.some((entry) => entry.leaf_id === leafId),
    ).toBe(true);
    expect(target.getAttribute("data-paper-position-changed")).toBe("true");
    return;
  }
  if (ledgerId === buttonId("候補へ戻る")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().proposalStep).toBe("candidates");
    return;
  }
  if (ledgerId === buttonId("この場所で作り直す")) {
    await expectProposalGeneration(target);
    return;
  }
  throw new Error(`紙上の場所画面の結果が未定義です: ${ledgerId}`);
}

async function expectConfirmOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  if (ledgerId === buttonId("選び直す")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().proposalStep).toBe("candidates");
    return;
  }
  if (ledgerId === buttonId("この展開図を使う")) {
    pressEnter(buttonTarget(target));
    await waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));
    expect(ipc.editApply).toHaveBeenCalledWith({
      type: "ReplaceCreasePattern",
      cp: expect.objectContaining({ next_edge_id: 40 }),
    });
    expect(useAppStore.getState().proposalStep).toBeNull();
    return;
  }
  throw new Error(`確認画面の結果が未定義です: ${ledgerId}`);
}

async function expectBusyProposalOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  if (ledgerId.startsWith("input:range:")) {
    expectSkeletonRangeResult(target, "ArrowRight");
    return;
  }
  if (ledgerId.endsWith("のこの先に足す")) {
    expectSkeletonAddResult(target);
    return;
  }
  if (ledgerId.endsWith("とその先を消す")) {
    expectSkeletonRemoveResult(target);
    return;
  }
  if (ledgerId === buttonId("出っぱりを増やす")) {
    const beforeIds = new Set(
      useAppStore.getState().proposalSkeleton.nodes.map((node) => node.id),
    );
    pressEnter(buttonTarget(target));
    const added = useAppStore
      .getState()
      .proposalSkeleton.nodes.filter((node) => !beforeIds.has(node.id));
    expect(added).toHaveLength(1);
    expect(added[0].parent).toBe(0);
    return;
  }
  if (ledgerId === buttonId("やめる")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().proposalStep).toBeNull();
    return;
  }
  throw new Error(`処理中画面の結果が未定義です: ${ledgerId}`);
}

async function expectRecoveryOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  if (ledgerId === buttonId("あとで確認する")) {
    // 旧台帳には未登録→新台帳では、候補を消すIPCを呼ばず画面だけ閉じる結果を固定する。
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().recoveryDismissed).toBe(true);
    expect(useAppStore.getState().recovery).not.toBeNull();
    expect(ipc.recoveryRestore).not.toHaveBeenCalled();
    return;
  }
  const accept = ledgerId === buttonId("復元する");
  if (!accept && ledgerId !== buttonId("破棄する")) {
    throw new Error(`復旧画面の結果が未定義です: ${ledgerId}`);
  }
  pressEnter(buttonTarget(target));
  await waitFor(() =>
    expect(ipc.recoveryRestore).toHaveBeenCalledWith(accept, 8_002),
  );
  expect(useAppStore.getState().recovery).toBeNull();
}

async function expectHelpOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  if (ledgerId === buttonId("ヘルプセンターを閉じる")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().helpOpen).toBe(false);
    return;
  }
  if (ledgerId.startsWith("input:search:")) {
    const search = inputTarget(target, "search");
    fireEvent.keyDown(search, { key: "折" });
    fireEvent.change(search, { target: { value: "折" } });
    fireEvent.keyUp(search, { key: "折" });
    expect(useAppStore.getState().helpQuery).toBe("折");
    expect(search.value).toBe("折");
    return;
  }
  const chapter = HELP_CHAPTER_LEDGER.find(
    ([name]) => ledgerId === buttonId(name),
  );
  if (chapter !== undefined) {
    if (chapter[1] === "overview") {
      const other = screen.getByRole("button", {
        name: /ショートカット一覧/u,
      });
      expect(other).toBeInstanceOf(HTMLButtonElement);
      pressEnter(other as HTMLButtonElement);
      expect(useAppStore.getState().helpChapterId).toBe("shortcuts");
      focusTarget(target);
    }
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().helpChapterId).toBe(chapter[1]);
    expect(target.getAttribute("aria-current")).toBe("page");
    return;
  }
  if (ledgerId === buttonId("基本操作ガイドをもう一度")) {
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().helpOpen).toBe(false);
    expect(useAppStore.getState().guideOpen).toBe(true);
    expect(useAppStore.getState().guideStep).toBe(0);
    return;
  }
  throw new Error(`ヘルプ画面の結果が未定義です: ${ledgerId}`);
}

async function expectCpOperation(
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  if (ledgerId === buttonId("展開図の詳しい操作方法 ▼")) {
    expect(target.getAttribute("aria-expanded")).toBe("false");
    pressEnter(buttonTarget(target));
    expect(useAppStore.getState().cpHelpExpanded).toBe(true);
    expect(target.getAttribute("aria-expanded")).toBe("true");
    return;
  }
  if (target instanceof HTMLCanvasElement) {
    const start = useAppStore.getState().lineInputStart;
    expect(start).not.toBeNull();
    fireEvent.keyDown(target, { key: "ArrowRight" });
    fireEvent.keyUp(target, { key: "ArrowRight" });
    fireEvent.keyDown(target, { key: "Enter" });
    fireEvent.keyUp(target, { key: "Enter" });
    await waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));
    const operation = vi.mocked(ipc.editApply).mock.calls[0][0];
    expect(operation.type).toBe("AddSegment");
    if (operation.type !== "AddSegment") {
      throw new Error("展開図からAddSegment以外が送られました");
    }
    expect(operation.a).toEqual(start);
    expect(operation.b).not.toEqual(start);
    await waitFor(() => expect(useAppStore.getState().lineInputStart).toBeNull());
    expect(useAppStore.getState().operationStage).toBe(2);
    return;
  }
  if (ledgerId === "div:展開図に表示している手順") {
    // これは実行操作ではなく、キーボード利用者が現在の手順を読めるfocus対象。
    const before = {
      lineInputStart: useAppStore.getState().lineInputStart,
      operationStage: useAppStore.getState().operationStage,
    };
    fireEvent.keyDown(target, { key: "Enter" });
    fireEvent.keyUp(target, { key: "Enter" });
    expect(ipc.editApply).not.toHaveBeenCalled();
    expect({
      lineInputStart: useAppStore.getState().lineInputStart,
      operationStage: useAppStore.getState().operationStage,
    }).toEqual(before);
    return;
  }
  throw new Error(`展開図の結果が未定義です: ${ledgerId}`);
}

async function expectOperationResult(
  stateId: number,
  target: FocusTarget,
  ledgerId: string,
): Promise<void> {
  switch (stateId) {
    case 1:
      return expectNewStateOperation(target, ledgerId);
    case 2:
      return expectExportStateOperation(target, ledgerId);
    case 3:
      return expectDeepProposalOperation(target, ledgerId);
    case 4:
      return expectCandidateOperation(target, ledgerId);
    case 5:
      return expectPaperPositionOperation(target, ledgerId);
    case 6:
      return expectConfirmOperation(target, ledgerId);
    case 7:
      return expectBusyProposalOperation(target, ledgerId);
    case 8:
      return expectRecoveryOperation(target, ledgerId);
    case 9:
      return expectHelpOperation(target, ledgerId);
    case 10:
      return expectCpOperation(target, ledgerId);
    default:
      throw new Error(`未知の固定状態です: ${stateId}`);
  }
}

interface RadioChoiceCase {
  stateId: 1 | 2;
  id: string;
  label: string;
  expected: boolean | "CpSvg" | "CpPng" | "FoldJson";
  disabled: boolean;
}

const RADIO_CHOICE_CASES: readonly RadioChoiceCase[] = [
  {
    stateId: 1,
    id: "1:input:radio:正方形(たて・よこが同じ)",
    label: "正方形(たて・よこが同じ)",
    expected: true,
    disabled: false,
  },
  {
    stateId: 1,
    id: "1:input:radio:長方形(たて・よこを別に決める)",
    label: "長方形(たて・よこを別に決める)",
    expected: false,
    disabled: false,
  },
  {
    stateId: 2,
    id: "2:input:radio:展開図(SVG)",
    label: "展開図(SVG)",
    expected: "CpSvg",
    disabled: false,
  },
  {
    stateId: 2,
    id: "2:input:radio:展開図(PNG)",
    label: "展開図(PNG)",
    expected: "CpPng",
    disabled: false,
  },
  {
    stateId: 2,
    id: "2:input:radio:折り図(PDF)",
    label: "折り図(PDF)",
    expected: "CpPng",
    disabled: true,
  },
  {
    stateId: 2,
    id: "2:input:radio:折り図(ページごとのSVG)",
    label: "折り図(ページごとのSVG)",
    expected: "CpPng",
    disabled: true,
  },
  // 旧: 新規2＋書き出し4＝全6 → 新: 新規2＋書き出し5＝全7。5番目の正式追加を照合する更新であり、緩和ではない。
  {
    stateId: 2,
    id: "2:input:radio:ほかの折り紙ソフトのファイル",
    label: "ほかの折り紙ソフトのファイル",
    expected: "FoldJson",
    disabled: false,
  },
];

function mountRadioState(stateId: 1 | 2): TenStateCase {
  cleanup();
  useAppStore.setState(initialStoreState, true);
  const state = TEN_STATES.find((candidate) => candidate.id === stateId);
  if (state === undefined) throw new Error(`radioの固定状態${stateId}がありません`);
  state.arrange();
  render(state.node());
  state.root();
  return state;
}

function namedRadio(label: string): HTMLInputElement {
  const radio = screen.getByLabelText(label);
  expect(radio).toBeInstanceOf(HTMLInputElement);
  return radio as HTMLInputElement;
}

function expectRadioStoreResult(testCase: RadioChoiceCase): void {
  if (testCase.stateId === 1) {
    expect(useAppStore.getState().newPaperDraft.square).toBe(testCase.expected);
  } else {
    expect(useAppStore.getState().exportKind).toBe(testCase.expected);
  }
}

function selectRadioByArrow(target: HTMLInputElement): void {
  let current = Array.from(
    document.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
  ).find((radio) => radio.name === target.name && radio.checked && !radio.disabled);
  if (current === undefined) throw new Error(`${visibleName(target)}の現在値がありません`);
  for (let index = 0; index <= 8; index += 1) {
    if (current === target) return;
    current = pressRadioArrow(current, "ArrowRight");
  }
  throw new Error(`${visibleName(target)}へ矢印キーで到達できません`);
}

function radioChoiceId(stateId: 1 | 2, radio: HTMLInputElement): string {
  return `${stateId}:${operationLedgerId(radio)}`;
}

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState(initialStoreState, true);
  vi.mocked(save).mockResolvedValue("C:\\検査\\keyboard-ledger.png");
  vi.mocked(ipc.documentNew).mockResolvedValue(viewOf());
  vi.mocked(ipc.documentExport).mockResolvedValue([]);
  vi.mocked(ipc.editApply).mockResolvedValue(viewOf());
  vi.mocked(ipc.proposalApply).mockResolvedValue(viewOf());
  vi.mocked(ipc.proposalGenerate).mockImplementation(
    async (_skeleton, _paper, _seed, jobId) => ({
      job_id: jobId,
      candidates: [candidate(91)],
    }),
  );
  // 旧mock未設定（undefined）→新mock null: 復元・破棄後は残存候補を再照会し、
  // この固定状態では「残りなし」をIPC契約どおり表すため。
  vi.mocked(ipc.recoveryCheck).mockResolvedValue(null);
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
  it("深い形の先端12個は大きさを変えず、余裕を持って重ならない", () => {
    const state = TEN_STATES[2];
    state.arrange();
    render(state.node());
    const root = state.root();
    state.assertState(root);
    expectSeparatedTipHandles(root);
  });

  it("深い形の先端を1つ決めた後も、12個の操作丸は余裕を持って重ならない", () => {
    const state = TEN_STATES[2];
    state.arrange();
    render(state.node());
    const root = state.root();
    const firstHandle = root.querySelector<SVGCircleElement>("[data-tip-handle]");
    if (firstHandle === null) throw new Error("先端の操作丸がありません");
    const leafId = Number(firstHandle.dataset.tipHandle);

    fireEvent.keyDown(firstHandle, { key: "ArrowRight" });
    fireEvent.keyUp(firstHandle, { key: "ArrowRight" });

    const decidedHandle = root.querySelector<SVGCircleElement>(
      `[data-tip-handle="${leafId}"]`,
    );
    const limb = root.querySelector<SVGLineElement>(
      `[data-preview-part="${leafId}"]`,
    );
    if (decidedHandle === null || limb === null) {
      throw new Error(`先端${leafId}の操作丸または枝線がありません`);
    }
    expect(decidedHandle.dataset.tipDecided).toBe("true");
    expect(Number(decidedHandle.getAttribute("cx"))).toBeCloseTo(
      Number(limb.getAttribute("x2")),
    );
    expect(Number(decidedHandle.getAttribute("cy"))).toBeCloseTo(
      Number(limb.getAttribute("y2")),
    );
    expectSeparatedTipHandles(root);
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

  it.each(TEN_STATES)(
    "$id $label: 全Tab停止位置を名前付き台帳と照合し、同数の入替も見逃さない",
    (state) => {
      state.arrange();
      render(state.node());
      const actual = focusableElements(state.root()).map(operationLedgerId);
      expect(new Set(actual).size, `${state.label}の重複しない操作ID`).toBe(
        actual.length,
      );
      expect(actual).toEqual(EXPECTED_OPERATION_LEDGER[state.id]);
    },
  );

  it("現在の台帳は10状態の全148停止位置で、1件は読むためだけのfocus対象と明記する", () => {
    // 旧値147→新値148: 復旧画面の「あとで確認する」1件を追加したため。
    expect(OPERATION_RESULT_CASES).toHaveLength(148);
    expect(
      OPERATION_RESULT_CASES.filter(
        ({ ledgerId }) => ledgerId === "div:展開図に表示している手順",
      ),
    ).toHaveLength(1);
  });

  it("10状態の148 Tab停止位置を構造監査し、既知の負座標focusを表示範囲へ戻す", () => {
    const audited: string[] = [];
    const issues: FocusViewportIssue[] = [];
    const originalInnerWidth = Object.getOwnPropertyDescriptor(
      window,
      "innerWidth",
    );
    const originalInnerHeight = Object.getOwnPropertyDescriptor(
      window,
      "innerHeight",
    );

    try {
      Object.defineProperty(window, "innerWidth", {
        configurable: true,
        value: 500,
      });
      Object.defineProperty(window, "innerHeight", {
        configurable: true,
        value: 350,
      });
      for (const state of TEN_STATES) {
        cleanup();
        useAppStore.setState(initialStoreState, true);
        state.arrange();
        render(state.node());
        const root = state.root();
        const targets = focusableElements(root);
        expect(targets.map(operationLedgerId)).toEqual(
          EXPECTED_OPERATION_LEDGER[state.id],
        );
        for (const target of targets) {
          audited.push(`${state.id}:${operationLedgerId(target)}`);
          const issue = auditFocusViewport(state, root, target);
          if (issue !== null) issues.push(issue);
        }
      }

      // 同じ画面外モデルでも通常幅では製品側が縦位置を動かさないことを固定する。
      Object.defineProperty(window, "innerWidth", {
        configurable: true,
        value: 1000,
      });
      cleanup();
      useAppStore.setState(initialStoreState, true);
      const paperState = TEN_STATES.find(
        (state) => state.id === 5,
      );
      if (paperState === undefined)
        throw new Error("紙上12個の固定状態がありません");
      paperState.arrange();
      render(paperState.node());
      const paperRoot = paperState.root();
      const paperHandle = paperRoot.querySelector<FocusTarget>(
        '[data-paper-position-handle="12"]',
      );
      const paperStep = paperRoot.querySelector<HTMLElement>(
        ".paper-position-step",
      );
      if (paperHandle === null || paperStep === null)
        throw new Error("通常幅を確認する紙の丸印と縦送り領域がありません");
      paperStep.scrollTop = 100;
      Object.defineProperty(paperStep, "getBoundingClientRect", {
        configurable: true,
        value: () => modeledRect(0, 350, 500),
      });
      Object.defineProperty(paperHandle, "getBoundingClientRect", {
        configurable: true,
        value: () => modeledRect(-32.16, 19.85),
      });
      const activeAtNormalWidth = document.activeElement as
        | HTMLElement
        | SVGElement
        | null;
      if (
        activeAtNormalWidth !== null &&
        "blur" in activeAtNormalWidth &&
        typeof activeAtNormalWidth.blur === "function"
      ) {
        act(() => activeAtNormalWidth.blur());
      }
      focusTarget(paperHandle);
      expect(paperStep.scrollTop, "通常1000pxでは縦位置を変えません").toBe(
        100,
      );
    } finally {
      if (originalInnerWidth === undefined)
        Reflect.deleteProperty(window, "innerWidth");
      else Object.defineProperty(window, "innerWidth", originalInnerWidth);
      if (originalInnerHeight === undefined)
        Reflect.deleteProperty(window, "innerHeight");
      else Object.defineProperty(window, "innerHeight", originalInnerHeight);
    }

    // 旧値147→新値148: 復旧画面の保留操作も全数構造監査へ含める。
    expect(audited).toHaveLength(148);
    expect(new Set(audited).size).toBe(148);
    expect(
      issues,
      "構造148件を全数監査し、実機で再現した負座標モデル1件はfocus後に表示範囲へ戻します。実座標全数はCDPで測ります。",
    ).toEqual([]);
  });

  // 旧「全6」→新「全7」。書き出しの5番目を加えた意図変更の照合であり、期待値の緩和ではない。
  it("radioの全7選択肢は、名前付き台帳と無効状態まで完全一致する", () => {
    const actual = ([1, 2] as const).flatMap((stateId) => {
      mountRadioState(stateId);
      return Array.from(
        document.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
      ).map((radio) => ({
        id: radioChoiceId(stateId, radio),
        disabled: radio.disabled,
      }));
    });
    const expected = RADIO_CHOICE_CASES.map(({ id, disabled }) => ({
      id,
      disabled,
    }));
    expect(new Set(actual.map(({ id }) => id)).size).toBe(actual.length);
    expect(actual).toEqual(expected);
  });

  it.each(OPERATION_RESULT_CASES)(
    "$ledgerId: 状態を独立mountし、配送だけでなく製品の操作結果まで確かめる",
    async ({ state, ledgerId }) => {
      state.arrange();
      const audit = auditInput();
      try {
        render(state.node());
        const root = state.root();
        const targets = focusableElements(root).filter(
          (candidate) => operationLedgerId(candidate) === ledgerId,
        );
        expect(targets, `${state.label}の${ledgerId}`).toHaveLength(1);
        const target = targets[0];
        focusTarget(target);
        await expectOperationResult(state.id, target, ledgerId);
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

  it.each(RADIO_CHOICE_CASES)(
    "radio $stateId $label: 全選択肢の矢印キー・Space結果を別々のmountで確かめる",
    (testCase) => {
      mountRadioState(testCase.stateId);
      let target = namedRadio(testCase.label);
      expect(target.disabled).toBe(testCase.disabled);

      if (testCase.disabled) {
        const selected = Array.from(
          document.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
        ).find((radio) => radio.name === target.name && radio.checked);
        if (selected === undefined) throw new Error("現在の選択肢がありません");
        const moved = pressRadioArrow(selected, "ArrowRight");
        expect(moved).not.toBe(target);
        const beforeDisabledSpace = useAppStore.getState().exportKind;
        act(() => target.focus());
        pressSpace(target);
        expect(target.checked).toBe(false);
        expect(useAppStore.getState().exportKind).toBe(beforeDisabledSpace);
        return;
      }

      if (target.checked) {
        const other = Array.from(
          document.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
        ).find(
          (radio) =>
            radio.name === target.name &&
            !radio.disabled &&
            radio !== target,
        );
        if (other === undefined) {
          throw new Error(`${testCase.id}を矢印で選び直す別選択肢がありません`);
        }
        selectRadioByArrow(other);
        expect(other.checked).toBe(true);
        target = namedRadio(testCase.label);
      }
      selectRadioByArrow(target);
      expect(target.checked).toBe(true);
      expect(document.activeElement).toBe(target);
      expectRadioStoreResult(testCase);

      mountRadioState(testCase.stateId);
      target = namedRadio(testCase.label);
      const other = Array.from(
        document.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
      ).find(
        (radio) =>
          radio.name === target.name &&
          !radio.disabled &&
          radio !== target,
      );
      if (other !== undefined && target.checked) {
        selectRadioByArrow(other);
        expect(other.checked).toBe(true);
      }
      act(() => target.focus({ preventScroll: true }));
      pressSpace(target);
      expect(target.checked).toBe(true);
      expectRadioStoreResult(testCase);
    },
  );

  it("処理中に形をキーボードで変えた場合、変更前の形から届いた候補を採用しない", async () => {
    const state = TEN_STATES.find(({ id }) => id === 3);
    if (state === undefined) throw new Error("Proposal・skeleton状態がありません");
    state.arrange();
    render(state.node());
    state.root();

    const pending = deferred<Awaited<ReturnType<typeof ipc.proposalGenerate>>>();
    let requestedJobId = "";
    vi.mocked(ipc.proposalGenerate).mockImplementation(
      (_skeleton, _paper, _seed, jobId) => {
        requestedJobId = jobId;
        return pending.promise;
      },
    );

    let generation!: Promise<void>;
    act(() => {
      generation = useAppStore.getState().generateProposal();
    });
    await waitFor(() => expect(useAppStore.getState().proposalBusy).toBe(true));

    const target = screen.getAllByRole("slider", { name: /の長さ$/ })[0];
    expect(target).toBeInstanceOf(HTMLInputElement);
    const input = target as HTMLInputElement;
    const before = Number(input.value);
    focusTarget(input);
    const after = pressRangeArrow(input, "ArrowLeft");
    expect(after, "処理中の形編集が実際に製品状態へ反映されたこと").not.toBe(
      before,
    );
    expect(useAppStore.getState().proposalSkeleton.nodes[1]?.length).toBe(after);

    await act(async () => {
      pending.resolve({
        job_id: requestedJobId,
        candidates: [candidate(777)],
      });
      await generation;
    });

    expect(
      useAppStore.getState().proposalCandidates,
      "入力変更前の形を使った古い計算結果は、現在の形の候補として採用しない",
    ).toEqual([]);
    expect(useAppStore.getState().proposalStep).toBe("skeleton");
  });
});
