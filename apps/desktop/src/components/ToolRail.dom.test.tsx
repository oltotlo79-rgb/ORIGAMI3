// @vitest-environment jsdom
// ツールレールの画面テスト: 既存の同じ区画に並ぶ道具と、
// 作図補助のサブメニュー(二等分/垂線/等分/角度線)の切り替え。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { ToolRail } from "./ToolRail";
import { useAppStore } from "../store/appStore";
import { DEFAULT_CONSTRUCT } from "../lib/construct";

afterEach(() => {
  cleanup();
  useAppStore.setState({
    activeTool: "select",
    construct: DEFAULT_CONSTRUCT,
    techniqueDraft: null,
  });
});

describe("ツールレール", () => {
  it("測るを含む10個の道具と全体表示が同じレールに並ぶ", () => {
    render(<ToolRail onFitView={() => {}} />);
    expect(screen.getAllByRole("button")).toHaveLength(11);
    fireEvent.click(screen.getByRole("button", { name: "測る" }));
    expect(useAppStore.getState().activeTool).toBe("measure");
  });

  it("紙をつかんで引くツールを選べる", () => {
    render(<ToolRail onFitView={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "引く" }));
    expect(useAppStore.getState().activeTool).toBe("pull");
  });

  it("紙を動かすgroupはつまんで動かす・引くの既存2道具だけを含む", () => {
    render(<ToolRail onFitView={() => {}} />);

    const group = screen.getByRole("group", { name: "紙を動かす" });
    expect(within(group).getByText("紙を動かす")).toBeTruthy();
    expect(
      within(group)
        .getAllByRole("button")
        .map((button) => button.dataset.testid),
    ).toEqual(["tool-fold", "tool-pull"]);

    const fold = within(group).getByRole("button", { name: "折る" });
    expect(fold.dataset.tooltip).toContain("つまんで動かす");
    expect(fold.dataset.tooltip).toContain("つまんだ層");
    expect(fold.dataset.tooltip).toContain("8通り");
    expect(group.contains(screen.getByRole("button", { name: "技法" }))).toBe(false);
  });

  it("技法サブメニューは手動で技法を選んでいる間だけ表示する", () => {
    render(<ToolRail onFitView={() => {}} />);
    const menu = () => screen.queryByRole("group", { name: "技法を選ぶ" });

    expect(menu()).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "折る" }));
    expect(menu()).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "引く" }));
    expect(menu()).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "技法" }));
    expect(menu()).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "折る" }));
    expect(menu()).toBeNull();
  });

  it("group化してもfoldとpullは従来のToolIdを選ぶ", () => {
    render(<ToolRail onFitView={() => {}} />);
    const fold = screen.getByTestId("tool-fold");
    const pull = screen.getByTestId("tool-pull");

    fireEvent.click(fold);
    expect(useAppStore.getState().activeTool).toBe("fold");
    expect(fold.classList.contains("active")).toBe(true);
    expect(pull.classList.contains("active")).toBe(false);

    fireEvent.click(pull);
    expect(useAppStore.getState().activeTool).toBe("pull");
    expect(fold.classList.contains("active")).toBe(false);
    expect(pull.classList.contains("active")).toBe(true);
  });

  it("作図を選ぶとサブメニューが出て、等分では数を選べる", () => {
    render(<ToolRail onFitView={() => {}} />);
    // 作図を選ぶまではサブメニューを出さない(常設のボタンを増やさないため)
    expect(screen.queryByRole("group", { name: "作図の種類を選ぶ" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "作図" }));
    expect(useAppStore.getState().activeTool).toBe("construct");
    const menu = screen.getByRole("group", { name: "作図の種類を選ぶ" });
    for (const label of ["二等分", "垂線", "等分", "角度線"]) {
      expect(screen.getByRole("button", { name: label })).toBeTruthy();
    }
    // 種類ごとの数選びは、その種類を選んだときだけ出す
    expect(menu.querySelector("select")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "等分" }));
    expect(useAppStore.getState().construct.kind).toBe("divide");
    const select = screen.getByLabelText("いくつに等分するか") as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "6" } });
    expect(useAppStore.getState().construct.divisions).toBe(6);

    fireEvent.click(screen.getByRole("button", { name: "角度線" }));
    const step = screen.getByLabelText("角度の刻み") as HTMLSelectElement;
    expect(step.value).toBe("22.5");
  });
});
