// @vitest-environment jsdom
// 紙の色(PAP-003)と方眼の数(CPE-003)の画面テスト。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { PaperAppearance } from "./PaperAppearance";
import { useAppStore } from "../store/appStore";
import { DEFAULT_DISPLAY } from "../lib/displayPrefs";

afterEach(() => {
  cleanup();
  useAppStore.setState({ display: DEFAULT_DISPLAY, doc: null, mirrorDraw: false });
});

describe("紙の色と方眼", () => {
  it("今の色と方眼の数を見せる", () => {
    render(<PaperAppearance />);
    expect(screen.getByLabelText("紙の表の色")).toHaveProperty("value", "#ed1c24");
    expect(screen.getByLabelText("紙の裏の色")).toHaveProperty("value", "#ffffff");
    expect(screen.getByLabelText("方眼の数(1辺)")).toHaveProperty("value", "8");
    // 何のための数かを折り紙の言葉で添える
    expect(screen.getByText(/等分した目盛り/)).not.toBeNull();
  });

  it("色を変えるとストアに入る", () => {
    render(<PaperAppearance />);
    fireEvent.change(screen.getByLabelText("紙の表の色"), {
      target: { value: "#0080ff" },
    });
    expect(useAppStore.getState().display.front_color).toEqual([0, 128, 255]);
    fireEvent.change(screen.getByLabelText("紙の裏の色"), {
      target: { value: "#000000" },
    });
    expect(useAppStore.getState().display.back_color).toEqual([0, 0, 0]);
  });

  it("方眼の数は2〜64に収まる", () => {
    render(<PaperAppearance />);
    const input = screen.getByLabelText("方眼の数(1辺)");
    fireEvent.change(input, { target: { value: "16" } });
    expect(useAppStore.getState().display.grid_divisions).toBe(16);
    fireEvent.change(input, { target: { value: "1" } });
    expect(useAppStore.getState().display.grid_divisions).toBe(2);
  });
});

describe("左右対称に描く(CPE-010)", () => {
  it("切替が出ていて、はじめは切ってある", () => {
    render(<PaperAppearance />);
    const box = screen.getByLabelText("左右対称に描く");
    expect(box).toHaveProperty("checked", false);
    // v1では線を引くときだけ効くことを言葉で伝える
    expect(screen.getByText(/線を引くときだけ効きます/)).not.toBeNull();
    expect(screen.getByText(/線を消す・種類を変えるときは片側ずつ/)).not.toBeNull();
  });

  it("入れると今その状態だと分かる案内が出る", () => {
    render(<PaperAppearance />);
    fireEvent.click(screen.getByLabelText("左右対称に描く"));
    expect(useAppStore.getState().mirrorDraw).toBe(true);
    expect(screen.getByText(/左右対称に描いています/)).not.toBeNull();
    expect(screen.getByText(/紙の縦の中心線/)).not.toBeNull();
  });
});
