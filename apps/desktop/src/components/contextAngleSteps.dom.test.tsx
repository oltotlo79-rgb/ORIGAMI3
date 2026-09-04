// @vitest-environment jsdom
// StepContent(手順を選んでいるときの内容)の技法名mirror検査(設計§6)。
// - 手順に記録されたtechnique_classificationがあれば見出しにその表示名を出す。
// - 項目が無ければ従来どおりkindのTECHNIQUE_LABELを出す。
// - 「折り方」selectでkindを明示的に選び直すとtechnique_classificationが落ちる。
// - NoteInput(注記)の変更ではtechnique_classificationを保持する。

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StepContent } from "./contextAngleSteps";
import { useAppStore } from "../store/appStore";
import type { Document, DocumentView, FoldStep } from "../lib/types";
import { DEFAULT_DISPLAY } from "../lib/displayPrefs";

vi.mock("../ipc/client", () => ({
  sequenceApply: vi.fn(),
}));

import * as ipc from "../ipc/client";

function step(overrides: Partial<FoldStep> = {}): FoldStep {
  return {
    id: 1,
    kind: "Simple",
    drivers: [],
    layer_order: null,
    note: "",
    ...overrides,
  };
}

function doc(steps: FoldStep[]): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: { vertices: [], edges: [], next_vertex_id: 0, next_edge_id: 0 },
    sequence: steps,
    display: DEFAULT_DISPLAY,
  };
}

/** applySequenceOpの先で参照される状態を、他のruntime分岐へ入らない値に固定する。 */
function seed(steps: FoldStep[]): void {
  useAppStore.setState({
    doc: doc(steps),
    skipped: [],
    replaySkipped: [],
    currentStep: null,
    foldAllPreview: null,
    drivers: new Map(),
    pinnedFolds: new Map(),
    frame3d: null,
    display: DEFAULT_DISPLAY,
  });
}

/** sequenceApplyの戻り値。sequence長を0にして、runtime側の追加ipc呼び出しを避ける
 * (ContextPanel.dom.test.tsxのresolvedView()と同じ最小形)。*/
function resolvedView(): DocumentView {
  return {
    doc: doc([]),
    faces: [],
    warnings: [],
    violations: [],
    frame: null,
    skipped: [],
    contact_detected: false,
  };
}

afterEach(() => {
  cleanup();
  useAppStore.setState({ doc: null, skipped: [], replaySkipped: [] });
  vi.mocked(ipc.sequenceApply).mockReset();
});

describe("StepContentの見出し(technique_classification mirror)", () => {
  it("分類があれば分類の表示名を出す", () => {
    seed([
      step({
        kind: "Simple",
        technique_classification: { kind: "Squash", origin: "Automatic" },
      }),
    ]);
    render(<StepContent number={1} />);
    expect(screen.getByText(/手順1: 開いてつぶす/)).toBeTruthy();
  });

  it("項目が無ければ従来どおりkindの表示名を出す", () => {
    seed([step({ kind: "Pleat" })]);
    render(<StepContent number={1} />);
    expect(screen.getByText(/手順1: 段折り/)).toBeTruthy();
  });
});

describe("「折り方」selectでのtechnique_classificationのクリア", () => {
  it("selectでkindを明示的に変えると、送るUpdateStepからtechnique_classificationが落ちる", async () => {
    seed([
      step({
        kind: "Simple",
        technique_classification: { kind: "Squash", origin: "Automatic" },
      }),
    ]);
    vi.mocked(ipc.sequenceApply).mockResolvedValue(resolvedView());
    render(<StepContent number={1} />);

    fireEvent.change(screen.getByLabelText("折り方"), {
      target: { value: "Pleat" },
    });

    await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(1));
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "UpdateStep") throw new Error("UpdateStepではない");
    expect(op.step.kind).toBe("Pleat");
    expect("technique_classification" in op.step).toBe(false);
  });

  it("項目が無い手順でselectを変えても項目を新設しない", async () => {
    seed([step({ kind: "Pleat" })]);
    vi.mocked(ipc.sequenceApply).mockResolvedValue(resolvedView());
    render(<StepContent number={1} />);

    fireEvent.change(screen.getByLabelText("折り方"), {
      target: { value: "Squash" },
    });

    await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(1));
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "UpdateStep") throw new Error("UpdateStepではない");
    expect(op.step.kind).toBe("Squash");
    expect("technique_classification" in op.step).toBe(false);
  });
});

describe("NoteInputでの変更はtechnique_classificationを保持する", () => {
  it("注記だけを変えて確定すると、送るUpdateStepのtechnique_classificationがそのまま残る", async () => {
    const classification = {
      kind: "Squash" as const,
      origin: "Automatic" as const,
    };
    seed([
      step({ kind: "Simple", technique_classification: classification }),
    ]);
    vi.mocked(ipc.sequenceApply).mockResolvedValue(resolvedView());
    render(<StepContent number={1} />);

    const noteInput = screen.getByPlaceholderText(
      "この手順の覚え書き(Enterで確定)",
    );
    fireEvent.change(noteInput, { target: { value: "テスト注記" } });
    fireEvent.keyDown(noteInput, { key: "Enter" });

    await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(1));
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    if (op.type !== "UpdateStep") throw new Error("UpdateStepではない");
    expect(op.step.note).toBe("テスト注記");
    expect(op.step.technique_classification).toEqual(classification);
  });
});
