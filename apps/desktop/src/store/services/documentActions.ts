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
  type FoldLine,
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
  FoldPoseInput,
  FoldTargetInfo,
  SeqOp,
  Vec2,
} from "../../lib/types";
import type {
  SpatialAlignTarget,
  SpatialFoldTarget,
} from "../../lib/spatialAlignTypes";
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
  type FoldDraft,
  type FoldDraftPatch,
  type FoldTargetSelection,
  type FoldThroughApplyOp,
  type FoldThroughOperation,
  type MeasureEdgePick,
  type MeasurePointPick,
  type SpatialFoldDrag,
  type SpatialAlignPickResult,
  type SpatialMaterialFoldInput,
  type SpatialMaterialForMovingSide,
} from "../slices/documentSlice";

interface FoldThroughCoordinateInput {
  line: FoldLine;
  keepSidePoint: Vec2;
}

type SpatialFoldRoute =
  | { kind: "legacy" | "folded"; input: FoldThroughCoordinateInput | null }
  | { kind: "material"; input: SpatialMaterialFoldInput | null };

type MaterialPoseInputResult =
  | { ok: true; poseBefore: FoldPoseInput | null }
  | { ok: false };

function finiteVec2(point: Vec2): boolean {
  return Number.isFinite(point[0]) && Number.isFinite(point[1]);
}

function validSpatialMaterialInput(
  input: SpatialMaterialFoldInput | null | undefined,
): input is SpatialMaterialFoldInput {
  if (
    !input ||
    !finiteVec2(input.materialLine[0]) ||
    !finiteVec2(input.materialLine[1]) ||
    !finiteVec2(input.materialKeepSidePoint)
  ) {
    return false;
  }
  return (
    input.materialLine[0][0] !== input.materialLine[1][0] ||
    input.materialLine[0][1] !== input.materialLine[1][1]
  );
}

function validFoldThroughCoordinateInput(
  input: FoldThroughCoordinateInput | null,
): input is FoldThroughCoordinateInput {
  return (
    input !== null &&
    finiteVec2(input.line[0]) &&
    finiteVec2(input.line[1]) &&
    finiteVec2(input.keepSidePoint) &&
    (input.line[0][0] !== input.line[1][0] ||
      input.line[0][1] !== input.line[1][1])
  );
}

function finiteVec2Value(value: unknown): value is Vec2 {
  return (
    Array.isArray(value) &&
    value.length === 2 &&
    typeof value[0] === "number" &&
    Number.isFinite(value[0]) &&
    typeof value[1] === "number" &&
    Number.isFinite(value[1])
  );
}

function foldedCoordinateInput(
  value: unknown,
  movingSide: "left" | "right",
): FoldThroughCoordinateInput | null {
  if (typeof value !== "object" || value === null) return null;
  if (!("line" in value) || !Array.isArray(value.line) || value.line.length !== 2) {
    return null;
  }
  const [a, b] = value.line;
  if (!finiteVec2Value(a) || !finiteVec2Value(b)) return null;
  if (!("keepPointForMovingSide" in value)) return null;
  const keepBySide = value.keepPointForMovingSide;
  if (typeof keepBySide !== "object" || keepBySide === null) return null;
  const keep =
    movingSide === "left"
      ? "left" in keepBySide
        ? keepBySide.left
        : null
      : "right" in keepBySide
        ? keepBySide.right
        : null;
  if (!finiteVec2Value(keep)) return null;
  const input: FoldThroughCoordinateInput = {
    line: [a, b],
    keepSidePoint: keep,
  };
  return validFoldThroughCoordinateInput(input) ? input : null;
}

