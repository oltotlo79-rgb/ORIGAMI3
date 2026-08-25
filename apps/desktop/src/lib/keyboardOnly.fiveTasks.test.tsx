// @vitest-environment jsdom
// 施策9-B1: 利用者向けの固定5課題を、実DOMと実ストアactionで入口から完了まで通す。
// jsdomに無いブラウザー既定動作だけを補い、IPCとcanvas描画だけをleaf mockにする。

import { createRef } from "react";
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

const rendered = vi.hoisted(() => ({
  overlay: null as unknown,
  selection: null as unknown,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  save: vi.fn(),
}));

vi.mock("../components/CpEditor/renderer", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../components/CpEditor/renderer")>();
  return {
    ...actual,
    render: vi.fn((...args: unknown[]) => {
      rendered.selection = args[6];
      rendered.overlay = args[7];
    }),
  };
});

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

import { save as chooseSavePath } from "@tauri-apps/plugin-dialog";
import { AppToolbar } from "../components/AppToolbar";
import { ContextPanel } from "../components/ContextPanel";
import { CpEditor } from "../components/CpEditor/CpEditor";
import type { RenderOverlay } from "../components/CpEditor/renderer";
import { ExportDialog } from "../components/dialogs/ExportDialog";
import {
  focusableElements,
  type FocusTarget,
} from "../components/dialogs/ModalDialog";
import { NewDocumentDialog } from "../components/dialogs/NewDocumentDialog";
import { Timeline } from "../components/Timeline";
import { ToolRail } from "../components/ToolRail";
import * as ipc from "../ipc/client";
import { DEFAULT_CONSTRUCT } from "./construct";
import { DEFAULT_CURVE } from "./curve";
import { STEP_DURATION_MS } from "./playback";
import type {
  Document,
  DocumentView,
  EditOp,
  EdgeKind,
  FoldAllPreviewOutcome,
  FoldStep,
  Frame3D,
  ReplayResult,
} from "./types";
import {
  DEFAULT_NEW_PAPER,
  resetFoldAllPreviewRuntime,
  resetPoseThrottle,
  useAppStore,
} from "../store/appStore";

const initialStoreState = useAppStore.getState();

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

function withCreases(...kinds: EdgeKind[]): Document {
  const doc = structuredClone(BASE_DOCUMENT);
  for (const [index, kind] of kinds.entries()) {
    const id = 4 + index;
    doc.cp.edges.push({
      id,
      v0: index % 2 === 0 ? 0 : 3,
      v1: index % 2 === 0 ? 2 : 1,
      kind,
    });
  }
  doc.cp.next_edge_id = 4 + kinds.length;
  return doc;
}

/** 400px表示で中心から10px離れ、6pxでは届かず既存12px範囲なら届く折り線。 */
function withOffsetCrease(): Document {
  const doc = structuredClone(BASE_DOCUMENT);
  doc.cp.vertices.push(
    { id: 4, pos: [0, 0.525] },
    { id: 5, pos: [1, 0.525] },
  );
  doc.cp.edges.push({ id: 4, v0: 4, v1: 5, kind: "Mountain" });
  doc.cp.next_vertex_id = 6;
  doc.cp.next_edge_id = 5;
  return doc;
}

function facesFor(doc: Document): DocumentView["faces"] {
  const creaseIds = doc.cp.edges
    .filter((edge) => edge.kind === "Mountain" || edge.kind === "Valley")
    .map((edge) => edge.id);
  if (creaseIds.length === 0) {
    return [{ id: 0, vertices: [0, 1, 2, 3], edges: [0, 1, 2, 3] }];
  }
  // IPC/WebGLのleaf境界。各折り線を2面が共有する、製品と同じhinge契約を返す。
  return [
    { id: 0, vertices: [0, 1, 2], edges: [0, 1, ...creaseIds] },
    { id: 1, vertices: [0, 2, 3], edges: [2, 3, ...creaseIds] },
  ];
}

function viewOf(doc: Document, frame: Frame3D | null = null): DocumentView {
  return {
    doc: structuredClone(doc),
    faces: facesFor(doc),
    warnings: [],
    violations: [],
    frame,
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

function replayResult(): ReplayResult {
  return { frame: frameAt(100), skipped: [], warnings: [] };
}

function recordedStep(id = 1): FoldStep {
  return {
    id,
    kind: "Pose",
    drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: 90 }],
    layer_order: null,
    note: "",
  };
}

