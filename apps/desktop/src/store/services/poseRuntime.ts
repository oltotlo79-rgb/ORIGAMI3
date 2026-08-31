import type { StoreApi } from "zustand";
import * as ipc from "../../ipc/client";
import { softOf } from "../../lib/displayPrefs";
import type {
  DisplaySettings,
  Driver,
  SoftMesh,
  SoftSettings,
} from "../../lib/types";
import {
  MAX_PIN_RELEASES_ON_SETTLE,
  keptFoldsFailed,
  pinReleaseCandidates,
  pinReleaseNotice,
  releasedPins as releasedPinsOf,
  splitKeptFolds,
  withFoldDeviationNotice,
} from "../../lib/settledFolds";
import { keepIfSame } from "./commandService";
import type { SerialQueue } from "../ipcQueue";
import { nonZeroDriverCount } from "../slices/documentSlice";
import {
  ANGLE_GROUP_MS,
  ANGLE_HISTORY_LIMIT,
  FINISH_JUMP_NOTICE,
  FINISH_JUMP_NOTICE_THRESHOLD,
  POSE_THROTTLE_MS,
  finishComparisonFrame,
  keepIfSameReleasedPins,
  maximumFrameVertexMovement,
  selfIntersectionDisplayState,
  type AngleSnapshot,
  type PoseReplaySliceHostState,
} from "../slices/poseReplaySlice";

/** たわみ指定を作品へ書き込むまでの待ち（ms）。 */
const SOFT_SAVE_MS = 400;

/** B4が所有する設定と案内を、B2 runtimeが必要な分だけ読む境界。 */
export interface PoseRuntimeHostState extends PoseReplaySliceHostState {
  display: DisplaySettings;
  pullMirror: boolean;
  paperActionTipVisible: boolean;
  paperActionTipExpanded: boolean;
  setDisplay: (patch: Partial<DisplaySettings>) => Promise<void>;
  completeGuideAction: (action: "angle" | "pull" | "inflate" | "fold") => void;
}

interface PoseRuntimeDependencies {
  queue: SerialQueue;
  fail: (error: unknown) => void;
  replayRuntime: () => PoseReplayBridge;
}

interface PoseReplayBridge {
  runReplay: (
    upTo: number,
    t: number,
    coalesce?: boolean,
    settlePins?: boolean,
    poseOverrides?: ReadonlyMap<number, number> | null,
    mode?: ipc.PoseSolveMode,
  ) => Promise<boolean>;
}

interface TrailingThrottle {
  schedule: () => void;
  reset: () => void;
  flush: () => void;
  clearAll: () => void;
}

/** 連続入力をintervalMsごとにまとめ、予約中の最後の1件を必ず残す。 */
export function createTrailingThrottle(
  intervalMs: number,
  fn: () => void,
): TrailingThrottle {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastRun = 0;
  const clear = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  return {
    schedule: () => {
      clear();
      const wait = Math.max(0, intervalMs - (Date.now() - lastRun));
      timer = setTimeout(() => {
        timer = null;
        lastRun = Date.now();
        fn();
      }, wait);
    },
    reset: () => {
      clear();
      lastRun = Date.now();
    },
    flush: () => {
      if (timer === null) return;
      clear();
      lastRun = Date.now();
      fn();
    },
    clearAll: () => {
      clear();
      lastRun = 0;
    },
  };
}

let resetRuntime: () => void = () => {};

/** テスト用: 角度・softの間引きと履歴まとめ時計を初期化する。 */
export function resetPoseThrottle(): void {
  resetRuntime();
}

