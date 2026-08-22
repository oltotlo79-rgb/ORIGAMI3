import { describe, expect, it } from "vitest";
import {
  layoutViewerOverlayCards,
  VIEWER_OVERLAY_SCROLL_CONTROLS_HEIGHT_PX,
  viewerOverlayScrollState,
  viewerOverlayScrollTarget,
  viewerOverlayStackViewport,
} from "./viewerOverlayLayout";
import {
  overlayRectsOverlap,
  viewCubeOverlayRects,
} from "./viewCube";

const PANE_HEIGHT = 335.12;
const WIDTHS = [744, 465, 186] as const;
const HINT_HEIGHT = { collapsed: 92, expanded: 306 } as const;
const NOTICE_STATES = [
  { name: "なし", heights: [] },
  { name: "通知", heights: [32] },
  { name: "原因候補", heights: [32] },
  { name: "両方", heights: [32, 32] },
] as const;
const LOWER_CARDS = [
  { name: "紙の小さい札", height: 32 },
  { name: "紙の詳しい札", height: 122 },
  { name: "折り方の札", height: 170 },
] as const;

function verifyLayout(width: number, heights: readonly number[]) {
  const layout = layoutViewerOverlayCards(width, PANE_HEIGHT, heights);
  const value = layout;
  expect(value.horizontalOverflowPx).toBe(0);
  expect(value.viewport.left).toBeGreaterThanOrEqual(0);
  expect(value.viewport.right).toBeLessThanOrEqual(width);
  for (let index = 0; index < value.cards.length; index += 1) {
    const card = value.cards[index];
    expect(card.left).toBe(value.viewport.left);
    expect(card.right).toBe(value.viewport.right);
    if (index > 0) {
      expect(card.top).toBeGreaterThanOrEqual(value.cards[index - 1].bottom);
    }
  }
  const last = value.cards[value.cards.length - 1];
  if (last !== undefined) {
    // 最大まで送れば最後の札の下端が列の表示下端へ届く。
    expect(last.bottom - value.scrollRange).toBeLessThanOrEqual(
      value.viewport.bottom + 1e-9,
    );
    const end = viewerOverlayScrollState({
      contentHeight: value.contentHeight,
      availableHeight: value.viewport.height,
      scrollTop: Number.MAX_VALUE,
    });
    // 送りボタンが出る場合はその40px上を表示下端にし、末尾の札をボタンで隠さない。
    expect(last.bottom - end.maxScrollTop).toBeLessThanOrEqual(
      value.viewport.top + end.viewportHeight + 1e-9,
    );
  }
  expect(value.scrollRange > 0).toBe(value.contentHeight > value.viewport.height);
}