function seed(doc: Document = BASE_DOCUMENT): void {
  const creaseIds = doc.cp.edges
    .filter((edge) => edge.kind === "Mountain" || edge.kind === "Valley")
    .map((edge) => edge.id);
  useAppStore.setState({
    doc: structuredClone(doc),
    docEpoch: 1,
    currentStep: null,
    stepCreases: [],
    faces: facesFor(doc),
    hinges: new Set(creaseIds),
    frame3d: null,
    foldAllPreview: null,
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: "select",
    operationStage: 0,
    lineInputStart: null,
    construct: { ...DEFAULT_CONSTRUCT },
    curve: { ...DEFAULT_CURVE, enabled: false },
    mirrorDraw: false,
    newDialogOpen: false,
    newPaperDraft: DEFAULT_NEW_PAPER,
    exportOpen: false,
    exportKind: "CpSvg",
    exportIncludeAux: true,
    exportLongSide: 2048,
    exportBusy: false,
    exportError: null,
    exportSavedPath: null,
    drivers: new Map(),
    pinnedFolds: new Map(),
    releasedPins: [],
    sequenceTargets: new Map(),
    poseAngles: new Map(),
    poseWarnings: [],
    relaxations: [],
    playing: false,
    playT: 1,
    errorMessage: null,
    documentSavedPath: null,
    angleUndoStack: [],
    angleRedoStack: [],
    docUndoDepth: 0,
    activeAngleIntent: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    alignDraft: null,
    foldDraft: null,
    techniqueDraft: null,
  });
}

function overlay(): RenderOverlay {
  if (rendered.overlay === null) {
    throw new Error("展開図の表示状態がまだ作られていません");
  }
  return rendered.overlay as RenderOverlay;
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
  return (
    [
      element.getAttribute("aria-label"),
      labelledText,
      explicitLabel,
      ownerLabel,
      element.getAttribute("data-tooltip"),
      element.textContent,
    ].find(
      (value): value is string =>
        typeof value === "string" && value.trim() !== "",
    )?.trim() ?? ""
  );
}

function isDisplayed(element: HTMLElement): boolean {
  const style = getComputedStyle(element);
  return (
    element.closest('[hidden], [aria-hidden="true"]') === null &&
    !element.hidden &&
    style.display !== "none" &&
    style.visibility !== "hidden"
  );
}

interface InputAudit {
  lowLevelEvents: string[];
  clicks: Array<{ detail: number; pointerType: string }>;
  focusProblems: string[];
  focusedNames: string[];
  stop: () => void;
}

function auditKeyboardPath(): InputAudit {
  const lowLevelEvents: string[] = [];
  const clicks: Array<{ detail: number; pointerType: string }> = [];
  const focusProblems: string[] = [];
  const focusedNames: string[] = [];
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
  const focus = (event: Event) => {
    const target = event.target;
    if (!(target instanceof HTMLElement) || target === document.body) return;
    const name = visibleName(target);
    focusedNames.push(name);
    if (!isDisplayed(target)) focusProblems.push(`見えない選択: ${name}`);
    if (name === "") focusProblems.push(`名前の無い選択: ${target.tagName}`);
  };
  for (const type of lowLevelTypes) {
    document.addEventListener(type, lowLevel, true);
  }
  document.addEventListener("click", click, true);
  document.addEventListener("focusin", focus, true);
  return {
    lowLevelEvents,
    clicks,
    focusProblems,
    focusedNames,
    stop: () => {
      for (const type of lowLevelTypes) {
        document.removeEventListener(type, lowLevel, true);
      }
      document.removeEventListener("click", click, true);
      document.removeEventListener("focusin", focus, true);
    },
  };
}

function focusFromBody(): void {
  document.body.tabIndex = -1;
  document.body.focus();
  expect(document.activeElement).toBe(document.body);
}

