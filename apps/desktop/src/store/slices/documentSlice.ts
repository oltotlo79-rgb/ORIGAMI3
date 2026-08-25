import type { GrabMode } from "../../components/Viewer3D/grabFold";
import type { ConstructOptions } from "../../lib/construct";
import type { CurveOptions } from "../../lib/curve";
import {
  ALIGN_EPS,
  ALIGN_STEPS,
  alignRefPoint,
  movingSideOf,
  type AlignMode,
  type AlignTarget,
  type FoldLine,
} from "../../lib/alignFold";
import type { MirrorAxisChoice, MirrorAxisPreset } from "../../lib/mirror";
import type { TechniqueLayerPreset } from "../../lib/techniqueLayers";
import { foldPoseInputFromDrivers } from "../../lib/poseStep";
import type {
  LayerMotionMode,
  LayerMotionPartDraft,
  LayerTurnMode,
} from "../../lib/layerMotion";
import type {
  AngleRelaxation,
  Document,
  DocumentView,
  EdgeKind,
  EditOp,
  Face,
  FoldDirection,
  FoldThroughProposal,
  Frame3D,
  MotionPart,
  SeqOp,
  StepCreases,
  TechniqueKind,
  Vec2,
} from "../../lib/types";
import type { SerialQueue } from "../ipcQueue";
import type { ToolId } from "../toolTypes";

/** 選択中の線・頂点(ID)。DOMのSelectionと紛れないよう注意する。 */
export interface Selection {
  edgeIds: number[];
  vertexIds: number[];
}

export type MeasureMode = "angle" | "length" | "distance";
export type MeasureDisplay = "decimal" | "exact";

export interface MeasureEdgePick {
  kind: "edge";
  edgeId: number;
}

export interface MeasurePointPick {
  kind: "point";
  cp: Vec2;
  faceId: number | null;
  vertexId: number | null;
}

export type MeasurePick = MeasureEdgePick | MeasurePointPick;

export interface MeasureDraft {
  mode: MeasureMode;
  picks: MeasurePick[];
  display: MeasureDisplay | null;
}

export type FoldTarget = "all" | "top";

export interface FoldDraft {
  line: [Vec2, Vec2];
  direction: FoldDirection;
  target: FoldTarget;
  movingSide: "left" | "right";
  docEpoch: number;
  stepCount: number;
  upTo: number;
}

export type AddSegmentOp = Extract<EditOp, { type: "AddSegment" }>;
export type FoldThroughApplyOp = Extract<SeqOp, { type: "FoldThrough" }>;
type FoldThroughInput = Omit<FoldThroughApplyOp, "accept_additional_crease">;

export type SpatialFoldDrag = {
  from: [number, number, number];
  to: [number, number, number];
  grab_face: number;
  mode: GrabMode;
};

export type FoldThroughOperation = FoldThroughInput & { spatial?: SpatialFoldDrag };

export interface PendingFoldThrough {
  proposal: FoldThroughProposal;
  operation: FoldThroughOperation;
  docEpoch: number;
  stepCount: number;
}

export interface AlignDraft {
  mode: AlignMode;
  picks: AlignTarget[];
  cpPicks?: (AlignCpPick | null)[];
  solutions: FoldLine[];
  solutionIndex: number;
  reason: string | null;
}

export type AlignCpPick =
  | { kind: "vertex"; id: number }
  | { kind: "edge"; id: number };

export interface TechniqueDraft {
  kind: TechniqueKind;
  flap: number[];
  flapCandidates: number[];
  flapPickCount: number;
  line: [Vec2, Vec2] | null;
  movingSide: "left" | "right";
  widthMm: number;
  polygon: Vec2[];
  center: Vec2 | null;
  referencePoint: Vec2 | null;
  twistDeg: number;
  openToBack: boolean;
  motionMode: LayerMotionMode;
  motionTurn: LayerTurnMode;
  motionDirection: FoldDirection;
  motionAnchor: number;
  motionReverseLayers: boolean;
  motionAxisEdgeId: number | null;
  motionParts: MotionPart[];
  docEpoch: number;
  stepCount: number;
  upTo: number;
}

/** 新しい配列を持つ空選択。既存command serviceも同じ値を使う。 */
export const EMPTY_SELECTION: Selection = { edgeIds: [], vertexIds: [] };

/** 新しい配列を持つ空の測定状態。 */
export function emptyMeasureDraft(mode: MeasureMode = "angle"): MeasureDraft {
  return { mode, picks: [], display: null };
}

