// IPCクライアント: Rust側コマンドの型付きラッパー(1関数5行以内)。
// 失敗時はErr(string)がPromiseのrejectになる。
// 折り図の書き出しは新しいコマンドを足さずExportKindを増やして対応する。

import { invoke } from "@tauri-apps/api/core";
import type {
  CreasePattern,
  DocumentExportKind,
  DocumentView,
  Driver,
  EditOp,
  ExportOptions,
  FoldIssue,
  FoldAllPreviewOutcome,
  FoldStep,
  Paper,
  ProposalControl,
  ProposalJobId,
  ProposalJobResult,
  ProposalProgressSnapshot,
  RecoveryChoices,
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
export type PoseSolveMode = "Follow" | "Canonical";

export function poseSolve(
  hard: Driver[],
  preferred?: Driver[] | null,
  soft?: SoftSettings | null,
  warmSeed?: Driver[] | null,
  upTo = 0,
  t = 1,
  mode: PoseSolveMode = "Follow",
): Promise<SolveResult> {
  return invoke("pose_solve", {
    request: {
      hard,
      preferred: preferred?.length ? preferred : null,
      warmSeed: warmSeed?.length ? warmSeed : null,
      soft: soft ?? null,
      upTo,
      t,
      mode,
    },
  });
}

/**
 * 全ての山折り・谷折りを同じ割合で動かした一時的な形を求める。
 * `warmSeed` は直前応答の `next_warm_seed` だけを渡し、作品へ保存しない。
 */
export function foldAllPreview(
  percent: number,
  warmSeed?: Driver[] | null,
): Promise<FoldAllPreviewOutcome> {
  return invoke("fold_all_preview", {
    percent,
    warmSeed: warmSeed?.length ? warmSeed : null,
  });
}

/** 前回の異常終了で残った、利用者が選べる作業をすべて調べる。 */
export function recoveryCheck(): Promise<RecoveryChoices | RecoveryInfo | null> {
  return invoke("recovery_check");
}

/** accept=trueなら選んだ内容を復元、falseなら選んだ内容だけを捨てる。 */
export function recoveryRestore(
  accept: boolean,
  candidateId?: number | null,
): Promise<DocumentView | null> {
  if (candidateId === undefined || candidateId === null) {
    return invoke("recovery_restore", { accept });
  }
  return invoke("recovery_restore", { accept, candidateId });
}

/** 骨格から展開図の候補を作る(最大4件)。seedを変えると別の配置が出る。
 * 候補ごとに折り方も探して付ける */
export function proposalGenerate(
  skeleton: Skeleton,
  paper: Paper,
  seed: number,
  jobId: ProposalJobId,
): Promise<ProposalJobResult> {
  return invoke("proposal_generate", {
    jobId,
    skeleton,
    paper,
    seed,
    withFoldPlan: true,
  });
}

/** 指定した提案計算の進み具合を読む。回収済みならnull */
export function proposalProgress(
  jobId: ProposalJobId,
): Promise<ProposalProgressSnapshot | null> {
  return invoke("proposal_progress", { jobId });
}

/** 同種の実行制御を1入口へまとめる。現在は取消しだけ。 */
export function proposalControl(
  operation: ProposalControl,
): Promise<ProposalProgressSnapshot> {
  return invoke("proposal_control", { operation });
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
  kind: DocumentExportKind,
  path: string,
  options: ExportOptions,
): Promise<FoldIssue[]> {
  return invoke("document_export", { kind, path, options });
}