/** jsdomが省くTabの既定移動だけを、製品のmodal trapを尊重して補う。 */
function pressTab(shiftKey = false): FocusTarget {
  const before = document.activeElement ?? document.body;
  const stopsBefore = focusableElements(document.body);
  const event = new KeyboardEvent("keydown", {
    key: "Tab",
    shiftKey,
    bubbles: true,
    cancelable: true,
  });
  fireEvent(before, event);
  if (!event.defaultPrevented) {
    const index = stopsBefore.indexOf(before as FocusTarget);
    const nextIndex = shiftKey
      ? index <= 0
        ? stopsBefore.length - 1
        : index - 1
      : index < 0 || index === stopsBefore.length - 1
        ? 0
        : index + 1;
    stopsBefore[nextIndex]?.focus({ preventScroll: true });
  }
  const focused = document.activeElement as FocusTarget | null;
  if (focused === null || focused === document.body) {
    throw new Error("Tabで選べる場所へ移れませんでした");
  }
  fireEvent.keyUp(focused, { key: "Tab", shiftKey });
  return focused;
}

function tabTo(target: HTMLElement): void {
  const limit = focusableElements(document.body).length + 2;
  for (let index = 0; index < limit; index += 1) {
    if (document.activeElement === target) return;
    pressTab();
  }
  throw new Error(`Tabで対象へ到達できません: ${visibleName(target)}`);
}

/** buttonのEnter既定clickだけを補う。pointer/mouseイベントは作らない。 */
function pressEnter(button: HTMLButtonElement): void {
  expect(document.activeElement).toBe(button);
  const down = new KeyboardEvent("keydown", {
    key: "Enter",
    bubbles: true,
    cancelable: true,
  });
  fireEvent(button, down);
  if (!down.defaultPrevented) button.click();
  fireEvent.keyUp(button, { key: "Enter" });
}

/** radioの矢印キー既定動作(次を選択してfocus移動)だけを補う。 */
function pressRadioNext(
  current: HTMLInputElement,
  next: HTMLInputElement,
): void {
  expect(document.activeElement).toBe(current);
  expect(next.disabled).toBe(false);
  const down = new KeyboardEvent("keydown", {
    key: "ArrowDown",
    bubbles: true,
    cancelable: true,
  });
  fireEvent(current, down);
  if (!down.defaultPrevented) {
    next.focus({ preventScroll: true });
    next.click();
  }
  fireEvent.keyUp(next, { key: "ArrowDown" });
}

function pressCanvasEnter(canvas: HTMLCanvasElement): void {
  fireEvent.keyDown(canvas, { key: "Enter" });
  fireEvent.keyUp(canvas, { key: "Enter" });
}

function moveCursorTo(
  canvas: HTMLCanvasElement,
  target: readonly [number, number],
): void {
  const current = overlay().keyboardCursor;
  if (current == null) throw new Error("キーボードの現在位置がありません");
  const horizontal = Math.round((target[0] - current[0]) * 8);
  const vertical = Math.round((target[1] - current[1]) * 8);
  const move = (
    key: "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown",
    count: number,
  ) => {
    for (let index = 0; index < count; index += 1) {
      fireEvent.keyDown(canvas, { key, shiftKey: true });
    }
    if (count > 0) fireEvent.keyUp(canvas, { key, shiftKey: true });
  };
  move(horizontal < 0 ? "ArrowLeft" : "ArrowRight", Math.abs(horizontal));
  move(vertical < 0 ? "ArrowDown" : "ArrowUp", Math.abs(vertical));
}

async function drawLine(
  canvas: HTMLCanvasElement,
  start: readonly [number, number],
  end: readonly [number, number],
): Promise<void> {
  tabTo(canvas);
  await waitFor(() => {
    expect(overlay().keyboardCursor).toBeDefined();
    expect(overlay().keyboardCursor).not.toBeNull();
  });
  moveCursorTo(canvas, start);
  await waitFor(() => {
    expect(overlay().keyboardCursor?.[0]).toBeCloseTo(start[0], 12);
    expect(overlay().keyboardCursor?.[1]).toBeCloseTo(start[1], 12);
  });
  pressCanvasEnter(canvas);
  expect(useAppStore.getState().lineInputStart).toEqual(start);
  moveCursorTo(canvas, end);
  pressCanvasEnter(canvas);
}

function expectSettledFocus(target: HTMLElement): void {
  expect(document.activeElement).toBe(target);
  expect(target.isConnected).toBe(true);
  expect(isDisplayed(target)).toBe(true);
  expect(visibleName(target)).not.toBe("");
}