function spatialFoldRoute(draft: FoldDraft): SpatialFoldRoute {
  if (!Object.prototype.hasOwnProperty.call(draft, "spatialTarget")) {
    return {
      kind: "legacy",
      input: {
        line: draft.line,
        keepSidePoint: keepSidePoint(draft.line, draft.movingSide),
      },
    };
  }
  const spatialTarget = draft.spatialTarget;
  if (spatialTarget === null || spatialTarget === undefined) {
    return { kind: "material", input: null };
  }
  const folded: unknown = spatialTarget.foldedPlane;
  if (folded !== null && folded !== undefined) {
    return {
      kind: "folded",
      input: foldedCoordinateInput(folded, draft.movingSide),
    };
  }
  const input = draft.spatialMaterialForMovingSide?.[draft.movingSide] ?? null;
  return {
    kind: "material",
    input: validSpatialMaterialInput(input) ? input : null,
  };
}

/** 材料経路だけが使う、利用者のsigned宣言を丸めず保存するPose入力。 */
function materialPoseInputFromDrivers(
  drivers: ReadonlyMap<number, number>,
): MaterialPoseInputResult {
  const entries = [...drivers].sort(([left], [right]) => left - right);
  if (
    entries.some(
      ([edge, angle]) =>
        !Number.isSafeInteger(edge) || edge < 0 || !Number.isFinite(angle),
    )
  ) {
    return { ok: false };
  }
  return {
    ok: true,
    poseBefore:
      entries.length === 0
        ? null
        : {
            drivers: entries.map(([edge_id, target_angle_deg]) => ({
              edge_id,
              target_angle_deg,
            })),
          },
  };
}

function sameFoldThroughCoordinateInput(
  a: FoldThroughCoordinateInput | null,
  b: FoldThroughCoordinateInput | null,
): boolean {
  if (a === null || b === null) return a === b;
  return (
    a.line[0][0] === b.line[0][0] &&
    a.line[0][1] === b.line[0][1] &&
    a.line[1][0] === b.line[1][0] &&
    a.line[1][1] === b.line[1][1] &&
    a.keepSidePoint[0] === b.keepSidePoint[0] &&
    a.keepSidePoint[1] === b.keepSidePoint[1]
  );
}

function sameSpatialMaterialInput(
  a: SpatialMaterialFoldInput | null,
  b: SpatialMaterialFoldInput | null,
): boolean {
  if (a === null || b === null) return a === b;
  return (
    a.materialLine[0][0] === b.materialLine[0][0] &&
    a.materialLine[0][1] === b.materialLine[0][1] &&
    a.materialLine[1][0] === b.materialLine[1][0] &&
    a.materialLine[1][1] === b.materialLine[1][1] &&
    a.materialKeepSidePoint[0] === b.materialKeepSidePoint[0] &&
    a.materialKeepSidePoint[1] === b.materialKeepSidePoint[1]
  );
}

function sameSpatialFoldRoute(left: FoldDraft, right: FoldDraft): boolean {
  const a = spatialFoldRoute(left);
  const b = spatialFoldRoute(right);
  if (a.kind !== b.kind) return false;
  if (a.kind === "material" && b.kind === "material") {
    return sameSpatialMaterialInput(a.input, b.input);
  }
  if (a.kind === "material" || b.kind === "material") return false;
  return sameFoldThroughCoordinateInput(a.input, b.input);
}

interface NormalizedSpatialAlignResult {
  solutions: (SpatialFoldTarget | null)[];
  materialSolutions: (SpatialMaterialForMovingSide | null)[];
  lines: FoldLine[];
  solutionIndices: number[];
  reason: string | null;
}

function unavailableSpatialSolutions(count: number): null[] {
  return Array.from({ length: count }, () => null);
}

function sameMaterialLine(left: FoldLine, right: FoldLine): boolean {
  return (
    left[0][0] === right[0][0] &&
    left[0][1] === right[0][1] &&
    left[1][0] === right[1][0] &&
    left[1][1] === right[1][1]
  );
}

function materialLineOf(
  material: SpatialMaterialForMovingSide | null,
): FoldLine | null {
  if (material === null) return null;
  if (
    (material.left !== null && !validSpatialMaterialInput(material.left)) ||
    (material.right !== null && !validSpatialMaterialInput(material.right))
  ) {
    return null;
  }
  const left = material.left?.materialLine ?? null;
  const right = material.right?.materialLine ?? null;
  if (left !== null && right !== null && !sameMaterialLine(left, right)) {
    return null;
  }
  const line = left ?? right;
  return line === null
    ? null
    : [
        [...line[0]],
        [...line[1]],
      ];
}

