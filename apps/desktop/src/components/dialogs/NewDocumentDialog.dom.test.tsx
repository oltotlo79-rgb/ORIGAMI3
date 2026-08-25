// @vitest-environment jsdom
// 新規作成ダイアログ(PAP-001)の画面テスト:
// 閉じていれば何も出さない、形と大きさを選べる、正方形ならたては触らせない。

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NewDocumentDialog } from "./NewDocumentDialog";
import { DEFAULT_NEW_PAPER, useAppStore } from "../../store/appStore";

afterEach(() => {
  cleanup();
  useAppStore.setState({
    newDialogOpen: false,
    newPaperDraft: DEFAULT_NEW_PAPER,
  });
});

describe("新規作成ダイアログ", () => {
  it("開いていなければ何も出さない", () => {
    render(<NewDocumentDialog />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("紙の形と大きさを日本語で選べる", () => {
    useAppStore.setState({ newDialogOpen: true });
    render(<NewDocumentDialog />);
    expect(screen.getByRole("dialog").textContent).toContain("新しい紙を用意する");
    expect(screen.getByLabelText("よこ(mm)")).toHaveProperty("value", "150");
    // 正方形の間は「たて」は触らせず、なぜ触れないかを添える
    const height = screen.getByLabelText("たて(mm)") as HTMLInputElement;
    expect(height.disabled).toBe(true);
    expect(height.getAttribute("data-tooltip")).toBe(
      "正方形なので、横と同じ長さになります",
    );
    expect(height.hasAttribute("title")).toBe(false);
    expect(
      screen.getByRole("button", { name: "この紙で作りはじめる" }),
    ).not.toBeNull();
  });

  it("長方形を選ぶとたても指定できる", () => {
    useAppStore.setState({ newDialogOpen: true });
    render(<NewDocumentDialog />);
    fireEvent.click(screen.getByLabelText("長方形(たて・よこを別に決める)"));
    const height = screen.getByLabelText("たて(mm)") as HTMLInputElement;
    expect(height.disabled).toBe(false);
    fireEvent.change(height, { target: { value: "100" } });
    expect(useAppStore.getState().newPaperDraft.heightMm).toBe(100);
  });

  it("横の上下ボタンは1mmずつ変わり、正方形では縦の上下ボタンも無効になる", () => {
    useAppStore.setState({ newDialogOpen: true });
    render(<NewDocumentDialog />);

    const width = screen.getByLabelText("紙の横の長さ（mm）") as HTMLInputElement;
    fireEvent.click(screen.getByRole("button", { name: "紙の横の長さ（mm）を増やす" }));
    expect(width.value).toBe("151");
    expect(useAppStore.getState().newPaperDraft.widthMm).toBe(151);

    fireEvent.click(screen.getByRole("button", { name: "紙の横の長さ（mm）を減らす" }));
    expect(width.value).toBe("150");
    expect(useAppStore.getState().newPaperDraft.widthMm).toBe(150);

    expect(
      (screen.getByRole("button", { name: "紙の縦の長さ（mm）を増やす" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "紙の縦の長さ（mm）を減らす" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("よく使う大きさを押すとその大きさが入る", () => {
    useAppStore.setState({ newDialogOpen: true });
    render(<NewDocumentDialog />);
    fireEvent.click(screen.getByRole("button", { name: "A4の紙" }));
    expect(useAppStore.getState().newPaperDraft).toEqual({
      widthMm: 297,
      heightMm: 210,
      square: false,
    });
  });

  it("「この紙で作りはじめる」で作成、「やめる」で閉じる", () => {
    const confirmNewDocument = vi.fn().mockResolvedValue(undefined);
    useAppStore.setState({ newDialogOpen: true, confirmNewDocument });
    render(<NewDocumentDialog />);
    fireEvent.click(screen.getByRole("button", { name: "この紙で作りはじめる" }));
    expect(confirmNewDocument).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "やめる" }));
    expect(useAppStore.getState().newDialogOpen).toBe(false);
  });

  it("0以下の大きさでは作りはじめられず、理由を出す", () => {
    useAppStore.setState({ newDialogOpen: true });
    render(<NewDocumentDialog />);
    fireEvent.change(screen.getByLabelText("よこ(mm)"), {
      target: { value: "0" },
    });
    const ok = screen.getByRole("button", {
      name: "この紙で作りはじめる",
    }) as HTMLButtonElement;
    expect(ok.disabled).toBe(true);
    expect(screen.getByText("大きさは0より大きいmmで入れてください")).not.toBeNull();
  });

  it("キーボードだけで開いて入力し、Tab循環とEscape後の入口復帰を100回確かめる", async () => {
    const { container } = render(
      <>
        <button
          type="button"
          onClick={() => useAppStore.getState().openNewDialog()}
        >
          新しい紙を開く
        </button>
        <NewDocumentDialog />
      </>,
    );
    const pointerDown = vi.fn();
    const mouseDown = vi.fn();
    document.addEventListener("pointerdown", pointerDown);
    document.addEventListener("mousedown", mouseDown);
    const trigger = screen.getByRole("button", { name: "新しい紙を開く" });
    expect(trigger).toBeInstanceOf(HTMLButtonElement);
    expect((trigger as HTMLButtonElement).disabled).toBe(false);

    try {
      for (let cycle = 0; cycle < 100; cycle += 1) {
        trigger.focus();
        const enter = new KeyboardEvent("keydown", {
          key: "Enter",
          bubbles: true,
          cancelable: true,
        });
        fireEvent(trigger, enter);
        expect(enter.defaultPrevented).toBe(false);
        // jsdomはbuttonのEnter既定動作を作らないため、ブラウザが発生させるclickだけを代行する。
        if (!enter.defaultPrevented) act(() => (trigger as HTMLButtonElement).click());
        fireEvent.keyUp(trigger, { key: "Enter" });

        const first = screen.getByLabelText("正方形(たて・よこが同じ)");
        const last = screen.getByRole("button", { name: "やめる" });
        expect(document.activeElement).toBe(first);
        expect(container.hasAttribute("inert")).toBe(true);

        if (cycle === 0) {
          const width = screen.getByLabelText("紙の横の長さ（mm）");
          (width as HTMLInputElement).focus();
          fireEvent.input(width, { target: { value: "160" } });
          expect(useAppStore.getState().newPaperDraft.widthMm).toBe(160);

          for (let attempt = 0; attempt < 100; attempt += 1) {
            last.focus();
            fireEvent.keyDown(last, { key: "Tab" });
            expect(document.activeElement).toBe(first);

            first.focus();
            fireEvent.keyDown(first, { key: "Tab", shiftKey: true });
            expect(document.activeElement).toBe(last);
          }
        }

        fireEvent.keyDown(first, { key: "Escape" });
        expect(screen.queryByRole("dialog")).toBeNull();
        await Promise.resolve();
        expect(document.activeElement).toBe(trigger);
        expect(container.hasAttribute("inert")).toBe(false);
      }

      expect(pointerDown).toHaveBeenCalledTimes(0);
      expect(mouseDown).toHaveBeenCalledTimes(0);
    } finally {
      document.removeEventListener("pointerdown", pointerDown);
      document.removeEventListener("mousedown", mouseDown);
    }
  }, 15_000);

  it("長方形で開いたときは選択中の形を最初に選ぶ", () => {
    useAppStore.setState({
      newDialogOpen: true,
      newPaperDraft: { widthMm: 297, heightMm: 210, square: false },
    });
    render(<NewDocumentDialog />);
    expect(document.activeElement).toBe(
      screen.getByLabelText("長方形(たて・よこを別に決める)"),
    );
  });
});
