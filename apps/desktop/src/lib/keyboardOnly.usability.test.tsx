// @vitest-environment jsdom
// マウスを使わず、展開図へ線を引く→形を折る→元へ戻す、までを実DOMで通す。

import { readFileSync } from "node:fs";
import { useRef } from "react";
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
import { AppToolbar } from "../components/AppToolbar";
import { ContextPanel } from "../components/ContextPanel";
import { ContextPanelSplitter } from "../components/ContextPanelSplitter";
import { CpEditor } from "../components/CpEditor/CpEditor";
import type { RenderOverlay } from "../components/CpEditor/renderer";
import { HistoryShortcuts } from "../components/HistoryShortcuts";
import { PaneSplitter } from "../components/PaneSplitter";
import { Timeline } from "../components/Timeline";
import { ToolRail } from "../components/ToolRail";
import { Viewer3D } from "../components/Viewer3D/Viewer3D";
import { DEFAULT_CONSTRUCT } from "./construct";
import { DEFAULT_CURVE } from "./curve";
import type {
  Document,
  DocumentView,
  EditOp,
  FoldAllPreviewOutcome,
  Frame3D,
} from "./types";

const held = vi.hoisted(() => ({ overlay: null as unknown }));

vi.mock("../components/CpEditor/renderer", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../components/CpEditor/renderer")>();
  return {
    ...actual,
    render: vi.fn((...args: unknown[]) => {
      held.overlay = args[7];
    }),
  };
});

// WebGLだけはjsdomに無いためscene lifetimeを無害化する。Viewer3D自身、
// ViewCube、視点を戻すボタン、pointer handlerを持つcanvasは実DOMを使う。
vi.mock("../components/Viewer3D/sceneBuilder", async (importOriginal) => {
  const actual =
    await importOriginal<
      typeof import("../components/Viewer3D/sceneBuilder")
    >();
  const THREE = await import("three");
  return {
    ...actual,
    createScene: () => {
      const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
      camera.position.set(0.5, 0.5, 2);
      camera.lookAt(0.5, 0.5, 0);
      camera.updateMatrixWorld(true);
      camera.updateProjectionMatrix();
      const scene = {
        camera,
        contentGroup: new THREE.Group(),
        highlightGroup: new THREE.Group(),
        content: null as unknown,
        soft: null as unknown,
        pickSurface: null as unknown,
        render: vi.fn(),
        syncTheme: vi.fn(),
        resize: vi.fn(),
        resetCamera: vi.fn(),
        setContent: vi.fn((content: unknown) => {
          scene.content = content;
        }),
        setSupplementalEdges: vi.fn(),
        setHighlight: vi.fn(),
        setPreview: vi.fn(),
        setSoft: vi.fn((content: unknown) => {
          scene.soft = content;
        }),
        setDrawMode: vi.fn(),
        dispose: vi.fn(),
      };
      return scene;
    },
  };
});

vi.mock("../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editApplyBatch: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  foldAllPreview: vi.fn(),
  poseSolve: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
}));

import * as ipc from "../ipc/client";
import {
  resetFoldAllPreviewRuntime,
  resetPoseThrottle,
  useAppStore,
} from "../store/appStore";

const initialStoreState = useAppStore.getState();
const baseLayoutCss = readFileSync("src/styles/base-layout.css", "utf8");
const viewerCss = readFileSync("src/styles/viewer.css", "utf8");

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

function documentWithCrease(): Document {
  const doc = structuredClone(BASE_DOCUMENT);
  doc.cp.edges.push({ id: 4, v0: 0, v1: 2, kind: "Mountain" });
  doc.cp.next_edge_id = 5;
  return doc;
}

function viewOf(doc: Document): DocumentView {
  return {
    doc: structuredClone(doc),
    faces: [
      { id: 0, vertices: [0, 1, 2], edges: [0, 1, 4] },
      { id: 1, vertices: [0, 2, 3], edges: [4, 2, 3] },
    ],
    warnings: [],
    violations: [],
    frame: null,
    skipped: [],
    contact_detected: false,
  };
}

function frameAt(percent: number): Frame3D {
  return {
    faces: [
      {
        face: 0,
        polygon: [
          [0, 0, 0],
          [1, 0, 0],
          [0, 1, percent / 100],
        ],
        layer: 0,
        surface_rank: 0,
        mirrored: false,
      },
    ],
    warnings: [],
  };
}

