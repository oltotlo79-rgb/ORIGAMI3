// 紙をつかんで動かす折り操作(UI-007)のストア側テスト。
// ドラッグの2点が、そのままFoldThroughの折り線・動かさない側になることを確かめる。

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Document, DocumentView, Face } from "../lib/types";

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
import { useAppStore } from "./appStore";

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

/** sequence_applyへ渡された引数(FoldThrough) */
function lastFoldOp() {
  const calls = vi.mocked(ipc.sequenceApply).mock.calls;
  const op = calls[calls.length - 1][0];
  if (op.type !== "FoldThrough") throw new Error(`FoldThroughではない: ${op.type}`);
  return op;
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
    expect(useAppStore.getState().errorMessage).toBeNull();
  });

  it("Shift(重なった紙を全部)は対象層を指定しない", async () => {
    await useAppStore.getState().foldByDrag([0.25, 0.5], [0.75, 0.5], "all");
    expect(lastFoldOp().target_layers).toBeNull();
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
});
