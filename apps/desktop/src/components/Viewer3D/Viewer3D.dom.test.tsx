// @vitest-environment jsdom
// 立体表示の画面テスト(常時ヒント・つかんで折る・実行前プレビュー)。
// WebGLはテスト環境に無いので、シーン(createScene)だけ差し替え、
// 三角形分割や当たり判定などの計算は本物を使う。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import * as THREE from "three";
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
        soft: null as unknown,
        render: vi.fn(),
        syncTheme: vi.fn(),
        resize: vi.fn(),
        resetCamera: vi.fn(),
        setContent: vi.fn((c: unknown) => {
          scene.content = c;
        }),
        setHighlight: vi.fn(),
        setPreview: vi.fn(),
        setSoft: vi.fn((c: unknown) => {
          scene.soft = c;
        }),
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
    edges: [],
    next_vertex_id: 4,
    next_edge_id: 4,
  },
  sequence: [],
  display: { front_color: [230, 90, 60], back_color: [245, 245, 245], grid_divisions: 8 },
};
const FACES: Face[] = [{ id: 0, vertices: [0, 1, 2, 3], edges: [10, 11, 12, 13] }];
const EDGE_DOC: Document = {
  ...DOC,
  cp: {
    vertices: [
      ...DOC.cp.vertices,
      { id: 4, pos: [0.25, 0.1] },
      { id: 5, pos: [0.75, 0.1] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Border" },
      { id: 2, v0: 2, v1: 3, kind: "Border" },
      { id: 3, v0: 3, v1: 0, kind: "Border" },
      { id: 5, v0: 0, v1: 2, kind: "Mountain" },
      { id: 6, v0: 4, v1: 5, kind: "Aux" },
    ],
    next_vertex_id: 6,
    next_edge_id: 7,
  },
};
const EDGE_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
  { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
];
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
      pendingFoldThrough: null,
      foldThroughBusy: false,
      techniqueDraft: null,
      selection: { edgeIds: [], vertexIds: [] },
      uiTheme: "pop",
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

  it("テーマ変更時にCSS変数から3D背景を読み直す", () => {
    renderViewer();
    const syncTheme = held.scene.syncTheme as ReturnType<typeof vi.fn>;
    syncTheme.mockClear();
    act(() => useAppStore.getState().setUiTheme("classic"));
    expect(syncTheme).toHaveBeenCalledTimes(1);
  });

  it("巻き込みの追加折り目を、畳み平面の位置へ水色の参照線として出す", async () => {
    useAppStore.setState({
      pendingFoldThrough: {
        proposal: {
          folded_line: [
            [0.3, 0.2],
            [0.3, 0.8],
          ],
          crease_segments: [],
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
    });
    renderViewer();

    await waitFor(() => {
      const calls = (held.scene.setHighlight as ReturnType<typeof vi.fn>).mock.calls;
      const segments = calls[calls.length - 1]?.[0] as
        | { role?: string; a: THREE.Vector3; b: THREE.Vector3 }[]
        | undefined;
      expect(segments?.[0]?.role).toBe("reference");
      expect(segments?.[0]?.a.toArray()).toEqual([0.3, 0.2, 0.002]);
      expect(segments?.[0]?.b.toArray()).toEqual([0.3, 0.8, 0.002]);
    });
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
    await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(2));
    const op = vi.mocked(ipc.sequenceApply).mock.calls[1][0];
    expect(op.type).toBe("FoldThrough");
    if (op.type !== "FoldThrough") return;
    // 画面中央(x=0.5)に折り線が立ち、離した側が動かない側になる
    expect(op.line[0][0]).toBeCloseTo(0.5, 2);
    expect(op.keep_side_point[0]).toBeGreaterThan(0.5);
    expect(op.target_layers).toEqual([0]);
  });

  it("技法で紙をクリックすると候補層を保存し、初期対象は候補全部にする", () => {
    useAppStore.getState().beginTechnique("Squash");
    const canvas = renderViewer();

    fireEvent.pointerDown(canvas, {
      button: 0,
      pointerId: 1,
      clientX: 200,
      clientY: 200,
    });
    fireEvent.pointerUp(canvas, {
      button: 0,
      pointerId: 1,
      clientX: 200,
      clientY: 200,
    });

    expect(useAppStore.getState().techniqueDraft?.flapCandidates).toEqual([0]);
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual([0]);
  });

  // SIM-012: たわみは見た目だけの表現。当たり判定は剛体折りの多角形のままなので、
  // 細かい網を描いている間も折る・つかむ操作がそのまま使える
  it("たわみを入れても紙をつかんで折れる(当たり判定は元の面のまま)", async () => {
    useAppStore.setState({
      softMesh: {
        positions: [
          [0, 0, 0],
          [1, 0, 0],
          [1, 1, 0],
        ],
        triangles: [[0, 1, 2]],
        triangle_faces: [0],
        triangle_layers: [0],
        warnings: [],
      },
    });
    const canvas = renderViewer();
    // たわみの網は別の表示物として渡り、元の面(当たり判定に使う)は残っている
    expect(held.scene.soft).not.toBeNull();
    expect(held.scene.content).not.toBeNull();

    fireEvent.pointerDown(canvas, { button: 0, pointerId: 1, clientX: 150, clientY: 200 });
    fireEvent.pointerMove(canvas, { pointerId: 1, clientX: 250, clientY: 200 });
    fireEvent.pointerUp(canvas, { button: 0, pointerId: 1, clientX: 250, clientY: 200 });
    await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(2));
    expect(vi.mocked(ipc.sequenceApply).mock.calls[1][0].type).toBe("FoldThrough");
    useAppStore.setState({ softMesh: null });
  });
});