function foldOutcome(percent: number): FoldAllPreviewOutcome {
  return {
    frame: frameAt(percent),
    converged: true,
    angles: { "4": percent * 1.8 },
    iterations: 1,
    requested_percent: percent,
    requested_angles: [{ hinge: 4, target_angle_deg: percent * 1.8 }],
    next_warm_seed: [{ hinge: 4, target_angle_deg: percent * 1.8 }],
    suspect_hinges: [],
    contact_detected: false,
    flat_fold_violations: [],
    layer_order: "unavailable_without_sequence",
  };
}

function overlay(): RenderOverlay {
  if (held.overlay === null) {
    throw new Error("展開図の表示状態がまだ作られていません");
  }
  return held.overlay as RenderOverlay;
}

function KeyboardWorkspace() {
  const fit2dRef = useRef<(() => void) | null>(null);
  const fit3dRef = useRef<(() => void) | null>(null);
  return (
    <main className="app" data-testid="keyboard-workspace">
      <AppToolbar onOpenHelp={() => useAppStore.getState().openHelp()} />
      <div className="main-row">
        <ToolRail
          onFitView={() => {
            fit2dRef.current?.();
            fit3dRef.current?.();
          }}
        />
        <section className="pane pane-2d">
          <CpEditor fitRef={fit2dRef} />
        </section>
        <PaneSplitter />
        <section className="pane pane-3d">
          <div className="pane-3d-view">
            <Viewer3D fitRef={fit3dRef} />
          </div>
          <Timeline />
        </section>
      </div>
      <ContextPanelSplitter />
      <ContextPanel />
      <HistoryShortcuts />
    </main>
  );
}

function seed(doc: Document = BASE_DOCUMENT): void {
  const hasCrease = doc.cp.edges.some(
    (edge) => edge.kind === "Mountain" || edge.kind === "Valley",
  );
  useAppStore.setState({
    doc: structuredClone(doc),
    docEpoch: 1,
    currentStep: null,
    stepCreases: [],
    faces: hasCrease
      ? [
          { id: 0, vertices: [0, 1, 2], edges: [0, 1, 4] },
          { id: 1, vertices: [0, 2, 3], edges: [4, 2, 3] },
        ]
      : [],
    hinges: hasCrease ? new Set([4]) : new Set(),
    frame3d: null,
    foldAllPreview: null,
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: "select",
    operationStage: 0,
    lineInputStart: null,
    construct: { ...DEFAULT_CONSTRUCT },
    curve: { ...DEFAULT_CURVE, enabled: false },
    mirrorDraw: false,
    contextHelpExpanded: false,
    cpHelpExpanded: false,
    paperHelpExpanded: false,
    paperColorExpanded: false,
    viewerHintExpanded: false,
    violations: [],
    flatFoldViolations: [],
    warnings: [],
    poseWarnings: [],
    replayWarnings: [],
    suspectHinges: [],
    activeAngleIntent: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    alignDraft: null,
    foldDraft: null,
    techniqueDraft: null,
    drivers: new Map(),
    pinnedFolds: new Map(),
    releasedPins: [],
    sequenceTargets: new Map(),
    poseAngles: new Map(),
    relaxations: [],
    playing: false,
    playT: 1,
    errorMessage: null,
    documentSavedPath: null,
    angleUndoStack: [],
    angleRedoStack: [],
    docUndoDepth: 0,
  });
}

interface InputAudit {
  lowLevelEvents: string[];
  clicks: Array<{ detail: number; pointerType: string }>;
  stop: () => void;
}

/** clickはEnter/Spaceでも発生するため、detail=0だけをキーボード由来として別計数する。 */
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
  const lowLevel = (event: Event) => lowLevelEvents.push(event.type);
  const click = (event: Event) => {
    const input = event as MouseEvent & { pointerType?: string };
    clicks.push({
      detail: input.detail,
      pointerType: input.pointerType ?? "",
    });
  };
  for (const type of lowLevelTypes) {
    document.addEventListener(type, lowLevel, true);
  }
  document.addEventListener("click", click, true);
  return {
    lowLevelEvents,
    clicks,
    stop: () => {
      for (const type of lowLevelTypes) {
        document.removeEventListener(type, lowLevel, true);
      }
      document.removeEventListener("click", click, true);
    },
  };
}

