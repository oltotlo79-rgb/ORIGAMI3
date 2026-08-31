import type { StoreApi } from "zustand";
import { hingeEdgeIds } from "../../lib/hinges";
import {
  DEFAULT_MIRROR_AXIS,
  rebindSelectedMirrorAxis,
  type MirrorAxisChoice,
} from "../../lib/mirror";
import type {
  AngleRelaxation,
  Document,
  DisplaySettings,
  DocumentView,
  Face,
  FoldIssue,
  Frame3D,
  SelfIntersectionPair,
  SoftMesh,
  StepCreases,
} from "../../lib/types";
import type { ReleasedPin } from "../../lib/settledFolds";
import {
  EMPTY_SELECTION,
  emptyMeasureDraft,
  type AlignDraft,
  type FoldDraft,
  type MeasureDraft,
  type PendingFoldThrough,
  type Selection,
  type TechniqueDraft,
} from "../slices/documentSlice";
import type { ToolId } from "../toolTypes";
import { createSerialQueue, type SerialQueue } from "../ipcQueue";

/** 選んでいた基準線が編集で消えたときの非阻害案内。 */
export const MIRROR_AXIS_REMOVED_NOTICE =
  "基準にしていた線が無くなったので、紙の縦の中心線に戻しました";

/** 同じ配列なら参照を保ち、Zustand購読の不要な更新を避ける。 */
export function keepIfSame<T>(previous: T[], next: T[]): T[] {
  return previous.length === next.length &&
    previous.every((item, index) => item === next[index])
    ? previous
    : next;
}

/** ドキュメント更新後、存在しなくなったIDを選択から取り除く。 */
function pruneSelection(selection: Selection, doc: Document): Selection {
  const edgeIds = new Set(doc.cp.edges.map((edge) => edge.id));
  const vertexIds = new Set(doc.cp.vertices.map((vertex) => vertex.id));
  return {
    edgeIds: selection.edgeIds.filter((id) => edgeIds.has(id)),
    vertexIds: selection.vertexIds.filter((id) => vertexIds.has(id)),
  };
}

/** command serviceが一括反映する、store全体の構造契約。 */
interface CommandHostState {
  doc: Document | null;
  stepCreases: StepCreases[];
  faces: Face[];
  hinges: ReadonlySet<number>;
  warnings: string[];
  foldIssues: FoldIssue[];
  flatFoldViolations: number[];
  violations: number[];
  selection: Selection;
  hoveredHinge: number | null;
  activeTool: ToolId;
  measureDraft: MeasureDraft;
  foldDraft: FoldDraft | null;
  pendingFoldThrough: PendingFoldThrough | null;
  foldThroughBusy: boolean;
  alignDraft: AlignDraft | null;
  techniqueDraft: TechniqueDraft | null;
  display: DisplaySettings;
  operationStage: number;
  lineInputStart: [number, number] | null;
  paperActionTipVisible: boolean;
  paperActionTipExpanded: boolean;
  suspectHinges: number[];
  sequenceTargets: Map<number, number>;
  relaxations: AngleRelaxation[];
  currentStep: number | null;
  skipped: number[];
  errorMessage: string | null;
  documentSavedPath: string | null;
  docEpoch: number;
  drivers: Map<number, number>;
  pinnedFolds: ReadonlyMap<number, number>;
  releasedPins: ReleasedPin[];
  releasedPinHinges: number[];
  poseAngles: Map<number, number>;
  poseWarnings: string[];
  poseConverged: boolean;
  poseBestEffort: boolean;
  poseClosureRms: number | null;
  contactDetected: boolean;
  activeAngleIntent: { generation: number; hinges: number[]; fixAll: boolean } | null;
  angleIntentGeneration: number;
  pullHinge: number | null;
  pullMirrorHinge: number | null;
  frame3d: Frame3D | null;
  selfIntersectionPairs: readonly SelfIntersectionPair[];
  focusedSelfIntersectionPairIndex: number;
  softMesh: SoftMesh | null;
  softWarnings: string[];
  playT: number;
  replaySkipped: number[];
  replayWarnings: string[];
  mirrorAxis: MirrorAxisChoice;
  mirrorAxisNotice: string | null;
}

