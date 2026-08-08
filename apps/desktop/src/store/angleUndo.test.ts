// 折り角度の「元に戻す/やり直し」のテスト(実機で見つかった不具合の再発防止)。
// 症状: 折り線を引く → 角度を変える → 元に戻す、で折り目(線)が消えた。
// 角度の変更は作品データではないのでedit_undoの履歴に載らず、直前の
// 「線の追加」が取り消されていた。角度の履歴を画面側に持って直した。

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Document, DocumentView, Face, SolveResult } from "../lib/types";

vi.mock("../ipc/client", () => ({
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

import * as ipc from "../ipc/client";
import { resetPoseThrottle, useAppStore } from "./appStore";

/** 角度の間引き(16ms)より少し長く待つ時間(ms) */
const POSE_WAIT_MS = 100;

/** 対角線(辺5)で2つの面に分かれた正方形 */
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
const FACES: Face[] = [
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
const SOLVED: SolveResult = {
  frame: { faces: [], warnings: [] },
  converged: true,
  angles: {},
  iterations: 1,
};

/** 実機の操作を待つ(間引きの実行と応答の反映) */
const settle = () => new Promise((r) => setTimeout(r, POSE_WAIT_MS));

beforeEach(() => {
  vi.clearAllMocks();
  resetPoseThrottle();
  vi.mocked(ipc.poseSolve).mockResolvedValue(SOLVED);
  vi.mocked(ipc.editUndo).mockResolvedValue(VIEW);
  vi.mocked(ipc.editRedo).mockResolvedValue(VIEW);
  useAppStore.setState({
    doc: DOC,
    faces: FACES,
    hinges: new Set([5, 7]),
    drivers: new Map(),
    poseAngles: new Map(),
    frame3d: null,
    playing: false,
    playT: 1,
    currentStep: null,
    errorMessage: null,
  });
});

describe("折り角度の元に戻す/やり直し", () => {
  it("線を引いた後に角度を変えて元に戻すと、角度だけが戻り折り線は残る", async () => {
    vi.mocked(ipc.editApply).mockResolvedValue(VIEW);
    await useAppStore
      .getState()
      .applyEdit({ type: "AddSegment", a: [0, 0], b: [1, 1], kind: "Mountain" });
    useAppStore.getState().setDriverAngle(5, 90);
    await settle();
    expect(useAppStore.getState().drivers.get(5)).toBe(90);

    await useAppStore.getState().undo();
    await settle();
    // 作品データへの取り消しは送られない = 引いた折り線は消えない
    expect(vi.mocked(ipc.editUndo)).not.toHaveBeenCalled();
    expect(useAppStore.getState().drivers.size).toBe(0);
    // 3D表示も作り直される
    expect(useAppStore.getState().frame3d).toEqual(SOLVED.frame);

    // もう一度押すと、今度は線の追加が戻る
    await useAppStore.getState().undo();
    expect(vi.mocked(ipc.editUndo)).toHaveBeenCalledTimes(1);
  });

  it("やり直しは、作品データ → 折り角度の順に復元する", async () => {
    vi.mocked(ipc.editApply).mockResolvedValue(VIEW);
    await useAppStore
      .getState()
      .applyEdit({ type: "AddSegment", a: [0, 0], b: [1, 1], kind: "Mountain" });
    useAppStore.getState().setDriverAngle(5, 90);
    await settle();
    await useAppStore.getState().undo(); // 角度が戻る
    await useAppStore.getState().undo(); // 線の追加が戻る
    await settle();

    await useAppStore.getState().redo(); // まず線の追加をやり直す
    expect(vi.mocked(ipc.editRedo)).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().drivers.size).toBe(0);

    await useAppStore.getState().redo(); // 次に角度をやり直す
    await settle();
    expect(vi.mocked(ipc.editRedo)).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().drivers.get(5)).toBe(90);
  });

  it("紙を1回ドラッグしても履歴は1件しか増えない", () => {
    const store = useAppStore.getState();
    store.beginPull(5, new Map());
    store.pullTo(-30);
    store.pullTo(-60);
    store.pullTo(-90);
    store.endPull();
    expect(useAppStore.getState().angleUndoStack).toHaveLength(1);

    store.beginPull(5, new Map());
    store.pullTo(-120);
    store.endPull();
    expect(useAppStore.getState().angleUndoStack).toHaveLength(2);
  });

  it("スライダーを動かしている間の細かい変更は1件にまとまる", () => {
    const store = useAppStore.getState();
    for (const deg of [10, 20, 30, 40, 50]) store.setDriverAngle(5, deg);
    expect(useAppStore.getState().angleUndoStack).toHaveLength(1);
    // 別の折り線を動かせば別の操作として積む
    store.setDriverAngle(7, 20);
    expect(useAppStore.getState().angleUndoStack).toHaveLength(2);
  });

  it("履歴は50件を超えて溜まらない", () => {
    const store = useAppStore.getState();
    for (let i = 0; i < 60; i += 1) store.setDriverAngle(i % 2 === 0 ? 5 : 7, i);
    expect(useAppStore.getState().angleUndoStack).toHaveLength(50);
  });

  it("展開図を編集すると、その後の元に戻すは作品データ側へ回る", async () => {
    vi.mocked(ipc.editApply).mockResolvedValue(VIEW);
    useAppStore.getState().setDriverAngle(5, 90);
    await settle();
    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [5] });
    expect(useAppStore.getState().angleUndoStack).toHaveLength(0);

    await useAppStore.getState().undo();
    expect(vi.mocked(ipc.editUndo)).toHaveBeenCalledTimes(1);
  });

  it("編集が断られたときは角度の履歴を捨てない(角度を戻せなくならない)", async () => {
    vi.mocked(ipc.editApply).mockRejectedValue("その線は引けません");
    useAppStore.getState().setDriverAngle(5, 90);
    await settle();
    await useAppStore
      .getState()
      .applyEdit({ type: "AddSegment", a: [0, 0], b: [1, 1], kind: "Mountain" });
    expect(useAppStore.getState().errorMessage).toBe("その線は引けません");
    expect(useAppStore.getState().angleUndoStack).toHaveLength(1);

    await useAppStore.getState().undo();
    await settle();
    expect(vi.mocked(ipc.editUndo)).not.toHaveBeenCalled();
    expect(useAppStore.getState().drivers.size).toBe(0);
  });

  it("作品を開くと折り角度の履歴は捨てられる(保存しない情報だから)", async () => {
    useAppStore.getState().setDriverAngle(5, 90);
    expect(useAppStore.getState().angleUndoStack).toHaveLength(1);

    vi.mocked(ipc.documentOpen).mockResolvedValue(VIEW);
    await useAppStore.getState().openDocument("sample.ori3");
    expect(useAppStore.getState().angleUndoStack).toHaveLength(0);
    expect(useAppStore.getState().angleRedoStack).toHaveLength(0);
  });

  it("戻して指定が減った折り線には0度を明示して形を作り直す", async () => {
    const store = useAppStore.getState();
    store.setDriverAngle(5, 90);
    await settle();
    store.setDriverAngle(7, 45);
    await settle();
    vi.mocked(ipc.poseSolve).mockClear();

    await useAppStore.getState().undo();
    await settle();
    const [hard, keep] = vi.mocked(ipc.poseSolve).mock.calls[0];
    expect(hard).toEqual([{ hinge: 7, target_angle_deg: 0 }]);
    expect(keep).toEqual([{ hinge: 5, target_angle_deg: 90 }]);
  });
});
