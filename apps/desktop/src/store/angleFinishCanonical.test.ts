import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Document,
  DocumentView,
  Face,
  Frame3D,
  ReplayResult,
  SolveResult,
} from "../lib/types";

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
import {
  FINISH_JUMP_NOTICE,
  maximumFrameVertexMovement,
  resetPoseThrottle,
  useAppStore,
} from "./appStore";

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
      { id: 7, v0: 0, v1: 2, kind: "Valley" },
      { id: 9, v0: 0, v1: 2, kind: "Mountain" },
    ],
    next_vertex_id: 4,
    next_edge_id: 10,
  },
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

const SEQUENCE_DOC: Document = {
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

function frame(
  warning: string,
  z: number,
  surfaceRank: number,
  mirrored: boolean,
): Frame3D {
  return {
    faces: [
      {
        face: 0,
        polygon: [
          [0, 0, z],
          [1, 0, z + 0.25],
          [0, 1, z + 0.5],
        ],
        layer: surfaceRank,
        surface_rank: surfaceRank,
        mirrored,
      },
      {
        face: 1,
        polygon: [
          [0, 0, z + 1],
          [1, 1, z + 1.25],
          [0, 1, z + 1.5],
        ],
        layer: surfaceRank + 1,
        surface_rank: surfaceRank + 1,
        mirrored: !mirrored,
      },
    ],
    warnings: [warning],
  };
}

function solved(resultFrame: Frame3D, angles: Record<string, number>): SolveResult {
  return {
    frame: resultFrame,
    converged: true,
    angles,
    iterations: 1,
    closure_rms: 1e-15,
    best_effort: false,
    relaxations: [],
    suspect_hinges: [],
    contact_detected: false,
    soft: null,
    flat_fold_violations: [],
  };
}

function replayed(
  resultFrame: Frame3D,
  angles: Record<string, number>,
): ReplayResult {
  return {
    frame: resultFrame,
    skipped: [],
    warnings: [],
    sequence_targets: [{ hinge: 5, target_angle_deg: 60 }],
    angles,
    suspect_hinges: [],
    relaxations: [],
    closure_rms: 1e-15,
    best_effort: false,
    converged: true,
    contact_detected: false,
    soft: null,
    flat_fold_violations: [],
  };
}

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
} {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const DEFAULT_SOLVE = solved(frame("default", 0, 0, false), {
  5: 0,
  7: 0,
  9: 0,
});

beforeEach(() => {
  vi.clearAllMocks();
  resetPoseThrottle();
  vi.mocked(ipc.poseSolve).mockReset().mockResolvedValue(DEFAULT_SOLVE);
  vi.mocked(ipc.sequenceReplay)
    .mockReset()
    .mockResolvedValue(replayed(frame("default replay", 0, 0, false), {}));
  vi.mocked(ipc.editUndo).mockReset().mockResolvedValue(VIEW);
  vi.mocked(ipc.editRedo).mockReset().mockResolvedValue(VIEW);
  useAppStore.setState({
    doc: DOC,
    faces: FACES,
    display: DOC.display,
    hinges: new Set([5, 7, 9]),
    drivers: new Map(),
    pinnedFolds: new Map(),
    angleUndoStack: [],
    angleRedoStack: [],
    docUndoDepth: 0,
    poseAngles: new Map(),
    frame3d: null,
    sequenceTargets: new Map(),
    currentStep: null,
    playT: 1,
    playing: false,
    activeAngleIntent: null,
    releasedPins: [],
    releasedPinHinges: [],
    poseWarnings: [],
    foldAllPreview: null,
    selection: { edgeIds: [], vertexIds: [] },
    errorMessage: null,
  });
});

describe("角度ジェスチャー確定時のcanonical再導出", () => {
  it("drag中のFollow入力を変えず、二重finishでもcanonicalを1回だけ採用する", async () => {
    const followFrame = frame("follow", 10, 7, true);
    const canonicalFrame = frame("canonical", 20, 2, false);
    const follow = deferred<SolveResult>();
    const canonical = deferred<SolveResult>();
    vi.mocked(ipc.poseSolve)
      .mockReset()
      .mockReturnValueOnce(follow.promise)
      .mockReturnValueOnce(canonical.promise)
      .mockResolvedValue(DEFAULT_SOLVE);
    useAppStore.setState({
      drivers: new Map([[7, 30]]),
      poseAngles: new Map([
        [5, 5],
        [7, 30],
        [9, -5],
      ]),
      frame3d: frame("before", -10, 4, true),
    });

    useAppStore.getState().setDriverAngle(5, 72);
    const firstFinish = useAppStore.getState().finishAngleIntent();
    const secondFinish = useAppStore.getState().finishAngleIntent();

    await vi.waitFor(() => expect(ipc.poseSolve).toHaveBeenCalledTimes(1));
    expect(vi.mocked(ipc.poseSolve).mock.calls[0]).toEqual([
      [{ hinge: 5, target_angle_deg: 72 }],
      [{ hinge: 7, target_angle_deg: 30 }],
      null,
      [],
      0,
      1,
    ]);

    // 選択は強調だけを消し、Followの計算世代と確定待ちは失わせない。
    useAppStore.getState().setSelection({ edgeIds: [9], vertexIds: [2] });
    expect(useAppStore.getState().activeAngleIntent).toBeNull();

    follow.resolve(
      solved(followFrame, {
        5: 71.5,
        7: 29.5,
        9: -8,
      }),
    );
    await vi.waitFor(() => expect(ipc.poseSolve).toHaveBeenCalledTimes(2));
    expect(vi.mocked(ipc.poseSolve).mock.calls[1]).toEqual([
      [],
      [
        { hinge: 5, target_angle_deg: 72 },
        { hinge: 7, target_angle_deg: 30 },
      ],
      null,
      [
        { hinge: 5, target_angle_deg: 72 },
        { hinge: 7, target_angle_deg: 30 },
        { hinge: 9, target_angle_deg: 0 },
      ],
      0,
      1,
      "Canonical",
    ]);

    canonical.resolve(
      solved(canonicalFrame, {
        5: 72,
        7: 28,
        9: -9,
      }),
    );
    await Promise.all([firstFinish, secondFinish]);

    expect(ipc.poseSolve).toHaveBeenCalledTimes(2);
    expect(useAppStore.getState().frame3d).toEqual(canonicalFrame);
    expect(Object.fromEntries(useAppStore.getState().poseAngles)).toEqual({
      5: 72,
      7: 28,
      9: -9,
    });
    expect(useAppStore.getState().selection).toEqual({
      edgeIds: [9],
      vertexIds: [2],
    });
    expect(useAppStore.getState().activeAngleIntent).toBeNull();
    expect(
      useAppStore
        .getState()
        .poseWarnings.filter((warning) => warning === FINISH_JUMP_NOTICE),
    ).toHaveLength(1);
  });

  it("sequence位置と希望角を揃え、replay実角をcanonicalの隠れた入力にしない", async () => {
    const replayFrame = frame("replay seed", 30, 8, true);
    const canonicalFrame = frame("sequence canonical", 40, 1, false);
    vi.mocked(ipc.sequenceReplay).mockResolvedValue(
      replayed(replayFrame, {
        9: -10,
        5: 60,
        7: 20,
      }),
    );
    vi.mocked(ipc.poseSolve)
      .mockReset()
      .mockResolvedValueOnce(
        solved(frame("sequence follow", 15, 3, true), {
          5: 60,
          7: 89,
          9: -11,
        }),
      )
      .mockResolvedValueOnce(
        solved(canonicalFrame, {
          5: 60,
          7: 90,
          9: -10,
        }),
      );
    useAppStore.setState({
      doc: SEQUENCE_DOC,
      currentStep: 1,
      playT: 0.4,
    });

    useAppStore.getState().setDriverAngle(7, 90);
    await useAppStore.getState().finishAngleIntent();

    expect(ipc.sequenceReplay).toHaveBeenCalledTimes(1);
    expect(ipc.sequenceReplay).toHaveBeenCalledWith(1, 0.4, null);
    expect(ipc.poseSolve).toHaveBeenCalledTimes(2);
    expect(vi.mocked(ipc.poseSolve).mock.calls[0]).toEqual([
      [{ hinge: 7, target_angle_deg: 90 }],
      [],
      null,
      [],
      1,
      0.4,
    ]);
    expect(vi.mocked(ipc.poseSolve).mock.calls[1]).toEqual([
      [],
      [
        { hinge: 5, target_angle_deg: 60 },
        { hinge: 7, target_angle_deg: 90 },
      ],
      null,
      [
        { hinge: 5, target_angle_deg: 60 },
        { hinge: 7, target_angle_deg: 20 },
        { hinge: 9, target_angle_deg: -10 },
      ],
      1,
      0.4,
      "Canonical",
    ]);
    expect(useAppStore.getState().frame3d).toEqual(canonicalFrame);
    expect(Object.fromEntries(useAppStore.getState().poseAngles)).toEqual({
      5: 60,
      7: 90,
      9: -10,
    });
  });

  it("1ジェスチャー後のUndo 1回でdrivers・実角・full frameを直前canonicalへ戻す", async () => {
    const beforeFrame = frame("before canonical", 50, 5, true);
    const afterFrame = frame("after canonical", 60, 0, false);
    vi.mocked(ipc.poseSolve)
      .mockReset()
      .mockResolvedValueOnce(
        solved(frame("undo follow", 55, 6, false), {
          5: 71,
          7: 29,
          9: -8,
        }),
      )
      .mockResolvedValueOnce(
        solved(afterFrame, {
          5: 72,
          7: 28,
          9: -9,
        }),
      )
      .mockResolvedValueOnce(
        solved(beforeFrame, {
          5: 0,
          7: 30,
          9: -5,
        }),
      );
    useAppStore.setState({
      drivers: new Map([[7, 30]]),
      poseAngles: new Map([
        [5, 0],
        [7, 30],
        [9, -5],
      ]),
      frame3d: beforeFrame,
    });

    useAppStore.getState().setDriverAngle(5, 72);
    await useAppStore.getState().finishAngleIntent();
    expect(useAppStore.getState().angleUndoStack).toHaveLength(1);
    expect(useAppStore.getState().frame3d).toEqual(afterFrame);

    await useAppStore.getState().undo();

    expect(ipc.editUndo).not.toHaveBeenCalled();
    expect(Object.fromEntries(useAppStore.getState().drivers)).toEqual({ 7: 30 });
    expect(Object.fromEntries(useAppStore.getState().poseAngles)).toEqual({
      5: 0,
      7: 30,
      9: -5,
    });
    expect(useAppStore.getState().frame3d).toEqual(beforeFrame);
    expect(useAppStore.getState().angleUndoStack).toHaveLength(0);
    expect(useAppStore.getState().angleRedoStack).toHaveLength(1);
    expect(ipc.poseSolve).toHaveBeenCalledTimes(3);
    expect(vi.mocked(ipc.poseSolve).mock.calls[2]).toEqual([
      [],
      [{ hinge: 7, target_angle_deg: 30 }],
      null,
      [
        { hinge: 5, target_angle_deg: 0 },
        { hinge: 7, target_angle_deg: 30 },
        { hinge: 9, target_angle_deg: 0 },
      ],
      0,
      1,
      "Canonical",
    ]);
  });

  it("同じhingeでもfinish済みの別ジェスチャーは700ms以内に履歴2件になる", async () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-08-25T00:00:00Z"));
      useAppStore.getState().setDriverAngle(5, 30);
      await useAppStore.getState().finishAngleIntent();

      vi.setSystemTime(new Date("2026-08-25T00:00:00.100Z"));
      useAppStore.getState().setDriverAngle(5, 60);
      await useAppStore.getState().finishAngleIntent();

      expect(useAppStore.getState().angleUndoStack).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("face IDで頂点を対応させ、比較不能frameを移動0として扱わない", () => {
    const before = frame("before", 0, 0, false);
    const after = frame("after", 0, 0, false);
    after.faces.reverse();
    after.faces.find((face) => face.face === 1)!.polygon[2][2] += 0.08;

    expect(maximumFrameVertexMovement(before, after)).toBeCloseTo(0.08, 12);
    expect(
      maximumFrameVertexMovement(before, {
        ...after,
        faces: after.faces.slice(1),
      }),
    ).toBeNull();
    const nonFinite = frame("nonfinite", 0, 0, false);
    nonFinite.faces[0].polygon[0][0] = Number.NaN;
    expect(maximumFrameVertexMovement(before, nonFinite)).toBeNull();
  });

  it("確定時の移動が紙の長辺の10%未満なら大移動の警告を足さない", async () => {
    const followFrame = frame("follow", 0, 0, false);
    const canonicalFrame = frame("canonical", 0.05, 0, false);
    vi.mocked(ipc.poseSolve)
      .mockReset()
      .mockResolvedValueOnce(solved(followFrame, { 5: 45, 7: 0, 9: 0 }))
      .mockResolvedValueOnce(
        solved(canonicalFrame, { 5: 45, 7: 0, 9: 0 }),
      );

    useAppStore.getState().setDriverAngle(5, 45);
    await useAppStore.getState().finishAngleIntent();

    expect(useAppStore.getState().frame3d).toEqual(canonicalFrame);
    expect(useAppStore.getState().poseWarnings).not.toContain(FINISH_JUMP_NOTICE);
  });
});