function moveMany(
  canvas: HTMLCanvasElement,
  key: "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown",
  count: number,
): void {
  for (let index = 0; index < count; index += 1) {
    fireEvent.keyDown(canvas, { key, shiftKey: true });
  }
  fireEvent.keyUp(canvas, { key: "Shift" });
}

function isDisabled(element: HTMLElement): boolean {
  return "disabled" in element && Boolean((element as HTMLButtonElement).disabled);
}

function isDisplayedFocusTarget(element: HTMLElement): boolean {
  const hiddenOwner = element.closest<HTMLElement>(
    '[hidden], [aria-hidden="true"], [inert]',
  );
  const style = getComputedStyle(element);
  return (
    hiddenOwner === null &&
    !element.hidden &&
    style.display !== "none" &&
    style.visibility !== "hidden"
  );
}

const ACTIONABLE_SELECTOR = [
  "a[href]",
  "button",
  "canvas",
  "input:not([type='hidden'])",
  "select",
  "summary",
  "textarea",
  "[role='button']",
  "[role='slider']",
  "[tabindex]",
].join(",");

function actionableElements(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(ACTIONABLE_SELECTOR)].filter(
    (element, index, all) => {
      if (
        element.matches("[data-view-cube-target]") &&
        element.tabIndex < 0
      ) {
        const target = element.dataset.viewCubeTarget;
        const sameActionHasTabStop = all.some(
          (candidate) =>
            candidate !== element &&
            candidate.dataset.viewCubeTarget === target &&
            candidate.tabIndex >= 0,
        );
        if (sameActionHasTabStop) return false;
      }
      return (
        all.indexOf(element) === index &&
        !isDisabled(element) &&
        isDisplayedFocusTarget(element)
      );
    },
  );
}

function tabStops(root: HTMLElement): HTMLElement[] {
  const candidates = actionableElements(root).filter(
    (element) => element.tabIndex >= 0,
  );
  const positive = candidates
    .filter((element) => element.tabIndex > 0)
    .sort((a, b) => a.tabIndex - b.tabIndex);
  return [...positive, ...candidates.filter((element) => element.tabIndex === 0)];
}

/** jsdomが省くTabのブラウザ既定動作だけを補い、keydown/keyup自体はDOMへ送る。 */
function pressTab(root: HTMLElement, shiftKey = false): HTMLElement {
  const stops = tabStops(root);
  if (stops.length === 0) throw new Error("Tabで辿る対象がありません");
  const active = document.activeElement as HTMLElement | null;
  const keyTarget = active ?? document.body;
  fireEvent.keyDown(keyTarget, { key: "Tab", shiftKey });
  const index = active === null ? -1 : stops.indexOf(active);
  const nextIndex = shiftKey
    ? index <= 0
      ? stops.length - 1
      : index - 1
    : index < 0 || index === stops.length - 1
      ? 0
      : index + 1;
  const next = stops[nextIndex];
  next.focus();
  fireEvent.keyUp(next, { key: "Tab", shiftKey });
  return next;
}

function tabTo(root: HTMLElement, target: HTMLElement): void {
  const limit = tabStops(root).length + 1;
  for (let index = 0; index < limit; index += 1) {
    if (document.activeElement === target) return;
    pressTab(root);
  }
  throw new Error(`Tabで対象へ到達できません: ${visibleName(target)}`);
}

/** jsdomに無いbuttonのEnter既定clickだけを補う。低レベルpointer/mouseは発生しない。 */
function pressEnterOnButton(button: HTMLButtonElement): void {
  button.focus();
  fireEvent.keyDown(button, { key: "Enter" });
  button.click();
  fireEvent.keyUp(button, { key: "Enter" });
}

/** native rangeのキー既定値変更をchangeで補い、製品のonChange/onKeyUpを通す。 */
function pressRangeEnd(slider: HTMLInputElement): void {
  slider.focus();
  // jsdomにはrangeの既定値変更が無い。実ブラウザでkeydownとchangeの間に起きる
  // 値変更をReactへ伝え、その同じキーの完了をkeyupとして送る。
  fireEvent.change(slider, { target: { value: slider.max } });
  expect(useAppStore.getState().foldAllPreview?.percent).toBe(
    Number(slider.max),
  );
  fireEvent.keyUp(slider, { key: "End" });
}

