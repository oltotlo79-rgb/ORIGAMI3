import type { BackendCommandName } from "../../../desktop/src/ipc/runtime";

export type WebCommandRoute = "core" | "proposal" | "browser" | "mixed";

/** 18件すべてのWeb実行経路。コマンド追加時の分類漏れは型エラーにする。 */
export const WEB_COMMAND_ROUTES = {
  document_new: "core",
  document_open: "mixed",
  document_save: "mixed",
  edit_apply: "core",
  edit_apply_batch: "core",
  edit_undo: "core",
  edit_redo: "core",
  sequence_apply: "core",
  sequence_replay: "core",
  pose_solve: "core",
  fold_all_preview: "core",
  recovery_check: "browser",
  recovery_restore: "mixed",
  proposal_generate: "proposal",
  proposal_progress: "proposal",
  proposal_control: "proposal",
  proposal_apply: "core",
  document_export: "mixed",
} as const satisfies Record<BackendCommandName, WebCommandRoute>;
