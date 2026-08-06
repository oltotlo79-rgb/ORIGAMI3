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

describe("引くツールの左右同時の切替(UI-007)", () => {
  it("引くツールを選ぶと切替が出て、押すと設定が変わる", () => {
    seed(new Map());
    // 何も選んでいない状態で「引く」ツールにする(常設UIは増やさない)
    useAppStore.setState({
      activeTool: "pull",
      selection: { edgeIds: [], vertexIds: [] },
      pullMirror: true,
    });
    render(<ContextPanel />);

    const box = screen.getByLabelText("左右対称に動かす") as HTMLInputElement;
    expect(box.checked).toBe(true); // 既定はオン(作品はほとんど左右対称なので)
    expect(screen.getAllByText(/鶴の両羽が一緒に開きます/).length).toBe(1);

    fireEvent.click(box);
    expect(useAppStore.getState().pullMirror).toBe(false);
    expect(screen.getAllByText(/つかんだ側の折り線だけが動きます/).length).toBe(1);
  });

  it("他のツールでは出さない(下部パネルの内容を増やしすぎない)", () => {
    seed(new Map());
    useAppStore.setState({
      activeTool: "select",
      selection: { edgeIds: [], vertexIds: [] },
    });
    render(<ContextPanel />);
    expect(screen.queryByLabelText("左右対称に動かす")).toBeNull();
  });
});

describe("ねじり折りの中央多角形(TEC-009)", () => {
  /** ねじり折りを選び、角をcount個置いた状態にする */
  function seedTwist(count: number, center: [number, number] | null = null) {
    seed(new Map());
    const pts: [number, number][] = [
      [0.2, 0.2],
      [0.8, 0.2],
      [0.5, 0.9],
      [0.3, 0.5],
    ];
    useAppStore.setState({
      activeTool: "technique",
      selection: { edgeIds: [], vertexIds: [] },
      techniqueDraft: {
        kind: "Twist",
        flap: [],
        line: null,
        movingSide: "right",
        widthMm: 10,
        polygon: pts.slice(0, count),
        center,
        twistDeg: 30,
        docEpoch: 0,
        stepCount: 0,
        upTo: 0,
      },
    });
  }

  it("角が足りないうちは、何をすればよいかを見せて適用できない", () => {
    seedTwist(2);
    render(<ContextPanel />);

    expect(screen.getAllByText(/角を2個指定/).length).toBe(1);
    expect(screen.getAllByText(/あと3個以上必要/).length).toBe(1);
    expect(screen.getAllByText(/角を順にクリック/).length).toBeGreaterThan(0);
    const apply = screen.getByRole("button", { name: "適用" });
    expect(apply).toHaveProperty("disabled", true);
  });

  it("3つ以上そろえば、層を選ばなくても適用できる", () => {
    seedTwist(3);
    render(<ContextPanel />);

    expect(screen.getAllByText(/3角形/).length).toBe(1);
    expect(screen.getByRole("button", { name: "適用" })).toHaveProperty(
      "disabled",
      false,
    );
    // ねじる角は数値で決められる(既定30度)
    const deg = screen.getByLabelText("ねじる角(度)") as HTMLInputElement;
    expect(deg.value).toBe("30");
  });

  it("角を1つ戻す・中心を重心へ戻すが効く", () => {
    seedTwist(3, [0.4, 0.4]);
    render(<ContextPanel />);

    expect(screen.getAllByText(/中心は指定した点/).length).toBe(1);
    fireEvent.click(screen.getByRole("button", { name: "中心を重心へ戻す" }));
    expect(useAppStore.getState().techniqueDraft?.center).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "角を1つ戻す" }));
    expect(useAppStore.getState().techniqueDraft?.polygon).toHaveLength(2);
  });
});
