import type { StoreApi } from "zustand";
import * as ipc from "../../ipc/client";
import { startPlayback } from "../../lib/playback";
import { buildPoseStep, currentAngles } from "../../lib/poseStep";
import type {
  AngleRelaxation,
  DocumentView,
  SeqOp,
} from "../../lib/types";
import type { SerialQueue } from "../ipcQueue";
import {
  ANGLE_HISTORY_LIMIT,
  poseRecordReason,
  relaxationNotices as defaultRelaxationNotices,
  type PoseReplaySlice,
  type PoseReplaySliceHostState,
} from "../slices/poseReplaySlice";
import {
  createFoldAllRuntime,
  type FoldAllRuntime,
} from "./foldAllRuntime";
import {
  createPoseRuntime,
  type PoseRuntime,
  type PoseRuntimeHostState,
} from "./poseRuntime";
import {
  createReplayRuntime,
  type ReplayRuntime,
} from "./replayRuntime";

interface PoseReplayHostState
  extends PoseReplaySliceHostState,
    PoseRuntimeHostState {
  proposalStep: unknown | null;
  proposalBusy: boolean;
}

interface PoseReplayDependencies {
  queue: SerialQueue;
  fail: (error: unknown) => void;
  runViewCommand: (
    task: () => Promise<DocumentView>,
    isNewDocument: boolean,
  ) => Promise<void>;
  applyDocChangeResult: (
    task: () => Promise<DocumentView>,
    isNewDocument?: boolean,
    sameDocument?: boolean,
    clearAngleHistory?: boolean,
  ) => Promise<boolean>;
  latestDocChange: () => Promise<void>;
  invalidateFoldThrough: () => void;
  undoProposalPositionState: () => boolean;
  redoProposalPositionState: () => boolean;
  relaxationNotices?: (
    relaxations: readonly AngleRelaxation[],
  ) => AngleRelaxation[];
}

interface PoseReplayInternals {
  discardFoldAllPreview: FoldAllRuntime["discardFoldAllPreview"];
  invalidateFoldAllEntry: FoldAllRuntime["invalidateFoldAllEntry"];
  restoreAfterFoldAllPreview: FoldAllRuntime["restoreAfterFoldAllPreview"];
  waitForFoldAllRestore: FoldAllRuntime["waitForFoldAllRestore"];
  isFoldAllEntering: FoldAllRuntime["isEntering"];
  stopPlayback: ReplayRuntime["stopPlayback"];
  resetPoseSchedule: PoseRuntime["pose"]["reset"];
  clearAngleHistory: PoseRuntime["clearAngleHistory"];
  syncSequence: ReplayRuntime["syncSequence"];
  syncPose: PoseRuntime["syncPose"];
  clearZeroOnlyDrivers: PoseRuntime["clearZeroOnlyDrivers"];
  flushSoftSave: PoseRuntime["flushSoftSave"];
  isStepReplayPending: ReplayRuntime["isStepReplayPending"];
  scheduleSoftShape: PoseRuntime["scheduleSoftShape"];
  queueSoftSave: PoseRuntime["queueSoftSave"];
}

export interface CreatedPoseReplaySlice {
  slice: PoseReplaySlice;
  internals: PoseReplayInternals;
}