describe("1000×700の3D案内札", () => {
  it("最大幅744pxと既定幅465pxでは立方体の左、最狭186pxでは下へ置く", () => {
    expect(viewerOverlayStackViewport(744, PANE_HEIGHT)).toEqual({
      left: 12,
      top: 12,
      right: 442,
      bottom: 323.12,
      width: 430,
      height: 311.12,
    });
    expect(viewerOverlayStackViewport(465, PANE_HEIGHT)).toEqual({
      left: 12,
      top: 12,
      right: 313,
      bottom: 323.12,
      width: 301,
      height: 311.12,
    });
    expect(viewerOverlayStackViewport(186, PANE_HEIGHT)).toEqual({
      left: 12,
      top: 148,
      right: 174,
      bottom: 279.12,
      width: 162,
      height: 131.12,
    });
    for (const width of WIDTHS) {
      const stack = viewerOverlayStackViewport(width, PANE_HEIGHT);
      const fixed = viewCubeOverlayRects(width, PANE_HEIGHT);
      expect(overlayRectsOverlap(stack, fixed.cube)).toBe(false);
      expect(overlayRectsOverlap(stack, fixed.resetButton)).toBe(false);
    }
  });

  it("高さ不足のときだけ40pxの操作行を予約し、先頭・途中・末尾を判定する", () => {
    expect(
      viewerOverlayScrollState({
        contentHeight: 311,
        availableHeight: 311.12,
        scrollTop: 0,
      }),
    ).toEqual({
      overflowing: false,
      canScrollUp: false,
      canScrollDown: false,
      scrollTop: 0,
      maxScrollTop: 0,
      viewportHeight: 311.12,
    });

    const start = viewerOverlayScrollState({
      contentHeight: 500,
      availableHeight: 311.12,
      scrollTop: 0,
    });
    expect(start.viewportHeight).toBe(
      311.12 - VIEWER_OVERLAY_SCROLL_CONTROLS_HEIGHT_PX,
    );
    expect(start.maxScrollTop).toBeCloseTo(228.88, 10);
    expect(start.canScrollUp).toBe(false);
    expect(start.canScrollDown).toBe(true);

    const middle = viewerOverlayScrollState({
      contentHeight: 500,
      availableHeight: 311.12,
      scrollTop: 100,
    });
    expect(middle.canScrollUp).toBe(true);
    expect(middle.canScrollDown).toBe(true);

    const end = viewerOverlayScrollState({
      contentHeight: 500,
      availableHeight: 311.12,
      scrollTop: 999,
    });
    expect(end.scrollTop).toBeCloseTo(228.88, 10);
    expect(end.canScrollUp).toBe(true);
    expect(end.canScrollDown).toBe(false);

    const subpixelOverflow = viewerOverlayScrollState({
      contentHeight: 311.62,
      availableHeight: 311.12,
      scrollTop: 0,
    });
    expect(subpixelOverflow.overflowing).toBe(true);
    expect(subpixelOverflow.canScrollDown).toBe(true);

    const subpixelBeforeEnd = viewerOverlayScrollState({
      contentHeight: 500,
      availableHeight: 311.12,
      scrollTop: end.maxScrollTop - 0.5,
    });
    expect(subpixelBeforeEnd.canScrollDown).toBe(true);
    expect(
      viewerOverlayScrollTarget(
        {
          contentHeight: 500,
          availableHeight: 311.12,
          scrollTop: subpixelBeforeEnd.scrollTop,
        },
        "down",
      ),
    ).toBe(end.maxScrollTop);
  });

  it("上下操作は狭い131.12px領域でも端を越えず、必ず前後へ送る", () => {
    const metrics = {
      contentHeight: 306,
      availableHeight: 131.12,
      scrollTop: 0,
    };
    const firstDown = viewerOverlayScrollTarget(metrics, "down");
    expect(firstDown).toBeGreaterThanOrEqual(48);
    expect(firstDown).toBeLessThan(306);
    const end = viewerOverlayScrollState({ ...metrics, scrollTop: 999 });
    expect(
      viewerOverlayScrollTarget({ ...metrics, scrollTop: end.scrollTop }, "down"),
    ).toBe(end.maxScrollTop);
    expect(
      viewerOverlayScrollTarget({ ...metrics, scrollTop: end.scrollTop }, "up"),
    ).toBeLessThan(end.scrollTop);
  });

  it("通知・開閉・下側の札を網羅した39状態で交差せず最後まで送れる", () => {
    let checked = 0;
    for (const width of WIDTHS) {
      for (const hint of ["collapsed", "expanded"] as const) {
        for (const notice of NOTICE_STATES) {
          verifyLayout(width, [...notice.heights, HINT_HEIGHT[hint]]);
          checked += 1;
        }
      }
      for (const lower of LOWER_CARDS) {
        verifyLayout(width, [HINT_HEIGHT.expanded, lower.height]);
        checked += 1;
      }
      for (const lower of LOWER_CARDS.slice(1)) {
        verifyLayout(width, [32, 32, HINT_HEIGHT.expanded, lower.height]);
        checked += 1;
      }
    }
    expect(checked).toBe(39);
  });
});