async function newPaperPath(): Promise<void> {
  seed();
  vi.mocked(ipc.documentNew).mockImplementation(async (paper) => {
    const doc = structuredClone(BASE_DOCUMENT);
    doc.paper = paper;
    return viewOf(doc);
  });
  render(
    <>
      <AppToolbar onOpenHelp={() => undefined} />
      <NewDocumentDialog />
    </>,
  );
  const trigger = screen.getByRole("button", { name: "新規" });
  focusFromBody();
  tabTo(trigger);
  pressEnter(trigger as HTMLButtonElement);
  await waitFor(() =>
    expect(document.activeElement).toBe(
      screen.getByRole("radio", { name: "正方形(たて・よこが同じ)" }),
    ),
  );
  const confirm = screen.getByRole("button", {
    name: "この紙で作りはじめる",
  }) as HTMLButtonElement;
  tabTo(confirm);
  pressEnter(confirm);
  await waitFor(() => expect(ipc.documentNew).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(trigger));
  expect(useAppStore.getState().doc?.paper).toEqual({
    width_mm: 150,
    height_mm: 150,
  });
  expectSettledFocus(trigger);
}

async function mountainValleyPath(): Promise<void> {
  seed();
  let nextDocument = structuredClone(BASE_DOCUMENT);
  vi.mocked(ipc.editApply).mockImplementation(async (operation: EditOp) => {
    if (operation.type !== "AddSegment") {
      throw new Error("線を足す以外の編集が送られました");
    }
    const id = nextDocument.cp.next_edge_id;
    const doc = structuredClone(nextDocument);
    doc.cp.edges.push({
      id,
      v0: operation.kind === "Mountain" ? 0 : 3,
      v1: operation.kind === "Mountain" ? 2 : 1,
      kind: operation.kind,
    });
    doc.cp.next_edge_id += 1;
    nextDocument = doc;
    return viewOf(doc);
  });
  const fitRef = createRef<(() => void) | null>();
  const view = render(
    <main>
      <ToolRail onFitView={() => undefined} />
      <CpEditor fitRef={fitRef} />
    </main>,
  );
  const canvas = view.container.querySelector("canvas.cp-canvas");
  if (!(canvas instanceof HTMLCanvasElement)) {
    throw new Error("展開図がありません");
  }

  focusFromBody();
  const mountain = screen.getByRole("button", { name: "山" });
  tabTo(mountain);
  pressEnter(mountain as HTMLButtonElement);
  expect(useAppStore.getState().activeTool).toBe("mountain");
  await waitFor(() => expect(mountain.classList.contains("active")).toBe(true));
  await act(async () => Promise.resolve());
  await drawLine(canvas, [0, 0], [1, 1]);
  await waitFor(() => expect(useAppStore.getState().doc?.cp.edges).toHaveLength(5));

  const valley = screen.getByRole("button", { name: "谷" });
  tabTo(valley);
  pressEnter(valley as HTMLButtonElement);
  expect(useAppStore.getState().activeTool).toBe("valley");
  await waitFor(() => expect(valley.classList.contains("active")).toBe(true));
  await act(async () => Promise.resolve());
  await drawLine(canvas, [0, 1], [1, 0]);
  await waitFor(() => expect(useAppStore.getState().doc?.cp.edges).toHaveLength(6));

  expect(
    vi.mocked(ipc.editApply).mock.calls.map(([operation]) =>
      operation.type === "AddSegment" ? operation.kind : operation.type,
    ),
  ).toEqual(["Mountain", "Valley"]);
  expect(useAppStore.getState().hinges).toEqual(new Set([4, 5]));
  expectSettledFocus(canvas);
}

async function foldIn3dPath(): Promise<void> {
  seed(withCreases("Mountain"));
  vi.mocked(ipc.foldAllPreview).mockImplementation(async (percent) =>
    foldOutcome(percent),
  );
  render(<ContextPanel />);
  const trigger = screen.getByRole("button", {
    name: /全部いっぺんに折ってみる/u,
  });
  focusFromBody();
  tabTo(trigger);
  await act(async () => pressEnter(trigger as HTMLButtonElement));
  const slider = await screen.findByRole("slider", {
    name: "全部の折り目を動かす割合",
  });
  tabTo(slider);
  fireEvent.keyDown(slider, { key: "End" });
  fireEvent.change(slider, { target: { value: "100" } });
  fireEvent.keyUp(slider, { key: "End" });
  await waitFor(() => {
    expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(100);
    expect(useAppStore.getState().frame3d).toEqual(frameAt(100));
  });
  expect(slider.getAttribute("aria-valuetext")).toBe("100%");
  expect(screen.getByText("これは仮の形です")).toBeTruthy();
  expectSettledFocus(slider);
}