/** 対角線(辺5)で2つの面に分かれた正方形。面1が動く側 */
const PULL_DOC: Document = {
  ...DOC,
  cp: {
    ...DOC.cp,
    edges: [{ id: 5, v0: 0, v1: 2, kind: "Mountain" }],
  },
};
const PULL_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
  { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
];

describe("Viewer3D(紙をつかんで引く)", () => {
  beforeEach(() => {
    stubLayout();
    vi.mocked(ipc.poseSolve).mockReset();
    vi.mocked(ipc.poseSolve).mockResolvedValue({
      frame: { faces: [], warnings: [] },
      converged: true,
      angles: {},
      iterations: 1,
    });
    useAppStore.setState({
      doc: PULL_DOC,
      faces: PULL_FACES,
      hinges: new Set([5]),
      frame3d: null,
      activeTool: "pull",
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      poseAngles: new Map(),
      pullHinge: null,
      errorMessage: null,
      foldDraft: null,
      techniqueDraft: null,
    });
  });
  afterEach(() => cleanup());

  it("つじつまを合わせて全体が動くことを案内する", () => {
    renderViewer();
    expect(screen.getByRole("status").textContent).toContain("つじつま");
  });

  it("紙をドラッグすると、駆動する折り線の角度が追従計算へ送られる", async () => {
    const canvas = renderViewer();
    // 真上から見た視点では、画面の動きが紙の面内にしか向かず紙は起こせない。
    // 実際の初期視点と同じ斜め上から見る位置に置き直す
    const cam = held.scene.camera as THREE.PerspectiveCamera;
    cam.position.set(0.5, -1.6, 1.4);
    cam.lookAt(0.5, 0.5, 0);
    cam.updateMatrixWorld(true);
    cam.updateProjectionMatrix();
    // 対角線より上(面1)の点(0.2, 0.8)が画面のどこに写るかを求めてつかむ
    const ndc = new THREE.Vector3(0.2, 0.8, 0).project(cam);
    const sx = ((ndc.x + 1) / 2) * 400;
    const sy = ((1 - ndc.y) / 2) * 400;
    fireEvent.pointerDown(canvas, { button: 0, pointerId: 1, clientX: sx, clientY: sy });
    expect(useAppStore.getState().pullHinge).toBe(5); // 動かす折り線が決まる
    // 引き始めの実角は固定条件へ偽装せず、出発点のwarm seedとして送る。
    await waitFor(() => expect(ipc.poseSolve).toHaveBeenCalled());
    expect(vi.mocked(ipc.poseSolve).mock.calls[0][0]).toEqual([]);
    expect(vi.mocked(ipc.poseSolve).mock.calls[0][3]).toEqual([
      { hinge: 5, target_angle_deg: 0 },
    ]);

    // 画面の上へ引く = 斜め視点では紙を起こす向きになる
    fireEvent.pointerMove(canvas, { pointerId: 1, clientX: sx, clientY: sy - 60 });
    await waitFor(() => {
      const deg = useAppStore.getState().drivers.get(5);
      expect(deg).toBeDefined();
      expect(Math.abs(deg!)).toBeGreaterThan(1); // ドラッグ量が角度になった
    });

    fireEvent.pointerUp(canvas, { button: 0, pointerId: 1, clientX: sx, clientY: sy - 60 });
    expect(useAppStore.getState().pullHinge).toBeNull(); // 色付けは消える
    expect(useAppStore.getState().drivers.has(5)).toBe(true); // 形は残る
  });
});

