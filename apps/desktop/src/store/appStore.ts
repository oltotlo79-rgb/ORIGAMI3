// アプリ全体の状態を1本で管理するZustandストア(要件§2: フロント状態はストア1本)。
// IPC呼び出しはactionの中で行い、成功したらDocumentViewの内容を一括反映、
// 失敗(reject)はerrorMessageへ入れる(「止めずに警告」原則)。
// 全IPC要求は直列化キュー(ipcQueue.ts)を通し、連続操作でも適用順を発行順に
// 固定する。最新でない応答は破棄する(古いdocによる上書きを防ぐ)。

import { create } from "zustand";
import * as ipc from "../ipc/client";
import { createSerialQueue } from "./ipcQueue";
import { hingeEdgeIds } from "../lib/hinges";
import { advancePlayback, startPlayback } from "../lib/playback";
import {
  foldLayers,
  keepSidePoint,
  offsetPoint,
  topMovingFace,
} from "../components/Viewer3D/foldDraw";
import { planGrabFold, type GrabMode } from "../components/Viewer3D/grabFold";
import { foldBlockReason } from "../lib/viewerHint";
import { buildPoseStep, currentAngles, hasPoseAngle } from "../lib/poseStep";
import { DEFAULT_CONSTRUCT, type ConstructOptions } from "../lib/construct";
import { DEFAULT_CURVE, firstCrossing, rulingLines, type CurveOptions } from "../lib/curve";
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
import {
  ALIGN_STEPS,
  alignRefPoint,
  movingSideOf,
  solveAlign,
  type AlignMode,
  type AlignTarget,
  type FoldLine,
} from "../lib/alignFold";
import {
  DEFAULT_MIRROR_AXIS,
  mirrorLineForChoice,
  mirrorSegmentSet,
  mirrorSegments,
  paperMirrorLine,
  rebindSelectedMirrorAxis,
  selectedEdgeMirrorLine,
  type MirrorAxisChoice,
  type MirrorAxisPreset,
  type MirrorLine,
  type Segment,
} from "../lib/mirror";
import { withMirrorEdges } from "../lib/mirrorEdit";
import {
  DEFAULT_TWIST_DEG,
  addTwistVertex,
  isTwistPolygonReady,
  polygonCentroid,
  twistReferencePoint,
  undoTwistVertex,
} from "../lib/twistPolygon";
import type {
  AngleRelaxation,
  Document,
  DisplaySettings,
  DocumentView,
  Driver,
  EdgeKind,
  EditOp,
  ExportKind,
  Face,
  FoldDirection,
  FoldThroughProposal,
  Frame3D,
  MotionPart,
  Paper,
  ProposalCandidate,
  RecoveryInfo,
  SeqOp,
  Skeleton,
  SoftMesh,
  SoftSettings,
  StepCreases,
  TechniqueKind,
  Vec2,
} from "../lib/types";
import { defaultSkeleton } from "../lib/skeleton";
import { loadOnboarding, saveOnboarding } from "../lib/firstRunGuide";
import type { HelpChapterId } from "../help/helpTypes";
import {
  clampTechniqueLayerCount,
  minimumTechniqueFlap,
  techniqueFlapForPreset,
  techniqueUsesOpenToBack,
  toggleTechniqueFlap as toggleTechniqueFlapSelection,
  type TechniqueLayerPreset,
} from "../lib/techniqueLayers";
import {
  buildLayerMotionPart,
  hasLayerMotionInput,
  type LayerMotionMode,
  type LayerMotionPartDraft,
  type LayerTurnMode,
} from "../lib/layerMotion";

/** ヒンジ角の連続操作(スライダー)を間引く間隔(ms) */
/** 追従計算は60fps相当で最大1回。runLatestが計算待ちを最新1件へまとめる。 */
const POSE_THROTTLE_MS = 16;

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

/** 希望角との差がこの値以上なら、画面で「追従した」と知らせる。 */
export const RELAX_NOTICE_EPS_DEG = 0.1;

/** 選んだ基準線が編集後に無くなったとき、既存の通知欄へ出す文言。 */
export const MIRROR_AXIS_REMOVED_NOTICE =
  "基準にしていた線が無くなったので、紙の縦の中心線に戻しました";

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

export type ToolId =
  | "select"
  | "mountain"
  | "valley"
  | "aux"
  | "delete"
  | "fold"
  | "pull"
  | "technique"
  | "construct";

/** UI-012の4操作と、全てできた後の完了画面。 */
export type GuideStep = 0 | 1 | 2 | 3 | 4;
export type GuideAction = "fold" | "angle" | "pull" | "inflate";
const GUIDE_ACTIONS: GuideAction[] = ["fold", "angle", "pull", "inflate"];

/** 提案ウィザードの3画面(骨格を作る → 候補を選ぶ → 確認する) */
export type ProposalStep = "skeleton" | "candidates" | "confirm";

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

/** 選択中の線・頂点(ID)。DOMのSelectionと紛れないよう注意 */
export interface Selection {
  edgeIds: number[];
  vertexIds: number[];
}

/** 折る対象の層: 全ての層 / いちばん上の1枚 */
export type FoldTarget = "all" | "top";

/** 引いた折り線と、確定前の設定(コンテキストパネルで変える) */
export interface FoldDraft {
  /** 折り線(畳み平面座標=3D表示のxy)。始点→終点の向きが左右の基準になる */
  line: [Vec2, Vec2];
  /** 折る向き(Up=手前へ折る/谷、Down=向こうへ折る/山) */
  direction: FoldDirection;
  target: FoldTarget;
  /** 折り線のどちら側を動かすか(線の進行方向に対する左右) */
  movingSide: "left" | "right";
  /** 線を引いた時点の作品の世代番号(新規・開くで変わる) */
  docEpoch: number;
  /** 線を引いた時点の手順の数。線は「その形の上」で引いたものなので、
   * 手順が増減していたら別の形に対する線になってしまう */
  stepCount: number;
  /** 線を引いた時点で見ていた位置(=折りが挟まる位置)。見る手順を移すと
   * 別の形の上の線になってしまうので、ここが変わった線は捨てる */
  upTo: number;
}

/** 展開図へ線を1本足す要求。 */
type AddSegmentOp = Extract<EditOp, { type: "AddSegment" }>;

/**
 * まだ送っていない線も入れて測るための作業用の写し。
 * 「紙が曲がるための線」をどこで止めるかは、その曲線で先に引いた線にも
 * ぶつかったところで決まるので、送る前の形をここで組み立てる。
 */
function editableCopy(doc: Document): Document {
  return {
    ...doc,
    cp: { ...doc.cp, vertices: [...doc.cp.vertices], edges: [...doc.cp.edges] },
  };
}

/** 作業用の写しへ線を1本足す(交点での分割はしない。止まる位置の判定にしか使わない) */
function addWorkingEdge(work: Document, a: Vec2, b: Vec2, kind: EdgeKind): void {
  const v0 = work.cp.next_vertex_id;
  work.cp.vertices.push({ id: v0, pos: a }, { id: v0 + 1, pos: b });
  work.cp.next_vertex_id = v0 + 2;
  const edgeId = work.cp.next_edge_id;
  work.cp.edges.push({ id: edgeId, v0, v1: v0 + 1, kind });
  work.cp.next_edge_id = edgeId + 1;
}

/** 実際に作品へ適用するFoldThrough。事前確認では最後の指定だけを除いて使う。 */
type FoldThroughApplyOp = Extract<SeqOp, { type: "FoldThrough" }>;
type FoldThroughInput = Omit<FoldThroughApplyOp, "accept_additional_crease">;

/** 立体姿勢の紙をつかんだドラッグ。座標は3D表示と同じ世界座標。 */
export type SpatialFoldDrag = {
  from: [number, number, number];
  to: [number, number, number];
  grab_face: number;
  mode: GrabMode;
};

/** 平坦用FoldThroughに、立体姿勢の入力を必要なときだけ添える。 */
type FoldThroughOperation = FoldThroughInput & { spatial?: SpatialFoldDrag };

/**
 * Frame3Dが従来の畳み平面から外れているか。
 * flat用のfoldLayersと同じ許容差を使い、平坦経路の選択を変えない。
 */
export function isSpatialFoldFrame(frame: Frame3D | null): boolean {
  return (
    frame !== null &&
    frame.faces.some((face) => face.polygon.some(([, , z]) => Math.abs(z) > 1e-6))
  );
}

/**
 * 縁への衝突が1か所だけ見つかり、巻き込み用の追加折り目を選べる状態。
 * 元の折り入力も保持し、利用者の答えを同じ条件へ適用する。
 */
export interface PendingFoldThrough {
  proposal: FoldThroughProposal;
  operation: FoldThroughOperation;
  docEpoch: number;
  stepCount: number;
}

/**
 * 「合わせて折る」の途中経過(折り紙の基準合わせ)。
 * 展開図または3D表示で点・線を順に選び、そろった時点で折り線を計算してFoldDraftを作る。
 * 折り方の決定(山谷・対象の層・折る/やめる)は既存の折り確定UIをそのまま使う。
 */
export interface AlignDraft {
  /** 藤田・羽鳥の7基本作図、または既存線をそのまま使う合わせ方 */
  mode: AlignMode;
  /** 選んだ対象(ALIGN_STEPSの順。まだ足りない間は途中まで) */
  picks: AlignTarget[];
  /** 展開図から選んだ対象のID。3Dで選んだ段階はnull。picksと同じ順に並ぶ。 */
  cpPicks?: (AlignCpPick | null)[];
  /** 求まった折り線(0〜3本。カーソルに近い順) */
  solutions: FoldLine[];
  /** 今使っている解の番号(「別の解」で切り替える) */
  solutionIndex: number;
  /** 解が求まらなかった理由(求まったならnull) */
  reason: string | null;
}

