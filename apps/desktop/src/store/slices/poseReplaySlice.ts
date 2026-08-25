import { currentAngles, hasPoseAngle } from "../../lib/poseStep";
import type {
  AngleRelaxation,
  Document,
  Driver,
  FoldAllLayerOrder,
  Frame3D,
  SeqOp,
  SoftMesh,
} from "../../lib/types";
import type { ReleasedPin } from "../../lib/settledFolds";
import type { ToolId } from "../toolTypes";
import type { DocumentSlice, Selection } from "./documentSlice";

/** ヒンジ角の連続操作(スライダー)を間引く間隔(ms) */
/** 追従計算は60fps相当で最大1回。runLatestが計算待ちを最新1件へまとめる。 */
export const POSE_THROTTLE_MS = 16;

/**
 * 一斉表示専用の送信間隔(ms)。
 *
 * 1秒120入力を65回以下へまとめる受入条件に合わせ、60fps相当の16msとする。
 * 専用queueも同時1件・待機最新1件に保つため、計算が追いつかない場合に
 * 要求列は増えない。入力値自体はZustandへ直ちに反映し、つまみは遅らせない。
 */
export const FOLD_ALL_THROTTLE_MS = 16;

/** 画面更新の仕組みが無い環境(テスト)で1コマを送る間隔(ms) */
export const FALLBACK_FRAME_MS = 16;

/** 折り角度の履歴に残す件数の上限。作品データではないので溜め込みすぎない */
export const ANGLE_HISTORY_LIMIT = 50;

/** 同じ折り線への続けざまの角度変更(スライダーを動かしている間)を
 * 1件にまとめる時間(ms)。ドラッグ1回=履歴1件にするための間隔 */
export const ANGLE_GROUP_MS = 700;

/**
 * 角度確定後に、形が大きく変わったことを知らせる境目。
 *
 * 3D座標は紙の長辺を1としている。実機の通常の末尾補正は0.0488〜0.0674、
 * 問題になった枝の切替は1.0728〜1.5135だったため、その間の0.1（長辺の10%）を
 * 通知境界にする。正しさや候補選択には使わず、確定後の非阻害通知だけに使う。
 */
export const FINISH_JUMP_NOTICE_THRESHOLD = 0.1;

/** 利用者向けにはsolver内部の用語を出さず、起きた見た目の変化だけを伝える。 */
export const FINISH_JUMP_NOTICE =
  "角度を確定したときに紙の形が大きく変わりました。意図した折り方になっているか確認してください。";

/** 希望角との差がこの値以上なら、画面で「追従した」と知らせる。 */
export const RELAX_NOTICE_EPS_DEG = 0.1;

/** 0.1°を10進入力から計算したときの丸め誤差だけを吸収する比較余裕。 */
const RELAX_NOTICE_COMPARE_EPS_DEG = 1e-12;

/** 数値診断から、画面に常時表示する追従だけを辺ID順で取り出す。 */
export function relaxationNotices(
  relaxations: readonly AngleRelaxation[],
): AngleRelaxation[] {
  return relaxations
    .filter(
      (item) =>
        Number.isFinite(item.delta_deg) &&
        Math.abs(item.delta_deg) + RELAX_NOTICE_COMPARE_EPS_DEG >=
          RELAX_NOTICE_EPS_DEG,
    )
    .sort((a, b) => a.hinge - b.hinge);
}

/**
 * 「元に戻す」で戻す1件ぶんの角度の状態。
 *
 * 角度の指定と、どの折り目を固定していたかを組にして控える。
 * 固定の付け外しも1回で戻せるようにするため(片方だけを控えると、
 * 戻した角度と固定が食い違う)。どちらも作品ファイルには保存しない。
 */
export interface AngleSnapshot {
  drivers: ReadonlyMap<number, number>;
  pinned: ReadonlyMap<number, number>;
}

/** 1回の角度操作に属する「いま固定する折り目」。作品には保存しない。 */
export interface ActiveAngleIntent {
  generation: number;
  /** いま動かしている折り目(3Dで水色に光る) */
  hinges: number[];
  /** 全部を「その角度ちょうど」で固定してよいか。
   *
   * 紙を引く操作(左右対称の相手を含む)は2本までなので固定してよい。
   * まとめてスライダーで動かす場合は、全部を同じ角度で固定すると実際の紙では
   * 成り立たないため、代表1本だけを固定する。 */
  fixAll: boolean;
}

/** 一斉表示へ入る直前の入力状態。3D座標は保存せず、この入力から作り直す。 */
export interface FoldAllReturnState {
  docEpoch: number;
  currentStep: number | null;
  playT: number;
  activeTool: ToolId;
  selection: Selection;
}

