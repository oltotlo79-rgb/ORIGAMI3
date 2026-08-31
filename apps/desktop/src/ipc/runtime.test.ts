// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Ori3WebBridge } from "./runtime";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => tauri);

beforeEach(() => {
  vi.resetModules();
  tauri.invoke.mockReset();
  tauri.isTauri.mockReset();
  delete window.__ori3Web;
});

describe("実行環境の切り替え", () => {
  it("公開するバックエンドコマンドを18件に固定する", async () => {
    tauri.isTauri.mockReturnValue(false);
    const { BACKEND_COMMAND_NAMES } = await import("./runtime");

    expect(BACKEND_COMMAND_NAMES).toEqual([
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
    ]);
    expect(new Set(BACKEND_COMMAND_NAMES).size).toBe(18);
  });

  it("Tauriでは従来のinvokeへ名前と引数をそのまま渡す", async () => {
    const result = { ok: true };
    tauri.isTauri.mockReturnValue(true);
    tauri.invoke.mockResolvedValue(result);
    const webInvoke = vi.fn();
    window.__ori3Web = {
      invoke: webInvoke as Ori3WebBridge["invoke"],
    };

    const { callBackend, detectRuntime } = await import("./runtime");

    await expect(
      callBackend("sequence_apply", { op: { type: "test" } }),
    ).resolves.toBe(result);
    await callBackend("recovery_check");

    expect(detectRuntime()).toBe("tauri");
    expect(tauri.isTauri).toHaveBeenCalledTimes(1);
    expect(tauri.invoke).toHaveBeenNthCalledWith(1, "sequence_apply", {
      op: { type: "test" },
    });
    expect(tauri.invoke).toHaveBeenNthCalledWith(2, "recovery_check");
    expect(webInvoke).not.toHaveBeenCalled();
  });

  it("Webではモジュール初期化時に選んだ橋へ名前と引数を渡す", async () => {
    const result = { ok: true };
    const webInvoke = vi.fn().mockResolvedValue(result);
    tauri.isTauri.mockReturnValue(false);
    window.__ori3Web = {
      invoke: webInvoke as Ori3WebBridge["invoke"],
    };

    const { callBackend, detectRuntime } = await import("./runtime");
    tauri.isTauri.mockReturnValue(true);

    await expect(
      callBackend("document_export", { kind: "Svg" }),
    ).resolves.toBe(result);
    await callBackend("recovery_check");

    expect(detectRuntime()).toBe("web");
    expect(tauri.isTauri).toHaveBeenCalledTimes(1);
    expect(webInvoke).toHaveBeenNthCalledWith(1, "document_export", {
      kind: "Svg",
    });
    expect(webInvoke).toHaveBeenNthCalledWith(
      2,
      "recovery_check",
      undefined,
    );
    expect(tauri.invoke).not.toHaveBeenCalled();
  });

  it("Webの計算機能が未準備なら利用者向けの日本語で理由を示す", async () => {
    tauri.isTauri.mockReturnValue(false);
    const { callBackend } = await import("./runtime");

    await expect(callBackend("recovery_check")).rejects.toBe(
      "Web版の計算機能を準備できていないため、この操作はまだ利用できません。ページを読み直して、もう一度お試しください。",
    );
    expect(tauri.invoke).not.toHaveBeenCalled();
  });
});
