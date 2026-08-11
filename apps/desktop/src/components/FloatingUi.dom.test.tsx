// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { PaperAppearance } from "./PaperAppearance";
import { DEFAULT_DISPLAY } from "../lib/displayPrefs";
import { findOverflowingFloatingUi } from "../lib/floatingUi";
import { useAppStore } from "../store/appStore";

const VIEWPORT = { width: 1000, height: 700 };

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return {
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
    x: left,
    y: top,
    toJSON: () => ({}),
  };
}

const POSITION_UNCONTROLLABLE_INPUT_TYPES = new Set([
  "color",
  "date",
  "datetime-local",
  "file",
  "month",
  "time",
  "week",
]);

beforeEach(() => {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: VIEWPORT.width,
  });
  Object.defineProperty(window, "innerHeight", {
    configurable: true,
    value: VIEWPORT.height,
  });
  useAppStore.setState({
    display: DEFAULT_DISPLAY,
    paperColorExpanded: true,
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    display: DEFAULT_DISPLAY,
    paperColorExpanded: false,
  });
});

describe("浮動UIの共通画面内検査", () => {
  it("右下の「その他の色」から開いても四辺が画面内に収まる", () => {
    render(<PaperAppearance />);

    // OS標準の色選択は位置を検査できないため、アプリ内の浮動UIへ置き換える。
    const nativePopupInputs = Array.from(document.querySelectorAll("input")).filter(
      (input) => POSITION_UNCONTROLLABLE_INPUT_TYPES.has(input.type),
    );
    expect(nativePopupInputs).toHaveLength(0);
    const trigger = screen.getByRole("button", {
      name: "紙の表のその他の色を開く",
    });
    Object.defineProperty(trigger, "getBoundingClientRect", {
      configurable: true,
      value: () => rect(920, 650, 64, 32),
    });

    fireEvent.click(trigger);
    const picker = screen.getByRole("dialog", { name: "紙の表の色を選ぶ" });
    Object.defineProperty(picker, "getBoundingClientRect", {
      configurable: true,
      value: () => {
        const left = Number.parseFloat(picker.style.left);
        const top = Number.parseFloat(picker.style.top);
        return rect(left, top, 304, 332);
      },
    });
    fireEvent(window, new Event("resize"));

    expect(document.querySelectorAll("[data-floating-ui]")).toHaveLength(1);
    expect(findOverflowingFloatingUi(document, VIEWPORT)).toEqual([]);
  });
});