async function recordAndReplayPath(): Promise<void> {
  const doc = withCreases("Mountain");
  seed(doc);
  useAppStore.setState({
    selection: { edgeIds: [4], vertexIds: [] },
    drivers: new Map([[4, 90]]),
    poseAngles: new Map([[4, 90]]),
    frame3d: frameAt(90),
  });
  vi.mocked(ipc.sequenceApply).mockImplementation(async (operation) => {
    if (operation.type !== "PushStep") {
      throw new Error("手順を記録する以外の変更が送られました");
    }
    const recorded = structuredClone(doc);
    recorded.sequence = [operation.step];
    return viewOf(recorded, frameAt(90));
  });
  vi.mocked(ipc.sequenceReplay).mockResolvedValue(replayResult());
  render(
    <main>
      <ContextPanel />
      <Timeline />
    </main>,
  );
  const record = screen.getByRole("button", {
    name: "この形で仕上げる",
  }) as HTMLButtonElement;
  expect(record.disabled).toBe(false);
  focusFromBody();
  tabTo(record);
  pressEnter(record);
  await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(useAppStore.getState().doc?.sequence).toHaveLength(1));
  expect(useAppStore.getState().doc?.sequence[0].kind).toBe("Pose");

  const play = screen.getByRole("button", { name: "▶ 再生" }) as HTMLButtonElement;
  tabTo(play);
  vi.useFakeTimers();
  pressEnter(play);
  expect(useAppStore.getState().playing).toBe(true);
  expect(useAppStore.getState().currentStep).toBe(0);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(STEP_DURATION_MS * 3);
  });
  expect(useAppStore.getState().playing).toBe(false);
  expect(useAppStore.getState().currentStep).toBe(1);
  expect(ipc.sequenceReplay).toHaveBeenCalled();
  const replayCalls = vi.mocked(ipc.sequenceReplay).mock.calls;
  expect(replayCalls[replayCalls.length - 1]?.slice(0, 2)).toEqual([1, 1]);
  expectSettledFocus(play);
  vi.useRealTimers();
}

async function exportDiagramPdfPath(): Promise<void> {
  const doc = withCreases("Mountain");
  doc.sequence = [recordedStep()];
  seed(doc);
  vi.mocked(chooseSavePath).mockResolvedValue("C:/出力/鳥の基本形-折り図.pdf");
  vi.mocked(ipc.documentExport).mockResolvedValue([]);
  render(
    <>
      <AppToolbar onOpenHelp={() => undefined} />
      <ExportDialog />
    </>,
  );
  const trigger = screen.getByRole("button", { name: "書き出し" });
  focusFromBody();
  tabTo(trigger);
  pressEnter(trigger as HTMLButtonElement);
  const cpSvg = await screen.findByRole("radio", { name: "展開図(SVG)" });
  await waitFor(() => expect(document.activeElement).toBe(cpSvg));
  const cpPng = screen.getByRole("radio", { name: "展開図(PNG)" });
  pressRadioNext(cpSvg as HTMLInputElement, cpPng as HTMLInputElement);
  const diagramPdf = screen.getByRole("radio", { name: "折り図(PDF)" });
  pressRadioNext(cpPng as HTMLInputElement, diagramPdf as HTMLInputElement);
  expect((diagramPdf as HTMLInputElement).checked).toBe(true);

  const exportButton = screen.getByRole("button", {
    name: "保存先を選んで書き出す",
  }) as HTMLButtonElement;
  tabTo(exportButton);
  pressEnter(exportButton);
  await waitFor(() =>
    expect(ipc.documentExport).toHaveBeenCalledWith(
      "DiagramPdf",
      "C:/出力/鳥の基本形-折り図.pdf",
      { include_aux: true, png_long_side: 2048 },
    ),
  );
  expect(await screen.findByText(/保存しました/u)).toBeTruthy();

  const close = screen.getByRole("button", { name: "閉じる" });
  tabTo(close);
  pressEnter(close as HTMLButtonElement);
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(trigger));
  expectSettledFocus(trigger);
}

/**
 * `docs/usability/tasks.md` の5課題は、前の課題で作った作品を次へ渡す累積課題である。
 * 単独seedの5経路は部品の診断には使えるが、9-B1の合格根拠にはしない。
 *
 * 線を引いた後も選択道具と同じkeyboard cursorを使い、Enterをpointerと共通の
 * 一点選択へ渡す。選択後は同じ作品の永続する折り→記録→再生→PDFまで完走する。
 * この一続きだけを9-B1の合格根拠とし、途中のseedで後半を置き換えない。
 */
