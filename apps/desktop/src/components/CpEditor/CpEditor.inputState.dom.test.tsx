// @vitest-environment jsdom
// 作図・曲線の設定変更で、別の入力として扱うべき途中点が残らないことを確かめる。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type React from "react";
import {
  DEFAULT_CONSTRUCT,
  constructHint,
  type ConstructKind,
} from "../../lib/construct";
import { DEFAULT_CURVE, type CurveShape } from "../../lib/curve";
import type { Document, Vec2 } from "../../lib/types";

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

function overlay(): RenderOverlay {
  if (!held.overlay) throw new Error("描画オーバーレイがまだ作られていない");
  return held.overlay as RenderOverlay;
}

function renderEditor(): HTMLCanvasElement {
  const fitRef = { current: null } as React.RefObject<(() => void) | null>;
  const view = render(<CpEditor fitRef={fitRef} />);
  const canvas = view.container.querySelector("canvas");
  if (!canvas) throw new Error("展開図のcanvasがない");
  return canvas;
}

function pointerClick(canvas: HTMLCanvasElement, at: Vec2): void {
  fireEvent.pointerDown(canvas, {
    button: 0,
    pointerId: 1,
    clientX: at[0],
    clientY: at[1],
  });
  fireEvent.pointerUp(canvas, {
    button: 0,
    pointerId: 1,
    clientX: at[0],
    clientY: at[1],
  });
}

function pointerMove(canvas: HTMLCanvasElement, at: Vec2): void {
  fireEvent.pointerMove(canvas, {
    pointerId: 1,
    clientX: at[0],
    clientY: at[1],
  });
}

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
  Element.prototype.setPointerCapture = vi.fn();
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  useAppStore.setState({
    doc: DOC,
    currentStep: null,
    faces: [],
    frame3d: null,
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: "select",
    operationStage: 0,
    violations: [],
    suspectHinges: [],
    activeAngleIntent: null,
    pendingFoldThrough: null,
    alignDraft: null,
    foldDraft: null,
    construct: { ...DEFAULT_CONSTRUCT },
    curve: { ...DEFAULT_CURVE },
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    doc: null,
    currentStep: null,
    activeTool: "select",
    pendingFoldThrough: null,
    alignDraft: null,
    foldDraft: null,
    construct: { ...DEFAULT_CONSTRUCT },
    curve: { ...DEFAULT_CURVE },
  });
});

describe("D09 作図方式を変えたときの途中入力", () => {
  const pointInputSwitches: { from: ConstructKind; to: ConstructKind; points: Vec2[] }[] = [
    { from: "bisector", to: "perpendicular", points: [[110, 200], [200, 200]] },
    { from: "bisector", to: "divide", points: [[110, 200], [200, 200]] },
    { from: "bisector", to: "angle", points: [[110, 200], [200, 200]] },
    { from: "divide", to: "bisector", points: [[110, 200]] },
  ];

  it.each(pointInputSwitches)(
    "$fromで置いた点を$toへ引き継がず、残る途中入力は0件",
    async ({ from, to, points }) => {
      useAppStore.setState({
        activeTool: "construct",
        construct: { ...DEFAULT_CONSTRUCT, kind: from },
      });
      const canvas = renderEditor();
      for (const point of points) pointerClick(canvas, point);
      await waitFor(() => expect(overlay().constructPoints).toHaveLength(points.length));

      act(() => useAppStore.getState().setConstruct({ kind: to }));

      await waitFor(() => {
        expect(overlay().constructPoints).toHaveLength(0);
        expect(overlay().hint).toBe(constructHint(to, 0, DEFAULT_CONSTRUCT.divisions));
      });
    },
  );

  it("垂線で選んだ線を二等分へ引き継がず、残る途中入力は0件", async () => {
    useAppStore.setState({
      activeTool: "construct",
      construct: { ...DEFAULT_CONSTRUCT, kind: "perpendicular" },
    });
    const canvas = renderEditor();
    pointerClick(canvas, [200, 380]);
    await waitFor(() =>
      expect(overlay().hint).toBe(constructHint("perpendicular", 1, DEFAULT_CONSTRUCT.divisions)),
    );

    act(() => useAppStore.getState().setConstruct({ kind: "bisector" }));

    await waitFor(() => {
      expect(overlay().constructPoints).toHaveLength(0);
      expect(overlay().hint).toBe(
        constructHint("bisector", 0, DEFAULT_CONSTRUCT.divisions),
      );
    });
  });
});

describe("D10 曲線の入切・形を変えたときの途中入力", () => {
  const cases: {
    shape: CurveShape;
    points: Vec2[];
    change: "toggle" | "shape";
  }[] = [
    { shape: "arc", points: [[110, 290], [290, 290]], change: "toggle" },
    {
      shape: "bezier",
      points: [[110, 290], [290, 290], [155, 110]],
      change: "toggle",
    },
    { shape: "arc", points: [[110, 290], [290, 290]], change: "shape" },
    {
      shape: "bezier",
      points: [[110, 290], [290, 290], [155, 110]],
      change: "shape",
    },
  ];

  it.each(cases)(
    "$shapeの途中で$changeを変えると、再開後に残る途中入力は0件",
    async ({ shape, points, change }) => {
      useAppStore.setState({
        activeTool: "mountain",
        curve: { ...DEFAULT_CURVE, enabled: true, shape },
      });
      const canvas = renderEditor();
      for (const point of points) pointerClick(canvas, point);
      await waitFor(() => expect(overlay().constructPoints).toHaveLength(points.length));

      if (change === "toggle") {
        act(() => useAppStore.getState().setCurve({ enabled: false }));
        await waitFor(() => expect(overlay().constructPoints).toHaveLength(0));
        act(() => useAppStore.getState().setCurve({ enabled: true }));
      } else {
        act(() =>
          useAppStore.getState().setCurve({
            shape: shape === "arc" ? "bezier" : "arc",
          }),
        );
      }

      await waitFor(() => expect(overlay().constructPoints).toHaveLength(0));
    },
  );
});

describe("D11 作図中の吸着候補", () => {
  it("通常線と作図へ同じ緑丸用の吸着候補を渡す", async () => {
    useAppStore.setState({ activeTool: "mountain" });
    const canvas = renderEditor();
    // 紙中央の方眼交点から画面上2pxだけずらし、同じ候補へ吸着させる。
    pointerMove(canvas, [202, 200]);
    await waitFor(() =>
      expect(overlay().hoverSnap).toEqual({ pos: [0.5, 0.5], kind: "grid" }),
    );
    const lineSnap = overlay().hoverSnap;

    act(() => useAppStore.setState({ activeTool: "construct" }));
    pointerMove(canvas, [202, 200]);

    await waitFor(() => expect(overlay().hoverSnap).toEqual(lineSnap));
  });
});
