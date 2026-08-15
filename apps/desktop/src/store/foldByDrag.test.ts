// 紙をつかんで動かす折り操作(UI-007)のストア側テスト。
// ドラッグの2点が、そのままFoldThroughの折り線・動かさない側になることを確かめる。

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Document, DocumentView, Face, Frame3D } from "../lib/types";

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
import { isSpatialFoldFrame, useAppStore } from "./appStore";

/** 正方形1枚(折り線なし) */
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
  display: {
    front_color: [230, 90, 60],
    back_color: [245, 245, 245],
    grid_divisions: 8,
  },
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

const POSE_STEP = {
  id: 0,
  kind: "Pose" as const,
  drivers: [
    {
      a: [0.5, 0] as [number, number],
      b: [0.5, 1] as [number, number],
      target_angle_deg: 90,
    },
  ],
  layer_order: null,
  note: "立体の形を残す",
};

/** 中央のヒンジで右半分を90°起こした、replayで作れる立体姿勢。 */
const NON_FLAT_DOC: Document = {
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
  sequence: [POSE_STEP],
};

const NON_FLAT_FACES: Face[] = [
  { id: 0, vertices: [0, 1, 4, 5], edges: [0, 6, 4, 5] },
  { id: 1, vertices: [1, 2, 3, 4], edges: [1, 2, 3, 6] },
];

const NON_FLAT_FRAME: Frame3D = {
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
    },
  ],
  warnings: [],
};

const AFTER_NON_FLAT_FOLD_FRAME: Frame3D = {
  faces: [
    NON_FLAT_FRAME.faces[0],
    {
      face: 1,
      polygon: [
        [0.5, 0, 0],
        [0.5, 0, 0.5],
        [0.5, 1, 0],
      ],
      layer: 1,
    },
    {
      face: 2,
      polygon: [
        [0.5, 0, 0.5],
        [0.5, 0.6, -0.3],
        [0.5, 1, 0],
      ],
      layer: 2,
    },
  ],
  warnings: [],
};

const AFTER_NON_FLAT_FOLD_FACES: Face[] = [
  NON_FLAT_FACES[0],
  { id: 1, vertices: [1, 2, 4], edges: [1, 7, 6] },
  { id: 2, vertices: [2, 3, 4], edges: [2, 3, 7] },
];

const AFTER_NON_FLAT_FOLD_DOC: Document = {
  ...NON_FLAT_DOC,
  cp: {
    ...NON_FLAT_DOC.cp,
    edges: [
      ...NON_FLAT_DOC.cp.edges,
      { id: 7, v0: 2, v1: 4, kind: "Valley" },
    ],
    next_edge_id: 8,
  },
  sequence: [
    POSE_STEP,
    {
      id: 1,
      kind: "Simple",
      drivers: [
        { a: [1, 0], b: [0.5, 1], target_angle_deg: -180 },
      ],
      layer_order: null,
      note: "",
    },
  ],
};

/** sequence_applyへ渡された引数(FoldThrough) */
function lastFoldOp() {
  const calls = vi.mocked(ipc.sequenceApply).mock.calls;
  const op = calls[calls.length - 1][0];
  if (op.type !== "FoldThrough") throw new Error(`FoldThroughではない: ${op.type}`);
  return op;
}

function spatialPayload(op: unknown) {
  return (
    op as {
      spatial?: {
        from: [number, number, number];
        to: [number, number, number];
        grab_face: number;
        mode: string;
      };
    }
  ).spatial;
}

