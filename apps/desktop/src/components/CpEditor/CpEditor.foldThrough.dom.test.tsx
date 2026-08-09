// @vitest-environment jsdom
// 巻き込み用の追加折り目が、RustのCP座標のまま2D描画へ渡ることを確かめる。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import type React from "react";
import type { Document } from "../../lib/types";

const held = vi.hoisted(() => ({ overlay: null as unknown }));

vi.mock("./renderer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./renderer")>();
  return {
    ...actual,
    render: vi.fn((...args: unknown[]) => {
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
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({ doc: null, pendingFoldThrough: null, suspectHinges: [] });
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
