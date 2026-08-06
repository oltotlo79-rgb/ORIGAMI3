// @vitest-environment jsdom
// ツールレールの画面テスト: ボタン数の上限(要件§2で10個以内)と、
// 作図補助のサブメニュー(二等分/垂線/等分/角度線)の切り替え。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ToolRail } from "./ToolRail";
import { useAppStore } from "../store/appStore";
import { DEFAULT_CONSTRUCT } from "../lib/construct";

afterEach(() => {
  cleanup();
  useAppStore.setState({ activeTool: "select", construct: DEFAULT_CONSTRUCT });
});

describe("ツールレール", () => {
  it("常設のボタンは10個以内に収まっている", () => {
    render(<ToolRail onFitView={() => {}} />);
    expect(screen.getAllByRole("button").length).toBeLessThanOrEqual(10);
  });

  it("紙をつかんで引くツールを選べる", () => {
    render(<ToolRail onFitView={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "引く" }));
    expect(useAppStore.getState().activeTool).toBe("pull");
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