interface CommandServiceCallbacks {
  /** 一斉表示を捨て、入口前の道具だけを新しい作品へ引き継ぐ。 */
  discardFoldAllPreview: () => { activeTool: ToolId } | null;
  stopPlayback: () => void;
  /** 新規/開く時に、角度追従の16ms予約だけを取り消す（pose.reset相当）。 */
  resetPoseSchedule: () => void;
  clearAngleHistory: () => void;
  syncSequence: (view: DocumentView) => Promise<void>;
  syncPose: () => Promise<void>;
}

interface CommandService<State extends CommandHostState> {
  queue: SerialQueue;
  fail: (error: unknown) => void;
  runViewCommandResult: (
    task: () => Promise<DocumentView>,
    isNewDocument: boolean,
    applySuccessfulView?: boolean,
  ) => Promise<boolean>;
  runViewCommand: (
    task: () => Promise<DocumentView>,
    isNewDocument: boolean,
  ) => Promise<void>;
  applyDocChangeResult: (
    task: () => Promise<DocumentView>,
    isNewDocument?: boolean,
    preserveAngleHistory?: boolean,
    applySuccessfulView?: boolean,
  ) => Promise<boolean>;
  applyDocChange: (
    task: () => Promise<DocumentView>,
    isNewDocument?: boolean,
  ) => Promise<void>;
  latestDocChange: () => Promise<void>;
  /** Stateは型引数へ残し、別storeを作らず同じset/getへ結び付ける。 */
  readonly __stateType?: State;
}

/**
 * DocumentViewを返す命令の直列化・一括反映・最新世代判定を所有する。
 * queue実装は既存ipcQueue.tsのcreateSerialQueueをそのまま使う。
 */
