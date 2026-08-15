// @vitest-environment jsdom
// 紙をクリックしたときの案内が邪魔にならず、引く・ふくらますへ直接進めることを確かめる。
// 過去の手順を見ている間は、両方の入口が押せない状態になり理由が出ることも確かめる。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { PaperActionTip } from "./PaperActionTip";
import { useAppStore } from "../../store/appStore";
import type { Document } from "../../lib/types";

const initialStoreState = useAppStore.getState();

/** 対角線(辺4)で折る手順を2つ持つ正方形。手順1は「過去の手順」になる。 */
const DOC: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [1, 0] },
      { id: 2, pos: [1, 1] },
      { id: 3, pos: [0, 1] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Border" },
      { id: 2, v0: 2, v1: 3, kind: "Border" },
      { id: 3, v0: 3, v1: 0, kind: "Border" },
      { id: 4, v0: 0, v1: 2, kind: "Mountain" },
    ],
    next_vertex_id: 4,
    next_edge_id: 5,
  },
  sequence: [
    {
      id: 0,
      kind: "Simple",
      drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: 90 }],
      layer_order: null,
      note: "",
    },
    {
      id: 1,
      kind: "Simple",
      drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: 180 }],
      layer_order: null,
      note: "",
    },
  ],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

function seed(expanded = true, currentStep: number | null = null) {
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
    doc: DOC,
    hinges: new Set([4]),
    currentStep,
    playT: 1,
    playing: false,
  });
}

function pullButton(): HTMLButtonElement {
  return screen.getByRole("button", {
    name: /この紙を引いて動かす/,
  }) as HTMLButtonElement;
}

function inflateButton(): HTMLButtonElement {
  return screen.getByRole("button", {
    name: /この紙をふくらます/,
  }) as HTMLButtonElement;
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
    expect(pullButton()).toBeTruthy();
    expect(inflateButton()).toBeTruthy();
  });

  it("引く入口で引くツールへ切り替え、案内を閉じる", () => {
    seed();
    render(<PaperActionTip />);

    fireEvent.click(pullButton());

    const state = useAppStore.getState();
    expect(state.activeTool).toBe("pull");
    expect(state.paperActionTipVisible).toBe(false);
    expect(state.paperActionTipExpanded).toBe(false);
    expect(state.selection).toEqual({ edgeIds: [], vertexIds: [] });
  });

  it("ふくらます入口で選択を外し、たわみを有効にして小さい案内へ戻す", () => {
    seed();
    render(<PaperActionTip />);

    fireEvent.click(inflateButton());

    const state = useAppStore.getState();
    expect(state.activeTool).toBe("select");
    expect(state.selection).toEqual({ edgeIds: [], vertexIds: [] });
    expect(state.display.soft_enabled).toBe(true);
    expect(state.paperActionTipVisible).toBe(true);
    expect(state.paperActionTipExpanded).toBe(false);
    expect(screen.getByRole("button", { name: /この紙を動かす・ふくらます/ })).toBeTruthy();
  });

  it("最新の形では2つとも押せる", () => {
    seed();
    render(<PaperActionTip />);

    expect(pullButton().disabled).toBe(false);
    expect(inflateButton().disabled).toBe(false);
  });

  it("過去の手順を見ている間は2つとも押せず、理由が本文と吹き出しに出る", () => {
    seed(true, 1);
    render(<PaperActionTip />);

    expect(pullButton().disabled).toBe(true);
    expect(inflateButton().disabled).toBe(true);
    expect(pullButton().getAttribute("data-tooltip")).toContain(
      "前の手順の形を見ている間は引けません",
    );
    expect(inflateButton().getAttribute("data-tooltip")).toContain(
      "手順を選んでいる間は、ふくらます設定を開けません",
    );
    // 押せないボタンは吹き出しが出ないことがあるので、本文にも理由を出す
    expect(screen.getByText(/前の手順の形を見ている間は引けません/)).toBeTruthy();
    expect(
      screen.getByText(/手順を選んでいる間は、ふくらます設定を開けません/),
    ).toBeTruthy();
    expect(screen.getByText("今はこの紙を動かせません")).toBeTruthy();
  });

  it("押せない入口を押しても、ツールも丸みの設定も変わらない", () => {
    seed(true, 1);
    render(<PaperActionTip />);

    fireEvent.click(pullButton());
    fireEvent.click(inflateButton());

    const state = useAppStore.getState();
    expect(state.activeTool).toBe("select");
    expect(state.display.soft_enabled).toBe(false);
    expect(state.paperActionTipVisible).toBe(true);
    expect(state.paperActionTipExpanded).toBe(true);
  });

  it("再生中は引く入口だけが押せなくなる(丸みは手順を選んでいなければ開ける)", () => {
    seed();
    useAppStore.setState({ playing: true });
    render(<PaperActionTip />);

    expect(pullButton().disabled).toBe(true);
    expect(inflateButton().disabled).toBe(false);
    expect(screen.getByText("この紙、もっと動かせます！")).toBeTruthy();
  });
});
