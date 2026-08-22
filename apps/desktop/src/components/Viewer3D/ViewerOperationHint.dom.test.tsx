// @vitest-environment jsdom
// 3D操作の吹き出しが、モード別の割り当てと開閉状態を正しく伝えるか確かめる。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { ViewerOperationHint } from "./ViewerOperationHint";
import { useAppStore, type ToolId } from "../../store/appStore";
import { SELECTABLE_3D_EDGE_TARGETS } from "../../lib/viewerHint";

const initialStoreState = useAppStore.getState();
function renderHint(tool: ToolId, expanded = true, blocked = false, aligning = false) {
  useAppStore.setState({ activeTool: tool, viewerHintExpanded: expanded });
  return render(
    <ViewerOperationHint
      hint="紙の上で操作できます"
      blocked={blocked}
      aligning={aligning}
    />,
  );
}

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("3Dビューの操作ヒント", () => {
  it.each<{
    tool: ToolId;
    mode: string;
    assignments: [string, string][];
  }>([
    {
      tool: "select",
      mode: "見る・選ぶ",
      assignments: [
        [
          "左クリック／ドラッグ",
          `${SELECTABLE_3D_EDGE_TARGETS}を選ぶ／視点を回す`,
        ],
        ["右ドラッグ", "視点を動かす"],
        ["ホイール", "拡大・縮小"],
      ],
    },
    {
      tool: "fold",
      mode: "折る",
      assignments: [
        ["左ドラッグ", "紙をつかんで折る"],
        ["右ドラッグ", "視点を動かす"],
        ["ホイール", "拡大・縮小"],
      ],
    },
    {
      tool: "pull",
      mode: "引く",
      assignments: [
        ["左ドラッグ", "紙を引く"],
        ["右ドラッグ", "視点を回す"],
        ["ホイール", "拡大・縮小"],
      ],
    },
    {
      tool: "technique",
      mode: "技法",
      assignments: [
        ["左クリック／ドラッグ", "層・角・基準点／折り線を選ぶ"],
        ["右ドラッグ", "視点を動かす"],
        ["ホイール", "拡大・縮小"],
      ],
    },
  ])("$modeモードのマウス割り当てを表示する", ({ tool, mode, assignments }) => {
    renderHint(tool);

    expect(screen.getByText(mode)).toBeTruthy();
    const group = screen.getByLabelText("マウス操作の割り当て");
    for (const [control, action] of assignments) {
      expect(within(group).getByText(control)).toBeTruthy();
      expect(within(group).getByText(action)).toBeTruthy();
    }
  });

  it("詳細を折りたたみ、同じボタンから再び開ける", () => {
    renderHint("fold");

    const close = screen.getByRole("button", { name: "詳しい3D操作方法を折りたたむ" });
    expect(close.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByLabelText("マウス操作の割り当て")).toBeTruthy();

    fireEvent.click(close);
    expect(useAppStore.getState().viewerHintExpanded).toBe(false);
    expect(screen.queryByLabelText("マウス操作の割り当て")).toBeNull();
    expect(screen.getByRole("status").textContent).toBe("紙の上で操作できます");
    const open = screen.getByRole("button", { name: "詳しい3D操作方法を開く" });
    expect(open.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(open);
    expect(useAppStore.getState().viewerHintExpanded).toBe(true);
    expect(screen.getByLabelText("マウス操作の割り当て")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "詳しい3D操作方法を折りたたむ" }).getAttribute(
        "aria-expanded",
      ),
    ).toBe("true");
  });

  it("現在の操作案内を一意のstatusとして常に残す", () => {
    renderHint("pull", false);

    const statuses = screen.getAllByRole("status");
    expect(statuses).toHaveLength(1);
    expect(statuses[0].textContent).toBe("紙の上で操作できます");
    expect(screen.queryByLabelText("マウス操作の割り当て")).toBeNull();
  });

  it("折れない間は実際の視点操作、合わせ中は選択と視点操作を案内する", () => {
    const { unmount } = renderHint("fold", true, true);
    const blocked = screen.getByLabelText("マウス操作の割り当て");
    expect(within(blocked).getByText("視点を回す")).toBeTruthy();
    expect(within(blocked).queryByText("紙をつかんで折る")).toBeNull();

    unmount();
    renderHint("fold", true, false, true);
    const aligning = screen.getByLabelText("マウス操作の割り当て");
    expect(within(aligning).getByText("左クリック／ドラッグ")).toBeTruthy();
    expect(
      within(aligning).getByText(
        `点・${SELECTABLE_3D_EDGE_TARGETS}の上で選ぶ／他の場所で視点を回す`,
      ),
    ).toBeTruthy();
  });

});
