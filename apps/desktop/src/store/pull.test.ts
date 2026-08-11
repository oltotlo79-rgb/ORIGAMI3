// 「3Dの紙をつかんで引く」操作(UI-007)のストア側テスト。
//   - 引き始めに、今見えている形の全ての折り角をwarm seedとして送る
//     (固定条件にせず、ソルバーの出発点だけを今の形へ合わせる)
//   - 引いている間の角度は16ms間引きで、駆動する1本だけが送られる
//   - 離しても形(角度指定)は残り、色付けだけ消える

import { beforeEach, describe, expect, it, vi } from "vitest";
import { planPull, pullDeltaDeg } from "../lib/grabDrive";
import type { Document, Driver, Face, Frame3D, SolveResult } from "../lib/types";

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
import { pullBlockReason, resetPoseThrottle, useAppStore } from "./appStore";

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

const SOLVED: SolveResult = {
  frame: { faces: [], warnings: [] },
  converged: true,
  angles: {},
  iterations: 1,
};

/** 手順再生が作った立体表示(層の重なり付き)の代わり */
const FOLDED: Frame3D = {
  faces: [
    { face: 0, polygon: [[0, 0, 0], [1, 0, 0], [1, 1, 0]], layer: 0 },
    { face: 1, polygon: [[0, 0, 0], [1, 1, 0], [1, 0, 0]], layer: 1 },
  ],
  warnings: [],
};

function poseCalls(): Driver[][] {
  return vi.mocked(ipc.poseSolve).mock.calls.map(([drivers]) => drivers);
}

function warmCalls(): (Driver[] | null | undefined)[] {
  return vi.mocked(ipc.poseSolve).mock.calls.map(([, , , warm]) => warm);
}

function preferredCalls(): Driver[][] {
  return vi.mocked(ipc.poseSolve).mock.calls.map(([, preferred]) => preferred ?? []);
}

