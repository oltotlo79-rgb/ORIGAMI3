// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { ViewerOverlayStack } from "./ViewerOverlayStack";

const resizeObservers = new Set<MockResizeObserver>();
const originalResizeObserver = window.ResizeObserver;

class MockResizeObserver {
  readonly callback: ResizeObserverCallback;

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }

  observe() {
    resizeObservers.add(this);
  }
  unobserve() {}

  disconnect() {
    resizeObservers.delete(this);
  }
}

function notifyResize() {
  act(() => {
    for (const observer of resizeObservers) {
      observer.callback([], observer as unknown as ResizeObserver);
    }
  });
}

function fixedMetric(element: Element, name: "clientHeight" | "offsetTop" | "offsetHeight", read: () => number) {
  Object.defineProperty(element, name, { configurable: true, get: read });
}

beforeAll(() => {
  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    value: MockResizeObserver,
  });
});

afterAll(() => {
  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    value: originalResizeObserver,
  });
});

afterEach(() => {
  cleanup();
  resizeObservers.clear();
});

describe("3D案内列の上下操作", () => {
  it("高さ不足時だけ現れ、先頭・途中・末尾のdisabledを正しく切り替える", () => {
    render(
      <ViewerOverlayStack>
        <div data-testid="first">最初の案内</div>
        <div data-testid="last">最後の案内</div>
      </ViewerOverlayStack>,
    );
    const region = document.querySelector<HTMLElement>(".viewer-overlay-region")!;
    const stack = document.querySelector<HTMLElement>(".viewer-overlay-stack")!;
    const first = screen.getByTestId("first");
    const last = screen.getByTestId("last");
    let lastTop = 98;
    let lastHeight = 90;
    fixedMetric(region, "clientHeight", () => 100);
    fixedMetric(first, "offsetTop", () => 0);
    fixedMetric(first, "offsetHeight", () => 90);
    fixedMetric(last, "offsetTop", () => lastTop);
    fixedMetric(last, "offsetHeight", () => lastHeight);

    notifyResize();
    const up = screen.getByRole("button", { name: "3Dの案内を上へ送る" });
    const down = screen.getByRole("button", { name: "3Dの案内を下へ送る" });
    expect(region.dataset.overflow).toBe("true");
    expect(up).toHaveProperty("disabled", true);
    expect(down).toHaveProperty("disabled", false);
    expect(up.textContent).toBe("▲ 上へ");
    expect(down.textContent).toBe("▼ 下へ");
    expect(up.dataset.tooltip).toBe("3Dの案内を上へ送る");
    expect(down.dataset.tooltip).toBe("3Dの案内を下へ送る");

    fireEvent.click(down);
    expect(stack.scrollTop).toBeGreaterThan(0);
    expect(up).toHaveProperty("disabled", false);
    expect(down).toHaveProperty("disabled", false);

    fireEvent.click(down);
    fireEvent.click(down);
    expect(stack.scrollTop).toBe(128);
    expect(up).toHaveProperty("disabled", false);
    expect(down).toHaveProperty("disabled", true);

    fireEvent.click(up);
    expect(stack.scrollTop).toBeLessThan(128);
    expect(down).toHaveProperty("disabled", false);

    // 折返しが減って領域へ収まれば、操作を消して先頭へ戻す。
    lastTop = 40;
    lastHeight = 40;
    notifyResize();
    expect(region.dataset.overflow).toBe("false");
    expect(stack.scrollTop).toBe(0);
    expect(
      screen.queryByRole("button", { name: "3Dの案内を上へ送る" }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: "3Dの案内を下へ送る" }),
    ).toBeNull();
  });

  it("収まる間は操作要素を増やさない", () => {
    render(
      <ViewerOverlayStack>
        <div data-testid="only">案内</div>
      </ViewerOverlayStack>,
    );
    const region = document.querySelector<HTMLElement>(".viewer-overlay-region")!;
    const only = screen.getByTestId("only");
    fixedMetric(region, "clientHeight", () => 131.12);
    fixedMetric(only, "offsetTop", () => 0);
    fixedMetric(only, "offsetHeight", () => 92);
    notifyResize();
    expect(region.dataset.overflow).toBe("false");
    expect(document.querySelectorAll(".viewer-overlay-scroll-controls button")).toHaveLength(0);
  });
});
