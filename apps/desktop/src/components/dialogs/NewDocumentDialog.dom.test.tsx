// @vitest-environment jsdom
// 新規作成ダイアログ(PAP-001)の画面テスト:
// 閉じていれば何も出さない、形と大きさを選べる、正方形ならたては触らせない。

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
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
    const { container } = render(<NewDocumentDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("紙の形と大きさを日本語で選べる", () => {
    useAppStore.setState({ newDialogOpen: true });
    render(<NewDocumentDialog />);
    expect(screen.getByRole("dialog").textContent).toContain("新しい紙を用意する");
    expect(screen.getByLabelText("よこ(mm)")).toHaveProperty("value", "150");
    // 正方形の間は「たて」は触らせず、なぜ触れないかを添える
    const height = screen.getByLabelText("たて(mm)") as HTMLInputElement;
    expect(height.disabled).toBe(true);
    expect(height.title).toContain("よこと同じ");
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
});