async function cumulativeFiveTasksPath(): Promise<void> {
  seed();
  vi.mocked(ipc.documentNew).mockImplementation(async (paper) => {
    const doc = structuredClone(BASE_DOCUMENT);
    doc.paper = paper;
    return viewOf(doc);
  });
  vi.mocked(ipc.editApply).mockImplementation(async (operation: EditOp) => {
    if (operation.type !== "AddSegment") {
      throw new Error("線を足す以外の編集が送られました");
    }
    const doc = withCreases(operation.kind);
    return viewOf(doc);
  });
  vi.mocked(ipc.poseSolve).mockImplementation(async (hard, preferred) => {
    const requested = [...hard, ...(preferred ?? [])].find(
      (driver) => driver.hinge === 4,
    )?.target_angle_deg ?? 0;
    return {
      frame: frameAt(Math.abs(requested) / 1.8),
      converged: true,
      angles: { "4": requested },
      iterations: 1,
    };
  });
  vi.mocked(ipc.sequenceApply).mockImplementation(async (operation) => {
    if (operation.type !== "PushStep") {
      throw new Error("手順を記録する以外の変更が送られました");
    }
    const current = useAppStore.getState().doc;
    if (current === null) throw new Error("記録する紙がありません");
    const recorded = structuredClone(current);
    recorded.sequence = [...recorded.sequence, operation.step];
    return viewOf(recorded, frameAt(100));
  });
  vi.mocked(ipc.sequenceReplay).mockResolvedValue(replayResult());
  vi.mocked(chooseSavePath).mockResolvedValue(
    "C:/出力/折り鶴-ここまでの折り方.pdf",
  );
  vi.mocked(ipc.documentExport).mockResolvedValue([]);

  const fitRef = createRef<(() => void) | null>();
  const view = render(
    <main data-testid="cumulative-five-tasks">
      <AppToolbar onOpenHelp={() => undefined} />
      <ToolRail onFitView={() => undefined} />
      <CpEditor fitRef={fitRef} />
      <ContextPanel />
      <Timeline />
      <NewDocumentDialog />
      <ExportDialog />
    </main>,
  );

  // 課題1: 実入口から正方形の紙を作る。
  const newTrigger = screen.getByRole("button", { name: "新規" });
  focusFromBody();
  tabTo(newTrigger);
  pressEnter(newTrigger as HTMLButtonElement);
  const confirm = await screen.findByRole("button", {
    name: "この紙で作りはじめる",
  });
  tabTo(confirm);
  pressEnter(confirm as HTMLButtonElement);
  await waitFor(() => expect(ipc.documentNew).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(useAppStore.getState().doc?.paper).toEqual({
    width_mm: 150,
    height_mm: 150,
  });

  // 課題2: 同じ作品へ山折り線を1本足す。
  const mountain = screen.getByRole("button", { name: "山" });
  tabTo(mountain);
  pressEnter(mountain as HTMLButtonElement);
  await waitFor(() => expect(mountain.classList.contains("active")).toBe(true));
  await act(async () => Promise.resolve());
  const canvas = view.container.querySelector("canvas.cp-canvas");
  if (!(canvas instanceof HTMLCanvasElement)) {
    throw new Error("展開図がありません");
  }
  await drawLine(canvas, [0, 0], [1, 1]);
  await waitFor(() => expect(useAppStore.getState().doc?.cp.edges).toHaveLength(5));
  expect(useAppStore.getState().doc?.cp.edges[4]?.kind).toBe("Mountain");

  // 課題3の入口: 既存cursor上のEnterを、pointerと共通の一点選択へ渡す。
  const select = screen.getByRole("button", { name: "選択" });
  tabTo(select);
  pressEnter(select as HTMLButtonElement);
  await waitFor(() => expect(select.classList.contains("active")).toBe(true));
  await act(async () => Promise.resolve());
  tabTo(canvas);
  pressCanvasEnter(canvas);
  expect(
    useAppStore.getState().selection.edgeIds,
    "累積課題3: Tabで展開図へ入り、Enterで引いた折り線を選ぶ回帰です。" +
      "失敗時はCpEditor.tsxのキー配送とinteraction.tsの共通選択を確認してください",
  ).toEqual([4]);

  // 課題3: 選択で現れた角度操作をEndまで動かし、同じ紙の3D形を折る。
  const angle = screen.getByRole("slider", {
    name: "折り目 #4の角度",
  }) as HTMLInputElement;
  tabTo(angle);
  fireEvent.keyDown(angle, { key: "End" });
  fireEvent.change(angle, { target: { value: angle.max } });
  fireEvent.keyUp(angle, { key: "End" });
  await waitFor(() => expect(useAppStore.getState().drivers.get(4)).toBe(180));
  await waitFor(() => expect(useAppStore.getState().frame3d).toEqual(frameAt(100)));

  // 課題4: 同じ形を手順へ残し、実Timelineを最初から最後まで再生する。
  const record = screen.getByRole("button", {
    name: "この形で仕上げる",
  }) as HTMLButtonElement;
  expect(record.disabled).toBe(false);
  tabTo(record);
  pressEnter(record);
  await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(useAppStore.getState().doc?.sequence).toHaveLength(1));
  const play = screen.getByRole("button", { name: "▶ 再生" }) as HTMLButtonElement;
  tabTo(play);
  vi.useFakeTimers();
  pressEnter(play);
  expect(useAppStore.getState().playing).toBe(true);
  expect(useAppStore.getState().currentStep).toBe(0);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(STEP_DURATION_MS * 3);
  });
  expect(useAppStore.getState().playing).toBe(false);
  expect(useAppStore.getState().currentStep).toBe(1);
  const replayCalls = vi.mocked(ipc.sequenceReplay).mock.calls;
  expect(replayCalls[replayCalls.length - 1]?.slice(0, 2)).toEqual([1, 1]);
  vi.useRealTimers();

  // 課題5: 手順を持つ同じ作品を、実書き出し画面からPDFへ渡す。
  const exportTrigger = screen.getByRole("button", { name: "書き出し" });
  tabTo(exportTrigger);
  pressEnter(exportTrigger as HTMLButtonElement);
  const cpSvg = await screen.findByRole("radio", { name: "展開図(SVG)" });
  await waitFor(() => expect(document.activeElement).toBe(cpSvg));
  const cpPng = screen.getByRole("radio", { name: "展開図(PNG)" });
  pressRadioNext(cpSvg as HTMLInputElement, cpPng as HTMLInputElement);
  const diagramPdf = screen.getByRole("radio", { name: "折り図(PDF)" });
  pressRadioNext(cpPng as HTMLInputElement, diagramPdf as HTMLInputElement);
  const exportButton = screen.getByRole("button", {
    name: "保存先を選んで書き出す",
  }) as HTMLButtonElement;
  tabTo(exportButton);
  pressEnter(exportButton);
  await waitFor(() =>
    expect(ipc.documentExport).toHaveBeenCalledWith(
      "DiagramPdf",
      "C:/出力/折り鶴-ここまでの折り方.pdf",
      { include_aux: true, png_long_side: 2048 },
    ),
  );
  expect(await screen.findByText(/保存しました/u)).toBeTruthy();
  const close = screen.getByRole("button", { name: "閉じる" });
  tabTo(close);
  pressEnter(close as HTMLButtonElement);
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(exportTrigger));
  expectSettledFocus(exportTrigger);
}