describe("Viewer3D(指している場所のカーソル)", () => {
  beforeEach(() => {
    stubLayout();
    useAppStore.setState({
      doc: EDGE_DOC,
      faces: EDGE_FACES,
      hinges: new Set([5]),
      frame3d: null,
      activeTool: "select",
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      relaxations: [],
      activeAngleIntent: null,
      errorMessage: null,
      foldDraft: null,
      pendingFoldThrough: null,
      foldThroughBusy: false,
      alignDraft: null,
      techniqueDraft: null,
      selection: { edgeIds: [], vertexIds: [] },
      suspectHinges: [],
      paperActionTipVisible: false,
      paperActionTipExpanded: false,
    });
  });

  afterEach(() => {
    cleanup();
    useAppStore.setState(initialStoreState, true);
  });

  it("選べる場所はpointer、つかめる紙面はgrab、操作不可ならnot-allowedになる", async () => {
    const canvas = renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    // 選択モードでは、折り線も紙面もクリックできる対象としてpointerになる。
    fireEvent.pointerMove(canvas, { clientX: 200, clientY: 200 });
    expect(canvas.style.cursor).toBe("pointer");

    fireEvent.pointerMove(canvas, { clientX: 150, clientY: 200 });
    expect(canvas.style.cursor).toBe("pointer");

    // 折るモードの紙面は実際につかめるのでgrabになる。
    act(() => useAppStore.setState({ activeTool: "fold", playing: false }));
    fireEvent.pointerMove(canvas, { clientX: 150, clientY: 200 });
    expect(canvas.style.cursor).toBe("grab");

    act(() => useAppStore.setState({ activeTool: "fold", playing: true }));
    fireEvent.pointerMove(canvas, { clientX: 150, clientY: 200 });
    expect(canvas.style.cursor).toBe("not-allowed");
  });

  it("Ctrl+クリックでヒンジを既存選択へ追加し、もう一度で解除する", async () => {
    useAppStore.setState({ selection: { edgeIds: [0], vertexIds: [] } });
    const canvas = renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    const ctrlClick = () => {
      fireEvent.pointerDown(canvas, {
        button: 0,
        pointerId: 1,
        clientX: 200,
        clientY: 200,
        ctrlKey: true,
      });
      fireEvent.pointerUp(canvas, {
        button: 0,
        pointerId: 1,
        clientX: 200,
        clientY: 200,
        ctrlKey: true,
      });
    };

    ctrlClick();
    expect(useAppStore.getState().selection.edgeIds).toEqual([0, 5]);
    ctrlClick();
    expect(useAppStore.getState().selection.edgeIds).toEqual([0]);
  });

  it("スライダー行のホバー対象だけをfocus色の役割で3D強調する", async () => {
    useAppStore.setState({ selection: { edgeIds: [0, 5, 6], vertexIds: [] } });
    renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    act(() => useAppStore.getState().setHoveredHinge(5));
    await waitFor(() => {
      const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
      const calls = setHighlight.mock.calls;
      const last = calls[calls.length - 1][0] as { edgeId: number; role?: string }[];
      expect(last.find((segment) => segment.edgeId === 5)?.role).toBe("focus");
      expect(last.find((segment) => segment.edgeId === 0)?.role).toBe("reference");
    });
  });

  it("食い込み候補をsuspect役割で強調し、解消したら消す", async () => {
    useAppStore.setState({ suspectHinges: [5] });
    renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    await waitFor(() => {
      const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
      const calls = setHighlight.mock.calls;
      const last = calls[calls.length - 1]?.[0] as
        | { edgeId: number; role?: string }[]
        | undefined;
      expect(last?.find((segment) => segment.edgeId === 5)?.role).toBe("suspect");
    });

    act(() => useAppStore.setState({ suspectHinges: [] }));

    await waitFor(() => {
      const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
      const calls = setHighlight.mock.calls;
      const last = calls[calls.length - 1]?.[0] as
        | { edgeId: number; role?: string }[]
        | undefined;
      expect(last?.some((segment) => segment.role === "suspect")).toBe(false);
    });
  });

  it("追従診断は色付けせず、操作中は水色、食い込みは赤だけで示す", async () => {
    useAppStore.setState({
      relaxations: [
        { hinge: 5, target_angle_deg: 90, actual_angle_deg: 72, delta_deg: -18 },
      ],
    });
    renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    const lastHingeHighlights = () => {
      const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
      const calls = setHighlight.mock.calls;
      const last = calls[calls.length - 1]?.[0] as
        | { edgeId: number; role?: string; ownerFace?: number }[]
        | undefined;
      return (
        last
          ?.filter((segment) => segment.edgeId === 5)
          .map(({ edgeId, role, ownerFace }) => ({ edgeId, role, ownerFace })) ?? []
      );
    };

    await waitFor(() => expect(lastHingeHighlights()).toEqual([]));

    act(() =>
      useAppStore.setState({ activeAngleIntent: { generation: 4, hinges: [5], fixAll: true } }),
    );
    await waitFor(() =>
      expect(lastHingeHighlights()).toEqual([
        { edgeId: 5, role: "active", ownerFace: 0 },
        { edgeId: 5, role: "active", ownerFace: 1 },
      ]),
    );

    act(() => useAppStore.setState({ suspectHinges: [5] }));
    await waitFor(() =>
      expect(lastHingeHighlights()).toEqual([
        { edgeId: 5, role: "suspect", ownerFace: 0 },
        { edgeId: 5, role: "suspect", ownerFace: 1 },
      ]),
    );
  });

  it("合わせて折る途中は、いま選べる点の近くだけpointerになる", async () => {
    act(() => {
      useAppStore.setState({ activeTool: "fold" });
      useAppStore.getState().beginAlign("pointPoint");
    });
    const canvas = renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    fireEvent.pointerMove(canvas, { clientX: 200, clientY: 200 });
    expect(canvas.style.cursor).toBe("default");

    fireEvent.pointerMove(canvas, { clientX: 79, clientY: 321 });
    expect(canvas.style.cursor).toBe("pointer");
  });
});

