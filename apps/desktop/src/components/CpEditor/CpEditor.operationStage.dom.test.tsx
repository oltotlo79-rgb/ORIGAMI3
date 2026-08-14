// @vitest-environment jsdom
// 展開図での実クリックと、下部の操作手順の現在位置が同じ進行を示すことを確かめる。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type React from "react";
import { DEFAULT_CONSTRUCT, type ConstructKind } from "../../lib/construct";
import { DEFAULT_CURVE } from "../../lib/curve";
import type { Document, Vec2 } from "../../lib/types";

vi.mock("./renderer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./renderer")>();
  return { ...actual, render: vi.fn() };
});

vi.mock("../../ipc/client", () => ({}));

import { OperationSteps } from "../OperationSteps";
import { useAppStore, type ToolId } from "../../store/appStore";
import { CpEditor } from "./CpEditor";

const initialStoreState = useAppStore.getState();

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

function renderEditorWithGuide(
  tool: ToolId,
  constructKind: ConstructKind = DEFAULT_CONSTRUCT.kind,
): HTMLCanvasElement {
  useAppStore.setState({
    doc: DOC,
    currentStep: null,
    faces: [],
    frame3d: null,
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: tool,
    operationStage: 0,
    contextHelpExpanded: true,
    violations: [],
    suspectHinges: [],
    activeAngleIntent: null,
    pendingFoldThrough: null,
    alignDraft: null,
    foldDraft: null,
    construct: { ...DEFAULT_CONSTRUCT, kind: constructKind },
    curve: { ...DEFAULT_CURVE, enabled: false },
    drawSegment: vi.fn(async () => {}),
    applyEdit: vi.fn(async () => {}),
  });
  const fitRef = { current: null } as React.RefObject<(() => void) | null>;
  const view = render(
    <>
      <CpEditor fitRef={fitRef} />
      <OperationSteps />
    </>,
  );
  const canvas = view.container.querySelector("canvas");
  if (!canvas) throw new Error("展開図のcanvasがない");
  return canvas;
}

async function expectCurrentStage(stage: 0 | 1 | 2): Promise<void> {
  await waitFor(() => {
    expect(useAppStore.getState().operationStage).toBe(stage);
    expect(
      screen.getAllByRole("listitem").map((item) => item.getAttribute("aria-current")),
    ).toEqual([stage === 0 ? "step" : null, stage === 1 ? "step" : null, stage === 2 ? "step" : null]);
  });
}

beforeEach(() => {
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
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("D15 直線入力と下部の進み方", () => {
  it.each([
    "mountain",
    "valley",
    "aux",
  ] as const)("%sの2クリックが1→2→完成と進む", async (tool) => {
    const canvas = renderEditorWithGuide(tool);
    await expectCurrentStage(0);

    pointerClick(canvas, [110, 200]);
    await expectCurrentStage(1);

    pointerClick(canvas, [290, 200]);
    await expectCurrentStage(2);
  });
});

describe("D15 複数入力作図と下部の進み方", () => {
  it.each<{
    kind: ConstructKind;
    clicks: Vec2[];
  }>([
    {
      kind: "bisector",
      clicks: [
        [110, 290],
        [200, 200],
        [290, 290],
      ],
    },
    {
      kind: "perpendicular",
      clicks: [
        [200, 380],
        [200, 200],
      ],
    },
    {
      kind: "divide",
      clicks: [
        [110, 200],
        [290, 200],
      ],
    },
  ])("$kindは入力途中を2、完了を3として強調する", async ({ kind, clicks }) => {
    const canvas = renderEditorWithGuide("construct", kind);
    await expectCurrentStage(0);

    for (const [index, point] of clicks.entries()) {
      pointerClick(canvas, point);
      await expectCurrentStage(index === clicks.length - 1 ? 2 : 1);
    }
  });
});
