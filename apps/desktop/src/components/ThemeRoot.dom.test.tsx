// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { useAppStore } from "../store/appStore";
import { ThemeRoot } from "./ThemeRoot";

afterEach(() => {
  cleanup();
  useAppStore.setState({ uiTheme: "pop" });
});

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
