// @vitest-environment jsdom
// 紙をクリックしたときの案内が邪魔にならず、引く・ふくらますへ直接進めることを確かめる。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { PaperActionTip } from "./PaperActionTip";
import { useAppStore } from "../../store/appStore";

const initialStoreState = useAppStore.getState();

function seed(expanded = true) {
  useAppStore.setState({
    activeTool: "select",
    paperActionTipVisible: true,
    paperActionTipExpanded: expanded,
    selection: { edgeIds: [5], vertexIds: [2] },
    display: {
      ...initialStoreState.display,
      soft_enabled: false,
      soft_pressure: 0,
    },
    doc: null,
  });
}

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("クリックした紙の操作案内", () => {
  it("大きい案内を小さくし、小さいヒントから再び開ける", () => {
    seed();
    render(<PaperActionTip />);

    expect(screen.getByLabelText("クリックした紙でできること")).toBeTruthy();
    expect(screen.getByText("この紙、もっと動かせます！")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "紙の操作案内を小さくする" }));
    expect(screen.queryByLabelText("クリックした紙でできること")).toBeNull();
    const compact = screen.getByRole("button", { name: /この紙を動かす・ふくらます/ });
    expect(compact).toBeTruthy();

    fireEvent.click(compact);
    expect(screen.getByLabelText("クリックした紙でできること")).toBeTruthy();
    expect(screen.getByRole("button", { name: /この紙を引いて動かす/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /この紙をふくらます/ })).toBeTruthy();
  });

  it("引く入口で引くツールへ切り替え、案内を閉じる", () => {
    seed();
    render(<PaperActionTip />);

    fireEvent.click(screen.getByRole("button", { name: /この紙を引いて動かす/ }));

    const state = useAppStore.getState();
    expect(state.activeTool).toBe("pull");
    expect(state.paperActionTipVisible).toBe(false);
    expect(state.paperActionTipExpanded).toBe(false);
    expect(state.selection).toEqual({ edgeIds: [], vertexIds: [] });
  });

  it("ふくらます入口で選択を外し、たわみを有効にして小さい案内へ戻す", () => {
    seed();
    render(<PaperActionTip />);

    fireEvent.click(screen.getByRole("button", { name: /この紙をふくらます/ }));

    const state = useAppStore.getState();
    expect(state.activeTool).toBe("select");
    expect(state.selection).toEqual({ edgeIds: [], vertexIds: [] });
    expect(state.display.soft_enabled).toBe(true);
    expect(state.paperActionTipVisible).toBe(true);
    expect(state.paperActionTipExpanded).toBe(false);
    expect(screen.getByRole("button", { name: /この紙を動かす・ふくらます/ })).toBeTruthy();
  });
});