export function selectionForMeasure(picks: readonly MeasurePick[]): Selection {
  return {
    edgeIds: [
      ...new Set(
        picks
          .filter((pick): pick is MeasureEdgePick => pick.kind === "edge")
          .map((pick) => pick.edgeId),
      ),
    ],
    vertexIds: [
      ...new Set(
        picks
          .filter((pick): pick is MeasurePointPick => pick.kind === "point")
          .map((pick) => pick.vertexId)
          .filter((id): id is number => id !== null),
      ),
    ],
  };
}

export function editableCopy(doc: Document): Document {
  return {
    ...doc,
    cp: { ...doc.cp, vertices: [...doc.cp.vertices], edges: [...doc.cp.edges] },
  };
}

export function addWorkingEdge(
  work: Document,
  a: Vec2,
  b: Vec2,
  kind: EdgeKind,
): void {
  const v0 = work.cp.next_vertex_id;
  work.cp.vertices.push({ id: v0, pos: a }, { id: v0 + 1, pos: b });
  work.cp.next_vertex_id = v0 + 2;
  const edgeId = work.cp.next_edge_id;
  work.cp.edges.push({ id: edgeId, v0, v1: v0 + 1, kind });
  work.cp.next_edge_id = edgeId + 1;
}

export function isSpatialFoldFrame(frame: Frame3D | null): boolean {
  return (
    frame !== null &&
    frame.faces.some((face) => face.polygon.some(([, , z]) => Math.abs(z) > 1e-6))
  );
}

export function layerMotionPartDraft(
  draft: TechniqueDraft,
): LayerMotionPartDraft {
  return {
    layers: draft.flap,
    line: draft.line,
    mode: draft.motionMode,
    turn: draft.motionTurn,
    direction: draft.motionDirection,
    anchor: draft.motionAnchor,
    reverseLayers: draft.motionReverseLayers,
  };
}

export function clearCurrentLayerMotion(draft: TechniqueDraft): TechniqueDraft {
  return {
    ...draft,
    flap: [],
    flapCandidates: [],
    flapPickCount: 1,
    line: null,
    motionMode: "reflect",
    motionTurn: "Keep",
    motionDirection: "Up",
    motionAnchor: 0,
    motionReverseLayers: false,
    motionAxisEdgeId: null,
  };
}

export function automaticMovingSide(
  line: FoldLine,
  first: AlignTarget | null | undefined,
): FoldDraft["movingSide"] | null {
  if (!first) return null;
  const dx = line[1][0] - line[0][0];
  const dy = line[1][1] - line[0][1];
  const length = Math.hypot(dx, dy);
  if (length < ALIGN_EPS) return null;
  const signedDistance = (point: Vec2): number =>
    (dx * (point[1] - line[0][1]) - dy * (point[0] - line[0][0])) / length;
  const sideOf = (point: Vec2): FoldDraft["movingSide"] | null => {
    const distance = signedDistance(point);
    if (distance > ALIGN_EPS) return "left";
    if (distance < -ALIGN_EPS) return "right";
    return null;
  };

  if (first.kind === "point") return sideOf(first.p);
  const a = sideOf(first.a);
  const b = sideOf(first.b);
  if (a === null) return b;
  if (b === null) return a;
  return a === b ? a : null;
}

export function initialMovingSide(
  line: FoldLine,
  first: AlignTarget | null | undefined,
): FoldDraft["movingSide"] {
  if (!first) return "right";
  return automaticMovingSide(line, first) ?? movingSideOf(line, alignRefPoint(first));
}

export function alignFoldDraft(
  state: { docEpoch: number; doc: Document | null; currentStep: number | null },
  line: FoldLine,
  picks: AlignTarget[],
): FoldDraft | null {
  if (!state.doc || picks.length === 0) return null;
  return {
    line,
    direction: "Up",
    target: "all",
    movingSide: initialMovingSide(line, picks[0]),
    docEpoch: state.docEpoch,
    stepCount: state.doc.sequence.length,
    upTo: foldInsertAt(state),
  };
}

export function isAlignComplete(draft: AlignDraft): boolean {
  return draft.picks.length >= ALIGN_STEPS[draft.mode].length;
}

export function nextAlignKind(draft: AlignDraft): "point" | "line" | null {
  const steps = ALIGN_STEPS[draft.mode];
  return draft.picks.length < steps.length ? steps[draft.picks.length] : null;
}

