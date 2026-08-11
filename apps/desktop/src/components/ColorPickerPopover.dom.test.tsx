// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ColorPickerPopover } from "./ColorPickerPopover";

beforeEach(() => {
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    callback(0);
    return 1;
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function openPicker(onSelect = vi.fn(), value = "#ed1c24") {
  render(
    <div className="app" data-theme="modern">
      <button type="button">外側の操作</button>
      <ColorPickerPopover label="紙の表" value={value} onSelect={onSelect} />
    </div>,
  );
  const trigger = screen.getByRole("button", {
    name: "紙の表のその他の色を開く",
  });
  fireEvent.click(trigger);
  return { trigger, onSelect };
}

describe("アプリ内の色選択", () => {
  it("5テーマを持つapp内へ非モーダルで開き、外側の操作をふさがない", () => {
    const outside = vi.fn();
    render(
      <div className="app" data-theme="japanese">
        <button type="button" onClick={outside}>
          外側の操作
        </button>
        <ColorPickerPopover label="紙の表" value="#ed1c24" onSelect={vi.fn()} />
      </div>,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "紙の表のその他の色を開く" }),
    );
    const dialog = screen.getByRole("dialog", { name: "紙の表の色を選ぶ" });
    expect(dialog.getAttribute("aria-modal")).toBeNull();
    expect(dialog.closest('.app[data-theme="japanese"]')).not.toBeNull();
    expect(document.querySelector(".dialog-backdrop")).toBeNull();

    const outsideButton = screen.getByRole("button", { name: "外側の操作" });
    fireEvent.pointerDown(outsideButton);
    fireEvent.click(outsideButton);
    expect(outside).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog", { name: "紙の表の色を選ぶ" })).toBeNull();
  });

  it("彩度と明度を矢印で調整し、Enterで確定する", () => {
    const { onSelect } = openPicker();
    const plane = screen.getByRole("slider", { name: "紙の表の彩度と明度" });
    const before = plane.getAttribute("aria-valuetext");
    fireEvent.keyDown(plane, { key: "ArrowDown" });
    expect(plane.getAttribute("aria-valuetext")).not.toBe(before);
    fireEvent.keyDown(plane, { key: "Enter" });
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog", { name: "紙の表の色を選ぶ" })).toBeNull();
  });

  it("白からでも選んだ色相を保持してから彩度を付けられる", () => {
    openPicker(vi.fn(), "#ffffff");
    const hue = screen.getByRole("slider", { name: "紙の表の色相" });
    fireEvent.change(hue, { target: { value: "210" } });
    expect((hue as HTMLInputElement).value).toBe("210");

    const plane = screen.getByRole("slider", { name: "紙の表の彩度と明度" });
    fireEvent.keyDown(plane, { key: "ArrowRight" });
    expect(plane.getAttribute("aria-valuetext")).toContain("彩度1%");
    expect((hue as HTMLInputElement).value).toBe("210");
  });

  it("16進数を直接入力でき、不正値では確定しない", () => {
    const { onSelect } = openPicker();
    const hex = screen.getByLabelText("紙の表の16進数の色コード");
    fireEvent.change(hex, { target: { value: "#12GG00" } });
    expect(hex.getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByRole("alert")).not.toBeNull();
    fireEvent.keyDown(hex, { key: "Enter" });
    expect(onSelect).not.toHaveBeenCalled();

    fireEvent.change(hex, { target: { value: "#0080FF" } });
    fireEvent.keyDown(hex, { key: "Enter" });
    expect(onSelect).toHaveBeenCalledWith("#0080ff");
  });

  it("Escでは変更せず閉じて起点へ焦点を戻す", () => {
    const { trigger, onSelect } = openPicker();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "紙の表の色を選ぶ" })).toBeNull();
    expect(onSelect).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(trigger);
  });
});
