import { describe, expect, it } from "vitest";
import { BACKEND_COMMAND_NAMES } from "../../../desktop/src/ipc/runtime";
import { WEB_COMMAND_ROUTES } from "./routes";

describe("Web版の18コマンド経路", () => {
  it("閉じたコマンド一覧とrouteのキーが全件一致する", () => {
    expect(Object.keys(WEB_COMMAND_ROUTES)).toEqual(BACKEND_COMMAND_NAMES);
    expect(WEB_COMMAND_ROUTES).toEqual({
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
    });
  });

  it("core 10件・proposal 3件・browser 1件・mixed 4件に固定する", () => {
    const counts = Object.values(WEB_COMMAND_ROUTES).reduce(
      (result, route) => ({
        ...result,
        [route]: result[route] + 1,
      }),
      { core: 0, proposal: 0, browser: 0, mixed: 0 },
    );

    expect(counts).toEqual({ core: 10, proposal: 3, browser: 1, mixed: 4 });
  });
});
