// @vitest-environment jsdom
// 展開図をキーボードだけで選び、矢印とEnterで実際のストア経路へ線を追加できることを確かめる。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type React from "react";
import { DEFAULT_CONSTRUCT } from "../../lib/construct";
import { DEFAULT_CURVE } from "../../lib/curve";
import type { Document, DocumentView, EdgeKind, EditOp } from "../../lib/types";

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

vi.mock("../../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editApplyBatch: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
}));

import * as ipc from "../../ipc/client";
import { useAppStore, type ToolId } from "../../store/appStore";
import { CpEditor } from "./CpEditor";
import type { RenderOverlay } from "./renderer";

const initialStoreState = useAppStore.getState();

const BASE_DOCUMENT: Document = {
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

function viewOf(doc: Document): DocumentView {
  return {
    doc: structuredClone(doc),
    faces: [],
    warnings: [],
    violations: [],
    frame: null,
    skipped: [],
    contact_detected: false,
  };
}

function documentWithDiagonal(kind: EdgeKind): Document {
  const doc = structuredClone(BASE_DOCUMENT);
  doc.cp.edges.push({ id: 4, v0: 0, v1: 2, kind });
  doc.cp.next_edge_id = 5;
  return doc;
}

function overlay(): RenderOverlay {
  if (!held.overlay) throw new Error("描画オーバーレイがまだ作られていない");
  return held.overlay as RenderOverlay;
}

function renderEditor(tool: ToolId, document = BASE_DOCUMENT): HTMLCanvasElement {
  useAppStore.setState({
    doc: structuredClone(document),
    currentStep: null,
    stepCreases: [],
    faces: [],
    frame3d: null,
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: tool,
    operationStage: 0,
    lineInputStart: null,
    contextHelpExpanded: true,
    violations: [],
    suspectHinges: [],
    activeAngleIntent: null,
    pendingFoldThrough: null,
    alignDraft: null,
    foldDraft: null,
    construct: { ...DEFAULT_CONSTRUCT },
    curve: { ...DEFAULT_CURVE, enabled: false },
    mirrorDraw: false,
    errorMessage: null,
    angleUndoStack: [],
    angleRedoStack: [],
    docUndoDepth: 0,
  });
  const fitRef = { current: null } as React.RefObject<(() => void) | null>;
  const view = render(<CpEditor fitRef={fitRef} />);
  const canvas = view.container.querySelector("canvas");
  if (!canvas) throw new Error("展開図のcanvasがない");
  return canvas;
}

function focusCanvas(canvas: HTMLCanvasElement): void {
  expect(canvas.tabIndex).toBe(0);
  canvas.focus();
  expect(document.activeElement).toBe(canvas);
}

function moveMany(
  canvas: HTMLCanvasElement,
  key: "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown",
  count: number,
): void {
  for (let index = 0; index < count; index += 1) {
    fireEvent.keyDown(canvas, { key, shiftKey: true });
  }
  fireEvent.keyUp(canvas, { key: "Shift" });
}

function noPointerEvents(canvas: HTMLCanvasElement) {
  const pointerdown = vi.fn();
  const mousedown = vi.fn();
  const click = vi.fn();
  canvas.addEventListener("pointerdown", pointerdown);
  canvas.addEventListener("mousedown", mousedown);
  canvas.addEventListener("click", click);
  return { pointerdown, mousedown, click };
}

beforeEach(() => {
  held.overlay = null;
  vi.clearAllMocks();
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

describe("展開図で線を引くキーボード経路", () => {
  it.each([
    ["mountain", "Mountain"],
    ["valley", "Valley"],
    ["aux", "Aux"],
  ] as const)(
    "%sはpointer・mouse・clickを0回のまま、矢印とEnter 2回で実際の作品へ1本足す",
    async (tool, kind) => {
      const expectedDocument = documentWithDiagonal(kind);
      vi.mocked(ipc.editApply).mockImplementation(async (op: EditOp) => {
        if (op.type !== "AddSegment") throw new Error("AddSegment以外が送られた");
        return viewOf(expectedDocument);
      });
      const canvas = renderEditor(tool);
      const events = noPointerEvents(canvas);

      expect(canvas.getAttribute("aria-label")).toContain("矢印キーで位置を動かし");
      focusCanvas(canvas);
      await waitFor(() => expect(overlay().keyboardCursor).toEqual([0.5, 0.5]));

      moveMany(canvas, "ArrowLeft", 4);
      moveMany(canvas, "ArrowDown", 4);
      await waitFor(() => expect(overlay().keyboardCursor).toEqual([0, 0]));
      fireEvent.keyDown(canvas, { key: "Enter" });
      expect(useAppStore.getState().lineInputStart).toEqual([0, 0]);
      expect(useAppStore.getState().operationStage).toBe(1);

      moveMany(canvas, "ArrowRight", 4);
      moveMany(canvas, "ArrowUp", 4);
      await waitFor(() => {
        expect(overlay().keyboardCursor?.[0]).toBeCloseTo(1, 12);
        expect(overlay().keyboardCursor?.[1]).toBeCloseTo(1, 12);
      });
      fireEvent.keyDown(canvas, { key: "Enter" });

      await waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));
      const sent = vi.mocked(ipc.editApply).mock.calls[0][0];
      expect(sent.type).toBe("AddSegment");
      if (sent.type !== "AddSegment") throw new Error("AddSegment以外が送られた");
      expect(sent.a[0]).toBeCloseTo(0, 12);
      expect(sent.a[1]).toBeCloseTo(0, 12);
      expect(sent.b[0]).toBeCloseTo(1, 12);
      expect(sent.b[1]).toBeCloseTo(1, 12);
      expect(sent.kind).toBe(kind);
      await waitFor(() => expect(useAppStore.getState().doc).toEqual(expectedDocument));
      expect(useAppStore.getState().doc?.cp.edges).toHaveLength(5);
      expect(useAppStore.getState().lineInputStart).toBeNull();
      expect(useAppStore.getState().operationStage).toBe(2);
      expect(events.pointerdown).toHaveBeenCalledTimes(0);
      expect(events.mousedown).toHaveBeenCalledTimes(0);
      expect(events.click).toHaveBeenCalledTimes(0);
    },
  );

  it("始点の後にEscapeを押すと作品・辺・履歴を変えず、途中状態と進み方だけ最初へ戻す", async () => {
    const canvas = renderEditor("mountain");
    useAppStore.setState({ docUndoDepth: 2 });
    const events = noPointerEvents(canvas);
    const beforeDocument = structuredClone(useAppStore.getState().doc);
    const beforeEdges = useAppStore.getState().doc?.cp.edges.length;
    const beforeUndoDepth = useAppStore.getState().docUndoDepth;

    focusCanvas(canvas);
    moveMany(canvas, "ArrowLeft", 4);
    moveMany(canvas, "ArrowDown", 4);
    fireEvent.keyDown(canvas, { key: "Enter" });
    expect(useAppStore.getState().lineInputStart).toEqual([0, 0]);
    expect(useAppStore.getState().operationStage).toBe(1);

    fireEvent.keyDown(canvas, { key: "Escape" });

    await waitFor(() => expect(useAppStore.getState().lineInputStart).toBeNull());
    expect(useAppStore.getState().operationStage).toBe(0);
    expect(useAppStore.getState().doc).toEqual(beforeDocument);
    expect(useAppStore.getState().doc?.cp.edges).toHaveLength(beforeEdges ?? -1);
    expect(useAppStore.getState().docUndoDepth).toBe(beforeUndoDepth);
    expect(ipc.editApply).not.toHaveBeenCalled();
    expect(ipc.editApplyBatch).not.toHaveBeenCalled();
    expect(events.pointerdown).toHaveBeenCalledTimes(0);
    expect(events.mousedown).toHaveBeenCalledTimes(0);
    expect(events.click).toHaveBeenCalledTimes(0);
  });

  it("方向へ吸着している間は、一般のキーボード案内より吸着中の案内を優先する", async () => {
    const canvas = renderEditor("mountain");
    focusCanvas(canvas);
    moveMany(canvas, "ArrowLeft", 4);
    moveMany(canvas, "ArrowDown", 4);
    fireEvent.keyDown(canvas, { key: "Enter" });
    fireEvent.keyDown(canvas, { key: "ArrowRight" });
    fireEvent.keyDown(canvas, { key: "ArrowUp" });

    await waitFor(() => {
      expect([
        "二等分方向に吸着中(Shiftで解除)",
        "辺・折り目の延長方向に吸着中(Shiftで解除)",
      ]).toContain(overlay().hint);
      expect(overlay().hint).not.toBe(
        "矢印キーで終わりの位置を動かし、Enterで決めます。Escapeでやめます",
      );
      expect(overlay().directionGuide).not.toBeNull();
    });
  });

  it("別の作品になったら、選ばれたままでも古い現在位置と吸着表示を消す", async () => {
    const canvas = renderEditor("aux");
    focusCanvas(canvas);
    await waitFor(() => {
      expect(overlay().keyboardCursor).toEqual([0.5, 0.5]);
      expect(overlay().hoverSnap).not.toBeNull();
    });
    const nextEpoch = useAppStore.getState().docEpoch + 1;

    act(() => useAppStore.setState({ docEpoch: nextEpoch }));

    await waitFor(() => {
      expect(document.activeElement).toBe(canvas);
      expect(overlay().keyboardCursor).toBeNull();
      expect(overlay().hoverSnap).toBeNull();
      expect(overlay().directionGuide).toBeNull();
    });
  });

  it("本物の道具変更で線から選択へ移ると、始点と進み方を最初へ戻す", async () => {
    const canvas = renderEditor("mountain");
    focusCanvas(canvas);
    moveMany(canvas, "ArrowLeft", 4);
    moveMany(canvas, "ArrowDown", 4);
    fireEvent.keyDown(canvas, { key: "Enter" });
    expect(useAppStore.getState().lineInputStart).toEqual([0, 0]);
    expect(useAppStore.getState().operationStage).toBe(1);

    act(() => useAppStore.getState().setTool("select"));

    await waitFor(() => {
      expect(useAppStore.getState().activeTool).toBe("select");
      expect(useAppStore.getState().lineInputStart).toBeNull();
      expect(useAppStore.getState().operationStage).toBe(0);
    });
  });

  it("本物の元に戻すで作品の応答を適用すると、始点と進み方を最初へ戻す", async () => {
    const beforeUndo = documentWithDiagonal("Mountain");
    vi.mocked(ipc.editUndo).mockResolvedValue(viewOf(BASE_DOCUMENT));
    const canvas = renderEditor("mountain", beforeUndo);
    focusCanvas(canvas);
    moveMany(canvas, "ArrowLeft", 4);
    moveMany(canvas, "ArrowDown", 4);
    fireEvent.keyDown(canvas, { key: "Enter" });
    expect(useAppStore.getState().lineInputStart).toEqual([0, 0]);
    expect(useAppStore.getState().operationStage).toBe(1);
    expect(useAppStore.getState().doc?.cp.edges).toHaveLength(5);

    await act(async () => {
      await useAppStore.getState().undo();
    });

    expect(ipc.editUndo).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(useAppStore.getState().doc).toEqual(BASE_DOCUMENT);
      expect(useAppStore.getState().doc?.cp.edges).toHaveLength(4);
      expect(useAppStore.getState().lineInputStart).toBeNull();
      expect(useAppStore.getState().operationStage).toBe(0);
    });
  });
});
