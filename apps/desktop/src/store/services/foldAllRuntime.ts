import type { StoreApi } from "zustand";
import * as ipc from "../../ipc/client";
import type { AngleRelaxation } from "../../lib/types";
import { createSerialQueue } from "../ipcQueue";
import { EMPTY_SELECTION } from "../slices/documentSlice";
import {
  FOLD_ALL_THROTTLE_MS,
  type FoldAllReturnState,
  type PoseReplaySliceHostState,
} from "../slices/poseReplaySlice";
import {
  createTrailingThrottle,
  type PoseRuntime,
  type PoseRuntimeHostState,
} from "./poseRuntime";
import type { ReplayRuntime } from "./replayRuntime";

interface FoldAllHostState
  extends PoseReplaySliceHostState,
    Pick<
      PoseRuntimeHostState,
      "paperActionTipVisible" | "paperActionTipExpanded"
    > {}

interface FoldAllDependencies {
  poseRuntime: PoseRuntime;
  replayRuntime: ReplayRuntime;
  latestDocChange: () => Promise<void>;
  invalidateFoldThrough: () => void;
  relaxationNotices: (
    relaxations: readonly AngleRelaxation[],
  ) => AngleRelaxation[];
}

export interface FoldAllRuntime {
  enterFoldAllPreview: () => Promise<void>;
  setFoldAllPercent: (percent: number) => void;
  finishFoldAllPercent: () => void;
  leaveFoldAllPreview: () => Promise<void>;
  discardFoldAllPreview: () => FoldAllReturnState | null;
  restoreAfterFoldAllPreview: (restoreUi?: boolean) => Promise<boolean>;
  waitForFoldAllRestore: () => Promise<void>;
  invalidateFoldAllEntry: () => void;
  isEntering: () => boolean;
  reset: () => void;
}

let resetRuntime: () => void = () => {};

/** テスト用: 一斉表示の予約・専用queue・世代を初期化する。 */
export function resetFoldAllPreviewRuntime(): void {
  resetRuntime();
}

