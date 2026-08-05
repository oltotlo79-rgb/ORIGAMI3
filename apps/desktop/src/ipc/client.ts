// IPCクライアント: 実装済み9コマンドの型付きラッパー(1関数5行以内)。
// 失敗時はErr(string)がPromiseのrejectになる。
// 未実装コマンド(document_export等)は実装タスクで追加する。

import { invoke } from "@tauri-apps/api/core";
import type {
  DocumentView,
  Driver,
  EditOp,
  Paper,
  ReplayResult,
  SeqOp,
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
