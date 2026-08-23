// IPCクライアント: Rust側コマンドの型付きラッパー(1関数5行以内)。
// 失敗時はErr(string)がPromiseのrejectになる。
// 折り図の書き出しは新しいコマンドを足さずExportKindを増やして対応する。

import { invoke } from "@tauri-apps/api/core";
import type {
  CreasePattern,
  DocumentView,
  Driver,
  EditOp,
  ExportKind,
  ExportOptions,
  FoldStep,
  Paper,
  ProposalCandidate,
  RecoveryInfo,
  ReplayResult,
  SeqOp,
  Skeleton,
  SoftSettings,
  SolveResult,
} from "../lib/types";

export function documentNew(paper: Paper): Promise<DocumentView> {
  return invoke("document_new", { paper });
}

export function documentOpen(path: string): Promise<DocumentView> {
  return invoke("document_open", { path });
}

/** pathがnullなら前回の保存先へ上書き保存 */
export function documentSave(path: string | null): Promise<void> {
  return invoke("document_save", { path });
}

export function editApply(op: EditOp): Promise<DocumentView> {
  return invoke("edit_apply", { op });
}

/** 画面での1回の入力から生じた複数の編集を、元に戻す1回で戻せるようにまとめて送る */
export function editApplyBatch(ops: EditOp[]): Promise<DocumentView> {
  return invoke("edit_apply_batch", { ops });
}

export function editUndo(): Promise<DocumentView> {
  return invoke("edit_undo");
}

export function editRedo(): Promise<DocumentView> {
  return invoke("edit_redo");
}

export function sequenceApply(op: SeqOp): Promise<DocumentView> {
  return invoke("sequence_apply", { op });
}

/** 手順の再生。upTo=0は初期状態(平ら)、tは0..=1の補間係数(1で完了)。
 * softを渡すとたわみの網も一緒に返る(省略時は従来どおり) */
export function sequenceReplay(
  upTo: number,
  t: number,
  soft?: SoftSettings | null,
): Promise<ReplayResult> {
  return invoke("sequence_replay", { upTo, t, soft: soft ?? null });
}

/** 折り角度の追従計算。前回解(warm start)はRust側のstoreが保持する。
 *
 * `hard` は厳密に固定する折り線(いま操作しているヒンジ)、`preferred` は
 * 「なるべく保ちたい目標」(以前に指定した折り線)。内部頂点のまわりでは
 * 折り角どうしに拘束があるので、指定済みを全部固定すると紙が切れて見える。
 * preferredを渡すと閉包を満たす形のうち目標にいちばん近いものが返る。
 * `warmSeed` は現在表示中の実角を初期値として渡すだけで、固定条件にはしない。
 * softを渡すとたわみの網も一緒に返る(省略時は従来どおり) */
export function poseSolve(
  hard: Driver[],
  preferred?: Driver[] | null,
  soft?: SoftSettings | null,
  warmSeed?: Driver[] | null,
  upTo = 0,
  t = 1,
): Promise<SolveResult> {
  return invoke("pose_solve", {
    hard,
    preferred: preferred?.length ? preferred : null,
    warmSeed: warmSeed?.length ? warmSeed : null,
    soft: soft ?? null,
    upTo,
    t,
  });
}

/** 前回の異常終了で残った自動保存を調べる。無ければnull */
export function recoveryCheck(): Promise<RecoveryInfo | null> {
  return invoke("recovery_check");
}

/** accept=trueなら自動保存の内容を復元、falseなら自動保存ファイルを捨てる */
export function recoveryRestore(accept: boolean): Promise<DocumentView | null> {
  return invoke("recovery_restore", { accept });
}

/** 骨格から展開図の候補を作る(最大4件)。seedを変えると別の配置が出る。
 * 候補ごとに折り方も探して付ける */
export function proposalGenerate(
  skeleton: Skeleton,
  paper: Paper,
  seed: number,
): Promise<ProposalCandidate[]> {
  return invoke("proposal_generate", {
    skeleton,
    paper,
    seed,
    withFoldPlan: true,
  });
}

/** 提案の計算がどこまで進んだか(commands.rs::ProposalProgress) */
export interface ProposalProgress {
  /** 計算が終わった候補の数 */
  done: number;
  /** 計算する候補の数。まだ始まっていなければ 0 */
  total: number;
}

/** 提案の計算の進み具合を読む。計算中でもすぐ返る */
export function proposalProgress(): Promise<ProposalProgress> {
  return invoke("proposal_progress");
}

/** 提案の展開図と折り手順をまとめて入れる。元に戻す1回で入れる前へ戻る */
export function proposalApply(
  cp: CreasePattern,
  steps: FoldStep[],
): Promise<DocumentView> {
  return invoke("proposal_apply", { cp, steps });
}

/** 展開図を画像ファイルとして保存する(EXP-001 / EXP-002) */
export function documentExport(
  kind: ExportKind,
  path: string,
  options: ExportOptions,
): Promise<void> {
  return invoke("document_export", { kind, path, options });
}