/** 展開図側で既存の選択強調へ対応付けるための、選んだ対象のID。 */
export type AlignCpPick =
  | { kind: "vertex"; id: number }
  | { kind: "edge"; id: number };

/** 技法の下ごしらえ(選んだ技法・フラップ・折り線)。確定するまで保持する */
export interface TechniqueDraft {
  /** 選んだ技法(実装済みのものだけ。lib/techniques.tsのSUPPORTED_TECHNIQUESを参照) */
  kind: TechniqueKind;
  /** 対象フラップ(3D表示でクリックした場所に重なっている層の面ID) */
  flap: number[];
  /** クリック地点に重なる候補層。facesAtPointと同じ奥→手前の順 */
  flapCandidates: number[];
  /** 「手前/奥からN枚」「手前からN枚目」で使う枚数・奥行き */
  flapPickCount: number;
  /** 折り線(畳み平面座標)。まだ引いていなければnull */
  line: [Vec2, Vec2] | null;
  /** 折り線のどちら側が動くか(線の進行方向に対する左右) */
  movingSide: "left" | "right";
  /** 段折りの段の幅(mm) */
  widthMm: number;
  /** ねじり折りの中央多角形(畳み平面座標)。3D画面で順にクリックした頂点。
   * 3点以上そろうと、この形のまま折る(辺の数も長さも仮定しない) */
  polygon: Vec2[];
  /** ねじり折りの中心(畳み平面座標)。nullなら多角形の重心を使う */
  center: Vec2 | null;
  /**
   * 名前付き技法へ渡す基準点(畳み平面座標)。nullなら折り線と動かす側から
   * 自動で決める。技法ごとの意味は、つぶす先・持ち上げる先端・沈める先端・
   * 寄せる先など。ねじり折りの中心は上のcenterを使う。
   */
  referencePoint: Vec2 | null;
  /** ねじり折りのねじる角(度)。動かす側の指定で向きが決まる */
  twistDeg: number;
  /** 対応技法で、動かした紙を向こう側(重なりの下)へ入れるか */
  openToBack: boolean;
  /** kind=Simpleを「層操作」入口として使うときの現在partの変換。 */
  motionMode: LayerMotionMode;
  /** Stayで紙を動かさず重ね替えるときの置き場所。 */
  motionTurn: LayerTurnMode;
  /** Outside/Inside/Besideの手前(Up)・奥(Down)。 */
  motionDirection: FoldDirection;
  /** Besideで隣へ置く基準面ID。 */
  motionAnchor: number;
  /** 現在part内の重なり順を反転するか。 */
  motionReverseLayers: boolean;
  /** クリックで正確に選んだ既存折り目。手描き軸ならnull。 */
  motionAxisEdgeId: number | null;
  /** 1手として同時に適用する、追加済みのpart。 */
  motionParts: MotionPart[];
  /** 選んだ時点の作品の世代番号(新規・開くで変わる) */
  docEpoch: number;
  /** 選んだ時点の手順の数 */
  stepCount: number;
  /** 選んだ時点で見ていた位置(=技法が挟まる位置)。FoldDraftと同じ意味 */
  upTo: number;
}