export interface PoseRuntime {
  pose: TrailingThrottle;
  driverList: (drivers: ReadonlyMap<number, number>) => Driver[];
  flatDrivers: (hinges: ReadonlySet<number>) => Driver[];
  activateAngleIntent: (hinges: readonly number[], fixAll?: boolean) => number;
  cancelAngleIntent: () => void;
  repinned: (
    hinges: readonly number[],
    deg: number,
  ) => { pinnedFolds?: ReadonlyMap<number, number> };
  angleSnapshot: () => AngleSnapshot;
  pushAngleUndo: (key: string | null) => void;
  resetAngleGrouping: () => void;
  clearAngleHistory: () => void;
  splitDrivers: () => { hard: Driver[]; preferred: Driver[] };
  preferredWithout: (hardHinges: readonly number[]) => Driver[];
  clearReleasedPins: () => void;
  requestPoseSolve: (
    hard: Driver[],
    preferred?: Driver[],
    coalesce?: boolean,
    applyFrame?: boolean,
    warmSeed?: Driver[],
    mode?: ipc.PoseSolveMode,
  ) => Promise<boolean>;
  waitForLatestPose: () => Promise<void>;
  syncPose: () => Promise<void>;
  finishCurrentAngleIntent: () => Promise<void>;
  applyAngleSnapshot: (next: AngleSnapshot) => Promise<boolean>;
  softArg: () => SoftSettings | null;
  softResult: (
    mesh: SoftMesh | null | undefined,
  ) => { softMesh: SoftMesh | null; softWarnings: string[] };
  solveReplayPose: (
    preferred: Driver[],
    coalesce: boolean,
    warmSeed: Driver[],
    settlePins: boolean,
    replayPosition: { upTo: number; replayT: number },
    mode: ipc.PoseSolveMode,
  ) => Promise<boolean>;
  clearZeroOnlyDrivers: () => void;
  scheduleSoftShape: () => void;
  clearSoftShape: () => void;
  queueSoftSave: (display: DisplaySettings) => void;
  flushSoftSave: () => Promise<void>;
  setResetExtension: (extension: () => void) => void;
  reset: () => void;
}

