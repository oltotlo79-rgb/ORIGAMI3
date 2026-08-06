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
import {
  DEFAULT_DISPLAY,
  clampDivisions,
  clampSplitRatio,
  loadPrefs,
  savePrefs,
} from "../lib/displayPrefs";
import { mirrorAxisX, mirrorSegments } from "../lib/mirror";
import {
  DEFAULT_TWIST_DEG,
  addTwistVertex,
  isTwistPolygonReady,
  polygonCentroid,
  twistReferencePoint,
  undoTwistVertex,
} from "../lib/twistPolygon";
import type {
  Document,
  DisplaySettings,
  DocumentView,
  Driver,
  EdgeKind,
  EditOp,
  ExportKind,
  Face,
  FoldDirection,
  Frame3D,
  Paper,
  ProposalCandidate,
  RecoveryInfo,
  SeqOp,
  Skeleton,
  TechniqueKind,
  Vec2,
} from "../lib/types";
import { defaultSkeleton } from "../lib/skeleton";

/** ヒンジ角の連続操作(スライダー)を間引く間隔(ms) */
const POSE_THROTTLE_MS = 60;

/** 画面更新の仕組みが無い環境(テスト)で1コマを送る間隔(ms) */
const FALLBACK_FRAME_MS = 16;

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

/** 技法の下ごしらえ(選んだ技法・フラップ・折り線)。確定するまで保持する */
export interface TechniqueDraft {
  /** 選んだ技法(実装済みのものだけ。lib/techniques.tsのSUPPORTED_TECHNIQUESを参照) */
  kind: TechniqueKind;
  /** 対象フラップ(3D表示でクリックした場所に重なっている層の面ID) */
  flap: number[];
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
  /** ねじり折りのねじる角(度)。動かす側の指定で向きが決まる */
  twistDeg: number;
  /** 選んだ時点の作品の世代番号(新規・開くで変わる) */
  docEpoch: number;
  /** 選んだ時点の手順の数 */
  stepCount: number;
  /** 選んだ時点で見ていた位置(=技法が挟まる位置)。FoldDraftと同じ意味 */
  upTo: number;
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
  if (!hasPoseAngle(currentAngles(s.hinges, s.drivers, s.poseAngles))) {
    return "まだ角度が付いていません(折り線を選んで角度を変えてください)";
  }
  return null;
}

interface AppState {
  doc: Document | null;
  faces: Face[];
  warnings: string[];
  violations: number[];
  selection: Selection;
  activeTool: ToolId;
  /** 引いたばかりの折り線と確定前の設定。nullなら折り線を引いていない */
  foldDraft: FoldDraft | null;
  /** 選んだ技法と、その下ごしらえ(フラップ・折り線)。nullなら技法を選んでいない */
  techniqueDraft: TechniqueDraft | null;
  /** 作図補助の選択(どの作図か・等分数・角度の刻み)。CpEditorが使う */
  construct: ConstructOptions;
  /** 3D表示フレーム。nullなら平ら(展開図から直接描く) */
  frame3d: Frame3D | null;
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
  /** 作品の世代番号。新規/開くの成功で増える(エディタの表示リセット合図) */
  docEpoch: number;
  /** 利用者が指定した折り角度(度)。キーは辺ID */
  drivers: Map<number, number>;
  /** 直近のソルバー解(度)。キーは辺ID。未指定ヒンジの現在角の表示に使う */
  poseAngles: Map<number, number>;
  /** 追従計算からの警告(不収束など)。展開図の検査警告とは別に持つ */
  poseWarnings: string[];
  /** 追従計算が収束したか(falseなら3D区画のバッジで知らせる) */
  poseConverged: boolean;
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
  /** 左右対称に線を引くか(CPE-010)。紙の縦の中心線が対称軸。
   * 効くのは線を引くときだけで、消す・種類を変えるときは片側ずつになる */
  mirrorDraw: boolean;
  /** 3Dで紙を引くとき左右対称の相手も同時に動かすか(UI-007)。既定はオン。
   * 画面の使い方の好みなので端末に覚えておく(作品の中身には入れない) */
  pullMirror: boolean;

