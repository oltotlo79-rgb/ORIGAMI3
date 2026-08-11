// @vitest-environment jsdom
// 上の作業領域と下部コンテキストパネルの境目の画面テスト。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ContextPanelSplitter } from "./ContextPanelSplitter";
import { useAppStore } from "../store/appStore";

const DEFAULT_RATIO = 0.32;
const MIN_RATIO = 0.25;
const MAX_RATIO = 0.55;

/**
 * 実画面と同じく main-row / 仕切り / context-panel を兄弟にする。
 * 仕切りを除く使用可能高さは 600 + 300 = 900px、下端は1008px。
 */
function renderInLayout() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const result = render(
    <>
      <main className="main-row" data-testid="main-row" />
      <ContextPanelSplitter />
      <footer className="context-panel" data-testid="context-panel" />
    </>,
    { container },
  );

  const main = screen.getByTestId("main-row");
  const panel = screen.getByTestId("context-panel");
  main.getBoundingClientRect = () =>
    ({
      left: 0,
      top: 100,
      right: 1000,
      bottom: 700,
      width: 1000,
      height: 600,
      x: 0,
      y: 100,
      toJSON: () => ({}),
    }) as DOMRect;
  panel.getBoundingClientRect = () =>
    ({
      left: 0,
      top: 708,
      right: 1000,
      bottom: 1008,
      width: 1000,
      height: 300,
      x: 0,
      y: 708,
      toJSON: () => ({}),
    }) as DOMRect;

  return result;
}

function preparePointerCapture(handle: HTMLElement) {
  handle.setPointerCapture = () => {};
  handle.releasePointerCapture = () => {};
}

afterEach(() => {
  cleanup();
  useAppStore.setState({ contextPanelRatio: DEFAULT_RATIO });
});

describe("今できる操作の欄との境目", () => {
  it("上下に広さを変える水平の仕切りだと支援技術にも伝える", () => {
    useAppStore.setState({ contextPanelRatio: DEFAULT_RATIO });
    renderInLayout();

    const handle = screen.getByRole("separator");
    expect(handle.getAttribute("aria-orientation")).toBe("horizontal");
    expect(handle.getAttribute("aria-label")).toContain("広さを変える");
    expect(handle.getAttribute("data-tooltip")).toBe(
      "上下にドラッグして、下の操作欄の広さを変えます",
    );
    expect(handle.hasAttribute("title")).toBe(false);
    expect(handle.getAttribute("tabindex")).toBe("0");
    expect(handle.getAttribute("aria-valuenow")).toBe("32");
    expect(handle.getAttribute("aria-valuemin")).toBe("25");
    expect(handle.getAttribute("aria-valuemax")).toBe("55");
  });

  it("上へドラッグすると欄が広がり、離した後は動かない", () => {
    useAppStore.setState({ contextPanelRatio: DEFAULT_RATIO });
    renderInLayout();
    const handle = screen.getByRole("separator");
    preparePointerCapture(handle);

    fireEvent.pointerDown(handle, { pointerId: 1, clientY: 708 });
    // 取っ手中央の補正5pxを含め、下端1008pxから使用可能高さ900pxの
    // 40%だけ上に動かす: 1008 - 900 * 0.4 - 5 = 643px。
    fireEvent.pointerMove(handle, { pointerId: 1, clientY: 643 });
    expect(useAppStore.getState().contextPanelRatio).toBeCloseTo(0.4, 5);

    fireEvent.pointerUp(handle, { pointerId: 1, clientY: 643 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientY: 783 });
    expect(useAppStore.getState().contextPanelRatio).toBeCloseTo(0.4, 5);
  });

  it("画面外までドラッグしても上限と下限で止まる", () => {
    useAppStore.setState({ contextPanelRatio: DEFAULT_RATIO });
    renderInLayout();
    const handle = screen.getByRole("separator");
    preparePointerCapture(handle);

    fireEvent.pointerDown(handle, { pointerId: 2, clientY: 708 });
    fireEvent.pointerMove(handle, { pointerId: 2, clientY: -10_000 });
    expect(useAppStore.getState().contextPanelRatio).toBe(MAX_RATIO);
    fireEvent.pointerMove(handle, { pointerId: 2, clientY: 10_000 });
    expect(useAppStore.getState().contextPanelRatio).toBe(MIN_RATIO);
  });

  it("上下の矢印キーでも2%ずつ広さを変えられる", () => {
    useAppStore.setState({ contextPanelRatio: DEFAULT_RATIO });
    renderInLayout();
    const handle = screen.getByRole("separator");

    fireEvent.keyDown(handle, { key: "ArrowUp" });
    expect(useAppStore.getState().contextPanelRatio).toBeCloseTo(0.34, 10);
    fireEvent.keyDown(handle, { key: "ArrowDown" });
    expect(useAppStore.getState().contextPanelRatio).toBeCloseTo(DEFAULT_RATIO, 10);
  });
});
