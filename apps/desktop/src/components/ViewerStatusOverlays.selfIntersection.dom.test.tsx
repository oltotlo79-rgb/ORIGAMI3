// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useAppStore } from "../store/appStore";
import { ViewerStatusOverlays } from "./ViewerStatusOverlays";

const initialStoreState = useAppStore.getState();

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("3Dビューのめり込み警告", () => {
  it.each(["折り鶴", "やっこさん"])(
    "%sで計算された0組からは専用バッジを出さない",
    () => {
      // 各作品が本当に0組になる契約はRustの実標本検査で固定し、ここでは
      // その計算済みwire値を画面が勝手に警告へ変えない境界を固定する。
      useAppStore.setState({
        selfIntersectionPairs: [],
        focusedSelfIntersectionPairIndex: 0,
      });

      render(<ViewerStatusOverlays />);

      expect(
        screen.queryByRole("button", { name: /紙のめり込み/ }),
      ).toBeNull();
    },
  );

  it("意図的な貫通で返った組数とFace IDを表示する", () => {
    useAppStore.setState({
      selfIntersectionPairs: [[0, 2]],
      focusedSelfIntersectionPairIndex: 0,
    });

    render(<ViewerStatusOverlays />);

    expect(
      screen.getByRole("button", {
        name: /紙のめり込み 1組（1\/1、Face ID 0 ↔ 2）/,
      }),
    ).toBeTruthy();
  });

  it("backendの決定順を保ったまま各面ペアを巡回する", () => {
    useAppStore.setState({
      selfIntersectionPairs: [
        [2, 5],
        [7, 9],
      ],
      focusedSelfIntersectionPairIndex: 0,
    });

    render(<ViewerStatusOverlays />);

    fireEvent.click(
      screen.getByRole("button", {
        name: /紙のめり込み 2組（1\/2、Face ID 2 ↔ 5）/,
      }),
    );
    expect(useAppStore.getState().focusedSelfIntersectionPairIndex).toBe(1);
    fireEvent.click(
      screen.getByRole("button", {
        name: /紙のめり込み 2組（2\/2、Face ID 7 ↔ 9）/,
      }),
    );
    expect(useAppStore.getState().focusedSelfIntersectionPairIndex).toBe(0);
    expect(
      screen.getByRole("button", {
        name: /紙のめり込み 2組（1\/2、Face ID 2 ↔ 5）/,
      }),
    ).toBeTruthy();
  });

  it("検出設定OFFなら古い面ペアが残っていてもバッジを出さない", () => {
    useAppStore.setState((state) => ({
      display: {
        ...state.display,
        penetration_prevention_enabled: false,
      },
      selfIntersectionPairs: [[0, 2]],
      focusedSelfIntersectionPairIndex: 0,
    }));

    render(<ViewerStatusOverlays />);

    expect(
      screen.queryByRole("button", { name: /紙のめり込み/ }),
    ).toBeNull();
  });
});