describe("Viewer3D(ねじり折りの中央多角形を指す)", () => {
  /** ねじり折りを選んだ状態(角はまだ置いていない) */
  function seedTwist() {
    useAppStore.setState({
      doc: DOC,
      faces: FACES,
      hinges: new Set<number>(),
      frame3d: null,
      activeTool: "technique",
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      errorMessage: null,
      foldDraft: null,
      techniqueDraft: {
        kind: "Twist",
        flap: [],
        flapCandidates: [],
        flapPickCount: 1,
        line: null,
        movingSide: "right",
        widthMm: 10,
        polygon: [],
        center: null,
        referencePoint: null,
        twistDeg: 30,
        openToBack: false,
        motionMode: "reflect",
        motionTurn: "Keep",
        motionDirection: "Up",
        motionAnchor: 0,
        motionReverseLayers: false,
        motionAxisEdgeId: null,
        motionParts: [],
        docEpoch: 0,
        stepCount: 0,
        upTo: 0,
      },
    });
  }

  /** 紙の上を1回クリックする(動かさないのでクリック扱いになる) */
  function click(
    canvas: Element,
    x: number,
    y: number,
    ctrlKey = false,
    shiftKey = false,
  ) {
    fireEvent.pointerDown(canvas, {
      button: 0,
      pointerId: 1,
      clientX: x,
      clientY: y,
      ctrlKey,
      shiftKey,
    });
    fireEvent.pointerUp(canvas, {
      button: 0,
      pointerId: 1,
      clientX: x,
      clientY: y,
      ctrlKey,
      shiftKey,
    });
  }

  beforeEach(() => {
    stubLayout();
    seedTwist();
  });
  afterEach(() => cleanup());

  it("クリックのたびに角が増え、何をすればよいかが常に出る", () => {
    const canvas = renderViewer();
    expect(screen.getByRole("status").textContent).toContain("角を順にクリック");

    click(canvas, 170, 170);
    click(canvas, 230, 170);
    click(canvas, 200, 230);
    expect(useAppStore.getState().techniqueDraft?.polygon).toHaveLength(3);
    // 3つそろうと案内が「適用」へ変わり、下見の線分が渡される
    expect(screen.getByRole("status").textContent).toContain("3角形");
    const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
    const last = setHighlight.mock.calls[setHighlight.mock.calls.length - 1][0];
    expect((last as unknown[]).length).toBe(3 + 6); // 辺3本+頂点3つ×2本
  });

  it("Ctrl+クリックで中心を指せる。Backspaceで1つ戻り、Escでやめる", () => {
    const canvas = renderViewer();
    click(canvas, 170, 170);
    click(canvas, 230, 170);
    click(canvas, 200, 230, true); // 中心の指定(角は増えない)
    expect(useAppStore.getState().techniqueDraft?.polygon).toHaveLength(2);
    expect(useAppStore.getState().techniqueDraft?.center).not.toBeNull();

    fireEvent.keyDown(window, { key: "Backspace" });
    expect(useAppStore.getState().techniqueDraft?.polygon).toHaveLength(1);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(useAppStore.getState().techniqueDraft).toBeNull();
  });

  it("Shift+クリックで中央多角形を増やさず対象層を選べる", () => {
    const canvas = renderViewer();
    expect(screen.getByRole("status").textContent).toContain("Shift+クリック");

    click(canvas, 200, 200, false, true);

    expect(useAppStore.getState().techniqueDraft?.polygon).toEqual([]);
    expect(useAppStore.getState().techniqueDraft?.flapCandidates).toEqual([0]);
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual([0]);
  });

  it("ねじり以外はCtrl+クリックで任意の基準点を指せる", () => {
    const draft = useAppStore.getState().techniqueDraft;
    if (!draft) throw new Error("技法ドラフトがない");
    useAppStore.setState({ techniqueDraft: { ...draft, kind: "Swivel" } });
    const canvas = renderViewer();
    expect(screen.getByRole("status").textContent).toContain("Ctrl+クリック");

    click(canvas, 230, 200, true);

    const reference = useAppStore.getState().techniqueDraft?.referencePoint;
    expect(reference).not.toBeNull();
    expect(reference?.[0]).toBeGreaterThan(0.5);
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual([]);
    const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
    const last = setHighlight.mock.calls[setHighlight.mock.calls.length - 1][0];
    expect((last as unknown[]).length).toBe(2); // 基準点の十字
  });
});

