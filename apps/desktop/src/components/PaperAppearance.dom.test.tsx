// @vitest-environment jsdom
// 紙の色(PAP-003)と方眼の数(CPE-003)の画面テスト。

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import {
  GRID_DIVISION_PRESETS,
  PAPER_COLOR_PALETTE,
  PaperAppearance,
} from "./PaperAppearance";
import { ThemeRoot } from "./ThemeRoot";
import { useAppStore } from "../store/appStore";
import {
  DEFAULT_CONTEXT_PANEL_RATIO,
  DEFAULT_DISPLAY,
  DEFAULT_SPLIT_RATIO,
  UI_THEMES,
} from "../lib/displayPrefs";

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
    doc: null,
    selection: { edgeIds: [], vertexIds: [] },
    mirrorAxis: { kind: "paperVertical" },
    paperHelpExpanded: true,
    paperColorExpanded: false,
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    display: DEFAULT_DISPLAY,
    doc: null,
    selection: { edgeIds: [], vertexIds: [] },
    mirrorDraw: false,
    mirrorAxis: { kind: "paperVertical" },
    mirrorAxisNotice: null,
    wheelBehavior: "scroll",
    uiTheme: "pop",
    splitRatio: DEFAULT_SPLIT_RATIO,
    contextPanelRatio: DEFAULT_CONTEXT_PANEL_RATIO,
    softWarnings: [],
    paperHelpExpanded: true,
    paperColorExpanded: false,
  });
});