function visibleName(element: HTMLElement): string {
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
  return [
    element.getAttribute("aria-label"),
    labelledText,
    explicitLabel,
    ownerLabel,
    element.getAttribute("data-tooltip"),
    element.getAttribute("title"),
    element.textContent,
  ]
    .find((value): value is string =>
      typeof value === "string" && value.trim() !== "",
    )
    ?.trim() ?? "";
}

function focusFromBody(): void {
  document.body.tabIndex = -1;
  document.body.focus();
  expect(document.activeElement).toBe(document.body);
}

function expectFocusVisibleContract(): void {
  const globalRule = /:focus-visible\s*\{([\s\S]*?)\}/u.exec(baseLayoutCss)?.[1];
  expect(globalRule).toBeDefined();
  expect(globalRule).toContain("outline: 2px solid var(--color-accent)");
  expect(globalRule).toContain("outline-offset: 2px");
  expect(globalRule).toContain("box-shadow: var(--focus-ring)");
  const canvasRule = /\.cp-canvas:focus-visible\s*\{([\s\S]*?)\}/u.exec(
    viewerCss,
  )?.[1];
  expect(canvasRule).toBeDefined();
  expect(canvasRule).toContain("outline-offset: -4px");
  expect(canvasRule).not.toContain("outline: none");
}

function expectAllActionsAreTabStops(root: HTMLElement): void {
  const actions = actionableElements(root);
  expect(actions.length).toBeGreaterThan(0);
  for (const action of actions) {
    expect(
      action.tabIndex,
      `${visibleName(action)}のtabIndex`,
    ).toBeGreaterThanOrEqual(0);
    expect(visibleName(action), `${action.tagName}の画面上の名前`).not.toBe("");
  }
}

function expectEveryTabStopReachable(root: HTMLElement): void {
  const expected = tabStops(root);
  focusFromBody();
  const forward = new Set<HTMLElement>();
  for (const target of expected) {
    const focused = pressTab(root);
    expect(focused).toBe(target);
    expect(document.activeElement).toBe(target);
    expect(target.matches(":focus")).toBe(true);
    expect(isDisplayedFocusTarget(target)).toBe(true);
    forward.add(target);
  }
  expect(forward.size).toBe(expected.length);

  focusFromBody();
  const backward = new Set<HTMLElement>();
  for (const target of [...expected].reverse()) {
    const focused = pressTab(root, true);
    expect(focused).toBe(target);
    expect(document.activeElement).toBe(target);
    expect(isDisplayedFocusTarget(target)).toBe(true);
    backward.add(target);
  }
  expect(backward.size).toBe(expected.length);
}

beforeEach(() => {
  held.overlay = null;
  vi.clearAllMocks();
  resetPoseThrottle();
  resetFoldAllPreviewRuntime();
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
  seed();
  vi.mocked(ipc.foldAllPreview).mockImplementation(async (percent) =>
    foldOutcome(percent),
  );
  vi.mocked(ipc.poseSolve).mockResolvedValue({
    frame: frameAt(0),
    converged: true,
    angles: { "4": 0 },
    iterations: 1,
  });
});

afterEach(() => {
  cleanup();
  resetPoseThrottle();
  resetFoldAllPreviewRuntime();
  document.body.removeAttribute("tabindex");
  useAppStore.setState(initialStoreState, true);
});

function arrangeCreaseResponse(): void {
  const withCrease = documentWithCrease();
  vi.mocked(ipc.editApply).mockImplementation(async (op: EditOp) => {
    if (op.type !== "AddSegment") {
      throw new Error("線を引く以外の編集が送られました");
    }
    return viewOf(withCrease);
  });
}