export const STALE_DRAFT_MESSAGE =
  "折り方を決めた後に作品または表示する手順が変わったため、選んだ内容を取り消しました。今の形でもう一度選んでください";
const PLAYING_FOLD_MESSAGE =
  "手順を再生している間は、この折り方を確定できません。選んだ内容は残してあります。再生を止めてください";
const PARTIAL_PLAYBACK_FOLD_MESSAGE =
  "手順の途中の形からは、この折り方をまだ記録できません。選んだ内容は残してあります。手順を最後まで進めてください";
const ANGLED_FOLD_MESSAGE =
  "角度を変えた形からは、この折り方をまだ記録できません。選んだ内容は残してあります。角度を0°に戻すと、このまま折れます";
export const TECHNIQUE_FALLBACK_HINT = "手動の折り操作で代替してください";
export const DEFAULT_PLEAT_WIDTH_MM = 10;

export function canFoldNow(state: {
  doc: Document | null;
  currentStep: number | null;
  playT: number;
  playing: boolean;
  drivers: Map<number, number>;
  activeTool?: ToolId;
}): boolean {
  return (state.activeTool === "technique"
    ? foldUnavailableMessage(state)
    : foldThroughUnavailableMessage(state)) === null;
}

export function nonZeroDriverCount(
  drivers: ReadonlyMap<number, number>,
): number {
  let count = 0;
  for (const angle of drivers.values()) {
    if (angle !== 0) count++;
  }
  return count;
}

export function foldUnavailableMessage(state: {
  doc: Document | null;
  playT: number;
  playing: boolean;
  drivers: ReadonlyMap<number, number>;
}): string | null {
  if (!state.doc) return "紙がありません。上の「新規」で紙を出してください";
  if (state.playing) return PLAYING_FOLD_MESSAGE;
  if (state.playT !== 1) return PARTIAL_PLAYBACK_FOLD_MESSAGE;
  if (nonZeroDriverCount(state.drivers) > 0) return ANGLED_FOLD_MESSAGE;
  return null;
}

/**
 * FoldThrough専用の確定条件。0/+180/-180°の平坦姿勢は、符号付き宣言から
 * Rust側で再現してから折れる。技法はまだ従来の条件を使う。
 */
export function foldThroughUnavailableMessage(state: {
  doc: Document | null;
  playT: number;
  playing: boolean;
  drivers: ReadonlyMap<number, number>;
}): string | null {
  if (!state.doc) return "紙がありません。上の「新規」で紙を出してください";
  if (state.playing) return PLAYING_FOLD_MESSAGE;
  if (state.playT !== 1) return PARTIAL_PLAYBACK_FOLD_MESSAGE;
  const pose = foldPoseInputFromDrivers(state.drivers);
  if (pose.ok) return null;
  if (pose.reason === "invalid") {
    return "角度の値を読み取れないため、この折り方を確定できません。選んだ内容は残してあります。角度を入力し直してください";
  }
  return ANGLED_FOLD_MESSAGE;
}

export function foldInsertAt(state: {
  doc: Document | null;
  currentStep: number | null;
}): number {
  const total = state.doc?.sequence.length ?? 0;
  return state.currentStep === null ? total : Math.min(state.currentStep, total);
}

export interface DocumentSliceState {
  doc: Document | null;
  stepCreases: StepCreases[];
  faces: Face[];
  warnings: string[];
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
  construct: ConstructOptions;
  curve: CurveOptions;
  errorMessage: string | null;
  documentSavedPath: string | null;
  docEpoch: number;
}

