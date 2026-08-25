// アプリ全体の状態を1本で管理するZustandストア(要件§2: フロント状態はストア1本)。
// IPC呼び出しはactionの中で行い、成功したらDocumentViewの内容を一括反映、
// 失敗(reject)はerrorMessageへ入れる(「止めずに警告」原則)。
// 全IPC要求は直列化キュー(ipcQueue.ts)を通し、連続操作でも適用順を発行順に
// 固定する。最新でない応答は破棄する(古いdocによる上書きを防ぐ)。

import { create } from "zustand";
import * as ipc from "../ipc/client";
import { createSerialQueue } from "./ipcQueue";
import type { ToolId } from "./toolTypes";
export type { ToolId } from "./toolTypes";
import {
  createCommandService,
  keepIfSame,
} from "./services/commandService";
export { MIRROR_AXIS_REMOVED_NOTICE } from "./services/commandService";
import {
  EMPTY_SELECTION,
} from "./slices/documentSlice";
import { createDocumentSlice } from "./services/documentActions";
import type {
  DocumentSlice,
  Selection,
} from "./slices/documentSlice";
export type {
  AlignCpPick,
  AlignDraft,
  FoldDraft,
  FoldTarget,
  MeasureDisplay,
  MeasureDraft,
  MeasureEdgePick,
  MeasureMode,
  MeasurePick,
  MeasurePointPick,
  PendingFoldThrough,
  Selection,
  SpatialFoldDrag,
  TechniqueDraft,
} from "./slices/documentSlice";
export {
  alignFoldDraft,
  automaticMovingSide,
  canFoldNow,
  foldInsertAt,
  initialMovingSide,
  isAlignComplete,
  isSpatialFoldFrame,
  nextAlignKind,
} from "./slices/documentSlice";
import { advancePlayback, startPlayback } from "../lib/playback";
import { buildPoseStep, currentAngles, hasPoseAngle } from "../lib/poseStep";
import {
  DEFAULT_DISPLAY,
  DEFAULT_CONTEXT_PANEL_RATIO,
  DEFAULT_SPLIT_RATIO,
  clampContextPanelRatio,
  clampDivisions,
  clampSplitRatio,
  clampUnit,
  loadPrefs,
  savePrefs,
  softOf,
  type UiTheme,
  type WheelBehavior,
} from "../lib/displayPrefs";
import type { MirrorAxisChoice } from "../lib/mirror";
import type {
  AngleRelaxation,
  Document,
  DisplaySettings,
  DocumentView,
  Driver,
  ExportKind,
  FoldAllLayerOrder,
  Frame3D,
  Paper,
  PaperPosition2d,
  PaperTipPosition,
  ProposalCandidate,
  ProposalJobId,
  ProposalProgressSnapshot,
  RecoveryInfo,
  SeqOp,
  Skeleton,
  SoftMesh,
  SoftSettings,
  TipPos2d,
  Vec2,
} from "../lib/types";
import {
  clampTipPos,
  defaultSkeleton,
  leafNodes,
  setTipPos,
} from "../lib/skeleton";
import {
  clampPaperPosition,
  paperBounds,
} from "../lib/paperPosition";
import {
  paperEditorPositions,
  proposalLeafPositionStates,
  proposalRequestSkeleton,
  setLastMovedSource,
  setSpecifiedPaperPosition,
  type ProposalPositionLastMoved,
} from "../lib/proposalPosition";
import {
  keptFoldsFailed,
  MAX_PIN_RELEASES_ON_SETTLE,
  pinReleaseCandidates,
  pinReleaseNotice,
  releasedPins as releasedPinsOf,
  splitKeptFolds,
  type ReleasedPin,
  withFoldDeviationNotice,
} from "../lib/settledFolds";
import { loadOnboarding, saveOnboarding } from "../lib/firstRunGuide";
import type { HelpChapterId } from "../help/helpTypes";

/** ヒンジ角の連続操作(スライダー)を間引く間隔(ms) */
/** 追従計算は60fps相当で最大1回。runLatestが計算待ちを最新1件へまとめる。 */
const POSE_THROTTLE_MS = 16;

/**
 * 一斉表示専用の送信間隔(ms)。
 *
 * 1秒120入力を65回以下へまとめる受入条件に合わせ、60fps相当の16msとする。
 * 専用queueも同時1件・待機最新1件に保つため、計算が追いつかない場合に
 * 要求列は増えない。入力値自体はZustandへ直ちに反映し、つまみは遅らせない。
 */
const FOLD_ALL_THROTTLE_MS = 16;

/** たわみの指定を作品へ書き込むまでの待ち(ms)。つまみを動かしている間の
 * 書き込みをまとめ、元に戻す履歴が細かく埋まらないようにする */
const SOFT_SAVE_MS = 400;

/** 画面更新の仕組みが無い環境(テスト)で1コマを送る間隔(ms) */
const FALLBACK_FRAME_MS = 16;

/** 折り角度の履歴に残す件数の上限。作品データではないので溜め込みすぎない */
const ANGLE_HISTORY_LIMIT = 50;

/** 同じ折り線への続けざまの角度変更(スライダーを動かしている間)を
 * 1件にまとめる時間(ms)。ドラッグ1回=履歴1件にするための間隔 */
const ANGLE_GROUP_MS = 700;

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

/** UI-012の4操作と、全てできた後の完了画面。 */
export type GuideStep = 0 | 1 | 2 | 3 | 4;
export type GuideAction = "fold" | "angle" | "pull" | "inflate";
const GUIDE_ACTIONS: GuideAction[] = ["fold", "angle", "pull", "inflate"];

/** 提案ウィザードの画面。紙位置は選んだ候補だけを大きく出す任意の別画面。 */
export type ProposalStep =
  | "skeleton"
  | "candidates"
  | "paper-position"
  | "confirm";

/** 提案画面の場所操作を1回戻すための、作品へ保存しない一時状態。 */
export interface ProposalPositionSnapshot {
  step: ProposalStep;
  skeleton: Skeleton;
  candidates: ProposalCandidate[];
  selected: number | null;
  paperSource: number | null;
  paperPositions: PaperTipPosition[];
  paperSpecified: PaperTipPosition[];
  lastMoved: ProposalPositionLastMoved[];
}

/** 提案の計算に使う紙(作品が無いときの控え。App.tsxの既定と同じ) */
const FALLBACK_PAPER: Paper = { width_mm: 150, height_mm: 150 };

/** 新規作成ダイアログで決める紙(PAP-001)。squareなら縦を横に合わせる */
export interface NewPaperDraft {
  widthMm: number;
  heightMm: number;
  square: boolean;
}

/** 新規作成ダイアログの初期値(起動時と同じ150×150mmの正方形) */
export const DEFAULT_NEW_PAPER: NewPaperDraft = {
  widthMm: 150,
  heightMm: 150,
  square: true,
};