async function drawDiagonalWithKeys(root: HTMLElement): Promise<void> {
  const canvas = root.querySelector("canvas.cp-canvas");
  if (!(canvas instanceof HTMLCanvasElement)) {
    throw new Error("展開図がありません");
  }
  const mountain = screen.getByRole("button", { name: "山" }) as HTMLButtonElement;
  expect(useAppStore.getState().activeTool).toBe("select");
  expect(mountain.classList.contains("active")).toBe(false);

  focusFromBody();
  tabTo(root, mountain);
  expect(document.activeElement).toBe(mountain);
  pressEnterOnButton(mountain);
  expect(useAppStore.getState().activeTool).toBe("mountain");
  await waitFor(() =>
    expect(mountain.classList.contains("active")).toBe(true),
  );
  await waitFor(() =>
    expect(canvas.getAttribute("aria-label")).toContain("Enterを2回"),
  );

  tabTo(root, canvas);
  await waitFor(() => expect(overlay().keyboardCursor).toEqual([0.5, 0.5]));
  expect(document.activeElement).toBe(canvas);
  moveMany(canvas, "ArrowLeft", 4);
  moveMany(canvas, "ArrowDown", 4);
  await waitFor(() => expect(overlay().keyboardCursor).toEqual([0, 0]));

  fireEvent.keyDown(canvas, { key: "Enter" });
  expect(useAppStore.getState().lineInputStart).toEqual([0, 0]);
  expect(useAppStore.getState().operationStage).toBe(1);
  expect(overlay().preview).not.toBeNull();
  expect(overlay().hint).toContain("終わりの位置");

  moveMany(canvas, "ArrowRight", 4);
  moveMany(canvas, "ArrowUp", 4);
  fireEvent.keyDown(canvas, { key: "Enter" });

  await waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));
  const sent = vi.mocked(ipc.editApply).mock.calls[0][0];
  expect(sent.type).toBe("AddSegment");
  if (sent.type !== "AddSegment") {
    throw new Error("線を引く以外の編集が送られました");
  }
  expect(sent.a[0]).toBeCloseTo(0, 12);
  expect(sent.a[1]).toBeCloseTo(0, 12);
  expect(sent.b[0]).toBeCloseTo(1, 12);
  expect(sent.b[1]).toBeCloseTo(1, 12);
  expect(sent.kind).toBe("Mountain");
  await waitFor(() => {
    expect(useAppStore.getState().doc?.cp.edges).toHaveLength(5);
    expect(useAppStore.getState().hinges).toEqual(new Set([4]));
  });
}

