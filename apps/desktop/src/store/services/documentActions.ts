import type { StoreApi } from "zustand";
import * as ipc from "../../ipc/client";
import {
  foldLayers,
  keepSidePoint,
  offsetPoint,
  topMovingFace,
} from "../../components/Viewer3D/foldDraw";
import { planGrabFold } from "../../components/Viewer3D/grabFold";
import { foldPoseInputFromDrivers } from "../../lib/poseStep";
import { foldBlockReason } from "../../lib/viewerHint";
import { DEFAULT_CONSTRUCT } from "../../lib/construct";
import {
  DEFAULT_CURVE,
  firstCrossing,
  rulingLines,
} from "../../lib/curve";
import {
  ALIGN_STEPS,
  solveAlign,
} from "../../lib/alignFold";
import {
  mirrorLineForChoice,
  mirrorSegmentSet,
  mirrorSegments,
  paperMirrorLine,
  selectedEdgeMirrorLine,
  type MirrorAxisChoice,
  type MirrorLine,
  type Segment,
} from "../../lib/mirror";
import { withMirrorEdges } from "../../lib/mirrorEdit";
import {
  DEFAULT_TWIST_DEG,
  addTwistVertex,
  isTwistPolygonReady,
  polygonCentroid,
  twistReferencePoint,
  undoTwistVertex,
} from "../../lib/twistPolygon";
import {
  clampTechniqueLayerCount,
  minimumTechniqueFlap,
  techniqueFlapForPreset,
  techniqueUsesOpenToBack,
  toggleTechniqueFlap as toggleTechniqueFlapSelection,
} from "../../lib/techniqueLayers";
import {
  buildLayerMotionPart,
  hasLayerMotionInput,
} from "../../lib/layerMotion";
import type {
  Document,
  EdgeKind,
  EditOp,
  SeqOp,
  Vec2,
} from "../../lib/types";
import { createGenerationGate } from "./generationGate";
import {
  DEFAULT_PLEAT_WIDTH_MM,
  EMPTY_SELECTION,
  STALE_DRAFT_MESSAGE,
  TECHNIQUE_FALLBACK_HINT,
  addWorkingEdge,
  alignFoldDraft,
  clearCurrentLayerMotion,
  editableCopy,
  emptyMeasureDraft,
  foldInsertAt,
  foldThroughUnavailableMessage,
  foldUnavailableMessage,
  initialMovingSide,
  isAlignComplete,
  isSpatialFoldFrame,
  layerMotionPartDraft,
  selectionForMeasure,
  type AddSegmentOp,
  type AlignCpPick,
  type DocumentSlice,
  type DocumentSliceDependencies,
  type DocumentSliceFactoryResult,
  type DocumentSliceHostState,
  type FoldThroughApplyOp,
  type FoldThroughOperation,
  type MeasureEdgePick,
  type MeasurePointPick,
  type SpatialFoldDrag,
} from "../slices/documentSlice";

/**
 * document/CP状態を同じZustand storeへ合成するfactory。
 * createは呼ばず、appStoreが所有するset/getと既存command serviceを受け取る。
 */
