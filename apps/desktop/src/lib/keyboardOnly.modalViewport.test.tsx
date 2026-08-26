// @vitest-environment jsdom

import { useRef } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import {
  ModalDialog,
  type FocusTarget,
} from "../components/dialogs/ModalDialog";

function rect(top: number, height: number, left = 40, width = 160): DOMRect {
  return {
    x: left,
    y: top,
    top,
    right: left + width,
    bottom: top + height,
    left,
    width,
    height,
    toJSON: () => ({}),
  };
}

function ScrollDialog() {
  const first = useRef<HTMLButtonElement>(null);
  return (
    <ModalDialog
      labelledBy="scroll-dialog-title"
      initialFocusRef={first}
      escapeAction={{ kind: "stay" }}
    >
      <h2 id="scroll-dialog-title">長い画面</h2>
      <div data-testid="scroll-area" style={{ overflowY: "auto" }}>
        <button ref={first} type="button">先頭</button>
        <button type="button">末尾</button>
      </div>
    </ModalDialog>
  );
}

afterEach(() => cleanup());

describe("キーボードfocusを画面内へ送る共通契約", () => {
  it("通常のTabと両方向の循環で、最寄りの縦送り領域へfocus輪4pxごと表示する", () => {
    render(<ScrollDialog />);
    const scroller = screen.getByTestId("scroll-area");
    const first = screen.getByRole("button", { name: "先頭" });
    const last = screen.getByRole("button", { name: "末尾" });
    let scrollTop = 0;

    Object.defineProperties(scroller, {
      clientHeight: { configurable: true, value: 100 },
      clientTop: { configurable: true, value: 0 },
      scrollHeight: { configurable: true, value: 500 },
      scrollTop: {
        configurable: true,
        get: () => scrollTop,
        set: (value: number) => { scrollTop = value; },
      },
    });
    scroller.getBoundingClientRect = () => rect(20, 100);
    first.getBoundingClientRect = () => rect(20 - scrollTop, 32);
    last.getBoundingClientRect = () => rect(400 - scrollTop, 32);

    last.focus();
    expect(document.activeElement).toBe(last);
    expect(scrollTop).toBe(316);
    expect(last.getBoundingClientRect()).toMatchObject({ top: 84, bottom: 116 });

    const forward = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      cancelable: true,
    });
    fireEvent(last, forward);
    expect(forward.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(first);
    expect(scrollTop).toBe(0);
    expect(first.getBoundingClientRect()).toMatchObject({ top: 20, bottom: 52 });

    const backward = new KeyboardEvent("keydown", {
      key: "Tab",
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    fireEvent(first, backward);
    expect(backward.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(last);
    expect(scrollTop).toBe(316);
  });

  it("SVGの丸い操作は対象半径ぶんの余白を取り、横位置は変えない", () => {
    function SvgDialog() {
      const target = useRef<SVGCircleElement>(null);
      return (
        <ModalDialog
          labelledBy="svg-dialog-title"
          initialFocusRef={target as React.RefObject<FocusTarget | null>}
          escapeAction={{ kind: "stay" }}
        >
          <h2 id="svg-dialog-title">紙の位置</h2>
          <div data-testid="svg-scroll" style={{ overflowY: "auto" }}>
            <svg aria-label="紙">
              <circle
                ref={target}
                role="button"
                tabIndex={0}
                aria-label="12番目"
                data-paper-position-handle="12"
              />
            </svg>
          </div>
        </ModalDialog>
      );
    }

    render(<SvgDialog />);
    const scroller = screen.getByTestId("svg-scroll");
    const target = screen.getByRole("button", { name: "12番目" });
    let scrollTop = 0;
    let scrollLeft = 17;
    Object.defineProperties(scroller, {
      clientHeight: { configurable: true, value: 100 },
      clientTop: { configurable: true, value: 0 },
      scrollHeight: { configurable: true, value: 500 },
      scrollTop: {
        configurable: true,
        get: () => scrollTop,
        set: (value: number) => { scrollTop = value; },
      },
      scrollLeft: {
        configurable: true,
        get: () => scrollLeft,
        set: (value: number) => { scrollLeft = value; },
      },
    });
    scroller.getBoundingClientRect = () => rect(20, 100);
    target.getBoundingClientRect = () => rect(405 - scrollTop, 20);

    screen.getByRole("dialog", { name: "紙の位置" }).focus();
    (target as FocusTarget).focus();
    expect(scrollTop).toBe(315);
    expect(target.getBoundingClientRect()).toMatchObject({ top: 90, bottom: 110 });
    expect(scrollLeft).toBe(17);
  });
});