describe("施策9: マウスを使わない一続きの操作", () => {
  it("初期状態からTab・Enter・矢印だけで山折り線を1本追加し、pointer/mouse入力は0件", async () => {
    arrangeCreaseResponse();
    const input = auditInput();
    try {
      const view = render(<KeyboardWorkspace />);
      await drawDiagonalWithKeys(view.getByTestId("keyboard-workspace"));
      expect(input.lowLevelEvents).toEqual([]);
      expect(input.clicks).toEqual([{ detail: 0, pointerType: "" }]);
    } finally {
      input.stop();
    }
  });

  it("線を1本足し、仮の形を100%まで折り、Ctrl+Zでいつもの形へ戻す", async () => {
    arrangeCreaseResponse();
    const input = auditInput();
    const focusSnapshots: Array<{ hidden: boolean; name: string }> = [];
    const onFocus = (event: FocusEvent) => {
      const target = event.target;
      if (!(target instanceof HTMLElement)) return;
      focusSnapshots.push({
        hidden: !isDisplayedFocusTarget(target),
        name: visibleName(target),
      });
    };
    document.addEventListener("focusin", onFocus, true);

    try {
      const view = render(<KeyboardWorkspace />);
      const root = view.getByTestId("keyboard-workspace");
      await drawDiagonalWithKeys(root);

      const foldAll = screen.getByRole("button", {
        name: /全部いっぺんに折ってみる/,
      }) as HTMLButtonElement;
      expect(foldAll.disabled).toBe(false);
      tabTo(root, foldAll);
      expect(document.activeElement).toBe(foldAll);
      await act(async () => pressEnterOnButton(foldAll));

      expect(await screen.findByText("これは仮の形です")).toBeTruthy();
      await waitFor(() =>
        expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(0),
      );
      const slider = screen.getByRole("slider", {
        name: "全部の折り目を動かす割合",
      }) as HTMLInputElement;
      tabTo(root, slider);
      expect(document.activeElement).toBe(slider);
      expect(
        useAppStore.getState().foldAllPreview?.returning,
        "Tabのkeyupだけで0%から復帰を始め、割合操作を閉じてはいけません",
      ).toBe(false);
      pressRangeEnd(slider);

      await waitFor(() => {
        expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(100);
        expect(useAppStore.getState().frame3d).toEqual(frameAt(100));
      });
      expect(slider.getAttribute("aria-valuetext")).toBe("100%");
      expect(screen.getByText("これは仮の形です")).toBeTruthy();

      const returnButton = screen.getByRole("button", {
        name: "いつもの表示に戻る",
      }) as HTMLButtonElement;
      tabTo(root, returnButton);
      const undoEvent = new KeyboardEvent("keydown", {
        key: "z",
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      });
      fireEvent(returnButton, undoEvent);
      expect(undoEvent.defaultPrevented).toBe(true);

      await waitFor(() => expect(useAppStore.getState().foldAllPreview).toBeNull());
      expect(useAppStore.getState().frame3d).toEqual(frameAt(0));
      expect(useAppStore.getState().doc?.cp.edges).toHaveLength(5);
      expect(ipc.editUndo).not.toHaveBeenCalled();
      expect(
        screen.getByRole("button", { name: /全部いっぺんに折ってみる/ }),
      ).toBeTruthy();

      expect(input.lowLevelEvents).toEqual([]);
      expect(input.clicks).toHaveLength(2);
      expect(input.clicks).toEqual([
        { detail: 0, pointerType: "" },
        { detail: 0, pointerType: "" },
      ]);
      expect(focusSnapshots.length).toBeGreaterThan(0);
      expect(focusSnapshots.every((snapshot) => !snapshot.hidden)).toBe(true);
      expect(focusSnapshots.every((snapshot) => snapshot.name !== "")).toBe(true);
    } finally {
      document.removeEventListener("focusin", onFocus, true);
      input.stop();
      // 後段の受入条件が失敗しても、そこへ至る入力がマウス由来でないことは
      // 独立して機械計数する。keyboard activationのclickはdetail=0だけを許す。
      expect(input.lowLevelEvents).toEqual([]);
      expect(
        input.clicks.every(
          (click) => click.detail === 0 && click.pointerType === "",
        ),
      ).toBe(true);
      expect(focusSnapshots.every((snapshot) => !snapshot.hidden)).toBe(true);
      expect(focusSnapshots.every((snapshot) => snapshot.name !== "")).toBe(true);
    }
  });

  it("検査用作業画面の現在のTab対象をTabとShift+Tabで往復できる", async () => {
    seed(documentWithCrease());
    const view = render(<KeyboardWorkspace />);
    const root = view.getByTestId("keyboard-workspace");

    expectFocusVisibleContract();
    expect(screen.getAllByRole("separator")).toHaveLength(2);
    expectEveryTabStopReachable(root);
    const canvas = root.querySelector("canvas.cp-canvas");
    if (!(canvas instanceof HTMLCanvasElement)) {
      throw new Error("展開図がありません");
    }
    const mountain = screen.getByRole("button", { name: "山" }) as HTMLButtonElement;
    focusFromBody();
    tabTo(root, mountain);
    pressEnterOnButton(mountain);
    tabTo(root, canvas);
    await waitFor(() => expect(overlay().keyboardCursor).toEqual([0.5, 0.5]));
    pressTab(root);
    await waitFor(() => expect(overlay().keyboardCursor).toBeNull());
    expect(document.activeElement).not.toBe(canvas);

    const foldAll = screen.getByRole("button", {
      name: /全部いっぺんに折ってみる/,
    }) as HTMLButtonElement;
    tabTo(root, foldAll);
    await act(async () => pressEnterOnButton(foldAll));
    expect(await screen.findByText("これは仮の形です")).toBeTruthy();
    await waitFor(() =>
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(0),
    );
    const slider = screen.getByRole("slider", {
      name: "全部の折り目を動かす割合",
    }) as HTMLInputElement;
    pressRangeEnd(slider);
    await waitFor(() =>
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(100),
    );
    expectEveryTabStopReachable(root);
  });

  it("pointerで操作できる3D表示もTab対象で、見える名前を持つ", () => {
    seed(documentWithCrease());
    const view = render(<KeyboardWorkspace />);
    const root = view.getByTestId("keyboard-workspace");
    const viewerCanvas = root.querySelector("canvas.viewer3d-canvas");
    if (!(viewerCanvas instanceof HTMLCanvasElement)) {
      throw new Error("3D表示がありません");
    }
    expect(viewerCanvas.getAttribute("data-tooltip")).not.toBeNull();
    expectAllActionsAreTabStops(root);
  });
});
