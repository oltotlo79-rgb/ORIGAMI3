// @vitest-environment jsdom
// 選択中のツールに合う手順と、操作の進行に合う現在位置を確かめる。

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import { OperationSteps } from "./OperationSteps";
import { useAppStore, type ToolId } from "../store/appStore";

const initialStoreState = useAppStore.getState();

function seed(tool: ToolId, operationStage = 0) {
  useAppStore.setState({
    activeTool: tool,
    operationStage,
    selection: { edgeIds: [], vertexIds: [] },
    foldDraft: null,
    pendingFoldThrough: null,
    alignDraft: null,
    techniqueDraft: null,
  });
}

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("今できる操作の手順", () => {
  it.each<{
    tool: ToolId;
    title: string;
    steps: string[];
  }>([
    {
      tool: "select",
      title: "紙と折り線を選ぶ",
      steps: ["クリック（Ctrlで複数選択）", "下の角度を個別・一括で変える"],
    },
    {
      tool: "mountain",
      title: "山折り線を引く",
      steps: ["展開図で始点をクリック", "終点をクリック", "線が完成"],
    },
    {
      tool: "valley",
      title: "谷折り線を引く",
      steps: ["展開図で始点をクリック", "終点をクリック", "線が完成"],
    },
    {
      tool: "aux",
      title: "補助線を引く",
      steps: ["展開図で始点をクリック", "終点をクリック", "線が完成"],
    },
    {
      tool: "delete",
      title: "線を削除する",
      steps: ["消したい線にカーソルを合わせてクリック"],
    },
    {
      tool: "fold",
      title: "紙を折る",
      steps: ["3Dの紙をつかむ", "折りたい方へドラッグ", "離して折る"],
    },
    {
      tool: "pull",
      title: "紙を引いて動かす",
      steps: ["3Dの紙をつかむ", "動かしたい方へドラッグ", "離して形を残す"],
    },
    {
      tool: "technique",
      title: "技法で折る",
      steps: ["左で技法を選ぶ", "3Dで紙の層と折り線を選ぶ", "下の「適用」で折る"],
    },
    {
      tool: "construct",
      title: "作図の補助線を引く",
      steps: ["左で作図の種類を選ぶ", "展開図の点・線を順にクリック", "できた補助線を確認"],
    },
  ])("$toolでは「$title」の手順を表示する", ({ tool, title, steps }) => {
    seed(tool);
    render(<OperationSteps />);

    const guide = screen.getByRole("region", { name: `${title}の操作手順` });
    expect(within(guide).getByText(title)).toBeTruthy();
    for (const step of steps) {
      expect(within(guide).getByText(step)).toBeTruthy();
    }
    expect(within(guide).getAllByRole("listitem")).toHaveLength(steps.length);
  });

  it("操作が進むたびにaria-currentを次の手順へ移す", () => {
    seed("mountain");
    render(<OperationSteps />);

    const items = () => screen.getAllByRole("listitem");
    expect(items().map((item) => item.getAttribute("aria-current"))).toEqual([
      "step",
      null,
      null,
    ]);

    act(() => useAppStore.getState().setOperationStage(1));
    expect(items().map((item) => item.getAttribute("aria-current"))).toEqual([
      null,
      "step",
      null,
    ]);
    expect(items()[0].className).toContain("completed");

    act(() => useAppStore.getState().setOperationStage(2));
    expect(items().map((item) => item.getAttribute("aria-current"))).toEqual([
      null,
      null,
      "step",
    ]);
    expect(items()[1].className).toContain("completed");
  });
});
