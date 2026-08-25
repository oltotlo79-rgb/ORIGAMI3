// @vitest-environment jsdom
// 復旧ダイアログの画面テスト(SYS-003 / UI-006):
// 残っていないときは何も出さない、あれば文言と2つのボタンを出し、押すと答えが渡る。

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { RecoveryDialog, fileName, formatSavedAt } from "./RecoveryDialog";
import { focusableElements } from "./dialogs/ModalDialog";
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
    render(<RecoveryDialog />);
    expect(screen.queryByRole("dialog")).toBeNull();
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

  it("復元を最初に選び、2ボタンだけを循環し、Escapeでは判断しない", () => {
    const resolveRecovery = vi.fn().mockResolvedValue(undefined);
    useAppStore.setState({ recovery: INFO, resolveRecovery });
    const { container } = render(<RecoveryDialog />);

    const restore = screen.getByRole("button", { name: "復元する" });
    const discard = screen.getByRole("button", { name: "破棄する" });
    const dialog = screen.getByRole("dialog");
    expect(document.activeElement).toBe(restore);
    expect(focusableElements(dialog)).toEqual([restore, discard]);
    expect(container.hasAttribute("inert")).toBe(true);

    for (let attempt = 0; attempt < 100; attempt += 1) {
      discard.focus();
      fireEvent.keyDown(discard, { key: "Tab" });
      expect(document.activeElement).toBe(restore);

      restore.focus();
      fireEvent.keyDown(restore, { key: "Tab", shiftKey: true });
      expect(document.activeElement).toBe(discard);

      fireEvent.keyDown(discard, { key: "Escape" });
    }

    expect(resolveRecovery).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).not.toBeNull();
  });

  it.each([
    ["復元する", true],
    ["破棄する", false],
  ] as const)("キーボードで「%s」を決めた後は元の操作へ戻る", async (label, answer) => {
    const resolveRecovery = vi.fn(async () => {
      useAppStore.setState({ recovery: null });
    });
    render(
      <>
        <button type="button">作業へ戻る</button>
        <RecoveryDialog />
      </>,
    );
    const returnTarget = screen.getByRole("button", { name: "作業へ戻る" });
    returnTarget.focus();
    act(() => useAppStore.setState({ recovery: INFO, resolveRecovery }));

    const restore = screen.getByRole("button", { name: "復元する" });
    const action = screen.getByRole("button", { name: label });
    expect(document.activeElement).toBe(restore);
    if (action !== restore) {
      const tab = new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      });
      fireEvent(restore, tab);
      if (!tab.defaultPrevented) action.focus();
    }
    expect(document.activeElement).toBe(action);

    const pointerDown = vi.fn();
    const mouseDown = vi.fn();
    const layer = screen.getByRole("dialog").parentElement;
    layer?.addEventListener("pointerdown", pointerDown);
    layer?.addEventListener("mousedown", mouseDown);
    expect(action).toBeInstanceOf(HTMLButtonElement);
    const enter = new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    });
    fireEvent(action, enter);
    expect(enter.defaultPrevented).toBe(false);
    // jsdomはbuttonのEnter既定動作を作らないため、ブラウザが発生させるclickだけを代行する。
    if (!enter.defaultPrevented) act(() => (action as HTMLButtonElement).click());
    fireEvent.keyUp(action, { key: "Enter" });

    expect(resolveRecovery).toHaveBeenCalledWith(answer);
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() =>
      expect(document.activeElement).toBe(returnTarget),
    );
    expect(pointerDown).toHaveBeenCalledTimes(0);
    expect(mouseDown).toHaveBeenCalledTimes(0);
  });
});
