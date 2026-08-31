import type { StoreApi } from "zustand";
import * as ipc from "../../ipc/client";
import { advancePlayback } from "../../lib/playback";
import type { DocumentView, SeqOp } from "../../lib/types";
import { withFoldDeviationNotice } from "../../lib/settledFolds";
import type { SerialQueue } from "../ipcQueue";
import {
  FALLBACK_FRAME_MS,
  keepIfSameReleasedPins,
  selfIntersectionDisplayState,
} from "../slices/poseReplaySlice";
import { keepIfSame } from "./commandService";
import type { PoseRuntime, PoseRuntimeHostState } from "./poseRuntime";

interface ReplayRuntimeDependencies {
  queue: SerialQueue;
  fail: (error: unknown) => void;
  applyDocChangeResult: (
    task: () => Promise<DocumentView>,
    isNewDocument?: boolean,
    sameDocument?: boolean,
    clearAngleHistory?: boolean,
  ) => Promise<boolean>;
  invalidateFoldThrough: () => void;
  poseRuntime: PoseRuntime;
}

export interface ReplayRuntime {
  runReplay: (
    upTo: number,
    t: number,
    coalesce?: boolean,
    settlePins?: boolean,
    poseOverrides?: ReadonlyMap<number, number> | null,
    mode?: ipc.PoseSolveMode,
  ) => Promise<boolean>;
  selectStepAndWait: (step: number | null) => Promise<void>;
  syncSequence: (view: DocumentView) => Promise<void>;
  stopPlayback: () => void;
  applySequenceChange: (op: SeqOp, samePosition?: boolean) => Promise<boolean>;
  schedulePlayback: () => void;
  isStepReplayPending: () => boolean;
}

