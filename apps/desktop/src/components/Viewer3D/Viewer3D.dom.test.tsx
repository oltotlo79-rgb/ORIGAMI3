// @vitest-environment jsdom
// 立体表示の画面テスト(常時ヒント・つかんで折る・実行前プレビュー)。
// WebGLはテスト環境に無いので、シーン(createScene)だけ差し替え、
// 三角形分割や当たり判定などの計算は本物を使う。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { Document, DocumentView, Face } from "../../lib/types";

const held = vi.hoisted(() => ({ scene: null as unknown as Record<string, unknown> }));

vi.mock("./sceneBuilder", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./sceneBuilder")>();
  const THREE = await import("three");
  return {
    ...actual,
    createScene: () => {
      // 紙(0..1の正方形)を真上から見るカメラ。画面中央が(0.5, 0.5)になる
      const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
      camera.position.set(0.5, 0.5, 2);
      camera.lookAt(0.5, 0.5, 0);
      camera.updateMatrixWorld(true);
      camera.updateProjectionMatrix();
      const scene = {
        camera,
        contentGroup: new THREE.Group(),
        highlightGroup: new THREE.Group(),
        content: null as unknown,
        render: vi.fn(),
        resize: vi.fn(),
        resetCamera: vi.fn(),
        setContent: vi.fn((c: unknown) => {
          scene.content = c;
        }),
        setHighlight: vi.fn(),
        setPreview: vi.fn(),
        setDrawMode: vi.fn(),
        dispose: vi.fn(),
      };
      held.scene = scene as unknown as Record<string, unknown>;
      return scene;
    },
  };
});

vi.mock("../../ipc/client", () => ({
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  documentSave: vi.fn(),
  editApply: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
}));

import * as ipc from "../../ipc/client";
import { useAppStore } from "../../store/appStore";
import { Viewer3D } from "./Viewer3D";

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
    edges: [],
    next_vertex_id: 4,
    next_edge_id: 4,
  },
  sequence: [],
  display: { front_color: [230, 90, 60], back_color: [245, 245, 245], grid_divisions: 8 },
};
const FACES: Face[] = [{ id: 0, vertices: [0, 1, 2, 3], edges: [10, 11, 12, 13] }];
const VIEW: DocumentView = {
  doc: DOC,
  faces: FACES,
  warnings: [],
  violations: [],
  frame: null,
  skipped: [],
};

/** 400×400pxのcanvasとして扱う(jsdomは実寸を持たないので固定する) */
function stubLayout() {
  Object.defineProperty(HTMLCanvasElement.prototype, "clientWidth", {
    configurable: true,
    value: 400,
  });
  Object.defineProperty(HTMLCanvasElement.prototype, "clientHeight", {
    configurable: true,
    value: 400,
  });
  HTMLCanvasElement.prototype.getBoundingClientRect = () =>
    ({ width: 400, height: 400, left: 0, top: 0, right: 400, bottom: 400, x: 0, y: 0 }) as DOMRect;
  Element.prototype.setPointerCapture = vi.fn();
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

function renderViewer() {
  const fitRef = { current: null } as React.RefObject<(() => void) | null>;
  render(<Viewer3D fitRef={fitRef} />);
  return document.querySelector("canvas")!;
}

describe("Viewer3D(画面)", () => {
  beforeEach(() => {
    stubLayout();
    vi.mocked(ipc.sequenceApply).mockReset();
    vi.mocked(ipc.sequenceApply).mockResolvedValue(VIEW);
    useAppStore.setState({
      doc: DOC,
      faces: FACES,
      hinges: new Set<number>(),
      frame3d: null,
      activeTool: "fold",
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      errorMessage: null,
      foldDraft: null,
      techniqueDraft: null,
    });
  });
  afterEach(() => cleanup());

  it("今できることを1行で常に出す(修飾キーの意味つき)", () => {
    renderViewer();
    const hint = screen.getByRole("status");
    expect(hint.textContent).toContain("紙をつかんでドラッグ");
    expect(hint.textContent).toContain("Shift");
    expect(hint.textContent).toContain("Alt");
  });

  it("折れない状態では理由を出す(操作は隠さない)", () => {
    useAppStore.setState({ playing: true });
    renderViewer();
    expect(screen.getByRole("status").textContent).toContain("再生中");
  });

  it("紙をドラッグすると折れる。途中では結果を半透明で下見できる", async () => {
    const canvas = renderViewer();
    fireEvent.pointerDown(canvas, { button: 0, pointerId: 1, clientX: 150, clientY: 200 });
    fireEvent.pointerMove(canvas, { pointerId: 1, clientX: 250, clientY: 200 });
    // ドラッグ中は折った結果の形が下見として渡される(まだ折っていない)
    const setPreview = held.scene.setPreview as ReturnType<typeof vi.fn>;
    const calls = setPreview.mock.calls;
    const preview = calls[calls.length - 1][0] as number[][][];
    expect(preview.length).toBeGreaterThan(0);
    expect(ipc.sequenceApply).not.toHaveBeenCalled();

    fireEvent.pointerUp(canvas, { button: 0, pointerId: 1, clientX: 250, clientY: 200 });
    await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalled());
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type).toBe("FoldThrough");
    if (op.type !== "FoldThrough") return;
    // 画面中央(x=0.5)に折り線が立ち、離した側が動かない側になる
    expect(op.line[0][0]).toBeCloseTo(0.5, 2);
    expect(op.keep_side_point[0]).toBeGreaterThan(0.5);
    expect(op.target_layers).toEqual([0]);
  });
});