function spatialMovingSide(
  material: SpatialMaterialForMovingSide | null | undefined,
  preferred: FoldDraft["movingSide"] = "right",
): FoldDraft["movingSide"] {
  if (validSpatialMaterialInput(material?.[preferred])) return preferred;
  const other = preferred === "left" ? "right" : "left";
  return validSpatialMaterialInput(material?.[other]) ? other : preferred;
}

/**
 * 3D solverが返した配列長とindexだけを正本にする。材料配列の欠落・長さ違い・
 * left/right不一致を並べ替えたり別解で補ったりせず、そのindexをnullにする。
 */
function normalizeSpatialAlignResult(
  update: SpatialAlignPickResult | undefined,
  spatialPicks: readonly (SpatialAlignTarget | null)[],
): NormalizedSpatialAlignResult {
  const solutionCount = update?.solutions.length ?? 0;
  const unavailable = unavailableSpatialSolutions(solutionCount);
  const picksAreComplete = spatialPicks.every((pick) => pick !== null);
  const providedMaterials = update?.materialSolutions;
  const solutions = picksAreComplete ? [...(update?.solutions ?? [])] : [...unavailable];
  const materialSolutions =
    picksAreComplete && providedMaterials?.length === solutionCount
      ? [...providedMaterials]
      : [...unavailable];
  const lines: FoldLine[] = [];
  const solutionIndices: number[] = [];
  materialSolutions.forEach((material, index) => {
    if (solutions[index] === null) return;
    const line = materialLineOf(material);
    if (line === null) return;
    lines.push(line);
    solutionIndices.push(index);
  });
  return {
    solutions,
    materialSolutions,
    lines,
    solutionIndices,
    reason: update?.reason ?? null,
  };
}

