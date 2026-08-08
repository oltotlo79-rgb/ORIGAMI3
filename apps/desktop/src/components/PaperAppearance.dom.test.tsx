// @vitest-environment jsdom
// 紙の色(PAP-003)と方眼の数(CPE-003)の画面テスト。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { PaperAppearance } from "./PaperAppearance";
import { useAppStore } from "../store/appStore";
import { DEFAULT_DISPLAY } from "../lib/displayPrefs";

afterEach(() => {
  cleanup();
  useAppStore.setState({
    display: DEFAULT_DISPLAY,
    doc: null,
    mirrorDraw: false,
    wheelBehavior: "scroll",
    softWarnings: [],
  });
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

describe("展開図のホイール動作", () => {
  it("既定はスクロールで、上下・左右・拡大縮小の割り当てを見せる", () => {
    render(<PaperAppearance />);
    expect(screen.getByLabelText("ホイールの動作")).toHaveProperty("value", "scroll");
    expect(screen.getByText(/Shift\+ホイール: 左右/)).not.toBeNull();
    expect(screen.getByText(/Ctrl\+ホイール: カーソル位置を中心に拡大縮小/)).not.toBeNull();
  });

  it("拡大縮小へ切り替えるとスクロールがCtrl+ホイールへ入れ替わる", () => {
    render(<PaperAppearance />);
    fireEvent.change(screen.getByLabelText("ホイールの動作"), {
      target: { value: "zoom" },
    });
    expect(useAppStore.getState().wheelBehavior).toBe("zoom");
    expect(screen.getByText(/Ctrl\+ホイール: 上下/)).not.toBeNull();
    expect(screen.getByText(/Ctrl\+Shift\+ホイール: 左右/)).not.toBeNull();
  });
});

describe("重なり防止", () => {
  it("既定はオンで、切ると作品の表示設定へその場で入る", () => {
    render(<PaperAppearance />);
    const box = screen.getByLabelText("重なり防止");
    expect(box).toHaveProperty("checked", true);
    expect(screen.getByText(/完全には防げません/)).not.toBeNull();

    fireEvent.click(box);
    expect(useAppStore.getState().display.overlap_prevention_enabled).toBe(false);
    expect(box).toHaveProperty("checked", false);
  });
});

describe("紙のたわみ(SIM-012 / SIM-013)", () => {
  it("切替が出ていて、はじめは切ってある(つまみもまだ出ない)", () => {
    render(<PaperAppearance />);
    const box = screen.getByLabelText("紙のたわみを表現する");
    expect(box).toHaveProperty("checked", false);
    expect(screen.queryByLabelText("膨らみの強さ")).toBeNull();
    // 説明を読まなくても何が起きるか分かる言葉を添える
    expect(screen.getByText(/紙が丸く曲がった形/)).not.toBeNull();
  });

  it("入れると硬さと膨らみのつまみが出る", () => {
    render(<PaperAppearance />);
    fireEvent.click(screen.getByLabelText("紙のたわみを表現する"));
    expect(useAppStore.getState().display.soft_enabled).toBe(true);
    expect(screen.getByLabelText("紙の硬さ")).toHaveProperty("value", "0.5");
    expect(screen.getByLabelText("膨らみの強さ")).toHaveProperty("value", "0");
  });

  it("膨らみを動かすとその場でストアに入る(見ながら調整できる)", () => {
    useAppStore.setState({ display: { ...DEFAULT_DISPLAY, soft_enabled: true } });
    render(<PaperAppearance />);
    fireEvent.change(screen.getByLabelText("膨らみの強さ"), {
      target: { value: "0.75" },
    });
    expect(useAppStore.getState().display.soft_pressure).toBe(0.75);
    fireEvent.change(screen.getByLabelText("紙の硬さ"), { target: { value: "0.2" } });
    expect(useAppStore.getState().display.soft_stiffness).toBe(0.2);
  });

  it("計算からの注意書きは日本語でそのまま出る", () => {
    useAppStore.setState({ softWarnings: ["面の分割の細かさは4までに丸めました"] });
    render(<PaperAppearance />);
    expect(screen.getByText("面の分割の細かさは4までに丸めました")).not.toBeNull();
  });
});

describe("左右対称に描く(CPE-010)", () => {
  it("切替が出ていて、はじめは切ってある", () => {
    render(<PaperAppearance />);
    const box = screen.getByLabelText("左右対称に描く");
    expect(box).toHaveProperty("checked", false);
    // 消す・種類を変えるときにも効くこと(相手が無ければ片側だけ)を言葉で伝える
    expect(
      screen.getByText(/線を消すとき・種類を変えるときにも効き/),
    ).not.toBeNull();
    expect(screen.getByText(/対になる線が無いところは、その線だけが変わります/)).not.toBeNull();
  });

  it("入れると今その状態だと分かる案内が出る", () => {
    render(<PaperAppearance />);
    fireEvent.click(screen.getByLabelText("左右対称に描く"));
    expect(useAppStore.getState().mirrorDraw).toBe(true);
    expect(screen.getByText(/左右対称に描いています/)).not.toBeNull();
    expect(screen.getByText(/紙の縦の中心線/)).not.toBeNull();
  });
});
