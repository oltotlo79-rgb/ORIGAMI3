import type { RefObject } from "react";
import {
  captureViewer3DReadback,
  type Viewer3DReadback,
} from "./captureReadbackBridge";
import { stepDisplayLabel } from "./lib/techniques";
import { useAppStore, type ToolId } from "./store/appStore";

export type CaptureView = "3d" | "cp" | "both" | "normal";

export interface CaptureStepInfo {
  number: number;
  name: string;
}

export interface CaptureDocumentInfo {
  version: 1;
  stepCount: number;
  steps: CaptureStepInfo[];
}

export interface CaptureStatus {
  version: 1;
  ready: true;
  generation: string;
  heartbeat: number;
  url: string;
  title: string;
}

/**
 * 3D画面の入力途中状態を、描画資源を起動せず同期で読むための軽量snapshot。
 * `selectedLayerCount` は通常grabの実対象面数。`technique.selectedLayerCount`や完全折りの「ひだ数」とは別物。
 */
export interface Viewer3DInteractionCapture {
  readonly grab: {
    readonly active: boolean;
    readonly spatial: boolean | null;
    readonly face: number | null;
    readonly mode: "flap" | "all" | "single" | null;
    readonly selectedLayerCount: number;
  };
  readonly preview: {
    readonly visible: boolean;
    readonly polygonCount: number;
    readonly segmentCount: number;
  };
}

export type Viewer3DInteractionReader = () => Viewer3DInteractionCapture;

export const EMPTY_VIEWER3D_INTERACTION_CAPTURE: Viewer3DInteractionCapture =
  Object.freeze({
    grab: Object.freeze({
      active: false,
      spatial: null,
      face: null,
      mode: null,
      selectedLayerCount: 0,
    }),
    preview: Object.freeze({
      visible: false,
      polygonCount: 0,
      segmentCount: 0,
    }),
  });

let viewer3DInteractionReader: Viewer3DInteractionReader | null = null;

/** 遅れてmountしたViewerだけがreaderを登録する。古いcleanupは新しい登録を消さない。 */
export function registerViewer3DInteractionReader(
  reader: Viewer3DInteractionReader,
): () => void {
  viewer3DInteractionReader = reader;
  return () => {
    if (viewer3DInteractionReader === reader) viewer3DInteractionReader = null;
  };
}

/** reader未登録時は3Dを要求せず、呼ばれた時だけ現在値を読む。 */
export function captureViewer3DInteraction(): Viewer3DInteractionCapture {
  return viewer3DInteractionReader?.() ?? EMPTY_VIEWER3D_INTERACTION_CAPTURE;
}

/**
 * CDP受入検査用の読み取り専用状態。通常画面の描画・ストア状態は変更しない。
 * 重い3D readbackは含めず、必要な検査が呼んだときだけ現在のstoreを写す。
 */
export interface CaptureInteractionState {
  readonly version: 1;
  readonly stepCount: number;
  readonly currentStep: number | null;
  readonly activeTool: ToolId;
  readonly selectedEdgeCount: number;
  readonly selectedVertexCount: number;
  /** 作品そのものの要素数。CDPから作図の前後差だけを読むために使う。 */
  readonly document: {
    readonly vertexCount: number;
    readonly edgeCount: number;
  };
  /** 診断とnative dialogの状態を、内容・保存先を露出せず件数/有無だけで返す。 */
  readonly diagnosis: {
    readonly cpViolationCount: number;
    readonly flatFoldViolationCount: number;
    readonly warningCount: number;
    readonly recoveryVisible: boolean;
    readonly exportOpen: boolean;
    readonly exportCompleted: boolean;
  };
  /** 引く操作の対象。値の変更や操作開始は行わない。 */
  readonly pull: {
    readonly hinge: number | null;
    readonly mirrorHinge: number | null;
  };
  readonly fold: {
    readonly draftActive: boolean;
    readonly target: "all" | "top" | "topPleats" | null;
    readonly pendingConfirmation: boolean;
  };
  readonly technique: {
    readonly active: boolean;
    readonly kind: string | null;
    /** 技法draftの選択層数。同名の`viewer3d.grab.selectedLayerCount`や完全折りの「ひだ数」とは別物。 */
    readonly selectedLayerCount: number;
    readonly candidateLayerCount: number;
    readonly completedPartCount: number;
    readonly guideCreaseCount: number;
  };
  readonly viewer3d: Viewer3DInteractionCapture;
}

export interface CaptureAngleOperation {
  hinge: number;
  deg: number;
}

export interface CaptureCanonicalFace3D {
  face: number;
  polygon: [number, number, number][];
  layer: number;
  surfaceRank: number;
  mirrored: boolean;
}