interface FiveTaskRow {
  id: "new-paper" | "crease-lines" | "fold-3d" | "record-replay" | "pdf";
  task: string;
  focusLifecycle: "modal-return" | "nonmodal-final-control";
  run: () => Promise<void>;
}

const FIVE_TASKS = [
  {
    id: "new-paper",
    task: "新しい紙を作る",
    focusLifecycle: "modal-return",
    run: newPaperPath,
  },
  {
    id: "crease-lines",
    task: "山折り線と谷折り線を追加する",
    focusLifecycle: "nonmodal-final-control",
    run: mountainValleyPath,
  },
  {
    id: "fold-3d",
    task: "手順とは別の3D仮表示で紙を100%まで折る",
    focusLifecycle: "nonmodal-final-control",
    run: foldIn3dPath,
  },
  {
    id: "record-replay",
    task: "手順を記録して最初から最後まで再生する",
    focusLifecycle: "nonmodal-final-control",
    run: recordAndReplayPath,
  },
  {
    id: "pdf",
    task: "折り図PDFを書き出す",
    focusLifecycle: "modal-return",
    run: exportDiagramPdfPath,
  },
] as const satisfies readonly FiveTaskRow[];

const CUMULATIVE_TASK_MANIFEST = [
  "新しい正方形の紙を作る",
  "同じ紙へ山折りまたは谷折りの線を1本引く",
  "引いた折り線を使って紙を1回折る",
  "ここまでの折り方を記録して最初から最後まで再生する",
  "ここまでの折り方を折り図PDFへ書き出す",
] as const;