/** 下書きから実際の紙を作る(正方形なら縦=横) */
export function draftToPaper(draft: NewPaperDraft): Paper {
  return {
    width_mm: draft.widthMm,
    height_mm: draft.square ? draft.widthMm : draft.heightMm,
  };
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
 * 面配列の順番は表示都合で変わり得るため、face IDで対応付ける。
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
function finishComparisonFrame(frame: Frame3D | null): Frame3D | null {
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

/** 0°は「平ら」を明示しただけなので、折る操作を妨げる角度指定に数えない。 */
function nonZeroDriverCount(drivers: ReadonlyMap<number, number>): number {
  let count = 0;
  for (const angle of drivers.values()) {
    if (angle !== 0) count++;
  }
  return count;
}

/**
 * 3Dで紙をつかんで引けない理由(引けるならnull)。UI-007。
 * 引く操作は「今見えている立体の形」を出発点にするので、平らに畳んだ状態でも
 * 手順で折り上げた状態でも使える。形が動いている最中(再生・折り途中)だけ断る。
 */
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

/**
 * ストアの状態から「引けない理由」を組み立てる。
 * 3Dビューのカーソル・案内文と、紙をクリックしたときの案内が同じ判断を使う
 * (別々に書くと、片方だけ押せる入口が残ってしまう)。
 */
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

/**
 * 手順を選んでいて、下のパネルがその手順の設定で埋まっている状態か。
 * 「折る前」(0)と「最新」(null)は手順を選んでいない扱いにする。
 */
export function stepPanelSelected(s: { currentStep: number | null }): boolean {
  return s.currentStep !== null && s.currentStep >= 1;
}

/**
 * ふくらます設定を開けない理由(開けるならnull)。
 * 丸みのつまみは「何も選んでいないときのパネル」1か所だけに置く決まりなので、
 * 手順を選んでいる間は同じ場所がその手順の設定で埋まり、開けない。
 * すでに付けた丸みは、手順を選んでいる間もそのまま表示に効く
 * (手順の再生も丸みを付けて描くため)。
 */
export function inflateBlockReason(s: {
  doc: Document | null;
  currentStep: number | null;
}): string | null {
  if (!s.doc) return "紙がありません。上の「新規」で紙を出してください";
  if (stepPanelSelected(s))
    return "手順を選んでいる間は、ふくらます設定を開けません。手順をいちばん新しい形へ戻してください";
  return null;
}

/**
 * 今の形を手順として残せない理由(残せるならnull)。SIM-009。
 * 押せないときもボタンは消さず、この短い日本語を添えて理由を見せる。
 */
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
  // 計算待ちの間はposeAnglesが直前値のことがある。利用者が非0°を指定済みなら
  // ボタンを出しておき、recordPoseStep側で末尾solveを待って実角を再確認する。
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

interface AppState extends DocumentSlice {
  /** 3D表示フレーム。nullなら平ら(展開図から直接描く) */
  frame3d: Frame3D | null;
  /** 全折り目を同じ割合で動かす一時表示。作品・設定・Undoには保存しない。 */
  foldAllPreview: FoldAllPreviewState | null;
  /** 補正後にも残る食い込みの原因候補。2D/3Dの赤い強調で共用する。 */
  suspectHinges: number[];
  /** 表示中の保存手順から現在の辺へ解決した希望角。作品には保存しない。 */
  sequenceTargets: Map<number, number>;
  /** 前の希望角が紙のつながりに合わせて譲った診断。作品には保存しない。 */
  relaxations: AngleRelaxation[];
  /** たわみの三角形の網(SIM-012)。たわみを切っているとnull(従来の描き方に戻る) */
  softMesh: SoftMesh | null;
  /** たわみの計算から返った注意書き(日本語)。設定パネルに出す */
  softWarnings: string[];
  /** 折り角度を指定できる辺(ヒンジ)のID集合。doc/faces更新時に1度だけ導出する */
  hinges: ReadonlySet<number>;
  /** 表示中の折り手順番号(1始まり。0は折る前、nullは全手順を折った最新の状態) */
  currentStep: number | null;
  /** 表示中の手順の進み具合(0..1)。再生アニメーションの途中経過 */
  playT: number;
  /** 手順を自動再生中か */
  playing: boolean;
  /** 全手順を通した再生で飛ばされた手順のID(DocumentView由来。作品全体の結果)。
   * タイムラインの赤表示はこちらを見る */
  skipped: number[];
  /** 途中の手順までを再生し直したときに飛ばされた手順のID。
   * その手順までのことしか分からないので、全体のskippedとは別に持つ
   * (混ぜると、途中の手順を選んだだけで後ろの手順の赤表示が消えてしまう) */
  replaySkipped: number[];
  /** 手順再生からの警告(飛ばした理由など)。展開図・追従計算の警告とは別に持つ */
  replayWarnings: string[];
  /** 利用者が指定した折り角度(度)。キーは辺ID */
  drivers: Map<number, number>;
  /**
   * 利用者が角度を固定した折り目と、その角度(度)。キーは辺ID。
   *
   * 固定した折り目は、ほかの折り目を動かしても角度が変わらない。
   * 角度の指定(drivers)と同じ性格の一時的な指定なので、**作品ファイルには
   * 保存しない**(保存すると、同じ展開図と手順から作り直した形が固定の有無で
   * 変わってしまい、「展開図+折り手順から導出する」決まりに反する)。
   * 端末の設定にも書かない。作品を開く・新規作成で捨てる。
   */
  pinnedFolds: ReadonlyMap<number, number>;
  /**
   * 固定したままでは紙が裂けるため、やむを得ず動かした折り目。
   * 1回の計算ごとに作り直す診断で、作品には保存しない。
   */
  releasedPins: ReleasedPin[];
  /**
   * いま「動かさない側」から外している折り目を、外した順に並べたもの。
   *
   * つまみを動かしている間ずっと持ち越す(毎コマ探し直すと固定が付いたり
   * 外れたりして形が揺れる)。戻すときは最後に外した1本から戻す。
   * 自動で守っている折り切りも含むので、利用者へ知らせる `releasedPins` とは別。
   */
  releasedPinHinges: number[];
  /** 折り角度を変える前の指定の控え(古い順)。「元に戻す」はまずここを使う。
   * 3Dの形は作品データではないので保存せず、作品を開く・新規作成で捨てる */
  angleUndoStack: AngleSnapshot[];
  /** 「元に戻す」で戻した角度をやり直すための控え(古い順) */
  angleRedoStack: AngleSnapshot[];
  /** 作品データ側(edit_undo)を戻した回数。「やり直し」はこちらを先に消化する
   * (線の追加を戻した後は、まず線の追加をやり直すのが自然な順番) */
  docUndoDepth: number;
  /** 直近のソルバー解(度)。キーは辺ID。未指定ヒンジの現在角の表示に使う */
  poseAngles: Map<number, number>;
  /** 追従計算からの警告(不収束など)。展開図の検査警告とは別に持つ */
  poseWarnings: string[];
  /** 追従計算が収束したか(falseなら3D区画のバッジで知らせる) */
  poseConverged: boolean;
  /** 収束前でも現在指定を守った最良の有限候補を表示しているか。 */
  poseBestEffort: boolean;
  /** 表示中の追従結果の閉包残差RMS。 */
  poseClosureRms: number | null;
  /** 紙どうしの接触を検出したか。接触しても要求角まで進む。 */
  contactDetected: boolean;
  /** 現在操作中の折り目と要求世代。Zustand内だけに置く一時情報。 */
  activeAngleIntent: ActiveAngleIntent | null;
  /** 新しい操作が古い操作を必ず追い越すための単調増加番号。 */
  angleIntentGeneration: number;
  /** 今つかんで引いている折り線の辺ID(3D表示で色を付ける)。引いていなければnull */
  pullHinge: number | null;
  /** 一緒に動かしている左右対称の相手の折り線(3D表示で同じ色を付ける)。
   * 左右同時が切ってあるか、対称の相手が無ければnull */
  pullMirrorHinge: number | null;
  /** 前回の異常終了で残った作業中の内容。あれば復旧ダイアログを出す(SYS-003) */
  recovery: RecoveryInfo | null;

  /** 提案ウィザードの今の画面。nullなら閉じている(PRO-004: 常設UIは増やさない) */
  proposalStep: ProposalStep | null;
  /** ウィザードで編集中の骨格(PRO-001) */
  proposalSkeleton: Skeleton;
  /** 生成された候補(最大4件。PRO-005) */
  proposalCandidates: ProposalCandidate[];
  /** 選んでいる候補の添字。まだ選んでいなければnull */
  proposalSelected: number | null;
  /** 紙位置の大きな別画面を開いた元候補。開いていなければnull */
  proposalPaperSource: number | null;
  /** 紙の上で動かしている先端の場所。完成形の位置とは別に保つ一時状態 */
  proposalPaperPositions: PaperTipPosition[];
  /** 利用者が紙の画面で実際に動かした場所。候補由来の表示位置は含めない */
  proposalPaperSpecified: PaperTipPosition[];
  /** 葉ごとに、完成形と紙の上のどちらを最後に動かしたか */
  proposalPositionLastMoved: ProposalPositionLastMoved[];
  /** 提案位置の「元に戻す」履歴（古い順） */
  proposalPositionUndoStack: ProposalPositionSnapshot[];
  /** 提案位置の「やり直す」履歴（古い順） */
  proposalPositionRedoStack: ProposalPositionSnapshot[];
  /** 生成中か(「計算中…」の表示用) */
  proposalBusy: boolean;
  /** 現在の提案計算。画面へ値そのものは出さない。計算していなければnull */
  proposalJobId: ProposalJobId | null;
  /** 生成中の一貫した進み具合(「4件中2件め」の表示用)。計算していないときはnull */
  proposalProgress: ProposalProgressSnapshot | null;
  /** 進み具合だけを読めなかったときの非阻害の案内。計算自体は続ける。 */
  proposalProgressWarning: string | null;
  /** 生成に失敗した理由(日本語)。成功したらnull */
  proposalError: string | null;
  /** 次に使う乱数の初期値。作り直すたびに増やして別の配置を出す */
  proposalSeed: number;

  /** 書き出しダイアログを開いているか(常設UIは増やさない。EXP-001/EXP-002) */
  exportOpen: boolean;
  /** 書き出す種類 */
  exportKind: ExportKind;
  /** 補助線も含めるか */
  exportIncludeAux: boolean;
  /** PNGのときの長いほうの辺の点数 */
  exportLongSide: number;
  /** 書き出し中か(ボタンの二度押し防止) */
  exportBusy: boolean;
  /** 書き出しに失敗した理由(日本語)。成功したらnull */
  exportError: string | null;
  /** 保存できたファイルの場所。まだならnull(「保存しました」の表示用) */
  exportSavedPath: string | null;

  /** 新規作成ダイアログを開いているか(常設UIは増やさない。PAP-001) */
  newDialogOpen: boolean;
  /** 新規作成ダイアログで決めている紙の形と大きさ */
  newPaperDraft: NewPaperDraft;
  /** 紙の色・方眼の分割数(PAP-003 / CPE-003)。作品ごとの設定なので
   * doc.displayの写し(作品を開くたびにその作品の値になる)。
   * まだ作品が無い間だけ既定値を持つ */
  display: DisplaySettings;
  /** 中央の2D区画の幅の割合(残りが3D区画。UI-004) */
  splitRatio: number;
  /** 下部の「今できる操作」の高さの割合。端末にだけ保存する。 */
  contextPanelRatio: number;
  /** 対称に線を引くか(CPE-010)。消す・種類を変える操作にも同じ基準線で効く。 */
  mirrorDraw: boolean;
  /** 描画・削除・線種変更で共通して使う基準線。作品ファイルには保存しない。 */
  mirrorAxis: MirrorAxisChoice;
  /** 選んだ基準線が無くなったとき、既存の下部通知欄へ出す非エラーのお知らせ。 */
  mirrorAxisNotice: string | null;
  /** 3Dで紙を引くとき左右対称の相手も同時に動かすか(UI-007)。既定はオン。
   * 画面の使い方の好みなので端末に覚えておく(作品の中身には入れない) */
  pullMirror: boolean;
  /** 2D展開図で修飾キーなしのホイールをスクロールと拡大縮小のどちらに使うか。 */
  wheelBehavior: WheelBehavior;
  /** 画面全体のデザイン。端末にだけ保存し、作品ファイルには含めない。 */
  uiTheme: UiTheme;
  /** 下部の詳しい操作方法を展開しているか。閉じても今できる操作は常に残す。 */
  contextHelpExpanded: boolean;
  /** 3D操作の吹き出しを展開しているか。閉じても見出しは常に残す。 */
  viewerHintExpanded: boolean;
  /** 2D展開図の詳しいホイール操作を展開しているか。 */
  cpHelpExpanded: boolean;
  /** 紙の丸み・膨らみについての詳しい説明を展開しているか。 */
  paperHelpExpanded: boolean;
  /** 紙の表裏の色見本を展開しているか。畳んでも現在色は小さく残す。 */
  paperColorExpanded: boolean;
  /** 初回ガイドを隅のカードとして表示しているか。 */
  guideOpen: boolean;
  /** 0〜3が実践する4操作、4は全操作を終えた完了画面。 */
  guideStep: GuideStep;
  /** 詳しいヘルプセンターを独立ダイアログで表示しているか。 */
  helpOpen: boolean;
  /** ヘルプセンターで最後に選んだ章。 */
  helpChapterId: HelpChapterId;
  /** 章題と本文を絞り込む検索文字列。 */
  helpQuery: string;
  /** 選択中ツールの手順のうち、いま強調する段階(0始まり)。 */
  operationStage: number;
  /** 展開図で線を引く途中の始点。ポインタとキーボードで共用し、作品には保存しない。 */
  lineInputStart: Vec2 | null;
  /** 3Dで紙そのものを選んだときの「引く・膨らます」案内を出しているか。 */
  paperActionTipVisible: boolean;
  /** 紙の操作案内を詳しく開いているか。閉じた後は小さな入口だけ残す。 */
  paperActionTipExpanded: boolean;
  /** 3Dで引くとき左右同時に動かすかを切り替える(次回起動時も同じ設定に戻る) */
  setPullMirror: (on: boolean) => void;
  /** 2D展開図のホイール動作を切り替える(次回起動時も同じ設定に戻る) */
  setWheelBehavior: (behavior: WheelBehavior) => void;
  /** 画面デザインを切り替える(次回起動時も同じ設定に戻る) */
  setUiTheme: (theme: UiTheme) => void;
  /** 下部の詳しい操作方法を開閉する。 */
  toggleContextHelp: () => void;
  /** 3D操作の吹き出しを開閉する。 */
  toggleViewerHint: () => void;
  /** 2D展開図の詳しいホイール操作を開閉する。 */
  toggleCpHelp: () => void;
  /** 紙の丸み・膨らみについての詳しい説明を開閉する。 */
  togglePaperHelp: () => void;
  /** 紙の表裏の色見本を開閉する。 */
  togglePaperColor: () => void;
  /** ヘルプから基本操作ガイドを最初の操作へ戻して表示する。 */
  openGuide: () => void;
  /** ヘルプセンターを開く。ツールバーとF1から共用する。 */
  openHelp: () => void;
  /** ヘルプセンターを閉じる。 */
  closeHelp: () => void;
  /** ヘルプセンターで表示する章を選ぶ。 */
  selectHelpChapter: (chapterId: HelpChapterId) => void;
  /** ヘルプセンターの検索文字列を変える。 */
  setHelpQuery: (query: string) => void;
  /** ×またはスキップで閉じ、この端末では初回表示済みとして覚える。 */
  dismissGuide: () => void;
  /** 該当する実操作が成功したときだけ、ガイドを次へ進める。 */
  completeGuideAction: (action: GuideAction) => void;
  /** 選択中ツールの手順表示を、指定した段階へ進める/戻す。 */
  setOperationStage: (stage: number) => void;
  /** 展開図で線を引く始点を更新する。nullで引きかけを取り消す。 */
  setLineInputStart: (start: Vec2 | null) => void;
  /** 3Dで紙を選んだとき、引く・膨らますへの入口を表示する。 */
  showPaperActionTip: () => void;
  /** 詳しい紙の操作案内を小さな入口へ畳む。 */
  collapsePaperActionTip: () => void;
  /** 小さな紙の操作入口をもう一度詳しく開く。 */
  expandPaperActionTip: () => void;
  /** 空白などを選んだとき紙の操作案内を隠す。 */
  hidePaperActionTip: () => void;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
  /** 手順の追加・変更・削除(sequence_apply)。再生中なら止めてから送る */
  applySequenceOp: (op: SeqOp) => Promise<void>;
  /** 表示する手順を選ぶ(0=折る前、null=最新)。再生中なら止める */
  selectStep: (step: number | null) => void;
  /** 撮影用: 表示する手順を選び、立体の再計算が終わるまで待つ。 */
  selectStepForCapture: (step: number) => Promise<void>;
  /** 表示する手順を前後に動かす(コマ送り) */
  stepBy: (delta: number) => void;
  /** 再生と一時停止を切り替える */
  togglePlay: () => void;
  /**
   * 3Dで紙をつかんで引く操作を始める(UI-007)。
   * 今見えている形の全ヒンジ角(anglesは lib/grabDrive の読み取り結果)を
   * そのまま送ってソルバーの出発点を今の形に合わせ、駆動する折り線を覚える。
   * これがないと、手順で折り上げた作品を引いたとたん平らな解へ飛んでしまう
   */
  beginPull: (
    hinge: number,
    angles: ReadonlyMap<number, number>,
    mirrorHinge?: number | null,
  ) => void;
  /** 引いている間の角度(度)。16ms間引きで追従計算を呼ぶ。
   * 左右対称の相手がいれば同じ角度で一緒に動かす */
  pullTo: (deg: number) => void;
  /** 引く操作を終える(角度指定は残る。色付けだけ消す) */
  endPull: () => void;
  /** ヒンジの折り角度を指定する(16ms間引きで追従計算を呼ぶ) */
  setDriverAngle: (hinge: number, deg: number) => void;
  /** 複数ヒンジを同じ角度へまとめ、1回の追従計算へ送る */
  setDriverAngles: (hinges: readonly number[], deg: number) => void;
  /** 末尾の角度要求を送り終えてから、操作中の強調を解除する。 */
  finishAngleIntent: () => Promise<void>;
  /** 1本の角度指定を解除する(形は残りの指定から計算し直す) */
  clearDriver: (hinge: number) => void;
  /** 全ての角度指定を解除して平らに戻す */
  clearDrivers: () => void;
  /** 全折り目を同じ割合で動かす一時表示へ入る。0%の形を直ちに求める。 */
  enterFoldAllPreview: () => Promise<void>;
  /** 一時表示の割合を0..=100へ更新する。連続入力は16msごとにまとめる。 */
  setFoldAllPercent: (percent: number) => void;
  /** pointer up等で最後の割合を確定する。0%なら入口前の通常表示へ戻る。 */
  finishFoldAllPercent: () => void;
  /** 一時表示を閉じ、入る前の手順位置から通常の形を作り直す。 */
  leaveFoldAllPreview: () => Promise<void>;
  /** 折り目1本の角度の固定を、押すたびに付け外しする */
  togglePinnedFold: (hinge: number) => void;
  /** 選んだ折り目の角度をまとめて固定する・まとめて外す */
  setPinnedFolds: (hinges: readonly number[], pinned: boolean) => void;
  /** 今つけている立体的な形を「仕上げの角度」の手順として残す(SIM-009) */
  recordPoseStep: () => Promise<void>;
  /** 起動時に、前回の異常終了で残った作業中の内容があるか調べる */
  checkRecovery: () => Promise<void>;
  /** 復旧ダイアログの答えを実行する(true=復元する / false=破棄する) */
  resolveRecovery: (accept: boolean) => Promise<void>;
  /** 提案ウィザードを開く(骨格は初期状態に戻す) */
  openProposal: () => void;
  /** 提案ウィザードを閉じる */
  closeProposal: () => void;
  /** ウィザードの画面を切り替える */
  setProposalStep: (step: ProposalStep) => void;
  /** 編集した骨格を差し替える(前の候補は作り直しになるので捨てる) */
  setProposalSkeleton: (skeleton: Skeleton) => void;
  /** 完成形の先端位置1件を動かし、紙位置を保ったまま最後の操作として記録する */
  setProposalTipPosition: (leafId: number, position: TipPos2d | null) => void;
  /** 今の骨格で候補を作り、候補選びの画面へ進む(PRO-005) */
  generateProposal: () => Promise<void>;
  /** 候補を選ぶ */
  selectProposalCandidate: (index: number) => void;
  /** 選んだ候補だけを大きく出し、紙の上の場所を動かす画面を開く */
  openProposalPaperPositionEditor: () => void;
  /** 紙の上の先端1件を動かす */
  setProposalPaperPosition: (
    leafId: number,
    position: PaperPosition2d,
  ) => void;
  /** 紙の上の場所を、別画面を開いたときの候補へ戻す */
  resetProposalPaperPositions: () => void;
  /** 場所が違う葉を、いま使っていない側へ戻す */
  restoreOtherProposalPosition: (leafId: number) => void;
  /** 提案画面で行った場所操作を1件戻す */
  undoProposalPosition: () => void;
  /** 戻した提案画面の場所操作を1件やり直す */
  redoProposalPosition: () => void;
  /** 葉ごとに完成形／紙の上の使う場所をまとめた要求を1件送り、候補を作り直す */
  generateProposalFromPaperPositions: () => Promise<void>;
  /** 選んだ候補を今の作品の展開図にしてウィザードを閉じる(PRO-003) */
  applyProposalCandidate: () => Promise<void>;
  /** 書き出しダイアログを開く(前回の結果表示は消す) */
  openExport: () => void;
  /** 書き出しダイアログを閉じる */
  closeExport: () => void;
  /** 書き出しの指定を変える(種類・補助線・大きさ) */
  setExportOption: (patch: Partial<ExportSettings>) => void;
  /** 指定の場所へ書き出す。成功したらexportSavedPathに場所が入る */
  runExport: (path: string) => Promise<void>;
  /** 新規作成ダイアログを開く(前回決めた大きさをそのまま出す) */
  openNewDialog: () => void;
  /** 新規作成ダイアログを閉じる(作らずにやめる) */
  closeNewDialog: () => void;
  /** 新規作成ダイアログの紙の指定を変える */
  setNewPaperDraft: (patch: Partial<NewPaperDraft>) => void;
  /** ダイアログで決めた大きさの紙で作り直す */
  confirmNewDocument: () => Promise<void>;
  /** 紙の色・方眼の分割数を変える(作品ごとの設定として保存する) */
  setDisplay: (patch: Partial<DisplaySettings>) => Promise<void>;
  /**
   * 紙のたわみの指定を変える(SIM-012 / SIM-013)。
   * 画面はその場で変え、3D表示の作り直しは16msに1回へ間引いて依頼する
   * (膨らみのつまみを動かしながら形を見られるように)。
   * 作品への保存は少し遅らせてまとめる(つまみ1回の操作で履歴が埋まらないように)
   */
  setSoft: (patch: Partial<DisplaySettings>) => void;
  /** 2D区画と3D区画の分割比を変える(次回起動時も同じ位置に戻る) */
  setSplitRatio: (ratio: number) => void;
  /** 下部の「今できる操作」の高さを変える(次回起動時も同じ位置に戻る) */
  setContextPanelRatio: (ratio: number) => void;
  /** 2D/3Dと下部パネルの広さを、初めて開いたときの値へ戻す。 */
  resetPaneSizes: () => void;
  /** 手順の順番を入れ替える(numberは1始まり、deltaは-1で前へ/+1で後ろへ、0は同一位置) */
  moveStep: (number: number, delta: number) => Promise<void>;
}

/** 書き出しダイアログで変えられる指定 */
export interface ExportSettings {
  exportKind: ExportKind;
  exportIncludeAux: boolean;
  exportLongSide: number;
}

/** PNGの既定の長辺(点)。Rust側のDEFAULT_LONG_SIDE_PXと揃える(EXP-002) */
export const DEFAULT_PNG_LONG_SIDE = 2048;

/**
 * トレーリングエッジのスロットル。連続呼び出しをintervalMsごと1回に間引き、
 * 最後の呼び出しは必ず実行する(スライダーを離した位置の角度が捨てられない)。
 * reset()は予約を取り消し、「たった今実行した」ものとして間隔を測り直す
 * (別経路で要求を送った直後に、間引き分が二重に飛ぶのを防ぐ)。
 */
function createTrailingThrottle(intervalMs: number, fn: () => void) {
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
    /** 予約中の末尾1件を待たずに実行する。予約が無ければ何もしない。 */
    flush: () => {
      if (timer === null) return;
      clear();
      lastRun = Date.now();
      fn();
    },
    /** 予約と間隔の基準を完全に初期化する(テストの前処理用) */
    clearAll: () => {
      clear();
      lastRun = 0;
    },
  };
}

/** テスト用: 角度計算の間引き状態を初期化する。
 * 間引きの基準時刻はストア(アプリ全体で1個)が持ち続けるため、初期化できないと
 * 前のテストで進めた時計が次のテストの待ち時間に響く(テストの順序依存の原因)。*/
let resetThrottle: () => void = () => {};
export function resetPoseThrottle(): void {
  resetThrottle();
}

/** テスト用: 一斉表示の予約と世代を初期化する。 */
let resetFoldAllRuntime: () => void = () => {};
export function resetFoldAllPreviewRuntime(): void {
  resetFoldAllRuntime();
}

/** 中身が同じなら前の配列を使い回す(毎回の計算で画面を描き直さないため)。 */
function keepIfSameReleasedPins(
  prev: ReleasedPin[],
  next: ReleasedPin[],
): ReleasedPin[] {
  const same =
    prev.length === next.length &&
    prev.every(
      (pin, i) =>
        pin.hinge === next[i].hinge &&
        pin.pinned === next[i].pinned &&
        pin.actual === next[i].actual,
    );
  return same ? prev : next;
}

/** その手順が再生で飛ばされているか。作品全体の再生結果(DocumentView由来)を見て、
 * 途中まで再生し直したときの結果もあわせて見る。
 * 部分再生の結果だけで判断すると、途中の手順を選んだ瞬間に後ろの手順の印が消える */
export function isStepSkipped(
  s: { skipped: number[]; replaySkipped: number[] },
  stepId: number,
): boolean {
  return s.skipped.includes(stepId) || s.replaySkipped.includes(stepId);
}

/** 1件の提案計算にだけ属する、保存しないruntime資源。 */
interface ActiveProposalJob {
  jobId: ProposalJobId;
  generation: number;
  stopPolling: (() => void) | null;
  releaseFullHold: (() => void) | null;
}

export const useAppStore = create<AppState>((set, get) => {
  const commandService = createCommandService<AppState>(set, get, {
    discardFoldAllPreview: () => discardFoldAllPreview(),
    stopPlayback: () => stopPlayback(),
    resetPoseSchedule: () => pose.reset(),
    clearAngleHistory: () => clearAngleHistory(),
    syncSequence: (view) => syncSequence(view),
    syncPose: () => syncPose(),
  });
  const {
    queue,
    fail,
    runViewCommand,
    applyDocChangeResult,
    applyDocChange,
  } = commandService;
  // 読み取り専用の一斉表示は作品編集FIFOから独立させる。保存要求が後から
  // 積まれただけで表示結果を古い扱いにせず、同時実行はこのqueue内で1件に保つ。
  const foldAllQueue = createSerialQueue();
  const prefs = loadPrefs();
  let onboarding = loadOnboarding();
  /** 一斉表示へ入り直した古い応答を捨てる世代。 */
  let foldAllSessionGeneration = 0;
  /** 16ms待ちの間にも新入力が旧応答を追い越せる要求番号。 */
  let foldAllRequestGeneration = 0;
  /** 通常表示へ戻す操作を連打したとき、最後の操作だけを完了させる世代。 */
  let foldAllExitGeneration = 0;
  /** 保存・書き出しが復帰計算を「古い応答」にしないために待つ約束。 */
  let latestFoldAllRestorePromise: Promise<void> = Promise.resolve();
  /** 遅延保存を確定している間の入口二重実行を防ぐ。作品状態ではない実行制御。 */
  let foldAllEntering = false;
  /** 入口待機中に新規・開くが始まった場合、その古い入口を打ち切る世代。 */
  let foldAllEnterGeneration = 0;
  /** 閉じた／作り直した提案画面へ古い計算結果を戻さないための要求世代。 */
  let proposalGeneration = 0;
  /** 現在の提案計算だけが所有するpollと満杯表示の時計。 */
  let activeProposalJob: ActiveProposalJob | null = null;
  /**
   * 提案の計算中に進み具合を読み直す間隔(ミリ秒)。
   *
   * 候補は同時に計算していて、終わった件数だけが増えていく。
   * いちばん軽い骨格でも計算は0.005秒で終わるので、その場合は
   * 一度も表示されないまま消える(それでよい)。重い骨格では数秒かかるので、
   * この間隔なら棒がなめらかに伸びる。
   */
  const PROPOSAL_PROGRESS_POLL_MS = 150;

  /**
   * 計算が終わるまで進み具合を読み続ける。返った関数を呼ぶと止まる。
   *
   * 読めなくても計算は止めない(`CLAUDE.md` §8「止めずに警告する」)。
   * 進み具合はあくまで待ち時間の目安で、これが無くても結果は同じ。
   */
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

  /** cancelの失敗は画面を閉じる処理を止めず、未処理のrejectも残さない。 */
  const requestProposalCancel = (job: ActiveProposalJob): void => {
    try {
      void ipc
        .proposalControl({ type: "Cancel", job_id: job.jobId })
        .catch(() => undefined);
    } catch {
      // backendのwatchdogが残った計算を回収する。画面側は既に無効化済み。
    }
  };

  /** 現在の計算を画面から切り離し、時計を同期的に0件へ戻す。 */
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

  /**
   * 計算が終わったあと、棒を最後まで伸ばしたまま見せておく時間(ミリ秒)。
   *
   * 最後の1件が終わってから計算が返るまでは1ミリ秒ほどしかない。
   * 読み取りは `PROPOSAL_PROGRESS_POLL_MS` ごとなので、**最後の「4/4 件」に間に合わない**。
   * 実測(2026-08-24、先端12本、最適化ありのアプリ、100msごとに画面を読んだ)では
   * `0/4`→`1/4`(4.53秒)→`2/4`(4.88秒)→`3/4`(5.77秒)と伸びたあと、
   * 6.87秒で**棒が3/4のまま消えた**。利用者には棒が途中で終わったように見えていた。
   * そこで計算が返った時点で自分で満杯にし、この時間だけ見せてから消す。
   */
  const PROPOSAL_PROGRESS_FULL_HOLD_MS = 150;

  /**
   * 棒を最後まで伸ばして、少しの間だけ見せる。
   *
   * 表示のためだけの処理で、候補の中身も計算も変えない。
   * 進み具合を一度も読めていなければ(総数0。軽い骨格は0.005秒で終わるので
   * よくある)、棒はそもそも出ていないので何もせずに返る。
   */
  const holdFullProposalBar = async (job: ActiveProposalJob): Promise<void> => {
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

  /** 2つの生成入口でjob照合・timer回収・古い応答破棄を同じにする。 */
  const runProposalGeneration = async (
    requestSkeleton: Skeleton,
    paper: Paper,
    seed: number,
    resultState: (candidates: ProposalCandidate[]) => Partial<AppState>,
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
      // 発行済みpollもstopped flagで遮断してから、満杯表示を保持する。
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
    } catch (e) {
      if (!isCurrentProposalJob(job)) return;
      set({
        proposalBusy: false,
        proposalJobId: null,
        proposalProgress: null,
        proposalProgressWarning: null,
        proposalError: typeof e === "string" ? e : String(e),
      });
    } finally {
      stopProposalJobTimers(job);
      if (activeProposalJob === job) activeProposalJob = null;
    }
  };
  /** 同じ葉のつまみを続けて動かす間は、ドラッグ1回を履歴1件へまとめる。 */
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
    const s = get();
    if (s.proposalStep === null) {
      throw new Error("proposal position snapshot requires an open proposal");
    }
    return {
      step: s.proposalStep,
      skeleton: cloneProposalSkeleton(s.proposalSkeleton),
      // 候補は生成後に書き換えず配列ごと交換するため、履歴間で安全に共有できる。
      candidates: s.proposalCandidates,
      selected: s.proposalSelected,
      paperSource: s.proposalPaperSource,
      paperPositions: clonePaperPositions(s.proposalPaperPositions),
      paperSpecified: clonePaperPositions(s.proposalPaperSpecified),
      lastMoved: s.proposalPositionLastMoved.map((entry) => ({ ...entry })),
    };
  };

  /** 角度スライダーと同じ700msの作法で、連続ドラッグを1履歴へまとめる。 */
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
    const s = get();
    set({
      proposalPositionUndoStack: [
        ...s.proposalPositionUndoStack,
        proposalPositionSnapshot(),
      ].slice(-ANGLE_HISTORY_LIMIT),
      proposalPositionRedoStack: [],
    });
  };

  const undoProposalPositionState = (): boolean => {
    const s = get();
    if (s.proposalBusy) return false;
    const previous =
      s.proposalPositionUndoStack[s.proposalPositionUndoStack.length - 1];
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
      proposalPositionLastMoved: previous.lastMoved.map((entry) => ({ ...entry })),
      proposalPositionUndoStack: s.proposalPositionUndoStack.slice(0, -1),
      proposalPositionRedoStack: [
        ...s.proposalPositionRedoStack,
        proposalPositionSnapshot(),
      ].slice(-ANGLE_HISTORY_LIMIT),
      proposalError: null,
    });
    return true;
  };

  const redoProposalPositionState = (): boolean => {
    const s = get();
    if (s.proposalBusy) return false;
    const next =
      s.proposalPositionRedoStack[s.proposalPositionRedoStack.length - 1];
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
      proposalPositionRedoStack: s.proposalPositionRedoStack.slice(0, -1),
      proposalPositionUndoStack: [
        ...s.proposalPositionUndoStack,
        proposalPositionSnapshot(),
      ].slice(-ANGLE_HISTORY_LIMIT),
      proposalError: null,
    });
    return true;
  };

  /**
   * 0°だけの角度指定は形を変えない。折りを確定する時点で空へ正規化し、
   * 折った後も「角度を変えているため折れない」という古い案内を残さない。
   */
  const clearZeroOnlyDrivers = (): void => {
    const drivers = get().drivers;
    if (drivers.size > 0 && nonZeroDriverCount(drivers) === 0) {
      set({ drivers: new Map() });
    }
  };

  /** 画面の使い方の好み(作品の中身ではないもの)を端末に覚えておく */
  const persistPrefs = () => {
    const {
      splitRatio,
      contextPanelRatio,
      mirrorDraw,
      mirrorAxis,
      pullMirror,
      wheelBehavior,
      uiTheme,
      contextHelpExpanded,
      viewerHintExpanded,
      cpHelpExpanded,
      paperHelpExpanded,
      paperColorExpanded,
    } = get();
    savePrefs({
      splitRatio,
      contextPanelRatio,
      mirrorDraw,
      // 作品内の線は保存しない。選択中なら再起動時の戻り先も初期値の縦にする。
      mirrorAxis:
        mirrorAxis.kind === "paperHorizontal"
          ? "paperHorizontal"
          : "paperVertical",
      pullMirror,
      wheelBehavior,
      uiTheme,
      contextHelpExpanded,
      viewerHintExpanded,
      cpHelpExpanded,
      paperHelpExpanded,
      paperColorExpanded,
    });
  };

  /** driversの配列表現(IPCの引数) */
  const driverList = (drivers: Map<number, number>): Driver[] =>
    [...drivers]
      .sort(([left], [right]) => left - right)
      .map(([hinge, deg]) => ({ hinge, target_angle_deg: deg }));

  /** 全ての折り線に0度(平ら)を指定したdriver列。
   * 何も指定せずに送るとRust側が前回解(warm start)を引き継ぎ、折れたままの
   * 形が返る。平らに戻したいときは必ず0度を明示する必要がある */
  const flatDrivers = (hinges: ReadonlySet<number>): Driver[] =>
    [...hinges]
      .sort((left, right) => left - right)
      .map((hinge) => ({ hinge, target_angle_deg: 0 }));

  /**
   * 角度Undo/Redoで戻した希望角から作る明示seed。
   * CanonicalではRustがこの値を候補入力に使わないが、Followとのwire互換と、
   * どの状態を復元しようとしたかの診断を保つ。権威はDocument＋希望角である。
   */
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

  /** 選択から外れて強調表示だけ消えた後も、確定待ちの計算世代を保持する。 */
  let unfinishedAngleIntentGeneration: number | null = null;

  /** 新しい角度操作をZustandへ世代付きで載せる。 */
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

  /** 古い結果が同じ世代番号を再利用しないよう、解除時も番号を進める。 */
  const cancelAngleIntent = (): void => {
    unfinishedAngleIntentGeneration = null;
    set((s) => ({
      angleIntentGeneration: s.angleIntentGeneration + 1,
      activeAngleIntent: null,
    }));
  };

  /** 今のドラッグで角度の履歴をもう積んだか(ドラッグ1回=履歴1件にする) */
  let pullPushed = false;
  /** ガイド用: 今の「引く」ドラッグで角度が実際に1度以上変わったか。 */
  let pullMovedForGuide = false;
  /** ガイド用: 細かなpointer moveを足し上げても判定できるよう、つかんだ瞬間の角度を保つ。 */
  let pullGuideStartAngle: number | null = null;

  /** 直前に履歴へ積んだ操作の目印と時刻(続けざまの操作を1件にまとめるため) */
  let lastAngleKey: string | null = null;
  let lastAngleAt = 0;

  /**
   * 固定してある折り目のつまみを利用者自身が動かしたときに、
   * 固定する角度も同じ値へ更新するための差分。固定していなければ何も返さない。
   */
  const repinned = (
    hinges: readonly number[],
    deg: number,
  ): { pinnedFolds?: ReadonlyMap<number, number> } => {
    const s = get();
    const target = hinges.filter((hinge) => s.pinnedFolds.has(hinge));
    if (target.length === 0) return {};
    const next = new Map(s.pinnedFolds);
    for (const hinge of target) next.set(hinge, deg);
    return { pinnedFolds: next };
  };

  /** いまの角度の指定と固定を、履歴へ積める形にまとめる。 */
  const angleSnapshot = (): AngleSnapshot => {
    const s = get();
    return { drivers: new Map(s.drivers), pinned: new Map(s.pinnedFolds) };
  };

  /**
   * 角度を変える直前の指定と固定を履歴へ積む(角度を変える操作の入口で必ず呼ぶ)。
   * keyが同じ操作の続き(スライダーを動かしている最中)なら積み直さないので、
   * ドラッグ1回・スライダー1回の操作が履歴1件になる。keyがnullなら常に1件。
   * 固定の付け外しは key=null で呼ぶので、1回押すごとに1件戻せる。
   */
  const pushAngleUndo = (key: string | null): void => {
    const now = Date.now();
    if (key !== null && key === lastAngleKey && now - lastAngleAt < ANGLE_GROUP_MS) {
      lastAngleAt = now; // 同じ操作の続きなので履歴は増やさない
      return;
    }
    lastAngleKey = key;
    lastAngleAt = now;
    const s = get();
    set({
      angleUndoStack: [...s.angleUndoStack, angleSnapshot()].slice(
        -ANGLE_HISTORY_LIMIT,
      ),
      angleRedoStack: [], // 新しい操作をしたらやり直しの先は消える
    });
  };

  /** 角度の履歴を捨てる(作品データを変えたとき・別の作品になったとき)。
   * 作品データの変更のほうが新しい操作になるので、「元に戻す」はそちらへ回す */
  const clearAngleHistory = (): void => {
    lastAngleKey = null;
    const s = get();
    if (s.angleUndoStack.length === 0 && s.angleRedoStack.length === 0 && s.docUndoDepth === 0) {
      return;
    }
    set({ angleUndoStack: [], angleRedoStack: [], docUndoDepth: 0 });
  };

  /**
   * いま計算へ送る角度の一覧を作る。
   *
   * 後の段が勝つ順に重ねる: 保存手順の希望 → 固定した角度 → 今の指定。
   * 固定を手順の希望より後に置くのは、利用者が「この角度のままにして」と
   * 明示したものだから。今の指定を最後に置くのは、固定した折り目の
   * つまみを利用者自身が動かしたときに、その操作を止めないため
   * (固定は「ほかの折り目のせいで動かない」という意味であって、
   * 利用者自身も動かせないという意味ではない)。
   */
  const mergedTargets = (s: AppState): Map<number, number> => {
    const merged = new Map(
      [...s.sequenceTargets].filter(([hinge]) => s.hinges.has(hinge)),
    );
    for (const [hinge, deg] of s.pinnedFolds) {
      if (s.hinges.has(hinge)) merged.set(hinge, deg);
    }
    for (const [hinge, deg] of s.drivers) {
      merged.set(hinge, deg);
    }
    return merged;
  };

  /**
   * 保存手順 → 同じ作業中の希望 → 現在操作中hard、の順で角度をまとめる。
   * 同じ辺は後の値が勝ち、hardとpreferredへ重複して載せない。
   */
  const splitDrivers = (): { hard: Driver[]; preferred: Driver[] } => {
    const s = get();
    // まとめてスライダーで動かすときは、角度を固定するのは代表の1本だけにする。
    //
    // 選んだ折り線を全て「その角度ちょうど」で固定すると、実際の紙では成り立たない。
    // 鶴の花弁折りで8本を同時に動かした実測では、紙が閉じず(閉包RMS 8.714e-3)
    // 食い込みも出た。代表1本だけを固定して残りを希望角にすると、45度でも90度でも
    // 147度でも閉包1e-15以下・食い込み0組で解け、8本の実際の角度は要求から
    // 1.6度以内に収まる。折り線どうしがわずかに違う角度を取れることが、
    // 紙が破れないために必要。残りは希望角として同じ1回の計算へ渡る。
    const moving = (s.activeAngleIntent?.hinges ?? [])
      .filter((hinge) => s.drivers.has(hinge))
      .sort((a, b) => a - b);
    const active = new Set(
      s.activeAngleIntent?.fixAll === false ? moving.slice(0, 1) : moving,
    );
    const merged = mergedTargets(s);
    const hard: Driver[] = [];
    const preferred: Driver[] = [];
    for (const [hinge, deg] of [...merged].sort(([a], [b]) => a - b)) {
      (active.has(hinge) ? hard : preferred).push({
        hinge,
        target_angle_deg: deg,
      });
    }
    return { hard, preferred };
  };

  /** 明示hardを除いた現在のpreferred一覧。解除の1回だけhardを送る経路で使う。 */
  const preferredWithout = (hardHinges: readonly number[]): Driver[] => {
    const excluded = new Set(hardHinges);
    return [...mergedTargets(get())]
      .filter(([hinge]) => !excluded.has(hinge))
      .sort(([a], [b]) => a - b)
      .map(([hinge, target_angle_deg]) => ({ hinge, target_angle_deg }));
  };

  /** 追従計算を直列化キュー経由で実行し、3D表示へ反映する。
   * coalesce=true(スライダーの連続操作・手順再生)は「最新の形が出れば良い」
   * ので待ち行列に最新1件だけを置く(runLatest)。追い越された要求は実行されない。
   * 一方、解除操作のように「その1回だけ0度を明示する」意味を持つ要求は、
   * 追い越されると意味が失われるのでFIFO(run)で必ず送る。
   * 表示専用の結果は、実行済みでも新しい要求に追い越されていれば適用しない。
   * 成功時にerrorMessageは触らない(編集側のエラー報告を消さないため) */
  /** 今の作品の設定から、たわみ計算へ渡す指定を作る(切ってあればnull=送らない) */
  const softArg = (): SoftSettings | null => {
    const s = softOf(get().display);
    return s.enabled ? s : null;
  };

  /** たわみの結果を画面の状態へ移す。切っているときはnullに戻して従来の描画へ */
  const softResult = (mesh: SoftMesh | null | undefined) => ({
    softMesh: mesh ?? null,
    softWarnings: keepIfSame(get().softWarnings, mesh?.warnings ?? []),
  });

  /** 外した折り目の控えを空にする(作品や指定が変わって前提が消えたとき)。 */
  const clearReleasedPins = (): void => {
    if (get().releasedPinHinges.length > 0) set({ releasedPinHinges: [] });
  };

  /**
   * 追従計算を1回送る。固定した折り目は「動かさない側」へ入れる。
   *
   * 解き直しの回数は**必ず回数で打ち切る。ミリ秒では打ち切らない**
   * (時間で打ち切ると計算機の速さで外れる本数が変わり、同じ操作をしても
   * 同じ形にならなくなる)。
   *
   * 流れ:
   *   1. 試す: 固定を全部そのままにして解く。折れれば終わり(計算1回)。
   *   2. 診断: 折れなければ、固定を全部「なるべく守る」側にして1回だけ解く。
   *      折り目ごとの譲り量が返るので、**どの固定が成り立っていないかが1回で分かる**
   *      (総当たりで1本ずつ試す必要がない)。
   *   3. 1本だけ外す: 外す順番のいちばん先のものを1本外して解き直す。
   *      折れれば終わり。折れなければ次の1本(深く探すときだけ)。
   *   4. どうしても折れないときは、診断の形をそのまま表示する。
   *      **操作は止めない**(CLAUDE.md §8)。
   *
   * `deepSearch=false`(つまみを動かしている最中)は1コマで「試す+診断」までにし、
   * 外す判断は次のコマへ持ち越す。1コマの計算は最大2回に収まる。
   */
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
    // 次の16ms要求がまだキューへ積まれていない間でも、新しい角度操作が
    // 始まった時点から旧世代の表示結果を採用しない。
    const requestGeneration = get().angleIntentGeneration;
    pose.reset();
    const soft = softArg();
    const position = get();
    const total = position.doc?.sequence.length ?? 0;
    const upTo = replayPosition?.upTo ?? position.currentStep ?? total;
    const replayT =
      replayPosition?.replayT ?? (position.currentStep === null ? 1 : position.playT);
    const pinnedHinges = new Set(position.pinnedFolds.keys());
    const send = (h: Driver[], p: Driver[]) =>
      mode === "Follow"
        ? ipc.poseSolve(h, p, soft, warmSeed, upTo, replayT)
        : ipc.poseSolve(h, p, soft, warmSeed, upTo, replayT, mode);
    const attempt = (h: Driver[], p: Driver[]) => {
      const call = () => send(h, p);
      return coalesce ? queue.runLatest(call) : queue.run(call);
    };
    /** いまの「外した折り目」で、動かさない側と守る側へ分けて1回解く。 */
    const attemptWithReleased = async (released: ReadonlySet<number>) => {
      const { kept, rest } = splitKeptFolds(preferred, pinnedHinges, released);
      return { kept, result: await attempt([...hard, ...kept], rest) };
    };

    let releasedOrder = [...get().releasedPinHinges];
    const released = new Set(releasedOrder);
    // 固定を外した折り目のうち、もう固定されていないものは控えから捨てる。
    for (const hinge of [...released]) {
      if (!pinnedHinges.has(hinge) && !preferred.some((d) => d.hinge === hinge)) {
        released.delete(hinge);
        releasedOrder = releasedOrder.filter((h) => h !== hinge);
      }
    }
    let { kept, result: r } = await attemptWithReleased(released);
    if (requestGeneration !== get().angleIntentGeneration) return false;

    // 深く探すときは、まず「外したままの折り目を1本戻せないか」を試す。
    // これをしないと、原因になっていた別の指定を直しても固定が戻らない。
    if (deepSearch && r.ok && r.isLatest && !keptFoldsFailed(r.value) && released.size > 0) {
      const readmit = [...releasedOrder].reverse().find((h) => released.has(h));
      if (readmit !== undefined) {
        const candidate = new Set(released);
        candidate.delete(readmit);
        const retry = await attemptWithReleased(candidate);
        if (requestGeneration !== get().angleIntentGeneration) return false;
        if (retry.result.ok && retry.result.isLatest && !keptFoldsFailed(retry.result.value)) {
          released.delete(readmit);
          releasedOrder = releasedOrder.filter((h) => h !== readmit);
          kept = retry.kept;
          r = retry.result;
        }
      }
    }

    // 折れなかったときだけ、原因を1回の計算で診断する。
    if (r.ok && r.isLatest && kept.length > 0 && keptFoldsFailed(r.value)) {
      const diagnosis = await attempt(hard, preferred);
      if (requestGeneration !== get().angleIntentGeneration) return false;
      if (diagnosis.ok && diagnosis.isLatest) {
        const diagnosedAngles = new Map(
          Object.entries(diagnosis.value.angles).map(([id, deg]) => [
            Number(id),
            deg,
          ]),
        );
        // 診断は「固定を全部ゆるめた形」なので、外す候補の並びは
        // 何本外しても変わらない。1回とった診断を使い回す。
        const candidates = pinReleaseCandidates(
          kept,
          pinnedHinges,
          diagnosedAngles,
        );
        // 折れないあいだ、外す順番の先頭から1本ずつ外して確かめる。
        // つまみを動かしている最中は1本ぶんの判断だけ残し、次のコマで確かめる。
        const limit = deepSearch ? MAX_PIN_RELEASES_ON_SETTLE : 1;
        let attemptsLeft = limit;
        // 診断の形は必ず解けている(固定を全部ゆるめた形)ので、
        // どうしても折れなかったときの表示に使う。
        r = diagnosis;
        for (const candidate of candidates) {
          if (attemptsLeft <= 0) break;
          if (released.has(candidate.hinge)) continue;
          released.add(candidate.hinge);
          releasedOrder = [...releasedOrder, candidate.hinge];
          attemptsLeft--;
          if (!deepSearch) break; // 確かめるのは次のコマで
          const verified = await attemptWithReleased(released);
          if (requestGeneration !== get().angleIntentGeneration) return false;
          if (!verified.result.ok || !verified.result.isLatest) break;
          if (!keptFoldsFailed(verified.result.value)) {
            // 1本外しただけで折れた。ここで打ち切るので、外した本数は最小。
            r = verified.result;
            break;
          }
        }
      }
    }

    if (!r.ok) {
      if (r.isLatest) fail(r.error);
      return false;
    }
    if (!r.isLatest) return false;
    const poseAngles = new Map(
      Object.entries(r.value.angles).map(([id, deg]) => [Number(id), deg]),
    );
    // 指定した角度にならなかった折り目を利用者へ知らせる。以前は15.8°違っても
    // 何も出ていなかった。区画は増やさず、既存の警告の出し口へ1行足す。
    const deviationWarnings = withFoldDeviationNotice(
      r.value.frame.warnings,
      [...hard, ...preferred],
      poseAngles,
    );
    // 固定したままでは折れず、動くことになった折り目を知らせる。
    // 出し口は上と同じで、新しい区画は作らない。
    //
    // 「外した」折り目だけを見てはいけない。どうしても折れないときは
    // 固定を全部ゆるめた形を表示するので、外した覚えのない折り目も動く。
    // 実機で確かめたところ、そのとき何も知らされないまま固定した折り目が
    // -180°から100°へ動いていた。表示している形で**実際に動いた固定**を数える。
    const movedPins = releasedPinsOf(
      preferred.filter((d) => pinnedHinges.has(d.hinge)),
      poseAngles,
    );
    const pinNotice = pinReleaseNotice(movedPins);
    const poseWarnings =
      pinNotice === null
        ? deviationWarnings
        : [...deviationWarnings, pinNotice];
    set({
      // 出発点合わせ(applyFrame=false)では形は変わらないので、手順再生が
      // 持っていた層の重なり情報を消さないよう立体表示はそのままにする
      ...(applyFrame ? { frame3d: r.value.frame } : {}),
      ...(applyFrame ? softResult(r.value.soft) : {}),
      ...(applyFrame
        ? {
            suspectHinges: keepIfSame(
              get().suspectHinges,
              r.value.suspect_hinges ?? [],
            ),
            poseWarnings: keepIfSame(get().poseWarnings, poseWarnings),
            flatFoldViolations: keepIfSame(
              get().flatFoldViolations,
              r.value.flat_fold_violations ?? [],
            ),
            poseConverged: r.value.converged,
            poseBestEffort: r.value.best_effort === true,
            poseClosureRms:
              typeof r.value.closure_rms === "number"
                ? r.value.closure_rms
                : null,
            relaxations: r.value.relaxations ?? [],
            contactDetected: r.value.contact_detected === true,
            releasedPins: keepIfSameReleasedPins(get().releasedPins, movedPins),
          }
        : {}),
      // 外している折り目の控えは表示ではなく次の計算の入力なので、
      // 形を更新しない要求(出発点合わせ)でも必ず残す。
      releasedPinHinges: keepIfSame(get().releasedPinHinges, releasedOrder),
      poseAngles,
    });
    return true;
  };

  /** record/end操作が「最後に送った計算」まで待てるようPromiseを保持する。 */
  let latestPosePromise: Promise<boolean> = Promise.resolve(true);
  const requestPoseSolve = (
    hard: Driver[],
    preferred: Driver[] = [],
    coalesce = false,
    applyFrame = true,
    warmSeed: Driver[] = [],
    mode: ipc.PoseSolveMode = "Follow",
  ): Promise<boolean> => {
    // つまみを動かしている最中(coalesce)以外は、1回で決着させたい要求なので
    // 固定を外す本数まで深く探す。
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

  /**
   * 予約済みの末尾要求を送り、待っている間にさらに新しい要求が積まれた場合も
   * 本当に最新の1件が完了するまで待つ。Pose保存だけが使う非ブロッキング待機。
   */
  const waitForLatestPose = async (): Promise<void> => {
    while (true) {
      pose.flush();
      const pending = latestPosePromise;
      await pending;
      // await中に始まった操作の末尾タイマーも、読み取り前に即時送信する。
      pose.flush();
      if (pending === latestPosePromise) return;
    }
  };

  /** 展開図の更新後、残っている角度指定で立体形状を計算し直す。
   * 折り線でなくなった辺の指定は捨てる */
  const syncPose = async (): Promise<void> => {
    // applyViewの直後に呼ばれるので、hingesは更新後の展開図から導出済み
    const hinges = get().hinges;
    const before = get().drivers;
    const kept = new Map([...before].filter(([hinge]) => hinges.has(hinge)));
    if (kept.size !== before.size) set({ drivers: kept });
    // 折り線でなくなった辺の固定も一緒に捨てる(角度の指定と同じ扱い)
    const pinnedBefore = get().pinnedFolds;
    const pinnedKept = new Map(
      [...pinnedBefore].filter(([hinge]) => hinges.has(hinge)),
    );
    if (pinnedKept.size !== pinnedBefore.size) {
      clearReleasedPins();
      set({ pinnedFolds: pinnedKept, releasedPins: [] });
    }
    // 平らのまま(指定も立体形状も無い)なら計算する必要はない
    if (kept.size === 0 && get().frame3d === null) return;
    if (kept.size === 0) {
      // 指定が全て無くなった(線の種類変更などで折り線でなくなった)場合、
      // 空のまま送ると前回の計算結果を引き継いで折れたまま残り、画面には
      // 平らへ戻す操作も出なくなる。全ての折り線へ0度を明示して平らに戻す
      await requestPoseSolve(flatDrivers(hinges));
      return;
    }
    // 展開図を編集した後は操作中の折り線がないので、全部を目標として
    // 「閉包を満たす形のうち目標にいちばん近いもの」を解く(紙が切れない)
    cancelAngleIntent();
    await requestPoseSolve([], preferredWithout([]));
  };

  // スライダーの連続操作を間引く(実行時点の最新driversを送る)。
  // 間引いてもなお計算が追いつかない場合に備え、待ち行列は最新1件だけにする
  const pose = createTrailingThrottle(POSE_THROTTLE_MS, () => {
    const { hard, preferred } = splitDrivers();
    latestPosePromise = runPoseSolve(hard, preferred, true);
  });

  /**
   * pointer up / blur / Enterでは末尾のFollow要求を必ず送り、その後に1回だけ
   * document-derived seedからcanonical再導出する。ドラッグ中の16ms Follow経路は
   * 変えず、確定した状態だけを操作順・backend warm cacheから独立させる。
   */
  const finishCurrentAngleIntent = async (): Promise<void> => {
    const generation = unfinishedAngleIntentGeneration;
    // pointer up / blur / Enterはジェスチャーの境界でもある。同じスライダーを
    // 700ms以内にもう一度動かしても、次の1ジェスチャーを同じ履歴へ混ぜない。
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

    // Followだけが失敗しても、同じ希望からの最終canonicalが成功すれば操作は
    // 完了している。無関係な後発エラーは消さないよう、Followが残した同じ文面だけ
    // 成功後に取り除く。
    const errorAfterFollow = get().errorMessage;
    const followError =
      !followSucceeded && errorAfterFollow !== errorBeforeFollow ? errorAfterFollow : null;
    // Followの形は候補生成や選択へ渡さない。確定後に利用者へ見た目の移動を
    // 知らせるためだけ、対応頂点の数値をこのローカル変数へコピーする。
    const followFrame = followSucceeded ? finishComparisonFrame(get().frame3d) : null;

    // Undo/Redoと同じ入口を使う。RustのCanonical計算がDocumentと希望角から
    // 固定候補を作る。released pinも先に捨て、最後に触ったhinge・stored warm・
    // frontendのFollow frameを入力へ残さない。
    // applyAngleSnapshotはawaitより前にactiveを解除するため、pointerupとblurが
    // 同時にこの入口を呼んでも同じ世代のcanonical solveは1回だけになる。
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
      if (movement !== null && movement >= FINISH_JUMP_NOTICE_THRESHOLD) {
        set((s) => ({
          poseWarnings: s.poseWarnings.includes(FINISH_JUMP_NOTICE)
            ? s.poseWarnings
            : [...s.poseWarnings, FINISH_JUMP_NOTICE],
        }));
      }
    }
  };

  /**
   * 履歴から取り出したdriversへ戻し、その形を展開図と手順から作り直す。
   * 追従角や3D頂点は履歴へ保存しない。RustのCanonical計算がDocumentと戻した
   * 希望角だけから固定候補を作り、stored warmを読まないため、undo/redo先を
   * 操作経路に依存せず同じ形へ戻せる。
   */
  const applyAngleSnapshot = async (next: AngleSnapshot): Promise<boolean> => {
    const drivers = new Map(next.drivers);
    // 固定も一緒に戻す。片方だけ戻すと、戻した角度と固定が食い違う。
    const pinnedFolds = new Map(next.pinned);
    clearReleasedPins();
    set({ drivers, pinnedFolds, releasedPins: [] });
    cancelAngleIntent();
    const restoreGeneration = get().angleIntentGeneration;
    pose.clearAll(); // 予約済みの間引き計算は古い指定なので捨てる

    const s = get();
    const total = s.doc?.sequence.length ?? 0;
    if (total > 0) {
      const upTo = s.currentStep ?? total;
      const t = s.currentStep === null ? 1 : s.playT;
      // 再生の基準角へ、復元するdriverと固定を同じ1回で重ねる。
      // 未固定の再生frameを一瞬表示してから追従形へ替えることもしない。
      const restored = await runReplay(
        upTo,
        t,
        false,
        true,
        drivers,
        "Canonical",
      );
      // 連打中に次のundo/redoや新しい角度操作が始まった古い復元は打ち切る。
      if (restoreGeneration !== get().angleIntentGeneration) return false;
      return restored;
    }

    return await requestPoseSolve(
      [],
      preferredWithout([]),
      false,
      true,
      snapshotSeed(drivers, s.hinges),
      "Canonical",
    );
  };

  /** 今見えている形をもう一度作り直す(たわみの指定を変えたときに使う)。
   * 手順のある作品は再生、無い作品は角度の追従計算で作る(どちらも最新1件だけ) */
  const refreshShape = (): void => {
    const s = get();
    if (!s.doc) return;
    const total = s.doc.sequence.length;
    if (total === 0) {
      void requestPoseSolve([], preferredWithout([]), true);
      return;
    }
    void runReplay(s.currentStep ?? total, s.currentStep === null ? 1 : s.playT, true);
  };

  // つまみを動かしている間も形が付いてくるよう、角度と同じ16ms間引きに乗せる
  const softShape = createTrailingThrottle(POSE_THROTTLE_MS, refreshShape);
  // 作品への保存はもう少しまとめる(つまみ1回の操作で元に戻す履歴が埋まらないように)
  /** たわみの指定にまだ作品へ書き込んでいないものがあるか */
  let softPending = false;
  /** 遅延保存へ渡す、setSoftが最後に指定した完全な表示設定。
   * 途中で別編集の古いDocumentViewが返ってきても、この値を上書きしない。 */
  let pendingSoftDisplay: DisplaySettings | null = null;
  const softSave = createTrailingThrottle(SOFT_SAVE_MS, () => {
    const display = pendingSoftDisplay;
    pendingSoftDisplay = null;
    softPending = false;
    if (display) void get().setDisplay(display);
  });

  /** 書き込み待ちのたわみの指定を今すぐ確定する(手順として残す前に呼ぶ) */
  const flushSoftSave = async (): Promise<void> => {
    if (!softPending) return;
    const display = pendingSoftDisplay;
    pendingSoftDisplay = null;
    softPending = false;
    softSave.reset(); // 予約を取り消す(同じ内容を二度送らない)
    if (display) await get().setDisplay(display);
  };

  resetThrottle = () => {
    softPending = false;
    pendingSoftDisplay = null;
    pose.clearAll();
    latestPosePromise = Promise.resolve(true);
    softShape.clearAll();
    softSave.clearAll();
    // 角度の履歴のまとめ判定も時計を持つので、一緒に初期化する
    lastAngleKey = null;
    lastAngleAt = 0;
    pullPushed = false;
    pullMovedForGuide = false;
    pullGuideStartAngle = null;
    cancelAngleIntent();
    clearAngleHistory();
  };

  /** 手順の再生結果を3D表示へ反映する。
   * coalesce=true(再生アニメーション)は「最新の形が出れば良い」ので
   * 待ち行列に最新1件だけを置く(runLatest)。追い越された要求は実行されない。
   *
   * 固定があるときは、再生結果を出発点にして既存の追従計算へ渡す。
   * sequenceReplayには固定を渡す引数が無いため、そのframeを直接表示すると
   * 手順を選ぶ・再生する・undoで戻す経路だけ固定が効かなくなるためである。 */
  const runReplay = async (
    upTo: number,
    t: number,
    coalesce = false,
    settlePins = !coalesce,
    poseOverrides: ReadonlyMap<number, number> | null = null,
    mode: ipc.PoseSolveMode = "Follow",
  ): Promise<boolean> => {
    // poseと同じく、次の角度要求がthrottle待ちでも旧再生を表示しない。
    const requestGeneration = get().angleIntentGeneration;
    const call = () => ipc.sequenceReplay(upTo, t, softArg());
    const r = await (coalesce ? queue.runLatest(call) : queue.run(call));
    if (requestGeneration !== get().angleIntentGeneration) return false;
    if (!r.ok) {
      if (r.isLatest) {
        // 再生できない状態のままでは毎コマ失敗するので、止めて理由を知らせる
        stopPlayback();
        fail(r.error);
      }
      return false;
    }
    if (!r.isLatest) return false;
    const s = get();
    const requestedAngles = [...(r.value.sequence_targets ?? [])].sort(
      (a, b) => a.hinge - b.hinge,
    );
    const sequenceTargets = new Map(
      requestedAngles.map((driver) => [driver.hinge, driver.target_angle_deg]),
    );
    const replayAngles = r.value.angles
      ? new Map(
          Object.entries(r.value.angles).map(([id, deg]) => [Number(id), deg]),
        )
      : null;
    const activePins = [...s.pinnedFolds].filter(([hinge]) => s.hinges.has(hinge));
    const replayState = {
      // upToまでの再生結果なので、作品全体のskippedは上書きしない
      replaySkipped: keepIfSame(s.replaySkipped, r.value.skipped),
      replayWarnings: keepIfSame(s.replayWarnings, r.value.warnings),
      sequenceTargets,
    };

    if (activePins.length === 0 && (poseOverrides === null || poseOverrides.size === 0)) {
      // frameが別の計算結果へ変わるときは、そのframeと組になっていた通知も
      // 必ず同時に置き換える。これをしないとundo後も「固定が動いた」という
      // 直前の追従計算の知らせだけが残る。
      set({
        ...replayState,
        frame3d: r.value.frame,
        ...softResult(r.value.soft),
        flatFoldViolations: keepIfSame(
          s.flatFoldViolations,
          r.value.flat_fold_violations ?? [],
        ),
        suspectHinges: keepIfSame(
          s.suspectHinges,
          r.value.suspect_hinges ?? [],
        ),
        poseAngles: replayAngles ?? s.poseAngles,
        poseWarnings: keepIfSame(
          s.poseWarnings,
          withFoldDeviationNotice(
            r.value.frame.warnings,
            requestedAngles,
            replayAngles ?? new Map(),
          ),
        ),
        releasedPins: keepIfSameReleasedPins(s.releasedPins, []),
        releasedPinHinges: keepIfSame(s.releasedPinHinges, []),
        relaxations: r.value.relaxations ?? [],
        poseConverged: r.value.converged ?? true,
        poseBestEffort: r.value.best_effort === true,
        poseClosureRms:
          typeof r.value.closure_rms === "number"
            ? r.value.closure_rms
            : null,
        contactDetected: r.value.contact_detected === true,
      });
      return true;
    }

    // 再生する位置が変われば、以前その形で外した固定は前提が違う。
    // 全て固定した状態からもう一度試し、外す必要がある場合だけ最小限を診断する。
    set({
      ...replayState,
      poseWarnings: [],
      releasedPins: [],
      releasedPinHinges: [],
    });
    const preferred = new Map(sequenceTargets);
    for (const [hinge, deg] of activePins) preferred.set(hinge, deg);
    for (const [hinge, deg] of poseOverrides ?? []) preferred.set(hinge, deg);
    const pending = runPoseSolve(
      [],
      driverList(preferred),
      coalesce,
      true,
      replayAngles === null ? [] : driverList(replayAngles),
      // 再生中は1コマを軽く保ち、静止した最終形で解除本数を詰める。
      // 成り立たない指定でも表示は止めない。
      settlePins,
      // sequenceReplayと固定付きsolveが必ず同じ再生位置を計算する。
      // 次のtickが先にstateを進めても、別の時点の形を混ぜない。
      { upTo, replayT: t },
      mode,
    );
    latestPosePromise = pending;
    return await pending;
  };

  // 手順の表示切替中はframe3dが直前の手順を指す。合わせ対象をその古い位置で
  // 記録しないよう、完了するまで新しい合わせ操作の開始だけを待ってもらう。
  let stepReplayGeneration = 0;
  let stepReplayPending = false;

  /**
   * 手順表示を切り替え、バックエンドの再生結果まで待つ共通処理。
   * 通常UIはこのPromiseを待たず、撮影APIだけが待つことで操作感を変えずに
   * 「番号だけ先に変わり、立体は前の手順」という撮影競合を防ぐ。
   */
  const selectStepAndWait = async (step: number | null): Promise<void> => {
    stopPlayback();
    documentSlice.internals.invalidateFoldThrough();
    // 別の手順の形を見せる操作なので、その前の形の上に引いた折り線は捨てる
    // (残すとコンテキストパネルに折りUIが出たままになり、手順の設定も出せない)
    if (get().foldDraft || get().alignDraft) {
      set({ foldDraft: null, alignDraft: null });
    }
    if (get().techniqueDraft) set({ techniqueDraft: null });
    const s = get();
    const total = s.doc?.sequence.length ?? 0;
    if (total === 0) {
      set({ currentStep: null, playT: 1 });
      return;
    }
    const upTo = step === null ? total : Math.max(0, Math.min(step, total));
    const next = step === null ? null : upTo;
    // すでに同じ形を表示しているなら描き直しを頼まない
    // (端で「次へ」を連打したときに、同じ要求が何度も飛ぶのを防ぐ)
    if (s.currentStep === next && s.playT === 1) return;
    const replayGeneration = ++stepReplayGeneration;
    stepReplayPending = true;
    set({ currentStep: next, playT: 1 });
    // 最新(null)の形も再生で作る(DocumentViewのframeと同じ内容になる)。
    // 途中まで折った表示から戻すときに、必ず最新の形へ描き直すため
    try {
      await runReplay(upTo, 1);
    } finally {
      if (stepReplayGeneration === replayGeneration) stepReplayPending = false;
    }
  };

  /** 手順のある作品では、立体表示は手順の再生結果で表す(角度スライダーは
   * 手順の無い作品の確認用)。
   * 最新表示中(currentStep=null)はviewに自動再生の結果(立体・飛ばした手順・
   * 警告はview.warningsへ合流済み)が載っているので、再生は呼ばない。
   * 途中の手順を表示しているときだけ、その手順までを再生し直す */
  const syncSequence = async (view: DocumentView): Promise<void> => {
    // 表示中の手順番号はapplyViewで手順数まで詰めてある
    const step = get().currentStep;
    if (step === null) {
      const hasPins = [...get().pinnedFolds.keys()].some((hinge) =>
        get().hinges.has(hinge),
      );
      // DocumentViewの自動再生frameにも固定は含まれない。固定があるときは
      // runReplayへ集約し、再生の基準角から固定付きで解き直す。
      if (hasPins) {
        set({ replaySkipped: [], replayWarnings: [] });
        await runReplay(view.doc.sequence.length, 1, true, true);
        return;
      }
      const viewAngles = new Map(
        Object.entries(view.angles ?? {}).map(([id, deg]) => [Number(id), deg]),
      );
      set((s) => ({
        frame3d: view.frame,
        replaySkipped: [],
        replayWarnings: [],
        poseWarnings: keepIfSame(
          s.poseWarnings,
          withFoldDeviationNotice(
            view.frame?.warnings ?? [],
            view.sequence_targets ?? [],
            viewAngles,
          ),
        ),
        releasedPins: keepIfSameReleasedPins(s.releasedPins, []),
        releasedPinHinges: keepIfSame(s.releasedPinHinges, []),
        suspectHinges: keepIfSame(
          s.suspectHinges,
          view.suspect_hinges ?? [],
        ),
      }));
      // 自動再生の結果にはたわみの網が入っていないので、たわみを使うときだけ
      // 同じ形をもう一度たわませて描き直す(切っていれば今までどおり1往復のまま)
      if (softArg()) await runReplay(view.doc.sequence.length, 1, true);
      return;
    }
    // 描き直すのはその手順を折り終えた形(t=1)。一時停止していた途中の進み具合を
    // 残すと、次に再生したとき表示が一度巻き戻ってから折り直される
    set({ playT: 1 });
    await runReplay(step, 1, true, true);
  };

  /** 次のコマを予約し、取り消す手続きを返す。画面のある環境では画面更新に
   * 合わせ、無い環境(テスト)ではタイマーで代用する */
  const scheduleFrame = (cb: (ts: number) => void): (() => void) => {
    if (typeof requestAnimationFrame === "function") {
      const id = requestAnimationFrame(cb);
      return () => cancelAnimationFrame(id);
    }
    const timer = setTimeout(() => cb(Date.now()), FALLBACK_FRAME_MS);
    return () => clearTimeout(timer);
  };

  /** 予約中のコマの取り消し手続き(nullなら予約していない) */
  let cancelFrame: (() => void) | null = null;
  /** 前のコマの時刻(0なら1コマ目。経過時間0として扱う) */
  let lastTs = 0;

  /** 再生を止める(予約中のコマも取り消す) */
  const stopPlayback = (): void => {
    cancelFrame?.();
    cancelFrame = null;
    if (get().playing) set({ playing: false });
  };

  /** 一斉表示の入力だけを0..=100へ正規化する。非有限値は現在値を保つ。 */
  const normalizedFoldAllPercent = (percent: number): number | null =>
    Number.isFinite(percent) ? Math.max(0, Math.min(100, percent)) : null;

  /**
   * 一斉表示の計算を専用の最新優先queueへ送る。
   *
   * warm seedはtaskが実際に始まる時点で読む。実行中の応答をtask内で先に
   * Zustandへ置いてからreturnするため、次の待機要求が必ず直前の実角を使える。
   * frameはsession・request・docEpochがすべて現在のときだけ採用する。
   */
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
      set((s) =>
        s.foldAllPreview?.session === session &&
        !s.foldAllPreview.returning &&
        request === foldAllRequestGeneration &&
        s.docEpoch === s.foldAllPreview.returnState.docEpoch
          ? {
              foldAllPreview: {
                ...s.foldAllPreview,
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
        set((s) =>
          s.foldAllPreview?.session === session &&
          !s.foldAllPreview.returning &&
          request === foldAllRequestGeneration
            ? {
                foldAllPreview: {
                  ...s.foldAllPreview,
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
    set((s) =>
      s.foldAllPreview?.session === session &&
      !s.foldAllPreview.returning &&
      request === foldAllRequestGeneration
        ? {
            frame3d: outcome.frame,
            // 一斉表示はたわみの網を返さない。前の網を残すと新しいframeが隠れる。
            softMesh: null,
            softWarnings: [],
            foldAllPreview: {
              ...s.foldAllPreview,
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

  // 入力は16msごとに送り、計算が追いつかなくても待機は最後の1件だけにする。
  const foldAllShape = createTrailingThrottle(FOLD_ALL_THROTTLE_MS, () => {
    const active = get().foldAllPreview;
    if (active === null || active.returning) return;
    void runFoldAllPreview(
      active.percent,
      active.session,
      foldAllRequestGeneration,
    );
  });

  /** 新規・開くの成功時だけ、一斉表示と遅れて届く計算を即時に破棄する。 */
  const discardFoldAllPreview = (): FoldAllReturnState | null => {
    const active = get().foldAllPreview;
    if (active === null) return null;
    foldAllSessionGeneration++;
    foldAllRequestGeneration++;
    foldAllExitGeneration++;
    foldAllShape.clearAll();
    cancelAngleIntent();
    set({ foldAllPreview: null });
    return active.returnState;
  };

  /**
   * 3D座標の控えを使わず、通常表示を展開図・手順・一時角度から作り直す。
   * 再計算が成功するまでは専用表示の目印と操作制限を残す。連打された場合は、
   * 最後に始まった復帰だけが目印を消す。
   */
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
    cancelAngleIntent();
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
    const s = get();
    if (s.doc === null) {
      if (token === foldAllExitGeneration) set({ foldAllPreview: null });
      return token === foldAllExitGeneration;
    }
    const total = s.doc.sequence.length;
    let restored: boolean;
    if (total > 0) {
      const upTo =
        previous.currentStep === null
          ? total
          : Math.max(0, Math.min(previous.currentStep, total));
      restored = await runReplay(
        upTo,
        previous.playT,
        false,
        true,
        new Map(s.drivers),
      );
    } else {
      const normalTargets = preferredWithout([]);
      restored = await requestPoseSolve(
        [],
        normalTargets.length > 0 ? normalTargets : flatDrivers(s.hinges),
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

  const restoreAfterFoldAllPreview = (restoreUi = true): Promise<boolean> => {
    let announceDone!: () => void;
    const announced = new Promise<void>((resolve) => {
      announceDone = resolve;
    });
    latestFoldAllRestorePromise = announced;
    const pending = restoreAfterFoldAllPreviewOnce(restoreUi);
    void pending.then(announceDone, announceDone);
    return pending;
  };

  /** 復帰中に後から保存・書き出しが始まっても、復帰応答を追い越させない。 */
  const waitForFoldAllRestore = async (): Promise<void> => {
    while (true) {
      const pending = latestFoldAllRestorePromise;
      await pending;
      if (pending === latestFoldAllRestorePromise) return;
    }
  };

  resetFoldAllRuntime = () => {
    foldAllSessionGeneration++;
    foldAllRequestGeneration++;
    foldAllExitGeneration++;
    foldAllEntering = false;
    foldAllEnterGeneration++;
    latestFoldAllRestorePromise = Promise.resolve();
    foldAllShape.clearAll();
    set({ foldAllPreview: null });
  };

  /** 手順操作を1回だけ送り、その要求自身の成否を返す。
   * 同一位置のMoveStepは成功しても作品を変えないため、再生・提案・角度履歴も
   * 変更しない。通常操作だけは従来どおり先に再生を止め、角度履歴を消す。 */
  const applySequenceChange = (
    op: SeqOp,
    samePosition = false,
  ): Promise<boolean> => {
    if (!samePosition) {
      stopPlayback();
    documentSlice.internals.invalidateFoldThrough();
    }
    return applyDocChangeResult(
      () => ipc.sequenceApply(op),
      false,
      samePosition,
      !samePosition,
    );
  };

  /** 再生の1コマ。進み具合を計算し、その時点の形を(最新1件だけの)要求で描く */
  const tick = (ts: number): void => {
    cancelFrame = null;
    const s = get();
    if (!s.playing) return;
    const total = s.doc?.sequence.length ?? 0;
    const dt = lastTs === 0 ? 0 : ts - lastTs;
    lastTs = ts;
    const next = advancePlayback(
      { step: s.currentStep ?? 0, t: s.playT, playing: true },
      dt,
      total,
    );
    set({ currentStep: next.step, playT: next.t, playing: next.playing });
    void runReplay(next.step, next.t, true, !next.playing);
    if (next.playing) cancelFrame = scheduleFrame(tick);
  };

  /** 初回案内の既読状態だけを端末へ覚える(作品の内容には含めない)。 */
  const updateOnboarding = (patch: Partial<typeof onboarding>) => {
    onboarding = { ...onboarding, ...patch };
    saveOnboarding(onboarding);
  };

  const documentSlice = createDocumentSlice<AppState>(set, get, {
    queue,
    runViewCommand,
    applyDocChange,
    fail,
    invalidateFoldAllEntry: () => {
      foldAllEnterGeneration++;
    },
    flushSoftSave,
    waitForFoldAllRestore,
    restoreAfterFoldAllPreview,
    stopPlayback,
    isStepReplayPending: () => stepReplayPending,
    persistPrefs,
    relaxationNotices,
    clearZeroOnlyDrivers,
  });

  return {
    ...documentSlice.slice,
    hinges: new Set<number>(),
    frame3d: null,
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
    recovery: null,
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
    exportOpen: false,
    exportKind: "CpSvg",
    exportIncludeAux: true,
    exportLongSide: DEFAULT_PNG_LONG_SIDE,
    exportBusy: false,
    exportError: null,
    exportSavedPath: null,
    newDialogOpen: false,
    newPaperDraft: DEFAULT_NEW_PAPER,
    display: DEFAULT_DISPLAY,
    splitRatio: prefs.splitRatio,
    contextPanelRatio: prefs.contextPanelRatio,
    mirrorDraw: prefs.mirrorDraw,
    mirrorAxis: { kind: prefs.mirrorAxis },
    mirrorAxisNotice: null,
    pullMirror: prefs.pullMirror,
    wheelBehavior: prefs.wheelBehavior,
    uiTheme: prefs.uiTheme,
    contextHelpExpanded: prefs.contextHelpExpanded,
    viewerHintExpanded: prefs.viewerHintExpanded,
    cpHelpExpanded: prefs.cpHelpExpanded,
    paperHelpExpanded: prefs.paperHelpExpanded,
    paperColorExpanded: prefs.paperColorExpanded,
    guideOpen: !onboarding.guideComplete,
    guideStep: 0,
    helpOpen: false,
    helpChapterId: "overview",
    helpQuery: "",
    operationStage: 0,
    lineInputStart: null,
    paperActionTipVisible: false,
    paperActionTipExpanded: false,

    setPullMirror: (on) => {
      set({ pullMirror: on });
      // 切ったら、いま一緒に動かしている相手もその場で外す(次のドラッグを待たない)
      if (!on) set({ pullMirrorHinge: null });
      persistPrefs();
    },

    setWheelBehavior: (behavior) => {
      set({ wheelBehavior: behavior });
      persistPrefs();
    },

    setUiTheme: (theme) => {
      set({ uiTheme: theme });
      persistPrefs();
    },

    toggleContextHelp: () => {
      set((s) => ({ contextHelpExpanded: !s.contextHelpExpanded }));
      persistPrefs();
    },

    toggleViewerHint: () => {
      set((s) => ({ viewerHintExpanded: !s.viewerHintExpanded }));
      persistPrefs();
    },

    toggleCpHelp: () => {
      set((s) => ({ cpHelpExpanded: !s.cpHelpExpanded }));
      persistPrefs();
    },

    togglePaperHelp: () => {
      set((s) => ({ paperHelpExpanded: !s.paperHelpExpanded }));
      persistPrefs();
    },

    togglePaperColor: () => {
      set((s) => ({ paperColorExpanded: !s.paperColorExpanded }));
      persistPrefs();
    },

    openGuide: () => set({ guideOpen: true, guideStep: 0 }),

    openHelp: () => set({ helpOpen: true }),

    closeHelp: () => set({ helpOpen: false }),

    selectHelpChapter: (chapterId) => set({ helpChapterId: chapterId }),

    setHelpQuery: (query) => set({ helpQuery: query }),

    dismissGuide: () => {
      set({ guideOpen: false });
      updateOnboarding({ guideComplete: true });
    },

    completeGuideAction: (action) => {
      const s = get();
      if (!s.guideOpen || s.guideStep >= 4 || GUIDE_ACTIONS[s.guideStep] !== action) {
        return;
      }
      const next = (s.guideStep + 1) as GuideStep;
      set({ guideStep: next });
      // 最後の操作を実際にできた時点で既読にする。完了カードは閉じるまで残す。
      if (next === 4) updateOnboarding({ guideComplete: true });
    },

    setOperationStage: (stage) => {
      const next = Math.max(0, Math.floor(stage));
      if (get().operationStage !== next) set({ operationStage: next });
    },

    setLineInputStart: (start) => {
      if (get().lineInputStart !== start) set({ lineInputStart: start });
    },

    showPaperActionTip: () => {
      const firstTime = !onboarding.paperActionTipSeen;
      set((s) => ({
        paperActionTipVisible: true,
        // 初回だけ詳しく開く。その後の紙選択では小さなヒントから始める。
        paperActionTipExpanded: s.paperActionTipVisible
          ? s.paperActionTipExpanded
          : firstTime,
      }));
      if (firstTime) updateOnboarding({ paperActionTipSeen: true });
    },

    collapsePaperActionTip: () => set({ paperActionTipExpanded: false }),

    expandPaperActionTip: () =>
      set({ paperActionTipVisible: true, paperActionTipExpanded: true }),

    hidePaperActionTip: () =>
      set({ paperActionTipVisible: false, paperActionTipExpanded: false }),

    // 「元に戻す」は“直前にした操作”を戻す。折り角度の変更は作品データでは
    // ないので作品側の履歴(edit_undo)に載らない。そこで角度の履歴を先に見て、
    // 残っていればそれを戻す(角度を変えた直後に線の追加が消えないように)
    undo: async () => {
      if (get().foldAllPreview !== null) {
        // 一斉表示そのものはUndoへ積まない。最初の1回は通常表示へ戻るだけ。
        await restoreAfterFoldAllPreview(true);
        return;
      }
      stopPlayback();
    documentSlice.internals.invalidateFoldThrough();
      const s = get();
      // 提案ダイアログの場所操作は作品へ保存しないため、開いている間だけ先に戻す。
      // 計算中は、その要求の入力と画面を食い違わせないため何も戻さない。
      if (s.proposalStep !== null && s.proposalBusy) return;
      if (s.proposalStep !== null && undoProposalPositionState()) return;
      const prev = s.angleUndoStack[s.angleUndoStack.length - 1];
      if (prev !== undefined) {
        lastAngleKey = null; // 戻した後の操作は必ず新しい1件にする
        set({
          angleUndoStack: s.angleUndoStack.slice(0, -1),
          angleRedoStack: [...s.angleRedoStack, angleSnapshot()].slice(
            -ANGLE_HISTORY_LIMIT,
          ),
          errorMessage: null,
        });
        await applyAngleSnapshot(prev);
        return;
      }
      await runViewCommand(() => ipc.editUndo(), false);
      // 戻せたぶんだけ「やり直し」は作品側を先に進める(操作と逆の順に戻す)
      if (get().errorMessage === null) set({ docUndoDepth: get().docUndoDepth + 1 });
    },

    redo: async () => {
      if (get().foldAllPreview !== null) {
        await restoreAfterFoldAllPreview(true);
        return;
      }
      stopPlayback();
    documentSlice.internals.invalidateFoldThrough();
      if (get().proposalStep !== null && get().proposalBusy) return;
      if (get().proposalStep !== null && redoProposalPositionState()) return;
      // 作品データを戻したぶんが残っていれば、そちらを先にやり直す
      if (get().docUndoDepth > 0) {
        await runViewCommand(() => ipc.editRedo(), false);
        if (get().errorMessage === null) {
          set({ docUndoDepth: Math.max(0, get().docUndoDepth - 1) });
        }
        return;
      }
      const s = get();
      const next = s.angleRedoStack[s.angleRedoStack.length - 1];
      if (next === undefined) {
        await runViewCommand(() => ipc.editRedo(), false);
        return;
      }
      lastAngleKey = null;
      set({
        angleRedoStack: s.angleRedoStack.slice(0, -1),
        angleUndoStack: [...s.angleUndoStack, angleSnapshot()].slice(
          -ANGLE_HISTORY_LIMIT,
        ),
        errorMessage: null,
      });
      await applyAngleSnapshot(next);
    },

    applySequenceOp: async (op) => {
      if (get().foldAllPreview !== null) {
        if (!(await restoreAfterFoldAllPreview(true))) return;
      }
      await applySequenceChange(op);
    },

    selectStep: (step) => {
      if (get().foldAllPreview === null) {
        void selectStepAndWait(step);
        return;
      }
      void restoreAfterFoldAllPreview(true).then((restored) => {
        if (restored) void selectStepAndWait(step);
      });
    },

    selectStepForCapture: async (step) => {
      if (get().foldAllPreview !== null) {
        if (!(await restoreAfterFoldAllPreview(true))) return;
      }
      await selectStepAndWait(step);
    },

    stepBy: (delta) => {
      const s = get();
      const total = s.doc?.sequence.length ?? 0;
      if (total === 0) return;
      // 最新表示(null)からのコマ送りは、最終手順を基準に数える
      const from = s.currentStep ?? total;
      s.selectStep(Math.max(0, Math.min(from + delta, total)));
    },

    togglePlay: () => {
      const s = get();
      if (s.foldAllPreview !== null) {
        void restoreAfterFoldAllPreview(true).then((restored) => {
          if (restored) get().togglePlay();
        });
        return;
      }
      if (s.playing) {
        stopPlayback();
        return;
      }
      const total = s.doc?.sequence.length ?? 0;
      const next = startPlayback(s.currentStep, s.playT, total);
      if (!next.playing) return;
    documentSlice.internals.invalidateFoldThrough();
      // 再生中は形が刻々と変わるので、引きかけの折り線・技法の下ごしらえは捨てる
      set({ foldDraft: null, alignDraft: null, techniqueDraft: null });
      set({ currentStep: next.step, playT: next.t, playing: true });
      lastTs = 0; // 止めていた間の時間は進めない(1コマ目の経過時間は0)
      cancelFrame = scheduleFrame(tick);
    },

    beginPull: (hinge, angles, mirrorHinge = null) => {
      if (!get().doc) return;
    documentSlice.internals.invalidateFoldThrough();
      pullPushed = false; // このドラッグではまだ履歴を積んでいない
      pullMovedForGuide = false;
      pullGuideStartAngle =
        angles.get(hinge) ?? get().drivers.get(hinge) ?? get().poseAngles.get(hinge) ?? 0;
      // 左右同時を切っている間は相手を覚えない(切替が次のドラッグから必ず効く)
      set({
        pullHinge: hinge,
        pullMirrorHinge: get().pullMirror ? mirrorHinge : null,
        errorMessage: null,
      });
      // 今見えている実角は固定条件ではなくwarm startとしてだけ渡す。
      // 保存済みの手順角はpreferredのままなので、引き始めても希望を失わない。
      if (angles.size > 0) {
        void requestPoseSolve(
          [],
          preferredWithout([]),
          false,
          false, // 形は今のまま(層の重なり表示を保つ)
          driverList(new Map(angles)),
        );
      }
    },

    pullTo: (deg) => {
      const { pullHinge, pullMirrorHinge } = get();
      if (pullHinge === null) return;
      // ドラッグ1回で履歴1件。つかんでから離すまでの最初の1回だけ積む
      if (!pullPushed) {
        pullPushed = true;
        pushAngleUndo(null);
      }
      // 左右対称の相手も同じ角度で動かす(鶴の両羽が一緒に開く)。
      // 2本まとめて1回の追従計算にするので、送る回数は片側だけのときと同じ
      const drivers = new Map(get().drivers);
      if (pullGuideStartAngle !== null && Math.abs(deg - pullGuideStartAngle) >= 1) {
        pullMovedForGuide = true;
      }
      drivers.set(pullHinge, deg);
      if (pullMirrorHinge !== null) drivers.set(pullMirrorHinge, deg);
      set({ drivers });
      // 引いている折り線(と対称の相手)だけを固定し、以前の指定は追従させる
      activateAngleIntent(
        pullMirrorHinge === null ? [pullHinge] : [pullHinge, pullMirrorHinge],
      );
      pose.schedule();
    },

    endPull: () => {
      if (get().pullHinge !== null) {
        set({ pullHinge: null, pullMirrorHinge: null });
        if (pullMovedForGuide) get().completeGuideAction("pull");
      }
      pullMovedForGuide = false;
      pullGuideStartAngle = null;
      void finishCurrentAngleIntent();
    },

    setDriverAngle: (hinge, deg) => {
      if (get().foldAllPreview !== null) return;
    documentSlice.internals.invalidateFoldThrough();
      const before = get().drivers.get(hinge) ?? get().poseAngles.get(hinge) ?? 0;
      // スライダーを動かしている間の細かい変更は1件にまとめる
      pushAngleUndo(`angle:${hinge}`);
      // 画面の反応を優先し、指定はその場で反映してから計算を間引いて依頼する
      const drivers = new Map(get().drivers);
      drivers.set(hinge, deg);
      // 固定した折り目のつまみを利用者自身が動かしたときは、固定する角度も
      // その値へ更新する(固定は「ほかの折り目のせいで動かない」という意味)。
      set({ drivers, ...repinned([hinge], deg) });
      if (Math.abs(deg - before) >= 1) get().completeGuideAction("angle");
      // いま動かしている1本だけを固定する。以前に指定した折り線まで固定すると
      // 内部頂点まわりの拘束と両立せず、面が離れて紙が切れて見える
      activateAngleIntent([hinge]);
      pose.schedule();
    },

    setDriverAngles: (hinges, deg) => {
      if (get().foldAllPreview !== null) return;
    documentSlice.internals.invalidateFoldThrough();
      const valid = [...new Set(hinges)]
        .filter((hinge) => get().hinges.has(hinge))
        .sort((a, b) => a - b);
      if (valid.length === 0) return;
      // 一括スライダーの連続変更も、選択した折り目の組ごとに履歴1件へまとめる。
      pushAngleUndo(`angles:${valid.join(",")}`);
      const s = get();
      const drivers = new Map(s.drivers);
      const changedForGuide = valid.some((hinge) => {
        const before = s.drivers.get(hinge) ?? s.poseAngles.get(hinge) ?? 0;
        return Math.abs(deg - before) >= 1;
      });
      for (const hinge of valid) drivers.set(hinge, deg);
      set({ drivers, ...repinned(valid, deg) });
      if (changedForGuide) get().completeGuideAction("angle");
      // 動かしている折り目は選んだ全部(3Dで全部が水色に光る)。
      // そのうち角度を固定するのは代表1本だけ(fixAll=false)。
      activateAngleIntent(valid, false);
      pose.schedule();
    },

    finishAngleIntent: finishCurrentAngleIntent,

    clearDriver: (hinge) => {
      if (get().foldAllPreview !== null) return;
      const drivers = new Map(get().drivers);
      if (!drivers.delete(hinge)) return;
    documentSlice.internals.invalidateFoldThrough();
      pushAngleUndo(null); // 1回押すごとに履歴1件
      set({ drivers });
      const generation = activateAngleIntent([hinge]);
      // 指定を消しただけだと、この折り線は前回の計算結果(warm start)を
      // 引き継いで折れたまま残る。1回だけ0度(平ら)を明示して送り、
      // 次回以降は残りの指定だけで計算する(「全て平らに戻す」と同じ考え方)
      void requestPoseSolve(
        [{ hinge, target_angle_deg: 0 }],
        preferredWithout([hinge]),
      ).finally(() => {
        if (get().activeAngleIntent?.generation === generation) {
          cancelAngleIntent();
        }
      });
    },

    clearDrivers: () => {
      if (get().foldAllPreview !== null) return;
      // 再生tickがこの0度計算より後に積まれると、正しい平坦結果が古い応答として
      // 捨てられるか、再生結果で折れた形へ上書きされる。予約中のコマを先に止め、
      // 「全て平らに戻す」を最後の表示要求にする。
      stopPlayback();
      const hinges = get().hinges;
    documentSlice.internals.invalidateFoldThrough();
      if (get().drivers.size > 0 || get().pinnedFolds.size > 0) pushAngleUndo(null);
      // 全部を平らへ戻すので、角度を固定したままにはできない(0度以外の固定は
      // 「平らに戻す」と矛盾する)。固定も一緒に外す。
      clearReleasedPins();
      set({ drivers: new Map(), pinnedFolds: new Map(), releasedPins: [] });
      cancelAngleIntent();
      // 全ての折り線を0度に固定する形は必ず閉じる(平ら)ので全部hardでよい
      void requestPoseSolve(flatDrivers(hinges));
    },

    enterFoldAllPreview: async () => {
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
        stopPlayback();
    documentSlice.internals.invalidateFoldThrough();
        pose.clearAll();
        softShape.clearAll();
        cancelAngleIntent();
        // 予約済みの丸み変更を、専用表示を公開する前に確定する。公開後に
        // setDisplayが通常編集として専用表示を閉じる競合を作らないためである。
        await flushSoftSave();
        if (enterToken !== foldAllEnterGeneration) return;
        // 入口を押す前から実行中だった展開図・手順・Undo等の応答を待つ。
        // 待っている間に別の作品変更が始まった場合も、安定するまで追いかける。
        while (true) {
          const changesBeforeEntry = commandService.latestDocChange();
          await changesBeforeEntry;
          if (enterToken !== foldAllEnterGeneration) return;
          if (changesBeforeEntry === commandService.latestDocChange()) break;
        }

        const before = get();
        if (
          before.foldAllPreview !== null ||
          before.doc === null ||
          before.hinges.size === 0
        ) {
          return;
        }
        stopPlayback();
    documentSlice.internals.invalidateFoldThrough();
        pose.clearAll();
        softShape.clearAll();
        cancelAngleIntent();
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
          // 3Dを直接引く道具のまま一斉表示へ入れない。専用表示を閉じたら戻す。
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
          // 通常姿勢の注意や赤線を一斉形へ重ねない。一斉表示の診断は専用stateで示す。
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
    },

    setFoldAllPercent: (percent) => {
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
    },

    finishFoldAllPercent: () => {
      const active = get().foldAllPreview;
      if (active === null || active.returning) return;
      if (active.percent === 0) {
        // 0%は単なる平面計算ではなく、利用者が一斉表示へ入る直前に見ていた
        // 手順位置・角度指定・選択から通常形を作り直す契約である。
        void restoreAfterFoldAllPreview(true);
        return;
      }
      foldAllShape.flush();
    },

    leaveFoldAllPreview: async () => {
      await restoreAfterFoldAllPreview(true);
    },

    togglePinnedFold: (hinge) => {
      const s = get();
      if (s.foldAllPreview !== null) return;
      if (!s.hinges.has(hinge)) return;
      get().setPinnedFolds([hinge], !s.pinnedFolds.has(hinge));
    },

    setPinnedFolds: (hinges, pinned) => {
      const s = get();
      if (s.foldAllPreview !== null) return;
      const valid = [...new Set(hinges)].filter((hinge) => s.hinges.has(hinge));
      if (valid.length === 0) return;
      const changed = valid.some(
        (hinge) => s.pinnedFolds.has(hinge) !== pinned,
      );
      if (!changed) return;
      // 固定の付け外しは1回押すごとに1件戻せるようにする(まとめない)
      pushAngleUndo(null);
      const next = new Map(s.pinnedFolds);
      for (const hinge of valid) {
        if (pinned) {
          // 固定する角度は、画面のつまみが示している値と同じ決め方で取る。
          // 表示は整数だが、ここでは丸めずそのままの値を覚える。
          next.set(
            hinge,
            s.drivers.get(hinge) ??
              s.sequenceTargets.get(hinge) ??
              s.poseAngles.get(hinge) ??
              0,
          );
        } else {
          next.delete(hinge);
        }
      }
      // 外した固定の控えは、固定そのものが変わった時点で作り直す
      clearReleasedPins();
      set({ pinnedFolds: next, releasedPins: [] });
      // 固定を付け外しした形をすぐ見せる。動かしている折り目は無いので、
      // 全部を「なるべく守る」側にして1回だけ解き直す。
      cancelAngleIntent();
      void requestPoseSolve([], preferredWithout([]));
    },

    recordPoseStep: async () => {
      const before = get();
      if (before.foldAllPreview !== null) return;
      if (!before.doc || before.playing || before.hinges.size === 0) {
        set({ errorMessage: poseRecordReason(before) });
        return;
      }
      // 16ms待ちの末尾solveを今すぐ送り、実行中の最新要求まで待ってから実角を読む。
      await waitForLatestPose();
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
      // 保存IPCを待つ間に始まった新しい角度操作を、完了処理で消さないための世代。
      const recordedGeneration = solved.angleIntentGeneration;
      // SIM-015: たわみは「硬さ・膨らみの強さ」というパラメータとしてだけ残す
      // (頂点の位置は保存しない)。書き込み待ちの指定があればここで先に確定させ、
      // 仕上げの手順と同じ作品ファイルへ必ず一緒に入るようにする
      await flushSoftSave();
      if (get().errorMessage !== null) return;
      const latest = get();
      if (!latest.doc) return;
      // 記録した形をそのまま見せる(手順の再生結果が最新の表示になる)
      set({ currentStep: null, errorMessage: null });
      await get().applySequenceOp({
        type: "PushStep",
        step: buildPoseStep(latest.doc, angles),
      });
      // 手順として残ったので、一時的な角度指定は役目を終える。
      // ここで平らに戻す計算は送らない(再生結果の立体表示を消さないため)
      if (get().errorMessage !== null) return;
      if (get().angleIntentGeneration === recordedGeneration) {
        set({ drivers: new Map() });
        cancelAngleIntent();
        return;
      }
      // 保存IPCを待つ間に始まった操作は、新しいDocumentViewの自動再生よりも
      // 利用者の現在操作を優先する。たわみ表示の再生が待機中poseを追い越しても、
      // 更新後の保存済み希望とdriversから必ず最後の形をもう一度求める。
      const { hard, preferred } = splitDrivers();
      await requestPoseSolve(
        hard,
        preferred,
        true,
        true,
        driverList(new Map(get().poseAngles)),
      );
    },

    checkRecovery: async () => {
      // 見つからなくても普通の起動なので、失敗しても利用者へ何も出さない
      const r = await queue.run(() => ipc.recoveryCheck());
      if (r.ok && r.value) set({ recovery: r.value });
    },

    resolveRecovery: async (accept) => {
      if (get().recovery === null) return;
      // 答えは1回きり。先に閉じてダイアログの二度押しを防ぐ
      set({ recovery: null });
      if (!accept) {
        const r = await queue.run(() => ipc.recoveryRestore(false));
        if (!r.ok) fail(r.error);
        return;
      }
      // 別の作品に入れ替わるので、新規・開くと同じ扱いで反映する
      await runViewCommand(async () => {
        const view = await ipc.recoveryRestore(true);
        if (!view) throw "作業中だった内容が見つかりませんでした";
        return view;
      }, true);
    },

    openProposal: () => {
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
    },

    closeProposal: () => {
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
    },

    setProposalStep: (step) => set({ proposalStep: step }),

    // 長さ・太さ・枝分かれを触ったら前の候補は別物になるので捨てる。
    // 紙位置は残った葉IDだけ保ち、別の葉の指定まで捨てない。
    setProposalSkeleton: (skeleton) => {
      const leafIds = new Set(leafNodes(skeleton).map((node) => node.id));
      const s = get();
      lastProposalPositionKey = null;
      set({
        proposalSkeleton: skeleton,
        proposalCandidates: [],
        proposalSelected: null,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalPaperSpecified: s.proposalPaperSpecified.filter((entry) =>
          leafIds.has(entry.leaf_id),
        ),
        proposalPositionLastMoved: s.proposalPositionLastMoved.filter((entry) =>
          leafIds.has(entry.leaf_id),
        ),
        // 骨格の構造変更は場所だけの履歴へ混ぜない。
        proposalPositionUndoStack: [],
        proposalPositionRedoStack: [],
      });
    },

    setProposalTipPosition: (leafId, position) => {
      const s = get();
      if (s.proposalBusy) return;
      const node = leafNodes(s.proposalSkeleton).find((leaf) => leaf.id === leafId);
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
      const paperExists = s.proposalPaperSpecified.some(
        (entry) => entry.leaf_id === leafId,
      );
      set({
        proposalSkeleton: setTipPos(s.proposalSkeleton, leafId, fit),
        proposalCandidates: [],
        proposalSelected: null,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalPositionLastMoved:
          fit !== null
            ? setLastMovedSource(
                s.proposalPositionLastMoved,
                leafId,
                "completion",
              )
            : paperExists
              ? setLastMovedSource(s.proposalPositionLastMoved, leafId, "paper")
              : s.proposalPositionLastMoved.filter(
                  (entry) => entry.leaf_id !== leafId,
                ),
        proposalError: null,
      });
    },

    generateProposal: async () => {
      const s = get();
      if (s.proposalBusy) return;
      const paper = s.doc?.paper ?? FALLBACK_PAPER;
      const seed = s.proposalSeed;
      const requestSkeleton = proposalRequestSkeleton(
        s.proposalSkeleton,
        s.proposalPaperSpecified,
        s.proposalPositionLastMoved,
        paper,
      );
      // 提案の計算は作品の状態を読まない独立処理。直列化キューに載せると
      // 数百msの計算の間だけ編集が止まるので、ここは載せずに直接呼ぶ
      await runProposalGeneration(requestSkeleton, paper, seed, (list) => ({
        proposalCandidates: list,
        proposalSelected: list.length > 0 ? 0 : null,
        proposalPaperSource: null,
        proposalPaperPositions: [],
        proposalStep: list.length > 0 ? "candidates" : "skeleton",
        proposalError:
          list.length > 0 ? null : "候補を作れませんでした。骨格を変えてみてください",
      }));
    },

    selectProposalCandidate: (index) => {
      const list = get().proposalCandidates;
      if (index < 0 || index >= list.length) return;
      set({
        proposalSelected: index,
        proposalPaperSource: null,
        proposalPaperPositions: [],
      });
    },

    openProposalPaperPositionEditor: () => {
      const s = get();
      const source = s.proposalSelected;
      const candidate = source === null ? undefined : s.proposalCandidates[source];
      if (!candidate) return;
      const positions = paperEditorPositions(
        candidate,
        s.proposalSkeleton,
        s.proposalPaperSpecified,
      );
      if (positions.length === 0) return;
      set({
        proposalStep: "paper-position",
        proposalPaperSource: source,
        proposalPaperPositions: positions,
        proposalError: null,
      });
    },

    setProposalPaperPosition: (leafId, position) => {
      const s = get();
      if (s.proposalBusy) return;
      const source = s.proposalPaperSource;
      const candidate = source === null ? undefined : s.proposalCandidates[source];
      if (!candidate) return;
      const index = s.proposalPaperPositions.findIndex(
        (entry) => entry.leaf_id === leafId,
      );
      if (index < 0) return;
      const fit = clampPaperPosition(position, paperBounds(candidate.cp));
      const specified = s.proposalPaperSpecified.find(
        (entry) => entry.leaf_id === leafId,
      )?.position;
      if (
        specified?.x === fit.x &&
        specified.y === fit.y &&
        s.proposalPositionLastMoved.find((entry) => entry.leaf_id === leafId)
          ?.source === "paper"
      ) {
        return;
      }
      pushProposalPositionUndo(`paper:${leafId}`);
      const next = s.proposalPaperPositions.map((entry, entryIndex) =>
        entryIndex === index ? { ...entry, position: fit } : entry,
      );
      set({
        proposalPaperPositions: next,
        proposalPaperSpecified: setSpecifiedPaperPosition(
          s.proposalPaperSpecified,
          leafId,
          fit,
        ),
        proposalPositionLastMoved: setLastMovedSource(
          s.proposalPositionLastMoved,
          leafId,
          "paper",
        ),
        proposalError: null,
      });
    },

    resetProposalPaperPositions: () => {
      const s = get();
      if (s.proposalBusy) return;
      const source = s.proposalPaperSource;
      const candidate = source === null ? undefined : s.proposalCandidates[source];
      if (!candidate) return;
      const shownIds = new Set(
        s.proposalPaperPositions.map((entry) => entry.leaf_id),
      );
      if (
        !s.proposalPaperSpecified.some((entry) => shownIds.has(entry.leaf_id))
      ) {
        return;
      }
      pushProposalPositionUndo(null);
      const paperSpecified = s.proposalPaperSpecified.filter(
        (entry) => !shownIds.has(entry.leaf_id),
      );
      let lastMoved = s.proposalPositionLastMoved.filter(
        (entry) => !shownIds.has(entry.leaf_id),
      );
      for (const leafId of shownIds) {
        const completion = s.proposalSkeleton.nodes.find(
          (node) => node.id === leafId,
        )?.tip_pos_2d;
        if (completion != null) {
          lastMoved = setLastMovedSource(lastMoved, leafId, "completion");
        }
      }
      set({
        proposalPaperPositions: paperEditorPositions(
          candidate,
          s.proposalSkeleton,
          paperSpecified,
        ),
        proposalPaperSpecified: paperSpecified,
        proposalPositionLastMoved: lastMoved,
        proposalError: null,
      });
    },

    restoreOtherProposalPosition: (leafId) => {
      const s = get();
      if (s.proposalBusy) return;
      const paper = s.doc?.paper ?? FALLBACK_PAPER;
      const state = proposalLeafPositionStates(
        s.proposalSkeleton,
        s.proposalPaperSpecified,
        s.proposalPositionLastMoved,
        paper,
      ).find((entry) => entry.leaf_id === leafId);
      if (!state || state.kind !== "different" || state.used === null) return;
      pushProposalPositionUndo(null);
      let skeleton = s.proposalSkeleton;
      let paperSpecified = s.proposalPaperSpecified;
      let lastMoved = s.proposalPositionLastMoved;
      if (state.used === "paper") {
        paperSpecified = paperSpecified.filter((entry) => entry.leaf_id !== leafId);
        lastMoved = setLastMovedSource(lastMoved, leafId, "completion");
      } else {
        skeleton = setTipPos(skeleton, leafId, null);
        lastMoved = setLastMovedSource(lastMoved, leafId, "paper");
      }
      const source = s.proposalPaperSource;
      const candidate =
        source === null ? undefined : s.proposalCandidates[source];
      const stayOnPaper = s.proposalStep === "paper-position" && candidate !== undefined;
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
    },

    undoProposalPosition: () => {
      undoProposalPositionState();
    },

    redoProposalPosition: () => {
      redoProposalPositionState();
    },

    generateProposalFromPaperPositions: async () => {
      const s = get();
      if (s.proposalBusy || s.proposalPaperPositions.length === 0) return;
      const paper = s.doc?.paper ?? FALLBACK_PAPER;
      const seed = s.proposalSeed;
      const requestSkeleton = proposalRequestSkeleton(
        s.proposalSkeleton,
        s.proposalPaperSpecified,
        s.proposalPositionLastMoved,
        paper,
      );
      // 葉ごとに決めた側を送信用の複製へまとめ、要求は1件だけ送る。
      // 完成形と紙の上の元入力はどちらもストアに残す。
      await runProposalGeneration(requestSkeleton, paper, seed, (list) => ({
        proposalCandidates: list.length > 0 ? list : s.proposalCandidates,
        proposalSelected: list.length > 0 ? 0 : s.proposalSelected,
        proposalPaperSource: list.length > 0 ? null : s.proposalPaperSource,
        proposalPaperPositions:
          list.length > 0 ? [] : s.proposalPaperPositions,
        proposalStep: list.length > 0 ? "candidates" : "paper-position",
        proposalError:
          list.length > 0
            ? null
            : "この場所では候補を作れませんでした。丸い印を少し離してみてください",
      }));
    },

    applyProposalCandidate: async () => {
      const s = get();
      const chosen =
        s.proposalSelected === null
          ? undefined
          : s.proposalCandidates[s.proposalSelected];
      if (!chosen) return;
      // 以後は普通の展開図として自由に編集できる(PRO-003)。
      // どちらの経路でも確定は1回だけなので、元に戻す1回で入れる前へ戻る
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
        // 折り方が付いているときは、展開図と折り手順を1回でまとめて入れる。
        // 別々に入れると、展開図だけ入って手順が無い状態が「元に戻す」の
        // 対象になってしまう(作業28)。
        await applyDocChange(() => ipc.proposalApply(plan.cp, plan.steps), true);
        return;
      }
      // 折り方が付いていないときは、今までどおり展開図だけを流し込む
      await get().applyEdit({ type: "ReplaceCreasePattern", cp: chosen.cp });
    },

    openExport: () =>
      set({ exportOpen: true, exportError: null, exportSavedPath: null }),

    closeExport: () => set({ exportOpen: false, exportBusy: false }),

    // 指定を変えたら前回の「保存しました」は別の話になるので消す
    setExportOption: (patch) =>
      set({ ...patch, exportError: null, exportSavedPath: null }),

    runExport: async (path) => {
      await waitForFoldAllRestore();
      const s = get();
      if (s.exportBusy) return;
      if (s.exportKind === "CpPng" && !Number.isFinite(s.exportLongSide)) {
        set({ exportError: "画像の大きさを数で入れてください" });
        return;
      }
      set({ exportBusy: true, exportError: null, exportSavedPath: null });
      // 書き出しは作品を書き換えないが、直前の編集が反映された内容を出したいので
      // 直列化キューに載せる(編集 → 書き出しの順が守られる)
      const r = await queue.run(() =>
        ipc.documentExport(s.exportKind, path, {
          include_aux: s.exportIncludeAux,
          png_long_side: Math.round(s.exportLongSide),
        }),
      );
      if (r.ok) {
        set({ exportBusy: false, exportSavedPath: path });
      } else {
        const e = r.error;
        set({
          exportBusy: false,
          exportError: typeof e === "string" ? e : String(e),
        });
      }
    },

    openNewDialog: () => set({ newDialogOpen: true, errorMessage: null }),

    closeNewDialog: () => set({ newDialogOpen: false }),

    setNewPaperDraft: (patch) =>
      set((s) => ({ newPaperDraft: { ...s.newPaperDraft, ...patch } })),

    confirmNewDocument: async () => {
      const paper = draftToPaper(get().newPaperDraft);
      if (!(paper.width_mm > 0) || !(paper.height_mm > 0)) {
        set({ errorMessage: "紙の大きさは0より大きいmmで入れてください" });
        return;
      }
      set({ newDialogOpen: false });
      await get().newDocument(paper);
    },

    setDisplay: async (patch) => {
      const display = { ...get().display, ...patch };
      if (patch.grid_divisions !== undefined) {
        display.grid_divisions = clampDivisions(patch.grid_divisions);
      }
      const doc = get().doc;
      // 色見本や数の入力はその場で見えたほうがよいので、先に画面へ映してから
      // 作品へ書き込む(設計原則3b)。書き込んだ結果が返ればそれで上書きされる
      set(doc ? { display, doc: { ...doc, display } } : { display });
      // 作品ごとの設定として保存する(.ori3に入り、元に戻す/やり直しも効く)。
      // 作品をまだ開いていないときは画面の表示だけ変える
      if (doc) await get().applyEdit({ type: "SetDisplay", display });
    },

    setSoft: (patch) => {
      const beforeSoft = softOf(get().display);
      const display = { ...get().display, ...patch };
      if (patch.soft_stiffness !== undefined) {
        display.soft_stiffness = clampUnit(patch.soft_stiffness, 0.5);
      }
      if (patch.soft_pressure !== undefined) {
        display.soft_pressure = clampUnit(patch.soft_pressure, 0);
      }
      const doc = get().doc;
      // つまみの位置はその場で映す(設計原則3b: 結果を見ながら調整できること)
      set(doc ? { display, doc: { ...doc, display } } : { display });
      const afterSoft = softOf(display);
      if (
        afterSoft.enabled &&
        afterSoft.pressure > 0 &&
        (!beforeSoft.enabled || Math.abs(afterSoft.pressure - beforeSoft.pressure) >= 0.01)
      ) {
        get().completeGuideAction("inflate");
      }
      // 切ったらすぐ従来の描き方へ戻す(次の計算を待たない)
      if (display.soft_enabled !== true) set({ softMesh: null, softWarnings: [] });
      softShape.schedule();
      if (doc) {
        softPending = true;
        pendingSoftDisplay = display;
        softSave.schedule();
      }
    },

    setSplitRatio: (ratio) => {
      set({ splitRatio: clampSplitRatio(ratio) });
      persistPrefs();
    },

    setContextPanelRatio: (ratio) => {
      set({ contextPanelRatio: clampContextPanelRatio(ratio) });
      persistPrefs();
    },

    resetPaneSizes: () => {
      set({
        splitRatio: DEFAULT_SPLIT_RATIO,
        contextPanelRatio: DEFAULT_CONTEXT_PANEL_RATIO,
      });
      persistPrefs();
    },

    moveStep: async (number, delta) => {
      if (foldAllEntering || get().foldAllPreview !== null) return;
      const visibleSteps = get().doc?.sequence ?? [];
      const visibleFrom = number - 1;
      const id = visibleSteps[visibleFrom]?.id;
      if (id === undefined) return;

      // 覚え書き・折り方の確定直後は、UpdateStepがFIFOに積まれていてもdocは
      // まだ旧値のことがある。確定結果の反映を待ち、IDから最新の手順を取り直す。
      // 待っている間に別の作品変更が始まった場合も、最後に始まった変更まで
      // 追いかける。安定した最新docを読んでからはawaitを挟まず移動を積む。
      while (true) {
        const changesBeforeMove = commandService.latestDocChange();
        await changesBeforeMove;
        if (changesBeforeMove === commandService.latestDocChange()) break;
      }
      // 待機中に一斉表示の入口が先に開くことがある。入口の前だけでなく、
      // 作品変更が安定した後にも確認し、一時姿勢のまま手順を変更しない。
      if (foldAllEntering || get().foldAllPreview !== null) return;
      const steps = get().doc?.sequence ?? [];
      const from = steps.findIndex((candidate) => candidate.id === id);
      const to = from + delta;
      if (from < 0 || to < 0 || to >= steps.length) return;

      // 既存手順の位置替えは1回のジェスチャーなので、Rust側の原子操作へ
      // 1回だけ送る。応答前に画面側のsequenceは変更しない。
      const succeeded = await applySequenceChange(
        { type: "MoveStep", id, to_index: to },
        to === from,
      );
      if (succeeded && to !== from) get().selectStep(to + 1);
    },
  };
});
