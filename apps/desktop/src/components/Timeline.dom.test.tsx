// @vitest-environment jsdom
// 手順タイムラインの画面テスト: 手順の選択と、
// 「この手順の前に折りを挟む」導線(SEQ-006)。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Timeline } from "./Timeline";
import { useAppStore } from "../store/appStore";
import type { Document, FoldStep } from "../lib/types";
import { DEFAULT_DISPLAY } from "../lib/displayPrefs";

function step(id: number): FoldStep {
  return { id, kind: "Simple", drivers: [], layer_order: null, note: "" };
}

function doc(count: number): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: { vertices: [], edges: [], next_vertex_id: 0, next_edge_id: 0 },
    sequence: Array.from({ length: count }, (_, i) => step(i + 1)),
    display: DEFAULT_DISPLAY,
  };
}

afterEach(() => {
  cleanup();
  useAppStore.setState({ doc: null, currentStep: null, playing: false });
});

describe("手順タイムライン", () => {
  it("手順がなければ案内だけを出す", () => {
    useAppStore.setState({ doc: doc(0), currentStep: null });
    render(<Timeline />);
    expect(screen.getByText("まだ手順がありません")).toBeTruthy();
  });

  it("手順ごとに「前に挟む」導線があり、押すと1つ前の形を表示する", () => {
    useAppStore.setState({ doc: doc(3), currentStep: null });
    render(<Timeline />);

    // 手順の数だけ挿入用のボタンがある(手順1の前〜手順3の前)
    const inserts = Array.from(
      document.querySelectorAll<HTMLButtonElement>(
        '.timeline-insert[data-tooltip*="の前に新しい折りを挟みます"]',
      ),
    );
    expect(inserts.length).toBe(3);
    expect(inserts.every((button) => !button.hasAttribute("title"))).toBe(true);

    // 手順2の前を押すと「手順1まで折った形」が出る(そこで折ると手順2の前に入る)
    fireEvent.click(inserts[1]);
    expect(useAppStore.getState().currentStep).toBe(1);

    // 手順1の前なら折る前の形
    fireEvent.click(inserts[0]);
    expect(useAppStore.getState().currentStep).toBe(0);
  });

  it("手順そのものを押せばその手順まで折った形を表示する", () => {
    useAppStore.setState({ doc: doc(3), currentStep: null });
    render(<Timeline />);
    fireEvent.click(screen.getByRole("button", { name: /^2 / }));
    expect(useAppStore.getState().currentStep).toBe(2);
  });

  it("D28: 折る前では「前へ」を押せない見た目にする", () => {
    useAppStore.setState({ doc: doc(3), currentStep: 0 });
    render(<Timeline />);

    const previous = screen.getByRole("button", { name: "◀ 前へ" }) as HTMLButtonElement;
    const first = screen.getByRole("button", { name: "最初へ" }) as HTMLButtonElement;
    expect(first.disabled).toBe(true);
    expect(previous.disabled).toBe(true);
    expect(previous.getAttribute("data-tooltip")).toBe(
      "まだ折る前なので、これより前へは戻れません",
    );
  });

  it("D28: 最後の手順では「次へ」を押せない見た目にする", () => {
    useAppStore.setState({ doc: doc(3), currentStep: 3 });
    render(<Timeline />);

    const next = screen.getByRole("button", { name: "次へ ▶" }) as HTMLButtonElement;
    expect(next.disabled).toBe(true);
    expect(next.getAttribute("data-tooltip")).toBe(
      "いちばん最後の状態なので、これより先へは進めません",
    );

    cleanup();
    useAppStore.setState({ currentStep: null });
    render(<Timeline />);
    expect(
      (screen.getByRole("button", { name: "次へ ▶" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });
});