/** 手順再生とanimation frameを、姿勢solveから独立したruntimeへまとめる。 */
export function createReplayRuntime<State extends PoseRuntimeHostState>(
  setState: StoreApi<State>["setState"],
  getState: StoreApi<State>["getState"],
  dependencies: ReplayRuntimeDependencies,
): ReplayRuntime {
  const set = setState as StoreApi<PoseRuntimeHostState>["setState"];
  const get = getState as StoreApi<PoseRuntimeHostState>["getState"];
  const {
    queue,
    fail,
    applyDocChangeResult,
    invalidateFoldThrough,
    poseRuntime,
  } = dependencies;

  let cancelFrame: (() => void) | null = null;
  let lastTs = 0;
  let stepReplayGeneration = 0;
  let stepReplayPending = false;

  const stopPlayback = (): void => {
    cancelFrame?.();
    cancelFrame = null;
    if (get().playing) set({ playing: false });
  };

  const runReplay: ReplayRuntime["runReplay"] = async (
    upTo,
    t,
    coalesce = false,
    settlePins = !coalesce,
    poseOverrides = null,
    mode = "Follow",
  ) => {
    const requestGeneration = get().angleIntentGeneration;
    const call = () => ipc.sequenceReplay(upTo, t, poseRuntime.softArg());
    const result = await (coalesce ? queue.runLatest(call) : queue.run(call));
    if (requestGeneration !== get().angleIntentGeneration) return false;
    if (!result.ok) {
      if (result.isLatest) {
        stopPlayback();
        fail(result.error);
      }
      return false;
    }
    if (!result.isLatest) return false;
    const state = get();
    const requestedAngles = [...(result.value.sequence_targets ?? [])].sort(
      (a, b) => a.hinge - b.hinge,
    );
    const sequenceTargets = new Map(
      requestedAngles.map((driver) => [
        driver.hinge,
        driver.target_angle_deg,
      ]),
    );
    const replayAngles = result.value.angles
      ? new Map(
          Object.entries(result.value.angles).map(([id, deg]) => [
            Number(id),
            deg,
          ]),
        )
      : null;
    const activePins = [...state.pinnedFolds].filter(([hinge]) =>
      state.hinges.has(hinge),
    );
    const replayState = {
      replaySkipped: keepIfSame(state.replaySkipped, result.value.skipped),
      replayWarnings: keepIfSame(
        state.replayWarnings,
        result.value.warnings,
      ),
      sequenceTargets,
    };

    if (
      activePins.length === 0 &&
      (poseOverrides === null || poseOverrides.size === 0)
    ) {
      set({
        ...replayState,
        frame3d: result.value.frame,
        ...selfIntersectionDisplayState(
          get(),
          result.value.self_intersection_pairs,
        ),
        ...poseRuntime.softResult(result.value.soft),
        flatFoldViolations: keepIfSame(
          state.flatFoldViolations,
          result.value.flat_fold_violations ?? [],
        ),
        suspectHinges: keepIfSame(
          state.suspectHinges,
          result.value.suspect_hinges ?? [],
        ),
        poseAngles: replayAngles ?? state.poseAngles,
        poseWarnings: keepIfSame(
          state.poseWarnings,
          withFoldDeviationNotice(
            result.value.frame.warnings,
            requestedAngles,
            replayAngles ?? new Map(),
          ),
        ),
        releasedPins: keepIfSameReleasedPins(state.releasedPins, []),
        releasedPinHinges: keepIfSame(state.releasedPinHinges, []),
        relaxations: result.value.relaxations ?? [],
        poseConverged: result.value.converged ?? true,
        poseBestEffort: result.value.best_effort === true,
        poseClosureRms:
          typeof result.value.closure_rms === "number"
            ? result.value.closure_rms
            : null,
        contactDetected: result.value.contact_detected === true,
      });
      return true;
    }

    set({
      ...replayState,
      poseWarnings: [],
      releasedPins: [],
      releasedPinHinges: [],
    });
    const preferred = new Map(sequenceTargets);
    for (const [hinge, deg] of activePins) preferred.set(hinge, deg);
    for (const [hinge, deg] of poseOverrides ?? []) preferred.set(hinge, deg);
    return await poseRuntime.solveReplayPose(
      poseRuntime.driverList(preferred),
      coalesce,
      replayAngles === null ? [] : poseRuntime.driverList(replayAngles),
      settlePins,
      { upTo, replayT: t },
      mode,
    );
  };

  const selectStepAndWait = async (step: number | null): Promise<void> => {
    stopPlayback();
    invalidateFoldThrough();
    if (get().foldDraft || get().alignDraft) {
      set({ foldDraft: null, alignDraft: null });
    }
    if (get().techniqueDraft) set({ techniqueDraft: null });
    const state = get();
    const total = state.doc?.sequence.length ?? 0;
    if (total === 0) {
      set({ currentStep: null, playT: 1 });
      return;
    }
    const upTo =
      step === null ? total : Math.max(0, Math.min(step, total));
    const next = step === null ? null : upTo;
    if (state.currentStep === next && state.playT === 1) return;
    const replayGeneration = ++stepReplayGeneration;
    stepReplayPending = true;
    set({ currentStep: next, playT: 1 });
    try {
      await runReplay(upTo, 1);
    } finally {
      if (stepReplayGeneration === replayGeneration) {
        stepReplayPending = false;
      }
    }
  };

  const syncSequence = async (view: DocumentView): Promise<void> => {
    const step = get().currentStep;
    if (step === null) {
      const hasPins = [...get().pinnedFolds.keys()].some((hinge) =>
        get().hinges.has(hinge),
      );
      if (hasPins) {
        set({ replaySkipped: [], replayWarnings: [] });
        await runReplay(view.doc.sequence.length, 1, true, true);
        return;
      }
      const viewAngles = new Map(
        Object.entries(view.angles ?? {}).map(([id, deg]) => [
          Number(id),
          deg,
        ]),
      );
      set((state) => ({
        frame3d: view.frame,
        ...selfIntersectionDisplayState(
          state,
          view.self_intersection_pairs,
        ),
        replaySkipped: [],
        replayWarnings: [],
        poseWarnings: keepIfSame(
          state.poseWarnings,
          withFoldDeviationNotice(
            view.frame?.warnings ?? [],
            view.sequence_targets ?? [],
            viewAngles,
          ),
        ),
        releasedPins: keepIfSameReleasedPins(state.releasedPins, []),
        releasedPinHinges: keepIfSame(state.releasedPinHinges, []),
        suspectHinges: keepIfSame(
          state.suspectHinges,
          view.suspect_hinges ?? [],
        ),
      }));
      if (poseRuntime.softArg()) {
        await runReplay(view.doc.sequence.length, 1, true);
      }
      return;
    }
    set({ playT: 1 });
    await runReplay(step, 1, true, true);
  };

  const scheduleFrame = (cb: (ts: number) => void): (() => void) => {
    if (typeof requestAnimationFrame === "function") {
      const id = requestAnimationFrame(cb);
      return () => cancelAnimationFrame(id);
    }
    const timer = setTimeout(() => cb(Date.now()), FALLBACK_FRAME_MS);
    return () => clearTimeout(timer);
  };

  const tick = (ts: number): void => {
    cancelFrame = null;
    const state = get();
    if (!state.playing) return;
    const total = state.doc?.sequence.length ?? 0;
    const dt = lastTs === 0 ? 0 : ts - lastTs;
    lastTs = ts;
    const next = advancePlayback(
      { step: state.currentStep ?? 0, t: state.playT, playing: true },
      dt,
      total,
    );
    set({ currentStep: next.step, playT: next.t, playing: next.playing });
    void runReplay(next.step, next.t, true, !next.playing);
    if (next.playing) cancelFrame = scheduleFrame(tick);
  };

  const schedulePlayback = (): void => {
    lastTs = 0;
    cancelFrame = scheduleFrame(tick);
  };

  const applySequenceChange = (
    op: SeqOp,
    samePosition = false,
  ): Promise<boolean> => {
    if (!samePosition) {
      stopPlayback();
      invalidateFoldThrough();
    }
    return applyDocChangeResult(
      () => ipc.sequenceApply(op),
      false,
      samePosition,
      !samePosition,
    );
  };

  return {
    runReplay,
    selectStepAndWait,
    syncSequence,
    stopPlayback,
    applySequenceChange,
    schedulePlayback,
    isStepReplayPending: () => stepReplayPending,
  };
}