describe("紙の色と方眼", () => {
  it("初期状態は紙の色を畳み、表裏の現在色と方眼の数を見せる", () => {
    render(<PaperAppearance />);
    const toggle = screen.getByRole("button", { name: "紙の色 ▼" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(toggle.textContent).toContain("表");
    expect(toggle.textContent).toContain("裏");
    const current = screen.getByText(/紙の表の現在色/);
    expect(current.textContent).toContain("#ed1c24");
    expect(current.textContent).toContain("#ffffff");
    expect(screen.queryByRole("group", { name: "紙の表の24色パレット" })).toBeNull();
    expect(document.querySelectorAll('input[type="color"]')).toHaveLength(0);
    expect(screen.getByLabelText("方眼の細かさ（1辺の等分数）")).toHaveProperty(
      "value",
      "8",
    );
    expect(screen.getByText("方眼の細かさ")).not.toBeNull();
    expect(screen.getByText((_, element) => element?.textContent === "8等分")).not.toBeNull();
    const input = screen.getByLabelText("方眼の細かさ（1辺の等分数）");
    expect(input.getAttribute("data-tooltip")).toBe(
      "1辺を何等分する方眼にするか指定します",
    );
    expect(input.hasAttribute("title")).toBe(false);
    expect(screen.queryByText(/等分した目盛り/)).toBeNull();
  });

  it("紙の色を開閉して端末へ覚え、16進数で表裏の色を確定できる", () => {
    render(<PaperAppearance />);
    fireEvent.click(screen.getByRole("button", { name: "紙の色 ▼" }));
    expect(useAppStore.getState().paperColorExpanded).toBe(true);
    expect(
      JSON.parse(globalThis.localStorage.getItem("origami3.prefs") ?? "{}"),
    ).toMatchObject({ paperColorExpanded: true });

    fireEvent.click(
      screen.getByRole("button", { name: "紙の表のその他の色を開く" }),
    );
    const frontHex = screen.getByLabelText("紙の表の16進数の色コード");
    fireEvent.change(frontHex, { target: { value: "#0080ff" } });
    fireEvent.keyDown(frontHex, { key: "Enter" });
    expect(useAppStore.getState().display.front_color).toEqual([0, 128, 255]);

    fireEvent.click(
      screen.getByRole("button", { name: "紙の裏のその他の色を開く" }),
    );
    const backHex = screen.getByLabelText("紙の裏の16進数の色コード");
    fireEvent.change(backHex, { target: { value: "#000000" } });
    fireEvent.keyDown(backHex, { key: "Enter" });
    expect(useAppStore.getState().display.back_color).toEqual([0, 0, 0]);
  });

  it("表と裏をそれぞれ24色の見本から選べて、現在色に印が付く", () => {
    render(<PaperAppearance />);
    fireEvent.click(screen.getByRole("button", { name: "紙の色 ▼" }));
    const front = screen.getByRole("group", { name: "紙の表の24色パレット" });
    const back = screen.getByRole("group", { name: "紙の裏の24色パレット" });
    expect(within(front).getAllByRole("button")).toHaveLength(24);
    expect(within(back).getAllByRole("button")).toHaveLength(24);
    expect(PAPER_COLOR_PALETTE).toHaveLength(24);

    const red = within(front).getByRole("button", { name: "紙の表を赤にする" });
    expect(red.getAttribute("data-tooltip")).toBe("赤を選びます");
    expect(red.hasAttribute("title")).toBe(false);
    expect(red.getAttribute("aria-pressed")).toBe("true");

    const purple = within(front).getByRole("button", { name: "紙の表を紫にする" });
    fireEvent.click(purple);
    expect(useAppStore.getState().display.front_color).toEqual([112, 64, 201]);
    expect(purple.getAttribute("aria-pressed")).toBe("true");
    expect(red.getAttribute("aria-pressed")).toBe("false");
  });

  it("よく使う9種類の方眼を選べて現在値を示す", () => {
    render(<PaperAppearance />);
    const presets = screen.getByRole("group", { name: "よく使う方眼の細かさ" });
    expect(within(presets).getAllByRole("button")).toHaveLength(9);
    expect(GRID_DIVISION_PRESETS).toEqual([4, 8, 12, 16, 24, 32, 64, 128, 256]);
    const eight = within(presets).getByRole("button", { name: "8" });
    expect(eight.getAttribute("aria-pressed")).toBe("true");

    const twentyFour = within(presets).getByRole("button", { name: "24" });
    fireEvent.click(twentyFour);
    expect(useAppStore.getState().display.grid_divisions).toBe(24);
    expect(twentyFour.getAttribute("aria-pressed")).toBe("true");
    expect(eight.getAttribute("aria-pressed")).toBe("false");
  });

  it("方眼の数は任意に指定でき、2〜1024に収まる", () => {
    render(<PaperAppearance />);
    const input = screen.getByLabelText("方眼の細かさ（1辺の等分数）");
    fireEvent.change(input, { target: { value: "16" } });
    expect(useAppStore.getState().display.grid_divisions).toBe(16);
    fireEvent.change(input, { target: { value: "1" } });
    expect(useAppStore.getState().display.grid_divisions).toBe(2);
    fireEvent.change(input, { target: { value: "1024" } });
    expect(useAppStore.getState().display.grid_divisions).toBe(1024);
    fireEvent.change(input, { target: { value: "2048" } });
    expect(useAppStore.getState().display.grid_divisions).toBe(1024);
    expect(input).toHaveProperty("max", "1024");
  });

  it("方眼の自由指定は上下ボタンで1ずつ変わり、上下限を越えない", () => {
    render(<PaperAppearance />);
    const input = screen.getByLabelText(
      "方眼の細かさ（1辺の等分数）",
    ) as HTMLInputElement;
    const increment = screen.getByRole("button", {
      name: "方眼の細かさ（1辺の等分数）を増やす",
    });
    const decrement = screen.getByRole("button", {
      name: "方眼の細かさ（1辺の等分数）を減らす",
    });

    fireEvent.click(increment);
    expect(input.value).toBe("9");
    expect(useAppStore.getState().display.grid_divisions).toBe(9);
    fireEvent.click(decrement);
    expect(input.value).toBe("8");
    expect(useAppStore.getState().display.grid_divisions).toBe(8);

    fireEvent.change(input, { target: { value: "2" } });
    fireEvent.click(decrement);
    expect(input.value).toBe("2");
    expect(useAppStore.getState().display.grid_divisions).toBe(2);

    fireEvent.change(input, { target: { value: "1024" } });
    fireEvent.click(increment);
    expect(input.value).toBe("1024");
    expect(useAppStore.getState().display.grid_divisions).toBe(1024);
  });
});

describe("展開図のホイール動作", () => {
  it("既定はスクロールで、割り当てを短い吹き出し用説明に持つ", () => {
    render(<PaperAppearance />);
    const select = screen.getByLabelText("ホイールの動作");
    expect(select).toHaveProperty("value", "scroll");
    expect(select.getAttribute("data-tooltip")).toBe(
      "ホイールで上下、Shiftで左右、Ctrlで拡大縮小します",
    );
    expect(select.hasAttribute("title")).toBe(false);
    expect(screen.queryByText(/Shift\+ホイール: 左右/)).toBeNull();
  });

  it("拡大縮小へ切り替えるとスクロールがCtrl+ホイールへ入れ替わる", () => {
    render(<PaperAppearance />);
    fireEvent.change(screen.getByLabelText("ホイールの動作"), {
      target: { value: "zoom" },
    });
    expect(useAppStore.getState().wheelBehavior).toBe("zoom");
    expect(
      screen.getByLabelText("ホイールの動作").getAttribute("data-tooltip"),
    ).toBe("ホイールで拡大縮小、Ctrlで上下、Ctrl+Shiftで左右へ動かします");
    expect(screen.queryByText(/Ctrl\+ホイール: 上下/)).toBeNull();
  });
});

describe("画面のデザイン", () => {
  it("既定はポップで、5テーマを日本語名から選べる", () => {
    useAppStore.setState({ uiTheme: "pop" });
    render(<PaperAppearance />);
    const select = screen.getByLabelText("画面のデザイン");
    expect(select).toHaveProperty("value", "pop");
    expect(within(select).getAllByRole("option").map((option) => option.textContent)).toEqual([
      "ポップ",
      "シンプル",
      "和風",
      "モダン",
      "クラシック",
    ]);
  });

  it("5テーマをその場で切り替え、data-themeと端末保存を同時に更新する", () => {
    render(
      <ThemeRoot>
        <PaperAppearance />
      </ThemeRoot>,
    );
    const app = document.querySelector(".app");
    expect(app).not.toBeNull();
    expect(app!.hasAttribute("data-theme")).toBe(false);

    for (const uiTheme of UI_THEMES) {
      fireEvent.change(screen.getByLabelText("画面のデザイン"), {
        target: { value: uiTheme },
      });

      expect(useAppStore.getState().uiTheme).toBe(uiTheme);
      if (uiTheme === "pop") expect(app!.hasAttribute("data-theme")).toBe(false);
      else expect(app!.getAttribute("data-theme")).toBe(uiTheme);
      expect(
        JSON.parse(globalThis.localStorage.getItem("origami3.prefs") ?? "{}"),
      ).toMatchObject({ uiTheme });
    }
  });

  it("表示区画の広さを初期値へ戻し、端末設定にも保存する", () => {
    useAppStore.setState({ splitRatio: 0.72, contextPanelRatio: 0.5 });
    render(<PaperAppearance />);

    fireEvent.click(screen.getByRole("button", { name: "表示の広さを初期に戻す" }));

    expect(useAppStore.getState().splitRatio).toBe(DEFAULT_SPLIT_RATIO);
    expect(useAppStore.getState().contextPanelRatio).toBe(DEFAULT_CONTEXT_PANEL_RATIO);
    expect(
      JSON.parse(globalThis.localStorage.getItem("origami3.prefs") ?? "{}"),
    ).toMatchObject({
      splitRatio: DEFAULT_SPLIT_RATIO,
      contextPanelRatio: DEFAULT_CONTEXT_PANEL_RATIO,
    });
  });
});

describe("重なり防止", () => {
  it("既定はオンで、切ると作品の表示設定へその場で入る", () => {
    render(<PaperAppearance />);
    const box = screen.getByLabelText("重なり防止");
    expect(box).toHaveProperty("checked", true);
    expect(box.getAttribute("data-tooltip")).toBe(
      "折る途中で紙どうしが突き抜けにくい補正を切り替えます",
    );
    expect(box.hasAttribute("title")).toBe(false);
    expect(screen.queryByText(/完全には防げません/)).toBeNull();

    fireEvent.click(box);
    expect(useAppStore.getState().display.overlap_prevention_enabled).toBe(false);
    expect(box).toHaveProperty("checked", false);
  });
});

describe("食い込み検出", () => {
  it("既定はオンで、切ると作品の表示設定へその場で入る", () => {
    render(<PaperAppearance />);
    const box = screen.getByLabelText("食い込み検出");
    expect(box).toHaveProperty("checked", true);
    expect(box.getAttribute("data-tooltip")).toBe(
      "紙の接触を赤い折り目と警告で知らせる検出を切り替えます",
    );
    expect(box.hasAttribute("title")).toBe(false);
    expect(screen.queryByText(/角度操作は止めません/)).toBeNull();

    fireEvent.click(box);
    expect(useAppStore.getState().display.penetration_prevention_enabled).toBe(false);
    expect(box).toHaveProperty("checked", false);
  });
});

describe("紙のたわみ(SIM-012 / SIM-013)", () => {
  it("切替と詳しい説明は初回に開いて出る(つまみはまだ出ない)", () => {
    render(<PaperAppearance />);
    const box = screen.getByLabelText("紙のたわみを表現する");
    expect(box).toHaveProperty("checked", false);
    expect(screen.queryByLabelText("膨らみの強さ")).toBeNull();
    expect(screen.getByText("丸みと膨らみを3Dで調整できます")).not.toBeNull();
    expect(screen.getByText(/折り目以外にも丸みを見せる表示/)).not.toBeNull();
    expect(
      screen.getByRole("button", { name: /丸みの詳しい操作方法/ }).getAttribute(
        "aria-expanded",
      ),
    ).toBe("true");
  });

  it("説明を畳んでも見出し・要点・入力・計算の注意を残し、選択を端末へ保存する", () => {
    useAppStore.setState({
      display: { ...DEFAULT_DISPLAY, soft_enabled: true },
      softWarnings: ["面の分割の細かさは4までに丸めました"],
      paperHelpExpanded: true,
    });
    render(<PaperAppearance />);

    const toggle = screen.getByRole("button", { name: /丸みの詳しい操作方法/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText(/硬さで紙の曲がりやすさ/)).not.toBeNull();

    fireEvent.click(toggle);

    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(useAppStore.getState().paperHelpExpanded).toBe(false);
    expect(
      JSON.parse(globalThis.localStorage.getItem("origami3.prefs") ?? "{}"),
    ).toMatchObject({ paperHelpExpanded: false });
    expect(screen.getByText("紙をふくらませる")).not.toBeNull();
    expect(screen.getByText("丸みと膨らみを3Dで調整できます")).not.toBeNull();
    expect(screen.getByLabelText("紙のたわみを表現する")).not.toBeNull();
    expect(screen.getByLabelText("紙の硬さ")).not.toBeNull();
    expect(screen.getByLabelText("膨らみの強さ")).not.toBeNull();
    expect(screen.getByText("面の分割の細かさは4までに丸めました")).not.toBeNull();
    expect(screen.queryByText(/硬さで紙の曲がりやすさ/)).toBeNull();

    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(useAppStore.getState().paperHelpExpanded).toBe(true);
    expect(screen.getByText(/硬さで紙の曲がりやすさ/)).not.toBeNull();
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
  it("切替と3つの基準が出て、初期値は紙の縦の中心線", () => {
    render(<PaperAppearance />);
    const box = screen.getByLabelText("左右対称に描く");
    expect(box).toHaveProperty("checked", false);
    expect(box.getAttribute("data-tooltip")).toContain("片側に線を引くだけ");
    expect(box.getAttribute("data-tooltip")).toContain("消す・線種変更もそろえます");
    expect(box.getAttribute("data-tooltip")).toContain(
      "現在の基準: 紙の縦の中心線",
    );
    expect(box.hasAttribute("title")).toBe(false);

    const group = screen.getByRole("group", { name: "基準にする線" });
    expect(within(group).getAllByRole("button")).toHaveLength(3);
    const vertical = within(group).getByRole("button", { name: "紙の縦の中心線" });
    const horizontal = within(group).getByRole("button", { name: "紙の横の中心線" });
    const selected = within(group).getByRole("button", {
      name: "この線を基準にする",
    }) as HTMLButtonElement;
    expect(vertical.getAttribute("aria-pressed")).toBe("true");
    expect(horizontal.getAttribute("aria-pressed")).toBe("false");
    expect(selected.disabled).toBe(true);
    expect(selected.getAttribute("data-tooltip")).toContain(
      "展開図で折り線または補助線を1本選ぶと使えます",
    );
    expect(selected.getAttribute("data-tooltip")).toContain(
      "現在の基準: 紙の縦の中心線",
    );
    expect(selected.hasAttribute("title")).toBe(false);
    expect(selected.parentElement?.getAttribute("tabindex")).toBe("0");
    expect(screen.getByText("現在: 紙の縦の中心線")).not.toBeNull();
  });

  it("入れるとストアと切替の状態がその場で変わる", () => {
    render(<PaperAppearance />);
    const box = screen.getByLabelText("左右対称に描く");
    fireEvent.click(box);
    expect(useAppStore.getState().mirrorDraw).toBe(true);
    expect(box).toHaveProperty("checked", true);
  });

  it("紙の横の中心線へ切り替え、現在表示と端末設定を同時に更新する", () => {
    render(<PaperAppearance />);
    const horizontal = screen.getByRole("button", { name: "紙の横の中心線" });

    fireEvent.click(horizontal);

    expect(useAppStore.getState().mirrorAxis).toEqual({ kind: "paperHorizontal" });
    expect(horizontal.getAttribute("aria-pressed")).toBe("true");
    expect(
      screen.getByRole("button", { name: "紙の縦の中心線" }).getAttribute(
        "aria-pressed",
      ),
    ).toBe("false");
    expect(screen.getByText("現在: 紙の横の中心線")).not.toBeNull();
    expect(
      JSON.parse(globalThis.localStorage.getItem("origami3.prefs") ?? "{}"),
    ).toMatchObject({ mirrorAxis: "paperHorizontal" });
  });
});
