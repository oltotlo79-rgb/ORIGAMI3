// @vitest-environment jsdom
// 復旧ダイアログの画面テスト(SYS-003 / UI-006):
// 残っていないときは何も出さない、あれば文言と2つのボタンを出し、押すと答えが渡る。

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { RecoveryDialog, fileName, formatSavedAt } from "./RecoveryDialog";
import { useAppStore } from "../store/appStore";

const INFO = {
  autosave_path: "C:\\作品\\鶴.ori3.autosave",
  document_path: "C:\\作品\\鶴.ori3",
  saved_at_ms: Date.UTC(2026, 7, 6, 3, 4),
};

afterEach(() => {
  cleanup();
  useAppStore.setState({ recovery: null });
});

describe("復旧ダイアログ", () => {
  it("前回が正常終了なら何も出さない", () => {
    const { container } = render(<RecoveryDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("残っていれば理由と選択肢を日本語で出す", () => {
    useAppStore.setState({ recovery: INFO });
    render(<RecoveryDialog />);
    expect(
      screen.getByText("前回の終了が正常に行われませんでした"),
    ).not.toBeNull();
    // 更新時刻と元の作品名を添える(どの内容が戻るのか分かるように)
    expect(screen.getByRole("dialog").textContent).toContain("復元しますか?");
    expect(screen.getByRole("dialog").textContent).toContain("鶴.ori3");
    expect(screen.getByRole("button", { name: "復元する" })).not.toBeNull();
    expect(screen.getByRole("button", { name: "破棄する" })).not.toBeNull();
  });

  it("「復元する」「破棄する」がそのまま答えとして渡る", () => {
    const resolveRecovery = vi.fn().mockResolvedValue(undefined);
    useAppStore.setState({ recovery: INFO, resolveRecovery });
    render(<RecoveryDialog />);

    fireEvent.click(screen.getByRole("button", { name: "復元する" }));
    expect(resolveRecovery).toHaveBeenLastCalledWith(true);
    fireEvent.click(screen.getByRole("button", { name: "破棄する" }));
    expect(resolveRecovery).toHaveBeenLastCalledWith(false);
  });

  it("時刻が分からなくても案内は出す", () => {
    expect(formatSavedAt(null)).toBe("");
    expect(formatSavedAt(INFO.saved_at_ms)).toContain("2026");
    expect(fileName("C:\\作品\\鶴.ori3")).toBe("鶴.ori3");
    expect(fileName("/home/作品/鶴.ori3")).toBe("鶴.ori3");
  });
});
