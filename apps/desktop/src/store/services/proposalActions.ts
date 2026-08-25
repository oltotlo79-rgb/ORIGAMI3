import type { StoreApi } from "zustand";
import * as ipc from "../../ipc/client";
import {
  clampTipPos,
  defaultSkeleton,
  leafNodes,
  setTipPos,
} from "../../lib/skeleton";
import { clampPaperPosition, paperBounds } from "../../lib/paperPosition";
import {
  paperEditorPositions,
  proposalLeafPositionStates,
  proposalRequestSkeleton,
  setLastMovedSource,
  setSpecifiedPaperPosition,
} from "../../lib/proposalPosition";
import type {
  Paper,
  PaperTipPosition,
  ProposalCandidate,
  ProposalJobId,
  Skeleton,
} from "../../lib/types";
import {
  ANGLE_GROUP_MS,
  ANGLE_HISTORY_LIMIT,
} from "../slices/poseReplaySlice";
import type {
  ProposalPositionSnapshot,
  ProposalSlice,
  ProposalSliceHostState,
  ProposalSliceState,
} from "../slices/proposalSlice";

/** 提案の計算に使う紙（作品が無いときの控え）。 */
const FALLBACK_PAPER: Paper = { width_mm: 150, height_mm: 150 };
const PROPOSAL_PROGRESS_POLL_MS = 150;
const PROPOSAL_PROGRESS_FULL_HOLD_MS = 150;

/** 1件の提案計算にだけ属する、保存しないruntime資源。 */
interface ActiveProposalJob {
  jobId: ProposalJobId;
  generation: number;
  stopPolling: (() => void) | null;
  releaseFullHold: (() => void) | null;
}

interface ProposalDependencies {
  applyDocChange: (
    task: () => ReturnType<typeof ipc.proposalApply>,
    isNewDocument?: boolean,
  ) => Promise<void>;
}

interface ProposalInternals {
  undoProposalPositionState: () => boolean;
  redoProposalPositionState: () => boolean;
}

interface CreatedProposalSlice {
  slice: ProposalSlice;
  internals: ProposalInternals;
}

