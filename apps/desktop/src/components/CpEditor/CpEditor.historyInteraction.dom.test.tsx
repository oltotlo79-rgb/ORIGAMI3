// @vitest-environment jsdom
// 過去手順の展開図では、画面に見えている線・点だけを操作対象にする回帰検査。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type React from "react";
import { DEFAULT_CONSTRUCT } from "../../lib/construct";
import { DEFAULT_CURVE } from "../../lib/curve";
import type { Document, EditOp, Vec2 } from "../../lib/types";

const held = vi.hoisted(() => ({
  document: null as unknown,
  overlay: null as unknown,
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
import { fitView, worldToScreen, type RenderOverlay } from "./renderer";

const FUTURE_EDGE_ID = 4;
const FUTURE_VERTEX_IDS = [4, 5];
const FUTURE_LINE_MIDDLE: Vec2 = [0.37, 0.5];

/** 手順1で初めて使う内部線だけが、最終作品に存在する。 */
const HISTORY_DOCUMENT: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [1, 0] },
      { id: 2, pos: [1, 1] },
      { id: 3, pos: [0, 1] },
      { id: 4, pos: [0.37, 0.2] },
      { id: 5, pos: [0.37, 0.8] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Border" },
      { id: 2, v0: 2, v1: 3, kind: "Border" },
      { id: 3, v0: 3, v1: 0, kind: "Border" },
      { id: FUTURE_EDGE_ID, v0: 4, v1: 5, kind: "Mountain" },
    ],
    next_vertex_id: 6,
    next_edge_id: 5,
  },
  sequence: [
    {
      id: 0,
      kind: "Simple",
      drivers: [{ a: [0.37, 0.2], b: [0.37, 0.8], target_angle_deg: 180 }],
      layer_order: null,
      note: "",
    },
  ],
  // 方眼候補を無くし、将来線そのものへの吸着だけを見分ける。
  display: { front_color: [237, 28, 36], back_color: [255, 255, 255], grid_divisions: 0 },
};

const ORIGINAL_APPLY_EDIT = useAppStore.getState().applyEdit;
const VIEW = fitView(HISTORY_DOCUMENT, 400, 400);

function screenPoint(world: Vec2): Vec2 {
  return worldToScreen(VIEW, world);
}

function pointerClick(canvas: HTMLCanvasElement, world: Vec2): void {
  const [clientX, clientY] = screenPoint(world);
  fireEvent.pointerDown(canvas, {
    button: 0,
    pointerId: 1,
    clientX,
    clientY,
  });
  fireEvent.pointerUp(canvas, {
    button: 0,
    pointerId: 1,
    clientX,
    clientY,
  });
}

async function renderEditor(): Promise<HTMLCanvasElement> {
  const fitRef = { current: null } as React.RefObject<(() => void) | null>;
  const view = render(<CpEditor fitRef={fitRef} />);
  const canvas = view.container.querySelector("canvas");
  if (!canvas) throw new Error("展開図のcanvasがない");

  await waitFor(() => {
    const drawn = held.document as Document | null;
    expect(drawn).not.toBeNull();
    // 検査対象の線が手順0の描画から消えていることを先に確かめる。
    expect(drawn?.cp.edges.some((edge) => edge.id === FUTURE_EDGE_ID)).toBe(false);
  });
  return canvas;
}

beforeEach(() => {
  held.document = null;
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
    doc: HISTORY_DOCUMENT,
    currentStep: 0,
    playT: 1,
    playing: false,
    faces: [],
    frame3d: null,
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: "select",
    violations: [FUTURE_VERTEX_IDS[0]],
    construct: { ...DEFAULT_CONSTRUCT, kind: "perpendicular" },
    curve: { ...DEFAULT_CURVE, enabled: false },
    alignDraft: null,
    foldDraft: null,
    pendingFoldThrough: null,
    operationStage: 0,
    applyEdit: ORIGINAL_APPLY_EDIT,
  });
});

afterEach(() => {
  cleanup();
  // 各検査で差し替え得るstore actionを必ず本物へ戻す。
  useAppStore.setState({
    applyEdit: ORIGINAL_APPLY_EDIT,
    doc: null,
    currentStep: null,
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: "select",
    violations: [],
    alignDraft: null,
    foldDraft: null,
    pendingFoldThrough: null,
  });
});

describe("D06 過去手順の展開図で見えない将来要素を操作しない", () => {
  it("選択: 見えない将来線をクリックしても選択しない", async () => {
    useAppStore.setState({ activeTool: "select" });
    const canvas = await renderEditor();

    pointerClick(canvas, FUTURE_LINE_MIDDLE);

    expect(useAppStore.getState().selection).toEqual({ edgeIds: [], vertexIds: [] });
  });

  it("削除: 見えない将来線を削除ツールでクリックしても編集を送らない", async () => {
    const applyEdit = vi
      .fn<(op: EditOp | EditOp[]) => Promise<void>>()
      .mockResolvedValue(undefined);
    useAppStore.setState({ activeTool: "delete", applyEdit });
    const canvas = await renderEditor();

    pointerClick(canvas, FUTURE_LINE_MIDDLE);

    expect(applyEdit).not.toHaveBeenCalled();
  });

  it("吸着: 見えない将来線へカーソルを吸着しない", async () => {
    useAppStore.setState({ activeTool: "mountain" });
    const canvas = await renderEditor();
    const [clientX, clientY] = screenPoint(FUTURE_LINE_MIDDLE);

    fireEvent.pointerMove(canvas, { pointerId: 1, clientX, clientY });

    const overlay = held.overlay as RenderOverlay;
    expect(overlay.hoverSnap).toBeNull();
  });

  it("作図: 見えない将来線を垂線の基準として使わない", async () => {
    const applyEdit = vi
      .fn<(op: EditOp | EditOp[]) => Promise<void>>()
      .mockResolvedValue(undefined);
    useAppStore.setState({ activeTool: "construct", applyEdit });
    const canvas = await renderEditor();

    pointerClick(canvas, FUTURE_LINE_MIDDLE);
    pointerClick(canvas, [0.65, 0.45]);

    expect(applyEdit).not.toHaveBeenCalled();
  });

  it("削除キー: 選択済みの将来線が見えない手順では削除しない", async () => {
    const applyEdit = vi
      .fn<(op: EditOp | EditOp[]) => Promise<void>>()
      .mockResolvedValue(undefined);
    useAppStore.setState({
      activeTool: "select",
      selection: { edgeIds: [FUTURE_EDGE_ID], vertexIds: [] },
      applyEdit,
    });
    await renderEditor();

    fireEvent.keyDown(window, { key: "Delete" });

    expect(applyEdit).not.toHaveBeenCalled();
  });
});

describe("D16 過去手順の違反表示", () => {
  // D16は次回に直す。
  it.fails("D16: 将来線だけに属する点と、その点の違反丸を手順0へ出さない", async () => {
    await renderEditor();

    const drawn = held.document as Document;
    const overlay = held.overlay as RenderOverlay;
    expect({
      vertexIds: drawn.cp.vertices.map((vertex) => vertex.id),
      violationIds: overlay.violations,
    }).toEqual({
      vertexIds: [0, 1, 2, 3],
      violationIds: [],
    });
  });
});