beforeEach(() => {
  rendered.overlay = null;
  rendered.selection = null;
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
  vi.mocked(ipc.poseSolve).mockResolvedValue({
    frame: frameAt(0),
    converged: true,
    angles: { "4": 0 },
    iterations: 1,
  });
  seed();
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
  resetPoseThrottle();
  resetFoldAllPreviewRuntime();
  document.body.removeAttribute("tabindex");
  useAppStore.setState(initialStoreState, true);
});

describe("施策9-B1の補助証拠: 単独入口のkeyboard-only 5経路", () => {
  it.each(FIVE_TASKS)(
    "$id: $task ($focusLifecycle)",
    async ({ run }) => {
      const audit = auditKeyboardPath();
      try {
        await run();
      } finally {
        audit.stop();
        expect(audit.lowLevelEvents, "pointer/mouse/wheel/contextmenu入力").toEqual(
          [],
        );
        expect(
          audit.clicks.every(
            (click) => click.detail === 0 && click.pointerType === "",
          ),
          "clickはEnter/Space/矢印のブラウザー既定動作だけ",
        ).toBe(true);
        expect(audit.focusProblems, "見えない・名前の無いfocus").toEqual([]);
        expect(audit.focusedNames.length, "Tabで到達した操作").toBeGreaterThan(0);
      }
    },
  );
});

describe("施策9-B1の合格判定: 固定5課題を同じ作品で累積する", () => {
  it("課題1から5まで前の結果を引き継ぎ、pointer入力0で完了する", async () => {
    expect(CUMULATIVE_TASK_MANIFEST).toHaveLength(5);
    const audit = auditKeyboardPath();
    try {
      await cumulativeFiveTasksPath();
    } finally {
      audit.stop();
      expect(audit.lowLevelEvents, "pointer/mouse/wheel/contextmenu入力").toEqual(
        [],
      );
      expect(
        audit.clicks.every(
          (click) => click.detail === 0 && click.pointerType === "",
        ),
        "clickはEnter/Space/矢印のブラウザー既定動作だけ",
      ).toBe(true);
      expect(audit.focusProblems, "見えない・名前の無いfocus").toEqual([]);
    }
  });
});

describe("選択道具のキーボード経路", () => {
  it("16px刻みの隙間にある折り線も選べ、Ctrl+Enterで同じ選択を解除する", async () => {
    seed(withOffsetCrease());
    const audit = auditKeyboardPath();
    const fitRef = createRef<(() => void) | null>();
    const view = render(
      <main>
        <ToolRail onFitView={() => undefined} />
        <CpEditor fitRef={fitRef} />
      </main>,
    );
    try {
      const canvas = view.container.querySelector("canvas.cp-canvas");
      if (!(canvas instanceof HTMLCanvasElement)) {
        throw new Error("展開図がありません");
      }
      expect(canvas.getAttribute("aria-label")).toContain("Enterで折り線または点を選べます");

      focusFromBody();
      tabTo(canvas);
      await waitFor(() => expect(overlay().keyboardCursor).toEqual([0.5, 0.5]));
      pressCanvasEnter(canvas);
      await waitFor(() => expect(useAppStore.getState().selection.edgeIds).toEqual([4]));
      await waitFor(() =>
        expect(rendered.selection).toEqual({ edgeIds: [4], vertexIds: [] }),
      );

      fireEvent.keyDown(canvas, { key: "Enter", ctrlKey: true });
      fireEvent.keyUp(canvas, { key: "Enter", ctrlKey: true });
      await waitFor(() => expect(useAppStore.getState().selection.edgeIds).toEqual([]));
    } finally {
      audit.stop();
      expect(audit.lowLevelEvents, "pointer/mouse/wheel/contextmenu入力").toEqual([]);
      expect(audit.focusProblems, "見えない・名前の無いfocus").toEqual([]);
    }
  });
});