/** 全折り目を同じ割合で動かしている間だけZustandに置く一時状態。 */
export interface FoldAllPreviewState {
  /** 同じ作品内で入り直した古い応答を見分ける番号。 */
  session: number;
  /** 最後に利用者が指定した割合。 */
  percent: number;
  /** 現在の3D表示へ反映できた割合。まだならnull。 */
  appliedPercent: number | null;
  busy: boolean;
  /** 通常の形を再計算している間も、専用表示の目印と操作制限を残す。 */
  returning: boolean;
  /** 更新要求だけが失敗した場合の非阻害の知らせ。 */
  error: string | null;
  converged: boolean | null;
  bestEffort: boolean;
  relaxationCount: number;
  flatFoldViolationCount: number;
  suspectHingeCount: number;
  contactDetected: boolean;
  layerOrder: FoldAllLayerOrder;
  /** 次の要求の出発角。Document・通常姿勢キャッシュには入れない。 */
  nextWarmSeed: Driver[];
  returnState: FoldAllReturnState;
}

/**
 * 同じ面・同じpolygon頂点の、確定前後の最大移動量を返す。
 * 比較できない不完全frameを「移動0」と誤認しないよう、その場合はnullにする。
 */
export function maximumFrameVertexMovement(
  before: Frame3D,
  after: Frame3D,
): number | null {
  if (before.faces.length !== after.faces.length) return null;
  const afterByFace = new Map(after.faces.map((face) => [face.face, face]));
  if (afterByFace.size !== after.faces.length) return null;
  const seen = new Set<number>();
  let maximum = 0;
  for (const beforeFace of before.faces) {
    if (seen.has(beforeFace.face)) return null;
    seen.add(beforeFace.face);
    const afterFace = afterByFace.get(beforeFace.face);
    if (!afterFace || beforeFace.polygon.length !== afterFace.polygon.length) {
      return null;
    }
    for (let index = 0; index < beforeFace.polygon.length; index++) {
      const beforePoint = beforeFace.polygon[index];
      const afterPoint = afterFace.polygon[index];
      const distance = Math.hypot(
        beforePoint[0] - afterPoint[0],
        beforePoint[1] - afterPoint[1],
        beforePoint[2] - afterPoint[2],
      );
      if (!Number.isFinite(distance)) return null;
      maximum = Math.max(maximum, distance);
    }
  }
  return seen.size === afterByFace.size ? maximum : null;
}

/** Followのframeは候補計算へ渡さず、確定後の通知比較だけに使う数値コピーにする。 */
export function finishComparisonFrame(frame: Frame3D | null): Frame3D | null {
  if (frame === null) return null;
  return {
    faces: frame.faces.map((face) => ({
      ...face,
      polygon: face.polygon.map(
        ([x, y, z]) => [x, y, z] as [number, number, number],
      ),
    })),
    warnings: [],
  };
}

/** 3Dで紙をつかんで引けない理由（引けるならnull）。 */
export function pullBlockReason(s: {
  doc: Document | null;
  playing: boolean;
  playT: number;
  hingeCount: number;
  currentStep: number | null;
  stepCount: number;
}): string | null {
  if (!s.doc) return "紙がありません。上の「新規」で紙を出してください";
  if (s.playing) return "再生中は引けません。下の再生ボタンで止めてください";
  if (s.playT !== 1) return "折り途中の形では引けません。手順を最後まで進めてください";
  if (s.hingeCount === 0) return "折り線がまだありません。先に折り線を引いてください";
  if (s.currentStep !== null && s.currentStep !== s.stepCount)
    return "前の手順の形を見ている間は引けません。手順をいちばん新しい形へ戻してください";
  return null;
}

/** ストアの状態から「引けない理由」を組み立てる。 */
export function pullBlockedOf(s: {
  doc: Document | null;
  playing: boolean;
  playT: number;
  hinges: ReadonlySet<number>;
  currentStep: number | null;
}): string | null {
  return pullBlockReason({
    doc: s.doc,
    playing: s.playing,
    playT: s.playT,
    hingeCount: s.hinges.size,
    currentStep: s.currentStep,
    stepCount: s.doc?.sequence.length ?? 0,
  });
}

/** 「折る前」（0）と「最新」（null）以外の手順が選ばれているか。 */
export function stepPanelSelected(s: { currentStep: number | null }): boolean {
  return s.currentStep !== null && s.currentStep >= 1;
}

/** ふくらます設定を開けない理由（開けるならnull）。 */
export function inflateBlockReason(s: {
  doc: Document | null;
  currentStep: number | null;
}): string | null {
  if (!s.doc) return "紙がありません。上の「新規」で紙を出してください";
  if (stepPanelSelected(s))
    return "手順を選んでいる間は、ふくらます設定を開けません。手順をいちばん新しい形へ戻してください";
  return null;
}