function withSpatialFoldSelection(
  draft: FoldDraft | null,
  spatialSolutions: readonly (SpatialFoldTarget | null)[] | undefined,
  materialSolutions: readonly (SpatialMaterialForMovingSide | null)[] | undefined,
  index: number,
): FoldDraft | null {
  if (
    draft === null ||
    spatialSolutions === undefined ||
    spatialSolutions[index] == null
  ) {
    return spatialSolutions === undefined ? draft : null;
  }
  return {
    ...draft,
    spatialTarget: spatialSolutions[index] ?? null,
    spatialMaterialForMovingSide: materialSolutions?.[index] ?? null,
  };
}

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
  const foldTargetGate = createGenerationGate();

  const invalidateFoldThrough = (): void => {
    foldThroughGate.issue();
    foldTargetGate.issue();
    if (get().pendingFoldThrough !== null) set({ pendingFoldThrough: null });
  };

  const finishFoldThroughBusy = (token: number): void => {
    if (foldThroughBusyGate.isCurrent(token) && get().foldThroughBusy) {
      set({ foldThroughBusy: false });
    }
  };

  const unavailableFoldTargetInfo = (reason: string | null = null): FoldTargetInfo => ({
    status: "unavailable",
    availableCount: null,
    reason,
    topAction: null,
  });

  const sameFoldTargetQuery = (
    current: DocumentSliceHostState["foldDraft"],
    started: NonNullable<DocumentSliceHostState["foldDraft"]>,
  ): current is NonNullable<DocumentSliceHostState["foldDraft"]> =>
    current !== null &&
    current.docEpoch === started.docEpoch &&
    current.stepCount === started.stepCount &&
    current.upTo === started.upTo &&
    current.movingSide === started.movingSide &&
    current.line[0][0] === started.line[0][0] &&
    current.line[0][1] === started.line[0][1] &&
    current.line[1][0] === started.line[1][0] &&
    current.line[1][1] === started.line[1][1] &&
    sameSpatialFoldRoute(current, started);

  const loadFoldTargetInfo = async (): Promise<void> => {
    const startedState = get();
    const started = startedState.foldDraft;
    if (
      !startedState.doc ||
      !started ||
      started.foldTargetBusy === true ||
      started.foldTargetInfo != null
    ) {
      return;
    }

    const revision = foldTargetGate.issue();
    set((state) =>
      sameFoldTargetQuery(state.foldDraft, started)
        ? { foldDraft: { ...state.foldDraft, foldTargetBusy: true } }
        : {},
    );

    const foldRoute = spatialFoldRoute(started);
    const pose = foldRoute.kind === "material"
      ? materialPoseInputFromDrivers(startedState.drivers)
      : foldPoseInputFromDrivers(startedState.drivers);
    if (foldRoute.input === null || !pose.ok) {
      if (!foldTargetGate.isCurrent(revision)) return;
      set((state) =>
        sameFoldTargetQuery(state.foldDraft, started)
          ? {
              foldDraft: {
                ...state.foldDraft,
                foldTargetBusy: false,
                foldTargetInfo: unavailableFoldTargetInfo(),
              },
            }
          : {},
      );
      return;
    }

    let operation: SeqOp;
    if (foldRoute.kind === "material") {
      const input = foldRoute.input;
      if (input === null) return;
      operation = {
        type: "PreviewFoldTargetsOnMaterial",
        up_to: started.upTo,
        material_line: input.materialLine,
        material_keep_side_point: input.materialKeepSidePoint,
        ...(pose.poseBefore ? { pose_before: pose.poseBefore } : {}),
      };
    } else {
      const input = foldRoute.input;
      if (input === null) return;
      operation = {
        type: "PreviewFoldTargets",
        up_to: started.upTo,
        line: input.line,
        keep_side_point: input.keepSidePoint,
        ...(pose.poseBefore ? { pose_before: pose.poseBefore } : {}),
      };
    }
    const result = await queue.run(() => ipc.sequenceApply(operation));
    if (!foldTargetGate.isCurrent(revision)) return;

    const info = result.ok
      ? (result.value.fold_target_info ?? unavailableFoldTargetInfo())
      : unavailableFoldTargetInfo();
    set((state) =>
      sameFoldTargetQuery(state.foldDraft, started)
        ? {
            foldDraft: {
              ...state.foldDraft,
              foldTargetBusy: false,
              foldTargetInfo: info,
            },
          }
        : {},
    );
  };

  const clearFoldTargetQuery = (draft: FoldDraft): FoldDraft => {
    const {
      foldTargetInfo: _oldInfo,
      foldTargetBusy: _oldBusy,
      ...withoutQuery
    } = draft;
    void _oldInfo;
    void _oldBusy;
    return withoutQuery as FoldDraft;
  };

  const patchFoldDraft = (draft: FoldDraft, patch: FoldDraftPatch): FoldDraft => {
    const sideChanged =
      patch.movingSide !== undefined && patch.movingSide !== draft.movingSide;
    const base = sideChanged ? clearFoldTargetQuery(draft) : draft;
    if (patch.target === "topPleats") {
      return { ...base, ...patch } as FoldDraft;
    }
    if (patch.target === "all" || patch.target === "top") {
      const { topPleatCount: _oldCount, ...withoutPleatCount } = base;
      void _oldCount;
      return { ...withoutPleatCount, ...patch } as FoldDraft;
    }
    return { ...base, ...patch } as FoldDraft;
  };

  const selectFoldTarget = (
    draft: FoldDraft,
    selection: FoldTargetSelection,
  ): FoldDraft => patchFoldDraft(draft, selection);

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

  const materialPoseBeforeMatchesDrivers = (
    operation: Extract<SeqOp, { type: "CreaseOnlyTop" }>,
    drivers: ReadonlyMap<number, number>,
  ): boolean => {
    const expected = operation.pose_before ?? null;
    const current = materialPoseInputFromDrivers(drivers);
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

  const applyCreaseOnlyTop = async (
    operation: Extract<SeqOp, { type: "CreaseOnlyTop" }>,
  ): Promise<void> => {
    const state = get();
    if (
      state.foldAllPreview !== null ||
      state.foldThroughBusy ||
      state.pendingFoldThrough ||
      !state.doc
    ) {
      return;
    }
    stopPlayback();
    foldThroughGate.issue();
    foldTargetGate.issue();
    const busyToken = foldThroughBusyGate.issue();
    const beforeEpoch = state.docEpoch;
    const beforeSequenceCount = state.doc.sequence.length;
    set({
      pendingFoldThrough: null,
      foldThroughBusy: true,
      errorMessage: null,
      currentStep:
        operation.up_to === state.doc.sequence.length
          ? null
          : operation.up_to + (operation.pose_before ? 2 : 1),
    });
    try {
      await applyDocChange(() => ipc.sequenceApply(operation));
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
        materialPoseBeforeMatchesDrivers(operation, completed.drivers)
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
        ...(operation.target_pleat_count != null
          ? { target_pleat_count: operation.target_pleat_count }
          : {}),
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
    foldIssues: [],
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
        const sideChanged =
          patch.movingSide !== undefined &&
          patch.movingSide !== draft.movingSide;
        if (sideChanged) foldTargetGate.issue();
        set({
          foldDraft: patchFoldDraft(draft, patch),
          operationStage:
            patch.direction !== undefined ||
            patch.movingSide !== undefined ||
            patch.target !== undefined
              ? 2
              : get().operationStage,
        });
      }
    },

    setFoldTarget: (selection) => {
      const draft = get().foldDraft;
      if (!draft) return;
      set({
        foldDraft: selectFoldTarget(draft, selection),
        operationStage: 2,
        errorMessage: null,
      });
    },

    requestFoldTargetInfo: loadFoldTargetInfo,

    cancelFoldDraft: () => {
      if (get().foldDraft || get().alignDraft) {
        foldTargetGate.issue();
        set({ foldDraft: null, alignDraft: null });
      }
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
      const foldRoute = spatialFoldRoute(draft);
      if (foldRoute.kind === "material") {
        const unavailable = foldBlockReason({
          hasDoc: true,
          playing: s.playing,
          playT: s.playT,
          driverAngles: [],
          currentStep: s.currentStep,
          stepCount: s.doc.sequence.length,
        });
        if (unavailable) {
          set({ errorMessage: unavailable });
          return;
        }
        if (foldRoute.input === null) {
          set({
            errorMessage:
              draft.foldTargetInfo?.reason ??
              "3Dの折り線を材料上の1本の線へ戻せないため、変更しませんでした。",
          });
          return;
        }
        if (
          draft.foldTargetInfo?.status !== "crease_only_top" ||
          draft.foldTargetInfo.topAction !== "crease_only_top"
        ) {
          set({
            errorMessage:
              draft.foldTargetInfo?.reason ??
              "この3Dの折り線は、最上紙へ折り目だけを付ける操作として確定できません。",
          });
          return;
        }
        const pose = materialPoseInputFromDrivers(s.drivers);
        if (!pose.ok) {
          set({
            errorMessage:
              "利用者が指定した折り角度をそのまま保存できないため、変更しませんでした。",
          });
          return;
        }
        const alignment =
          s.alignDraft && isAlignComplete(s.alignDraft)
            ? { mode: s.alignDraft.mode, picks: [...s.alignDraft.picks] }
            : null;
        await applyCreaseOnlyTop({
          type: "CreaseOnlyTop",
          up_to: draft.upTo,
          material_line: foldRoute.input.materialLine,
          material_keep_side_point: foldRoute.input.materialKeepSidePoint,
          direction: draft.direction,
          ...(pose.poseBefore ? { pose_before: pose.poseBefore } : {}),
          alignment,
        });
        return;
      }
      if (foldRoute.input === null) {
        set({
          errorMessage:
            "3Dの折り線を畳み平面上の1本の線として証明できないため、変更しませんでした。",
        });
        return;
      }
      const unavailable = foldThroughUnavailableMessage(s);
      if (unavailable) {
        set({ errorMessage: unavailable });
        return;
      }
      if (draft.foldTargetInfo?.status === "crease_only_top") {
        // 現行wireは照会結果だけで、crease-onlyを確定する入力をまだ持たない。
        // K=0や既存target_layersへ代用すると別の折りになるため送信しない。
        set({
          errorMessage:
            draft.foldTargetInfo.reason ??
            "いちばん上の紙に折り目だけを付ける処理は、まだ確定できません。",
        });
        return;
      }
      const foldLine = foldRoute.input.line;
      const keep = foldRoute.input.keepSidePoint;
      let targetLayers: number[] | null = null;
      let targetPleatCount: number | null = null;
      if (draft.target === "top") {
        const layers = foldLayers(s.frame3d, s.doc, s.faces);
        const top = topMovingFace(layers, foldLine, keep);
        if (top === null) {
          set({
            errorMessage:
              "黄色で示した側に、折り返せる紙がありません。「反対側の紙を折り返す」を押して、もう一度試してください",
          });
          return;
        }
        targetLayers = [top];
      } else if (draft.target === "topPleats") {
        const status = draft.foldTargetInfo?.status ?? null;
        if (
          status !== null &&
          status !== "ready" &&
          status !== "limited"
        ) {
          set({
            errorMessage:
              draft.foldTargetInfo?.reason ??
              "この折り線で同時に折れるひだを確認できません。",
          });
          return;
        }
        const available = draft.foldTargetInfo?.availableCount ?? null;
        if (
          !Number.isSafeInteger(draft.topPleatCount) ||
          draft.topPleatCount < 1 ||
          (available !== null && draft.topPleatCount > available)
        ) {
          set({
            errorMessage:
              available !== null
                ? `選んだ${draft.topPleatCount}枚は、今同時に折れる${available}枚を超えています。1枚から${available}枚までで選び直してください。`
                : "同時に折るひだの枚数を1枚以上で選び直してください。",
          });
          return;
        }
        targetPleatCount = draft.topPleatCount;
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
        line: foldLine,
        keep_side_point: keep,
        target_layers: targetLayers,
        // Kはひだ数であって面数ではない。selectedLayerCount、2 * K、
        // Face ID順、surface_rankから対象面を推測せず、RustへKだけを渡す。
        ...(targetPleatCount !== null
          ? { target_pleat_count: targetPleatCount }
          : {}),
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

    pickAlignTarget: (target, cursor = null, cpPick = null, spatial) => {
      const s = get();
      const draft = s.alignDraft;
      if (!draft || !s.doc) return;
      const steps = ALIGN_STEPS[draft.mode];
      const restarting = isAlignComplete(draft);
      const picks = restarting ? [target] : [...draft.picks, target];
      if (steps[picks.length - 1] !== target.kind) return;
      const previousCpPicks =
        draft.cpPicks ?? draft.picks.map((): AlignCpPick | null => null);
      const cpPicks = restarting
        ? [cpPick]
        : [...previousCpPicks, cpPick];
      const continuesSpatialCycle = !restarting && draft.spatialPicks !== undefined;
      const usesSpatialCycle = spatial !== undefined || continuesSpatialCycle;
      const previousSpatialPicks = restarting
        ? []
        : (draft.spatialPicks ??
          draft.picks.map((): SpatialAlignTarget | null => null));
      const currentSpatialTarget =
        spatial?.target?.kind === target.kind ? spatial.target : null;
      const spatialPicks = usesSpatialCycle
        ? [...previousSpatialPicks, currentSpatialTarget]
        : undefined;
      const normalizedSpatial = spatialPicks
        ? normalizeSpatialAlignResult(spatial, spatialPicks)
        : null;
      const solved = usesSpatialCycle ? null : solveAlign(draft.mode, picks, cursor);
      const solutions = solved?.lines ?? normalizedSpatial?.lines ?? [];
      const line = solutions[0] ?? null;
      const selectedSpatialIndex = normalizedSpatial?.solutionIndices[0] ?? 0;
      foldTargetGate.issue();
      const nextFoldDraft = withSpatialFoldSelection(
        line
          ? alignFoldDraft(
              s,
              line,
              picks,
              usesSpatialCycle
                ? spatialMovingSide(
                    normalizedSpatial?.materialSolutions[selectedSpatialIndex],
                    normalizedSpatial?.solutions[selectedSpatialIndex]
                      ?.sideForFirstPick.initial ?? "right",
                  )
                : undefined,
            )
          : null,
        normalizedSpatial?.solutions,
        normalizedSpatial?.materialSolutions,
        selectedSpatialIndex,
      );
      set({
        alignDraft: {
          mode: draft.mode,
          picks,
          cpPicks,
          solutions,
          solutionIndex: 0,
          reason: normalizedSpatial?.reason ?? solved?.reason ?? null,
          ...(spatialPicks === undefined || normalizedSpatial === null
            ? {}
            : {
                spatialPicks,
                spatialSolutions: normalizedSpatial.solutions,
                spatialMaterialSolutions: normalizedSpatial.materialSolutions,
                spatialSolutionIndices: normalizedSpatial.solutionIndices,
                spatialReason: normalizedSpatial.reason,
              }),
        },
        foldDraft: nextFoldDraft,
        errorMessage: null,
      });
    },

    nextAlignSolution: () => {
      const s = get();
      const draft = s.alignDraft;
      const solutionCount = draft?.solutions.length ?? 0;
      if (!draft || solutionCount < 2) return;
      const index = (draft.solutionIndex + 1) % solutionCount;
      const usesSpatialCycle = draft.spatialSolutionIndices !== undefined;
      const spatialIndex = draft.spatialSolutionIndices?.[index] ?? index;
      const line = draft.solutions[index];
      foldTargetGate.issue();
      const nextFoldDraft = withSpatialFoldSelection(
        s.foldDraft
          ? {
              ...clearFoldTargetQuery(s.foldDraft),
              line,
              movingSide: usesSpatialCycle
                ? spatialMovingSide(
                    draft.spatialMaterialSolutions?.[spatialIndex],
                    s.foldDraft.movingSide,
                  )
                : initialMovingSide(line, draft.picks[0]),
            }
          : alignFoldDraft(
              s,
              line,
              draft.picks,
              usesSpatialCycle
                ? spatialMovingSide(
                    draft.spatialMaterialSolutions?.[spatialIndex],
                    draft.spatialSolutions?.[spatialIndex]?.sideForFirstPick.initial ??
                      "right",
                  )
                : undefined,
            ),
        draft.spatialSolutions,
        draft.spatialMaterialSolutions,
        spatialIndex,
      );
      set({
        alignDraft: { ...draft, solutionIndex: index },
        foldDraft: nextFoldDraft,
      });
    },

    undoAlignPick: () => {
      const draft = get().alignDraft;
      if (!draft || draft.picks.length === 0) return;
      foldTargetGate.issue();
      const picks = draft.picks.slice(0, -1);
      const cpPicks = draft.cpPicks?.slice(0, -1);
      const spatialPicks = draft.spatialPicks?.slice(0, -1);
      const spatialReset =
        spatialPicks === undefined || spatialPicks.length === 0
          ? {}
          : {
              spatialPicks,
              spatialSolutions: [] as (SpatialFoldTarget | null)[],
              spatialMaterialSolutions: [] as (
                | SpatialMaterialForMovingSide
                | null
              )[],
              spatialSolutionIndices: [] as number[],
              spatialReason: null,
            };
      set({
        alignDraft: {
          mode: draft.mode,
          picks,
          cpPicks,
          solutions: [],
          solutionIndex: 0,
          reason: null,
          ...spatialReset,
        },
        foldDraft: null,
      });
    },

    cancelAlign: () => {
      if (get().alignDraft) {
        foldTargetGate.issue();
        set({ alignDraft: null, foldDraft: null });
      }
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
          // 紙をつかんでドラッグした操作であることをRustへ伝える。
          // 表示名そのものは送らず、名前の決定はRust側に閉じている。
          grab_move: true,
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
        // 平坦な姿勢でつかんだ場合も同じ「つかんで動かす」操作である。
        grab_move: true,
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