/** 技法下書きの共通項目から、汎用層操作の現在partだけを取り出す。 */
function layerMotionPartDraft(draft: TechniqueDraft): LayerMotionPartDraft {
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

/** part追加後、次の部分を選べる空の状態へ戻す。追加済みpartは呼び出し側で保つ。 */
function clearCurrentLayerMotion(draft: TechniqueDraft): TechniqueDraft {
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

/**
 * 合わせて求まった折り線から、確定前の折り(FoldDraft)を作る。
 * 動く側は「1つ目に選んだ対象がある側」にする(その対象が相手に重なる)。
 * 向き・対象の層は既存の折り操作と同じ既定にして、あとはパネルで決めてもらう。
 */
export function alignFoldDraft(
  s: { docEpoch: number; doc: Document | null; currentStep: number | null },
  line: FoldLine,
  picks: AlignTarget[],
): FoldDraft | null {
  if (!s.doc || picks.length === 0) return null;
  return {
    line,
    direction: "Up",
    target: "all",
    movingSide: movingSideOf(line, alignRefPoint(picks[0])),
    docEpoch: s.docEpoch,
    stepCount: s.doc.sequence.length,
    upTo: foldInsertAt(s),
  };
}

/** 選び終えたかどうか(合わせ方ごとに必要な数だけ選べたか) */
export function isAlignComplete(draft: AlignDraft): boolean {
  return draft.picks.length >= ALIGN_STEPS[draft.mode].length;
}

/** 次に選ぶべき対象の種類(選び終えていればnull) */
export function nextAlignKind(draft: AlignDraft): "point" | "line" | null {
  const steps = ALIGN_STEPS[draft.mode];
  return draft.picks.length < steps.length ? steps[draft.picks.length] : null;
}

/** 段折りの段の幅の初期値(mm) */
const DEFAULT_PLEAT_WIDTH_MM = 10;

/** 引きかけの折り線が今の形に合わなくなったときの案内 */
const STALE_DRAFT_MESSAGE =
  "引いた折り線は今の紙の形に合わなくなったため取り消しました。もう一度線を引いてください";

/** 技法が使えない形だったときに添える案内(要件§12) */
const TECHNIQUE_FALLBACK_HINT = "手動の折り操作で代替してください";

/**
 * 折る操作ができる状態か(平らに畳んだ状態を表示しているか)。
 * 再生中・折り途中(playT≠1)・角度スライダーでの変形中は、画面の形と
 * 畳み平面の座標が食い違うので折れない。
 *
 * 途中の手順を選んでいる間も折れる(SEQ-006)。そこで折ると、その手順の前へ
 * 折りが挟まり、後ろの手順はそのまま残って折り直される。
 */
export function canFoldNow(s: {
  doc: Document | null;
  currentStep: number | null;
  playT: number;
  playing: boolean;
  drivers: Map<number, number>;
}): boolean {
  return !(!s.doc || s.playing || s.playT !== 1 || s.drivers.size > 0);
}

/** 折り操作を挟む位置(=この折りの直前までの手順数)。
 * 「最新」を見ているなら末尾へ、途中の手順を見ているならその位置へ挟む */
export function foldInsertAt(s: {
  doc: Document | null;
  currentStep: number | null;
}): number {
  const total = s.doc?.sequence.length ?? 0;
  return s.currentStep === null ? total : Math.min(s.currentStep, total);
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

interface AppState {
  doc: Document | null;
  /** 手順ごとに展開図へ新しく足した折り線(作品ファイル由来の来歴)。
   * 過去の手順の展開図を推測せずに組み立てるために使う */
  stepCreases: StepCreases[];
  faces: Face[];
  warnings: string[];
  /** 平らにする操作の最新結果で、利用者へ知らせる点(raw violationsとは別) */
  flatFoldViolations: number[];
  violations: number[];
  selection: Selection;
  /** 複数スライダーのうち、いま指している折り目。2D/3Dの個別強調に使う。 */
  hoveredHinge: number | null;
  activeTool: ToolId;
  /** 引いたばかりの折り線と確定前の設定。nullなら折り線を引いていない */
  foldDraft: FoldDraft | null;
  /** 巻き込み用の追加折り目を提案中。2D/3Dのプレビューもこの値を使う */
  pendingFoldThrough: PendingFoldThrough | null;
  /** 折りの事前確認または、提案への答えを適用している途中 */
  foldThroughBusy: boolean;
  /** 「合わせて折る」の途中経過。nullなら合わせモードに入っていない */
  alignDraft: AlignDraft | null;
  /** 選んだ技法と、その下ごしらえ(フラップ・折り線)。nullなら技法を選んでいない */
  techniqueDraft: TechniqueDraft | null;
  /** 作図補助の選択(どの作図か・等分数・角度の刻み)。CpEditorが使う */
  construct: ConstructOptions;
  /** 曲線の折り目(CPE-011)の選択(直線/曲線・描き方・分割・曲がるための線) */
  curve: CurveOptions;
  /** 3D表示フレーム。nullなら平ら(展開図から直接描く) */
  frame3d: Frame3D | null;
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
  errorMessage: string | null;
  /** 直近に保存できた作品ファイルの場所。編集・次の保存開始・失敗で消す。 */
  documentSavedPath: string | null;
  /** 作品の世代番号。新規/開くの成功で増える(エディタの表示リセット合図) */
  docEpoch: number;
  /** 利用者が指定した折り角度(度)。キーは辺ID */
  drivers: Map<number, number>;
  /** 折り角度を変える前のdriversの控え(古い順)。「元に戻す」はまずここを使う。
   * 3Dの形は作品データではないので保存せず、作品を開く・新規作成で捨てる */
  angleUndoStack: ReadonlyMap<number, number>[];
  /** 「元に戻す」で戻した角度をやり直すための控え(古い順) */
  angleRedoStack: ReadonlyMap<number, number>[];
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
  /** 生成中か(「計算中…」の表示用) */
  proposalBusy: boolean;
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
  /** 3Dで紙そのものを選んだときの「引く・膨らます」案内を出しているか。 */
  paperActionTipVisible: boolean;
  /** 紙の操作案内を詳しく開いているか。閉じた後は小さな入口だけ残す。 */
  paperActionTipExpanded: boolean;

  newDocument: (paper: Paper) => Promise<void>;
  openDocument: (path: string) => Promise<void>;
  saveDocument: (path: string | null) => Promise<void>;
  /** 展開図を変える要求を送る。複数渡すと、画面での1回の入力として
   * 元に戻す1回でまとめて戻せるようになる(不具合D05) */
  applyEdit: (op: EditOp | EditOp[]) => Promise<void>;
  /** 展開図に線を1本引く(CPE-010)。左右対称のときは中心線で折り返した線も
   * 一緒に引き、2本で元に戻す1回分になる */
  drawSegment: (a: Vec2, b: Vec2, kind: EdgeKind) => Promise<void>;
  /** 曲線の折り目を折れ線として引く(CPE-011)。設定に応じて「紙が曲がるための
   * 線」も一緒に引く。左右対称の指定も効き、曲線1本が元に戻す1回分になる */
  drawCurve: (points: Vec2[], kind: EdgeKind) => Promise<void>;
  /** 左右対称に線を引くかを切り替える(次回起動時も同じ設定に戻る) */
  setMirrorDraw: (on: boolean) => void;
  /** 紙の縦・横の中心線を基準にする。端末設定へ保存する。 */
  setMirrorAxisPreset: (preset: MirrorAxisPreset) => void;
  /** 展開図で選んでいる折り線・補助線1本を基準にする。作品をまたいで保存しない。 */
  setSelectedLineAsMirrorAxis: () => void;
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
  setTool: (tool: ToolId) => void;
  setSelection: (selection: Selection) => void;
  /** 折り目ごとの角度行を指したときの一時的な強調(nullで解除) */
  setHoveredHinge: (hinge: number | null) => void;
  /** 引いた折り線を確定前の状態として置く(source="2d"は手順がある間は断る) */
  beginFoldDraft: (line: [Vec2, Vec2], source: "2d" | "3d") => void;
  /** 確定前の折りの設定(向き・対象層・動かす側)を変える */
  updateFoldDraft: (patch: Partial<FoldDraft>) => void;
  /** 引いた折り線を捨てる */
  cancelFoldDraft: () => void;
  /** 引いた折り線で実際に折る(sequence_apply FoldThrough)。成功したら折り線を捨てる */
  commitFoldDraft: () => Promise<void>;
  /** 巻き込み用の追加折り目を入れるか答え、元の折りを確定する */
  resolveFoldThroughProposal: (accept: boolean) => Promise<void>;
  /** 「合わせて折る」を始める(合わせ方を選ぶ)。同じ合わせ方をもう一度押すとやめる */
  beginAlign: (mode: AlignMode) => void;
  /** 合わせる対象(点・線)を1つ選ぶ。cursorは解が2つあるときの既定を決めるのに使う */
  pickAlignTarget: (
    target: AlignTarget,
    cursor?: Vec2 | null,
    cpPick?: AlignCpPick | null,
  ) => void;
  /** 解が2つあるときに別の解へ切り替える */
  nextAlignSolution: () => void;
  /** 直前の選択を取り消す(選択が無ければ何もしない) */
  undoAlignPick: () => void;
  /** 合わせモードをやめる(選択も求まった折り線も捨てる) */
  cancelAlign: () => void;
  /**
   * 紙をつかんで動かす折り操作(UI-007)。つかんだ点fromから離した点toへの
   * ドラッグを折り線・対象の層に翻訳して、そのまま折る(パネル操作は要らない)。
   * grabFaceは立体表示で実際につかんだ面(raycastで拾えなければnull)。
   * directionは立体で180°になったとき、ドラッグ途中に通った表裏の側を保つ。
   */
  foldByDrag: (
    from: Vec2 | [number, number, number],
    to: Vec2 | [number, number, number],
    mode: GrabMode,
    grabFace?: number | null,
    direction?: FoldDirection,
  ) => Promise<void>;
  /** 技法を選ぶ(ツールレールのサブメニュー)。下ごしらえを作り直す */
  beginTechnique: (kind: TechniqueKind) => void;
  /** 3D表示でクリックした場所の層をフラップとして選ぶ */
  setTechniqueFlap: (faces: number[]) => void;
  /** 候補層を「全部/手前・奥からN枚/手前からN枚目」で選ぶ */
  setTechniqueFlapPreset: (preset: TechniqueLayerPreset) => void;
  /** 候補層1枚のチェックを切り替える */
  toggleTechniqueFlap: (face: number) => void;
  /** 技法の折り線を引く */
  setTechniqueLine: (line: [Vec2, Vec2]) => void;
  /** 層操作で、3D上の既存折り目を正確な開閉軸として選ぶ。 */
  setLayerMotionAxis: (edgeId: number, line: [Vec2, Vec2]) => void;
  /** 現在の層操作partを同時操作の一覧へ追加する。 */
  addLayerMotionPart: () => void;
  /** 同時操作の一覧から直前のpartを外す。 */
  undoLayerMotionPart: () => void;
  /** ねじり折りの中央多角形へ頂点を1つ足す(3D画面のクリック) */
  addTechniqueVertex: (p: Vec2) => void;
  /** 直前に足した頂点を取り消す(頂点が無ければ何もしない) */
  undoTechniqueVertex: () => void;
  /** ねじり折りの中心を指定する(nullで多角形の重心へ戻す) */
  setTechniqueCenter: (p: Vec2 | null) => void;
  /** 名前付き技法の基準点を明示する(nullで自動へ戻す) */
  setTechniqueReferencePoint: (p: Vec2 | null) => void;
  /** 技法の設定(動かす側・段の幅)を変える */
  updateTechniqueDraft: (patch: Partial<TechniqueDraft>) => void;
  /** 作図補助(CPE-005)の選び方を変える(どの作図か・等分数・角度の刻み) */
  setConstruct: (patch: Partial<ConstructOptions>) => void;
  /** 曲線の折り目(CPE-011)の選び方を変える */
  setCurve: (patch: Partial<CurveOptions>) => void;
  /** 技法の下ごしらえを捨てる */
  cancelTechnique: () => void;
  /** 選んだ技法を実際に適用する(sequence_apply Technique) */
  commitTechnique: () => Promise<void>;
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
  /** 今の骨格で候補を作り、候補選びの画面へ進む(PRO-005) */
  generateProposal: () => Promise<void>;
  /** 候補を選ぶ */
  selectProposalCandidate: (index: number) => void;
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
  /** 手順の順番を入れ替える(numberは1始まり、deltaは-1で前へ/+1で後ろへ) */
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

const EMPTY_SELECTION: Selection = { edgeIds: [], vertexIds: [] };

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

/** 中身が同じなら前の配列を使い回す。再生中は毎コマ結果が届くので、
 * 内容が変わらない限り同じ配列を返して画面の再描画を起こさない */
function keepIfSame<T>(prev: T[], next: T[]): T[] {
  const same = prev.length === next.length && prev.every((v, i) => v === next[i]);
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

/** ドキュメント更新後、存在しなくなったIDを選択から取り除く */
function pruneSelection(selection: Selection, doc: Document): Selection {
  const edgeIds = new Set(doc.cp.edges.map((e) => e.id));
  const vertexIds = new Set(doc.cp.vertices.map((v) => v.id));
  return {
    edgeIds: selection.edgeIds.filter((id) => edgeIds.has(id)),
    vertexIds: selection.vertexIds.filter((id) => vertexIds.has(id)),
  };
}

export const useAppStore = create<AppState>((set, get) => {
  const queue = createSerialQueue();
  /**
   * 直前に開始した作品変更が、返されたDocumentViewをストアへ反映し終えるまでの約束。
   * IPCのFIFOだけでは、後続操作が呼ばれた瞬間に読むdocまでは新しくならないため、
   * 現在値を材料に次の操作を組み立てる箇所はこれを待つ。
   */
  let latestDocChange: Promise<void> = Promise.resolve();
  const prefs = loadPrefs();
  let onboarding = loadOnboarding();

  /**
   * FoldThroughの事前確認を無効にする世代番号。
   *
   * 提案は作品を止めない非モーダルUIなので、確認IPCの待機中にも利用者は
   * 手順表示やツールを変えられる。その場合、古い形から返った候補を採用しない。
   * 表示する状態ではないためZustandへ公開せず、このストア1本の内部で管理する。
   */
  let foldThroughRevision = 0;
  /** busyを開始した要求の番号。古い要求のfinallyが新しいbusyを消さないために使う */
  let foldThroughBusyToken = 0;
  /** 閉じた提案画面へ古い計算結果を戻さないための要求世代。 */
  let proposalGeneration = 0;

  const invalidateFoldThrough = (): void => {
    foldThroughRevision++;
    if (get().pendingFoldThrough !== null) set({ pendingFoldThrough: null });
  };

  const finishFoldThroughBusy = (token: number): void => {
    if (foldThroughBusyToken === token && get().foldThroughBusy) {
      set({ foldThroughBusy: false });
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

  /** DocumentViewの内容で状態を一括更新する(成功時共通処理)。
   * isNewDocument=true(新規/開く)なら選択を解除しdocEpochを進める。
   * 手順が減ったときは表示中の手順番号を手順数まで詰める。
   *
   * 引きかけの折り線はここで必ず捨てる。線は「引いた時点の形」の上の座標なので、
   * 展開図の編集・元に戻す・やり直し・手順の増減があった後に使うと、別の形の上で
   * 引いた線が黙って今の形へ適用されてしまう(折る操作自体の成功時も同じ経路を
   * 通るので、折り終わった線がそのまま残ることもない) */
  const applyView = (view: DocumentView, isNewDocument: boolean) => {
    const total = view.doc.sequence.length;
    set((s) => {
      const reboundSelectedAxis =
        !isNewDocument &&
        s.doc !== null &&
        s.mirrorAxis.kind === "selectedLine"
          ? rebindSelectedMirrorAxis(s.doc, view.doc, s.mirrorAxis)
          : null;
      const selectedAxisMissing =
        s.mirrorAxis.kind === "selectedLine" &&
        !isNewDocument &&
        reboundSelectedAxis === null;
      const resetSelectedAxis =
        s.mirrorAxis.kind === "selectedLine" &&
        (isNewDocument || selectedAxisMissing);
      return {
        // 紙の色・方眼は作品ごとの設定(doc.display)。ここを唯一の拠り所にして
        // 画面側の写し(display)をそろえる。人からもらった作品を開けば、
        // その作品の色と方眼がそのまま出る
        doc: view.doc,
        stepCreases: view.step_creases ?? [],
        display: view.doc.display,
        foldDraft: null,
        pendingFoldThrough: null,
        alignDraft: null,
        techniqueDraft: null,
        paperActionTipVisible: false,
        paperActionTipExpanded: false,
        faces: view.faces,
        hinges: hingeEdgeIds(view.doc, view.faces),
        warnings: view.warnings,
        flatFoldViolations: keepIfSame(
          s.flatFoldViolations,
          view.flat_fold_violations ?? [],
        ),
        violations: view.violations,
        skipped: view.skipped,
        suspectHinges: keepIfSame(s.suspectHinges, view.suspect_hinges ?? []),
        sequenceTargets: new Map(
          [...(view.sequence_targets ?? [])]
            .sort((a, b) => a.hinge - b.hinge)
            .map((driver) => [driver.hinge, driver.target_angle_deg]),
        ),
        poseAngles: new Map(
          Object.entries(view.angles ?? {}).map(([id, deg]) => [Number(id), deg]),
        ),
        relaxations: view.relaxations ?? [],
        poseConverged: view.converged ?? true,
        poseBestEffort: view.best_effort === true,
        poseClosureRms:
          typeof view.closure_rms === "number" ? view.closure_rms : null,
        currentStep:
          total === 0 || s.currentStep === null
            ? null
            : Math.min(s.currentStep, total),
        selection: isNewDocument
          ? EMPTY_SELECTION
          : pruneSelection(s.selection, view.doc),
        mirrorAxis: resetSelectedAxis
          ? DEFAULT_MIRROR_AXIS
          : reboundSelectedAxis ?? s.mirrorAxis,
        mirrorAxisNotice:
          !isNewDocument && selectedAxisMissing
            ? MIRROR_AXIS_REMOVED_NOTICE
            : isNewDocument
              ? null
              : s.mirrorAxisNotice,
        hoveredHinge: null,
        errorMessage: null,
        documentSavedPath: null,
        contactDetected: false,
        docEpoch: isNewDocument ? s.docEpoch + 1 : s.docEpoch,
      };
    });
  };

  /** IPC失敗(reject)をerrorMessageへ反映する */
  const fail = (e: unknown) => {
    set({
      errorMessage: typeof e === "string" ? e : String(e),
      documentSavedPath: null,
    });
  };

  /** DocumentViewを返すコマンドを直列化キュー経由で実行し、結果を反映する。
   * 直列化により適用順の逆転は起きないため、成功したviewは完了順に全て適用する
   * (途中の成功を捨てると、後続が失敗したときにバックエンドと画面が食い違う)。
   * 失敗の報告だけは最新要求に限る(古い失敗の直後には必ず新しい結果が続く) */
  const runViewCommand = async (
    task: () => Promise<DocumentView>,
    isNewDocument: boolean,
  ): Promise<void> => {
    const r = await queue.run(task);
    if (r.ok) {
      applyView(r.value, isNewDocument);
      if (isNewDocument) {
        // 別の作品になったので角度指定・立体形状・再生位置は捨てる
        stopPlayback();
        pose.reset();
        // 角度の履歴は作品データではないので、別の作品になったら捨てる
        clearAngleHistory();
        set({
          drivers: new Map(),
          poseWarnings: [],
          contactDetected: false,
          activeAngleIntent: null,
          angleIntentGeneration: get().angleIntentGeneration + 1,
          pullHinge: null,
          pullMirrorHinge: null,
          frame3d: r.value.frame,
          softMesh: null,
          softWarnings: [],
          currentStep: null,
          playT: 1,
          replaySkipped: [],
          replayWarnings: [],
          foldDraft: null,
          pendingFoldThrough: null,
          foldThroughBusy: false,
          alignDraft: null,
          techniqueDraft: null,
          operationStage: 0,
        });
      }
      if (r.value.doc.sequence.length > 0) {
        await syncSequence(r.value);
      } else if (!isNewDocument) {
        await syncPose();
      }
    } else if (r.isLatest) {
      fail(r.error);
    }
  };

  /** driversの配列表現(IPCの引数) */
  const driverList = (drivers: Map<number, number>): Driver[] =>
    [...drivers].map(([hinge, deg]) => ({ hinge, target_angle_deg: deg }));

  /** 全ての折り線に0度(平ら)を指定したdriver列。
   * 何も指定せずに送るとRust側が前回解(warm start)を引き継ぎ、折れたままの
   * 形が返る。平らに戻したいときは必ず0度を明示する必要がある */
  const flatDrivers = (hinges: ReadonlySet<number>): Driver[] =>
    [...hinges].map((hinge) => ({ hinge, target_angle_deg: 0 }));

  /** 元に戻す・やり直しの出発点。戻した先の指定角そのものを出発点にし、
   * 指定の無い折り線だけ平らにする。
   *
   * 以前は全ての折り線を0度にしていたため、角度の表示だけが戻って
   * 3Dは完全な平らのままになっていた(手順を記録していない作品で発生)。
   * 前回の姿勢を引き継がないので、崩れた形から解き直す問題も起きない。 */
  const snapshotSeed = (
    drivers: ReadonlyMap<number, number>,
    hinges: ReadonlySet<number>,
  ): Driver[] =>
    [...hinges].map((hinge) => ({
      hinge,
      target_angle_deg: drivers.get(hinge) ?? 0,
    }));

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
    return generation;
  };

  /** 古い結果が同じ世代番号を再利用しないよう、解除時も番号を進める。 */
  const cancelAngleIntent = (): void => {
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
   * 角度を変える直前のdriversを履歴へ積む(角度を変える操作の入口で必ず呼ぶ)。
   * keyが同じ操作の続き(スライダーを動かしている最中)なら積み直さないので、
   * ドラッグ1回・スライダー1回の操作が履歴1件になる。keyがnullなら常に1件。
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
      angleUndoStack: [...s.angleUndoStack, new Map(s.drivers)].slice(
        -ANGLE_HISTORY_LIMIT,
      ),
      angleRedoStack: [], // 新しい操作をしたらやり直しの先は消える
    });
  };

  /**
   * 作品データを変える要求(展開図の編集・手順の変更)を送る。
   * 成功したらそれが「直前にした操作」になるので、角度の履歴は捨てて
   * 「元に戻す」を作品側(edit_undo)へ回す。断られたときは何も変わって
   * いないので角度の履歴はそのまま残す(角度を戻せなくならないように)。
   */
  const applyDocChange = (
    task: () => Promise<DocumentView>,
    isNewDocument = false,
  ): Promise<void> => {
    const pending = (async () => {
      await runViewCommand(task, isNewDocument);
      if (get().errorMessage === null) clearAngleHistory();
    })();
    // 後続操作が待つためのPromiseはrejectを外へ漏らさない。呼び出し元には元の
    // pendingを返すので、これまでどおり個別の失敗を観測できる。
    latestDocChange = pending.catch(() => undefined);
    return pending;
  };

  /** 現在の指定を必ず有効な基準線へ解決する。不正な一時状態は縦中心へ安全に戻す。 */
  const activeMirrorLine = (
    doc: Document,
    choice: MirrorAxisChoice,
  ): MirrorLine =>
    mirrorLineForChoice(doc, choice) ??
    paperMirrorLine(doc.paper, "paperVertical");

  /** 1回の描画開始時に固定した基準線で、元と反対側の線も含めた追加要求を作る。
   * まだ送らずに返すので、呼び出し側が1回の入力ぶんをまとめて確定できる。
   * 反対側へ広げるのは `applyEdit` でも行うが、そちらは既に反対側を含む並びを
   * 渡しても線を増やさないので、ここで先に作っても本数は変わらない。 */
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

  /** この1回の要求が「線を引く」だけでできているか。 */
  const onlyAddSegments = (ops: EditOp[]): ops is AddSegmentOp[] =>
    ops.every((one) => one.type === "AddSegment");

  /**
   * 左右対称のとき、1回の入力で引く線をまとめて反対側へ広げる(不具合D13)。
   * 手で引く線・曲線だけでなく、作図補助(二等分・垂線・等分・角度線)のように
   * 何本も同時に引く入力も同じ経路を通る。基準線の上に乗る線や、集合の中で
   * 互いに鏡像になっている線は二重に引かない。
   */
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
    const merged = new Map(
      [...s.sequenceTargets].filter(([hinge]) => s.hinges.has(hinge)),
    );
    for (const [hinge, deg] of s.drivers) {
      merged.set(hinge, deg);
    }
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
    const s = get();
    const merged = new Map(
      [...s.sequenceTargets].filter(([hinge]) => s.hinges.has(hinge)),
    );
    for (const [hinge, deg] of s.drivers) {
      merged.set(hinge, deg);
    }
    return [...merged]
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

  const runPoseSolve = async (
    hard: Driver[],
    preferred: Driver[] = [],
    coalesce = false,
    applyFrame = true,
    warmSeed: Driver[] = [],
  ): Promise<void> => {
    // 次の16ms要求がまだキューへ積まれていない間でも、新しい角度操作が
    // 始まった時点から旧世代の表示結果を採用しない。
    const requestGeneration = get().angleIntentGeneration;
    pose.reset();
    const soft = softArg();
    const position = get();
    const total = position.doc?.sequence.length ?? 0;
    const upTo = position.currentStep ?? total;
    const replayT = position.currentStep === null ? 1 : position.playT;
    const call = () => ipc.poseSolve(hard, preferred, soft, warmSeed, upTo, replayT);
    const r = await (coalesce ? queue.runLatest(call) : queue.run(call));
    if (requestGeneration !== get().angleIntentGeneration) return;
    if (!r.ok) {
      if (r.isLatest) fail(r.error);
      return;
    }
    if (!r.isLatest) return;
    const poseAngles = new Map(
      Object.entries(r.value.angles).map(([id, deg]) => [Number(id), deg]),
    );
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
            poseWarnings: r.value.frame.warnings,
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
          }
        : {}),
      poseAngles,
    });
  };

  /** record/end操作が「最後に送った計算」まで待てるようPromiseを保持する。 */
  let latestPosePromise: Promise<void> = Promise.resolve();
  const requestPoseSolve = (
    hard: Driver[],
    preferred: Driver[] = [],
    coalesce = false,
    applyFrame = true,
    warmSeed: Driver[] = [],
  ): Promise<void> => {
    const pending = runPoseSolve(hard, preferred, coalesce, applyFrame, warmSeed);
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

  /** pointer up / blur / Enterでは末尾要求を必ず送り、その結果の後でactiveを消す。 */
  const finishCurrentAngleIntent = async (): Promise<void> => {
    const generation = get().activeAngleIntent?.generation;
    pose.flush();
    const pending = latestPosePromise;
    await pending;
    if (
      generation !== undefined &&
      get().activeAngleIntent?.generation === generation
    ) {
      cancelAngleIntent();
    }
  };

  /**
   * 履歴から取り出したdriversへ戻し、その形を展開図と手順から作り直す。
   * 追従角や3D頂点は履歴へ保存せず、手順なしなら平ら、手順ありなら再生結果を
   * 明示的なwarm seedにして解く。Rust側に残った直前解を出発点にしないため、
   * 未指定の折り目もundo/redo先の状態へ確実に戻る。
   */
  const applyAngleSnapshot = async (
    next: ReadonlyMap<number, number>,
  ): Promise<void> => {
    const drivers = new Map(next);
    set({ drivers });
    cancelAngleIntent();
    const restoreGeneration = get().angleIntentGeneration;
    pose.clearAll(); // 予約済みの間引き計算は古い指定なので捨てる

    const s = get();
    const total = s.doc?.sequence.length ?? 0;
    let warmSeed: Driver[];
    if (total > 0) {
      const upTo = s.currentStep ?? total;
      const t = s.currentStep === null ? 1 : s.playT;
      await runReplay(upTo, t);
      // 連打中に次のundo/redoや新しい角度操作が始まった古い復元は打ち切る。
      if (restoreGeneration !== get().angleIntentGeneration) return;
      if (get().errorMessage !== null) return;
      // driverが無ければ、手順再生の形がそのまま復元結果になる。
      if (drivers.size === 0) return;
      // 追従計算が破綻した後の姿勢を出発点にすると、元に戻しても崩れた形から
      // 計算し直すことになり、戻らない。警告が出ている姿勢は使わず、
      // 手順再生の形(平らな畳み)から解き直す。
      const broken = get().warnings.some(
        (warning) =>
          warning.includes("収束していません") || warning.includes("食い込"),
      );
      warmSeed = broken
        ? snapshotSeed(drivers, s.hinges)
        : driverList(new Map(get().poseAngles));
    } else {
      warmSeed = snapshotSeed(drivers, s.hinges);
    }

    await requestPoseSolve(
      [],
      preferredWithout([]),
      false,
      true,
      warmSeed,
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
    latestPosePromise = Promise.resolve();
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
   * 待ち行列に最新1件だけを置く(runLatest)。追い越された要求は実行されない */
  const runReplay = async (
    upTo: number,
    t: number,
    coalesce = false,
  ): Promise<void> => {
    // poseと同じく、次の角度要求がthrottle待ちでも旧再生を表示しない。
    const requestGeneration = get().angleIntentGeneration;
    const call = () => ipc.sequenceReplay(upTo, t, softArg());
    const r = await (coalesce ? queue.runLatest(call) : queue.run(call));
    if (requestGeneration !== get().angleIntentGeneration) return;
    if (!r.ok) {
      if (r.isLatest) {
        // 再生できない状態のままでは毎コマ失敗するので、止めて理由を知らせる
        stopPlayback();
        fail(r.error);
      }
      return;
    }
    if (!r.isLatest) return;
    const s = get();
    set({
      frame3d: r.value.frame,
      ...softResult(r.value.soft),
      // upToまでの再生結果なので、作品全体のskippedは上書きしない
      replaySkipped: keepIfSame(s.replaySkipped, r.value.skipped),
      replayWarnings: keepIfSame(s.replayWarnings, r.value.warnings),
      flatFoldViolations: keepIfSame(
        s.flatFoldViolations,
        r.value.flat_fold_violations ?? [],
      ),
      suspectHinges: keepIfSame(
        s.suspectHinges,
        r.value.suspect_hinges ?? [],
      ),
      sequenceTargets: new Map(
        [...(r.value.sequence_targets ?? [])]
          .sort((a, b) => a.hinge - b.hinge)
          .map((driver) => [driver.hinge, driver.target_angle_deg]),
      ),
      poseAngles: r.value.angles
        ? new Map(
            Object.entries(r.value.angles).map(([id, deg]) => [Number(id), deg]),
          )
        : s.poseAngles,
      relaxations: r.value.relaxations ?? [],
      poseConverged: r.value.converged ?? true,
      poseBestEffort: r.value.best_effort === true,
      poseClosureRms:
        typeof r.value.closure_rms === "number"
          ? r.value.closure_rms
          : null,
      contactDetected: r.value.contact_detected === true,
    });
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
    invalidateFoldThrough();
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
      set((s) => ({
        frame3d: view.frame,
        replaySkipped: [],
        replayWarnings: [],
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
    await runReplay(step, 1, true);
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
    void runReplay(next.step, next.t, true);
    if (next.playing) cancelFrame = scheduleFrame(tick);
  };

  /** 初回案内の既読状態だけを端末へ覚える(作品の内容には含めない)。 */
  const updateOnboarding = (patch: Partial<typeof onboarding>) => {
    onboarding = { ...onboarding, ...patch };
    saveOnboarding(onboarding);
  };

  /** FoldThroughを作品へ適用する。事前提案の有無にかかわらず最後はこの経路を通る。 */
  const applyFoldThrough = async (
    operation: FoldThroughOperation,
    acceptAdditionalCrease: boolean,
    busyToken: number,
  ): Promise<void> => {
    const s = get();
    if (!s.doc) {
      set({ pendingFoldThrough: null });
      finishFoldThroughBusy(busyToken);
      return;
    }
    const beforeEpoch = s.docEpoch;
    const beforeSequenceCount = s.doc.sequence.length;
    // 末尾へ足すなら最新表示、途中へ挟むなら挟んだ手順を表示する。
    set({
      currentStep:
        operation.up_to === s.doc.sequence.length ? null : operation.up_to + 1,
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
      // 最新でない失敗は共通処理が画面へ出さない。その場合も確認中のままにしない。
      // tokenを照合し、後から始まった別の確認のbusyは消さない。
      finishFoldThroughBusy(busyToken);
    }
    const completed = get();
    if (
      completed.errorMessage === null &&
      completed.docEpoch === beforeEpoch &&
      (completed.doc?.sequence.length ?? 0) > beforeSequenceCount
    ) {
      completed.completeGuideAction("fold");
    }
  };

  /**
   * 折りを適用する前に、縁へぶつかる典型ケースだけを非破壊で調べる。
   * PreviewFoldThroughのDocumentViewは作品更新として反映せず、提案だけを取り出す。
   * これにより、確定前の折り線や編集中の選択を事前確認で失わない。
   */
  const requestFoldThrough = async (operation: FoldThroughOperation): Promise<void> => {
    if (get().foldThroughBusy || get().pendingFoldThrough) return;
    stopPlayback();
    const started = get();
    const revision = ++foldThroughRevision;
    const busyToken = ++foldThroughBusyToken;
    set({
      pendingFoldThrough: null,
      foldThroughBusy: true,
      errorMessage: null,
    });
    const r = await queue.run(() => {
      const previewOperation: Extract<SeqOp, { type: "PreviewFoldThrough" }> & {
        spatial?: SpatialFoldDrag;
      } = {
        type: "PreviewFoldThrough",
        up_to: operation.up_to,
        line: operation.line,
        keep_side_point: operation.keep_side_point,
        target_layers: operation.target_layers,
        direction: operation.direction,
        ...(operation.spatial ? { spatial: operation.spatial } : {}),
      };
      return ipc.sequenceApply(previewOperation);
    });
    // ツール切替など、IPCを伴わない操作でも確認を取り消せるよう世代を先に見る。
    if (revision !== foldThroughRevision) {
      finishFoldThroughBusy(busyToken);
      return;
    }
    if (!r.ok) {
      if (r.isLatest) fail(r.error);
      finishFoldThroughBusy(busyToken);
      return;
    }
    // 後続の編集要求がすでに積まれたなら、その編集前の形に対する候補は使わない。
    if (!r.isLatest) {
      finishFoldThroughBusy(busyToken);
      return;
    }
    const current = get();
    if (
      !current.doc ||
      !canFoldNow(current) ||
      current.docEpoch !== started.docEpoch ||
      current.doc.sequence.length !== started.doc?.sequence.length ||
      foldInsertAt(current) !== operation.up_to
    ) {
      finishFoldThroughBusy(busyToken);
      set({ errorMessage: STALE_DRAFT_MESSAGE });
      return;
    }
    const proposal = r.value.fold_through_proposal ?? null;
    if (proposal) {
      set({
        pendingFoldThrough: {
          proposal,
          operation,
          docEpoch: current.docEpoch,
          stepCount: current.doc.sequence.length,
        },
        // 元の入力はoperationへ移したので、折り線の確定UIは提案UIへ切り替える。
        foldDraft: null,
        alignDraft: null,
        foldThroughBusy: false,
      });
      return;
    }
    // 単純な衝突が無い、または複雑で提案できない場合は従来の折りを続ける。
    await applyFoldThrough(operation, false, busyToken);
  };

  return {
    doc: null,
    stepCreases: [],
    faces: [],
    hinges: new Set<number>(),
    warnings: [],
    flatFoldViolations: [],
    violations: [],
    selection: EMPTY_SELECTION,
    hoveredHinge: null,
    activeTool: "select",
    foldDraft: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    alignDraft: null,
    techniqueDraft: null,
    construct: DEFAULT_CONSTRUCT,
    curve: DEFAULT_CURVE,
    frame3d: null,
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
    errorMessage: null,
    documentSavedPath: null,
    docEpoch: 0,
    drivers: new Map(),
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
    proposalBusy: false,
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
    paperActionTipVisible: false,
    paperActionTipExpanded: false,

    newDocument: (paper) => {
      invalidateFoldThrough();
      return runViewCommand(() => ipc.documentNew(paper), true);
    },

    openDocument: (path) => {
      invalidateFoldThrough();
      return runViewCommand(() => ipc.documentOpen(path), true);
    },

    saveDocument: async (path) => {
      // 次の結果が出るまでは、前回の保存成功を現在の結果として見せない。
      set({ documentSavedPath: null });
      // たわみのつまみを動かした直後でも、保存要求より前に作品データへ確定する。
      await flushSoftSave();
      // 保存も直列化する(直前の編集が確定してから保存されることを保証)。
      // 状態の更新はないので、応答の新旧に関わらず結果を報告する
      const r = await queue.run(() => ipc.documentSave(path));
      if (r.ok) {
        set({ errorMessage: null, documentSavedPath: path });
      } else {
        fail(r.error);
      }
    },

    // 展開図が変わると再生中の形も変わるので、編集系はいずれも先に再生を止める
    // (止めないと、折り直した形が次のコマですぐ上書きされて一瞬跳ねて見える)
    applyEdit: (op) => {
      stopPlayback();
      // 提案は現在のCPから計算したもの。編集要求を積んだ時点で応答前でも無効にする。
      invalidateFoldThrough();
      const ops = Array.isArray(op) ? op : [op];
      if (ops.length === 0) return Promise.resolve();
      // 左右対称のときは、引く・消す・種類を変えるのどれも反対側へ効かせる。
      // 引く線をここでそろえるので、手で引く線も作図補助の線も同じ動きになる
      // (CPE-010 / 不具合D13)。辺IDを増やす消す・種類変更は、展開図の
      // 右クリック消し・Deleteキー・コンテキストパネルのどこから来ても左右そろう。
      const s = get();
      const doc = s.doc;
      const axis =
        s.mirrorDraw && doc ? activeMirrorLine(doc, s.mirrorAxis) : null;
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
      // 1件だけなら今までどおり1件用の要求を送る。複数のときだけまとめ送りにして、
      // 元に戻す1回で入力1回ぶんが戻るようにする(不具合D05)
      return applyDocChange(
        () =>
          mirrored.length === 1
            ? ipc.editApply(mirrored[0])
            : ipc.editApplyBatch(mirrored),
        mirrored.some((one) => one.type === "ReplaceCreasePattern"),
      );
    },

    drawSegment: async (a, b, kind) => {
      if (!get().doc) return;
      // 反対側の線は applyEdit がまとめて作る(作図補助と同じ経路にするため)。
      await get().applyEdit({ type: "AddSegment", a, b, kind });
    },

    // 曲線は「十分細かい折れ線」として入れる(展開図の辺は直線だけなので)。
    // 曲線の折り目は両側の紙が曲がらないと折れない(平らな板2枚を曲線でつなぐと
    // 角度0以外では紙がちぎれる)ので、既定では「紙が曲がるための線」も一緒に
    // 引く。曲がるための線は隣の折り目に突き当たったところで止める。
    // 曲線1本ぶんの線は全部まとめて確定するので、元に戻す1回で引く前へ戻る
    drawCurve: async (points, kind) => {
      const s = get();
      if (!s.doc || points.length < 2) return;
      // 選んだ基準線が交点で分割されても、曲線1回の途中で基準を切り替えない。
      const axis = s.mirrorDraw
        ? activeMirrorLine(s.doc, s.mirrorAxis)
        : null;
      const ops: EditOp[] = [];
      // 曲がるための線をどこで止めるかは、この曲線で先に引いた線も数えて決める
      // (1本ずつ送っていたときと同じ止まり方にするため)。
      const drawn = editableCopy(s.doc);
      const add = (a: Vec2, b: Vec2, k: EdgeKind): void => {
        for (const one of segmentEditsWithAxis(a, b, k, axis)) {
          ops.push(one);
          addWorkingEdge(drawn, one.a, one.b, one.kind);
        }
      };
      for (let i = 0; i + 1 < points.length; i++) {
        add(points[i], points[i + 1], kind);
      }
      if (s.curve.rulings && kind !== "Aux") {
        const long = Math.max(s.doc.paper.width_mm, s.doc.paper.height_mm);
        const paper: Vec2 = [s.doc.paper.width_mm / long, s.doc.paper.height_mm / long];
        // 折り目の両側で曲がる向きが逆になるので、へこむ側は反対の線種にする
        const opposite: EdgeKind = kind === "Mountain" ? "Valley" : "Mountain";
        for (const r of rulingLines(points, paper)) {
          for (const [to, k] of [
            [r.concave, opposite],
            [r.convex, kind],
          ] as [Vec2, EdgeKind][]) {
            add(r.at, firstCrossing(drawn, r.at, to), k);
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
      const s = get();
      if (!s.doc) return;
      const selected = [...new Set(s.selection.edgeIds)];
      if (
        selected.length !== 1 ||
        selectedEdgeMirrorLine(s.doc, selected[0]) === null
      ) {
        return;
      }
      set({
        mirrorAxis: { kind: "selectedLine", edgeId: selected[0] },
        mirrorAxisNotice: null,
      });
      // 辺IDは保存せず、次回起動時の戻り先として縦中心を覚える。
      persistPrefs();
    },

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
      stopPlayback();
      invalidateFoldThrough();
      const s = get();
      const prev = s.angleUndoStack[s.angleUndoStack.length - 1];
      if (prev !== undefined) {
        lastAngleKey = null; // 戻した後の操作は必ず新しい1件にする
        set({
          angleUndoStack: s.angleUndoStack.slice(0, -1),
          angleRedoStack: [...s.angleRedoStack, new Map(s.drivers)].slice(
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
      stopPlayback();
      invalidateFoldThrough();
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
        angleUndoStack: [...s.angleUndoStack, new Map(s.drivers)].slice(
          -ANGLE_HISTORY_LIMIT,
        ),
        errorMessage: null,
      });
      await applyAngleSnapshot(next);
    },

    applySequenceOp: (op) => {
      // 手順が入れ替わると再生位置の意味が変わるので、先に止める
      stopPlayback();
      invalidateFoldThrough();
      return applyDocChange(() => ipc.sequenceApply(op));
    },

    selectStep: (step) => {
      void selectStepAndWait(step);
    },

    selectStepForCapture: (step) => selectStepAndWait(step),

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
      if (s.playing) {
        stopPlayback();
        return;
      }
      const total = s.doc?.sequence.length ?? 0;
      const next = startPlayback(s.currentStep, s.playT, total);
      if (!next.playing) return;
      invalidateFoldThrough();
      // 再生中は形が刻々と変わるので、引きかけの折り線・技法の下ごしらえは捨てる
      set({ foldDraft: null, alignDraft: null, techniqueDraft: null });
      set({ currentStep: next.step, playT: next.t, playing: true });
      lastTs = 0; // 止めていた間の時間は進めない(1コマ目の経過時間は0)
      cancelFrame = scheduleFrame(tick);
    },

    setTool: (tool) => {
      // ツール切替時は選択を保つ必要がないので解除する。
      // 引きかけの折り線も、別のツールへ移った時点で意味を失うので捨てる
      if (get().activeTool !== tool) {
        invalidateFoldThrough();
        set({
          activeTool: tool,
          selection: EMPTY_SELECTION,
          hoveredHinge: null,
          foldDraft: null,
          alignDraft: null,
          techniqueDraft: null,
          pullHinge: null,
          pullMirrorHinge: null,
          operationStage: 0,
          paperActionTipVisible: false,
          paperActionTipExpanded: false,
        });
      }
    },

    setSelection: (selection) =>
      set((s) => {
        // 「いま動かしている折り目」の水色は、その折り目が選択から外れたら消す。
        // 以前は角度を動かし終えても印が残り、何も選んでいないのに3Dの線が
        // 水色に光ったままになっていた(実機で確認)。
        // 紙を引いている間は選択と関係なく動かしているので、そのときは残す。
        const stillMoving =
          s.pullHinge !== null ||
          (s.activeAngleIntent?.hinges ?? []).some((hinge) =>
            selection.edgeIds.includes(hinge),
          );
        const dropActive = s.activeAngleIntent !== null && !stillMoving;
        return {
          selection,
          // スライダー行が消える選択変更なら、残ったホバー色も一緒に消す。
          hoveredHinge:
            s.hoveredHinge !== null && selection.edgeIds.includes(s.hoveredHinge)
              ? s.hoveredHinge
              : null,
          ...(dropActive
            ? {
                activeAngleIntent: null,
                angleIntentGeneration: s.angleIntentGeneration + 1,
              }
            : {}),
        };
      }),

    setHoveredHinge: (hinge) =>
      set((s) => ({
        hoveredHinge:
          hinge !== null &&
          s.hinges.has(hinge) &&
          (s.selection.edgeIds.includes(hinge) ||
            relaxationNotices(s.relaxations).some((item) => item.hinge === hinge))
            ? hinge
            : null,
      })),

    beginFoldDraft: (line, source) => {
      const s = get();
      // 事前確認中の折りAへ、別の折りBを上書きしない。回答後に改めて引ける。
      if (!s.doc || s.foldThroughBusy || s.pendingFoldThrough) return;
      // 展開図の座標と畳み平面の座標が一致するのは1回も折っていないときだけ
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
          // 線を引いた時点の形を覚えておき、折るときに食い違いを見つける
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
          // 向きや動かす側へ触れたら、最後の「折るで確定」を強調する。
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
      // 「やめる」は合わせて折るの途中経過もまとめて捨てる(選び直しは合わせ方から)
      if (get().foldDraft || get().alignDraft)
        set({ foldDraft: null, alignDraft: null });
    },

    commitFoldDraft: async () => {
      const s = get();
      const draft = s.foldDraft;
      if (!draft || !s.doc) return;
      // 線を引いた時点と今とで形が違えば、そのまま折ると黙って違う位置に折り目が
      // 入る。折らずに捨てて、引き直してもらう
      if (
        !canFoldNow(s) ||
        draft.docEpoch !== s.docEpoch ||
        draft.stepCount !== s.doc.sequence.length ||
        draft.upTo !== foldInsertAt(s)
      ) {
        set({ foldDraft: null, alignDraft: null, errorMessage: STALE_DRAFT_MESSAGE });
        return;
      }
      const keep = keepSidePoint(draft.line, draft.movingSide);
      let targetLayers: number[] | null = null;
      if (draft.target === "top") {
        const layers = foldLayers(s.frame3d, s.doc, s.faces);
        const top = topMovingFace(layers, draft.line, keep);
        if (top === null) {
          set({ errorMessage: "折り線の動く側に紙がありません" });
          return;
        }
        targetLayers = [top];
      }
      const alignment =
        s.alignDraft && isAlignComplete(s.alignDraft)
          ? { mode: s.alignDraft.mode, picks: [...s.alignDraft.picks] }
          : null;
      await requestFoldThrough({
        type: "FoldThrough",
        up_to: draft.upTo,
        line: draft.line,
        keep_side_point: keep,
        target_layers: targetLayers,
        direction: draft.direction,
        ...(alignment ? { alignment } : {}),
      });
    },

    resolveFoldThroughProposal: async (accept) => {
      const s = get();
      const pending = s.pendingFoldThrough;
      if (!pending || s.foldThroughBusy) return;
      if (
        !s.doc ||
        !canFoldNow(s) ||
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
      const busyToken = ++foldThroughBusyToken;
      set({ foldThroughBusy: true, errorMessage: null });
      await applyFoldThrough(pending.operation, accept, busyToken);
    },

    beginAlign: (mode) => {
      const s = get();
      if (!s.doc) return;
      invalidateFoldThrough();
      if (stepReplayPending) {
        set({
          errorMessage:
            "表示を切り替えています。切り替わってから、もう一度合わせ方を選んでください",
        });
        return;
      }
      // 同じ合わせ方をもう一度押したらやめる(入る・出るを1つのボタンで済ませる)
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
      // 選び終えたあとにもう一度選んだら、1つ目から選び直す
      const picks = isAlignComplete(draft) ? [target] : [...draft.picks, target];
      if (steps[picks.length - 1] !== target.kind) return; // 種類違いは受け付けない
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
        // 向き・対象の層など、パネルで決めた設定は引き継ぐ(線と動く側だけ入れ替える)
        foldDraft: s.foldDraft
          ? {
              ...s.foldDraft,
              line,
              movingSide: movingSideOf(line, alignRefPoint(draft.picks[0])),
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

    foldByDrag: async (from, to, mode, grabFace = null, direction = "Up") => {
      const s = get();
      if (!s.doc || s.foldThroughBusy || s.pendingFoldThrough) return;
      // 折れない状態は「なぜできないか」を短い日本語で伝える(要件UI-009)
      const reason = foldBlockReason({
        hasDoc: true,
        playing: s.playing,
        playT: s.playT,
        driverCount: s.drivers.size,
        currentStep: s.currentStep,
        stepCount: s.doc.sequence.length,
      });
      if (reason) {
        set({ errorMessage: reason });
        return;
      }
      const upTo = foldInsertAt(s);
      if (isSpatialFoldFrame(s.frame3d)) {
        // 非平坦では単一の上下順やXY投影を使えない。実際につかんだ3D点を
        // Rustへ渡し、再生した形の上で折り平面・つながった対象面・山谷を決める。
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
          // spatial分岐ではRustが使わない。旧serde形を保つ有限・非退化な値。
          line: [
            [0, 0],
            [1, 0],
          ],
          keep_side_point: [0, 1],
          target_layers: null,
          direction,
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
      // つかんだ紙は離した位置へ倒れてくる(=手前へ折る)。
      // 引きかけの折り線が残っていても、この操作で決着させる
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
            // ドラッグで引いた軸は既存折り目IDを保証できない。
            motionAxisEdgeId: draft.kind === "Simple" ? null : draft.motionAxisEdgeId,
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
      if (!draft || draft.kind !== "Simple" || draft.motionParts.length === 0) return;
      set({
        techniqueDraft: {
          ...draft,
          motionParts: draft.motionParts.slice(0, -1),
        },
        errorMessage: null,
      });
    },

    addTechniqueVertex: (p) => {
      const draft = get().techniqueDraft;
      if (draft) {
        set({ techniqueDraft: { ...draft, polygon: addTwistVertex(draft.polygon, p) } });
      }
    },

    undoTechniqueVertex: () => {
      const draft = get().techniqueDraft;
      if (draft && draft.polygon.length > 0) {
        set({ techniqueDraft: { ...draft, polygon: undoTwistVertex(draft.polygon) } });
      }
    },

    setTechniqueCenter: (p) => {
      const draft = get().techniqueDraft;
      if (draft) set({ techniqueDraft: { ...draft, center: p } });
    },

    setTechniqueReferencePoint: (p) => {
      const draft = get().techniqueDraft;
      if (draft && draft.kind !== "Simple" && draft.kind !== "Twist") {
        set({ techniqueDraft: { ...draft, referencePoint: p } });
      }
    },

    updateTechniqueDraft: (patch) => {
      const draft = get().techniqueDraft;
      if (draft) set({ techniqueDraft: { ...draft, ...patch } });
    },

    setConstruct: (patch) =>
      set((s) => ({ construct: { ...s.construct, ...patch } })),

    setCurve: (patch) => set((s) => ({ curve: { ...s.curve, ...patch } })),

    cancelTechnique: () => {
      if (get().techniqueDraft) set({ techniqueDraft: null });
    },

    commitTechnique: async () => {
      const s = get();
      const draft = s.techniqueDraft;
      if (!draft || !s.doc) return;
      // TechniqueKind::Simpleは通常の単純折りではなく、技法メニュー内の
      // 「層操作」入口として使う。FlatMotionは名前付き技法とは入力形が違う。
      if (draft.kind === "Simple") {
        if (
          !canFoldNow(s) ||
          draft.docEpoch !== s.docEpoch ||
          draft.stepCount !== s.doc.sequence.length ||
          draft.upTo !== foldInsertAt(s)
        ) {
          set({ techniqueDraft: null, errorMessage: STALE_DRAFT_MESSAGE });
          return;
        }
        const parts = [...draft.motionParts];
        const current = layerMotionPartDraft(draft);
        if (hasLayerMotionInput(current)) {
          if (draft.motionMode === "reflect" && draft.motionAxisEdgeId === null) {
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
          currentStep: draft.upTo === s.doc.sequence.length ? null : draft.upTo + 1,
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
      // ねじり折りは中央多角形を頂点で指せる(辺の数も長さも仮定しない)。
      // 3点以上そろっていれば、折り線1本の指し方(正多角形)より優先する
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
      // 選んだ時点と今とで形が違えば、そのまま折ると違う位置に折り目が入る
      if (
        !canFoldNow(s) ||
        draft.docEpoch !== s.docEpoch ||
        draft.stepCount !== s.doc.sequence.length ||
        draft.upTo !== foldInsertAt(s)
      ) {
        set({ techniqueDraft: null, errorMessage: STALE_DRAFT_MESSAGE });
        return;
      }
      // Rust側と同じ層数規約にする。中割り・かぶせだけ2枚以上、つぶし・花弁は
      // 1枚以上。段・沈め・ひだ寄せ・ねじりは空なら領域の全層を対象にする。
      const minimumFlap = minimumTechniqueFlap(draft.kind);
      if (draft.flap.length < minimumFlap) {
        set({
          errorMessage:
            `先に立体表示で紙をクリックし、対象の層(フラップ)を${minimumFlap}枚以上選んでください`,
        });
        return;
      }
      // 基準点の意味は技法ごとに違う。段折りは2本目の折り線の位置(段の幅ぶん
      // 動く側へ離した点)、中割り・かぶせは先端が向かう側(動かない側)の点、
      // 多角形で指したねじり折りはねじる角(向きは「動かす側」で決める)
      const scale = Math.max(s.doc.paper.width_mm, s.doc.paper.height_mm);
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
      // 多角形を指したときは1辺目をlineとして送る(エンジンはpolygonを優先する)
      const line: [Vec2, Vec2] =
        draft.line ?? [draft.polygon[0], draft.polygon[1]];
      const lineLength = Math.hypot(
        line[1][0] - line[0][0],
        line[1][1] - line[0][1],
      );
      // 沈め折りのreference_pointは「沈める先端側」そのもの。他の技法で使う
      // keepSidePoint(動かさない側・行き先)とは意味が逆なので、未指定時も
      // movingSideと同じ側へ点を置く。Ctrl+クリックの明示点は全技法で優先する。
      const automaticReference =
        draft.kind === "Pleat"
          ? offsetPoint(line, draft.movingSide, draft.widthMm / scale)
          : draft.kind === "OpenSink"
            ? offsetPoint(line, draft.movingSide, Math.max(0.01, lineLength * 0.25))
            : keepSidePoint(line, draft.movingSide);
      const reference =
        twistRef ??
        draft.referencePoint ??
        automaticReference;
      // 折った結果を見せる。末尾へ足したなら最新、途中へ挟んだなら挟んだ手順
      set({ currentStep: draft.upTo === s.doc.sequence.length ? null : draft.upTo + 1 });
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
        // 技法が当てはまらない形だったときは代わりの手を案内する(要件§12)
        set({ errorMessage: `${error}(${TECHNIQUE_FALLBACK_HINT})` });
      }
    },

    beginPull: (hinge, angles, mirrorHinge = null) => {
      if (!get().doc) return;
      invalidateFoldThrough();
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
      invalidateFoldThrough();
      const before = get().drivers.get(hinge) ?? get().poseAngles.get(hinge) ?? 0;
      // スライダーを動かしている間の細かい変更は1件にまとめる
      pushAngleUndo(`angle:${hinge}`);
      // 画面の反応を優先し、指定はその場で反映してから計算を間引いて依頼する
      const drivers = new Map(get().drivers);
      drivers.set(hinge, deg);
      set({ drivers });
      if (Math.abs(deg - before) >= 1) get().completeGuideAction("angle");
      // いま動かしている1本だけを固定する。以前に指定した折り線まで固定すると
      // 内部頂点まわりの拘束と両立せず、面が離れて紙が切れて見える
      activateAngleIntent([hinge]);
      pose.schedule();
    },

    setDriverAngles: (hinges, deg) => {
      invalidateFoldThrough();
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
      set({ drivers });
      if (changedForGuide) get().completeGuideAction("angle");
      // 動かしている折り目は選んだ全部(3Dで全部が水色に光る)。
      // そのうち角度を固定するのは代表1本だけ(fixAll=false)。
      activateAngleIntent(valid, false);
      pose.schedule();
    },

    finishAngleIntent: finishCurrentAngleIntent,

    clearDriver: (hinge) => {
      const drivers = new Map(get().drivers);
      if (!drivers.delete(hinge)) return;
      invalidateFoldThrough();
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
      // 再生tickがこの0度計算より後に積まれると、正しい平坦結果が古い応答として
      // 捨てられるか、再生結果で折れた形へ上書きされる。予約中のコマを先に止め、
      // 「全て平らに戻す」を最後の表示要求にする。
      stopPlayback();
      const hinges = get().hinges;
      invalidateFoldThrough();
      if (get().drivers.size > 0) pushAngleUndo(null);
      set({ drivers: new Map() });
      cancelAngleIntent();
      // 全ての折り線を0度に固定する形は必ず閉じる(平ら)ので全部hardでよい
      void requestPoseSolve(flatDrivers(hinges));
    },

    recordPoseStep: async () => {
      const before = get();
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
      proposalGeneration++;
      set({
        proposalStep: "skeleton",
        proposalSkeleton: defaultSkeleton(),
        proposalCandidates: [],
        proposalSelected: null,
        proposalBusy: false,
        proposalError: null,
      });
    },

    closeProposal: () => {
      proposalGeneration++;
      set({ proposalStep: null, proposalBusy: false });
    },

    setProposalStep: (step) => set({ proposalStep: step }),

    // 骨格を触ったら前の候補は別物になるので捨てる(古い形のまま選べてしまうのを防ぐ)
    setProposalSkeleton: (skeleton) =>
      set({
        proposalSkeleton: skeleton,
        proposalCandidates: [],
        proposalSelected: null,
      }),

    generateProposal: async () => {
      const s = get();
      if (s.proposalBusy) return;
      const paper = s.doc?.paper ?? FALLBACK_PAPER;
      const seed = s.proposalSeed;
      const generation = ++proposalGeneration;
      set({ proposalBusy: true, proposalError: null, proposalSeed: seed + 1 });
      // 提案の計算は作品の状態を読まない独立処理。直列化キューに載せると
      // 数百msの計算の間だけ編集が止まるので、ここは載せずに直接呼ぶ
      try {
        const list = await ipc.proposalGenerate(s.proposalSkeleton, paper, seed);
        if (generation !== proposalGeneration) return;
        set({
          proposalCandidates: list,
          proposalSelected: list.length > 0 ? 0 : null,
          proposalStep: list.length > 0 ? "candidates" : "skeleton",
          proposalError:
            list.length > 0 ? null : "候補を作れませんでした。骨格を変えてみてください",
          proposalBusy: false,
        });
      } catch (e) {
        if (generation !== proposalGeneration) return;
        set({
          proposalBusy: false,
          proposalError: typeof e === "string" ? e : String(e),
        });
      }
    },

    selectProposalCandidate: (index) => {
      const list = get().proposalCandidates;
      if (index < 0 || index >= list.length) return;
      set({ proposalSelected: index });
    },

    applyProposalCandidate: async () => {
      const s = get();
      const chosen =
        s.proposalSelected === null
          ? undefined
          : s.proposalCandidates[s.proposalSelected];
      if (!chosen) return;
      // 以後は普通の展開図として自由に編集できる(PRO-003)。
      // 元に戻せる操作なので、通常の編集と同じ経路(edit_apply)で流し込む
      proposalGeneration++;
      set({ proposalStep: null });
      await get().applyEdit({ type: "ReplaceCreasePattern", cp: chosen.cp });
    },

    openExport: () =>
      set({ exportOpen: true, exportError: null, exportSavedPath: null }),

    closeExport: () => set({ exportOpen: false, exportBusy: false }),

    // 指定を変えたら前回の「保存しました」は別の話になるので消す
    setExportOption: (patch) =>
      set({ ...patch, exportError: null, exportSavedPath: null }),

    runExport: async (path) => {
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
      const visibleSteps = get().doc?.sequence ?? [];
      const visibleFrom = number - 1;
      const visibleTo = visibleFrom + delta;
      const id = visibleSteps[visibleFrom]?.id;
      if (id === undefined || visibleTo < 0 || visibleTo >= visibleSteps.length) return;

      // 覚え書き・折り方の確定直後は、UpdateStepがFIFOに積まれていてもdocは
      // まだ旧値のことがある。確定結果の反映を待ち、IDから最新の手順を取り直す。
      const changesBeforeMove = latestDocChange;
      await changesBeforeMove;
      const steps = get().doc?.sequence ?? [];
      const from = steps.findIndex((candidate) => candidate.id === id);
      const to = from + delta;
      const step = steps[from];
      if (!step || from < 0 || to < 0 || to >= steps.length) return;
      // 途中への挿入はSeqOpに用意されているが、折り操作そのものを途中へ
      // 挟むのは断る仕様なので、既にある手順の位置替えとして使う
      // (取り除いてから入れ直す。元に戻すは2回ぶんになる)
      await get().applySequenceOp({ type: "RemoveStep", id: step.id });
      if (get().errorMessage !== null) return;
      await get().applySequenceOp({ type: "InsertStep", index: to, step });
      if (get().errorMessage === null) get().selectStep(to + 1);
    },
  };
});