/**
 * 合わせて折る(基準合わせ)の画面テスト。
 * カメラは紙(0..1の正方形)を真上から見ているので、画面の位置と紙の座標が対応する:
 * 中央(200,200)が(0.5,0.5)、左下の角(0,0)は約(79,321)、右上の角(1,1)は約(321,79)。
 */
describe("Viewer3D(合わせて折る)", () => {
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
      alignDraft: null,
      techniqueDraft: null,
    });
  });
  afterEach(() => cleanup());

  /** クリック(押して同じ場所で離す) */
  function click(canvas: Element, x: number, y: number) {
    fireEvent.pointerDown(canvas, { button: 0, pointerId: 1, clientX: x, clientY: y });
    fireEvent.pointerUp(canvas, { button: 0, pointerId: 1, clientX: x, clientY: y });
  }

  it("合わせモードでは、次に何を選べばよいかを常にヒントに出す", () => {
    useAppStore.getState().beginAlign("pointPoint");
    const canvas = renderViewer();
    expect(screen.getByRole("status").textContent).toContain("1つ目の点");

    click(canvas, 79, 321); // 紙の角(0,0)
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(1);
    expect(screen.getByRole("status").textContent).toContain("2つ目の点");
  });

  it("角を2つクリックすると、その垂直二等分線が折り線として求まる", () => {
    useAppStore.getState().beginAlign("pointPoint");
    const canvas = renderViewer();
    click(canvas, 79, 321); // (0,0)へ吸着
    click(canvas, 321, 79); // (1,1)へ吸着

    const picks = useAppStore.getState().alignDraft!.picks;
    expect(picks).toHaveLength(2);
    // クリックのずれに関わらず、紙の角ちょうどへ吸着している
    expect(picks[0]).toEqual({ kind: "point", p: [0, 0] });
    expect(picks[1]).toEqual({ kind: "point", p: [1, 1] });
    // 折り線は y = 1 - x
    for (const p of useAppStore.getState().foldDraft!.line) {
      expect(p[0] + p[1]).toBeCloseTo(1, 9);
    }
    expect(screen.getByRole("status").textContent).toContain("折り線が決まりました");
    // 選んだだけでは折らない(下のパネルで「折る」を押すまで待つ)
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
  });

  it("線と線を選ぶと、解が2つあることをヒントに出す", () => {
    useAppStore.getState().beginAlign("lineLine");
    const canvas = renderViewer();
    click(canvas, 200, 321); // 下の辺 (0,0)-(1,0)
    click(canvas, 79, 200); // 左の辺 (0,1)-(0,0)
    expect(useAppStore.getState().alignDraft?.solutions).toHaveLength(2);
    expect(screen.getByRole("status").textContent).toContain("解が2つ");
  });

  it("Backspaceで1つ戻す、Escでやめる", () => {
    useAppStore.getState().beginAlign("pointPoint");
    const canvas = renderViewer();
    click(canvas, 79, 321);
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(1);

    fireEvent.keyDown(window, { key: "Backspace" });
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(0);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(useAppStore.getState().alignDraft).toBeNull();
    // やめたら、いつもの「つかんで折る」案内へ戻る
    expect(screen.getByRole("status").textContent).toContain("紙をつかんでドラッグ");
  });
});