export function createCommandService<State extends CommandHostState>(
  set: StoreApi<State>["setState"],
  get: StoreApi<State>["getState"],
  callbacks: CommandServiceCallbacks,
): CommandService<State> {
  const queue = createSerialQueue();
  let latestDocChange: Promise<void> = Promise.resolve();

  const applyView = (view: DocumentView, isNewDocument: boolean): void => {
    const total = view.doc.sequence.length;
    set((state) => {
      const reboundSelectedAxis =
        !isNewDocument &&
        state.doc !== null &&
        state.mirrorAxis.kind === "selectedLine"
          ? rebindSelectedMirrorAxis(state.doc, view.doc, state.mirrorAxis)
          : null;
      const selectedAxisMissing =
        state.mirrorAxis.kind === "selectedLine" &&
        !isNewDocument &&
        reboundSelectedAxis === null;
      const resetSelectedAxis =
        state.mirrorAxis.kind === "selectedLine" &&
        (isNewDocument || selectedAxisMissing);
      return {
        doc: view.doc,
        stepCreases: view.step_creases ?? [],
        display: view.doc.display,
        operationStage: state.lineInputStart === null ? state.operationStage : 0,
        lineInputStart: null,
        foldDraft: null,
        pendingFoldThrough: null,
        alignDraft: null,
        techniqueDraft: null,
        measureDraft: emptyMeasureDraft(
          isNewDocument ? "angle" : state.measureDraft.mode,
        ),
        paperActionTipVisible: false,
        paperActionTipExpanded: false,
        faces: view.faces,
        hinges: hingeEdgeIds(view.doc, view.faces),
        warnings: view.warnings,
        foldIssues: view.fold_issues ?? [],
        flatFoldViolations: keepIfSame(
          state.flatFoldViolations,
          view.flat_fold_violations ?? [],
        ),
        violations: view.violations,
        skipped: view.skipped,
        suspectHinges: keepIfSame(
          state.suspectHinges,
          view.suspect_hinges ?? [],
        ),
        sequenceTargets: new Map(
          [...(view.sequence_targets ?? [])]
            .sort((left, right) => left.hinge - right.hinge)
            .map((driver) => [driver.hinge, driver.target_angle_deg]),
        ),
        poseAngles: new Map(
          Object.entries(view.angles ?? {}).map(([id, deg]) => [Number(id), deg]),
        ),
        relaxations: view.relaxations ?? [],
        poseConverged: view.converged ?? true,
        poseBestEffort: view.best_effort === true,
        poseClosureRms:
          typeof view.closure_rms === "number" ? view.closure_rms : null,
        currentStep:
          total === 0 || state.currentStep === null
            ? null
            : Math.min(state.currentStep, total),
        selection:
          isNewDocument || state.activeTool === "measure"
            ? EMPTY_SELECTION
            : pruneSelection(state.selection, view.doc),
        mirrorAxis: resetSelectedAxis
          ? DEFAULT_MIRROR_AXIS
          : reboundSelectedAxis ?? state.mirrorAxis,
        mirrorAxisNotice:
          !isNewDocument && selectedAxisMissing
            ? MIRROR_AXIS_REMOVED_NOTICE
            : isNewDocument
              ? null
              : state.mirrorAxisNotice,
        hoveredHinge: null,
        errorMessage: null,
        documentSavedPath: null,
        contactDetected: view.contact_detected,
        docEpoch: isNewDocument ? state.docEpoch + 1 : state.docEpoch,
      } as unknown as Partial<State>;
    });
  };

  const fail = (error: unknown): void => {
    set({
      errorMessage: typeof error === "string" ? error : String(error),
      documentSavedPath: null,
    } as Partial<State>);
  };

  const runViewCommandResult = async (
    task: () => Promise<DocumentView>,
    isNewDocument: boolean,
    applySuccessfulView = true,
  ): Promise<boolean> => {
    const result = await queue.run(task);
    if (result.ok) {
      const foldAllReturn = isNewDocument
        ? callbacks.discardFoldAllPreview()
        : null;
      if (!applySuccessfulView) return true;
      applyView(result.value, isNewDocument);
      if (isNewDocument) {
        callbacks.stopPlayback();
        callbacks.resetPoseSchedule();
        callbacks.clearAngleHistory();
        set({
          drivers: new Map<number, number>(),
          pinnedFolds: new Map<number, number>(),
          releasedPins: [],
          releasedPinHinges: [],
          poseWarnings: [],
          activeAngleIntent: null,
          angleIntentGeneration: get().angleIntentGeneration + 1,
          pullHinge: null,
          pullMirrorHinge: null,
          frame3d: result.value.frame,
          selfIntersectionPairs: result.value.self_intersection_pairs ?? [],
          focusedSelfIntersectionPairIndex: 0,
          softMesh: null,
          softWarnings: [],
          currentStep: null,
          playT: 1,
          replaySkipped: [],
          replayWarnings: [],
          foldDraft: null,
          pendingFoldThrough: null,
          foldThroughBusy: false,
          alignDraft: null,
          techniqueDraft: null,
          operationStage: 0,
          ...(foldAllReturn === null
            ? {}
            : { activeTool: foldAllReturn.activeTool }),
        } as unknown as Partial<State>);
      }
      if (result.value.doc.sequence.length > 0) {
        await callbacks.syncSequence(result.value);
      } else if (!isNewDocument) {
        await callbacks.syncPose();
      }
      return true;
    }
    if (result.isLatest) fail(result.error);
    return false;
  };

  const runViewCommand = async (
    task: () => Promise<DocumentView>,
    isNewDocument: boolean,
  ): Promise<void> => {
    const pending = runViewCommandResult(task, isNewDocument).then(
      () => undefined,
    );
    latestDocChange = pending;
    await pending;
  };

  const applyDocChangeResult = (
    task: () => Promise<DocumentView>,
    isNewDocument = false,
    preserveAngleHistory = false,
    applySuccessfulView = true,
  ): Promise<boolean> => {
    const pending = (async () => {
      const succeeded = await runViewCommandResult(
        task,
        isNewDocument,
        applySuccessfulView,
      );
      if (succeeded && !preserveAngleHistory) callbacks.clearAngleHistory();
      return succeeded;
    })();
    latestDocChange = pending.then(
      () => undefined,
      () => undefined,
    );
    return pending;
  };

  const applyDocChange = async (
    task: () => Promise<DocumentView>,
    isNewDocument = false,
  ): Promise<void> => {
    await applyDocChangeResult(task, isNewDocument);
  };

  return {
    queue,
    fail,
    runViewCommandResult,
    runViewCommand,
    applyDocChangeResult,
    applyDocChange,
    latestDocChange: () => latestDocChange,
  };
}