/** 提案state/action/job runtimeを既存の1本のZustand storeへ合成する。 */
export function createProposalSlice<State extends ProposalSliceHostState>(
  setState: StoreApi<State>["setState"],
  getState: StoreApi<State>["getState"],
  dependencies: ProposalDependencies,
): CreatedProposalSlice {
  const set = setState as StoreApi<ProposalSliceHostState>["setState"];
  const get = getState as StoreApi<ProposalSliceHostState>["getState"];
  const { applyDocChange } = dependencies;

  let proposalGeneration = 0;
  let activeProposalJob: ActiveProposalJob | null = null;

  const isCurrentProposalJob = (job: ActiveProposalJob): boolean =>
    activeProposalJob === job &&
    job.generation === proposalGeneration &&
    get().proposalJobId === job.jobId;

  const stopProposalJobTimers = (job: ActiveProposalJob): void => {
    const stopPolling = job.stopPolling;
    job.stopPolling = null;
    stopPolling?.();
    const releaseFullHold = job.releaseFullHold;
    job.releaseFullHold = null;
    releaseFullHold?.();
  };

  const requestProposalCancel = (job: ActiveProposalJob): void => {
    try {
      void ipc
        .proposalControl({ type: "Cancel", job_id: job.jobId })
        .catch(() => undefined);
    } catch {
      // backendのwatchdogが残った計算を回収する。画面側は既に無効化済み。
    }
  };

  const invalidateProposalJob = (cancel: boolean): void => {
    proposalGeneration++;
    const job = activeProposalJob;
    activeProposalJob = null;
    if (job !== null) {
      stopProposalJobTimers(job);
      if (cancel) requestProposalCancel(job);
    }
    set({
      proposalBusy: false,
      proposalJobId: null,
      proposalProgress: null,
      proposalProgressWarning: null,
    });
  };

  const startProposalJob = (): ActiveProposalJob => {
    const job: ActiveProposalJob = {
      jobId: crypto.randomUUID(),
      generation: ++proposalGeneration,
      stopPolling: null,
      releaseFullHold: null,
    };
    activeProposalJob = job;
    return job;
  };

  const warnProposalProgress = (job: ActiveProposalJob): void => {
    if (
      !isCurrentProposalJob(job) ||
      get().proposalProgressWarning !== null
    ) {
      return;
    }
    set({
      proposalProgressWarning:
        "進み具合を読み取れませんでした。計算はそのまま続けています。",
    });
  };

  const watchProposalProgress = (job: ActiveProposalJob): (() => void) => {
    let stopped = false;
    let pollInFlight = false;
    const poll = (): void => {
      if (stopped || pollInFlight || !isCurrentProposalJob(job)) return;
      pollInFlight = true;
      try {
        void ipc
          .proposalProgress(job.jobId)
          .then((progress) => {
            if (
              stopped ||
              progress === null ||
              progress.job_id !== job.jobId ||
              !isCurrentProposalJob(job)
            ) {
              return;
            }
            const previous = get().proposalProgress;
            if (
              previous?.job_id === job.jobId &&
              progress.done < previous.done
            ) {
              return;
            }
            set({ proposalProgress: progress });
          })
          .catch(() => {
            if (!stopped) warnProposalProgress(job);
          })
          .finally(() => {
            pollInFlight = false;
          });
      } catch {
        pollInFlight = false;
        warnProposalProgress(job);
      }
    };
    const timer = setInterval(poll, PROPOSAL_PROGRESS_POLL_MS);
    const stop = (): void => {
      if (stopped) return;
      stopped = true;
      clearInterval(timer);
    };
    job.stopPolling = stop;
    return stop;
  };

  const holdFullProposalBar = async (
    job: ActiveProposalJob,
  ): Promise<void> => {
    const progress = get().proposalProgress;
    if (
      progress === null ||
      progress.job_id !== job.jobId ||
      progress.total <= 0 ||
      !isCurrentProposalJob(job)
    ) {
      return;
    }
    set({ proposalProgress: { ...progress, done: progress.total } });
    await new Promise<void>((resolve) => {
      let finished = false;
      let timer: ReturnType<typeof setTimeout> | null = null;
      const finish = (): void => {
        if (finished) return;
        finished = true;
        if (timer !== null) clearTimeout(timer);
        if (job.releaseFullHold === finish) job.releaseFullHold = null;
        resolve();
      };
      job.releaseFullHold = finish;
      timer = setTimeout(finish, PROPOSAL_PROGRESS_FULL_HOLD_MS);
      if (!isCurrentProposalJob(job)) finish();
    });
  };

  const runProposalGeneration = async (
    requestSkeleton: Skeleton,
    paper: Paper,
    seed: number,
    resultState: (
      candidates: ProposalCandidate[],
    ) => Partial<ProposalSliceState>,
  ): Promise<void> => {
    let job: ActiveProposalJob;
    try {
      job = startProposalJob();
    } catch {
      set({
        proposalBusy: false,
        proposalJobId: null,
        proposalProgress: null,
        proposalProgressWarning: null,
        proposalError:
          "計算を始められませんでした。画面を開き直して、もう一度お試しください。",
      });
      return;
    }
    set({
      proposalBusy: true,
      proposalJobId: job.jobId,
      proposalProgress: null,
      proposalProgressWarning: null,
      proposalError: null,
      proposalSeed: seed + 1,
    });
    const stopWatching = watchProposalProgress(job);
    try {
      const result = await ipc.proposalGenerate(
        requestSkeleton,
        paper,
        seed,
        job.jobId,
      );
      if (!isCurrentProposalJob(job)) return;
      if (result.job_id !== job.jobId) {
        set({
          proposalBusy: false,
          proposalJobId: null,
          proposalProgress: null,
          proposalProgressWarning: null,
          proposalError:
            "別の計算結果が届いたため、使いませんでした。もう一度お試しください。",
        });
        return;
      }
      stopWatching();
      await holdFullProposalBar(job);
      if (!isCurrentProposalJob(job)) return;
      set({
        ...resultState(result.candidates),
        proposalBusy: false,
        proposalJobId: null,
        proposalProgress: null,
        proposalProgressWarning: null,
      });
    } catch (error) {
      if (!isCurrentProposalJob(job)) return;
      set({
        proposalBusy: false,
        proposalJobId: null,
        proposalProgress: null,
        proposalProgressWarning: null,
        proposalError:
          typeof error === "string" ? error : String(error),
      });
    } finally {
      stopProposalJobTimers(job);
      if (activeProposalJob === job) activeProposalJob = null;
    }
  };

  let lastProposalPositionKey: string | null = null;
  let lastProposalPositionAt = 0;

  const cloneProposalSkeleton = (skeleton: Skeleton): Skeleton => ({
    nodes: skeleton.nodes.map((node) => ({
      ...node,
      ...(node.tip_pos_2d == null
        ? {}
        : { tip_pos_2d: { ...node.tip_pos_2d } }),
    })),
  });

  const clonePaperPositions = (
    positions: readonly PaperTipPosition[],
  ): PaperTipPosition[] =>
    positions.map((entry) => ({
      leaf_id: entry.leaf_id,
      position: { ...entry.position },
    }));

  const proposalPositionSnapshot = (): ProposalPositionSnapshot => {
    const state = get();
    if (state.proposalStep === null) {
      throw new Error("proposal position snapshot requires an open proposal");
    }
    return {
      step: state.proposalStep,
      skeleton: cloneProposalSkeleton(state.proposalSkeleton),
      candidates: state.proposalCandidates,
      selected: state.proposalSelected,
      paperSource: state.proposalPaperSource,
      paperPositions: clonePaperPositions(state.proposalPaperPositions),
      paperSpecified: clonePaperPositions(state.proposalPaperSpecified),
      lastMoved: state.proposalPositionLastMoved.map((entry) => ({
        ...entry,
      })),
    };
  };

  const pushProposalPositionUndo = (key: string | null): void => {
    const now = Date.now();
    if (
      key !== null &&
      key === lastProposalPositionKey &&
      now - lastProposalPositionAt < ANGLE_GROUP_MS
    ) {
      lastProposalPositionAt = now;
      return;
    }
    lastProposalPositionKey = key;
    lastProposalPositionAt = now;
    const state = get();
    set({
      proposalPositionUndoStack: [
        ...state.proposalPositionUndoStack,
        proposalPositionSnapshot(),
      ].slice(-ANGLE_HISTORY_LIMIT),
      proposalPositionRedoStack: [],
    });
  };

  const undoProposalPositionState = (): boolean => {
    const state = get();
    if (state.proposalBusy) return false;
    const previous =
      state.proposalPositionUndoStack[
        state.proposalPositionUndoStack.length - 1
      ];
    if (!previous) return false;
    lastProposalPositionKey = null;
    set({
      proposalStep: previous.step,
      proposalSkeleton: cloneProposalSkeleton(previous.skeleton),
      proposalCandidates: previous.candidates,
      proposalSelected: previous.selected,
      proposalPaperSource: previous.paperSource,
      proposalPaperPositions: clonePaperPositions(previous.paperPositions),
      proposalPaperSpecified: clonePaperPositions(previous.paperSpecified),
      proposalPositionLastMoved: previous.lastMoved.map((entry) => ({
        ...entry,
      })),
      proposalPositionUndoStack: state.proposalPositionUndoStack.slice(0, -1),
      proposalPositionRedoStack: [
        ...state.proposalPositionRedoStack,
        proposalPositionSnapshot(),
      ].slice(-ANGLE_HISTORY_LIMIT),
      proposalError: null,
    });
    return true;
  };

  const redoProposalPositionState = (): boolean => {
    const state = get();
    if (state.proposalBusy) return false;
    const next =
      state.proposalPositionRedoStack[
        state.proposalPositionRedoStack.length - 1
      ];
    if (!next) return false;
    lastProposalPositionKey = null;
    set({
      proposalStep: next.step,
      proposalSkeleton: cloneProposalSkeleton(next.skeleton),
      proposalCandidates: next.candidates,
      proposalSelected: next.selected,
      proposalPaperSource: next.paperSource,
      proposalPaperPositions: clonePaperPositions(next.paperPositions),
      proposalPaperSpecified: clonePaperPositions(next.paperSpecified),
      proposalPositionLastMoved: next.lastMoved.map((entry) => ({ ...entry })),
      proposalPositionRedoStack: state.proposalPositionRedoStack.slice(0, -1),
      proposalPositionUndoStack: [
        ...state.proposalPositionUndoStack,
        proposalPositionSnapshot(),
      ].slice(-ANGLE_HISTORY_LIMIT),
      proposalError: null,
    });
    return true;
  };

  const openProposal = (): void => {
    invalidateProposalJob(true);
    lastProposalPositionKey = null;
    set({
      proposalStep: "skeleton",
      proposalSkeleton: defaultSkeleton(),
      proposalCandidates: [],
      proposalSelected: null,
      proposalPaperSource: null,
      proposalPaperPositions: [],
      proposalPaperSpecified: [],
      proposalPositionLastMoved: [],
      proposalPositionUndoStack: [],
      proposalPositionRedoStack: [],
      proposalBusy: false,
      proposalError: null,
    });
  };

  const closeProposal = (): void => {
    invalidateProposalJob(true);
    lastProposalPositionKey = null;
    set({
      proposalStep: null,
      proposalPaperSource: null,
      proposalPaperPositions: [],
      proposalPaperSpecified: [],
      proposalPositionLastMoved: [],
      proposalPositionUndoStack: [],
      proposalPositionRedoStack: [],
      proposalBusy: false,
    });
  };

  const setProposalSkeleton = (skeleton: Skeleton): void => {
    const leafIds = new Set(leafNodes(skeleton).map((node) => node.id));
    const state = get();
    lastProposalPositionKey = null;
    set({
      proposalSkeleton: skeleton,
      proposalCandidates: [],
      proposalSelected: null,
      proposalPaperSource: null,
      proposalPaperPositions: [],
      proposalPaperSpecified: state.proposalPaperSpecified.filter((entry) =>
        leafIds.has(entry.leaf_id),
      ),
      proposalPositionLastMoved: state.proposalPositionLastMoved.filter(
        (entry) => leafIds.has(entry.leaf_id),
      ),
      proposalPositionUndoStack: [],
      proposalPositionRedoStack: [],
    });
  };

  const setProposalTipPosition: ProposalSlice["setProposalTipPosition"] = (
    leafId,
    position,
  ) => {
    const state = get();
    if (state.proposalBusy) return;
    const node = leafNodes(state.proposalSkeleton).find(
      (leaf) => leaf.id === leafId,
    );
    if (!node) return;
    const fit = position === null ? null : clampTipPos(position);
    const before = node.tip_pos_2d ?? null;
    if (
      (before === null && fit === null) ||
      (before !== null &&
        fit !== null &&
        before.x === fit.x &&
        before.y === fit.y)
    ) {
      return;
    }
    pushProposalPositionUndo(`completion:${leafId}`);
    const paperExists = state.proposalPaperSpecified.some(
      (entry) => entry.leaf_id === leafId,
    );
    set({
      proposalSkeleton: setTipPos(state.proposalSkeleton, leafId, fit),
      proposalCandidates: [],
      proposalSelected: null,
      proposalPaperSource: null,
      proposalPaperPositions: [],
      proposalPositionLastMoved:
        fit !== null
          ? setLastMovedSource(
              state.proposalPositionLastMoved,
              leafId,
              "completion",
            )
          : paperExists
            ? setLastMovedSource(
                state.proposalPositionLastMoved,
                leafId,
                "paper",
              )
            : state.proposalPositionLastMoved.filter(
                (entry) => entry.leaf_id !== leafId,
              ),
      proposalError: null,
    });
  };

  const generateProposal = async (): Promise<void> => {
    const state = get();
    if (state.proposalBusy) return;
    const paper = state.doc?.paper ?? FALLBACK_PAPER;
    const seed = state.proposalSeed;
    const requestSkeleton = proposalRequestSkeleton(
      state.proposalSkeleton,
      state.proposalPaperSpecified,
      state.proposalPositionLastMoved,
      paper,
    );
    await runProposalGeneration(requestSkeleton, paper, seed, (list) => ({
      proposalCandidates: list,
      proposalSelected: list.length > 0 ? 0 : null,
      proposalPaperSource: null,
      proposalPaperPositions: [],
      proposalStep: list.length > 0 ? "candidates" : "skeleton",
      proposalError:
        list.length > 0
          ? null
          : "候補を作れませんでした。骨格を変えてみてください",
    }));
  };

  const selectProposalCandidate = (index: number): void => {
    const list = get().proposalCandidates;
    if (index < 0 || index >= list.length) return;
    set({
      proposalSelected: index,
      proposalPaperSource: null,
      proposalPaperPositions: [],
    });
  };

  const openProposalPaperPositionEditor = (): void => {
    const state = get();
    const source = state.proposalSelected;
    const candidate =
      source === null ? undefined : state.proposalCandidates[source];
    if (!candidate) return;
    const positions = paperEditorPositions(
      candidate,
      state.proposalSkeleton,
      state.proposalPaperSpecified,
    );
    if (positions.length === 0) return;
    set({
      proposalStep: "paper-position",
      proposalPaperSource: source,
      proposalPaperPositions: positions,
      proposalError: null,
    });
  };

  const setProposalPaperPosition: ProposalSlice["setProposalPaperPosition"] = (
    leafId,
    position,
  ) => {
    const state = get();
    if (state.proposalBusy) return;
    const source = state.proposalPaperSource;
    const candidate =
      source === null ? undefined : state.proposalCandidates[source];
    if (!candidate) return;
    const index = state.proposalPaperPositions.findIndex(
      (entry) => entry.leaf_id === leafId,
    );
    if (index < 0) return;
    const fit = clampPaperPosition(position, paperBounds(candidate.cp));
    const specified = state.proposalPaperSpecified.find(
      (entry) => entry.leaf_id === leafId,
    )?.position;
    if (
      specified?.x === fit.x &&
      specified.y === fit.y &&
      state.proposalPositionLastMoved.find(
        (entry) => entry.leaf_id === leafId,
      )?.source === "paper"
    ) {
      return;
    }
    pushProposalPositionUndo(`paper:${leafId}`);
    const next = state.proposalPaperPositions.map((entry, entryIndex) =>
      entryIndex === index ? { ...entry, position: fit } : entry,
    );
    set({
      proposalPaperPositions: next,
      proposalPaperSpecified: setSpecifiedPaperPosition(
        state.proposalPaperSpecified,
        leafId,
        fit,
      ),
      proposalPositionLastMoved: setLastMovedSource(
        state.proposalPositionLastMoved,
        leafId,
        "paper",
      ),
      proposalError: null,
    });
  };

  const resetProposalPaperPositions = (): void => {
    const state = get();
    if (state.proposalBusy) return;
    const source = state.proposalPaperSource;
    const candidate =
      source === null ? undefined : state.proposalCandidates[source];
    if (!candidate) return;
    const shownIds = new Set(
      state.proposalPaperPositions.map((entry) => entry.leaf_id),
    );
    if (
      !state.proposalPaperSpecified.some((entry) =>
        shownIds.has(entry.leaf_id),
      )
    ) {
      return;
    }
    pushProposalPositionUndo(null);
    const paperSpecified = state.proposalPaperSpecified.filter(
      (entry) => !shownIds.has(entry.leaf_id),
    );
    let lastMoved = state.proposalPositionLastMoved.filter(
      (entry) => !shownIds.has(entry.leaf_id),
    );
    for (const leafId of shownIds) {
      const completion = state.proposalSkeleton.nodes.find(
        (node) => node.id === leafId,
      )?.tip_pos_2d;
      if (completion != null) {
        lastMoved = setLastMovedSource(lastMoved, leafId, "completion");
      }
    }
    set({
      proposalPaperPositions: paperEditorPositions(
        candidate,
        state.proposalSkeleton,
        paperSpecified,
      ),
      proposalPaperSpecified: paperSpecified,
      proposalPositionLastMoved: lastMoved,
      proposalError: null,
    });
  };

  const restoreOtherProposalPosition = (leafId: number): void => {
    const state = get();
    if (state.proposalBusy) return;
    const paper = state.doc?.paper ?? FALLBACK_PAPER;
    const positionState = proposalLeafPositionStates(
      state.proposalSkeleton,
      state.proposalPaperSpecified,
      state.proposalPositionLastMoved,
      paper,
    ).find((entry) => entry.leaf_id === leafId);
    if (
      !positionState ||
      positionState.kind !== "different" ||
      positionState.used === null
    ) {
      return;
    }
    pushProposalPositionUndo(null);
    let skeleton = state.proposalSkeleton;
    let paperSpecified = state.proposalPaperSpecified;
    let lastMoved = state.proposalPositionLastMoved;
    if (positionState.used === "paper") {
      paperSpecified = paperSpecified.filter(
        (entry) => entry.leaf_id !== leafId,
      );
      lastMoved = setLastMovedSource(lastMoved, leafId, "completion");
    } else {
      skeleton = setTipPos(skeleton, leafId, null);
      lastMoved = setLastMovedSource(lastMoved, leafId, "paper");
    }
    const source = state.proposalPaperSource;
    const candidate =
      source === null ? undefined : state.proposalCandidates[source];
    const stayOnPaper =
      state.proposalStep === "paper-position" && candidate !== undefined;
    set({
      proposalSkeleton: skeleton,
      proposalPaperSpecified: paperSpecified,
      proposalPositionLastMoved: lastMoved,
      proposalPaperPositions: stayOnPaper
        ? paperEditorPositions(candidate, skeleton, paperSpecified)
        : [],
      ...(stayOnPaper
        ? {}
        : {
            proposalStep: "skeleton",
            proposalCandidates: [],
            proposalSelected: null,
            proposalPaperSource: null,
          }),
      proposalError: null,
    });
  };

  const generateProposalFromPaperPositions = async (): Promise<void> => {
    const state = get();
    if (state.proposalBusy || state.proposalPaperPositions.length === 0) return;
    const paper = state.doc?.paper ?? FALLBACK_PAPER;
    const seed = state.proposalSeed;
    const requestSkeleton = proposalRequestSkeleton(
      state.proposalSkeleton,
      state.proposalPaperSpecified,
      state.proposalPositionLastMoved,
      paper,
    );
    await runProposalGeneration(requestSkeleton, paper, seed, (list) => ({
      proposalCandidates:
        list.length > 0 ? list : state.proposalCandidates,
      proposalSelected: list.length > 0 ? 0 : state.proposalSelected,
      proposalPaperSource:
        list.length > 0 ? null : state.proposalPaperSource,
      proposalPaperPositions:
        list.length > 0 ? [] : state.proposalPaperPositions,
      proposalStep:
        list.length > 0 ? "candidates" : "paper-position",
      proposalError:
        list.length > 0
          ? null
          : "この場所では候補を作れませんでした。丸い印を少し離してみてください",
    }));
  };

  const applyProposalCandidate = async (): Promise<void> => {
    const state = get();
    const chosen =
      state.proposalSelected === null
        ? undefined
        : state.proposalCandidates[state.proposalSelected];
    if (!chosen) return;
    invalidateProposalJob(true);
    lastProposalPositionKey = null;
    set({
      proposalStep: null,
      proposalPaperSource: null,
      proposalPaperPositions: [],
      proposalPaperSpecified: [],
      proposalPositionLastMoved: [],
      proposalPositionUndoStack: [],
      proposalPositionRedoStack: [],
    });
    const plan = chosen.fold_plan;
    if (plan && plan.steps.length > 0) {
      await applyDocChange(
        () => ipc.proposalApply(plan.cp, plan.steps),
        true,
      );
      return;
    }
    await get().applyEdit({ type: "ReplaceCreasePattern", cp: chosen.cp });
  };

  const slice: ProposalSlice = {
    proposalStep: null,
    proposalSkeleton: defaultSkeleton(),
    proposalCandidates: [],
    proposalSelected: null,
    proposalPaperSource: null,
    proposalPaperPositions: [],
    proposalPaperSpecified: [],
    proposalPositionLastMoved: [],
    proposalPositionUndoStack: [],
    proposalPositionRedoStack: [],
    proposalBusy: false,
    proposalJobId: null,
    proposalProgress: null,
    proposalProgressWarning: null,
    proposalError: null,
    proposalSeed: 1,
    openProposal,
    closeProposal,
    setProposalStep: (step) => set({ proposalStep: step }),
    setProposalSkeleton,
    setProposalTipPosition,
    generateProposal,
    selectProposalCandidate,
    openProposalPaperPositionEditor,
    setProposalPaperPosition,
    resetProposalPaperPositions,
    restoreOtherProposalPosition,
    undoProposalPosition: () => {
      undoProposalPositionState();
    },
    redoProposalPosition: () => {
      redoProposalPositionState();
    },
    generateProposalFromPaperPositions,
    applyProposalCandidate,
  };

  return {
    slice,
    internals: {
      undoProposalPositionState,
      redoProposalPositionState,
    },
  };
}