export interface DocumentSliceActions {
  newDocument: (paper: Document["paper"]) => Promise<void>;
  openDocument: (path: string) => Promise<void>;
  saveDocument: (path: string | null) => Promise<void>;
  applyEdit: (op: EditOp | EditOp[]) => Promise<void>;
  drawSegment: (a: Vec2, b: Vec2, kind: EdgeKind) => Promise<void>;
  drawCurve: (points: Vec2[], kind: EdgeKind) => Promise<void>;
  setMirrorDraw: (on: boolean) => void;
  setMirrorAxisPreset: (preset: MirrorAxisPreset) => void;
  setSelectedLineAsMirrorAxis: () => void;
  setTool: (tool: ToolId) => void;
  setMeasureMode: (mode: MeasureMode) => void;
  setMeasureDisplay: (display: MeasureDisplay) => void;
  pickMeasureEdge: (edgeId: number) => void;
  pickMeasurePoint: (pick: Omit<MeasurePointPick, "kind">) => void;
  clearMeasurement: () => void;
  setSelection: (selection: Selection) => void;
  setHoveredHinge: (hinge: number | null) => void;
  beginFoldDraft: (line: [Vec2, Vec2], source: "2d" | "3d") => void;
  updateFoldDraft: (patch: Partial<FoldDraft>) => void;
  cancelFoldDraft: () => void;
  commitFoldDraft: () => Promise<void>;
  resolveFoldThroughProposal: (accept: boolean) => Promise<void>;
  beginAlign: (mode: AlignMode) => void;
  pickAlignTarget: (
    target: AlignTarget,
    cursor?: Vec2 | null,
    cpPick?: AlignCpPick | null,
  ) => void;
  nextAlignSolution: () => void;
  undoAlignPick: () => void;
  cancelAlign: () => void;
  foldByDrag: (
    from: Vec2 | [number, number, number],
    to: Vec2 | [number, number, number],
    mode: GrabMode,
    grabFace?: number | null,
    direction?: FoldDirection,
  ) => Promise<void>;
  beginTechnique: (kind: TechniqueKind) => void;
  setTechniqueFlap: (faces: number[]) => void;
  setTechniqueFlapPreset: (preset: TechniqueLayerPreset) => void;
  toggleTechniqueFlap: (face: number) => void;
  setTechniqueLine: (line: [Vec2, Vec2]) => void;
  setLayerMotionAxis: (edgeId: number, line: [Vec2, Vec2]) => void;
  addLayerMotionPart: () => void;
  undoLayerMotionPart: () => void;
  addTechniqueVertex: (point: Vec2) => void;
  undoTechniqueVertex: () => void;
  setTechniqueCenter: (point: Vec2 | null) => void;
  setTechniqueReferencePoint: (point: Vec2 | null) => void;
  updateTechniqueDraft: (patch: Partial<TechniqueDraft>) => void;
  setConstruct: (patch: Partial<ConstructOptions>) => void;
  setCurve: (patch: Partial<CurveOptions>) => void;
  cancelTechnique: () => void;
  commitTechnique: () => Promise<void>;
}

export type DocumentSlice = DocumentSliceState & DocumentSliceActions;

/** B2/B4が所有し、B1 actionが同じ1本のstore上で読む構造契約。 */
interface DocumentSliceExternalState {
  foldAllPreview: unknown | null;
  frame3d: Frame3D | null;
  hinges: ReadonlySet<number>;
  currentStep: number | null;
  playT: number;
  playing: boolean;
  drivers: Map<number, number>;
  relaxations: AngleRelaxation[];
  activeAngleIntent: {
    generation: number;
    hinges: number[];
    fixAll: boolean;
  } | null;
  angleIntentGeneration: number;
  pullHinge: number | null;
  pullMirrorHinge: number | null;
  mirrorDraw: boolean;
  mirrorAxis: MirrorAxisChoice;
  mirrorAxisNotice: string | null;
  operationStage: number;
  lineInputStart: Vec2 | null;
  paperActionTipVisible: boolean;
  paperActionTipExpanded: boolean;
}

interface DocumentSliceExternalActions {
  applySequenceOp: (operation: SeqOp) => Promise<void>;
  completeGuideAction: (action: "fold") => void;
}

export type DocumentSliceHostState = DocumentSlice &
  DocumentSliceExternalState &
  DocumentSliceExternalActions;

export interface DocumentSliceDependencies {
  queue: SerialQueue;
  runViewCommand: (
    task: () => Promise<DocumentView>,
    isNewDocument: boolean,
  ) => Promise<void>;
  applyDocChange: (
    task: () => Promise<DocumentView>,
    isNewDocument?: boolean,
  ) => Promise<void>;
  fail: (error: unknown) => void;
  invalidateFoldAllEntry: () => void;
  flushSoftSave: () => Promise<void>;
  waitForFoldAllRestore: () => Promise<void>;
  restoreAfterFoldAllPreview: (restoreInput: boolean) => Promise<boolean>;
  stopPlayback: () => void;
  isStepReplayPending: () => boolean;
  persistPrefs: () => void;
  relaxationNotices: (
    relaxations: readonly AngleRelaxation[],
  ) => AngleRelaxation[];
  clearZeroOnlyDrivers: () => void;
}

interface DocumentSliceInternals {
  invalidateFoldThrough: () => void;
}

export interface DocumentSliceFactoryResult {
  slice: DocumentSlice;
  internals: DocumentSliceInternals;
}