export interface CaptureCanonical3D {
  readonly version: 1;
  readonly desired: readonly (readonly [number, number])[];
  readonly actual: readonly (readonly [number, number])[];
  readonly faces: readonly CaptureCanonicalFace3D[];
  readonly readback: Viewer3DReadback;
}

export interface Origami3CaptureApi {
  readonly version: 1;
  getStatus(): CaptureStatus;
  /** 呼び出し時点の入力途中状態を読むだけで返す。 */
  getInteractionState(): CaptureInteractionState;
  openDocument(path: string): Promise<CaptureDocumentInfo>;
  getDocumentInfo(): CaptureDocumentInfo;
  goToStep(step: number): Promise<CaptureStepInfo>;
  setView(view: CaptureView): Promise<void>;
  waitForStable(): Promise<void>;
  /** 通常の角度操作と同じ入口を、1操作ずつpointer-up相当まで通す。 */
  runAnglePath(operations: readonly CaptureAngleOperation[]): Promise<void>;
  /** productionの3段描画を直ちに通し、CPU状態と実GPU画素を同期して読む。 */
  captureCanonical3D(): CaptureCanonical3D;
}

declare global {
  interface Window {
    /** WebView2/CDPからだけ使う、解説画像の自動撮影口。 */
    __origami3Capture?: Origami3CaptureApi;
  }
}

interface FitRefs {
  fit2d: RefObject<(() => void) | null>;
  fit3d: RefObject<(() => void) | null>;
  /** 遅れて読む3D本体を要求し、実sceneの登録完了まで待つ。 */
  ensureViewer3d: () => Promise<void>;
}

const CAPTURE_VIEW_ATTRIBUTE = "data-origami3-capture-view";
let fallbackGenerationCounter = 0;

function createCaptureGeneration(): string {
  if (typeof globalThis.crypto?.randomUUID === "function") {
    return globalThis.crypto.randomUUID();
  }

  fallbackGenerationCounter += 1;
  const randomValues = new Uint32Array(4);
  globalThis.crypto?.getRandomValues?.(randomValues);
  const randomPart = Array.from(randomValues, (value) => value.toString(16)).join("-");
  return `${Date.now().toString(36)}-${fallbackGenerationCounter.toString(36)}-${randomPart}`;
}

function nextPaint(): Promise<void> {
  return new Promise((resolve) => {
    let settled = false;
    const fallback = window.setTimeout(() => {
      if (settled) return;
      settled = true;
      resolve();
    }, 100);
    window.requestAnimationFrame(() => {
      if (settled) return;
      settled = true;
      window.clearTimeout(fallback);
      resolve();
    });
  });
}

async function waitForStable(): Promise<void> {
  await document.fonts?.ready;
  // Zustand更新 -> React effect -> Three.js/canvas描画 -> compositor の順を待つ。
  await nextPaint();
  await nextPaint();
  await nextPaint();
}

function documentInfo(): CaptureDocumentInfo {
  const doc = useAppStore.getState().doc;
  if (!doc) throw new Error("作品が開かれていません");
  const steps: CaptureStepInfo[] = [
    { number: 0, name: "折る前" },
    ...doc.sequence.map((step, index) => ({
      number: index + 1,
      name: `${index + 1} ${stepDisplayLabel(step)}`,
    })),
  ];
  return { version: 1, stepCount: doc.sequence.length, steps };
}

function interactionState(): CaptureInteractionState {
  const state = useAppStore.getState();
  const technique = state.techniqueDraft;
  return {
    version: 1,
    stepCount: state.doc?.sequence.length ?? 0,
    currentStep: state.currentStep,
    activeTool: state.activeTool,
    selectedEdgeCount: state.selection.edgeIds.length,
    selectedVertexCount: state.selection.vertexIds.length,
    document: {
      vertexCount: state.doc?.cp.vertices.length ?? 0,
      edgeCount: state.doc?.cp.edges.length ?? 0,
    },
    diagnosis: {
      cpViolationCount: state.violations.length,
      flatFoldViolationCount: state.flatFoldViolations.length,
      warningCount: state.warnings.length,
      recoveryVisible: state.recovery !== null,
      exportOpen: state.exportOpen,
      exportCompleted: state.exportSavedPath !== null,
    },
    pull: {
      hinge: state.pullHinge,
      mirrorHinge: state.pullMirrorHinge,
    },
    fold: {
      draftActive: state.foldDraft !== null,
      target: state.foldDraft?.target ?? null,
      pendingConfirmation: state.pendingFoldThrough !== null,
    },
    technique: {
      active: technique !== null,
      kind: technique?.kind ?? null,
      // 技法draftの選択層数。同名のviewer3d grab実対象面数や完全折りの「ひだ数」とは別物。
      selectedLayerCount: technique?.flap.length ?? 0,
      candidateLayerCount: technique?.flapCandidates.length ?? 0,
      completedPartCount: technique?.motionParts.length ?? 0,
      guideCreaseCount: technique !== null && technique.line !== null ? 1 : 0,
    },
    viewer3d: captureViewer3DInteraction(),
  };
}

