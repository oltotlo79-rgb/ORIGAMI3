import {
  invoke as tauriInvoke,
  isTauri,
  type InvokeArgs,
} from "@tauri-apps/api/core";

export type Runtime = "tauri" | "web";

export const BACKEND_COMMAND_NAMES = [
  "document_new",
  "document_open",
  "document_save",
  "edit_apply",
  "edit_apply_batch",
  "edit_undo",
  "edit_redo",
  "sequence_apply",
  "sequence_replay",
  "pose_solve",
  "fold_all_preview",
  "recovery_check",
  "recovery_restore",
  "proposal_generate",
  "proposal_progress",
  "proposal_control",
  "proposal_apply",
  "document_export",
] as const;

export type BackendCommandName = (typeof BACKEND_COMMAND_NAMES)[number];
export type BackendInvokeArgs = InvokeArgs;

export interface Ori3WebBridge {
  invoke<T>(name: BackendCommandName, args?: InvokeArgs): Promise<T>;
}

declare global {
  interface Window {
    __ori3Web?: Ori3WebBridge;
  }
}

const ACTIVE_RUNTIME: Runtime = isTauri() ? "tauri" : "web";

/** モジュール初期化時に1回だけ決めた実行環境を返す。 */
export function detectRuntime(): Runtime {
  return ACTIVE_RUNTIME;
}

const UNAVAILABLE_WEB_BRIDGE =
  "Web版の計算機能を準備できていないため、この操作はまだ利用できません。ページを読み直して、もう一度お試しください。";

/** 画面から計算機能を呼ぶ唯一の入口。 */
export function callBackend<T>(
  name: BackendCommandName,
  args?: InvokeArgs,
): Promise<T> {
  if (ACTIVE_RUNTIME === "tauri") {
    return args === undefined
      ? tauriInvoke<T>(name)
      : tauriInvoke<T>(name, args);
  }

  const bridge =
    typeof window === "undefined" ? undefined : window.__ori3Web;
  if (typeof bridge?.invoke !== "function") {
    return Promise.reject(UNAVAILABLE_WEB_BRIDGE);
  }
  return bridge.invoke<T>(name, args);
}
