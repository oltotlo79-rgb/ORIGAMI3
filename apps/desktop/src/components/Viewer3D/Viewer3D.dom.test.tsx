// @vitest-environment jsdom
// 立体表示の画面テスト(常時ヒント・つかんで折る・実行前プレビュー)。
// WebGLはテスト環境に無いので、シーン(createScene)だけ差し替え、
// 三角形分割や当たり判定などの計算は本物を使う。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import * as THREE from "three";
import type { Document, DocumentView, Face, Frame3D } from "../../lib/types";

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
      type DisplayContent = {
        mesh: THREE.Mesh;
        owner: {
          triangleFaces: number[];
          triangleLayers: number[];
          faceSurfaceRanks: ReadonlyMap<number, number>;
        };
      };
      const pickSurfaceOf = (content: unknown) => {
        if (content === null) return null;
        const displayed = content as DisplayContent;
        return {
          mesh: displayed.mesh,
          triangleFaceIds: displayed.owner.triangleFaces,
          triangleLayers: displayed.owner.triangleLayers,
          faceSurfaceRanks: displayed.owner.faceSurfaceRanks,
        };
      };
      const scene = {
        camera,
        contentGroup: new THREE.Group(),
        highlightGroup: new THREE.Group(),
        content: null as unknown,
        soft: null as unknown,
        pickSurface: null as ReturnType<typeof pickSurfaceOf>,
        render: vi.fn(),
        syncTheme: vi.fn(),
        resize: vi.fn(),
        resetCamera: vi.fn(),
        setContent: vi.fn((c: unknown) => {
          scene.content = c;
          // rigid側はViewer3Dの既存fallbackも画面テストで通す。
          scene.pickSurface = null;
        }),
        setSupplementalEdges: vi.fn(),
        setHighlight: vi.fn(),
        setPreview: vi.fn(),
        setSoft: vi.fn((c: unknown) => {
          scene.soft = c;
          // たわみ表示中だけは、本番sceneと同じく表示中の曲面を表面判定へ渡す。
          scene.pickSurface = c === null ? null : pickSurfaceOf(c);
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
  editApplyBatch: vi.fn(),
  editUndo: vi.fn(),
  editRedo: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
}));

import * as ipc from "../../ipc/client";
import { useAppStore } from "../../store/appStore";
import { Viewer3D } from "./Viewer3D";
import {
  PICK_TOLERANCE_PX,
  pickEdge,
  pickVertex,
} from "../CpEditor/interaction";
import type { EditOp, Vec2 } from "../../lib/types";

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
      { id: 6, pos: [0.25, 0.9] },
      { id: 7, pos: [0.75, 0.9] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Border" },
      { id: 2, v0: 2, v1: 3, kind: "Border" },
      { id: 3, v0: 3, v1: 0, kind: "Border" },
      { id: 5, v0: 0, v1: 2, kind: "Mountain" },
      { id: 6, v0: 4, v1: 5, kind: "Aux" },
      { id: 7, v0: 6, v1: 7, kind: "Valley" },
    ],
    next_vertex_id: 8,
    next_edge_id: 8,
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
  contact_detected: false,
};

