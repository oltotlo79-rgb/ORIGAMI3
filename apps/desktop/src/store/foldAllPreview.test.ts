// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/client", () => ({
  documentSave: vi.fn(),
  documentNew: vi.fn(),
  documentOpen: vi.fn(),
  editApply: vi.fn(),
  editRedo: vi.fn(),
  editUndo: vi.fn(),
  foldAllPreview: vi.fn(),
  poseSolve: vi.fn(),
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
}));

import * as ipc from "../ipc/client";
import type {
  Document,
  FoldAllPreviewOutcome,
  Frame3D,
  ReplayResult,
} from "../lib/types";
import {
  type AngleSnapshot,
  resetFoldAllPreviewRuntime,
  resetPoseThrottle,
  useAppStore,
} from "./appStore";

function angleHistoryValues(history: readonly AngleSnapshot[]) {
  return history.map((snapshot) => ({
    drivers: [...snapshot.drivers.entries()],
    pinned: [...snapshot.pinned.entries()],
  }));
}

function nonEmptyAngleHistories() {
  const undo: AngleSnapshot[] = [
    {
      drivers: new Map([[5, 25]]),
      pinned: new Map([[5, 25]]),
    },
  ];
  const redo: AngleSnapshot[] = [
    {
      drivers: new Map([[6, -35]]),
      pinned: new Map([[6, -35]]),
    },
  ];
  return { undo, redo };
}

function frameAt(marker: number): Frame3D {
  return {
    faces: [
      {
        face: 0,
        polygon: [
          [0, 0, marker],
          [1, 0, marker],
          [0, 1, marker],
        ],
        layer: 0,
        surface_rank: 0,
        mirrored: false,
      },
    ],
    warnings: [],
  };
}

function outcome(
  percent: number,
  patch: Partial<FoldAllPreviewOutcome> = {},
): FoldAllPreviewOutcome {
  return {
    frame: frameAt(percent),
    converged: true,
    angles: { "5": percent * 1.8, "6": percent * -1.8 },
    iterations: 2,
    requested_percent: percent,
    requested_angles: [
      { hinge: 5, target_angle_deg: percent * 1.8 },
      { hinge: 6, target_angle_deg: percent * -1.8 },
    ],
    next_warm_seed: [
      { hinge: 5, target_angle_deg: percent * 1.8 },
      { hinge: 6, target_angle_deg: percent * -1.8 },
    ],
    suspect_hinges: [],
    contact_detected: false,
    flat_fold_violations: [],
    layer_order: "unavailable_without_sequence",
    ...patch,
  };
}

function makeDocument(): Document {
  return {
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
        { id: 6, v0: 1, v1: 3, kind: "Valley" },
      ],
      next_vertex_id: 4,
      next_edge_id: 7,
    },
    sequence: [
      {
        id: 41,
        kind: "Simple",
        drivers: [{ a: [0, 0], b: [1, 1], target_angle_deg: 60 }],
        layer_order: null,
        note: "元の手順",
      },
    ],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

function normalReplay(marker = -1): ReplayResult {
  return {
    frame: frameAt(marker),
    skipped: [],
    warnings: [],
    angles: { "5": 60, "6": 0 },
    sequence_targets: [{ hinge: 5, target_angle_deg: 60 }],
    converged: true,
    best_effort: false,
    contact_detected: false,
  };
}

function currentView(display = useAppStore.getState().display) {
  const s = useAppStore.getState();
  if (!s.doc) throw new Error("作品がない");
  return {
    doc: { ...s.doc, display },
    faces: s.faces,
    warnings: [],
    violations: [],
    frame: frameAt(-1),
    skipped: [],
    contact_detected: false,
  };
}

