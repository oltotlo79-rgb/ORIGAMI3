import type {
  BackendCommandName,
  BackendInvokeArgs,
} from "../../../desktop/src/ipc/runtime";

export const INTERNAL_CORE_COMMAND_NAMES = [
  "__web_document_open_source",
  "__web_document_save_prepare",
  "__web_document_save_abort",
  "__web_document_export_prepare",
  "__web_recovery_set_choices",
  "__web_recovery_restore_source",
  "__web_recovery_snapshot",
] as const;

export type InternalCoreCommandName =
  (typeof INTERNAL_CORE_COMMAND_NAMES)[number];
export type CoreCommandName = BackendCommandName | InternalCoreCommandName;

export interface CoreWorkerRequest {
  type: "invoke";
  id: number;
  command: CoreCommandName;
  args?: BackendInvokeArgs;
}

export type CoreWorkerResponse =
  | {
      /** 指定された初期化が完了し、RPCの受付準備ができたことを示す。 */
      type: "ready";
    }
  | {
      type: "initialization-error";
      error: string;
    }
  | {
      /** 通信契約を継続できない異常。受信側はWorkerを終端する。 */
      type: "fatal-error";
      error: string;
    }
  | {
      type: "result";
      id: number;
      ok: true;
      value: unknown;
    }
  | {
      type: "result";
      id: number;
      ok: false;
      error: string;
    };
