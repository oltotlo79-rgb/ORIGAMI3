// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { FirstRunGuide } from "./FirstRunGuide";
import { useAppStore, type GuideAction, type GuideStep } from "../store/appStore";
import { DEFAULT_DISPLAY } from "../lib/displayPrefs";
import { ONBOARDING_STORAGE_KEY } from "../lib/firstRunGuide";

const originalStoreState = useAppStore.getState();
const onboardingStorage = globalThis.localStorage as Partial<Storage> | undefined;
const originalOnboarding =
  typeof onboardingStorage?.getItem === "function"
    ? onboardingStorage.getItem(ONBOARDING_STORAGE_KEY)
    : null;

function showStep(step: GuideStep): void {
  act(() => {
    useAppStore.setState({ guideOpen: true, guideStep: step });
  });
}

afterEach(() => {
  cleanup();
  useAppStore.setState(originalStoreState, true);
  if (
    typeof onboardingStorage?.removeItem === "function" &&
    typeof onboardingStorage.setItem === "function"
  ) {
    if (originalOnboarding === null) {
      onboardingStorage.removeItem(ONBOARDING_STORAGE_KEY);
    } else {
      onboardingStorage.setItem(ONBOARDING_STORAGE_KEY, originalOnboarding);
    }
  }
});

describe("初回ガイド", () => {
  it("キャンバスを塞がない非モーダルの隅カードとして表示する", () => {
    showStep(0);
    render(<FirstRunGuide />);

    const guide = screen.getByLabelText("基本操作ガイド");
    expect(guide.tagName).toBe("ASIDE");
    expect(guide.getAttribute("aria-modal")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByText("やってみて")).toBeTruthy();
    expect(screen.getByText("操作できると自動で次へ進みます")).toBeTruthy();
  });

  it("4段階それぞれで実際に試す操作を日本語で案内する", () => {
    const pages: Array<[GuideStep, string, RegExp]> = [
      [0, "線を引いて折ってみよう", /Ctrl.*ドラッグ/],
      [1, "角度を変えてみよう", /折り角度/],
      [2, "紙を引いて動かそう", /紙をつかみ/],
      [3, "紙をふくらませよう", /膨らみの強さ/],
    ];

    showStep(0);
    render(<FirstRunGuide />);

    for (const [step, title, instruction] of pages) {
      showStep(step);
      expect(screen.getByRole("heading", { name: title })).toBeTruthy();
      expect(screen.getByText(instruction)).toBeTruthy();
      expect(screen.getByText(`${step + 1} / 4`)).toBeTruthy();
      expect(screen.getByLabelText(`ステップ${step + 1}`).getAttribute("aria-current"))
        .toBe("step");
    }
  });

  it("準備ボタンで各操作を始められる状態にする", () => {
    showStep(0);
    render(<FirstRunGuide />);

    fireEvent.click(screen.getByRole("button", { name: "「折る」ツールにする" }));
    expect(useAppStore.getState().activeTool).toBe("fold");

    showStep(1);
    fireEvent.click(
      screen.getByRole("button", { name: "折り線を選べる状態にする" }),
    );
    expect(useAppStore.getState().activeTool).toBe("select");

    showStep(2);
    fireEvent.click(screen.getByRole("button", { name: "「引く」ツールにする" }));
    expect(useAppStore.getState().activeTool).toBe("pull");

    act(() => {
      useAppStore.setState({
        selection: { edgeIds: [12], vertexIds: [7] },
        display: { ...DEFAULT_DISPLAY, soft_enabled: false },
      });
    });
    showStep(3);
    fireEvent.click(screen.getByRole("button", { name: "ふくらみ設定を表示" }));
    const state = useAppStore.getState();
    expect(state.activeTool).toBe("select");
    expect(state.selection).toEqual({ edgeIds: [], vertexIds: [] });
    expect(state.display.soft_enabled).toBe(true);
  });

  it("completeGuideActionはfold→angle→pull→inflateの順だけを受け付ける", () => {
    showStep(0);
    render(<FirstRunGuide />);

    const complete = (action: GuideAction) => {
      act(() => useAppStore.getState().completeGuideAction(action));
    };

    complete("angle");
    expect(useAppStore.getState().guideStep).toBe(0);

    complete("fold");
    expect(useAppStore.getState().guideStep).toBe(1);
    expect(screen.getByRole("heading", { name: "角度を変えてみよう" })).toBeTruthy();

    complete("pull");
    expect(useAppStore.getState().guideStep).toBe(1);

    complete("angle");
    expect(useAppStore.getState().guideStep).toBe(2);
    complete("pull");
    expect(useAppStore.getState().guideStep).toBe(3);
    complete("inflate");
    expect(useAppStore.getState().guideStep).toBe(4);
  });

  it("×でもスキップでもいつでも閉じられる", () => {
    showStep(0);
    render(<FirstRunGuide />);

    fireEvent.click(screen.getByRole("button", { name: "基本操作ガイドを閉じる" }));
    expect(useAppStore.getState().guideOpen).toBe(false);
    expect(screen.queryByLabelText("基本操作ガイド")).toBeNull();

    showStep(2);
    fireEvent.click(screen.getByRole("button", { name: "ガイドをスキップ" }));
    expect(useAppStore.getState().guideOpen).toBe(false);
    expect(screen.queryByLabelText("基本操作ガイド")).toBeNull();
  });

  it("4操作を終えると完了を伝え、作品づくりへ戻れる", () => {
    showStep(4);
    render(<FirstRunGuide />);

    expect(screen.getByLabelText("基本操作ガイド完了")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "できました！" })).toBeTruthy();
    expect(screen.getByText(/4つの基本操作をすべて試せました/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "作品づくりを続ける" }));
    expect(useAppStore.getState().guideOpen).toBe(false);
  });
});