/** 一斉表示の一時状態と専用queueを所有する。作品・Undoへは混ぜない。 */
export function createFoldAllRuntime<State extends FoldAllHostState>(
  setState: StoreApi<State>["setState"],
  getState: StoreApi<State>["getState"],
  dependencies: FoldAllDependencies,
): FoldAllRuntime {
  const set = setState as StoreApi<FoldAllHostState>["setState"];
  const get = getState as StoreApi<FoldAllHostState>["getState"];
  const {
    poseRuntime,
    replayRuntime,
    latestDocChange,
    invalidateFoldThrough,
    relaxationNotices,
  } = dependencies;

  // 読み取り専用の一斉表示だけを、既存実装の専用queue 1本へ分ける。
  const foldAllQueue = createSerialQueue();
  let foldAllSessionGeneration = 0;
  let foldAllRequestGeneration = 0;
  let foldAllExitGeneration = 0;
  let latestFoldAllRestorePromise: Promise<void> = Promise.resolve();
  let foldAllEntering = false;
  let foldAllEnterGeneration = 0;

  const normalizedFoldAllPercent = (percent: number): number | null =>
    Number.isFinite(percent) ? Math.max(0, Math.min(100, percent)) : null;

  const runFoldAllPreview = async (
    percent: number,
    session: number,
    request: number,
  ): Promise<void> => {
    const result = await foldAllQueue.runLatest(async () => {
      const active = get().foldAllPreview;
      const warmSeed =
        active?.session === session ? active.nextWarmSeed : [];
      const outcome = await ipc.foldAllPreview(percent, warmSeed);
      set((state) =>
        state.foldAllPreview?.session === session &&
        !state.foldAllPreview.returning &&
        request === foldAllRequestGeneration &&
        state.docEpoch === state.foldAllPreview.returnState.docEpoch
          ? {
              foldAllPreview: {
                ...state.foldAllPreview,
                nextWarmSeed: outcome.next_warm_seed,
              },
            }
          : {},
      );
      return outcome;
    });

    const active = get().foldAllPreview;
    const isCurrent =
      active?.session === session &&
      !active.returning &&
      active.returnState.docEpoch === get().docEpoch &&
      request === foldAllRequestGeneration;
    if (!isCurrent) return;

    if (!result.ok) {
      if (result.isLatest) {
        set((state) =>
          state.foldAllPreview?.session === session &&
          !state.foldAllPreview.returning &&
          request === foldAllRequestGeneration
            ? {
                foldAllPreview: {
                  ...state.foldAllPreview,
                  busy: false,
                  error:
                    "形を更新できませんでした。つまみはそのまま動かせます。",
                },
              }
            : {},
        );
      }
      return;
    }
    if (!result.isLatest) return;

    const outcome = result.value;
    set((state) =>
      state.foldAllPreview?.session === session &&
      !state.foldAllPreview.returning &&
      request === foldAllRequestGeneration
        ? {
            frame3d: outcome.frame,
            softMesh: null,
            softWarnings: [],
            foldAllPreview: {
              ...state.foldAllPreview,
              appliedPercent: outcome.requested_percent,
              busy: false,
              error: null,
              converged: outcome.converged,
              bestEffort: outcome.best_effort === true,
              relaxationCount: relaxationNotices(
                outcome.relaxations ?? [],
              ).length,
              flatFoldViolationCount:
                outcome.flat_fold_violations.length,
              suspectHingeCount: outcome.suspect_hinges.length,
              contactDetected: outcome.contact_detected,
              layerOrder: outcome.layer_order,
              nextWarmSeed: outcome.next_warm_seed,
            },
          }
        : {},
    );
  };

  const foldAllShape = createTrailingThrottle(
    FOLD_ALL_THROTTLE_MS,
    () => {
      const active = get().foldAllPreview;
      if (active === null || active.returning) return;
      void runFoldAllPreview(
        active.percent,
        active.session,
        foldAllRequestGeneration,
      );
    },
  );

  const discardFoldAllPreview = (): FoldAllReturnState | null => {
    const active = get().foldAllPreview;
    if (active === null) return null;
    foldAllSessionGeneration++;
    foldAllRequestGeneration++;
    foldAllExitGeneration++;
    foldAllShape.clearAll();
    poseRuntime.cancelAngleIntent();
    set({ foldAllPreview: null });
    return active.returnState;
  };

  const restoreAfterFoldAllPreviewOnce = async (
    restoreUi = true,
  ): Promise<boolean> => {
    const active = get().foldAllPreview;
    if (active === null) return true;
    const previous = active.returnState;
    const token = ++foldAllExitGeneration;
    const session = ++foldAllSessionGeneration;
    foldAllRequestGeneration++;
    foldAllShape.clearAll();
    poseRuntime.cancelAngleIntent();
    set({
      foldAllPreview: {
        ...active,
        session,
        busy: false,
        returning: true,
        error: null,
      },
    });

    if (previous.docEpoch !== get().docEpoch) {
      if (token === foldAllExitGeneration) set({ foldAllPreview: null });
      return token === foldAllExitGeneration;
    }
    const state = get();
    if (state.doc === null) {
      if (token === foldAllExitGeneration) set({ foldAllPreview: null });
      return token === foldAllExitGeneration;
    }
    const total = state.doc.sequence.length;
    let restored: boolean;
    if (total > 0) {
      const upTo =
        previous.currentStep === null
          ? total
          : Math.max(0, Math.min(previous.currentStep, total));
      restored = await replayRuntime.runReplay(
        upTo,
        previous.playT,
        false,
        true,
        new Map(state.drivers),
      );
    } else {
      const normalTargets = poseRuntime.preferredWithout([]);
      restored = await poseRuntime.requestPoseSolve(
        [],
        normalTargets.length > 0
          ? normalTargets
          : poseRuntime.flatDrivers(state.hinges),
        false,
        true,
      );
    }

    if (token !== foldAllExitGeneration) return false;
    const current = get().foldAllPreview;
    if (current?.session !== session) return false;
    if (!restored) {
      set({
        errorMessage: null,
        foldAllPreview: {
          ...current,
          returning: false,
          error: "いつもの表示へ戻せませんでした。仮の形を表示したままです。",
        },
      });
      return false;
    }

    set({
      foldAllPreview: null,
      currentStep: previous.currentStep,
      playT: previous.playT,
      ...(restoreUi
        ? {
            activeTool: previous.activeTool,
            selection: {
              edgeIds: [...previous.selection.edgeIds],
              vertexIds: [...previous.selection.vertexIds],
            },
          }
        : {}),
    });
    return true;
  };

  const restoreAfterFoldAllPreview = (
    restoreUi = true,
  ): Promise<boolean> => {
    let announceDone!: () => void;
    const announced = new Promise<void>((resolve) => {
      announceDone = resolve;
    });
    latestFoldAllRestorePromise = announced;
    const pending = restoreAfterFoldAllPreviewOnce(restoreUi);
    void pending.then(announceDone, announceDone);
    return pending;
  };

  const waitForFoldAllRestore = async (): Promise<void> => {
    while (true) {
      const pending = latestFoldAllRestorePromise;
      await pending;
      if (pending === latestFoldAllRestorePromise) return;
    }
  };

  const enterFoldAllPreview = async (): Promise<void> => {
    const initial = get();
    if (
      foldAllEntering ||
      initial.foldAllPreview !== null ||
      initial.doc === null ||
      initial.hinges.size === 0
    ) {
      return;
    }

    foldAllEntering = true;
    const enterToken = ++foldAllEnterGeneration;
    try {
      replayRuntime.stopPlayback();
      invalidateFoldThrough();
      poseRuntime.pose.clearAll();
      poseRuntime.clearSoftShape();
      poseRuntime.cancelAngleIntent();
      await poseRuntime.flushSoftSave();
      if (enterToken !== foldAllEnterGeneration) return;
      while (true) {
        const changesBeforeEntry = latestDocChange();
        await changesBeforeEntry;
        if (enterToken !== foldAllEnterGeneration) return;
        if (changesBeforeEntry === latestDocChange()) break;
      }

      const before = get();
      if (
        before.foldAllPreview !== null ||
        before.doc === null ||
        before.hinges.size === 0
      ) {
        return;
      }
      replayRuntime.stopPlayback();
      invalidateFoldThrough();
      poseRuntime.pose.clearAll();
      poseRuntime.clearSoftShape();
      poseRuntime.cancelAngleIntent();
      const session = ++foldAllSessionGeneration;
      const request = ++foldAllRequestGeneration;
      const returnState: FoldAllReturnState = {
        docEpoch: before.docEpoch,
        currentStep: before.currentStep,
        playT: before.playT,
        activeTool: before.activeTool,
        selection: {
          edgeIds: [...before.selection.edgeIds],
          vertexIds: [...before.selection.vertexIds],
        },
      };
      set({
        foldAllPreview: {
          session,
          percent: 0,
          appliedPercent: null,
          busy: true,
          returning: false,
          error: null,
          converged: null,
          bestEffort: false,
          relaxationCount: 0,
          flatFoldViolationCount: 0,
          suspectHingeCount: 0,
          contactDetected: false,
          layerOrder: "unavailable_without_sequence",
          nextWarmSeed: [],
          returnState,
        },
        activeTool: "select",
        selection: EMPTY_SELECTION,
        hoveredHinge: null,
        foldDraft: null,
        alignDraft: null,
        techniqueDraft: null,
        pullHinge: null,
        pullMirrorHinge: null,
        paperActionTipVisible: false,
        paperActionTipExpanded: false,
        poseWarnings: [],
        replayWarnings: [],
        flatFoldViolations: [],
        suspectHinges: [],
        relaxations: [],
        poseConverged: true,
        poseBestEffort: false,
        poseClosureRms: null,
        contactDetected: false,
        errorMessage: null,
        documentSavedPath: null,
      });

      await runFoldAllPreview(0, session, request);
    } finally {
      foldAllEntering = false;
    }
  };

  const setFoldAllPercent = (percent: number): void => {
    const normalized = normalizedFoldAllPercent(percent);
    const active = get().foldAllPreview;
    if (normalized === null || active === null || active.returning) return;
    if (
      normalized === active.percent &&
      active.error === null &&
      active.busy
    ) {
      return;
    }
    foldAllRequestGeneration++;
    set({
      foldAllPreview: {
        ...active,
        percent: normalized,
        busy: true,
        error: null,
      },
    });
    foldAllShape.schedule();
  };

  const finishFoldAllPercent = (): void => {
    const active = get().foldAllPreview;
    if (active === null || active.returning) return;
    if (active.percent === 0) {
      // 0%は平面solveではなく、入口前の手順・角度・道具・選択を完全復帰する。
      void restoreAfterFoldAllPreview(true);
      return;
    }
    foldAllShape.flush();
  };

  const leaveFoldAllPreview = async (): Promise<void> => {
    await restoreAfterFoldAllPreview(true);
  };

  const reset = (): void => {
    foldAllSessionGeneration++;
    foldAllRequestGeneration++;
    foldAllExitGeneration++;
    foldAllEntering = false;
    foldAllEnterGeneration++;
    latestFoldAllRestorePromise = Promise.resolve();
    foldAllShape.clearAll();
    set({ foldAllPreview: null });
  };
  resetRuntime = reset;

  return {
    enterFoldAllPreview,
    setFoldAllPercent,
    finishFoldAllPercent,
    leaveFoldAllPreview,
    discardFoldAllPreview,
    restoreAfterFoldAllPreview,
    waitForFoldAllRestore,
    invalidateFoldAllEntry: () => {
      foldAllEnterGeneration++;
    },
    isEntering: () => foldAllEntering,
    reset,
  };
}
