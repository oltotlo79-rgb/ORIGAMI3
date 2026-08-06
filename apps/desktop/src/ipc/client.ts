// IPCクライアント: 実装済み13コマンドの型付きラッパー(1関数5行以内)。
// 失敗時はErr(string)がPromiseのrejectになる。
// コマンドは13個で打ち止め。折り図の書き出しはExportKindを増やして対応する。

import { invoke } from "@tauri-apps/api/core";
import type {
  DocumentView,
  Driver,
  EditOp,
  ExportKind,
  ExportOptions,
  Paper,
  ProposalCandidate,
  RecoveryInfo,
  ReplayResult,
  SeqOp,
  Skeleton,
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

export function editUndo(): Promise<DocumentView> {
  return invoke("edit_undo");
}

export function editRedo(): Promise<DocumentView> {
  return invoke("edit_redo");
}

export function sequenceApply(op: SeqOp): Promise<DocumentView> {
  return invoke("sequence_apply", { op });
}

/** 手順の再生。upTo=0は初期状態(平ら)、tは0..=1の補間係数(1で完了) */
export function sequenceReplay(upTo: number, t: number): Promise<ReplayResult> {
  return invoke("sequence_replay", { upTo, t });
}

/** 折り角度の追従計算。前回解(warm start)はRust側のstoreが保持する */
export function poseSolve(drivers: Driver[]): Promise<SolveResult> {
  return invoke("pose_solve", { drivers });
}

/** 前回の異常終了で残った自動保存を調べる。無ければnull */
export function recoveryCheck(): Promise<RecoveryInfo | null> {
  return invoke("recovery_check");
}

/** accept=trueなら自動保存の内容を復元、falseなら自動保存ファイルを捨てる */
export function recoveryRestore(accept: boolean): Promise<DocumentView | null> {
  return invoke("recovery_restore", { accept });
}

/** 骨格から展開図の候補を作る(最大4件)。seedを変えると別の配置が出る */
export function proposalGenerate(
  skeleton: Skeleton,
  paper: Paper,
  seed: number,
): Promise<ProposalCandidate[]> {
  return invoke("proposal_generate", { skeleton, paper, seed });
}

/** 展開図を画像ファイルとして保存する(EXP-001 / EXP-002) */
export function documentExport(
  kind: ExportKind,
  path: string,
  options: ExportOptions,
): Promise<void> {
  return invoke("document_export", { kind, path, options });
}
