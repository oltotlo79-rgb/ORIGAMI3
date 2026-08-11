// @vitest-environment jsdom
// 2Dと3Dの境目(UI-004)の画面テスト: ドラッグとキーで割合が変わる。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { PaneSplitter } from "./PaneSplitter";
import { useAppStore } from "../store/appStore";

/** 中央行(幅1064px = ツールレール64 + 取っ手6 + 使える幅994)の中に置く */
function renderInRow() {
  const row = document.createElement("div");
  row.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: 1064, height: 600 }) as DOMRect;
  document.body.appendChild(row);
  return render(<PaneSplitter />, { container: row });
}

afterEach(() => {
  cleanup();
  useAppStore.setState({ splitRatio: 0.5 });
});

describe("2Dと3Dの境目", () => {
  it("何をする所かが分かる説明を持つ", () => {
    renderInRow();
    const handle = screen.getByRole("separator");
    expect(handle.getAttribute("aria-label")).toContain("広さを変える");
    expect(handle.getAttribute("data-tooltip")).toBe(
      "左右にドラッグして、展開図と3Dの広さを変えます",
    );
    expect(handle.hasAttribute("title")).toBe(false);
    expect(handle.getAttribute("aria-valuenow")).toBe("50");
  });

  it("左右にドラッグすると割合が変わる", () => {
    renderInRow();
    const handle = screen.getByRole("separator");
    handle.setPointerCapture = () => {};
    handle.releasePointerCapture = () => {};
    fireEvent.pointerDown(handle, { pointerId: 1, clientX: 561 });
    // 64(レール)+ 994 * 0.3 ≒ 362px の位置まで動かす
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 362 });
    expect(useAppStore.getState().splitRatio).toBeCloseTo(0.3, 2);
    fireEvent.pointerUp(handle, { pointerId: 1, clientX: 362 });
    // 離した後は動かしても変わらない
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 800 });
    expect(useAppStore.getState().splitRatio).toBeCloseTo(0.3, 2);
  });

  it("キーボードでも少しずつ動かせる", () => {
    renderInRow();
    const handle = screen.getByRole("separator");
    fireEvent.keyDown(handle, { key: "ArrowLeft" });
    expect(useAppStore.getState().splitRatio).toBeCloseTo(0.48);
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(useAppStore.getState().splitRatio).toBeCloseTo(0.5);
  });
});
