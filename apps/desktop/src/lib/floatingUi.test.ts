// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  findOverflowingFloatingUi,
  placeFloatingUi,
  type FloatingSize,
  type FloatingViewport,
} from "./floatingUi";

function rect(left: number, top: number, width: number, height: number): DOMRect {
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

function expectInsideViewport(
  position: { left: number; top: number },
  size: FloatingSize,
  viewport: FloatingViewport,
  padding = 8,
): void {
  expect(position.left).toBeGreaterThanOrEqual(padding);
  expect(position.top).toBeGreaterThanOrEqual(padding);
  expect(position.left + size.width).toBeLessThanOrEqual(viewport.width - padding);
  expect(position.top + size.height).toBeLessThanOrEqual(viewport.height - padding);
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("浮動UIの共通配置", () => {
  const viewport = { width: 320, height: 240 };
  const size = { width: 100, height: 80 };

  it.each([
    ["左上では下・右方向", rect(8, 8, 20, 20), { left: 8, top: 36 }],
    ["左下では上・右方向", rect(8, 212, 20, 20), { left: 8, top: 124 }],
    ["右上では下・左方向", rect(292, 8, 20, 20), { left: 212, top: 36 }],
    ["右下では上・左方向", rect(292, 212, 20, 20), { left: 212, top: 124 }],
  ])("%sへ反転して四隅でも収まる", (_name, anchor, expected) => {
    const position = placeFloatingUi(anchor, size, viewport);
    expect(position).toEqual(expected);
    expectInsideViewport(position, size, viewport);
  });

  it("上下左右のどちら側にも入らないときは画面内へ詰める", () => {
    const smallViewport = { width: 120, height: 90 };
    const almostFullSize = { width: 100, height: 70 };
    const position = placeFloatingUi(
      rect(50, 40, 20, 20),
      almostFullSize,
      smallViewport,
    );

    expect(position).toEqual({ left: 12, top: 12 });
    expectInsideViewport(position, almostFullSize, smallViewport);
  });

  it("指定した余白と起点からの間隔を使う", () => {
    const position = placeFloatingUi(rect(12, 12, 20, 20), size, viewport, {
      padding: 12,
      gap: 6,
    });

    expect(position).toEqual({ left: 12, top: 38 });
    expectInsideViewport(position, size, viewport, 12);
  });
});

describe("浮動UIの共通はみ出し検査", () => {
  it("属性の付いた全要素を調べ、四辺の外側だけをまとめて返す", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const cases = [
      ["inside", rect(0, 0, 320, 240)],
      ["left", rect(-1, 20, 40, 40)],
      ["top", rect(20, -1, 40, 40)],
      ["right", rect(281, 20, 40, 40)],
      ["bottom", rect(20, 201, 40, 40)],
    ] as const;

    for (const [name, bounds] of cases) {
      const element = document.createElement("section");
      element.dataset.floatingUi = name;
      element.getBoundingClientRect = () => bounds;
      root.append(element);
    }
    const unmarked = document.createElement("section");
    unmarked.getBoundingClientRect = () => rect(-100, -100, 10, 10);
    root.append(unmarked);

    const overflowing = findOverflowingFloatingUi(root, {
      width: 320,
      height: 240,
    });

    expect(overflowing.map(({ element }) => element.dataset.floatingUi)).toEqual([
      "left",
      "top",
      "right",
      "bottom",
    ]);
  });

  it("既定ではdocumentと実際のwindow寸法を使う", () => {
    vi.stubGlobal("innerWidth", 160);
    vi.stubGlobal("innerHeight", 120);
    const element = document.createElement("div");
    element.dataset.floatingUi = "default-viewport";
    element.getBoundingClientRect = () => rect(130, 90, 40, 40);
    document.body.append(element);

    expect(findOverflowingFloatingUi().map(({ element: found }) => found)).toEqual([
      element,
    ]);
  });
});