function canonicalPairs(values: ReadonlyMap<number, number>): [number, number][] {
  return [...values.entries()].sort((left, right) => left[0] - right[0]);
}

function canonical3D(): CaptureCanonical3D {
  const state = useAppStore.getState();
  if (state.frame3d === null) throw new Error("3D表示フレームがまだありません");
  const faces = state.frame3d.faces
    .map((face): CaptureCanonicalFace3D => ({
      face: face.face,
      polygon: face.polygon.map(([x, y, z]) => [x, y, z]),
      layer: face.layer,
      surfaceRank: face.surface_rank ?? 0,
      mirrored: face.mirrored === true,
    }))
    .sort((left, right) => left.face - right.face);
  return {
    version: 1,
    desired: canonicalPairs(state.drivers),
    actual: canonicalPairs(state.poseAngles),
    faces,
    readback: captureViewer3DReadback(),
  };
}

function setCaptureView(view: CaptureView): void {
  if (view === "normal") {
    document.documentElement.removeAttribute(CAPTURE_VIEW_ATTRIBUTE);
  } else {
    document.documentElement.setAttribute(CAPTURE_VIEW_ATTRIBUTE, view);
  }
}

/**
 * 通常UIには見た目も操作も足さず、CDPのRuntime.evaluateから呼ぶ口だけを公開する。
 * React StrictModeの再マウントでも、cleanupが自分のAPIだけを取り外す。
 */
export function installCaptureApi({
  fit2d,
  fit3d,
  ensureViewer3d,
}: FitRefs): () => void {
  const generation = createCaptureGeneration();
  let heartbeat = 0;
  const api: Origami3CaptureApi = {
    version: 1,

    getStatus() {
      heartbeat += 1;
      return {
        version: 1,
        ready: true,
        generation,
        heartbeat,
        url: window.location.href,
        title: document.title,
      };
    },

    getInteractionState: interactionState,

    async openDocument(path) {
      setCaptureView("normal");
      await useAppStore.getState().openDocument(path);
      const state = useAppStore.getState();
      if (state.errorMessage) throw new Error(state.errorMessage);
      await waitForStable();
      return documentInfo();
    },

    getDocumentInfo: documentInfo,

    async goToStep(step) {
      const info = documentInfo();
      if (!Number.isInteger(step) || step < 0 || step > info.stepCount) {
        throw new Error(`手順番号は0〜${info.stepCount}で指定してください: ${step}`);
      }
      await useAppStore.getState().selectStepForCapture(step);
      const state = useAppStore.getState();
      if (state.errorMessage) throw new Error(state.errorMessage);
      await waitForStable();
      return info.steps[step];
    },

    async setView(view) {
      setCaptureView(view);
      if (view === "3d" || view === "both" || view === "normal") {
        await ensureViewer3d();
      }
      await nextPaint();
      if (view === "3d" || view === "both" || view === "normal") {
        fit3d.current?.();
      }
      if (view === "cp" || view === "both" || view === "normal") {
        fit2d.current?.();
      }
      await waitForStable();
    },

    waitForStable,

    async runAnglePath(operations) {
      if (!Array.isArray(operations) || operations.length === 0) {
        throw new Error("通常角度操作を1件以上指定してください");
      }
      for (const operation of operations) {
        if (
          operation === null ||
          typeof operation !== "object" ||
          !Number.isSafeInteger(operation.hinge) ||
          !Number.isFinite(operation.deg)
        ) {
          throw new Error(`通常角度操作が不正です: ${JSON.stringify(operation)}`);
        }
        const before = useAppStore.getState();
        if (!before.hinges.has(operation.hinge)) {
          throw new Error(`折り角度を指定できない辺です: ${operation.hinge}`);
        }
        before.setDriverAngle(operation.hinge, operation.deg);
        await useAppStore.getState().finishAngleIntent();
        const after = useAppStore.getState();
        if (after.errorMessage !== null) throw new Error(after.errorMessage);
      }
      await waitForStable();
    },

    captureCanonical3D: canonical3D,
  };

  window.__origami3Capture = api;
  return () => {
    if (window.__origami3Capture === api) delete window.__origami3Capture;
    setCaptureView("normal");
  };
}