describe("Viewer3D(視点を戻す)", () => {
  beforeEach(() => {
    stubLayout();
    useAppStore.setState({
      doc: DOC,
      faces: FACES,
      hinges: new Set<number>(),
      frame3d: null,
      activeTool: "select",
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      relaxations: [],
      activeAngleIntent: null,
      errorMessage: null,
      foldDraft: null,
      alignDraft: null,
      techniqueDraft: null,
    });
  });

  it("2Dで選んだヒンジ・縁・補助線を3Dへ渡し、操作対象外は水色の役割に分ける", async () => {
    useAppStore.setState({
      doc: EDGE_DOC,
      faces: EDGE_FACES,
      hinges: new Set([5]),
      activeTool: "select",
      selection: { edgeIds: [0, 5, 6], vertexIds: [] },
    });
    renderViewer();

    await waitFor(() => {
      const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
      const calls = setHighlight.mock.calls;
      expect(calls.length).toBeGreaterThan(0);
      const last = calls[calls.length - 1][0] as {
        edgeId: number;
        role?: string;
        ownerFace?: number;
        surfaceProbe?: THREE.Vector3;
      }[];
      expect(
        last.map(({ edgeId, role, ownerFace }) => ({ edgeId, role, ownerFace })),
      ).toEqual([
        { edgeId: 0, role: "reference", ownerFace: 0 },
        { edgeId: 5, role: "hinge", ownerFace: 0 },
        { edgeId: 5, role: "hinge", ownerFace: 1 },
        { edgeId: 6, role: "reference", ownerFace: 0 },
      ]);
      expect(last.slice(0, 3).every((segment) => segment.surfaceProbe instanceof THREE.Vector3)).toBe(
        true,
      );
      // 面内Auxは境界triangleが一意でないため、無方向探索へは落とさない。
      expect(last[3].surfaceProbe).toBeUndefined();
    });
    expect(screen.getByRole("status").textContent).toBe(
      "3Dの紙を見回し、折り線や辺を選べます",
    );
    expect(screen.getByRole("status").textContent).not.toContain("水色");
  });
  afterEach(() => cleanup());

  it("3D区画に「視点を戻す」ボタンが常に出ている", () => {
    renderViewer();
    const button = screen.getByRole("button", { name: "視点を戻す" });
    expect(button.getAttribute("data-tooltip")).toBe(
      "3Dを紙全体が見える視点へ戻します",
    );
    expect(button.hasAttribute("title")).toBe(false);
  });

  it("押すと紙全体が見える初期の視点へ戻す", () => {
    renderViewer();
    const resetCamera = held.scene.resetCamera as ReturnType<typeof vi.fn>;
    resetCamera.mockClear(); // 表示直後の1回を数えない
    fireEvent.click(screen.getByRole("button", { name: "視点を戻す" }));
    expect(resetCamera).toHaveBeenCalledTimes(1);
    // 紙の大きさ(150×150mm → 正規化して1×1)で全体が入る位置を求める
    expect(resetCamera.mock.calls[0]).toEqual([1, 1]);
  });
});