describe("foldByDrag", () => {
  beforeEach(() => {
    vi.mocked(ipc.sequenceApply).mockReset();
    vi.mocked(ipc.sequenceApply).mockResolvedValue(VIEW);
    useAppStore.setState({
      doc: DOC,
      faces: FACES,
      frame3d: null,
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      errorMessage: null,
      foldDraft: null,
      pendingFoldThrough: null,
      foldThroughBusy: false,
    });
  });

  it("つかんだ点から離した点へのドラッグで、その場で折れる", async () => {
    await useAppStore.getState().foldByDrag([0.25, 0.5], [0.75, 0.5], "flap");
    expect(vi.mocked(ipc.sequenceApply).mock.calls.map(([op]) => op.type)).toEqual([
      "PreviewFoldThrough",
      "FoldThrough",
    ]);
    const op = lastFoldOp();
    // 折り線は2点の垂直二等分線(x=0.5)、動かさない側は離した点
    expect(op.line[0][0]).toBeCloseTo(0.5);
    expect(op.keep_side_point).toEqual([0.75, 0.5]);
    expect(op.direction).toBe("Up");
    expect(op.target_layers).toEqual([0]);
    expect(op.accept_additional_crease).toBe(false);
    expect(spatialPayload(op)).toBeUndefined();
    expect(useAppStore.getState().errorMessage).toBeNull();
  });

  it("Shift(重なった紙を全部)は対象層を指定しない", async () => {
    await useAppStore.getState().foldByDrag([0.25, 0.5], [0.75, 0.5], "all");
    expect(lastFoldOp().target_layers).toBeNull();
  });

  it("立体経路の境界は全頂点の|z|が1e-6を超えたときだけにする", () => {
    const frameAtLimit: Frame3D = {
      faces: [
        {
          face: 0,
          polygon: [
            [0, 0, -1e-6],
            [1, 0, 1e-6],
            [1, 1, 0],
          ],
          layer: 0,
        },
      ],
      warnings: [],
    };
    expect(isSpatialFoldFrame(frameAtLimit)).toBe(false);
    expect(
      isSpatialFoldFrame({
        ...frameAtLimit,
        faces: [
          {
            ...frameAtLimit.faces[0],
            polygon: [
              [0, 0, -1e-6],
              [1, 0, 1.000001e-6],
              [1, 1, 0],
            ],
          },
        ],
      }),
    ).toBe(true);
  });

  it("追加折り目の候補があれば適用せず保持し、承諾後に同じ折りへ追加指定する", async () => {
    vi.mocked(ipc.sequenceApply)
      .mockResolvedValueOnce({
        ...VIEW,
        fold_through_proposal: {
          folded_line: [
            [0.4, 0.2],
            [0.4, 0.8],
          ],
          crease_segments: [
            [
              [0.6, 0.2],
              [0.6, 0.8],
            ],
          ],
          message: "縁に沿う追加折り目を入れると貫通を避けられます。",
        },
      })
      .mockResolvedValueOnce(VIEW);

    await useAppStore.getState().foldByDrag([0.25, 0.5], [0.75, 0.5], "flap");
    expect(vi.mocked(ipc.sequenceApply).mock.calls).toHaveLength(1);
    expect(vi.mocked(ipc.sequenceApply).mock.calls[0][0].type).toBe(
      "PreviewFoldThrough",
    );
    expect(useAppStore.getState().pendingFoldThrough?.proposal.crease_segments).toEqual([
      [
        [0.6, 0.2],
        [0.6, 0.8],
      ],
    ]);

    await useAppStore.getState().resolveFoldThroughProposal(true);
    const op = lastFoldOp();
    expect(op.accept_additional_crease).toBe(true);
    expect(op.line[0][0]).toBeCloseTo(0.5);
    expect(useAppStore.getState().pendingFoldThrough).toBeNull();
  });

  it("折れない状態では折らずに理由を出す", async () => {
    useAppStore.setState({ playing: true });
    await useAppStore.getState().foldByDrag([0.25, 0.5], [0.75, 0.5], "flap");
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
    expect(useAppStore.getState().errorMessage).toContain("再生中");
  });

  it("ドラッグが短すぎるときは折らずに促す", async () => {
    await useAppStore.getState().foldByDrag([0.5, 0.5], [0.5, 0.505], "flap");
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
    expect(useAppStore.getState().errorMessage).toContain("ドラッグ");
  });

  it("記録した立体姿勢から続けて折ると展開図・立体・手順が全て更新される", async () => {
    const beforeEdgeCount = NON_FLAT_DOC.cp.edges.length;
    vi.mocked(ipc.sequenceApply).mockImplementation(async (op) =>
      op.type === "PreviewFoldThrough"
        ? {
            ...VIEW,
            doc: NON_FLAT_DOC,
            faces: NON_FLAT_FACES,
            frame: NON_FLAT_FRAME,
          }
        : {
            ...VIEW,
            doc: AFTER_NON_FLAT_FOLD_DOC,
            faces: AFTER_NON_FLAT_FOLD_FACES,
            frame: AFTER_NON_FLAT_FOLD_FRAME,
          },
    );
    useAppStore.setState({
      doc: NON_FLAT_DOC,
      faces: NON_FLAT_FACES,
      frame3d: NON_FLAT_FRAME,
      currentStep: null,
      playT: 1,
      playing: false,
      drivers: new Map(),
      errorMessage: null,
      foldDraft: null,
      pendingFoldThrough: null,
      foldThroughBusy: false,
    });

    const spatial = {
      from: [0.5, 0.45, 0.4] as [number, number, number],
      to: [0.5, 0.35, 0.2] as [number, number, number],
      grab_face: 1,
      mode: "flap",
    };
    // 起こした右面の実際の点から動かす。垂直二等分面は新しい対角線(2--4)を通る。
    await useAppStore.getState().foldByDrag(
      spatial.from,
      spatial.to,
      "flap",
      spatial.grab_face,
      "Down",
    );

    const calls = vi.mocked(ipc.sequenceApply).mock.calls;
    expect(calls.map(([op]) => op.type)).toEqual([
      "PreviewFoldThrough",
      "FoldThrough",
    ]);
    expect(spatialPayload(calls[0][0])).toEqual(spatial);
    expect(spatialPayload(calls[1][0])).toEqual(spatial);
    expect((calls[0][0] as { direction?: string }).direction).toBe("Down");
    expect((calls[1][0] as { direction?: string }).direction).toBe("Down");
    const after = useAppStore.getState();
    expect(after.doc?.cp.edges.length).toBeGreaterThan(beforeEdgeCount);
    expect(after.frame3d).not.toEqual(NON_FLAT_FRAME);
    expect(after.doc?.sequence).toHaveLength(2);
    expect(after.errorMessage ?? "").not.toContain("折れる紙がありません");
    expect(after.errorMessage ?? "").not.toContain("折り途中の状態では折れません");
    expect(after.warnings.join("\n")).not.toContain("折れる紙がありません");
    expect(after.warnings.join("\n")).not.toContain("折り途中の状態では折れません");
  });
});
