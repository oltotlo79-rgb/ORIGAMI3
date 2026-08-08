// @vitest-environment jsdom
// アプリ全体の元に戻す/やり直しショートカットの画面テスト。

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { HistoryShortcuts } from "./HistoryShortcuts";
import { useAppStore } from "../store/appStore";

const originalUndo = useAppStore.getState().undo;
const originalRedo = useAppStore.getState().redo;

function installHistorySpies() {
  const undo = vi.fn(async () => undefined);
  const redo = vi.fn(async () => undefined);
  useAppStore.setState({ undo, redo });
  return { undo, redo };
}

function dispatchKey(target: EventTarget, init: KeyboardEventInit) {
  const event = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  target.dispatchEvent(event);
  return event;
}

afterEach(() => {
  cleanup();
  useAppStore.setState({ undo: originalUndo, redo: originalRedo });
  vi.restoreAllMocks();
});

describe("元に戻す/やり直しのキーボードショートカット", () => {
  it("Ctrl+Zで元に戻し、ブラウザ既定の操作を止める", () => {
    const { undo, redo } = installHistorySpies();
    render(<HistoryShortcuts />);

    const event = dispatchKey(window, { key: "z", ctrlKey: true });

    expect(undo).toHaveBeenCalledTimes(1);
    expect(redo).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(true);
  });

  it("Ctrl+YとCtrl+Shift+Zでやり直す", () => {
    const { undo, redo } = installHistorySpies();
    render(<HistoryShortcuts />);

    const ctrlY = dispatchKey(window, { key: "y", ctrlKey: true });
    const ctrlShiftZ = dispatchKey(window, {
      key: "Z",
      ctrlKey: true,
      shiftKey: true,
    });

    expect(undo).not.toHaveBeenCalled();
    expect(redo).toHaveBeenCalledTimes(2);
    expect(ctrlY.defaultPrevented).toBe(true);
    expect(ctrlShiftZ.defaultPrevented).toBe(true);
  });

  it("input・textarea・contenteditableで文字編集中は発火しない", () => {
    const { undo, redo } = installHistorySpies();
    const { getByTestId } = render(
      <>
        <HistoryShortcuts />
        <input data-testid="input" />
        <textarea data-testid="textarea" />
        <div data-testid="editable" contentEditable>
          <span data-testid="editable-child">編集中</span>
        </div>
      </>,
    );

    for (const id of ["input", "textarea", "editable-child"]) {
      const target = getByTestId(id);
      (target instanceof HTMLElement ? target : null)?.focus();
      const event = dispatchKey(target, { key: "z", ctrlKey: true });
      expect(event.defaultPrevented).toBe(false);
    }

    expect(undo).not.toHaveBeenCalled();
    expect(redo).not.toHaveBeenCalled();
  });

  it("入力欄にフォーカスがあればwindowへ届いたキーも横取りしない", () => {
    const { undo, redo } = installHistorySpies();
    const { getByRole } = render(
      <>
        <HistoryShortcuts />
        <textarea aria-label="文章" />
      </>,
    );
    getByRole("textbox", { name: "文章" }).focus();

    const event = dispatchKey(window, { key: "z", ctrlKey: true });

    expect(undo).not.toHaveBeenCalled();
    expect(redo).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });
});