/** 中央ヒンジで右半分を90°起こした、実際のraycastを検査する形。 */
const SPATIAL_DOC: Document = {
  ...DOC,
  cp: {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [0.5, 0] },
      { id: 2, pos: [1, 0] },
      { id: 3, pos: [1, 1] },
      { id: 4, pos: [0.5, 1] },
      { id: 5, pos: [0, 1] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Border" },
      { id: 2, v0: 2, v1: 3, kind: "Border" },
      { id: 3, v0: 3, v1: 4, kind: "Border" },
      { id: 4, v0: 4, v1: 5, kind: "Border" },
      { id: 5, v0: 5, v1: 0, kind: "Border" },
      { id: 6, v0: 1, v1: 4, kind: "Valley" },
    ],
    next_vertex_id: 6,
    next_edge_id: 7,
  },
  sequence: [
    {
      id: 0,
      kind: "Pose",
      drivers: [{ a: [0.5, 0], b: [0.5, 1], target_angle_deg: 90 }],
      layer_order: null,
      note: "立体の形を残す",
    },
  ],
};
const SPATIAL_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 4, 5], edges: [0, 6, 4, 5] },
  { id: 1, vertices: [1, 2, 3, 4], edges: [1, 2, 3, 6] },
];
const SPATIAL_FRAME: Frame3D = {
  faces: [
    {
      face: 0,
      polygon: [
        [0, 0, 0],
        [0.5, 0, 0],
        [0.5, 1, 0],
        [0, 1, 0],
      ],
      layer: 0,
      surface_rank: 0,
    },
    {
      face: 1,
      polygon: [
        [0.5, 0, 0],
        [0.5, 0, 0.5],
        [0.5, 1, 0.5],
        [0.5, 1, 0],
      ],
      layer: 1,
      surface_rank: 1,
    },
  ],
  warnings: [],
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

/** 表示中の真上カメラで、世界座標を400×400 canvasのCSS pxへ直す。 */
function canvasPoint(world: [number, number, number]): { x: number; y: number } {
  const camera = held.scene.camera as THREE.PerspectiveCamera;
  const ndc = new THREE.Vector3(...world).project(camera);
  return { x: ((ndc.x + 1) / 2) * 400, y: ((1 - ndc.y) / 2) * 400 };
}

function clickCanvas(canvas: Element, world: [number, number, number], ctrlKey = false) {
  const { x, y } = canvasPoint(world);
  fireEvent.pointerDown(canvas, { button: 0, pointerId: 1, clientX: x, clientY: y, ctrlKey });
  fireEvent.pointerUp(canvas, { button: 0, pointerId: 1, clientX: x, clientY: y, ctrlKey });
}

describe("Viewer3D(画面)", () => {
  beforeEach(() => {
    stubLayout();
    vi.mocked(ipc.sequenceApply).mockReset();
    vi.mocked(ipc.sequenceApply).mockResolvedValue(VIEW);
    vi.mocked(ipc.poseSolve).mockResolvedValue({
      frame: { faces: [], warnings: [] },
      converged: true,
      angles: {},
      iterations: 1,
    });
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

  it("mountしたsceneとcanvas listenerをunmountで各1回だけ終了する", () => {
    const add = vi.spyOn(HTMLCanvasElement.prototype, "addEventListener");
    const remove = vi.spyOn(HTMLCanvasElement.prototype, "removeEventListener");
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    const view = render(<Viewer3D fitRef={fitRef} />);
    const scene = held.scene;
    const dispose = scene.dispose as ReturnType<typeof vi.fn>;
    const ownedAdds = add.mock.calls.filter(
      ([type]) => type === "pointermove" || type === "wheel",
    );

    expect(ownedAdds).toHaveLength(2);
    expect(dispose).not.toHaveBeenCalled();

    view.unmount();

    expect(dispose).toHaveBeenCalledTimes(1);
    for (const [type, callback] of ownedAdds) {
      expect(remove).toHaveBeenCalledWith(type, callback);
    }
    add.mockRestore();
    remove.mockRestore();
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

  it("0°の角度指定だけが残っていても、上部案内は折れないと言わない", () => {
    useAppStore.setState({
      doc: EDGE_DOC,
      faces: EDGE_FACES,
      hinges: new Set([5]),
      drivers: new Map([[5, 45]]),
    });
    renderViewer();

    // 0°以外を指定している間は、今の立体姿勢から折れないと正しく案内する。
    const hint = screen.getByRole("status");
    expect(hint.textContent).toContain("今は折れません");
    expect(hint.textContent).toContain("角度を動かして形を変えている間");

    // 数値欄で0°へ戻した直後と同じく、指定そのものはMapに残す。
    act(() => useAppStore.setState({ drivers: new Map([[5, 0]]) }));
    expect([...useAppStore.getState().drivers]).toEqual([[5, 0]]);

    expect(hint.textContent).not.toContain("今は折れません");
    expect(hint.textContent).not.toContain("角度を動かして形を変えている間");
    expect(hint.textContent).toContain("紙をつかんでドラッグ");
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

  it("90°起こした面では実際の3D当たり点と画面奥行きを保って折る", async () => {
    useAppStore.setState({
      doc: SPATIAL_DOC,
      faces: SPATIAL_FACES,
      hinges: new Set([6]),
      frame3d: SPATIAL_FRAME,
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      errorMessage: null,
      foldDraft: null,
      pendingFoldThrough: null,
      foldThroughBusy: false,
    });
    const canvas = renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    // 垂直面を正面寄りから見る。真上では面が線に潰れるため、実際の操作と同じ斜め視点にする。
    const camera = held.scene.camera as THREE.PerspectiveCamera;
    camera.position.set(-1, -1.2, 1.2);
    camera.lookAt(0.5, 0.5, 0.25);
    camera.updateMatrixWorld(true);
    camera.updateProjectionMatrix();

    const grabbed: [number, number, number] = [0.5, 0.45, 0.4];
    const start = canvasPoint(grabbed);
    const dx = 36;
    const dy = -28;
    const grabbedNdc = new THREE.Vector3(...grabbed).project(camera);
    const expectedTo = new THREE.Vector3(
      grabbedNdc.x + (dx * 2) / 400,
      grabbedNdc.y - (dy * 2) / 400,
      grabbedNdc.z,
    ).unproject(camera);

    fireEvent.pointerDown(canvas, {
      button: 0,
      pointerId: 1,
      clientX: start.x,
      clientY: start.y,
    });
    fireEvent.pointerMove(canvas, {
      pointerId: 1,
      clientX: start.x + dx,
      clientY: start.y + dy,
    });

    // z=0専用の半透明面へ潰さず、折り平面と反射後の動く輪郭を3Dで下見する。
    const highlightCalls = (held.scene.setHighlight as ReturnType<typeof vi.fn>).mock
      .calls;
    const previewSegments = highlightCalls[highlightCalls.length - 1]?.[0] as
      | { role?: string; a: THREE.Vector3; b: THREE.Vector3 }[]
      | undefined;
    expect(previewSegments?.some((segment) => segment.role === "reference")).toBe(true);
    expect(previewSegments?.some((segment) => segment.role === "active")).toBe(true);
    const currentPoints = SPATIAL_FRAME.faces.flatMap((face) =>
      face.polygon.map((point) => new THREE.Vector3(...point)),
    );
    expect(
      previewSegments
        ?.filter((segment) => segment.role === "active")
        .some((segment) =>
          [segment.a, segment.b].some(
            (point) =>
              Math.min(...currentPoints.map((current) => current.distanceTo(point))) > 1e-6,
          ),
        ),
    ).toBe(true);

    fireEvent.pointerUp(canvas, {
      button: 0,
      pointerId: 1,
      clientX: start.x + dx,
      clientY: start.y + dy,
    });

    await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(2));
    const op = vi.mocked(ipc.sequenceApply).mock.calls[1][0] as unknown as {
      type: string;
      direction?: "Up" | "Down";
      spatial?: {
        from: [number, number, number];
        to: [number, number, number];
        grab_face: number;
        mode: string;
      };
    };
    expect(op.type).toBe("FoldThrough");
    expect(op.spatial?.grab_face).toBe(1);
    expect(op.spatial?.mode).toBe("flap");
    for (let axis = 0; axis < 3; axis++) {
      expect(op.spatial?.from[axis]).toBeCloseTo(grabbed[axis], 7);
      expect(op.spatial?.to[axis]).toBeCloseTo(expectedTo.getComponent(axis), 7);
    }
    expect(Math.abs(op.spatial?.from[2] ?? 0)).toBeGreaterThan(0.1);
    // この面の材質表法線は-x。ドラッグ途中で表裏どちらへ動いたかを
    // 180°時の山谷分岐として保持する。
    const materialNormal = new THREE.Vector3(-1, 0, 0);
    const travel = expectedTo.clone().sub(new THREE.Vector3(...grabbed));
    expect(op.direction).toBe(materialNormal.dot(travel) > 0 ? "Up" : "Down");
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
          [0, 1, 0],
        ],
        triangles: [
          [0, 1, 2],
          [0, 2, 3],
        ],
        triangle_faces: [0, 0],
        triangle_layers: [0, 0],
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
      pinnedFolds: new Map(),
      foldAllPreview: null,
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

  it("Ctrl+クリックで補助線を既存選択へ追加し、もう一度で解除する", async () => {
    useAppStore.setState({ selection: { edgeIds: [0], vertexIds: [] } });
    const canvas = renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    // 基本outlineに入らない補助線でも、2Dと同じ複数選択規則を使う。
    const ctrlClick = () => clickCanvas(canvas, [0.5, 0.1, 0], true);

    ctrlClick();
    expect(useAppStore.getState().selection.edgeIds).toEqual([0, 6]);
    ctrlClick();
    expect(useAppStore.getState().selection.edgeIds).toEqual([0]);
  });

  it.each([
    ["山折り線", 5, [0.5, 0.5, 0]],
    ["谷折り線", 7, [0.5, 0.9, 0]],
    ["補助線", 6, [0.5, 0.1, 0]],
    ["紙の輪郭の辺", 0, [0.5, 0, 0]],
  ] as const)("3Dの%sをクリックすると2Dと共通のSelectionへ入る", async (_label, edgeId, point) => {
    const canvas = renderViewer();
    await waitFor(() => {
      const calls = (held.scene.setSupplementalEdges as ReturnType<typeof vi.fn>).mock.calls;
      expect(calls.length).toBeGreaterThan(0);
    });

    clickCanvas(canvas, [...point]);

    expect(useAppStore.getState().selection).toEqual({ edgeIds: [edgeId], vertexIds: [] });
  });

  it("たわみ表示中も、曲面へ投影した補助線をクリックして共通Selectionへ入れる", async () => {
    useAppStore.setState({
      softMesh: {
        positions: [
          [0, 0, 0],
          [1, 1, 0],
          [1, 0, 0.2],
          [0, 1, 0],
        ],
        triangles: [
          [0, 2, 1],
          [0, 1, 3],
        ],
        triangle_faces: [0, 1],
        triangle_layers: [0, 0],
        warnings: [],
      },
    });
    const canvas = renderViewer();
    let segment: { a: THREE.Vector3; b: THREE.Vector3 } | undefined;
    await waitFor(() => {
      const calls = (held.scene.setSupplementalEdges as ReturnType<typeof vi.fn>).mock.calls;
      const displayed = calls[calls.length - 1]?.[0] as
        | { edgeId: number; a: THREE.Vector3; b: THREE.Vector3 }[]
        | undefined;
      segment = displayed?.find(({ edgeId }) => edgeId === 6);
      expect(segment).toBeDefined();
      expect(segment?.a.z === 0 && segment.b.z === 0).toBe(false);
    });

    const midpoint = segment!.a.clone().add(segment!.b).multiplyScalar(0.5);
    clickCanvas(canvas, [midpoint.x, midpoint.y, midpoint.z]);

    expect(useAppStore.getState().selection).toEqual({ edgeIds: [6], vertexIds: [] });
  });

  it("紙面クリックは面をSelectionへ入れず、紙の直接操作案内だけを開く", async () => {
    useAppStore.setState({ selection: { edgeIds: [0], vertexIds: [] } });
    const canvas = renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    clickCanvas(canvas, [0.2, 0.5, 0]);

    expect(useAppStore.getState().selection).toEqual({ edgeIds: [], vertexIds: [] });
    expect(useAppStore.getState().paperActionTipVisible).toBe(true);
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

  it("固定した折り目は、選んでいなくてもpinned役割で3D強調する", async () => {
    // どれを固定したかが、選び直さなくても分かるようにする。
    useAppStore.setState({
      selection: { edgeIds: [], vertexIds: [] },
      pinnedFolds: new Map([[5, 45]]),
    });
    renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    await waitFor(() => {
      const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
      const calls = setHighlight.mock.calls;
      const last = calls[calls.length - 1]?.[0] as
        | { edgeId: number; role?: string }[]
        | undefined;
      expect(last?.find((segment) => segment.edgeId === 5)?.role).toBe("pinned");
    });

    act(() => useAppStore.setState({ pinnedFolds: new Map() }));

    await waitFor(() => {
      const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
      const calls = setHighlight.mock.calls;
      const last = calls[calls.length - 1]?.[0] as
        | { edgeId: number; role?: string }[]
        | undefined;
      expect(last?.some((segment) => segment.role === "pinned")).toBe(false);
    });
  });

  it("固定した折り目が食い込みの原因候補なら、赤い強調を優先する", async () => {
    useAppStore.setState({
      pinnedFolds: new Map([[5, 45]]),
      suspectHinges: [5],
    });
    renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    await waitFor(() => {
      const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
      const calls = setHighlight.mock.calls;
      const last = calls[calls.length - 1]?.[0] as
        | { edgeId: number; role?: string }[]
        | undefined;
      expect(last?.find((segment) => segment.edgeId === 5)?.role).toBe("suspect");
      expect(last?.some((segment) => segment.role === "pinned")).toBe(false);
    });
  });

  it("全部をいっぺんに動かす間は専用の案内を出し、通常形の固定色と赤線を隠す", async () => {
    useAppStore.setState({
      pinnedFolds: new Map([[5, 45]]),
      suspectHinges: [5],
      foldAllPreview: {
        session: 1,
        percent: 50,
        appliedPercent: 50,
        busy: false,
        returning: false,
        error: null,
        converged: true,
        bestEffort: false,
        relaxationCount: 0,
        flatFoldViolationCount: 0,
        suspectHingeCount: 0,
        contactDetected: false,
        layerOrder: "unavailable_without_sequence",
        nextWarmSeed: [],
        returnState: {
          docEpoch: useAppStore.getState().docEpoch,
          currentStep: null,
          playT: 1,
          activeTool: "select",
          selection: { edgeIds: [], vertexIds: [] },
        },
      },
    });
    renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    expect(
      screen.getByText("下の「折る割合」を動かすと、全部の折り目が同じ割合で動きます"),
    ).toBeTruthy();
    expect(held.scene.setDrawMode).toHaveBeenLastCalledWith(false, false);
    await waitFor(() => {
      const setHighlight = held.scene.setHighlight as ReturnType<typeof vi.fn>;
      const calls = setHighlight.mock.calls;
      const last = calls[calls.length - 1]?.[0] as
        | { edgeId: number; role?: string }[]
        | undefined;
      expect(last?.some((segment) => segment.role === "pinned")).toBe(false);
      expect(last?.some((segment) => segment.role === "suspect")).toBe(false);
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

describe("Viewer3D(層操作の開閉軸の案内)", () => {
  /** 層操作(Simple)を選び、既定の開閉モード(reflect)のまま何も選んでいない状態。 */
  function seedLayerMotion() {
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
        kind: "Simple",
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

  beforeEach(() => {
    stubLayout();
    seedLayerMotion();
  });
  afterEach(() => cleanup());

  it("案内は既にある折り目をクリックすると教え、ドラッグでは軸を決められると読めない", () => {
    const canvas = renderViewer();
    const tooltip = canvas.getAttribute("data-tooltip") ?? "";
    // 実装(appStore.ts)はドラッグで引いた線の折り目IDを空にし、既存の折り目の
    // クリックだけを開閉軸として受け付ける。案内文もそれに合わせている必要がある。
    expect(tooltip).toContain("既存の折り目をクリック");
    expect(tooltip).not.toMatch(/ドラッグ/);
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
      doc: EDGE_DOC,
      faces: EDGE_FACES,
      hinges: new Set<number>([5]),
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
    expect(canvas.getAttribute("data-tooltip")).toContain(
      "山折り線・谷折り線・補助線・紙の輪郭の辺",
    );

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
    expect(useAppStore.getState().alignDraft?.cpPicks).toEqual([
      { kind: "edge", id: 0 },
      { kind: "edge", id: 3 },
    ]);
    expect(screen.getByRole("status").textContent).toContain("解が2つ");
  });

  it("基本outlineに無い補助線も、edge ID付きで合わせ入力へ入る", async () => {
    useAppStore.getState().beginAlign("existingLine");
    const canvas = renderViewer();
    await waitFor(() => {
      const calls = (held.scene.setSupplementalEdges as ReturnType<typeof vi.fn>).mock.calls;
      expect(calls.length).toBeGreaterThan(0);
    });

    clickCanvas(canvas, [0.5, 0.1, 0]);

    expect(useAppStore.getState().alignDraft?.cpPicks).toEqual([{ kind: "edge", id: 6 }]);
    expect(useAppStore.getState().alignDraft?.picks[0]).toMatchObject({ kind: "line" });
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
    const canvas = renderViewer();

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
    const supplementalCalls = (
      held.scene.setSupplementalEdges as ReturnType<typeof vi.fn>
    ).mock.calls;
    const supplemental = supplementalCalls[supplementalCalls.length - 1][0] as {
      edgeId: number;
      ownerFace?: number;
    }[];
    // 選択済みAuxは水色強調へ回り、未選択の非ヒンジ谷線は同じ候補から黒線表示へ渡る。
    expect(supplemental.map(({ edgeId, ownerFace }) => ({ edgeId, ownerFace }))).toEqual([
      { edgeId: 7, ownerFace: 1 },
    ]);
    expect(screen.getByRole("status").textContent).toBe(
      "3Dの紙を見回し、点と山折り線・谷折り線・補助線・紙の輪郭の辺を選べます(点はCtrl+クリックで足す・外す、ドラッグで動かせます)",
    );
    expect(screen.getByRole("status").textContent).not.toContain("水色");
    expect(canvas.getAttribute("data-tooltip")).toContain(
      "山折り線・谷折り線・補助線・紙の輪郭の辺",
    );
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
    // まだ折っていない(frame3d: null)ので、立体の実測範囲は紙の大きさそのもの
    // (150×150mm → 正規化して1×1)と一致する。案内の札の下端も渡す
    // (jsdomでは要素の大きさが取れないので札の下端は0)。
    const [box, hint] = resetCamera.mock.calls[0] as [THREE.Box3, number];
    expect(box.min.toArray()).toEqual([0, 0, 0]);
    expect(box.max.toArray()).toEqual([1, 1, 0]);
    expect(hint).toBe(0);
  });

  it("折り上がった立体の実際の広がりを基準にする(展開図の大きさではない)", async () => {
    // 右半分を90°起こした状態(SPATIAL_FRAME)。展開図(平らな紙)は(0,0)〜(1,1)だが、
    // 実際に立っている立体の広がりはx:[0,0.5]・y:[0,1]・z:[0,0.5]で、
    // 展開図の大きさ・中心((0.5,0.5,0))とは一致しない。
    // 直す前はここで展開図の大きさをそのまま使っており、立体の一部が
    // 画面の外へ出ることがあった(scratchpad/check3d-new-plans-report.md §6)。
    useAppStore.setState({
      doc: SPATIAL_DOC,
      faces: SPATIAL_FACES,
      hinges: new Set([6]),
      frame3d: SPATIAL_FRAME,
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      errorMessage: null,
      foldDraft: null,
      alignDraft: null,
      techniqueDraft: null,
    });
    renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());

    const resetCamera = held.scene.resetCamera as ReturnType<typeof vi.fn>;
    resetCamera.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "視点を戻す" }));
    expect(resetCamera).toHaveBeenCalledTimes(1);
    const [box] = resetCamera.mock.calls[0] as [THREE.Box3, number];
    expect(box.min.toArray()).toEqual([0, 0, 0]);
    expect(box.max.toArray()).toEqual([0.5, 1, 0.5]);
  });
});

// ---------------------------------------------------------------------------
// 3Dから展開図の点を指す(選ぶ・動かす・線を引く・作図する)
// ---------------------------------------------------------------------------

/** 4枚の面・13個の点を持つ展開図。面の内側に落ちている点も混ぜてある。 */
const GRID_DOC: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [0.5, 0] },
      { id: 2, pos: [1, 0] },
      { id: 3, pos: [1, 0.5] },
      { id: 4, pos: [1, 1] },
      { id: 5, pos: [0.5, 1] },
      { id: 6, pos: [0, 1] },
      { id: 7, pos: [0, 0.5] },
      { id: 8, pos: [0.5, 0.5] },
      { id: 9, pos: [0.15, 0.25] },
      { id: 10, pos: [0.35, 0.25] },
      { id: 11, pos: [0.65, 0.75] },
      { id: 12, pos: [0.85, 0.75] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Border" },
      { id: 2, v0: 2, v1: 3, kind: "Border" },
      { id: 3, v0: 3, v1: 4, kind: "Border" },
      { id: 4, v0: 4, v1: 5, kind: "Border" },
      { id: 5, v0: 5, v1: 6, kind: "Border" },
      { id: 6, v0: 6, v1: 7, kind: "Border" },
      { id: 7, v0: 7, v1: 0, kind: "Border" },
      { id: 8, v0: 1, v1: 8, kind: "Valley" },
      { id: 9, v0: 3, v1: 8, kind: "Valley" },
      { id: 10, v0: 5, v1: 8, kind: "Valley" },
      { id: 11, v0: 7, v1: 8, kind: "Valley" },
      { id: 12, v0: 9, v1: 10, kind: "Aux" },
      { id: 13, v0: 11, v1: 12, kind: "Aux" },
    ],
    next_vertex_id: 13,
    next_edge_id: 14,
  },
  sequence: [],
  display: { front_color: [230, 90, 60], back_color: [245, 245, 245], grid_divisions: 8 },
};

const GRID_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 8, 7], edges: [0, 8, 11, 7] },
  { id: 1, vertices: [1, 2, 3, 8], edges: [1, 2, 9, 8] },
  { id: 2, vertices: [8, 3, 4, 5], edges: [9, 3, 4, 10] },
  { id: 3, vertices: [7, 8, 5, 6], edges: [11, 10, 5, 6] },
];

const GRID_VIEW: DocumentView = {
  doc: GRID_DOC,
  faces: GRID_FACES,
  warnings: [],
  violations: [],
  frame: null,
  skipped: [],
  contact_detected: false,
};

/** その道具の1クリック(押して同じ場所で離す)。 */
function clickAt(canvas: Element, at: { x: number; y: number }, ctrlKey = false) {
  fireEvent.pointerDown(canvas, {
    button: 0,
    pointerId: 1,
    clientX: at.x,
    clientY: at.y,
    ctrlKey,
  });
  fireEvent.pointerUp(canvas, {
    button: 0,
    pointerId: 1,
    clientX: at.x,
    clientY: at.y,
    ctrlKey,
  });
}

/** 展開図の点(平らな表示なので世界座標と同じ)を画面のpxへ。 */
function gridPoint(pos: Vec2, dx = 0, dy = 0) {
  const at = canvasPoint([pos[0], pos[1], 0]);
  return { x: at.x + dx, y: at.y + dy };
}

/** 直前に送られた展開図の編集要求(1件・まとめ送りのどちらも同じ形で受け取る)。 */
function lastEditOps(): EditOp[] {
  const single = vi.mocked(ipc.editApply).mock.calls;
  const batch = vi.mocked(ipc.editApplyBatch).mock.calls;
  if (batch.length > 0) return batch[batch.length - 1][0];
  if (single.length > 0) return [single[single.length - 1][0]];
  return [];
}

describe("Viewer3D(3Dから展開図の点を指す)", () => {
  beforeEach(() => {
    stubLayout();
    vi.mocked(ipc.editApply).mockReset();
    vi.mocked(ipc.editApplyBatch).mockReset();
    vi.mocked(ipc.editApply).mockResolvedValue(GRID_VIEW);
    vi.mocked(ipc.editApplyBatch).mockResolvedValue(GRID_VIEW);
    vi.mocked(ipc.sequenceApply).mockReset();
    vi.mocked(ipc.sequenceApply).mockResolvedValue(GRID_VIEW);
    useAppStore.setState({
      ...initialStoreState,
      doc: GRID_DOC,
      faces: GRID_FACES,
      hinges: new Set([8, 9, 10, 11]),
      frame3d: null,
      activeTool: "select",
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      errorMessage: null,
      selection: { edgeIds: [], vertexIds: [] },
      mirrorDraw: false,
      curve: { enabled: false, shape: "arc", segments: 4, rulings: false },
      construct: { kind: "bisector", divisions: 4, stepDeg: 22.5 },
    });
  });
  afterEach(() => cleanup());

  // 合格条件1・3: 3Dで点を選べること、選んだ点が2Dで選んだ点と同じ頂点を指すこと
  it("13個すべての点を3Dから選べ、同じ場所を2Dで選んだ結果と頂点IDが一致する", () => {
    const canvas = renderViewer();
    // 世界座標1.0あたりの画面px(この表示は真上からの平行に近い見え方)
    const scalePx = gridPoint([1, 0]).x - gridPoint([0, 0]).x;
    let picked = 0;
    let matched = 0;
    for (const vertex of GRID_DOC.cp.vertices) {
      // ちょうど真上ではなく少しずらして押し、当たり判定そのものを試す
      const at = gridPoint(vertex.pos, 3, -3);
      clickAt(canvas, at);
      const from3d = useAppStore.getState().selection.vertexIds;
      if (from3d.length === 1) picked += 1;
      // 同じ場所を展開図区画で押したときに選ばれる点
      const world: Vec2 = [vertex.pos[0] + 3 / scalePx, vertex.pos[1] + 3 / scalePx];
      const from2d = pickVertex(GRID_DOC, world, PICK_TOLERANCE_PX / scalePx);
      if (from2d !== null && from3d.length === 1 && from3d[0] === from2d) matched += 1;
    }
    expect(picked).toBe(GRID_DOC.cp.vertices.length);
    expect(matched).toBe(GRID_DOC.cp.vertices.length);
    expect(GRID_DOC.cp.vertices.length).toBeGreaterThanOrEqual(10);
  });

  it("測定でも13点と14辺が3Dと展開図で同じ対象になり、食い違いが0件", () => {
    useAppStore.setState({
      activeTool: "measure",
      measureDraft: { mode: "distance", picks: [], display: null },
    });
    const canvas = renderViewer();
    const scalePx = gridPoint([1, 0]).x - gridPoint([0, 0]).x;
    let checked = 0;
    let mismatched = 0;

    for (const vertex of GRID_DOC.cp.vertices) {
      useAppStore.getState().clearMeasurement();
      const at = gridPoint(vertex.pos, 3, -3);
      clickAt(canvas, at);
      const picked = useAppStore.getState().measureDraft.picks[0];
      const world: Vec2 = [
        vertex.pos[0] + 3 / scalePx,
        vertex.pos[1] + 3 / scalePx,
      ];
      const from2d = pickVertex(
        GRID_DOC,
        world,
        PICK_TOLERANCE_PX / scalePx,
      );
      checked += 1;
      if (
        picked?.kind !== "point" ||
        picked.vertexId === null ||
        picked.vertexId !== from2d
      ) {
        mismatched += 1;
      }
    }

    useAppStore.getState().setMeasureMode("length");
    const positions = new Map(
      GRID_DOC.cp.vertices.map((vertex) => [vertex.id, vertex.pos]),
    );
    for (const edge of GRID_DOC.cp.edges) {
      const a = positions.get(edge.v0)!;
      const b = positions.get(edge.v1)!;
      const midpoint: Vec2 = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
      clickAt(canvas, gridPoint(midpoint));
      const picked = useAppStore.getState().measureDraft.picks[0];
      const from2d = pickEdge(
        GRID_DOC,
        midpoint,
        PICK_TOLERANCE_PX / scalePx,
      );
      checked += 1;
      if (
        picked?.kind !== "edge" ||
        picked.edgeId !== from2d ||
        picked.edgeId !== edge.id
      ) {
        mismatched += 1;
      }
    }

    expect(checked).toBe(27);
    expect(mismatched).toBe(0);
  });

  it("測定の3方式を3Dから必要数だけ指定でき、方眼点にも吸着する", () => {
    useAppStore.setState({
      activeTool: "measure",
      measureDraft: { mode: "angle", picks: [], display: null },
    });
    const canvas = renderViewer();

    clickAt(canvas, gridPoint([0.25, 0]));
    clickAt(canvas, gridPoint([1, 0.25]));
    expect(useAppStore.getState().measureDraft.picks).toHaveLength(2);

    useAppStore.getState().setMeasureMode("length");
    clickAt(canvas, gridPoint([0.75, 1]));
    expect(useAppStore.getState().measureDraft.picks).toHaveLength(1);

    useAppStore.getState().setMeasureMode("distance");
    clickAt(canvas, gridPoint([0.253, 0.128]));
    const first = useAppStore.getState().measureDraft.picks[0];
    expect(first).toMatchObject({
      kind: "point",
      cp: [0.25, 0.125],
      vertexId: null,
    });
    clickAt(canvas, gridPoint([0.75, 0.875]));
    expect(useAppStore.getState().measureDraft.picks).toHaveLength(2);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(useAppStore.getState().activeTool).toBe("measure");
    expect(useAppStore.getState().measureDraft).toEqual({
      mode: "distance",
      picks: [],
      display: null,
    });
  });

  // 合格条件1: 立体姿勢(折り角度が0でも±180°でもない)でも点を選べること
  it("90°起こした立体姿勢でも、見えている面が持つ点を3Dから選べる", async () => {
    useAppStore.setState({
      doc: SPATIAL_DOC,
      faces: SPATIAL_FACES,
      hinges: new Set([6]),
      frame3d: SPATIAL_FRAME,
      activeTool: "select",
      selection: { edgeIds: [], vertexIds: [] },
    });
    const canvas = renderViewer();
    await waitFor(() => expect(held.scene.content).not.toBeNull());
    const camera = held.scene.camera as THREE.PerspectiveCamera;
    camera.position.set(-1.4, -1.6, 1.6);
    camera.lookAt(0.4, 0.5, 0.2);
    camera.updateMatrixWorld(true);
    camera.updateProjectionMatrix();

    // 立てた面(x=0.5から+z方向へ90°)の角と、平らな面の角の両方を試す
    const tried: { world: [number, number, number]; vertexId: number }[] = [
      { world: [0.5, 0, 0.5], vertexId: 2 },
      { world: [0.5, 1, 0.5], vertexId: 3 },
      { world: [0, 0, 0], vertexId: 0 },
      { world: [0, 1, 0], vertexId: 5 },
    ];
    let picked = 0;
    for (const one of tried) {
      useAppStore.getState().setSelection({ edgeIds: [], vertexIds: [] });
      clickAt(canvas, canvasPoint(one.world));
      if (useAppStore.getState().selection.vertexIds[0] === one.vertexId) picked += 1;
    }
    expect(picked).toBe(tried.length);
  });

  // 合格条件2の1件目: 選択-点
  it("選択: 点をクリックすると、その点だけが選ばれる(線の選択は残さない)", () => {
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.5, 0.5]));
    expect(useAppStore.getState().selection).toEqual({ edgeIds: [], vertexIds: [8] });
    // Ctrlクリックで足す・外すができる
    fireEvent.pointerDown(canvas, {
      button: 0,
      pointerId: 1,
      clientX: gridPoint([0.15, 0.25]).x,
      clientY: gridPoint([0.15, 0.25]).y,
      ctrlKey: true,
    });
    fireEvent.pointerUp(canvas, {
      button: 0,
      pointerId: 1,
      clientX: gridPoint([0.15, 0.25]).x,
      clientY: gridPoint([0.15, 0.25]).y,
      ctrlKey: true,
    });
    expect(useAppStore.getState().selection.vertexIds).toEqual([8, 9]);
  });

  // 合格条件2の2件目: 点を動かす
  it("選択: 点をつかんでドラッグすると、展開図の点が動く", async () => {
    const canvas = renderViewer();
    const from = gridPoint([0.5, 0.5]);
    const to = gridPoint([0.6, 0.55]);
    fireEvent.pointerDown(canvas, {
      button: 0,
      pointerId: 1,
      clientX: from.x,
      clientY: from.y,
    });
    fireEvent.pointerMove(canvas, { pointerId: 1, clientX: to.x, clientY: to.y });
    fireEvent.pointerUp(canvas, {
      button: 0,
      pointerId: 1,
      clientX: to.x,
      clientY: to.y,
    });
    await waitFor(() => expect(ipc.editApply).toHaveBeenCalled());
    const op = lastEditOps()[0];
    expect(op.type).toBe("MoveVertex");
    if (op.type !== "MoveVertex") return;
    expect(op.id).toBe(8);
    expect(op.to[0]).toBeCloseTo(0.6, 3);
    expect(op.to[1]).toBeCloseTo(0.55, 3);
  });

  it("全部をいっぺんに動かした形では、視点だけを動かせて紙の点は編集しない", () => {
    useAppStore.setState({
      foldAllPreview: {
        session: 2,
        percent: 50,
        appliedPercent: 50,
        busy: false,
        returning: false,
        error: null,
        converged: true,
        bestEffort: false,
        relaxationCount: 0,
        flatFoldViolationCount: 0,
        suspectHingeCount: 0,
        contactDetected: false,
        layerOrder: "unavailable_without_sequence",
        nextWarmSeed: [],
        returnState: {
          docEpoch: useAppStore.getState().docEpoch,
          currentStep: null,
          playT: 1,
          activeTool: "select",
          selection: { edgeIds: [], vertexIds: [] },
        },
      },
    });
    const canvas = renderViewer();
    const from = gridPoint([0.5, 0.5]);
    const to = gridPoint([0.6, 0.55]);

    fireEvent.pointerDown(canvas, {
      button: 0,
      pointerId: 1,
      clientX: from.x,
      clientY: from.y,
    });
    fireEvent.pointerMove(canvas, { pointerId: 1, clientX: to.x, clientY: to.y });
    fireEvent.pointerUp(canvas, {
      button: 0,
      pointerId: 1,
      clientX: to.x,
      clientY: to.y,
    });

    expect(ipc.editApply).not.toHaveBeenCalled();
    expect(useAppStore.getState().selection).toEqual({ edgeIds: [], vertexIds: [] });
    expect(canvas.getAttribute("aria-label")).toBe(
      "全部の折り目を同じ割合で動かした形。ドラッグで視点を回せます",
    );
    expect(canvas.getAttribute("data-tooltip")).not.toContain("点はドラッグで動かせます");
  });

  it("選択: 紙の外形の点は動かさない(展開図区画と同じ規則)", () => {
    const canvas = renderViewer();
    const from = gridPoint([0, 0]);
    const to = gridPoint([0.2, 0.2]);
    fireEvent.pointerDown(canvas, {
      button: 0,
      pointerId: 1,
      clientX: from.x,
      clientY: from.y,
    });
    fireEvent.pointerMove(canvas, { pointerId: 1, clientX: to.x, clientY: to.y });
    fireEvent.pointerUp(canvas, {
      button: 0,
      pointerId: 1,
      clientX: to.x,
      clientY: to.y,
    });
    expect(ipc.editApply).not.toHaveBeenCalled();
  });

  // 合格条件2の3〜5件目: 山・谷・補助の2クリック直線
  it.each([
    ["mountain", "Mountain"],
    ["valley", "Valley"],
    ["aux", "Aux"],
  ] as const)("%s: 3Dで2点をクリックすると直線が引ける", async (tool, kind) => {
    useAppStore.setState({ activeTool: tool });
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.15, 0.25]));
    expect(ipc.editApply).not.toHaveBeenCalled(); // 1クリック目では引かない
    clickAt(canvas, gridPoint([0.35, 0.25]));
    await waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));
    const op = lastEditOps()[0];
    expect(op.type).toBe("AddSegment");
    if (op.type !== "AddSegment") return;
    expect(op.kind).toBe(kind);
    // 点の上をクリックしたので、展開図の点のちょうどの座標が使われる
    expect(op.a).toEqual([0.15, 0.25]);
    expect(op.b).toEqual([0.35, 0.25]);
  });

  // 合格条件2の6〜8件目: 山・谷・補助の曲線モード
  it.each([
    ["mountain", "Mountain"],
    ["valley", "Valley"],
    ["aux", "Aux"],
  ] as const)("%s: 3Dで3点をクリックすると曲線(折れ線)が引ける", async (tool, kind) => {
    useAppStore.setState({
      activeTool: tool,
      curve: { enabled: true, shape: "arc", segments: 4, rulings: false },
    });
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.15, 0.25]));
    clickAt(canvas, gridPoint([0.35, 0.25]));
    expect(ipc.editApplyBatch).not.toHaveBeenCalled(); // 3点そろうまで引かない
    clickAt(canvas, gridPoint([0.25, 0.4]));
    await waitFor(() => expect(ipc.editApplyBatch).toHaveBeenCalledTimes(1));
    const ops = lastEditOps();
    // 4分割の円弧なので、折れ線は4本
    expect(ops).toHaveLength(4);
    expect(ops.every((one) => one.type === "AddSegment" && one.kind === kind)).toBe(true);
  });

  // 合格条件2の9件目: 折る-2クリックの折り線
  it("折る: Ctrl+クリック2回で折り線を決められる", () => {
    useAppStore.setState({ activeTool: "fold" });
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.5, 0]), true);
    expect(useAppStore.getState().foldDraft).toBeNull(); // 1クリック目では決まらない
    clickAt(canvas, gridPoint([0.5, 1]), true);
    const draft = useAppStore.getState().foldDraft;
    expect(draft).not.toBeNull();
    expect(draft?.line[0][0]).toBeCloseTo(0.5, 6);
    expect(draft?.line[1][0]).toBeCloseTo(0.5, 6);
    expect(draft?.line[0][1]).toBeCloseTo(0, 6);
    expect(draft?.line[1][1]).toBeCloseTo(1, 6);
  });

  // 合格条件2の10〜13件目: 作図4通り
  it("作図(二等分): 3点をクリックすると補助線が引ける", async () => {
    useAppStore.setState({
      activeTool: "construct",
      construct: { kind: "bisector", divisions: 4, stepDeg: 22.5 },
    });
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.5, 0]));
    clickAt(canvas, gridPoint([0.5, 0.5]));
    clickAt(canvas, gridPoint([0, 0.5]));
    await waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));
    const ops = lastEditOps();
    expect(ops).toHaveLength(1);
    expect(ops[0].type === "AddSegment" && ops[0].kind).toBe("Aux");
  });

  it("作図(垂線): 線と点をクリックすると補助線が引ける", async () => {
    useAppStore.setState({
      activeTool: "construct",
      construct: { kind: "perpendicular", divisions: 4, stepDeg: 22.5 },
    });
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.5, 0.25])); // 折り目(1)-(8)の上
    expect(ipc.editApply).not.toHaveBeenCalled();
    clickAt(canvas, gridPoint([0.15, 0.25]));
    await waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));
    const op = lastEditOps()[0];
    expect(op.type).toBe("AddSegment");
    if (op.type !== "AddSegment") return;
    expect(op.a).toEqual([0.15, 0.25]);
    expect(op.b[0]).toBeCloseTo(0.5, 6);
    expect(op.b[1]).toBeCloseTo(0.25, 6);
  });

  it("作図(等分): 2点をクリックすると等分の目印が引ける", async () => {
    useAppStore.setState({
      activeTool: "construct",
      construct: { kind: "divide", divisions: 4, stepDeg: 22.5 },
    });
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.15, 0.25]));
    clickAt(canvas, gridPoint([0.35, 0.25]));
    await waitFor(() => expect(ipc.editApplyBatch).toHaveBeenCalledTimes(1));
    // 4等分なので目印は3本
    expect(lastEditOps()).toHaveLength(3);
  });

  it("作図(角度線): 1点をクリックすると方向線がまとめて引ける", async () => {
    useAppStore.setState({
      activeTool: "construct",
      construct: { kind: "angle", divisions: 4, stepDeg: 22.5 },
    });
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.5, 0.5]));
    await waitFor(() => expect(ipc.editApplyBatch).toHaveBeenCalledTimes(1));
    // 22.5°刻みで180°未満なので8本
    expect(lastEditOps()).toHaveLength(8);
  });

  it("Escで選びかけの点を捨てる(次のクリックが思わぬ線にならない)", async () => {
    useAppStore.setState({ activeTool: "valley" });
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.15, 0.25]));
    fireEvent.keyDown(window, { key: "Escape" });
    clickAt(canvas, gridPoint([0.35, 0.25]));
    // 1点目が捨てられているので、まだ線にならない
    await waitFor(() => expect(ipc.editApply).not.toHaveBeenCalled());
  });

  it("選んだ点は3Dの上に十字で出る(どこを指したか3Dだけで分かる)", async () => {
    const canvas = renderViewer();
    clickAt(canvas, gridPoint([0.5, 0.5]));
    await waitFor(() => {
      const calls = (held.scene.setHighlight as ReturnType<typeof vi.fn>).mock.calls;
      const last = calls[calls.length - 1][0] as {
        edgeId: number;
        role?: string;
        a: THREE.Vector3;
        b: THREE.Vector3;
      }[];
      const marks = last.filter((segment) => segment.edgeId === -1);
      expect(marks).toHaveLength(2); // 縦横1本ずつの十字
      for (const mark of marks) {
        expect(mark.a.distanceTo(new THREE.Vector3(0.5, 0.5, 0))).toBeLessThan(0.05);
      }
    });
  });
});
