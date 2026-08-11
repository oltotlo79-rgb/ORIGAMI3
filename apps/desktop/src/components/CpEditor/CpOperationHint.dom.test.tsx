// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { CpOperationHint } from "./CpOperationHint";
import { useAppStore } from "../../store/appStore";

let localValues: Record<string, string>;
const localStorageMock: Storage = {
  get length() {
    return Object.keys(localValues).length;
  },
  clear: () => {
    localValues = {};
  },
  getItem: (key) => localValues[key] ?? null,
  key: (index) => Object.keys(localValues)[index] ?? null,
  removeItem: (key) => {
    delete localValues[key];
  },
  setItem: (key, value) => {
    localValues[key] = value;
  },
};

beforeEach(() => {
  localValues = {};
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: localStorageMock,
  });
  useAppStore.setState({
    activeTool: "mountain",
    wheelBehavior: "scroll",
    cpHelpExpanded: true,
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    activeTool: "select",
    wheelBehavior: "scroll",
    cpHelpExpanded: true,
  });
});

describe("展開図左上の操作案内", () => {
  it("初回は詳しい操作を開き、畳んでも現在できる1行を残して選択を保存する", () => {
    const { unmount } = render(<CpOperationHint />);

    expect(screen.getByText("山折り線: 2回クリックで線を引きます")).toBeTruthy();
    expect(screen.getByText(/Shift\+ホイール: 左右/)).toBeTruthy();
    const close = screen.getByRole("button", {
      name: "展開図の詳しい操作方法 ▲",
    });
    expect(close.getAttribute("aria-expanded")).toBe("true");

    fireEvent.click(close);
    expect(useAppStore.getState().cpHelpExpanded).toBe(false);
    expect(screen.getByText("山折り線: 2回クリックで線を引きます")).toBeTruthy();
    expect(screen.queryByText(/Shift\+ホイール: 左右/)).toBeNull();
    expect(
      JSON.parse(globalThis.localStorage.getItem("origami3.prefs") ?? "{}"),
    ).toMatchObject({ cpHelpExpanded: false });

    unmount();
    render(<CpOperationHint />);
    expect(screen.queryByText(/Shift\+ホイール: 左右/)).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: "展開図の詳しい操作方法 ▼" }),
    );
    expect(screen.getByText(/Shift\+ホイール: 左右/)).toBeTruthy();
  });

  it("ホイール設定と選択中の道具に合わせて案内を更新する", () => {
    render(<CpOperationHint />);
    act(() => useAppStore.setState({ activeTool: "aux", wheelBehavior: "zoom" }));

    expect(screen.getByText("補助線: 2回クリックで線を引きます")).toBeTruthy();
    expect(screen.getByText(/Ctrl\+Shift\+ホイール: 左右/)).toBeTruthy();
  });
});
