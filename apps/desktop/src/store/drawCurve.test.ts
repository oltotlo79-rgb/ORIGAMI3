// 曲線の折り目(CPE-011)のストア側テスト。
// 曲線は折れ線として1本ずつ送られ、設定に応じて「紙が曲がるための線」も続く。

import { beforeEach, describe, expect, it, vi } from "vitest";
import { arcPolyline, DEFAULT_CURVE } from "../lib/curve";
import type { Document, DocumentView, EditOp, Vec2 } from "../lib/types";

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

/** 正方形(輪郭4辺だけ)の作品 */
const DOC: Document = {
  schema_version: 1,
  paper: { width_mm: 100, height_mm: 100 },
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
    ],
    next_vertex_id: 4,
    next_edge_id: 4,
  },
  sequence: [],
  display: { front_color: [230, 90, 60], back_color: [245, 245, 245], grid_divisions: 8 },
};

const VIEW: DocumentView = {
  doc: DOC,
  faces: [],
  warnings: [],
  violations: [],
  frame: null,
  skipped: [],
};

function addedSegments(): Extract<EditOp, { type: "AddSegment" }>[] {
  return vi
    .mocked(ipc.editApply)
    .mock.calls.map(([op]) => op)
    .filter((op): op is Extract<EditOp, { type: "AddSegment" }> => op.type === "AddSegment");
}

const ARC: Vec2[] = arcPolyline([0, 0.25], [0.5, 0.55], [1, 0.25], 0.005);

describe("曲線の折り目を引く", () => {
  beforeEach(() => {
    vi.mocked(ipc.editApply).mockReset();
    vi.mocked(ipc.editApply).mockResolvedValue(VIEW);
    useAppStore.setState({
      doc: DOC,
      faces: [],
      errorMessage: null,
      mirrorDraw: false,
      curve: { ...DEFAULT_CURVE, enabled: true },
    });
  });

  it("曲線は折れ線の区間の数だけ線として送られる", async () => {
    useAppStore.setState({ curve: { ...DEFAULT_CURVE, enabled: true, rulings: false } });
    await useAppStore.getState().drawCurve(ARC, "Valley");
    const segs = addedSegments();
    expect(segs).toHaveLength(ARC.length - 1);
    expect(segs.every((s) => s.kind === "Valley")).toBe(true);
    expect(segs[0].a).toEqual(ARC[0]);
    expect(segs[segs.length - 1].b).toEqual(ARC[ARC.length - 1]);
  });

  it("曲がるための線も引く設定なら、折り目の両側へ線が足される", async () => {
    await useAppStore.getState().drawCurve(ARC, "Valley");
    const segs = addedSegments();
    // 曲線の区間 + 内側の点ごとに2本(へこむ側・膨らむ側)
    expect(segs).toHaveLength(ARC.length - 1 + (ARC.length - 2) * 2);
    // へこむ側は曲線と反対の線種にする(曲がる向きが逆になるため)
    expect(segs.filter((s) => s.kind === "Mountain")).toHaveLength(ARC.length - 2);
  });

  it("補助線には曲がるための線を付けない(折りに関与しないため)", async () => {
    await useAppStore.getState().drawCurve(ARC, "Aux");
    expect(addedSegments()).toHaveLength(ARC.length - 1);
  });

  it("途中で断られたら理由を残してそこで止める", async () => {
    vi.mocked(ipc.editApply).mockRejectedValue("線を引けません");
    await useAppStore.getState().drawCurve(ARC, "Valley");
    expect(addedSegments()).toHaveLength(1);
    expect(useAppStore.getState().errorMessage).not.toBeNull();
  });

  it("左右対称に描く設定なら反対側にも同じ曲線が引かれる", async () => {
    useAppStore.setState({
      mirrorDraw: true,
      curve: { ...DEFAULT_CURVE, enabled: true, rulings: false },
    });
    await useAppStore.getState().drawCurve(ARC, "Valley");
    // 中心線をまたぐ区間は1本にまとまるので、区間数の1倍より多く2倍以下になる
    const n = addedSegments().length;
    expect(n).toBeGreaterThan(ARC.length - 1);
    expect(n).toBeLessThanOrEqual((ARC.length - 1) * 2);
  });
});
