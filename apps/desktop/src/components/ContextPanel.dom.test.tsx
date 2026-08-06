// @vitest-environment jsdom
// 「この形で仕上げる」ボタン(SIM-009)の画面テスト。
// 押せないときもボタンは消さず、理由を日本語で見せることを確かめる。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ContextPanel } from "./ContextPanel";
import { useAppStore } from "../store/appStore";
import type { Document } from "../lib/types";

vi.mock("../ipc/client", () => ({
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
}));

import * as ipc from "../ipc/client";

/** 対角線(辺ID 5)が折り線の正方形 */
function makeDoc(): Document {
  return {
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
        { id: 5, v0: 0, v1: 2, kind: "Mountain" },
      ],
      next_vertex_id: 4,
      next_edge_id: 6,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

/** 折り線を1本選んだ状態にする(角度は呼び出し側で足す) */
function seed(drivers: Map<number, number>, poseAngles = new Map<number, number>()) {
  useAppStore.setState({
    doc: makeDoc(),
    faces: [
      { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
      { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
    ],
    hinges: new Set([5]),
    selection: { edgeIds: [5], vertexIds: [] },
    drivers,
    poseAngles,
    currentStep: null,
    playing: false,
    playT: 1,
    foldDraft: null,
    techniqueDraft: null,
    warnings: [],
    poseWarnings: [],
    replayWarnings: [],
    errorMessage: null,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  cleanup();
  useAppStore.setState({ doc: null, drivers: new Map(), poseAngles: new Map() });
});

describe("この形で仕上げる(SIM-009)", () => {
  it("角度が付いていなければ、ボタンは残したまま理由を見せる", () => {
    seed(new Map());
    render(<ContextPanel />);

    const button = screen.getByRole("button", { name: "この形で仕上げる" });
    expect(button).toHaveProperty("disabled", true);
    expect(screen.getAllByText(/まだ角度が付いていません/).length).toBeGreaterThan(0);
  });

  it("角度が付いていれば押せて、手順として送られる", async () => {
    seed(new Map([[5, 90]]));
    vi.mocked(ipc.sequenceApply).mockResolvedValue({
      doc: makeDoc(),
      faces: [],
      warnings: [],
      violations: [],
      frame: null,
      skipped: [],
    });
    render(<ContextPanel />);

    const button = screen.getByRole("button", { name: "この形で仕上げる" });
    expect(button).toHaveProperty("disabled", false);
    fireEvent.click(button);
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(vi.mocked(ipc.sequenceApply)).toHaveBeenCalledTimes(1);
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type === "PushStep" && op.step.kind).toBe("Pose");
  });
});
