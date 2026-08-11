// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { useAppStore } from "../store/appStore";
import { ThemeRoot } from "./ThemeRoot";

afterEach(() => {
  cleanup();
  useAppStore.setState({ uiTheme: "pop", contextPanelRatio: 0.32 });
});

function expectFrVariable(
  element: HTMLElement,
  name: "--main-row-share" | "--context-panel-share",
  expected: number,
) {
  const value = element.style.getPropertyValue(name);
  expect(value.endsWith("fr")).toBe(true);
  expect(Number(value.slice(0, -2))).toBeCloseTo(expected, 10);
}

describe("画面デザインのDOM切り替え", () => {
  it("既定のポップはdata-themeなしで、選んだテーマへ即時に切り替わる", () => {
    useAppStore.setState({ uiTheme: "pop" });
    render(
      <ThemeRoot>
        <span>内容</span>
      </ThemeRoot>,
    );

    const app = screen.getByText("内容").parentElement!;
    expect(app.className).toBe("app");
    expect(app.hasAttribute("data-theme")).toBe(false);

    act(() => useAppStore.getState().setUiTheme("modern"));
    expect(app.getAttribute("data-theme")).toBe("modern");

    act(() => useAppStore.getState().setUiTheme("pop"));
    expect(app.hasAttribute("data-theme")).toBe(false);
  });
});

describe("上下区画の広さのDOM反映", () => {
  it("ストアの割合を上の区画と今できる操作のCSS変数へ即時に反映する", () => {
    useAppStore.setState({ contextPanelRatio: 0.32 });
    render(
      <ThemeRoot>
        <span>内容</span>
      </ThemeRoot>,
    );

    const app = screen.getByText("内容").parentElement!;
    expectFrVariable(app, "--main-row-share", 0.68);
    expectFrVariable(app, "--context-panel-share", 0.32);

    act(() => useAppStore.getState().setContextPanelRatio(0.4));
    expectFrVariable(app, "--main-row-share", 0.6);
    expectFrVariable(app, "--context-panel-share", 0.4);
  });
});