/** B2の31 state・23 actionを、既存の1本のZustand storeへ合成する。 */
export function createPoseReplaySlice<State extends PoseReplayHostState>(
  setState: StoreApi<State>["setState"],
  getState: StoreApi<State>["getState"],
  dependencies: PoseReplayDependencies,
): CreatedPoseReplaySlice {
  const set = setState as StoreApi<PoseReplayHostState>["setState"];
  const get = getState as StoreApi<PoseReplayHostState>["getState"];
  const {
    queue,
    fail,
    runViewCommand,
    applyDocChangeResult,
    latestDocChange,
    invalidateFoldThrough,
    undoProposalPositionState,
    redoProposalPositionState,
  } = dependencies;

  // 2つのruntimeの相互参照を、factory内で一度だけ遅延結線する。
  // eslint-disable-next-line prefer-const
  let replayRuntime!: ReplayRuntime;
  const poseRuntime = createPoseRuntime(set, get, {
    queue,
    fail,
    replayRuntime: () => replayRuntime,
  });
  replayRuntime = createReplayRuntime(set, get, {
    queue,
    fail,
    applyDocChangeResult,
    invalidateFoldThrough,
    poseRuntime,
  });
  const foldAllRuntime = createFoldAllRuntime(set, get, {
    poseRuntime,
    replayRuntime,
    latestDocChange,
    invalidateFoldThrough,
    relaxationNotices:
      dependencies.relaxationNotices ?? defaultRelaxationNotices,
  });

  let pullPushed = false;
  let pullMovedForGuide = false;
  let pullGuideStartAngle: number | null = null;
  poseRuntime.setResetExtension(() => {
    pullPushed = false;
    pullMovedForGuide = false;
    pullGuideStartAngle = null;
  });

  const undo = async (): Promise<void> => {
    if (get().foldAllPreview !== null) {
      await foldAllRuntime.restoreAfterFoldAllPreview(true);
      return;
    }
    replayRuntime.stopPlayback();
    invalidateFoldThrough();
    const state = get();
    if (state.proposalStep !== null && state.proposalBusy) return;
    if (state.proposalStep !== null && undoProposalPositionState()) return;
    const previous =
      state.angleUndoStack[state.angleUndoStack.length - 1];
    if (previous !== undefined) {
      poseRuntime.resetAngleGrouping();
      set({
        angleUndoStack: state.angleUndoStack.slice(0, -1),
        angleRedoStack: [
          ...state.angleRedoStack,
          poseRuntime.angleSnapshot(),
        ].slice(-ANGLE_HISTORY_LIMIT),
        errorMessage: null,
      });
      await poseRuntime.applyAngleSnapshot(previous);
      return;
    }
    await runViewCommand(() => ipc.editUndo(), false);
    if (get().errorMessage === null) {
      set({ docUndoDepth: get().docUndoDepth + 1 });
    }
  };

  const redo = async (): Promise<void> => {
    if (get().foldAllPreview !== null) {
      await foldAllRuntime.restoreAfterFoldAllPreview(true);
      return;
    }
    replayRuntime.stopPlayback();
    invalidateFoldThrough();
    if (get().proposalStep !== null && get().proposalBusy) return;
    if (get().proposalStep !== null && redoProposalPositionState()) return;
    if (get().docUndoDepth > 0) {
      await runViewCommand(() => ipc.editRedo(), false);
      if (get().errorMessage === null) {
        set({ docUndoDepth: Math.max(0, get().docUndoDepth - 1) });
      }
      return;
    }
    const state = get();
    const next = state.angleRedoStack[state.angleRedoStack.length - 1];
    if (next === undefined) {
      await runViewCommand(() => ipc.editRedo(), false);
      return;
    }
    poseRuntime.resetAngleGrouping();
    set({
      angleRedoStack: state.angleRedoStack.slice(0, -1),
      angleUndoStack: [
        ...state.angleUndoStack,
        poseRuntime.angleSnapshot(),
      ].slice(-ANGLE_HISTORY_LIMIT),
      errorMessage: null,
    });
    await poseRuntime.applyAngleSnapshot(next);
  };

  const applySequenceOp = async (op: SeqOp): Promise<void> => {
    if (get().foldAllPreview !== null) {
      if (!(await foldAllRuntime.restoreAfterFoldAllPreview(true))) return;
    }
    await replayRuntime.applySequenceChange(op);
  };

  const selectStep = (step: number | null): void => {
    if (get().foldAllPreview === null) {
      void replayRuntime.selectStepAndWait(step);
      return;
    }
    void foldAllRuntime.restoreAfterFoldAllPreview(true).then((restored) => {
      if (restored) void replayRuntime.selectStepAndWait(step);
    });
  };

  const selectStepForCapture = async (step: number): Promise<void> => {
    if (get().foldAllPreview !== null) {
      if (!(await foldAllRuntime.restoreAfterFoldAllPreview(true))) return;
    }
    await replayRuntime.selectStepAndWait(step);
  };

  const stepBy = (delta: number): void => {
    const state = get();
    const total = state.doc?.sequence.length ?? 0;
    if (total === 0) return;
    const from = state.currentStep ?? total;
    state.selectStep(Math.max(0, Math.min(from + delta, total)));
  };

  const togglePlay = (): void => {
    const state = get();
    if (state.foldAllPreview !== null) {
      void foldAllRuntime.restoreAfterFoldAllPreview(true).then((restored) => {
        if (restored) get().togglePlay();
      });
      return;
    }
    if (state.playing) {
      replayRuntime.stopPlayback();
      return;
    }
    const total = state.doc?.sequence.length ?? 0;
    const next = startPlayback(state.currentStep, state.playT, total);
    if (!next.playing) return;
    invalidateFoldThrough();
    set({ foldDraft: null, alignDraft: null, techniqueDraft: null });
    set({ currentStep: next.step, playT: next.t, playing: true });
    replayRuntime.schedulePlayback();
  };

  const beginPull = (
    hinge: number,
    angles: ReadonlyMap<number, number>,
    mirrorHinge: number | null = null,
  ): void => {
    if (!get().doc) return;
    invalidateFoldThrough();
    pullPushed = false;
    pullMovedForGuide = false;
    pullGuideStartAngle =
      angles.get(hinge) ??
      get().drivers.get(hinge) ??
      get().poseAngles.get(hinge) ??
      0;
    set({
      pullHinge: hinge,
      pullMirrorHinge: get().pullMirror ? mirrorHinge : null,
      errorMessage: null,
    });
    if (angles.size > 0) {
      void poseRuntime.requestPoseSolve(
        [],
        poseRuntime.preferredWithout([]),
        false,
        false,
        poseRuntime.driverList(new Map(angles)),
      );
    }
  };

  const pullTo = (deg: number): void => {
    const { pullHinge, pullMirrorHinge } = get();
    if (pullHinge === null) return;
    if (!pullPushed) {
      pullPushed = true;
      poseRuntime.pushAngleUndo(null);
    }
    const drivers = new Map(get().drivers);
    if (
      pullGuideStartAngle !== null &&
      Math.abs(deg - pullGuideStartAngle) >= 1
    ) {
      pullMovedForGuide = true;
    }
    drivers.set(pullHinge, deg);
    if (pullMirrorHinge !== null) drivers.set(pullMirrorHinge, deg);
    set({ drivers });
    poseRuntime.activateAngleIntent(
      pullMirrorHinge === null
        ? [pullHinge]
        : [pullHinge, pullMirrorHinge],
    );
    poseRuntime.pose.schedule();
  };

  const endPull = (): void => {
    if (get().pullHinge !== null) {
      set({ pullHinge: null, pullMirrorHinge: null });
      if (pullMovedForGuide) get().completeGuideAction("pull");
    }
    pullMovedForGuide = false;
    pullGuideStartAngle = null;
    void poseRuntime.finishCurrentAngleIntent();
  };

  const setDriverAngle = (hinge: number, deg: number): void => {
    if (get().foldAllPreview !== null) return;
    invalidateFoldThrough();
    const before =
      get().drivers.get(hinge) ?? get().poseAngles.get(hinge) ?? 0;
    poseRuntime.pushAngleUndo(`angle:${hinge}`);
    const drivers = new Map(get().drivers);
    drivers.set(hinge, deg);
    set({ drivers, ...poseRuntime.repinned([hinge], deg) });
    if (Math.abs(deg - before) >= 1) get().completeGuideAction("angle");
    poseRuntime.activateAngleIntent([hinge]);
    poseRuntime.pose.schedule();
  };

  const setDriverAngles = (
    hinges: readonly number[],
    deg: number,
  ): void => {
    if (get().foldAllPreview !== null) return;
    invalidateFoldThrough();
    const valid = [...new Set(hinges)]
      .filter((hinge) => get().hinges.has(hinge))
      .sort((a, b) => a - b);
    if (valid.length === 0) return;
    poseRuntime.pushAngleUndo(`angles:${valid.join(",")}`);
    const state = get();
    const drivers = new Map(state.drivers);
    const changedForGuide = valid.some((hinge) => {
      const before =
        state.drivers.get(hinge) ?? state.poseAngles.get(hinge) ?? 0;
      return Math.abs(deg - before) >= 1;
    });
    for (const hinge of valid) drivers.set(hinge, deg);
    set({ drivers, ...poseRuntime.repinned(valid, deg) });
    if (changedForGuide) get().completeGuideAction("angle");
    poseRuntime.activateAngleIntent(valid, false);
    poseRuntime.pose.schedule();
  };

  const clearDriver = (hinge: number): void => {
    if (get().foldAllPreview !== null) return;
    const drivers = new Map(get().drivers);
    if (!drivers.delete(hinge)) return;
    invalidateFoldThrough();
    poseRuntime.pushAngleUndo(null);
    set({ drivers });
    const generation = poseRuntime.activateAngleIntent([hinge]);
    void poseRuntime
      .requestPoseSolve(
        [{ hinge, target_angle_deg: 0 }],
        poseRuntime.preferredWithout([hinge]),
      )
      .finally(() => {
        if (get().activeAngleIntent?.generation === generation) {
          poseRuntime.cancelAngleIntent();
        }
      });
  };

  const clearDrivers = (): void => {
    if (get().foldAllPreview !== null) return;
    replayRuntime.stopPlayback();
    const hinges = get().hinges;
    invalidateFoldThrough();
    if (get().drivers.size > 0 || get().pinnedFolds.size > 0) {
      poseRuntime.pushAngleUndo(null);
    }
    poseRuntime.clearReleasedPins();
    set({ drivers: new Map(), pinnedFolds: new Map(), releasedPins: [] });
    poseRuntime.cancelAngleIntent();
    void poseRuntime.requestPoseSolve(poseRuntime.flatDrivers(hinges));
  };

  const togglePinnedFold = (hinge: number): void => {
    const state = get();
    if (state.foldAllPreview !== null || !state.hinges.has(hinge)) return;
    state.setPinnedFolds([hinge], !state.pinnedFolds.has(hinge));
  };

  const setPinnedFolds = (
    hinges: readonly number[],
    pinned: boolean,
  ): void => {
    const state = get();
    if (state.foldAllPreview !== null) return;
    const valid = [...new Set(hinges)].filter((hinge) =>
      state.hinges.has(hinge),
    );
    if (valid.length === 0) return;
    const changed = valid.some(
      (hinge) => state.pinnedFolds.has(hinge) !== pinned,
    );
    if (!changed) return;
    poseRuntime.pushAngleUndo(null);
    const next = new Map(state.pinnedFolds);
    for (const hinge of valid) {
      if (pinned) {
        next.set(
          hinge,
          state.drivers.get(hinge) ??
            state.sequenceTargets.get(hinge) ??
            state.poseAngles.get(hinge) ??
            0,
        );
      } else {
        next.delete(hinge);
      }
    }
    poseRuntime.clearReleasedPins();
    set({ pinnedFolds: next, releasedPins: [] });
    poseRuntime.cancelAngleIntent();
    void poseRuntime.requestPoseSolve([], poseRuntime.preferredWithout([]));
  };

  const recordPoseStep = async (): Promise<void> => {
    const before = get();
    if (before.foldAllPreview !== null) return;
    if (!before.doc || before.playing || before.hinges.size === 0) {
      set({ errorMessage: poseRecordReason(before) });
      return;
    }
    await poseRuntime.waitForLatestPose();
    const solved = get();
    const reason = poseRecordReason(solved);
    if (reason !== null || !solved.doc) {
      set({ errorMessage: reason });
      return;
    }
    const angles = currentAngles(
      solved.hinges,
      solved.drivers,
      solved.poseAngles,
    );
    const recordedGeneration = solved.angleIntentGeneration;
    await poseRuntime.flushSoftSave();
    if (get().errorMessage !== null) return;
    const latest = get();
    if (!latest.doc) return;
    set({ currentStep: null, errorMessage: null });
    await get().applySequenceOp({
      type: "PushStep",
      step: buildPoseStep(latest.doc, angles),
    });
    if (get().errorMessage !== null) return;
    if (get().angleIntentGeneration === recordedGeneration) {
      set({ drivers: new Map() });
      poseRuntime.cancelAngleIntent();
      return;
    }
    const { hard, preferred } = poseRuntime.splitDrivers();
    await poseRuntime.requestPoseSolve(
      hard,
      preferred,
      true,
      true,
      poseRuntime.driverList(new Map(get().poseAngles)),
    );
  };

  const moveStep = async (number: number, delta: number): Promise<void> => {
    if (foldAllRuntime.isEntering() || get().foldAllPreview !== null) return;
    const visibleSteps = get().doc?.sequence ?? [];
    const id = visibleSteps[number - 1]?.id;
    if (id === undefined) return;
    while (true) {
      const changesBeforeMove = latestDocChange();
      await changesBeforeMove;
      if (changesBeforeMove === latestDocChange()) break;
    }
    if (foldAllRuntime.isEntering() || get().foldAllPreview !== null) return;
    const steps = get().doc?.sequence ?? [];
    const from = steps.findIndex((candidate) => candidate.id === id);
    const to = from + delta;
    if (from < 0 || to < 0 || to >= steps.length) return;
    const succeeded = await replayRuntime.applySequenceChange(
      { type: "MoveStep", id, to_index: to },
      to === from,
    );
    if (succeeded && to !== from) get().selectStep(to + 1);
  };

  const focusNextSelfIntersectionPair = (): void => {
    set((state) => ({
      focusedSelfIntersectionPairIndex:
        state.selfIntersectionPairs.length === 0
          ? 0
          : (state.focusedSelfIntersectionPairIndex + 1) %
            state.selfIntersectionPairs.length,
    }));
  };

  const slice: PoseReplaySlice = {
    hinges: new Set<number>(),
    frame3d: null,
    selfIntersectionPairs: [],
    focusedSelfIntersectionPairIndex: 0,
    foldAllPreview: null,
    suspectHinges: [],
    sequenceTargets: new Map(),
    relaxations: [],
    softMesh: null,
    softWarnings: [],
    currentStep: null,
    playT: 1,
    playing: false,
    skipped: [],
    replaySkipped: [],
    replayWarnings: [],
    drivers: new Map(),
    pinnedFolds: new Map(),
    releasedPins: [],
    releasedPinHinges: [],
    angleUndoStack: [],
    angleRedoStack: [],
    docUndoDepth: 0,
    poseAngles: new Map(),
    poseWarnings: [],
    poseConverged: true,
    poseBestEffort: false,
    poseClosureRms: null,
    contactDetected: false,
    activeAngleIntent: null,
    angleIntentGeneration: 0,
    pullHinge: null,
    pullMirrorHinge: null,
    undo,
    redo,
    applySequenceOp,
    selectStep,
    selectStepForCapture,
    stepBy,
    togglePlay,
    beginPull,
    pullTo,
    endPull,
    setDriverAngle,
    setDriverAngles,
    finishAngleIntent: poseRuntime.finishCurrentAngleIntent,
    clearDriver,
    clearDrivers,
    enterFoldAllPreview: foldAllRuntime.enterFoldAllPreview,
    setFoldAllPercent: foldAllRuntime.setFoldAllPercent,
    finishFoldAllPercent: foldAllRuntime.finishFoldAllPercent,
    leaveFoldAllPreview: foldAllRuntime.leaveFoldAllPreview,
    focusNextSelfIntersectionPair,
    togglePinnedFold,
    setPinnedFolds,
    recordPoseStep,
    moveStep,
  };

  return {
    slice,
    internals: {
      discardFoldAllPreview: foldAllRuntime.discardFoldAllPreview,
      invalidateFoldAllEntry: foldAllRuntime.invalidateFoldAllEntry,
      restoreAfterFoldAllPreview:
        foldAllRuntime.restoreAfterFoldAllPreview,
      waitForFoldAllRestore: foldAllRuntime.waitForFoldAllRestore,
      isFoldAllEntering: foldAllRuntime.isEntering,
      stopPlayback: replayRuntime.stopPlayback,
      resetPoseSchedule: poseRuntime.pose.reset,
      clearAngleHistory: poseRuntime.clearAngleHistory,
      syncSequence: replayRuntime.syncSequence,
      syncPose: poseRuntime.syncPose,
      clearZeroOnlyDrivers: poseRuntime.clearZeroOnlyDrivers,
      flushSoftSave: poseRuntime.flushSoftSave,
      isStepReplayPending: replayRuntime.isStepReplayPending,
      scheduleSoftShape: poseRuntime.scheduleSoftShape,
      queueSoftSave: poseRuntime.queueSoftSave,
    },
  };
}
