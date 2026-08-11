// @vitest-environment jsdom
// 巻き込み用の追加折り目が、RustのCP座標のまま2D描画へ渡ることを確かめる。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import type React from "react";
import type { Document } from "../../lib/types";

const held = vi.hoisted(() => ({
  overlay: null as unknown,
  document: null as unknown,
}));

vi.mock("./renderer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./renderer")>();
  return {
    ...actual,
    render: vi.fn((...args: unknown[]) => {
      held.document = args[4];
      held.overlay = args[7];
    }),
  };
});

vi.mock("../../ipc/client", () => ({}));

import { useAppStore } from "../../store/appStore";
import { CpEditor } from "./CpEditor";
import type { RenderOverlay } from "./renderer";

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
    ],
    next_vertex_id: 4,
    next_edge_id: 4,
  },
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

beforeEach(() => {
  held.overlay = null;
  held.document = null;
  Object.defineProperty(HTMLCanvasElement.prototype, "clientWidth", {
    configurable: true,
    value: 400,
  });
  Object.defineProperty(HTMLCanvasElement.prototype, "clientHeight", {
    configurable: true,
    value: 400,
  });
  HTMLCanvasElement.prototype.getContext = vi.fn(() => ({})) as never;
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  useAppStore.setState({
    doc: DOC,
    faces: [],
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: "select",
    violations: [],
    suspectHinges: [],
    relaxations: [],
    activeAngleIntent: null,
    pendingFoldThrough: {
      proposal: {
        // 3D用の畳み平面座標とは違う値にし、取り違えを見つける。
        folded_line: [
          [0.2, 0.1],
          [0.2, 0.9],
        ],
        crease_segments: [
          [
            [0.65, 0.25],
            [0.65, 0.75],
          ],
        ],
        message: "追加折り目の候補です。",
      },
      operation: {
        type: "FoldThrough",
        up_to: 0,
        line: [
          [0.5, 0],
          [0.5, 1],
        ],
        keep_side_point: [0.25, 0.5],
        target_layers: null,
        direction: "Up",
      },
      docEpoch: useAppStore.getState().docEpoch,
      stepCount: 0,
    },
    foldThroughBusy: false,
    currentStep: null,
    playT: 1,
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    doc: null,
    pendingFoldThrough: null,
    suspectHinges: [],
    relaxations: [],
    activeAngleIntent: null,
    currentStep: null,
  });
});

describe("CpEditor 巻き込み折り目プレビュー", () => {
  it("展開図用のcrease_segmentsを描画オーバーレイへ渡す", async () => {
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    render(<CpEditor fitRef={fitRef} />);

    await waitFor(() => {
      const overlay = held.overlay as RenderOverlay | null;
      expect(overlay?.suggestedCreases).toEqual([
        [
          [0.65, 0.25],
          [0.65, 0.75],
        ],
      ]);
    });
  });
});

describe("CpEditor 食い込み候補の強調", () => {
  it("候補ヒンジを描画オーバーレイへ渡し、解消したら空にする", async () => {
    useAppStore.setState({ pendingFoldThrough: null, suspectHinges: [5] });
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    render(<CpEditor fitRef={fitRef} />);

    await waitFor(() => {
      const overlay = held.overlay as RenderOverlay | null;
      expect(overlay?.suspectHinges).toEqual([5]);
    });

    act(() => useAppStore.setState({ suspectHinges: [] }));

    await waitFor(() => {
      const overlay = held.overlay as RenderOverlay | null;
      expect(overlay?.suspectHinges).toEqual([]);
    });
  });
});

describe("CpEditor 自然追従の表示", () => {
  it("追従診断は描画へ渡さず、操作中と食い込み候補だけを強調する", async () => {
    useAppStore.setState({
      pendingFoldThrough: null,
      suspectHinges: [7],
      relaxations: [
        { hinge: 5, target_angle_deg: 90, actual_angle_deg: 72, delta_deg: -18 },
        { hinge: 6, target_angle_deg: 45, actual_angle_deg: 44.901, delta_deg: -0.099 },
      ],
      activeAngleIntent: { generation: 3, hinges: [9] },
    });
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    render(<CpEditor fitRef={fitRef} />);

    await waitFor(() => {
      const overlay = held.overlay as RenderOverlay | null;
      expect(overlay).not.toHaveProperty("relaxedHinges");
      expect(overlay?.activeHinges).toEqual([9]);
      expect(overlay?.suspectHinges).toEqual([7]);
    });
  });
});

describe("CpEditor 手順時点の展開図", () => {
  const HISTORY_DOC: Document = {
    ...DOC,
    cp: {
      ...DOC.cp,
      vertices: [
        ...DOC.cp.vertices,
        { id: 4, pos: [0.25, 0] },
        { id: 5, pos: [0.25, 1] },
        { id: 6, pos: [0.75, 0] },
        { id: 7, pos: [0.75, 1] },
      ],
      edges: [
        ...DOC.cp.edges,
        { id: 4, v0: 4, v1: 5, kind: "Mountain" },
        { id: 5, v0: 6, v1: 7, kind: "Valley" },
      ],
      next_vertex_id: 8,
      next_edge_id: 6,
    },
    sequence: [
      {
        id: 0,
        kind: "Simple",
        drivers: [{ a: [0.25, 0], b: [0.25, 1], target_angle_deg: 180 }],
        layer_order: null,
        note: "",
      },
      {
        id: 1,
        kind: "Simple",
        drivers: [{ a: [0.75, 0], b: [0.75, 1], target_angle_deg: -180 }],
        layer_order: null,
        note: "",
      },
    ],
  };

  it("戻る・進めるたびに、その手順までの折り線と現在番号を描く", async () => {
    useAppStore.setState({ doc: HISTORY_DOC, currentStep: 0 });
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    const screen = render(<CpEditor fitRef={fitRef} />);

    await waitFor(() => {
      const drawn = held.document as Document;
      expect(drawn.cp.edges.map((edge) => edge.id)).toEqual([0, 1, 2, 3]);
      expect(screen.getByText("手順 0 / 2")).toBeTruthy();
    });

    act(() => useAppStore.setState({ currentStep: 1 }));
    await waitFor(() => {
      const drawn = held.document as Document;
      expect(drawn.cp.edges.map((edge) => edge.id)).toEqual([0, 1, 2, 3, 4]);
      expect(screen.getByText("手順 1 / 2")).toBeTruthy();
    });

    act(() => useAppStore.setState({ currentStep: null }));
    await waitFor(() => {
      const drawn = held.document as Document;
      expect(drawn.cp.edges.map((edge) => edge.id)).toEqual([0, 1, 2, 3, 4, 5]);
      expect(screen.getByText("手順 2 / 2")).toBeTruthy();
    });

    act(() => useAppStore.setState({ currentStep: 0 }));
    await waitFor(() => {
      const drawn = held.document as Document;
      expect(drawn.cp.edges.map((edge) => edge.id)).toEqual([0, 1, 2, 3]);
      expect(screen.getByText("手順 0 / 2")).toBeTruthy();
    });
  });
});
