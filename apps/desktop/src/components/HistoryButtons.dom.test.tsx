// @vitest-environment jsdom
// 「元に戻す」「やり直し」の画面テスト。
// 折り角度の履歴と作品データの履歴のどちらに効くかが、ボタンの説明で分かること
// (設計原則3b)。実機で「角度を戻したつもりが折り目が消えた」ため入れた表示。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { HistoryButtons } from "./HistoryButtons";
import { useAppStore } from "../store/appStore";

afterEach(() => {
  cleanup();
  useAppStore.setState({
    angleUndoStack: [],
    angleRedoStack: [],
    docUndoDepth: 0,
  });
});

const undoButton = () => screen.getByRole("button", { name: "元に戻す" });
const redoButton = () => screen.getByRole("button", { name: "やり直し" });

describe("元に戻す/やり直しのボタン", () => {
  it("角度の履歴が無ければ、作品データを戻すと知らせる", () => {
    render(<HistoryButtons />);
    expect(undoButton().title).toContain("展開図・手順の変更を戻します");
    expect(redoButton().title).toContain("やり直せる操作はありません");
    expect(undoButton().title).toContain("(Ctrl+Z)");
    expect(redoButton().title).toContain("(Ctrl+Y)");
  });

  it("角度の履歴があれば、折り角度が戻ると知らせる", () => {
    useAppStore.setState({ angleUndoStack: [new Map([[5, 90]])] });
    render(<HistoryButtons />);
    expect(undoButton().title).toContain("折り角度の変更を戻します");
    expect(undoButton().title).toContain("折り線はそのまま残ります");
  });

  it("やり直しは作品データが先で、その後に折り角度と知らせる", () => {
    useAppStore.setState({
      docUndoDepth: 1,
      angleRedoStack: [new Map([[5, 90]])],
    });
    render(<HistoryButtons />);
    expect(redoButton().title).toContain("展開図・手順の変更をやり直します");

    cleanup();
    useAppStore.setState({ docUndoDepth: 0 });
    render(<HistoryButtons />);
    expect(redoButton().title).toBe("折り角度の変更をやり直します (Ctrl+Y)");
  });
});
