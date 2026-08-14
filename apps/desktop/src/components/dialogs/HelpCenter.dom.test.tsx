// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { HelpCenter } from "./HelpCenter";
import { useAppStore } from "../../store/appStore";

const originalStoreState = useAppStore.getState();

function showHelp(): void {
  act(() => {
    useAppStore.setState({
      helpOpen: true,
      helpChapterId: "overview",
      helpQuery: "",
      guideOpen: false,
    });
  });
}

afterEach(() => {
  cleanup();
  useAppStore.setState(originalStoreState, true);
});

describe("ヘルプセンター", () => {
  it("閉じた状態からF1で開き、Escで閉じる", () => {
    act(() => useAppStore.setState({ helpOpen: false }));
    render(<HelpCenter />);
    expect(screen.queryByRole("dialog")).toBeNull();

    const f1 = new KeyboardEvent("keydown", { key: "F1", bubbles: true, cancelable: true });
    fireEvent(window, f1);
    expect(f1.defaultPrevented).toBe(true);
    expect(screen.getByRole("dialog", { name: "ヘルプセンター" })).toBeTruthy();
    expect(useAppStore.getState().helpOpen).toBe(true);

    const escape = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    fireEvent(window, escape);
    expect(escape.defaultPrevented).toBe(true);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useAppStore.getState().helpOpen).toBe(false);
  });

  it("閉じるボタンでダイアログを閉じる", () => {
    showHelp();
    render(<HelpCenter />);

    fireEvent.click(screen.getByRole("button", { name: "ヘルプセンターを閉じる" }));

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useAppStore.getState().helpOpen).toBe(false);
  });

  it("目次から章を選ぶと本文と選択状態が変わる", () => {
    showHelp();
    render(<HelpCenter />);

    const timeline = screen.getByRole("button", { name: /手順の記録と再生/ });
    fireEvent.click(timeline);

    expect(useAppStore.getState().helpChapterId).toBe("timeline");
    expect(timeline.getAttribute("aria-current")).toBe("page");
    expect(screen.getByRole("heading", { name: "手順の記録と再生" })).toBeTruthy();
    expect(screen.getByText("途中へ新しい折りを挿入する")).toBeTruthy();
  });

  it("章題と本文の単純な文字列一致で目次と章表示を絞り込む", () => {
    showHelp();
    render(<HelpCenter />);
    const search = screen.getByRole("searchbox", { name: "章題・本文を検索" });

    fireEvent.change(search, { target: { value: "形から展開図" } });
    expect(screen.getByText("1章が見つかりました")).toBeTruthy();
    expect(screen.getByRole("button", { name: /形から展開図を提案/ })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "形から展開図を提案" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /画面の見かた/ })).toBeNull();

    fireEvent.change(search, { target: { value: "ベジェ曲線" } });
    expect(screen.getByText("1章が見つかりました")).toBeTruthy();
    expect(screen.getByRole("button", { name: /展開図に線を引く/ })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "展開図に線を引く" })).toBeTruthy();

    const clear = screen.getByRole("button", { name: "検索語を消す" });
    expect(clear.getAttribute("data-tooltip")).toBe("入力した検索語を消します");
    expect(clear.hasAttribute("title")).toBe(false);
    clear.focus();
    expect(document.activeElement).toBe(clear);
    fireEvent.click(clear);
    expect((search as HTMLInputElement).value).toBe("");
    expect(document.activeElement).toBe(search);
    expect(screen.queryByRole("button", { name: "検索語を消す" })).toBeNull();
    expect(screen.getByText("全13章")).toBeTruthy();
    expect(screen.getByRole("button", { name: /画面の見かた/ })).toBeTruthy();
  });

  it("ヘルプ内から基本操作ガイドを最初から開ける", () => {
    showHelp();
    act(() => useAppStore.setState({ guideStep: 3 }));
    render(<HelpCenter />);

    fireEvent.click(screen.getByRole("button", { name: "基本操作ガイドをもう一度" }));

    const state = useAppStore.getState();
    expect(state.helpOpen).toBe(false);
    expect(state.guideOpen).toBe(true);
    expect(state.guideStep).toBe(0);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
