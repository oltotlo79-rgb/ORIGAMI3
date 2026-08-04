// IPCクライアント: 実装済み7コマンドの型付きラッパー(1関数5行以内)。
// 失敗時はErr(string)がPromiseのrejectになる。
// 未実装コマンド(document_export等)は実装タスクで追加する。

import { invoke } from "@tauri-apps/api/core";
import type { DocumentView, EditOp, Paper, SeqOp } from "../lib/types";

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
