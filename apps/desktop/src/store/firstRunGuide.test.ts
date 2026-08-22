// UI-012: ガイドは説明の「次へ」ではなく、4つの実操作が成功したときだけ進む。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Document, DocumentView, ReplayResult } from "../lib/types";

vi.mock("../ipc/client", () => ({
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
  editApply: vi.fn(),
  editApplyBatch: vi.fn(),
}));

import * as ipc from "../ipc/client";
import { DEFAULT_DISPLAY } from "../lib/displayPrefs";
import { resetPoseThrottle, useAppStore } from "./appStore";

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
    next_edge_id: 0,
  },
  sequence: [],
  display: DEFAULT_DISPLAY,
};

function view(doc: Document = DOC): DocumentView {
  return {
    doc,
    faces: [{ id: 0, vertices: [0, 1, 2, 3], edges: [0, 1, 2, 3] }],
    warnings: [],
    violations: [],
    frame: null,
    skipped: [],
    contact_detected: false,
  };
}

const REPLAY: ReplayResult = {
  frame: { faces: [], warnings: [] },
  skipped: [],
  warnings: [],
};

beforeEach(() => {
  vi.clearAllMocks();
  resetPoseThrottle();
  vi.mocked(ipc.poseSolve).mockResolvedValue({
    frame: { faces: [], warnings: [] },
    converged: true,
    angles: {},
    iterations: 1,
  });
  vi.mocked(ipc.sequenceReplay).mockResolvedValue(REPLAY);
  useAppStore.setState({
    doc: DOC,
    faces: view().faces,
    hinges: new Set([5]),
    activeTool: "fold",
    foldDraft: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    alignDraft: null,
    techniqueDraft: null,
    currentStep: null,
    playT: 1,
    playing: false,
    drivers: new Map(),
    poseAngles: new Map(),
    errorMessage: null,
    guideOpen: true,
    guideStep: 0,
    display: DEFAULT_DISPLAY,
  });
});

afterEach(() => {
  resetPoseThrottle();
  useAppStore.setState({ guideOpen: false, guideStep: 0, doc: null });
});

describe("初回ガイドの実操作判定", () => {
  it("順番外の操作では進まず、同じ操作の繰り返しも飛び越さない", () => {
    const s = useAppStore.getState();
    s.completeGuideAction("angle");
    expect(useAppStore.getState().guideStep).toBe(0);
    s.completeGuideAction("fold");
    s.completeGuideAction("fold");
    expect(useAppStore.getState().guideStep).toBe(1);
  });

  it("折りの事前確認と適用が成功して手順が増えたときだけ角度へ進む", async () => {
    const folded: Document = {
      ...DOC,
      sequence: [
        {
          id: 1,
          kind: "Simple",
          drivers: [{ a: [0.5, 0], b: [0.5, 1], target_angle_deg: -180 }],
          layer_order: null,
          note: "",
        },
      ],
    };
    vi.mocked(ipc.sequenceApply)
      .mockResolvedValueOnce(view())
      .mockResolvedValueOnce(view(folded));

    const s = useAppStore.getState();
    s.beginFoldDraft(
      [
        [0.5, 0],
        [0.5, 1],
      ],
      "3d",
    );
    await s.commitFoldDraft();

    expect(ipc.sequenceApply).toHaveBeenCalledTimes(2);
    expect(useAppStore.getState().doc?.sequence).toHaveLength(1);
    expect(useAppStore.getState().guideStep).toBe(1);
  });

  it("角度変更→実際に引いて離す→正の膨らみ指定の順で完了する", () => {
    const s = useAppStore.getState();
    s.completeGuideAction("fold");

    s.setDriverAngle(5, 30);
    expect(useAppStore.getState().guideStep).toBe(2);

    s.beginPull(5, new Map([[5, 30]]));
    s.pullTo(30.5); // 1度未満では「動かした」と数えない
    s.endPull();
    expect(useAppStore.getState().guideStep).toBe(2);
    s.beginPull(5, new Map([[5, 30.5]]));
    // pointer moveごとは1度未満でも、つかんだ位置から累計1度以上なら達成する。
    s.pullTo(30.9);
    s.pullTo(31.3);
    s.pullTo(31.7);
    s.endPull();
    expect(useAppStore.getState().guideStep).toBe(3);

    s.setSoft({ soft_enabled: true });
    expect(useAppStore.getState().guideStep).toBe(3);
    s.setSoft({ soft_pressure: 0.4 });
    expect(useAppStore.getState().guideStep).toBe(4);
  });
});