  newDocument: (paper: Paper) => Promise<void>;
  openDocument: (path: string) => Promise<void>;
  saveDocument: (path: string | null) => Promise<void>;
  applyEdit: (op: EditOp) => Promise<void>;
  /** 展開図に線を1本引く(CPE-010)。左右対称のときは中心線で折り返した線も
   * 続けて引く(引いた順に直列化キューへ積むので適用順は変わらない) */
  drawSegment: (a: Vec2, b: Vec2, kind: EdgeKind) => Promise<void>;
  /** 左右対称に線を引くかを切り替える(次回起動時も同じ設定に戻る) */
  setMirrorDraw: (on: boolean) => void;
  /** 3Dで引くとき左右同時に動かすかを切り替える(次回起動時も同じ設定に戻る) */
  setPullMirror: (on: boolean) => void;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
  /** 手順の追加・変更・削除(sequence_apply)。再生中なら止めてから送る */
  applySequenceOp: (op: SeqOp) => Promise<void>;
  /** 表示する手順を選ぶ(0=折る前、null=最新)。再生中なら止める */
  selectStep: (step: number | null) => void;
  /** 表示する手順を前後に動かす(コマ送り) */
  stepBy: (delta: number) => void;
  /** 再生と一時停止を切り替える */
  togglePlay: () => void;
  setTool: (tool: ToolId) => void;
  setSelection: (selection: Selection) => void;
  /** 引いた折り線を確定前の状態として置く(source="2d"は手順がある間は断る) */
  beginFoldDraft: (line: [Vec2, Vec2], source: "2d" | "3d") => void;
  /** 確定前の折りの設定(向き・対象層・動かす側)を変える */
  updateFoldDraft: (patch: Partial<FoldDraft>) => void;
  /** 引いた折り線を捨てる */
  cancelFoldDraft: () => void;
  /** 引いた折り線で実際に折る(sequence_apply FoldThrough)。成功したら折り線を捨てる */
  commitFoldDraft: () => Promise<void>;
  /**
   * 紙をつかんで動かす折り操作(UI-007)。つかんだ点fromから離した点toへの
   * ドラッグを折り線・対象の層に翻訳して、そのまま折る(パネル操作は要らない)。
   * grabFaceは立体表示で実際につかんだ面(raycastで拾えなければnull)
   */
  foldByDrag: (
    from: Vec2,
    to: Vec2,
    mode: GrabMode,
    grabFace?: number | null,
  ) => Promise<void>;
  /** 技法を選ぶ(ツールレールのサブメニュー)。下ごしらえを作り直す */
  beginTechnique: (kind: TechniqueKind) => void;
  /** 3D表示でクリックした場所の層をフラップとして選ぶ */
  setTechniqueFlap: (faces: number[]) => void;
  /** 技法の折り線を引く */
  setTechniqueLine: (line: [Vec2, Vec2]) => void;
  /** ねじり折りの中央多角形へ頂点を1つ足す(3D画面のクリック) */
  addTechniqueVertex: (p: Vec2) => void;
  /** 直前に足した頂点を取り消す(頂点が無ければ何もしない) */
  undoTechniqueVertex: () => void;
  /** ねじり折りの中心を指定する(nullで多角形の重心へ戻す) */
  setTechniqueCenter: (p: Vec2 | null) => void;
  /** 技法の設定(動かす側・段の幅)を変える */
  updateTechniqueDraft: (patch: Partial<TechniqueDraft>) => void;
  /** 作図補助(CPE-005)の選び方を変える(どの作図か・等分数・角度の刻み) */
  setConstruct: (patch: Partial<ConstructOptions>) => void;
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
  /** 引いている間の角度(度)。60ms間引きで追従計算を呼ぶ。
   * 左右対称の相手がいれば同じ角度で一緒に動かす */
  pullTo: (deg: number) => void;
  /** 引く操作を終える(角度指定は残る。色付けだけ消す) */
  endPull: () => void;
  /** ヒンジの折り角度を指定する(60ms間引きで追従計算を呼ぶ) */
  setDriverAngle: (hinge: number, deg: number) => void;
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
  /** 2D区画と3D区画の分割比を変える(次回起動時も同じ位置に戻る) */
  setSplitRatio: (ratio: number) => void;
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
  const prefs = loadPrefs();

