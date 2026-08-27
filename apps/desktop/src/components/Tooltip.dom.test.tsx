// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { TooltipHost } from "./Tooltip";

function rect(
  left: number,
  top: number,
  width: number,
  height: number,
): DOMRect {
  return {
    x: left,
    y: top,
    left,
    top,
    right: left + width,
    bottom: top + height,
    width,
    height,
    toJSON: () => ({}),
  } as DOMRect;
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("共通の操作吹き出し", () => {
  it("マウスを乗せるとdata-tooltipを優先して表示し、離すと隠す", () => {
    render(
      <>
        <button
          type="button"
          data-tooltip="作品を保存します"
          aria-label="保存"
        >
          保存
        </button>
        <TooltipHost />
      </>,
    );

    const button = screen.getByRole("button", { name: "保存" });
    fireEvent.mouseOver(button);
    expect(screen.getByRole("tooltip").textContent).toBe("作品を保存します");

    fireEvent.mouseOut(button, { relatedTarget: document.body });
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("キーボードの焦点でもlabelから説明を表示し、焦点が外れると隠す", () => {
    render(
      <>
        <label htmlFor="fold-angle">折る角度</label>
        <input id="fold-angle" type="number" />
        <TooltipHost />
      </>,
    );

    const input = screen.getByRole("spinbutton", { name: "折る角度" });
    fireEvent.focusIn(input);
    const tooltip = screen.getByRole("tooltip");
    expect(tooltip.textContent).toBe("折る角度");
    expect(input.getAttribute("aria-describedby")).toContain(tooltip.id);

    fireEvent.focusOut(input, { relatedTarget: document.body });
    expect(screen.queryByRole("tooltip")).toBeNull();
    expect(input.hasAttribute("aria-describedby")).toBe(false);
  });

  it("説明属性が無いボタンは表示文字を短い説明に使う", () => {
    render(
      <>
        <button type="button">手順を開く</button>
        <TooltipHost />
      </>,
    );

    fireEvent.mouseOver(screen.getByRole("button", { name: "手順を開く" }));
    expect(screen.getByRole("tooltip").textContent).toBe("手順を開く");
  });

  it("上に置けないときは下へ出し、実測サイズで画面端から8px空ける", () => {
    vi.stubGlobal("innerWidth", 240);
    vi.stubGlobal("innerHeight", 140);
    render(
      <>
        <button type="button" aria-label="右上の操作">
          操作
        </button>
        <TooltipHost />
      </>,
    );

    const button = screen.getByRole("button", { name: "右上の操作" });
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
      function (this: HTMLElement) {
        if (this === button) return rect(220, 2, 20, 20);
        if (this.getAttribute("role") === "tooltip") return rect(0, 0, 100, 40);
        return rect(0, 0, 0, 0);
      },
    );

    fireEvent.mouseOver(button);
    const tooltip = screen.getByRole("tooltip");
    expect(tooltip.style.position).toBe("fixed");
    expect(tooltip.style.left).toBe("132px");
    expect(tooltip.style.top).toBe("30px");
    expect(Number.parseFloat(tooltip.style.left)).toBeGreaterThanOrEqual(8);
    expect(Number.parseFloat(tooltip.style.top)).toBeGreaterThanOrEqual(8);
    expect(Number.parseFloat(tooltip.style.left) + 100).toBeLessThanOrEqual(232);
    expect(Number.parseFloat(tooltip.style.top) + 40).toBeLessThanOrEqual(132);
  });

  it("hides without pointer movement when the displayed anchor unmounts", async () => {
    const { rerender } = render(
      <>
        <div>
          <button type="button" data-tooltip="全部の折り目を動かす割合">
            一斉折りの割合
          </button>
        </div>
        <TooltipHost />
      </>,
    );

    const anchor = screen.getByRole("button", { name: "一斉折りの割合" });
    fireEvent.mouseOver(anchor);
    expect(screen.getByRole("tooltip").textContent).toBe(
      "全部の折り目を動かす割合",
    );

    rerender(
      <>
        <div />
        <TooltipHost />
      </>,
    );

    await waitFor(() => {
      expect(screen.queryAllByRole("tooltip")).toHaveLength(0);
    });
  });
});