/** 姿勢solve・手順再生・角度履歴を、Zustandを増やさず1本のstoreへ載せる。 */
export function createPoseRuntime<State extends PoseRuntimeHostState>(
  setState: StoreApi<State>["setState"],
  getState: StoreApi<State>["getState"],
  dependencies: PoseRuntimeDependencies,
): PoseRuntime {
  const set = setState as StoreApi<PoseRuntimeHostState>["setState"];
  const get = getState as StoreApi<PoseRuntimeHostState>["getState"];
  const { queue, fail, replayRuntime } = dependencies;

  const clearZeroOnlyDrivers = (): void => {
    const drivers = get().drivers;
    if (drivers.size > 0 && nonZeroDriverCount(drivers) === 0) {
      set({ drivers: new Map() });
    }
  };

  const driverList = (drivers: ReadonlyMap<number, number>): Driver[] =>
    [...drivers]
      .sort(([left], [right]) => left - right)
      .map(([hinge, deg]) => ({ hinge, target_angle_deg: deg }));

  const flatDrivers = (hinges: ReadonlySet<number>): Driver[] =>
    [...hinges]
      .sort((left, right) => left - right)
      .map((hinge) => ({ hinge, target_angle_deg: 0 }));

  const snapshotSeed = (
    drivers: ReadonlyMap<number, number>,
    hinges: ReadonlySet<number>,
  ): Driver[] =>
    [...hinges]
      .sort((left, right) => left - right)
      .map((hinge) => ({
        hinge,
        target_angle_deg: drivers.get(hinge) ?? 0,
      }));

  let unfinishedAngleIntentGeneration: number | null = null;

  const activateAngleIntent = (
    hinges: readonly number[],
    fixAll = true,
  ): number => {
    const generation = get().angleIntentGeneration + 1;
    set({
      angleIntentGeneration: generation,
      activeAngleIntent: {
        generation,
        hinges: [...new Set(hinges)].sort((a, b) => a - b),
        fixAll,
      },
    });
    unfinishedAngleIntentGeneration = generation;
    return generation;
  };

  const cancelAngleIntent = (): void => {
    unfinishedAngleIntentGeneration = null;
    set((state) => ({
      angleIntentGeneration: state.angleIntentGeneration + 1,
      activeAngleIntent: null,
    }));
  };

  let lastAngleKey: string | null = null;
  let lastAngleAt = 0;

  const repinned = (
    hinges: readonly number[],
    deg: number,
  ): { pinnedFolds?: ReadonlyMap<number, number> } => {
    const state = get();
    const target = hinges.filter((hinge) => state.pinnedFolds.has(hinge));
    if (target.length === 0) return {};
    const next = new Map(state.pinnedFolds);
    for (const hinge of target) next.set(hinge, deg);
    return { pinnedFolds: next };
  };

  const angleSnapshot = (): AngleSnapshot => {
    const state = get();
    return {
      drivers: new Map(state.drivers),
      pinned: new Map(state.pinnedFolds),
    };
  };

  const pushAngleUndo = (key: string | null): void => {
    const now = Date.now();
    if (
      key !== null &&
      key === lastAngleKey &&
      now - lastAngleAt < ANGLE_GROUP_MS
    ) {
      lastAngleAt = now;
      return;
    }
    lastAngleKey = key;
    lastAngleAt = now;
    const state = get();
    set({
      angleUndoStack: [...state.angleUndoStack, angleSnapshot()].slice(
        -ANGLE_HISTORY_LIMIT,
      ),
      angleRedoStack: [],
    });
  };

  const clearAngleHistory = (): void => {
    lastAngleKey = null;
    const state = get();
    if (
      state.angleUndoStack.length === 0 &&
      state.angleRedoStack.length === 0 &&
      state.docUndoDepth === 0
    ) {
      return;
    }
    set({ angleUndoStack: [], angleRedoStack: [], docUndoDepth: 0 });
  };

  const mergedTargets = (
    state: PoseRuntimeHostState,
  ): Map<number, number> => {
    const merged = new Map(
      [...state.sequenceTargets].filter(([hinge]) => state.hinges.has(hinge)),
    );
    for (const [hinge, deg] of state.pinnedFolds) {
      if (state.hinges.has(hinge)) merged.set(hinge, deg);
    }
    for (const [hinge, deg] of state.drivers) merged.set(hinge, deg);
    return merged;
  };

  const splitDrivers = (): { hard: Driver[]; preferred: Driver[] } => {
    const state = get();
    const moving = (state.activeAngleIntent?.hinges ?? [])
      .filter((hinge) => state.drivers.has(hinge))
      .sort((a, b) => a - b);
    const active = new Set(
      state.activeAngleIntent?.fixAll === false ? moving.slice(0, 1) : moving,
    );
    const hard: Driver[] = [];
    const preferred: Driver[] = [];
    for (const [hinge, deg] of [...mergedTargets(state)].sort(
      ([a], [b]) => a - b,
    )) {
      (active.has(hinge) ? hard : preferred).push({
        hinge,
        target_angle_deg: deg,
      });
    }
    return { hard, preferred };
  };

  const preferredWithout = (hardHinges: readonly number[]): Driver[] => {
    const excluded = new Set(hardHinges);
    return [...mergedTargets(get())]
      .filter(([hinge]) => !excluded.has(hinge))
      .sort(([a], [b]) => a - b)
      .map(([hinge, target_angle_deg]) => ({ hinge, target_angle_deg }));
  };

  const softArg = (): SoftSettings | null => {
    const soft = softOf(get().display);
    return soft.enabled ? soft : null;
  };

  const softResult = (mesh: SoftMesh | null | undefined) => ({
    softMesh: mesh ?? null,
    softWarnings: keepIfSame(get().softWarnings, mesh?.warnings ?? []),
  });

  const clearReleasedPins = (): void => {
    if (get().releasedPinHinges.length > 0) set({ releasedPinHinges: [] });
  };

  // 相互参照するruntimeを、factory完成後に一度だけ結線する。
  // eslint-disable-next-line prefer-const
  let pose!: TrailingThrottle;

  const runPoseSolve = async (
    hard: Driver[],
    preferred: Driver[] = [],
    coalesce = false,
    applyFrame = true,
    warmSeed: Driver[] = [],
    deepSearch = false,
    replayPosition?: { upTo: number; replayT: number },
    mode: ipc.PoseSolveMode = "Follow",
  ): Promise<boolean> => {
    const requestGeneration = get().angleIntentGeneration;
    pose.reset();
    const soft = softArg();
    const position = get();
    const total = position.doc?.sequence.length ?? 0;
    const upTo = replayPosition?.upTo ?? position.currentStep ?? total;
    const replayT =
      replayPosition?.replayT ??
      (position.currentStep === null ? 1 : position.playT);
    const pinnedHinges = new Set(position.pinnedFolds.keys());
    const send = (h: Driver[], p: Driver[]) =>
      mode === "Follow"
        ? ipc.poseSolve(h, p, soft, warmSeed, upTo, replayT)
        : ipc.poseSolve(h, p, soft, warmSeed, upTo, replayT, mode);
    const attempt = (h: Driver[], p: Driver[]) => {
      const call = () => send(h, p);
      return coalesce ? queue.runLatest(call) : queue.run(call);
    };
    const attemptWithReleased = async (released: ReadonlySet<number>) => {
      const { kept, rest } = splitKeptFolds(
        preferred,
        pinnedHinges,
        released,
      );
      return { kept, result: await attempt([...hard, ...kept], rest) };
    };

    let releasedOrder = [...get().releasedPinHinges];
    const released = new Set(releasedOrder);
    for (const hinge of [...released]) {
      if (
        !pinnedHinges.has(hinge) &&
        !preferred.some((driver) => driver.hinge === hinge)
      ) {
        released.delete(hinge);
        releasedOrder = releasedOrder.filter((item) => item !== hinge);
      }
    }
    let { kept, result } = await attemptWithReleased(released);
    if (requestGeneration !== get().angleIntentGeneration) return false;

    if (
      deepSearch &&
      result.ok &&
      result.isLatest &&
      !keptFoldsFailed(result.value) &&
      released.size > 0
    ) {
      const readmit = [...releasedOrder]
        .reverse()
        .find((hinge) => released.has(hinge));
      if (readmit !== undefined) {
        const candidate = new Set(released);
        candidate.delete(readmit);
        const retry = await attemptWithReleased(candidate);
        if (requestGeneration !== get().angleIntentGeneration) return false;
        if (
          retry.result.ok &&
          retry.result.isLatest &&
          !keptFoldsFailed(retry.result.value)
        ) {
          released.delete(readmit);
          releasedOrder = releasedOrder.filter((hinge) => hinge !== readmit);
          kept = retry.kept;
          result = retry.result;
        }
      }
    }

    if (
      result.ok &&
      result.isLatest &&
      kept.length > 0 &&
      keptFoldsFailed(result.value)
    ) {
      const diagnosis = await attempt(hard, preferred);
      if (requestGeneration !== get().angleIntentGeneration) return false;
      if (diagnosis.ok && diagnosis.isLatest) {
        const diagnosedAngles = new Map(
          Object.entries(diagnosis.value.angles).map(([id, deg]) => [
            Number(id),
            deg,
          ]),
        );
        const candidates = pinReleaseCandidates(
          kept,
          pinnedHinges,
          diagnosedAngles,
        );
        let attemptsLeft = deepSearch ? MAX_PIN_RELEASES_ON_SETTLE : 1;
        result = diagnosis;
        for (const candidate of candidates) {
          if (attemptsLeft <= 0) break;
          if (released.has(candidate.hinge)) continue;
          released.add(candidate.hinge);
          releasedOrder = [...releasedOrder, candidate.hinge];
          attemptsLeft--;
          if (!deepSearch) break;
          const verified = await attemptWithReleased(released);
          if (requestGeneration !== get().angleIntentGeneration) return false;
          if (!verified.result.ok || !verified.result.isLatest) break;
          if (!keptFoldsFailed(verified.result.value)) {
            result = verified.result;
            break;
          }
        }
      }
    }

    if (!result.ok) {
      if (result.isLatest) fail(result.error);
      return false;
    }
    if (!result.isLatest) return false;
    const poseAngles = new Map(
      Object.entries(result.value.angles).map(([id, deg]) => [Number(id), deg]),
    );
    const deviationWarnings = withFoldDeviationNotice(
      result.value.frame.warnings,
      [...hard, ...preferred],
      poseAngles,
    );
    const movedPins = releasedPinsOf(
      preferred.filter((driver) => pinnedHinges.has(driver.hinge)),
      poseAngles,
    );
    const pinNotice = pinReleaseNotice(movedPins);
    const poseWarnings =
      pinNotice === null
        ? deviationWarnings
        : [...deviationWarnings, pinNotice];
    set({
      ...(applyFrame
        ? {
            frame3d: result.value.frame,
            ...selfIntersectionDisplayState(
              get(),
              result.value.self_intersection_pairs,
            ),
          }
        : {}),
      ...(applyFrame ? softResult(result.value.soft) : {}),
      ...(applyFrame
        ? {
            suspectHinges: keepIfSame(
              get().suspectHinges,
              result.value.suspect_hinges ?? [],
            ),
            poseWarnings: keepIfSame(get().poseWarnings, poseWarnings),
            flatFoldViolations: keepIfSame(
              get().flatFoldViolations,
              result.value.flat_fold_violations ?? [],
            ),
            poseConverged: result.value.converged,
            poseBestEffort: result.value.best_effort === true,
            poseClosureRms:
              typeof result.value.closure_rms === "number"
                ? result.value.closure_rms
                : null,
            relaxations: result.value.relaxations ?? [],
            contactDetected: result.value.contact_detected === true,
            releasedPins: keepIfSameReleasedPins(
              get().releasedPins,
              movedPins,
            ),
          }
        : {}),
      releasedPinHinges: keepIfSame(
        get().releasedPinHinges,
        releasedOrder,
      ),
      poseAngles,
    });
    return true;
  };

  let latestPosePromise: Promise<boolean> = Promise.resolve(true);
  const requestPoseSolve: PoseRuntime["requestPoseSolve"] = (
    hard,
    preferred = [],
    coalesce = false,
    applyFrame = true,
    warmSeed = [],
    mode = "Follow",
  ) => {
    const pending = runPoseSolve(
      hard,
      preferred,
      coalesce,
      applyFrame,
      warmSeed,
      !coalesce,
      undefined,
      mode,
    );
    latestPosePromise = pending;
    return pending;
  };

  const solveReplayPose: PoseRuntime["solveReplayPose"] = (
    preferred,
    coalesce,
    warmSeed,
    settlePins,
    replayPosition,
    mode,
  ) => {
    const pending = runPoseSolve(
      [],
      preferred,
      coalesce,
      true,
      warmSeed,
      settlePins,
      replayPosition,
      mode,
    );
    latestPosePromise = pending;
    return pending;
  };

  const waitForLatestPose = async (): Promise<void> => {
    while (true) {
      pose.flush();
      const pending = latestPosePromise;
      await pending;
      pose.flush();
      if (pending === latestPosePromise) return;
    }
  };

  const syncPose = async (): Promise<void> => {
    const hinges = get().hinges;
    const before = get().drivers;
    const kept = new Map([...before].filter(([hinge]) => hinges.has(hinge)));
    if (kept.size !== before.size) set({ drivers: kept });
    const pinnedBefore = get().pinnedFolds;
    const pinnedKept = new Map(
      [...pinnedBefore].filter(([hinge]) => hinges.has(hinge)),
    );
    if (pinnedKept.size !== pinnedBefore.size) {
      clearReleasedPins();
      set({ pinnedFolds: pinnedKept, releasedPins: [] });
    }
    if (kept.size === 0 && get().frame3d === null) return;
    if (kept.size === 0) {
      await requestPoseSolve(flatDrivers(hinges));
      return;
    }
    cancelAngleIntent();
    await requestPoseSolve([], preferredWithout([]));
  };

  pose = createTrailingThrottle(POSE_THROTTLE_MS, () => {
    const { hard, preferred } = splitDrivers();
    latestPosePromise = runPoseSolve(hard, preferred, true);
  });

  const applyAngleSnapshot = async (
    next: AngleSnapshot,
  ): Promise<boolean> => {
    const drivers = new Map(next.drivers);
    const pinnedFolds = new Map(next.pinned);
    clearReleasedPins();
    set({ drivers, pinnedFolds, releasedPins: [] });
    cancelAngleIntent();
    const restoreGeneration = get().angleIntentGeneration;
    pose.clearAll();

    const state = get();
    const total = state.doc?.sequence.length ?? 0;
    if (total > 0) {
      const upTo = state.currentStep ?? total;
      const t = state.currentStep === null ? 1 : state.playT;
      const restored = await replayRuntime().runReplay(
        upTo,
        t,
        false,
        true,
        drivers,
        "Canonical",
      );
      if (restoreGeneration !== get().angleIntentGeneration) return false;
      return restored;
    }

    return await requestPoseSolve(
      [],
      preferredWithout([]),
      false,
      true,
      snapshotSeed(drivers, state.hinges),
      "Canonical",
    );
  };

  const finishCurrentAngleIntent = async (): Promise<void> => {
    const generation = unfinishedAngleIntentGeneration;
    lastAngleKey = null;
    if (generation === null || generation !== get().angleIntentGeneration) {
      if (unfinishedAngleIntentGeneration === generation) {
        unfinishedAngleIntentGeneration = null;
      }
      return;
    }
    const errorBeforeFollow = get().errorMessage;
    pose.flush();
    const pending = latestPosePromise;
    const followSucceeded = await pending;
    if (
      generation !== get().angleIntentGeneration ||
      unfinishedAngleIntentGeneration !== generation
    ) {
      return;
    }
    const errorAfterFollow = get().errorMessage;
    const followError =
      !followSucceeded && errorAfterFollow !== errorBeforeFollow
        ? errorAfterFollow
        : null;
    const followFrame = followSucceeded
      ? finishComparisonFrame(get().frame3d)
      : null;
    const canonicalSucceeded = await applyAngleSnapshot(angleSnapshot());
    if (
      canonicalSucceeded &&
      followError !== null &&
      get().errorMessage === followError
    ) {
      set({ errorMessage: null });
    }
    const canonicalFrame = get().frame3d;
    if (canonicalSucceeded && followFrame !== null && canonicalFrame !== null) {
      const movement = maximumFrameVertexMovement(followFrame, canonicalFrame);
      if (
        movement !== null &&
        movement >= FINISH_JUMP_NOTICE_THRESHOLD
      ) {
        set((state) => ({
          poseWarnings: state.poseWarnings.includes(FINISH_JUMP_NOTICE)
            ? state.poseWarnings
            : [...state.poseWarnings, FINISH_JUMP_NOTICE],
        }));
      }
    }
  };

  const refreshShape = (): void => {
    const state = get();
    if (!state.doc) return;
    const total = state.doc.sequence.length;
    if (total === 0) {
      void requestPoseSolve([], preferredWithout([]), true);
      return;
    }
    void replayRuntime().runReplay(
      state.currentStep ?? total,
      state.currentStep === null ? 1 : state.playT,
      true,
    );
  };

  const softShape = createTrailingThrottle(POSE_THROTTLE_MS, refreshShape);
  let softPending = false;
  let pendingSoftDisplay: DisplaySettings | null = null;
  const softSave = createTrailingThrottle(SOFT_SAVE_MS, () => {
    const display = pendingSoftDisplay;
    pendingSoftDisplay = null;
    softPending = false;
    if (display) void get().setDisplay(display);
  });

  const flushSoftSave = async (): Promise<void> => {
    if (!softPending) return;
    const display = pendingSoftDisplay;
    pendingSoftDisplay = null;
    softPending = false;
    softSave.reset();
    if (display) await get().setDisplay(display);
  };

  const queueSoftSave = (display: DisplaySettings): void => {
    softPending = true;
    pendingSoftDisplay = display;
    softSave.schedule();
  };

  let resetExtension: () => void = () => {};
  const reset = (): void => {
    softPending = false;
    pendingSoftDisplay = null;
    pose.clearAll();
    latestPosePromise = Promise.resolve(true);
    softShape.clearAll();
    softSave.clearAll();
    lastAngleKey = null;
    lastAngleAt = 0;
    resetExtension();
    cancelAngleIntent();
    clearAngleHistory();
  };
  resetRuntime = reset;

  return {
    pose,
    driverList,
    flatDrivers,
    activateAngleIntent,
    cancelAngleIntent,
    repinned,
    angleSnapshot,
    pushAngleUndo,
    resetAngleGrouping: () => {
      lastAngleKey = null;
    },
    clearAngleHistory,
    splitDrivers,
    preferredWithout,
    clearReleasedPins,
    requestPoseSolve,
    waitForLatestPose,
    syncPose,
    finishCurrentAngleIntent,
    applyAngleSnapshot,
    softArg,
    softResult,
    solveReplayPose,
    clearZeroOnlyDrivers,
    scheduleSoftShape: softShape.schedule,
    clearSoftShape: softShape.clearAll,
    queueSoftSave,
    flushSoftSave,
    setResetExtension: (extension) => {
      resetExtension = extension;
    },
    reset,
  };
}