export function createDocumentSlice<State extends DocumentSliceHostState>(
  setState: StoreApi<State>["setState"],
  getState: StoreApi<State>["getState"],
  dependencies: DocumentSliceDependencies,
): DocumentSliceFactoryResult {
  const set = setState as StoreApi<DocumentSliceHostState>["setState"];
  const get = getState as StoreApi<DocumentSliceHostState>["getState"];
  const {
    queue,
    runViewCommand,
    applyDocChange,
    fail,
    invalidateFoldAllEntry,
    flushSoftSave,
    waitForFoldAllRestore,
    restoreAfterFoldAllPreview,
    stopPlayback,
    isStepReplayPending,
    persistPrefs,
    relaxationNotices,
    clearZeroOnlyDrivers,
  } = dependencies;
  const foldThroughGate = createGenerationGate();
  const foldThroughBusyGate = createGenerationGate();

  const invalidateFoldThrough = (): void => {
    foldThroughGate.issue();
    if (get().pendingFoldThrough !== null) set({ pendingFoldThrough: null });
  };

  const finishFoldThroughBusy = (token: number): void => {
    if (foldThroughBusyGate.isCurrent(token) && get().foldThroughBusy) {
      set({ foldThroughBusy: false });
    }
  };

  const activeMirrorLine = (
    doc: Document,
    choice: MirrorAxisChoice,
  ): MirrorLine =>
    mirrorLineForChoice(doc, choice) ??
    paperMirrorLine(doc.paper, "paperVertical");

  const segmentEditsWithAxis = (
    a: Vec2,
    b: Vec2,
    kind: EdgeKind,
    axis: MirrorLine | null,
  ): AddSegmentOp[] => {
    const segments = axis
      ? mirrorSegments([a, b], axis)
      : ([[a, b]] as [Vec2, Vec2][]);
    return segments.map(([p, q]) => ({ type: "AddSegment", a: p, b: q, kind }));
  };

  const onlyAddSegments = (ops: EditOp[]): ops is AddSegmentOp[] =>
    ops.every((one) => one.type === "AddSegment");

  const addSegmentsWithMirror = (
    ops: AddSegmentOp[],
    axis: MirrorLine,
  ): AddSegmentOp[] =>
    mirrorSegmentSet(
      ops.map((one) => ({ key: one.kind, segment: [one.a, one.b] as Segment })),
      axis,
    ).map(({ key, segment }) => ({
      type: "AddSegment",
      a: segment[0],
      b: segment[1],
      kind: key,
    }));

  const poseBeforeMatchesDrivers = (
    operation: FoldThroughOperation,
    drivers: ReadonlyMap<number, number>,
  ): boolean => {
    const expected = operation.pose_before ?? null;
    const current = foldPoseInputFromDrivers(drivers);
    if (!current.ok) return false;
    const actual = current.poseBefore;
    if (expected === null || actual === null) return expected === actual;
    return (
      expected.drivers.length === actual.drivers.length &&
      expected.drivers.every(
        (driver, index) =>
          driver.edge_id === actual.drivers[index].edge_id &&
          driver.target_angle_deg === actual.drivers[index].target_angle_deg,
      )
    );
  };

  const applyFoldThrough = async (
    operation: FoldThroughOperation,
    acceptAdditionalCrease: boolean,
    busyToken: number,
  ): Promise<void> => {
    const state = get();
    if (!state.doc) {
      set({ pendingFoldThrough: null });
      finishFoldThroughBusy(busyToken);
      return;
    }
    const beforeEpoch = state.docEpoch;
    const beforeSequenceCount = state.doc.sequence.length;
    set({
      currentStep:
        operation.up_to === state.doc.sequence.length
          ? null
          : operation.up_to + (operation.pose_before ? 2 : 1),
    });
    try {
      await applyDocChange(() => {
        const applyOperation: FoldThroughApplyOp & { spatial?: SpatialFoldDrag } = {
          ...operation,
          accept_additional_crease: acceptAdditionalCrease,
        };
        return ipc.sequenceApply(applyOperation);
      });
    } finally {
      finishFoldThroughBusy(busyToken);
    }
    const completed = get();
    if (
      completed.errorMessage === null &&
      completed.docEpoch === beforeEpoch &&
      (completed.doc?.sequence.length ?? 0) > beforeSequenceCount
    ) {
      if (
        operation.pose_before &&
        poseBeforeMatchesDrivers(operation, completed.drivers)
      ) {
        set((latest) => ({
          drivers: new Map(),
          activeAngleIntent: null,
          angleIntentGeneration: latest.angleIntentGeneration + 1,
        }));
      }
      completed.completeGuideAction("fold");
    }
  };

  const requestFoldThrough = async (
    operation: FoldThroughOperation,
  ): Promise<void> => {
    if (
      get().foldAllPreview !== null ||
      get().foldThroughBusy ||
      get().pendingFoldThrough
    ) {
      return;
    }
    clearZeroOnlyDrivers();
    stopPlayback();
    const started = get();
    const revision = foldThroughGate.issue();
    const busyToken = foldThroughBusyGate.issue();
    set({
      pendingFoldThrough: null,
      foldThroughBusy: true,
      errorMessage: null,
    });
    const result = await queue.run(() => {
      const previewOperation: Extract<SeqOp, { type: "PreviewFoldThrough" }> & {
        spatial?: SpatialFoldDrag;
      } = {
        type: "PreviewFoldThrough",
        up_to: operation.up_to,
        line: operation.line,
        keep_side_point: operation.keep_side_point,
        target_layers: operation.target_layers,
        direction: operation.direction,
        ...(operation.pose_before
          ? { pose_before: operation.pose_before }
          : {}),
        ...(operation.spatial ? { spatial: operation.spatial } : {}),
      };
      return ipc.sequenceApply(previewOperation);
    });
    if (!foldThroughGate.isCurrent(revision)) {
      finishFoldThroughBusy(busyToken);
      return;
    }
    if (!result.ok) {
      if (result.isLatest) fail(result.error);
      finishFoldThroughBusy(busyToken);
      return;
    }
    if (!result.isLatest) {
      finishFoldThroughBusy(busyToken);
      return;
    }
    const current = get();
    if (!current.doc) {
      finishFoldThroughBusy(busyToken);
      set({ foldDraft: null, alignDraft: null, errorMessage: STALE_DRAFT_MESSAGE });
      return;
    }
    const stale =
      current.docEpoch !== started.docEpoch ||
      current.doc.sequence.length !== started.doc?.sequence.length ||
      foldInsertAt(current) !== operation.up_to;
    if (stale) {
      finishFoldThroughBusy(busyToken);
      set({ foldDraft: null, alignDraft: null, errorMessage: STALE_DRAFT_MESSAGE });
      return;
    }
    const unavailable = foldThroughUnavailableMessage(current);
    if (unavailable) {
      finishFoldThroughBusy(busyToken);
      set({ errorMessage: unavailable });
      return;
    }
    const proposal = result.value.fold_through_proposal ?? null;
    if (proposal) {
      set({
        pendingFoldThrough: {
          proposal,
          operation,
          docEpoch: current.docEpoch,
          stepCount: current.doc.sequence.length,
        },
        foldDraft: null,
        alignDraft: null,
        foldThroughBusy: false,
      });
      return;
    }
    await applyFoldThrough(operation, false, busyToken);
  };

  const slice: DocumentSlice = {
    doc: null,
    stepCreases: [],
    faces: [],
    warnings: [],
    flatFoldViolations: [],
    violations: [],
    selection: EMPTY_SELECTION,
    hoveredHinge: null,
    activeTool: "select",
    measureDraft: emptyMeasureDraft(),
    foldDraft: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    alignDraft: null,
    techniqueDraft: null,
    construct: DEFAULT_CONSTRUCT,
    curve: DEFAULT_CURVE,
    errorMessage: null,
    documentSavedPath: null,
    docEpoch: 0,

    newDocument: (paper) => {
      invalidateFoldThrough();
      invalidateFoldAllEntry();
      return runViewCommand(() => ipc.documentNew(paper), true);
    },

    openDocument: (path) => {
      invalidateFoldThrough();
      invalidateFoldAllEntry();
      return runViewCommand(() => ipc.documentOpen(path), true);
    },

    saveDocument: async (path) => {
      await waitForFoldAllRestore();
      set({ documentSavedPath: null });
      await flushSoftSave();
      const result = await queue.run(() => ipc.documentSave(path));
      if (result.ok) {
        set({ errorMessage: null, documentSavedPath: path });
      } else {
        fail(result.error);
      }
    },

    applyEdit: async (op) => {
      if (get().foldAllPreview !== null) {
        if (!(await restoreAfterFoldAllPreview(false))) return;
      }
      stopPlayback();
      invalidateFoldThrough();
      const ops = Array.isArray(op) ? op : [op];
      if (ops.length === 0) return;
      const state = get();
      const doc = state.doc;
      const axis =
        state.mirrorDraw && doc
          ? activeMirrorLine(doc, state.mirrorAxis)
          : null;
      const withOpposite =
        axis && doc
          ? ops.map((one) =>
              one.type === "RemoveEdges" || one.type === "SetEdgeKind"
                ? { ...one, ids: withMirrorEdges(doc, one.ids, axis) }
                : one,
            )
          : ops;
      const mirrored =
        axis && onlyAddSegments(withOpposite)
          ? addSegmentsWithMirror(withOpposite, axis)
          : withOpposite;
      await applyDocChange(
        () =>
          mirrored.length === 1
            ? ipc.editApply(mirrored[0])
            : ipc.editApplyBatch(mirrored),
        mirrored.some((one) => one.type === "ReplaceCreasePattern"),
      );
    },

    drawSegment: async (a, b, kind) => {
      if (!get().doc) return;
      await get().applyEdit({ type: "AddSegment", a, b, kind });
    },

    drawCurve: async (points, kind) => {
      const state = get();
      if (!state.doc || points.length < 2) return;
      const axis = state.mirrorDraw
        ? activeMirrorLine(state.doc, state.mirrorAxis)
        : null;
      const ops: EditOp[] = [];
      const drawn = editableCopy(state.doc);
      const add = (a: Vec2, b: Vec2, edgeKind: EdgeKind): void => {
        for (const one of segmentEditsWithAxis(a, b, edgeKind, axis)) {
          ops.push(one);
          addWorkingEdge(drawn, one.a, one.b, one.kind);
        }
      };
      for (let index = 0; index + 1 < points.length; index++) {
        add(points[index], points[index + 1], kind);
      }
      if (state.curve.rulings && kind !== "Aux") {
        const long = Math.max(
          state.doc.paper.width_mm,
          state.doc.paper.height_mm,
        );
        const paper: Vec2 = [
          state.doc.paper.width_mm / long,
          state.doc.paper.height_mm / long,
        ];
        const opposite: EdgeKind =
          kind === "Mountain" ? "Valley" : "Mountain";
        for (const ruling of rulingLines(points, paper)) {
          for (const [to, edgeKind] of [
            [ruling.concave, opposite],
            [ruling.convex, kind],
          ] as [Vec2, EdgeKind][]) {
            add(
              ruling.at,
              firstCrossing(drawn, ruling.at, to),
              edgeKind,
            );
          }
        }
      }
      await get().applyEdit(ops);
    },

    setMirrorDraw: (on) => {
      set({ mirrorDraw: on });
      persistPrefs();
    },

    setMirrorAxisPreset: (preset) => {
      set({ mirrorAxis: { kind: preset }, mirrorAxisNotice: null });
      persistPrefs();
    },

    setSelectedLineAsMirrorAxis: () => {
      const state = get();
      if (!state.doc) return;
      const selected = [...new Set(state.selection.edgeIds)];
      if (
        selected.length !== 1 ||
        selectedEdgeMirrorLine(state.doc, selected[0]) === null
      ) {
        return;
      }
      set({
        mirrorAxis: { kind: "selectedLine", edgeId: selected[0] },
        mirrorAxisNotice: null,
      });
      persistPrefs();
    },

    setTool: (tool) => {
      if (get().foldAllPreview !== null) {
        void restoreAfterFoldAllPreview(false).then((restored) => {
          if (restored) get().setTool(tool);
        });
        return;
      }
      if (get().activeTool !== tool) {
        invalidateFoldThrough();
        set({
          activeTool: tool,
          selection: EMPTY_SELECTION,
          measureDraft: emptyMeasureDraft(),
          hoveredHinge: null,
          foldDraft: null,
          alignDraft: null,
          techniqueDraft: null,
          pullHinge: null,
          pullMirrorHinge: null,
          operationStage: 0,
          lineInputStart: null,
          paperActionTipVisible: false,
          paperActionTipExpanded: false,
        });
      }
    },

    setMeasureMode: (mode) => {
      const state = get();
      if (state.measureDraft.mode === mode) return;
      set({
        measureDraft: emptyMeasureDraft(mode),
        selection: EMPTY_SELECTION,
      });
    },

    setMeasureDisplay: (display) =>
      set((state) => ({
        measureDraft: { ...state.measureDraft, display },
      })),

    pickMeasureEdge: (edgeId) => {
      const state = get();
      if (
        state.activeTool !== "measure" ||
        state.measureDraft.mode === "distance" ||
        !state.doc?.cp.edges.some((edge) => edge.id === edgeId)
      ) {
        return;
      }
      const need = state.measureDraft.mode === "length" ? 1 : 2;
      const pick: MeasureEdgePick = { kind: "edge", edgeId };
      const previous = state.measureDraft.picks.filter(
        (candidate): candidate is MeasureEdgePick => candidate.kind === "edge",
      );
      const picks = previous.length >= need ? [pick] : [...previous, pick];
      set({
        measureDraft: { ...state.measureDraft, picks, display: null },
        selection: selectionForMeasure(picks),
      });
    },

    pickMeasurePoint: ({ cp, faceId, vertexId }) => {
      const state = get();
      if (
        state.activeTool !== "measure" ||
        state.measureDraft.mode !== "distance" ||
        !state.doc ||
        !Number.isFinite(cp[0]) ||
        !Number.isFinite(cp[1]) ||
        (faceId !== null && (!Number.isSafeInteger(faceId) || faceId < 0)) ||
        (vertexId !== null &&
          !state.doc.cp.vertices.some((vertex) => vertex.id === vertexId))
      ) {
        return;
      }
      const pick: MeasurePointPick = {
        kind: "point",
        cp: [cp[0], cp[1]],
        faceId,
        vertexId,
      };
      const previous = state.measureDraft.picks.filter(
        (candidate): candidate is MeasurePointPick => candidate.kind === "point",
      );
      const picks = previous.length >= 2 ? [pick] : [...previous, pick];
      set({
        measureDraft: { ...state.measureDraft, picks, display: null },
        selection: selectionForMeasure(picks),
      });
    },

    clearMeasurement: () =>
      set((state) => ({
        measureDraft: emptyMeasureDraft(state.measureDraft.mode),
        selection: EMPTY_SELECTION,
      })),

    setSelection: (selection) => {
      if (get().foldAllPreview !== null) return;
      set((state) => {
        const stillMoving =
          state.pullHinge !== null ||
          (state.activeAngleIntent?.hinges ?? []).some((hinge) =>
            selection.edgeIds.includes(hinge),
          );
        const dropActive = state.activeAngleIntent !== null && !stillMoving;
        return {
          selection,
          hoveredHinge:
            state.hoveredHinge !== null &&
            selection.edgeIds.includes(state.hoveredHinge)
              ? state.hoveredHinge
              : null,
          ...(dropActive ? { activeAngleIntent: null } : {}),
        };
      });
    },

    setHoveredHinge: (hinge) =>
      set((state) => ({
        hoveredHinge:
          hinge !== null &&
          state.hinges.has(hinge) &&
          (state.selection.edgeIds.includes(hinge) ||
            relaxationNotices(state.relaxations).some(
              (item) => item.hinge === hinge,
            ))
            ? hinge
            : null,
      })),

    beginFoldDraft: (line, source) => {
      const s = get();
      if (!s.doc || s.foldThroughBusy || s.pendingFoldThrough) return;
      if (source === "2d" && s.doc.sequence.length > 0) {
        set({
          errorMessage:
            "折る操作は3D画面から行ってください(展開図の位置と畳んだ紙の位置が食い違うため)",
        });
        return;
      }
      set({
        foldDraft: {
          line,
          direction: "Up",
          target: "all",
          movingSide: "right",
          docEpoch: s.docEpoch,
          stepCount: s.doc.sequence.length,
          upTo: foldInsertAt(s),
        },
        errorMessage: null,
        operationStage: 1,
      });
    },

    updateFoldDraft: (patch) => {
      const draft = get().foldDraft;
      if (draft) {
        set({
          foldDraft: { ...draft, ...patch },
          operationStage:
            patch.direction !== undefined ||
            patch.movingSide !== undefined ||
            patch.target !== undefined
              ? 2
              : get().operationStage,
        });
      }
    },

    cancelFoldDraft: () => {
      if (get().foldDraft || get().alignDraft)
        set({ foldDraft: null, alignDraft: null });
    },

    commitFoldDraft: async () => {
      const s = get();
      const draft = s.foldDraft;
      if (!draft || !s.doc) return;
      if (
        draft.docEpoch !== s.docEpoch ||
        draft.stepCount !== s.doc.sequence.length ||
        draft.upTo !== foldInsertAt(s)
      ) {
        set({
          foldDraft: null,
          alignDraft: null,
          errorMessage: STALE_DRAFT_MESSAGE,
        });
        return;
      }
      const unavailable = foldThroughUnavailableMessage(s);
      if (unavailable) {
        set({ errorMessage: unavailable });
        return;
      }
      const keep = keepSidePoint(draft.line, draft.movingSide);
      let targetLayers: number[] | null = null;
      if (draft.target === "top") {
        const layers = foldLayers(s.frame3d, s.doc, s.faces);
        const top = topMovingFace(layers, draft.line, keep);
        if (top === null) {
          set({
            errorMessage:
              "黄色で示した側に、折り返せる紙がありません。「反対側の紙を折り返す」を押して、もう一度試してください",
          });
          return;
        }
        targetLayers = [top];
      }
      const alignment =
        s.alignDraft && isAlignComplete(s.alignDraft)
          ? { mode: s.alignDraft.mode, picks: [...s.alignDraft.picks] }
          : null;
      const pose = foldPoseInputFromDrivers(s.drivers);
      if (!pose.ok) {
        set({ errorMessage: foldThroughUnavailableMessage(s) });
        return;
      }
      await requestFoldThrough({
        type: "FoldThrough",
        up_to: draft.upTo,
        line: draft.line,
        keep_side_point: keep,
        target_layers: targetLayers,
        direction: draft.direction,
        ...(pose.poseBefore ? { pose_before: pose.poseBefore } : {}),
        ...(alignment ? { alignment } : {}),
      });
    },

    resolveFoldThroughProposal: async (accept) => {
      const s = get();
      const pending = s.pendingFoldThrough;
      if (!pending || s.foldThroughBusy) return;
      if (
        !s.doc ||
        pending.docEpoch !== s.docEpoch ||
        pending.stepCount !== s.doc.sequence.length ||
        pending.operation.up_to !== foldInsertAt(s)
      ) {
        set({
          pendingFoldThrough: null,
          foldThroughBusy: false,
          errorMessage: STALE_DRAFT_MESSAGE,
        });
        return;
      }
      const unavailable = foldThroughUnavailableMessage(s);
      if (unavailable) {
        set({ errorMessage: unavailable });
        return;
      }
      const busyToken = foldThroughBusyGate.issue();
      set({ foldThroughBusy: true, errorMessage: null });
      await applyFoldThrough(pending.operation, accept, busyToken);
    },

    beginAlign: (mode) => {
      const s = get();
      if (!s.doc) return;
      invalidateFoldThrough();
      if (isStepReplayPending()) {
        set({
          errorMessage:
            "表示を切り替えています。切り替わってから、もう一度合わせ方を選んでください",
        });
        return;
      }
      if (s.alignDraft?.mode === mode) {
        set({ alignDraft: null, foldDraft: null });
        return;
      }
      set({
        activeTool: "fold",
        selection: EMPTY_SELECTION,
        foldDraft: null,
        techniqueDraft: null,
        alignDraft: {
          mode,
          picks: [],
          cpPicks: [],
          solutions: [],
          solutionIndex: 0,
          reason: null,
        },
        errorMessage: null,
      });
    },

    pickAlignTarget: (target, cursor = null, cpPick = null) => {
      const s = get();
      const draft = s.alignDraft;
      if (!draft || !s.doc) return;
      const steps = ALIGN_STEPS[draft.mode];
      const picks = isAlignComplete(draft) ? [target] : [...draft.picks, target];
      if (steps[picks.length - 1] !== target.kind) return;
      const previousCpPicks =
        draft.cpPicks ?? draft.picks.map((): AlignCpPick | null => null);
      const cpPicks = isAlignComplete(draft)
        ? [cpPick]
        : [...previousCpPicks, cpPick];
      const solved = solveAlign(draft.mode, picks, cursor);
      const line = solved.lines[0] ?? null;
      set({
        alignDraft: {
          mode: draft.mode,
          picks,
          cpPicks,
          solutions: solved.lines,
          solutionIndex: 0,
          reason: solved.reason,
        },
        foldDraft: line ? alignFoldDraft(s, line, picks) : null,
        errorMessage: null,
      });
    },

    nextAlignSolution: () => {
      const s = get();
      const draft = s.alignDraft;
      if (!draft || draft.solutions.length < 2) return;
      const index = (draft.solutionIndex + 1) % draft.solutions.length;
      const line = draft.solutions[index];
      set({
        alignDraft: { ...draft, solutionIndex: index },
        foldDraft: s.foldDraft
          ? {
              ...s.foldDraft,
              line,
              movingSide: initialMovingSide(line, draft.picks[0]),
            }
          : alignFoldDraft(s, line, draft.picks),
      });
    },

    undoAlignPick: () => {
      const draft = get().alignDraft;
      if (!draft || draft.picks.length === 0) return;
      set({
        alignDraft: {
          ...draft,
          picks: draft.picks.slice(0, -1),
          cpPicks: draft.cpPicks?.slice(0, -1),
          solutions: [],
          solutionIndex: 0,
          reason: null,
        },
        foldDraft: null,
      });
    },

    cancelAlign: () => {
      if (get().alignDraft) set({ alignDraft: null, foldDraft: null });
    },

    foldByDrag: async (
      from,
      to,
      mode,
      grabFace = null,
      direction = "Up",
    ) => {
      const s = get();
      if (
        s.foldAllPreview !== null ||
        !s.doc ||
        s.foldThroughBusy ||
        s.pendingFoldThrough
      ) {
        return;
      }
      const reason = foldBlockReason(
        {
          hasDoc: true,
          playing: s.playing,
          playT: s.playT,
          driverAngles: [...s.drivers.values()],
          currentStep: s.currentStep,
          stepCount: s.doc.sequence.length,
        },
        true,
      );
      if (reason) {
        set({ errorMessage: reason });
        return;
      }
      const pose = foldPoseInputFromDrivers(s.drivers);
      if (!pose.ok) {
        set({ errorMessage: foldThroughUnavailableMessage(s) });
        return;
      }
      const upTo = foldInsertAt(s);
      if (isSpatialFoldFrame(s.frame3d)) {
        const spatial: SpatialFoldDrag = {
          from: [from[0], from[1], from[2] ?? 0],
          to: [to[0], to[1], to[2] ?? 0],
          grab_face: grabFace ?? s.frame3d!.faces[0]?.face ?? 0,
          mode,
        };
        set({
          foldDraft: null,
          alignDraft: null,
        });
        await requestFoldThrough({
          type: "FoldThrough",
          up_to: upTo,
          line: [
            [0, 0],
            [1, 0],
          ],
          keep_side_point: [0, 1],
          target_layers: null,
          direction,
          ...(pose.poseBefore ? { pose_before: pose.poseBefore } : {}),
          spatial,
        });
        return;
      }
      const result = planGrabFold(
        foldLayers(s.frame3d, s.doc, s.faces),
        s.faces,
        [from[0], from[1]],
        [to[0], to[1]],
        mode,
        grabFace,
      );
      if (!result.ok) {
        set({ errorMessage: result.error });
        return;
      }
      set({
        foldDraft: null,
        alignDraft: null,
      });
      await requestFoldThrough({
        type: "FoldThrough",
        up_to: upTo,
        line: result.plan.line,
        keep_side_point: result.plan.keepSidePoint,
        target_layers: result.plan.targetLayers,
        direction: "Up",
        ...(pose.poseBefore ? { pose_before: pose.poseBefore } : {}),
      });
    },

    beginTechnique: (kind) => {
      const s = get();
      if (!s.doc) return;
      invalidateFoldThrough();
      set({
        activeTool: "technique",
        selection: EMPTY_SELECTION,
        foldDraft: null,
        alignDraft: null,
        techniqueDraft: {
          kind,
          flap: [],
          flapCandidates: [],
          flapPickCount: 1,
          line: null,
          movingSide: "right",
          widthMm: DEFAULT_PLEAT_WIDTH_MM,
          polygon: [],
          center: null,
          referencePoint: null,
          twistDeg: DEFAULT_TWIST_DEG,
          openToBack: false,
          motionMode: "reflect",
          motionTurn: "Keep",
          motionDirection: "Up",
          motionAnchor: 0,
          motionReverseLayers: false,
          motionAxisEdgeId: null,
          motionParts: [],
          docEpoch: s.docEpoch,
          stepCount: s.doc.sequence.length,
          upTo: foldInsertAt(s),
        },
        errorMessage: null,
      });
    },

    setTechniqueFlap: (faces) => {
      const draft = get().techniqueDraft;
      if (!draft) return;
      const candidates = [...new Set(faces)];
      set({
        techniqueDraft: {
          ...draft,
          flap: candidates,
          flapCandidates: candidates,
          flapPickCount: clampTechniqueLayerCount(
            draft.flapPickCount,
            candidates.length,
          ),
        },
      });
    },

    setTechniqueFlapPreset: (preset) => {
      const draft = get().techniqueDraft;
      if (!draft) return;
      set({
        techniqueDraft: {
          ...draft,
          flap: techniqueFlapForPreset(
            draft.flapCandidates,
            preset,
            draft.flapPickCount,
          ),
        },
      });
    },

    toggleTechniqueFlap: (face) => {
      const draft = get().techniqueDraft;
      if (!draft) return;
      set({
        techniqueDraft: {
          ...draft,
          flap: toggleTechniqueFlapSelection(
            draft.flapCandidates,
            draft.flap,
            face,
          ),
        },
      });
    },

    setTechniqueLine: (line) => {
      const draft = get().techniqueDraft;
      if (draft) {
        set({
          techniqueDraft: {
            ...draft,
            line,
            motionAxisEdgeId:
              draft.kind === "Simple" ? null : draft.motionAxisEdgeId,
          },
        });
      }
    },

    setLayerMotionAxis: (edgeId, line) => {
      const draft = get().techniqueDraft;
      if (!draft || draft.kind !== "Simple") return;
      set({
        techniqueDraft: {
          ...draft,
          line,
          motionAxisEdgeId: edgeId,
          motionMode: "reflect",
        },
        errorMessage: null,
      });
    },

    addLayerMotionPart: () => {
      const draft = get().techniqueDraft;
      if (!draft || draft.kind !== "Simple") return;
      if (draft.motionMode === "reflect" && draft.motionAxisEdgeId === null) {
        set({
          errorMessage:
            "立体表示で既存の折り目をクリックして、正確な開閉軸を選んでください",
        });
        return;
      }
      const built = buildLayerMotionPart(layerMotionPartDraft(draft));
      if (!built.ok) {
        set({ errorMessage: built.error });
        return;
      }
      set({
        techniqueDraft: {
          ...clearCurrentLayerMotion(draft),
          motionParts: [...draft.motionParts, built.part],
        },
        errorMessage: null,
        operationStage: 1,
      });
    },

    undoLayerMotionPart: () => {
      const draft = get().techniqueDraft;
      if (
        !draft ||
        draft.kind !== "Simple" ||
        draft.motionParts.length === 0
      ) {
        return;
      }
      set({
        techniqueDraft: {
          ...draft,
          motionParts: draft.motionParts.slice(0, -1),
        },
        errorMessage: null,
      });
    },

    addTechniqueVertex: (point) => {
      const draft = get().techniqueDraft;
      if (draft) {
        set({
          techniqueDraft: {
            ...draft,
            polygon: addTwistVertex(draft.polygon, point),
          },
        });
      }
    },

    undoTechniqueVertex: () => {
      const draft = get().techniqueDraft;
      if (draft && draft.polygon.length > 0) {
        set({
          techniqueDraft: {
            ...draft,
            polygon: undoTwistVertex(draft.polygon),
          },
        });
      }
    },

    setTechniqueCenter: (point) => {
      const draft = get().techniqueDraft;
      if (draft) set({ techniqueDraft: { ...draft, center: point } });
    },

    setTechniqueReferencePoint: (point) => {
      const draft = get().techniqueDraft;
      if (draft && draft.kind !== "Simple" && draft.kind !== "Twist") {
        set({ techniqueDraft: { ...draft, referencePoint: point } });
      }
    },

    updateTechniqueDraft: (patch) => {
      const draft = get().techniqueDraft;
      if (draft) set({ techniqueDraft: { ...draft, ...patch } });
    },

    setConstruct: (patch) =>
      set((state) => ({ construct: { ...state.construct, ...patch } })),

    setCurve: (patch) =>
      set((state) => ({ curve: { ...state.curve, ...patch } })),

    cancelTechnique: () => {
      if (get().techniqueDraft) set({ techniqueDraft: null });
    },

    commitTechnique: async () => {
      const s = get();
      const draft = s.techniqueDraft;
      if (!draft || !s.doc) return;
      if (draft.kind === "Simple") {
        if (
          draft.docEpoch !== s.docEpoch ||
          draft.stepCount !== s.doc.sequence.length ||
          draft.upTo !== foldInsertAt(s)
        ) {
          set({ techniqueDraft: null, errorMessage: STALE_DRAFT_MESSAGE });
          return;
        }
        const unavailable = foldUnavailableMessage(s);
        if (unavailable) {
          set({ errorMessage: unavailable });
          return;
        }
        clearZeroOnlyDrivers();
        const parts = [...draft.motionParts];
        const current = layerMotionPartDraft(draft);
        if (hasLayerMotionInput(current)) {
          if (
            draft.motionMode === "reflect" &&
            draft.motionAxisEdgeId === null
          ) {
            set({
              errorMessage:
                "立体表示で既存の折り目をクリックして、正確な開閉軸を選んでください",
            });
            return;
          }
          const built = buildLayerMotionPart(current);
          if (!built.ok) {
            set({ errorMessage: built.error });
            return;
          }
          parts.push(built.part);
        }
        if (parts.length === 0) {
          set({
            errorMessage:
              "既存折り目と対象層を選ぶか、重ね方・山谷反転を指定してください",
          });
          return;
        }
        set({
          currentStep:
            draft.upTo === s.doc.sequence.length ? null : draft.upTo + 1,
        });
        await get().applySequenceOp({
          type: "FlatMotion",
          up_to: draft.upTo,
          parts,
          kind: "Simple",
        });
        if (get().errorMessage === null) set({ techniqueDraft: null });
        return;
      }
      const byPolygon =
        draft.kind === "Twist" && isTwistPolygonReady(draft.polygon);
      if (!draft.line && !byPolygon) {
        set({
          errorMessage:
            draft.kind === "Twist"
              ? "中央の形が決まっていません。立体表示で角を3つ以上クリックしてください"
              : "折り線がありません。立体表示の紙の上をドラッグして折り線を引いてください",
        });
        return;
      }
      if (
        draft.docEpoch !== s.docEpoch ||
        draft.stepCount !== s.doc.sequence.length ||
        draft.upTo !== foldInsertAt(s)
      ) {
        set({ techniqueDraft: null, errorMessage: STALE_DRAFT_MESSAGE });
        return;
      }
      const unavailable = foldUnavailableMessage(s);
      if (unavailable) {
        set({ errorMessage: unavailable });
        return;
      }
      clearZeroOnlyDrivers();
      const minimumFlap = minimumTechniqueFlap(draft.kind);
      if (draft.flap.length < minimumFlap) {
        set({
          errorMessage:
            `先に立体表示で紙をクリックし、対象の層(フラップ)を${minimumFlap}枚以上選んでください`,
        });
        return;
      }
      const scale = Math.max(
        s.doc.paper.width_mm,
        s.doc.paper.height_mm,
      );
      const twistCenter = byPolygon
        ? (draft.center ?? polygonCentroid(draft.polygon))
        : null;
      const twistRef = twistCenter
        ? twistReferencePoint(
            draft.polygon,
            twistCenter,
            draft.movingSide === "right" ? draft.twistDeg : -draft.twistDeg,
          )
        : null;
      const line: [Vec2, Vec2] =
        draft.line ?? [draft.polygon[0], draft.polygon[1]];
      const lineLength = Math.hypot(
        line[1][0] - line[0][0],
        line[1][1] - line[0][1],
      );
      const automaticReference =
        draft.kind === "Pleat"
          ? offsetPoint(line, draft.movingSide, draft.widthMm / scale)
          : draft.kind === "OpenSink"
            ? offsetPoint(
                line,
                draft.movingSide,
                Math.max(0.01, lineLength * 0.25),
              )
            : keepSidePoint(line, draft.movingSide);
      const reference =
        twistRef ?? draft.referencePoint ?? automaticReference;
      set({
        currentStep:
          draft.upTo === s.doc.sequence.length ? null : draft.upTo + 1,
      });
      await get().applySequenceOp({
        type: "Technique",
        up_to: draft.upTo,
        kind: draft.kind,
        flap: draft.flap,
        line,
        reference_point: reference,
        ...(techniqueUsesOpenToBack(draft.kind)
          ? { open_to_back: draft.openToBack }
          : {}),
        ...(byPolygon && twistCenter
          ? { polygon: draft.polygon, center: twistCenter }
          : {}),
      });
      const error = get().errorMessage;
      if (error === null) {
        set({ techniqueDraft: null });
      } else if (!error.includes(TECHNIQUE_FALLBACK_HINT)) {
        set({ errorMessage: `${error}(${TECHNIQUE_FALLBACK_HINT})` });
      }
    },
  };

  return {
    slice,
    internals: { invalidateFoldThrough },
  };
}