/** 今の形を手順として残せない理由（残せるならnull）。 */
export function poseRecordReason(s: {
  doc: Document | null;
  playing: boolean;
  hinges: ReadonlySet<number>;
  drivers: ReadonlyMap<number, number>;
  poseAngles: ReadonlyMap<number, number>;
}): string | null {
  if (!s.doc) return "まだ紙がありません";
  if (s.playing) return "再生を止めてから残してください";
  if (s.hinges.size === 0) return "折り線がまだありません";
  const requested = new Map(
    [...s.hinges].map((hinge) => [hinge, s.drivers.get(hinge) ?? 0]),
  );
  if (
    !hasPoseAngle(requested) &&
    !hasPoseAngle(currentAngles(s.hinges, s.drivers, s.poseAngles))
  ) {
    return "まだ角度が付いていません(折り線を選んで角度を変えてください)";
  }
  return null;
}

/** その手順が全体再生または部分再生で飛ばされているか。 */
export function isStepSkipped(
  s: { skipped: number[]; replaySkipped: number[] },
  stepId: number,
): boolean {
  return s.skipped.includes(stepId) || s.replaySkipped.includes(stepId);
}

/** 中身が同じなら前の配列を使い回す。 */
export function keepIfSameReleasedPins(
  prev: ReleasedPin[],
  next: ReleasedPin[],
): ReleasedPin[] {
  const same =
    prev.length === next.length &&
    prev.every(
      (pin, index) =>
        pin.hinge === next[index].hinge &&
        pin.pinned === next[index].pinned &&
        pin.actual === next[index].actual,
    );
  return same ? prev : next;
}

/** 姿勢・再生・角度履歴が所有する状態。すべて同じZustandストアへ合成する。 */
export interface PoseReplaySliceState {
  frame3d: Frame3D | null;
  foldAllPreview: FoldAllPreviewState | null;
  suspectHinges: number[];
  sequenceTargets: Map<number, number>;
  relaxations: AngleRelaxation[];
  softMesh: SoftMesh | null;
  softWarnings: string[];
  hinges: ReadonlySet<number>;
  currentStep: number | null;
  playT: number;
  playing: boolean;
  skipped: number[];
  replaySkipped: number[];
  replayWarnings: string[];
  drivers: Map<number, number>;
  pinnedFolds: ReadonlyMap<number, number>;
  releasedPins: ReleasedPin[];
  releasedPinHinges: number[];
  angleUndoStack: AngleSnapshot[];
  angleRedoStack: AngleSnapshot[];
  docUndoDepth: number;
  poseAngles: Map<number, number>;
  poseWarnings: string[];
  poseConverged: boolean;
  poseBestEffort: boolean;
  poseClosureRms: number | null;
  contactDetected: boolean;
  activeAngleIntent: ActiveAngleIntent | null;
  angleIntentGeneration: number;
  pullHinge: number | null;
  pullMirrorHinge: number | null;
}

/** 姿勢・再生・角度履歴が所有する公開action。 */
export interface PoseReplaySliceActions {
  undo: () => Promise<void>;
  redo: () => Promise<void>;
  applySequenceOp: (op: SeqOp) => Promise<void>;
  selectStep: (step: number | null) => void;
  selectStepForCapture: (step: number) => Promise<void>;
  stepBy: (delta: number) => void;
  togglePlay: () => void;
  beginPull: (
    hinge: number,
    angles: ReadonlyMap<number, number>,
    mirrorHinge?: number | null,
  ) => void;
  pullTo: (deg: number) => void;
  endPull: () => void;
  setDriverAngle: (hinge: number, deg: number) => void;
  setDriverAngles: (hinges: readonly number[], deg: number) => void;
  finishAngleIntent: () => Promise<void>;
  clearDriver: (hinge: number) => void;
  clearDrivers: () => void;
  enterFoldAllPreview: () => Promise<void>;
  setFoldAllPercent: (percent: number) => void;
  finishFoldAllPercent: () => void;
  leaveFoldAllPreview: () => Promise<void>;
  togglePinnedFold: (hinge: number) => void;
  setPinnedFolds: (hinges: readonly number[], pinned: boolean) => void;
  recordPoseStep: () => Promise<void>;
  moveStep: (number: number, delta: number) => Promise<void>;
}

export type PoseReplaySlice = PoseReplaySliceState & PoseReplaySliceActions;

/** B1とB2を1本のZustandストアへ再合成するときの境界型。 */
export type PoseReplaySliceHostState = DocumentSlice & PoseReplaySlice;