describe("紙をつかんで引く", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetPoseThrottle();
    vi.mocked(ipc.poseSolve).mockResolvedValue(SOLVED);
    useAppStore.setState({
      doc: DOC,
      faces: FACES,
      hinges: new Set([5]),
      frame3d: FOLDED,
      drivers: new Map(),
      poseAngles: new Map(),
      sequenceTargets: new Map(),
      relaxations: [],
      activeAngleIntent: null,
      angleIntentGeneration: 0,
      pullHinge: null,
      pullMirrorHinge: null,
      pullMirror: true,
      currentStep: null,
      playT: 1,
      playing: false,
      errorMessage: null,
    });
  });

  it("引き始めに、今の形の折り角をそのまま送って出発点を合わせる", async () => {
    // 手順で折り上げた形(辺5が-180°)からつかんだ場合
    useAppStore.getState().beginPull(5, new Map([[5, -180]]));
    expect(useAppStore.getState().pullHinge).toBe(5);
    await Promise.resolve();
    await Promise.resolve();
    expect(poseCalls()).toEqual([[]]);
    expect(warmCalls()).toEqual([[{ hinge: 5, target_angle_deg: -180 }]]);
    // 出発点を合わせるだけなので、角度指定としては残さない
    expect(useAppStore.getState().drivers.size).toBe(0);
    // 形は変わらないので、手順再生が作った立体表示(層の重なり)は消さない
    expect(useAppStore.getState().frame3d).toBe(FOLDED);
  });

  it("引いている間は16ms間引きで、駆動する1本だけが送られる", async () => {
    vi.useFakeTimers();
    try {
      const store = useAppStore.getState();
      store.beginPull(5, new Map());
      store.pullTo(-170);
      store.pullTo(-160);
      store.pullTo(-150);
      expect(poseCalls()).toHaveLength(0); // まだ送っていない
      await vi.advanceTimersByTimeAsync(100);
      expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: -150 }]]);
      expect(useAppStore.getState().drivers.get(5)).toBe(-150);
    } finally {
      vi.useRealTimers();
    }
  });

  it("保存手順の希望角をpreferredに保ち、つかんだ同じ辺はhardだけにする", async () => {
    vi.useFakeTimers();
    try {
      useAppStore.setState({
        hinges: new Set([5, 7, 9]),
        sequenceTargets: new Map([
          [5, 90],
          [7, 45],
        ]),
        drivers: new Map([[9, 25]]),
      });
      const store = useAppStore.getState();
      store.beginPull(5, new Map());
      store.pullTo(100);
      await vi.advanceTimersByTimeAsync(100);

      expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: 100 }]]);
      expect(preferredCalls()).toEqual([
        [
          { hinge: 7, target_angle_deg: 45 },
          { hinge: 9, target_angle_deg: 25 },
        ],
      ]);
      expect(
        preferredCalls()[0].filter((driver) => driver.hinge === 5),
      ).toHaveLength(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("表示実角をwarmにし、保存希望は引き始めから全pointer moveまでpreferredに保つ", async () => {
    vi.useFakeTimers();
    try {
      useAppStore.setState({
        hinges: new Set([5, 7]),
        sequenceTargets: new Map([
          [5, 90],
          [7, 45],
        ]),
      });
      const store = useAppStore.getState();
      store.beginPull(
        5,
        new Map([
          [5, 80],
          [7, 40],
        ]),
      );
      await Promise.resolve();
      await Promise.resolve();

      expect(poseCalls()[0]).toEqual([]);
      expect(preferredCalls()[0]).toEqual([
        { hinge: 5, target_angle_deg: 90 },
        { hinge: 7, target_angle_deg: 45 },
      ]);
      expect(warmCalls()[0]).toEqual([
        { hinge: 5, target_angle_deg: 80 },
        { hinge: 7, target_angle_deg: 40 },
      ]);

      store.pullTo(100);
      await vi.advanceTimersByTimeAsync(100);
      expect(poseCalls()[1]).toEqual([{ hinge: 5, target_angle_deg: 100 }]);
      expect(preferredCalls()[1]).toEqual([{ hinge: 7, target_angle_deg: 45 }]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("離しても形は残り、色付けだけ消える", () => {
    const store = useAppStore.getState();
    store.beginPull(5, new Map());
    store.pullTo(-150);
    store.endPull();
    expect(useAppStore.getState().pullHinge).toBeNull();
    expect(useAppStore.getState().drivers.get(5)).toBe(-150);
  });

  it("つかんでいないときは角度を変えない", () => {
    useAppStore.getState().pullTo(-150);
    expect(useAppStore.getState().drivers.size).toBe(0);
  });

  it("左右同時なら、対称の相手の折り線も同じ角度で一緒に送られる", async () => {
    vi.useFakeTimers();
    try {
      useAppStore.setState({ pullMirror: true });
      useAppStore.setState({ hinges: new Set([5, 7]) });
      const store = useAppStore.getState();
      // 辺5(つかんだ折り線)と、その左右対称の相手として辺7
      store.beginPull(5, new Map(), 7);
      expect(useAppStore.getState().pullMirrorHinge).toBe(7);
      store.pullTo(-150);
      await vi.advanceTimersByTimeAsync(100);
      expect(poseCalls()).toEqual([
        [
          { hinge: 5, target_angle_deg: -150 },
          { hinge: 7, target_angle_deg: -150 },
        ],
      ]);
      // 離すと色付けは消えるが、両方の角度指定は残る
      store.endPull();
      expect(useAppStore.getState().pullMirrorHinge).toBeNull();
      expect(useAppStore.getState().drivers.get(5)).toBe(-150);
      expect(useAppStore.getState().drivers.get(7)).toBe(-150);
    } finally {
      vi.useRealTimers();
    }
  });

  it("左右同時を切っていれば、相手が見つかっていても1本だけ送る", async () => {
    vi.useFakeTimers();
    try {
      useAppStore.setState({ pullMirror: false });
      const store = useAppStore.getState();
      store.beginPull(5, new Map(), 7);
      expect(useAppStore.getState().pullMirrorHinge).toBeNull();
      store.pullTo(-150);
      await vi.advanceTimersByTimeAsync(100);
      expect(poseCalls()).toEqual([[{ hinge: 5, target_angle_deg: -150 }]]);
      expect(useAppStore.getState().drivers.has(7)).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("引いている最中に左右同時を切ると、その場で相手が外れる", () => {
    useAppStore.setState({ pullMirror: true });
    useAppStore.getState().beginPull(5, new Map(), 7);
    useAppStore.getState().setPullMirror(false);
    expect(useAppStore.getState().pullMirrorHinge).toBeNull();
    useAppStore.getState().pullTo(-150);
    expect(useAppStore.getState().drivers.has(7)).toBe(false);
  });

  // 根の面(ソルバーが固定する基準の面)をつかんでも動かせること。
  // planPullが根に接する折り線を選ぶので、ストアはふつうに角度を送れる
  it("根の面をつかんでも、選ばれた折り線の角度が送られる", async () => {
    vi.useFakeTimers();
    try {
      const plan = planPull(DOC, FACES, FOLDED, 0, [0.5, 0.2, 0], [0, 0, 1]);
      expect(plan).not.toBeNull();
      const store = useAppStore.getState();
      store.beginPull(plan!.hinge, new Map());
      store.pullTo(plan!.baseDeg + pullDeltaDeg(plan!.velocity, [0, 0, 0.2]));
      await vi.advanceTimersByTimeAsync(100);
      expect(poseCalls()).toHaveLength(1);
      expect(poseCalls()[0][0].hinge).toBe(plan!.hinge);
      expect(useAppStore.getState().drivers.get(plan!.hinge)).not.toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("紙が無ければ引き始められない", () => {
    useAppStore.setState({ doc: null });
    useAppStore.getState().beginPull(5, new Map([[5, 0]]));
    expect(useAppStore.getState().pullHinge).toBeNull();
  });
});

describe("pullBlockReason", () => {
  const READY = {
    doc: DOC,
    playing: false,
    playT: 1,
    hingeCount: 1,
    currentStep: null,
    stepCount: 3,
  };

  it("折り上がった作品(最新の形)でも引ける", () => {
    expect(pullBlockReason(READY)).toBeNull();
    expect(pullBlockReason({ ...READY, currentStep: 3 })).toBeNull();
  });

  it("再生中・折り途中・前の手順・折り線なしは理由を返す", () => {
    expect(pullBlockReason({ ...READY, playing: true })).toContain("再生中");
    expect(pullBlockReason({ ...READY, playT: 0.5 })).toContain("折り途中");
    expect(pullBlockReason({ ...READY, currentStep: 1 })).toContain("前の手順");
    expect(pullBlockReason({ ...READY, hingeCount: 0 })).toContain("折り線");
    expect(pullBlockReason({ ...READY, doc: null })).toContain("紙がありません");
  });
});