function seed() {
  const doc = makeDocument();
  useAppStore.setState({
    doc,
    docEpoch: 9,
    faces: [
      { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
      { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
    ],
    hinges: new Set([5, 6]),
    frame3d: frameAt(-1),
    foldAllPreview: null,
    currentStep: 1,
    playT: 0.4,
    playing: false,
    activeTool: "mountain",
    selection: { edgeIds: [], vertexIds: [] },
    drivers: new Map(),
    pinnedFolds: new Map(),
    sequenceTargets: new Map([[5, 60]]),
    poseAngles: new Map([[5, 60]]),
    angleUndoStack: [],
    angleRedoStack: [],
    docUndoDepth: 0,
    poseWarnings: [],
    replayWarnings: [],
    flatFoldViolations: [],
    softMesh: null,
    softWarnings: [],
    errorMessage: null,
  });
  return doc;
}

beforeEach(() => {
  vi.clearAllMocks();
  resetPoseThrottle();
  resetFoldAllPreviewRuntime();
  vi.mocked(ipc.foldAllPreview).mockImplementation(async (percent) =>
    outcome(percent),
  );
  vi.mocked(ipc.sequenceReplay).mockResolvedValue(normalReplay());
  vi.mocked(ipc.editApply).mockImplementation(async (op) =>
    currentView(op.type === "SetDisplay" ? op.display : undefined),
  );
  vi.mocked(ipc.poseSolve).mockResolvedValue({
    frame: frameAt(0),
    converged: true,
    angles: { "5": 0, "6": 0 },
    iterations: 1,
  });
});

describe("全部の折り目をいっぺんに動かす一時表示", () => {
  it("入口の0%は全頂点z=0の平らなframeを表示する", async () => {
    seed();

    await useAppStore.getState().enterFoldAllPreview();

    const state = useAppStore.getState();
    expect(ipc.foldAllPreview).toHaveBeenCalledWith(0, []);
    expect(state.foldAllPreview?.appliedPercent).toBe(0);
    const vertices = state.frame3d?.faces.flatMap((face) => face.polygon) ?? [];
    expect(vertices).not.toHaveLength(0);
    expect(vertices.every((vertex) => vertex[2] === 0)).toBe(true);
  });

  it("利用者が自分で変えた視点を除き、戻った後も手順・選択・道具・角度・履歴を保つ", async () => {
    const doc = seed();
    const poseAngles = new Map([
      [5, 60],
      [6, 0],
    ]);
    useAppStore.setState({
      selection: { edgeIds: [5], vertexIds: [] },
      poseAngles,
    });
    const sequence = doc.sequence;
    const sequenceJson = JSON.stringify(sequence);
    const undo = useAppStore.getState().angleUndoStack;
    const redo = useAppStore.getState().angleRedoStack;
    const docUndoDepth = useAppStore.getState().docUndoDepth;
    const poseAngleEntries = [...poseAngles.entries()];
    // カメラはViewer3D内で利用者が直接動かす状態なので、この復帰契約には含めない。
    // ここではストアが所有する手順位置・選択・道具・角度を従来どおり厳密に検査する。

    await useAppStore.getState().enterFoldAllPreview();
    useAppStore.getState().setFoldAllPercent(50);
    useAppStore.getState().finishFoldAllPercent();
    await vi.waitFor(() =>
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(50),
    );
    await useAppStore.getState().leaveFoldAllPreview();

    const restored = useAppStore.getState();
    expect(restored.foldAllPreview).toBeNull();
    expect(restored.doc).toBe(doc);
    expect(restored.doc?.sequence).toBe(sequence);
    expect(JSON.stringify(restored.doc?.sequence)).toBe(sequenceJson);
    expect(restored.angleUndoStack).toBe(undo);
    expect(restored.angleRedoStack).toBe(redo);
    expect(restored.docUndoDepth).toBe(docUndoDepth);
    expect(restored.currentStep).toBe(1);
    expect(restored.playT).toBe(0.4);
    expect(restored.activeTool).toBe("mountain");
    expect(restored.selection).toEqual({ edgeIds: [5], vertexIds: [] });
    expect(restored.drivers).toEqual(new Map());
    expect(restored.pinnedFolds).toEqual(new Map());
    expect(restored.sequenceTargets).toEqual(new Map([[5, 60]]));
    expect([...restored.poseAngles.entries()]).toEqual(poseAngleEntries);
    expect(restored.frame3d).toEqual(frameAt(-1));
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
    expect(ipc.editUndo).not.toHaveBeenCalled();
    expect(ipc.editRedo).not.toHaveBeenCalled();
  });

  it("50%から0%へ戻すと、利用者が変えた視点を除く手順・選択・道具・角度が戻る", async () => {
    const doc = seed();
    const poseAngles = new Map([
      [5, 60],
      [6, 0],
    ]);
    const { undo, redo } = nonEmptyAngleHistories();
    useAppStore.setState({
      selection: { edgeIds: [5], vertexIds: [] },
      poseAngles,
      angleUndoStack: undo,
      angleRedoStack: redo,
      docUndoDepth: 2,
    });
    const sequence = doc.sequence;
    const sequenceJson = JSON.stringify(sequence);
    const poseAngleEntries = [...poseAngles.entries()];
    const undoValues = angleHistoryValues(undo);
    const redoValues = angleHistoryValues(redo);

    await useAppStore.getState().enterFoldAllPreview();
    useAppStore.getState().setFoldAllPercent(50);
    useAppStore.getState().finishFoldAllPercent();
    await vi.waitFor(() =>
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(50),
    );

    useAppStore.getState().setFoldAllPercent(0);
    useAppStore.getState().finishFoldAllPercent();

    await vi.waitFor(() =>
      expect(useAppStore.getState().foldAllPreview).toBeNull(),
    );
    const restored = useAppStore.getState();
    expect(restored.doc).toBe(doc);
    expect(restored.doc?.sequence).toBe(sequence);
    expect(JSON.stringify(restored.doc?.sequence)).toBe(sequenceJson);
    expect(restored.frame3d).toEqual(frameAt(-1));
    expect(restored.currentStep).toBe(1);
    expect(restored.playT).toBe(0.4);
    expect(restored.activeTool).toBe("mountain");
    expect(restored.selection).toEqual({ edgeIds: [5], vertexIds: [] });
    expect(restored.drivers).toEqual(new Map());
    expect(restored.pinnedFolds).toEqual(new Map());
    expect(restored.sequenceTargets).toEqual(new Map([[5, 60]]));
    expect([...restored.poseAngles.entries()]).toEqual(poseAngleEntries);
    expect(restored.docUndoDepth).toBe(2);
    expect(restored.angleUndoStack).toBe(undo);
    expect(restored.angleRedoStack).toBe(redo);
    expect(angleHistoryValues(restored.angleUndoStack)).toEqual(undoValues);
    expect(angleHistoryValues(restored.angleRedoStack)).toEqual(redoValues);
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
    expect(ipc.editUndo).not.toHaveBeenCalled();
    expect(ipc.editRedo).not.toHaveBeenCalled();
  });

  it("手順が無い作品も一時角度と固定を変えず、追従計算から通常形へ戻す", async () => {
    const doc = seed();
    doc.sequence = [];
    const drivers = new Map([[5, 30]]);
    const pinnedFolds = new Map([[5, 30]]);
    const driverEntries = [...drivers.entries()];
    const pinnedEntries = [...pinnedFolds.entries()];
    useAppStore.setState({
      currentStep: null,
      playT: 1,
      drivers,
      pinnedFolds,
      sequenceTargets: new Map(),
    });
    await useAppStore.getState().enterFoldAllPreview();

    await useAppStore.getState().leaveFoldAllPreview();

    expect(useAppStore.getState().foldAllPreview).toBeNull();
    expect(useAppStore.getState().drivers).toBe(drivers);
    expect(useAppStore.getState().pinnedFolds).toBe(pinnedFolds);
    expect([...useAppStore.getState().drivers.entries()]).toEqual(driverEntries);
    expect([...useAppStore.getState().pinnedFolds.entries()]).toEqual(
      pinnedEntries,
    );
    expect(ipc.poseSolve).toHaveBeenCalled();
    expect(ipc.sequenceReplay).not.toHaveBeenCalled();
  });

  it("連続100入力を同時1件・待機最新1件にまとめ、最後の割合と直前実角を使う", async () => {
    seed();
    await useAppStore.getState().enterFoldAllPreview();
    let releaseFirst!: (value: FoldAllPreviewOutcome) => void;
    const first = new Promise<FoldAllPreviewOutcome>((resolve) => {
      releaseFirst = resolve;
    });
    let active = 0;
    let maxActive = 0;
    const calls: { percent: number; warm: number }[] = [];
    vi.mocked(ipc.foldAllPreview).mockImplementation(async (percent, warm) => {
      active++;
      maxActive = Math.max(maxActive, active);
      calls.push({
        percent,
        warm: warm?.[0]?.target_angle_deg ?? Number.NaN,
      });
      try {
        return calls.length === 1 ? await first : outcome(percent);
      } finally {
        active--;
      }
    });

    useAppStore.getState().setFoldAllPercent(1);
    useAppStore.getState().finishFoldAllPercent();
    await vi.waitFor(() => expect(calls).toHaveLength(1));
    for (let percent = 2; percent <= 100; percent++) {
      useAppStore.getState().setFoldAllPercent(percent);
    }
    useAppStore.getState().finishFoldAllPercent();
    expect(calls).toHaveLength(1);

    releaseFirst(outcome(1));
    await vi.waitFor(() => expect(calls).toHaveLength(2));
    await vi.waitFor(() =>
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(100),
    );

    expect(maxActive).toBe(1);
    expect(calls[1]).toEqual({ percent: 100, warm: 0 });
  });

  it("1秒120入力を16msごとに間引き、IPCを65回以下にして最後の割合を採用する", async () => {
    vi.useFakeTimers();
    try {
      seed();
      await useAppStore.getState().enterFoldAllPreview();

      for (let index = 0; index < 120; index++) {
        const percent = index === 119 ? 100 : (index % 99) + 1;
        useAppStore.getState().setFoldAllPercent(percent);
        await vi.advanceTimersByTimeAsync(1000 / 120);
      }
      useAppStore.getState().finishFoldAllPercent();
      await vi.runAllTimersAsync();
      await Promise.resolve();

      const calls = vi.mocked(ipc.foldAllPreview).mock.calls;
      expect(calls.length).toBeLessThanOrEqual(65);
      expect(calls[calls.length - 1]?.[0]).toBe(100);
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(100);
    } finally {
      vi.useRealTimers();
    }
  });

  it("異なる連続入力120回で、各回の最後の割合を120/120採用する", async () => {
    vi.useFakeTimers();
    try {
      seed();
      await useAppStore.getState().enterFoldAllPreview();
      let adopted = 0;

      for (let trial = 0; trial < 120; trial++) {
        const finalPercent = ((trial * 37) % 100) + 1;
        for (let offset = 7; offset >= 1; offset--) {
          useAppStore
            .getState()
            .setFoldAllPercent(Math.max(1, finalPercent - offset));
        }
        useAppStore.getState().setFoldAllPercent(finalPercent);
        useAppStore.getState().finishFoldAllPercent();
        await vi.runAllTimersAsync();
        await Promise.resolve();

        if (
          useAppStore.getState().foldAllPreview?.appliedPercent === finalPercent
        ) {
          adopted++;
        }
      }

      expect(adopted).toBe(120);
    } finally {
      vi.useRealTimers();
    }
  });

  it("不収束・平坦条件・接触・貫通を受けても次の割合を計算する", async () => {
    seed();
    vi.mocked(ipc.foldAllPreview).mockImplementation(async (percent) =>
      outcome(percent, {
        converged: false,
        best_effort: true,
        relaxations: [
          {
            hinge: 5,
            target_angle_deg: 90,
            actual_angle_deg: 70,
            delta_deg: -20,
          },
        ],
        flat_fold_violations: [2],
        suspect_hinges: [5],
        contact_detected: true,
      }),
    );

    await useAppStore.getState().enterFoldAllPreview();
    const warned = useAppStore.getState().foldAllPreview;
    expect(warned).toMatchObject({
      converged: false,
      bestEffort: true,
      relaxationCount: 1,
      flatFoldViolationCount: 1,
      suspectHingeCount: 1,
      contactDetected: true,
    });

    useAppStore.getState().setFoldAllPercent(75);
    useAppStore.getState().finishFoldAllPercent();
    await vi.waitFor(() =>
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(75),
    );
    expect(ipc.foldAllPreview).toHaveBeenLastCalledWith(
      75,
      outcome(0).next_warm_seed,
    );
  });

  it("戻った後に遅れて届く一斉表示のframeを採用しない", async () => {
    seed();
    let release!: (value: FoldAllPreviewOutcome) => void;
    vi.mocked(ipc.foldAllPreview).mockImplementation(
      () =>
        new Promise<FoldAllPreviewOutcome>((resolve) => {
          release = resolve;
        }),
    );

    const entering = useAppStore.getState().enterFoldAllPreview();
    await vi.waitFor(() => expect(ipc.foldAllPreview).toHaveBeenCalledTimes(1));
    await useAppStore.getState().leaveFoldAllPreview();
    expect(useAppStore.getState().frame3d).toEqual(frameAt(-1));

    release(outcome(100));
    await entering;
    expect(useAppStore.getState().foldAllPreview).toBeNull();
    expect(useAppStore.getState().frame3d).toEqual(frameAt(-1));
  });

  it("新しい作品へ替えた後は古い一斉形を捨て、入口前の道具だけを保つ", async () => {
    seed();
    let release!: (value: FoldAllPreviewOutcome) => void;
    vi.mocked(ipc.foldAllPreview).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    const entering = useAppStore.getState().enterFoldAllPreview();
    await vi.waitFor(() => expect(ipc.foldAllPreview).toHaveBeenCalledTimes(1));
    const nextDoc = makeDocument();
    vi.mocked(ipc.documentNew).mockResolvedValue({
      doc: nextDoc,
      faces: [
        { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
        { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
      ],
      warnings: [],
      violations: [],
      frame: frameAt(-99),
      skipped: [],
      contact_detected: false,
    });

    await useAppStore.getState().newDocument(nextDoc.paper);
    release(outcome(100));
    await entering;

    expect(useAppStore.getState().foldAllPreview).toBeNull();
    expect(useAppStore.getState().doc).toBe(nextDoc);
    expect(useAppStore.getState().activeTool).toBe("mountain");
    expect(useAppStore.getState().frame3d).not.toEqual(frameAt(100));
  });

  it("新しい作品を開けなかった場合は、一斉表示とつまみを保つ", async () => {
    seed();
    await useAppStore.getState().enterFoldAllPreview();
    vi.mocked(ipc.documentOpen).mockRejectedValueOnce(new Error("open failed"));

    await useAppStore.getState().openDocument("C:\\work\\missing.ori3");

    expect(useAppStore.getState().foldAllPreview).not.toBeNull();
    useAppStore.getState().setFoldAllPercent(25);
    useAppStore.getState().finishFoldAllPercent();
    await vi.waitFor(() =>
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(25),
    );
  });

  it("Undoは作品履歴を進めず、通常表示へ戻るだけ", async () => {
    const doc = seed();
    const before = JSON.stringify(doc.sequence);
    const { undo: angleUndo, redo: angleRedo } = nonEmptyAngleHistories();
    useAppStore.setState({
      docUndoDepth: 2,
      angleUndoStack: angleUndo,
      angleRedoStack: angleRedo,
    });
    const undoValues = angleHistoryValues(angleUndo);
    const redoValues = angleHistoryValues(angleRedo);
    await useAppStore.getState().enterFoldAllPreview();

    await useAppStore.getState().undo();

    expect(useAppStore.getState().foldAllPreview).toBeNull();
    expect(JSON.stringify(useAppStore.getState().doc?.sequence)).toBe(before);
    expect(useAppStore.getState().docUndoDepth).toBe(2);
    expect(useAppStore.getState().angleUndoStack).toBe(angleUndo);
    expect(useAppStore.getState().angleRedoStack).toBe(angleRedo);
    expect(angleHistoryValues(useAppStore.getState().angleUndoStack)).toEqual(
      undoValues,
    );
    expect(angleHistoryValues(useAppStore.getState().angleRedoStack)).toEqual(
      redoValues,
    );
    expect(ipc.editUndo).not.toHaveBeenCalled();
  });

  it("Redoは作品履歴を進めず、通常表示へ戻るだけ", async () => {
    const doc = seed();
    const before = JSON.stringify(doc.sequence);
    const { undo: angleUndo, redo: angleRedo } = nonEmptyAngleHistories();
    useAppStore.setState({
      docUndoDepth: 2,
      angleUndoStack: angleUndo,
      angleRedoStack: angleRedo,
    });
    const undoValues = angleHistoryValues(angleUndo);
    const redoValues = angleHistoryValues(angleRedo);
    await useAppStore.getState().enterFoldAllPreview();

    await useAppStore.getState().redo();

    expect(useAppStore.getState().foldAllPreview).toBeNull();
    expect(JSON.stringify(useAppStore.getState().doc?.sequence)).toBe(before);
    expect(useAppStore.getState().docUndoDepth).toBe(2);
    expect(useAppStore.getState().angleUndoStack).toBe(angleUndo);
    expect(useAppStore.getState().angleRedoStack).toBe(angleRedo);
    expect(angleHistoryValues(useAppStore.getState().angleUndoStack)).toEqual(
      undoValues,
    );
    expect(angleHistoryValues(useAppStore.getState().angleRedoStack)).toEqual(
      redoValues,
    );
    expect(ipc.editRedo).not.toHaveBeenCalled();
  });

  it("通常形の再計算に失敗したら、目印と操作可能なつまみを残す", async () => {
    seed();
    await useAppStore.getState().enterFoldAllPreview();
    vi.mocked(ipc.sequenceReplay).mockRejectedValueOnce(new Error("replay failed"));

    await useAppStore.getState().leaveFoldAllPreview();

    expect(useAppStore.getState().foldAllPreview).toMatchObject({
      returning: false,
      error: "いつもの表示へ戻せませんでした。仮の形を表示したままです。",
    });
    useAppStore.getState().setFoldAllPercent(50);
    useAppStore.getState().finishFoldAllPercent();
    await vi.waitFor(() =>
      expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(50),
    );
  });

  it("通常形へ戻るまでは目印を消さず、選んだ道具へ切り替えない", async () => {
    seed();
    await useAppStore.getState().enterFoldAllPreview();
    let release!: (value: ReplayResult) => void;
    vi.mocked(ipc.sequenceReplay).mockImplementationOnce(
      () =>
        new Promise<ReplayResult>((resolve) => {
          release = resolve;
        }),
    );

    useAppStore.getState().setTool("pull");
    await vi.waitFor(() => expect(ipc.sequenceReplay).toHaveBeenCalledTimes(1));
    expect(useAppStore.getState().foldAllPreview?.returning).toBe(true);
    expect(useAppStore.getState().activeTool).toBe("select");

    release(normalReplay());
    await vi.waitFor(() => expect(useAppStore.getState().foldAllPreview).toBeNull());
    expect(useAppStore.getState().activeTool).toBe("pull");
    expect(useAppStore.getState().frame3d).toEqual(frameAt(-1));
  });

  it("復帰中に保存しても復帰を失敗扱いにせず、戻った後で保存する", async () => {
    seed();
    await useAppStore.getState().enterFoldAllPreview();
    let release!: (value: ReplayResult) => void;
    vi.mocked(ipc.sequenceReplay).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    vi.mocked(ipc.documentSave).mockResolvedValue(undefined);

    const leaving = useAppStore.getState().leaveFoldAllPreview();
    await vi.waitFor(() => expect(ipc.sequenceReplay).toHaveBeenCalledTimes(1));
    const saving = useAppStore
      .getState()
      .saveDocument("C:\\work\\after-return.ori3");
    await Promise.resolve();
    expect(ipc.documentSave).not.toHaveBeenCalled();

    release(normalReplay());
    await leaving;
    await saving;
    expect(useAppStore.getState().foldAllPreview).toBeNull();
    expect(ipc.documentSave).toHaveBeenCalledOnce();
  });

  it("復帰中に手順を連打しても、最後に選んだ位置だけへ移る", async () => {
    seed();
    await useAppStore.getState().enterFoldAllPreview();
    const releases: Array<(value: ReplayResult) => void> = [];
    let calls = 0;
    vi.mocked(ipc.sequenceReplay).mockImplementation(async (upTo) => {
      calls++;
      if (calls <= 2) {
        return await new Promise<ReplayResult>((resolve) => releases.push(resolve));
      }
      return normalReplay(upTo);
    });

    useAppStore.getState().selectStep(0);
    await vi.waitFor(() => expect(releases).toHaveLength(1));
    useAppStore.getState().selectStep(1);
    releases[0](normalReplay(-10));
    await vi.waitFor(() => expect(releases).toHaveLength(2));
    releases[1](normalReplay(-20));

    await vi.waitFor(() => {
      expect(useAppStore.getState().foldAllPreview).toBeNull();
      expect(useAppStore.getState().currentStep).toBe(1);
      expect(useAppStore.getState().frame3d).toEqual(frameAt(1));
    });
    expect(vi.mocked(ipc.sequenceReplay).mock.calls.map(([upTo]) => upTo)).toEqual([
      1, 1, 1,
    ]);
  });

  it("待機中の丸み設定を先に確定してから0%表示へ入り、そのまま閉じない", async () => {
    seed();
    useAppStore.getState().setSoft({ soft_enabled: true, soft_pressure: 0.4 });

    await useAppStore.getState().enterFoldAllPreview();

    expect(ipc.editApply).toHaveBeenCalledWith(
      expect.objectContaining({ type: "SetDisplay" }),
    );
    expect(useAppStore.getState().foldAllPreview).toMatchObject({
      appliedPercent: 0,
      returning: false,
    });
  });

  it("入口より前から実行中の作品変更を反映してから0%を求める", async () => {
    seed();
    let release!: (value: ReturnType<typeof currentView>) => void;
    vi.mocked(ipc.editApply).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    const editing = useAppStore
      .getState()
      .applyEdit({ type: "RemoveEdges", ids: [] });
    await vi.waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));

    const entering = useAppStore.getState().enterFoldAllPreview();
    await Promise.resolve();
    expect(ipc.foldAllPreview).not.toHaveBeenCalled();

    release(currentView());
    await editing;
    await entering;
    expect(ipc.foldAllPreview).toHaveBeenCalledWith(0, []);
    expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(0);
  });

  it("入口待ちの間に始まった手順移動を、一斉表示の公開後には適用しない", async () => {
    seed();
    let release!: (value: ReturnType<typeof currentView>) => void;
    vi.mocked(ipc.editApply).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    const editing = useAppStore
      .getState()
      .applyEdit({ type: "RemoveEdges", ids: [] });
    await vi.waitFor(() => expect(ipc.editApply).toHaveBeenCalledTimes(1));

    const entering = useAppStore.getState().enterFoldAllPreview();
    const moving = useAppStore.getState().moveStep(1, 0);
    release(currentView());
    await editing;
    await entering;
    await moving;

    expect(useAppStore.getState().foldAllPreview?.appliedPercent).toBe(0);
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
  });

  it("保存しても一斉形や手順を記録せず、専用表示を続ける", async () => {
    const doc = seed();
    const sequence = doc.sequence;
    const sequenceJson = JSON.stringify(sequence);
    const documentJson = JSON.stringify(doc);
    const docUndoDepth = useAppStore.getState().docUndoDepth;
    const angleUndo = useAppStore.getState().angleUndoStack;
    const angleRedo = useAppStore.getState().angleRedoStack;
    vi.mocked(ipc.documentSave).mockResolvedValue(undefined);
    await useAppStore.getState().enterFoldAllPreview();

    const path = "C:\\work\\fold-all-save.ori3";
    await useAppStore.getState().saveDocument(path);

    const state = useAppStore.getState();
    expect(state.foldAllPreview).not.toBeNull();
    expect(state.doc).toBe(doc);
    expect(state.doc?.sequence).toBe(sequence);
    expect(JSON.stringify(state.doc?.sequence)).toBe(sequenceJson);
    const currentDocumentJson = JSON.stringify(state.doc);
    expect(currentDocumentJson).toBe(documentJson);
    expect(state.docUndoDepth).toBe(docUndoDepth);
    expect(state.angleUndoStack).toBe(angleUndo);
    expect(state.angleRedoStack).toBe(angleRedo);
    for (const temporaryField of [
      "foldAllPreview",
      "fold_all_preview",
      "requested_percent",
      "requested_angles",
      "next_warm_seed",
      "suspect_hinges",
      "flat_fold_violations",
    ]) {
      expect(currentDocumentJson).not.toContain(`"${temporaryField}"`);
    }
    expect(ipc.documentSave).toHaveBeenCalledWith(path);
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
  });

  it("専用表示中は、外部から呼ばれても仕上げ手順や角度履歴を作らない", async () => {
    seed();
    const undo = useAppStore.getState().angleUndoStack;
    await useAppStore.getState().enterFoldAllPreview();

    useAppStore.getState().setDriverAngle(5, 90);
    await useAppStore.getState().recordPoseStep();

    expect(useAppStore.getState().angleUndoStack).toBe(undo);
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
  });
});
