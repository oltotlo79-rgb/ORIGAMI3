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
  editApplyBatch: vi.fn(),
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
  contact_detected: false,
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
    angleUndoStack: [],
    angleRedoStack: [],
    docUndoDepth: 0,
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

  // 実機で見つかった不具合: 元に戻すと角度の表示は戻るのに、3Dが完全な平らになった。
  // 手順を記録していない作品では、戻した先の指定角ではなく「全ての折り線を0度」を
  // 出発点にして解き直していたため。戻した先の角度そのものを出発点にする。
  it("元に戻したとき、戻した先の角度を出発点にして解き直す(平らにしない)", async () => {
    const store = useAppStore.getState();
    store.setDriverAngle(5, 90);
    await settle();
    // 同じ折り線の連続変更は1件にまとまるので、別の折り線を動かして履歴を分ける
    store.setDriverAngle(7, 20);
    await settle();
    vi.mocked(ipc.poseSolve).mockClear();

    await useAppStore.getState().undo();
    await settle();

    expect(useAppStore.getState().drivers.get(5)).toBe(90);
    expect(useAppStore.getState().drivers.has(7)).toBe(false);
    expect(vi.mocked(ipc.poseSolve)).toHaveBeenCalled();
    const calls = vi.mocked(ipc.poseSolve).mock.calls;
    const seed = calls[calls.length - 1][3] as { hinge: number; target_angle_deg: number }[];
    expect(seed.find((d) => d.hinge === 5)?.target_angle_deg).toBe(90);
    // 指定の無い折り線は平らのまま
    expect(seed.find((d) => d.hinge === 7)?.target_angle_deg).toBe(0);
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

  it("追従した全角度と3D高さをcold startから複数回undo/redoして再現する", async () => {
    const allHinges = [5, 7, 9];
    let warmAngles = new Map<number, number>([
      [5, 90],
      [7, 30],
      [9, -15],
    ]);

    const solvedFrom = (driverAngle: number): SolveResult => {
      const followed = driverAngle / 3;
      const opposite = -driverAngle / 6;
      return {
        frame: {
          faces: [
            {
              face: 0,
              polygon: [
                [0, 0, 0],
                [1, 0, Math.abs(followed) / 5],
                [0, 1, 0],
              ],
              layer: 0,
            },
          ],
          warnings: [],
        },
        converged: true,
        angles: { 5: driverAngle, 7: followed, 9: opposite },
        iterations: 1,
      };
    };

    vi.mocked(ipc.poseSolve).mockImplementation(
      async (_hard, preferred, _soft, warmSeed) => {
        const cold = warmSeed?.length === allHinges.length;
        if (cold) {
          warmAngles = new Map(
            warmSeed.map((driver) => [driver.hinge, driver.target_angle_deg]),
          );
        }
        const target = preferred?.find((driver) => driver.hinge === 5)?.target_angle_deg ?? 0;
        if (cold) {
          const solved = solvedFrom(target);
          warmAngles = new Map(
            Object.entries(solved.angles).map(([hinge, angle]) => [Number(hinge), angle]),
          );
          return solved;
        }

        // 旧経路の再現: preferredだけを戻しても、未指定ヒンジ7/9は
        // 前のwarm startを引き継ぎ、変形したまま残る。
        warmAngles.set(5, target);
        const height = Math.abs(warmAngles.get(7) ?? 0) / 5;
        return {
          frame: {
            faces: [
              {
                face: 0,
                polygon: [
                  [0, 0, 0],
                  [1, 0, height],
                  [0, 1, 0],
                ],
                layer: 0,
              },
            ],
            warnings: [],
          },
          converged: true,
          angles: Object.fromEntries(warmAngles),
          iterations: 1,
        };
      },
    );

    useAppStore.setState({
      hinges: new Set(allHinges),
      drivers: new Map([[5, 90]]),
      angleUndoStack: [
        { drivers: new Map([[5, 30]]), pinned: new Map() },
        { drivers: new Map([[5, 60]]), pinned: new Map() },
      ],
      angleRedoStack: [],
      poseAngles: new Map(warmAngles),
      frame3d: solvedFrom(90).frame,
    });

    const zSpread = () => {
      const zs = (useAppStore.getState().frame3d?.faces ?? []).flatMap((face) =>
        face.polygon.map((point) => point[2]),
      );
      return zs.length === 0 ? 0 : Math.max(...zs) - Math.min(...zs);
    };
    const expectShape = (driverAngle: number) => {
      expect(Object.fromEntries(useAppStore.getState().poseAngles)).toEqual({
        5: driverAngle,
        7: driverAngle / 3,
        9: -driverAngle / 6,
      });
      expect(zSpread()).toBeCloseTo(Math.abs(driverAngle / 3) / 5, 10);
    };

    await useAppStore.getState().undo();
    await settle();
    expectShape(60);
    await useAppStore.getState().undo();
    await settle();
    expectShape(30);
    await useAppStore.getState().redo();
    await settle();
    expectShape(60);
    await useAppStore.getState().redo();
    await settle();
    expectShape(90);
  });

  it("保存済み手順がある角度undoは平らにせず、手順から基準形を再生する", async () => {
    const sequenceDoc: Document = {
      ...DOC,
      sequence: [
        {
          id: 1,
          kind: "Simple",
          drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: 60 }],
          layer_order: null,
          note: "",
        },
      ],
    };
    const replayFrame = {
      faces: [
        {
          face: 0,
          polygon: [
            [0, 0, 0],
            [1, 0, 4],
            [0, 1, 0],
          ] as [number, number, number][],
          layer: 0,
        },
      ],
      warnings: [],
    };
    vi.mocked(ipc.sequenceReplay).mockResolvedValue({
      frame: replayFrame,
      skipped: [],
      warnings: [],
      sequence_targets: [{ hinge: 5, target_angle_deg: 60 }],
      angles: { 5: 60, 7: 20, 9: -10 },
      converged: true,
    });
    vi.mocked(ipc.poseSolve).mockClear();
    useAppStore.setState({
      doc: sequenceDoc,
      hinges: new Set([5, 7, 9]),
      drivers: new Map([[7, 90]]),
      angleUndoStack: [{ drivers: new Map(), pinned: new Map() }],
      angleRedoStack: [],
      currentStep: null,
      playT: 1,
      poseAngles: new Map([
        [5, 60],
        [7, 90],
        [9, -25],
      ]),
    });

    await useAppStore.getState().undo();

    expect(vi.mocked(ipc.sequenceReplay)).toHaveBeenCalledWith(1, 1, null);
    expect(vi.mocked(ipc.poseSolve)).not.toHaveBeenCalled();
    expect(Object.fromEntries(useAppStore.getState().poseAngles)).toEqual({
      5: 60,
      7: 20,
      9: -10,
    });
    const zs = useAppStore
      .getState()
      .frame3d?.faces.flatMap((face) => face.polygon.map((point) => point[2]));
    expect(Math.max(...(zs ?? [0])) - Math.min(...(zs ?? [0]))).toBe(4);
  });

  it("保存済み手順で固定だけが残るundoも、再生後に固定付きで解き直す", async () => {
    const sequenceDoc: Document = {
      ...DOC,
      sequence: [
        {
          id: 1,
          kind: "Simple",
          drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: 60 }],
          layer_order: null,
          note: "",
        },
      ],
    };
    const replayFrame = { faces: [], warnings: ["固定なしの再生形"] };
    const pinnedFrame = { faces: [], warnings: [] };
    vi.mocked(ipc.sequenceReplay).mockResolvedValue({
      frame: replayFrame,
      skipped: [],
      warnings: [],
      sequence_targets: [{ hinge: 5, target_angle_deg: 60 }],
      angles: { 5: 60 },
      converged: true,
    });
    vi.mocked(ipc.poseSolve).mockImplementation(async (hard, keep) => ({
      ...SOLVED,
      frame: pinnedFrame,
      angles: Object.fromEntries(
        [...hard, ...(keep ?? [])].map((driver) => [
          driver.hinge,
          driver.target_angle_deg,
        ]),
      ),
      closure_rms: 1e-15,
    }));
    useAppStore.setState({
      doc: sequenceDoc,
      hinges: new Set([5, 7]),
      drivers: new Map([[7, -48]]),
      pinnedFolds: new Map(),
      angleUndoStack: [
        { drivers: new Map(), pinned: new Map([[5, 45]]) },
      ],
      angleRedoStack: [],
      currentStep: null,
      playT: 1,
      poseWarnings: [
        "42本の折り目が目標の角度に届きませんでした",
        "固定した折り目2本を動かしました",
      ],
      releasedPins: [
        { hinge: 5, pinned: -180, actual: -48, deviation: 132 },
      ],
      releasedPinHinges: [5],
    });

    await useAppStore.getState().undo();

    expect(vi.mocked(ipc.sequenceReplay)).toHaveBeenCalledWith(1, 1, null);
    expect(vi.mocked(ipc.poseSolve)).toHaveBeenCalledTimes(1);
    const poseCall = vi.mocked(ipc.poseSolve).mock.calls[0];
    expect(poseCall[0]).toContainEqual({ hinge: 5, target_angle_deg: 45 });
    expect(poseCall[4]).toBe(1);
    expect(poseCall[5]).toBe(1);
    const state = useAppStore.getState();
    expect(state.pinnedFolds.get(5)).toBe(45);
    expect(Math.abs((state.poseAngles.get(5) ?? Infinity) - 45)).toBeLessThan(1e-9);
    expect(state.frame3d).toEqual(pinnedFrame);
    expect(state.poseWarnings).toEqual([]);
    expect(state.releasedPins).toEqual([]);
    expect(state.releasedPinHinges).toEqual([]);
  });

  it("live3のundo状態では固定#45/#132を再生後も-180度に保ち、古い2警告を消す", async () => {
    const sequenceDoc: Document = {
      ...DOC,
      sequence: [
        {
          id: 8,
          kind: "Simple",
          drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: -180 }],
          layer_order: null,
          note: "",
        },
      ],
    };
    vi.mocked(ipc.sequenceReplay).mockResolvedValue({
      frame: { faces: [], warnings: [] },
      skipped: [],
      warnings: [],
      sequence_targets: [
        { hinge: 45, target_angle_deg: -180 },
        { hinge: 132, target_angle_deg: -180 },
      ],
      angles: { 45: -180, 132: -180 },
      converged: true,
      closure_rms: 9.963347703678168e-17,
    });
    vi.mocked(ipc.poseSolve).mockImplementation(async (hard, keep) => ({
      ...SOLVED,
      angles: Object.fromEntries(
        [...hard, ...(keep ?? [])].map((driver) => [
          driver.hinge,
          driver.target_angle_deg,
        ]),
      ),
      closure_rms: 9.963347703678168e-17,
    }));
    useAppStore.setState({
      doc: sequenceDoc,
      hinges: new Set([45, 46, 132]),
      drivers: new Map([[46, -48]]),
      pinnedFolds: new Map([
        [45, -180],
        [132, -180],
      ]),
      angleUndoStack: [
        {
          drivers: new Map(),
          pinned: new Map([
            [45, -180],
            [132, -180],
          ]),
        },
      ],
      angleRedoStack: [],
      currentStep: null,
      playT: 1,
      poseWarnings: [
        "42本の折り目が目標の角度に届きませんでした",
        "固定した折り目2本を動かしました",
      ],
      releasedPins: [
        { hinge: 45, pinned: -180, actual: -48, deviation: 132 },
        { hinge: 132, pinned: -180, actual: -48, deviation: 132 },
      ],
      releasedPinHinges: [45, 132],
    });

    await useAppStore.getState().undo();

    expect(vi.mocked(ipc.sequenceReplay)).toHaveBeenCalledWith(1, 1, null);
    expect(vi.mocked(ipc.poseSolve)).toHaveBeenCalledTimes(1);
    const hard = vi.mocked(ipc.poseSolve).mock.calls[0][0];
    expect(hard).toContainEqual({ hinge: 45, target_angle_deg: -180 });
    expect(hard).toContainEqual({ hinge: 132, target_angle_deg: -180 });
    const state = useAppStore.getState();
    expect(Math.abs((state.poseAngles.get(45) ?? Infinity) - -180)).toBeLessThan(
      1e-9,
    );
    expect(Math.abs((state.poseAngles.get(132) ?? Infinity) - -180)).toBeLessThan(
      1e-9,
    );
    expect(state.poseWarnings).toEqual([]);
    expect(state.releasedPins).toEqual([]);
    expect(state.releasedPinHinges).toEqual([]);
  });

  it("手順・固定・driverを戻すundoは1回で解き、未固定の再生frameを表示しない", async () => {
    const sequenceDoc: Document = {
      ...DOC,
      sequence: [
        {
          id: 1,
          kind: "Simple",
          drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: 60 }],
          layer_order: null,
          note: "",
        },
      ],
    };
    vi.mocked(ipc.sequenceReplay).mockResolvedValue({
      frame: { faces: [], warnings: ["未固定の再生frame"] },
      skipped: [],
      warnings: [],
      sequence_targets: [
        { hinge: 5, target_angle_deg: 60 },
        { hinge: 7, target_angle_deg: 20 },
      ],
      angles: { 5: 60, 7: 20 },
      converged: true,
    });
    vi.mocked(ipc.poseSolve).mockImplementation(async (hard, keep) => ({
      ...SOLVED,
      frame: { faces: [], warnings: ["固定とdriverを戻したframe"] },
      angles: Object.fromEntries(
        [...hard, ...(keep ?? [])].map((driver) => [
          driver.hinge,
          driver.target_angle_deg,
        ]),
      ),
      closure_rms: 1e-15,
    }));
    useAppStore.setState({
      doc: sequenceDoc,
      hinges: new Set([5, 7]),
      drivers: new Map([[7, 90]]),
      pinnedFolds: new Map([[5, 10]]),
      angleUndoStack: [
        {
          drivers: new Map([[7, -48]]),
          pinned: new Map([[5, 45]]),
        },
      ],
      angleRedoStack: [],
      currentStep: null,
      playT: 1,
      frame3d: { faces: [], warnings: ["undo前のframe"] },
    });
    const observedWarnings: string[][] = [];
    const unsubscribe = useAppStore.subscribe((state) => {
      observedWarnings.push(state.frame3d?.warnings ?? []);
    });

    try {
      await useAppStore.getState().undo();
    } finally {
      unsubscribe();
    }

    expect(vi.mocked(ipc.sequenceReplay)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(ipc.poseSolve)).toHaveBeenCalledTimes(1);
    const poseCall = vi.mocked(ipc.poseSolve).mock.calls[0];
    expect(poseCall[0]).toContainEqual({ hinge: 5, target_angle_deg: 45 });
    expect(poseCall[1]).toContainEqual({ hinge: 7, target_angle_deg: -48 });
    expect(observedWarnings).not.toContainEqual(["未固定の再生frame"]);
    const state = useAppStore.getState();
    expect(state.poseAngles.get(5)).toBe(45);
    expect(state.poseAngles.get(7)).toBe(-48);
    expect(state.frame3d?.warnings).toEqual(["固定とdriverを戻したframe"]);
  });
});