  /** 画面の使い方の好み(作品の中身ではないもの)を端末に覚えておく */
  const persistPrefs = () => {
    const { splitRatio, mirrorDraw, pullMirror } = get();
    savePrefs({ splitRatio, mirrorDraw, pullMirror });
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
    set((s) => ({
      // 紙の色・方眼は作品ごとの設定(doc.display)。ここを唯一の拠り所にして
      // 画面側の写し(display)をそろえる。人からもらった作品を開けば、
      // その作品の色と方眼がそのまま出る
      doc: view.doc,
      display: view.doc.display,
      foldDraft: null,
      techniqueDraft: null,
      faces: view.faces,
      hinges: hingeEdgeIds(view.doc, view.faces),
      warnings: view.warnings,
      violations: view.violations,
      skipped: view.skipped,
      currentStep:
        total === 0 || s.currentStep === null
          ? null
          : Math.min(s.currentStep, total),
      selection: isNewDocument
        ? EMPTY_SELECTION
        : pruneSelection(s.selection, view.doc),
      errorMessage: null,
      docEpoch: isNewDocument ? s.docEpoch + 1 : s.docEpoch,
    }));
  };

  /** IPC失敗(reject)をerrorMessageへ反映する */
  const fail = (e: unknown) => {
    set({ errorMessage: typeof e === "string" ? e : String(e) });
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
        set({
          drivers: new Map(),
          poseAngles: new Map(),
          poseWarnings: [],
          poseConverged: true,
          pullHinge: null,
          pullMirrorHinge: null,
          frame3d: r.value.frame,
          currentStep: null,
          playT: 1,
          replaySkipped: [],
          replayWarnings: [],
          foldDraft: null,
          techniqueDraft: null,
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

  /** 追従計算を直列化キュー経由で実行し、3D表示へ反映する。
   * coalesce=true(スライダーの連続操作・手順再生)は「最新の形が出れば良い」
   * ので待ち行列に最新1件だけを置く(runLatest)。追い越された要求は実行されない。
   * 一方、解除操作のように「その1回だけ0度を明示する」意味を持つ要求は、
   * 追い越されると意味が失われるのでFIFO(run)で必ず送る。
   * 実行された成功応答は完了順に全て適用する(runViewCommandと同じ規約)。
   * 成功時にerrorMessageは触らない(編集側のエラー報告を消さないため) */
  const runPoseSolve = async (
    drivers: Driver[],
    coalesce = false,
    applyFrame = true,
  ): Promise<void> => {
    pose.reset();
    const call = () => ipc.poseSolve(drivers);
    const r = await (coalesce ? queue.runLatest(call) : queue.run(call));
    if (r.ok) {
      set({
        // 出発点合わせ(applyFrame=false)では形は変わらないので、手順再生が
        // 持っていた層の重なり情報を消さないよう立体表示はそのままにする
        ...(applyFrame ? { frame3d: r.value.frame } : {}),
        poseWarnings: r.value.frame.warnings,
        poseConverged: r.value.converged,
        poseAngles: new Map(
          Object.entries(r.value.angles).map(([id, deg]) => [Number(id), deg]),
        ),
      });
    } else if (r.isLatest) {
      fail(r.error);
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
      await runPoseSolve(flatDrivers(hinges));
      return;
    }
    await runPoseSolve(driverList(kept));
  };

  // スライダーの連続操作を間引く(実行時点の最新driversを送る)。
  // 間引いてもなお計算が追いつかない場合に備え、待ち行列は最新1件だけにする
  const pose = createTrailingThrottle(POSE_THROTTLE_MS, () => {
    void runPoseSolve(driverList(get().drivers), true);
  });
  resetThrottle = pose.clearAll;

  /** 手順の再生結果を3D表示へ反映する。
   * coalesce=true(再生アニメーション)は「最新の形が出れば良い」ので
   * 待ち行列に最新1件だけを置く(runLatest)。追い越された要求は実行されない */
  const runReplay = async (
    upTo: number,
    t: number,
    coalesce = false,
  ): Promise<void> => {
    const call = () => ipc.sequenceReplay(upTo, t);
    const r = await (coalesce ? queue.runLatest(call) : queue.run(call));
    if (r.ok) {
      const s = get();
      set({
        frame3d: r.value.frame,
        // upToまでの再生結果なので、作品全体のskippedは上書きしない
        replaySkipped: keepIfSame(s.replaySkipped, r.value.skipped),
        replayWarnings: keepIfSame(s.replayWarnings, r.value.warnings),
      });
    } else if (r.isLatest) {
      // 再生できない状態のままでは毎コマ失敗するので、止めて理由を知らせる
      stopPlayback();
      fail(r.error);
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
      set({ frame3d: view.frame, replaySkipped: [], replayWarnings: [] });
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

  return {
    doc: null,
    faces: [],
    hinges: new Set<number>(),
    warnings: [],
    violations: [],
    selection: EMPTY_SELECTION,
    activeTool: "select",
    foldDraft: null,
    techniqueDraft: null,
    construct: DEFAULT_CONSTRUCT,
    frame3d: null,
    currentStep: null,
    playT: 1,
    playing: false,
    skipped: [],
    replaySkipped: [],
    replayWarnings: [],
    errorMessage: null,
    docEpoch: 0,
    drivers: new Map(),
    poseAngles: new Map(),
    poseWarnings: [],
    poseConverged: true,
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
    mirrorDraw: prefs.mirrorDraw,
    pullMirror: prefs.pullMirror,

    newDocument: (paper) => runViewCommand(() => ipc.documentNew(paper), true),

    openDocument: (path) => runViewCommand(() => ipc.documentOpen(path), true),

    saveDocument: async (path) => {
      // 保存も直列化する(直前の編集が確定してから保存されることを保証)。
      // 状態の更新はないので、応答の新旧に関わらず結果を報告する
      const r = await queue.run(() => ipc.documentSave(path));
      if (r.ok) {
        set({ errorMessage: null });
      } else {
        fail(r.error);
      }
    },

    // 展開図が変わると再生中の形も変わるので、編集系はいずれも先に再生を止める
    // (止めないと、折り直した形が次のコマですぐ上書きされて一瞬跳ねて見える)
    applyEdit: (op) => {
      stopPlayback();
      return runViewCommand(() => ipc.editApply(op), false);
    },

    drawSegment: async (a, b, kind) => {
      const s = get();
      if (!s.doc) return;
      // 左右対称のときは中心線の反対側にも同じ線を引く。中心線に重なる線や
      // もともと左右対称な線は、同じ線が二重にならないよう1本だけにする
      const segments = s.mirrorDraw
        ? mirrorSegments([a, b], mirrorAxisX(s.doc.paper))
        : [[a, b] as [Vec2, Vec2]];
      for (const [p, q] of segments) {
        await get().applyEdit({ type: "AddSegment", a: p, b: q, kind });
        // 1本目で断られたら理由を残したまま止める(片側だけ引かれた形にしない)
        if (get().errorMessage !== null) return;
      }
    },

    setMirrorDraw: (on) => {
      set({ mirrorDraw: on });
      persistPrefs();
    },

    setPullMirror: (on) => {
      set({ pullMirror: on });
      // 切ったら、いま一緒に動かしている相手もその場で外す(次のドラッグを待たない)
      if (!on) set({ pullMirrorHinge: null });
      persistPrefs();
    },

    undo: () => {
      stopPlayback();
      return runViewCommand(() => ipc.editUndo(), false);
    },

    redo: () => {
      stopPlayback();
      return runViewCommand(() => ipc.editRedo(), false);
    },

    applySequenceOp: (op) => {
      // 手順が入れ替わると再生位置の意味が変わるので、先に止める
      stopPlayback();
      return runViewCommand(() => ipc.sequenceApply(op), false);
    },

    selectStep: (step) => {
      stopPlayback();
      // 別の手順の形を見せる操作なので、その前の形の上に引いた折り線は捨てる
      // (残すとコンテキストパネルに折りUIが出たままになり、手順の設定も出せない)
      if (get().foldDraft) set({ foldDraft: null });
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
      set({ currentStep: next, playT: 1 });
      // 最新(null)の形も再生で作る(DocumentViewのframeと同じ内容になる)。
      // 途中まで折った表示から戻すときに、必ず最新の形へ描き直すため
      void runReplay(upTo, 1);
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
      if (s.playing) {
        stopPlayback();
        return;
      }
      const total = s.doc?.sequence.length ?? 0;
      const next = startPlayback(s.currentStep, s.playT, total);
      if (!next.playing) return;
      // 再生中は形が刻々と変わるので、引きかけの折り線・技法の下ごしらえは捨てる
      set({ foldDraft: null, techniqueDraft: null });
      set({ currentStep: next.step, playT: next.t, playing: true });
      lastTs = 0; // 止めていた間の時間は進めない(1コマ目の経過時間は0)
      cancelFrame = scheduleFrame(tick);
    },

    setTool: (tool) => {
      // ツール切替時は選択を保つ必要がないので解除する。
      // 引きかけの折り線も、別のツールへ移った時点で意味を失うので捨てる
      if (get().activeTool !== tool) {
        set({
          activeTool: tool,
          selection: EMPTY_SELECTION,
          foldDraft: null,
          techniqueDraft: null,
          pullHinge: null,
          pullMirrorHinge: null,
        });
      }
    },

    setSelection: (selection) => set({ selection }),

    beginFoldDraft: (line, source) => {
      const s = get();
      if (!s.doc) return;
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
      });
    },

    updateFoldDraft: (patch) => {
      const draft = get().foldDraft;
      if (draft) set({ foldDraft: { ...draft, ...patch } });
    },

    cancelFoldDraft: () => {
      if (get().foldDraft) set({ foldDraft: null });
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
        set({ foldDraft: null, errorMessage: STALE_DRAFT_MESSAGE });
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
      // 折った結果を見せる。末尾へ足したなら最新、途中へ挟んだなら挟んだ手順
      set({ currentStep: draft.upTo === s.doc.sequence.length ? null : draft.upTo + 1 });
      await get().applySequenceOp({
        type: "FoldThrough",
        up_to: draft.upTo,
        line: draft.line,
        keep_side_point: keep,
        target_layers: targetLayers,
        direction: draft.direction,
      });
      // 失敗したときは設定を変えてやり直せるよう、折り線を残す
      if (get().errorMessage === null) set({ foldDraft: null });
    },

    foldByDrag: async (from, to, mode, grabFace = null) => {
      const s = get();
      if (!s.doc) return;
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
      const result = planGrabFold(
        foldLayers(s.frame3d, s.doc, s.faces),
        s.faces,
        from,
        to,
        mode,
        grabFace,
      );
      if (!result.ok) {
        set({ errorMessage: result.error });
        return;
      }
      // つかんだ紙は離した位置へ倒れてくる(=手前へ折る)。
      // 引きかけの折り線が残っていても、この操作で決着させる
      const upTo = foldInsertAt(s);
      set({
        currentStep: upTo === s.doc.sequence.length ? null : upTo + 1,
        foldDraft: null,
      });
      await get().applySequenceOp({
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
      set({
        activeTool: "technique",
        selection: EMPTY_SELECTION,
        foldDraft: null,
        techniqueDraft: {
          kind,
          flap: [],
          line: null,
          movingSide: "right",
          widthMm: DEFAULT_PLEAT_WIDTH_MM,
          polygon: [],
          center: null,
          twistDeg: DEFAULT_TWIST_DEG,
          docEpoch: s.docEpoch,
          stepCount: s.doc.sequence.length,
          upTo: foldInsertAt(s),
        },
        errorMessage: null,
      });
    },

    setTechniqueFlap: (faces) => {
      const draft = get().techniqueDraft;
      if (draft) set({ techniqueDraft: { ...draft, flap: faces } });
    },

    setTechniqueLine: (line) => {
      const draft = get().techniqueDraft;
      if (draft) set({ techniqueDraft: { ...draft, line } });
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

    updateTechniqueDraft: (patch) => {
      const draft = get().techniqueDraft;
      if (draft) set({ techniqueDraft: { ...draft, ...patch } });
    },

    setConstruct: (patch) =>
      set((s) => ({ construct: { ...s.construct, ...patch } })),

    cancelTechnique: () => {
      if (get().techniqueDraft) set({ techniqueDraft: null });
    },

    commitTechnique: async () => {
      const s = get();
      const draft = s.techniqueDraft;
      if (!draft || !s.doc) return;
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
      // 中割り折り・かぶせ折りには重なった層が要る(層の数は奇数でもよい。
      // 先端をどちら向きに回すかは紙のつながりから決まる)。
      // 多角形で指したねじり折りは層を選ばなくてよい(選ばなければ全ての層)
      if (draft.kind !== "Pleat" && !byPolygon && draft.flap.length < 2) {
        set({
          errorMessage:
            "先に立体表示で紙をクリックし、重なった層(フラップ)を選んでください",
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
      const reference =
        twistRef ??
        (draft.kind === "Pleat"
          ? offsetPoint(line, draft.movingSide, draft.widthMm / scale)
          : keepSidePoint(line, draft.movingSide));
      // 折った結果を見せる。末尾へ足したなら最新、途中へ挟んだなら挟んだ手順
      set({ currentStep: draft.upTo === s.doc.sequence.length ? null : draft.upTo + 1 });
      await get().applySequenceOp({
        type: "Technique",
        up_to: draft.upTo,
        kind: draft.kind,
        flap: draft.flap,
        line,
        reference_point: reference,
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
      // 左右同時を切っている間は相手を覚えない(切替が次のドラッグから必ず効く)
      set({
        pullHinge: hinge,
        pullMirrorHinge: get().pullMirror ? mirrorHinge : null,
        errorMessage: null,
      });
      // 今見えている形をそのまま角度指定として1回だけ送り、次からの計算の
      // 出発点(warm start)を今の形に合わせる。全ヒンジを指定するので形は動かない
      if (angles.size > 0) {
        void runPoseSolve(
          [...angles].map(([h, deg]) => ({ hinge: h, target_angle_deg: deg })),
          false,
          false, // 形は今のまま(層の重なり表示を保つ)
        );
      }
    },

    pullTo: (deg) => {
      const { pullHinge, pullMirrorHinge } = get();
      if (pullHinge === null) return;
      // 左右対称の相手も同じ角度で動かす(鶴の両羽が一緒に開く)。
      // 2本まとめて1回の追従計算にするので、送る回数は片側だけのときと同じ
      const drivers = new Map(get().drivers);
      drivers.set(pullHinge, deg);
      if (pullMirrorHinge !== null) drivers.set(pullMirrorHinge, deg);
      set({ drivers });
      pose.schedule();
    },

    endPull: () => {
      if (get().pullHinge !== null) set({ pullHinge: null, pullMirrorHinge: null });
    },

    setDriverAngle: (hinge, deg) => {
      // 画面の反応を優先し、指定はその場で反映してから計算を間引いて依頼する
      const drivers = new Map(get().drivers);
      drivers.set(hinge, deg);
      set({ drivers });
      pose.schedule();
    },

    clearDriver: (hinge) => {
      const drivers = new Map(get().drivers);
      if (!drivers.delete(hinge)) return;
      set({ drivers });
      // 指定を消しただけだと、この折り線は前回の計算結果(warm start)を
      // 引き継いで折れたまま残る。1回だけ0度(平ら)を明示して送り、
      // 次回以降は残りの指定だけで計算する(「全て平らに戻す」と同じ考え方)
      void runPoseSolve([
        ...driverList(drivers),
        { hinge, target_angle_deg: 0 },
      ]);
    },

    clearDrivers: () => {
      const hinges = get().hinges;
      set({ drivers: new Map() });
      void runPoseSolve(flatDrivers(hinges));
    },

    recordPoseStep: async () => {
      const s = get();
      const reason = poseRecordReason(s);
      if (reason !== null || !s.doc) {
        set({ errorMessage: reason });
        return;
      }
      const angles = currentAngles(s.hinges, s.drivers, s.poseAngles);
      // 記録した形をそのまま見せる(手順の再生結果が最新の表示になる)
      set({ currentStep: null, errorMessage: null });
      await get().applySequenceOp({
        type: "PushStep",
        step: buildPoseStep(s.doc, angles),
      });
      // 手順として残ったので、一時的な角度指定は役目を終える。
      // ここで平らに戻す計算は送らない(再生結果の立体表示を消さないため)
      if (get().errorMessage === null) set({ drivers: new Map() });
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

    openProposal: () =>
      set({
        proposalStep: "skeleton",
        proposalSkeleton: defaultSkeleton(),
        proposalCandidates: [],
        proposalSelected: null,
        proposalBusy: false,
        proposalError: null,
      }),

    closeProposal: () => set({ proposalStep: null, proposalBusy: false }),

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
      set({ proposalBusy: true, proposalError: null, proposalSeed: seed + 1 });
      // 提案の計算は作品の状態を読まない独立処理。直列化キューに載せると
      // 数百msの計算の間だけ編集が止まるので、ここは載せずに直接呼ぶ
      try {
        const list = await ipc.proposalGenerate(s.proposalSkeleton, paper, seed);
        set({
          proposalCandidates: list,
          proposalSelected: list.length > 0 ? 0 : null,
          proposalStep: list.length > 0 ? "candidates" : "skeleton",
          proposalError:
            list.length > 0 ? null : "候補を作れませんでした。骨格を変えてみてください",
          proposalBusy: false,
        });
      } catch (e) {
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

    setSplitRatio: (ratio) => {
      set({ splitRatio: clampSplitRatio(ratio) });
      persistPrefs();
    },

    moveStep: async (number, delta) => {
      const s = get();
      const steps = s.doc?.sequence ?? [];
      const from = number - 1;
      const to = from + delta;
      const step = steps[from];
      if (!step || to < 0 || to >= steps.length) return;
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
