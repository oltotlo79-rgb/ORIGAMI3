// 紙のたわみ(SIM-012 / SIM-013 / SIM-015)をストアから使えることのテスト。
//  - 既定はオフで、切っている間はIPCへ指定を送らない(従来どおりの動き)
//  - 入れると指定が付き、返ってきた網が表示状態へ入る
//  - 膨らみのつまみは60ms間引き経由で送られ、最後の値が必ず届く
//  - 設定は作品(.ori3)へ保存され、「この形で仕上げる」でも一緒に確定する

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Document, DocumentView, SoftMesh, SolveResult } from "../lib/types";

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
  recoveryCheck: vi.fn(),
  recoveryRestore: vi.fn(),
  proposalGenerate: vi.fn(),
}));

import * as ipc from "../ipc/client";
import { resetPoseThrottle, useAppStore } from "./appStore";
import { DEFAULT_DISPLAY } from "../lib/displayPrefs";

/** 間引き(60ms)と保存待ち(400ms)の両方を過ぎるまで待つ */
const WAIT_MS = 600;
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function makeDoc(): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: { vertices: [], edges: [], next_vertex_id: 0, next_edge_id: 0 },
    sequence: [],
    display: DEFAULT_DISPLAY,
  };
}

function makeView(doc: Document): DocumentView {
  return { doc, faces: [], warnings: [], violations: [], frame: null, skipped: [] };
}

const MESH: SoftMesh = {
  positions: [
    [0, 0, 0],
    [1, 0, 0],
    [0, 1, 0],
  ],
  triangles: [[0, 1, 2]],
  triangle_faces: [0],
  triangle_layers: [0],
  warnings: ["展開図が大きいため、たわみの分割の細かさを1へ自動で落としました"],
};

function solveResult(soft: SoftMesh | null): SolveResult {
  return {
    frame: { faces: [], warnings: [] },
    converged: true,
    angles: {},
    iterations: 1,
    soft,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  resetPoseThrottle();
  useAppStore.setState({
    doc: makeDoc(),
    display: DEFAULT_DISPLAY,
    softMesh: null,
    softWarnings: [],
    drivers: new Map(),
    frame3d: null,
    currentStep: null,
    errorMessage: null,
  });
  vi.mocked(ipc.poseSolve).mockResolvedValue(solveResult(null));
  vi.mocked(ipc.editApply).mockImplementation(async (op) =>
    makeView({
      ...makeDoc(),
      display: op.type === "SetDisplay" ? op.display : DEFAULT_DISPLAY,
    }),
  );
});

describe("紙のたわみ(SIM-012)", () => {
  it("既定はオフで、切っている間はIPCへ指定を送らない", async () => {
    expect(useAppStore.getState().display.soft_enabled).toBe(false);
    useAppStore.getState().setDriverAngle(1, 90);
    await wait(WAIT_MS);
    expect(vi.mocked(ipc.poseSolve).mock.calls[0][1]).toBeNull();
    expect(useAppStore.getState().softMesh).toBeNull();
  });

  it("入れると指定が付き、返ってきた網が画面の状態へ入る", async () => {
    vi.mocked(ipc.poseSolve).mockResolvedValue(solveResult(MESH));
    useAppStore.getState().setSoft({ soft_enabled: true });
    await wait(WAIT_MS);
    const sent = vi.mocked(ipc.poseSolve).mock.calls[0][1];
    expect(sent).toMatchObject({ enabled: true, stiffness: 0.5, pressure: 0 });
    expect(useAppStore.getState().softMesh).toEqual(MESH);
    // 計算からの注意書きは日本語のまま画面へ渡る
    expect(useAppStore.getState().softWarnings).toEqual(MESH.warnings);
  });

  it("切ると網をすぐ捨てて従来の描き方へ戻る", async () => {
    useAppStore.setState({ softMesh: MESH, softWarnings: MESH.warnings });
    useAppStore.getState().setSoft({ soft_enabled: false });
    expect(useAppStore.getState().softMesh).toBeNull();
    expect(useAppStore.getState().softWarnings).toEqual([]);
    await wait(WAIT_MS);
  });
});

describe("膨らませる操作(SIM-013)", () => {
  it("つまみを続けて動かしても送るのは間引いた最後の値だけ", async () => {
    useAppStore.setState({ display: { ...DEFAULT_DISPLAY, soft_enabled: true } });
    for (const p of [0.2, 0.4, 0.6, 0.8]) {
      useAppStore.getState().setSoft({ soft_pressure: p });
    }
    // つまみの位置はその場で映る(結果を見ながら調整できる)
    expect(useAppStore.getState().display.soft_pressure).toBe(0.8);
    await wait(WAIT_MS);
    // 途中の値(0.2〜0.6)は1回も送られず、最後の値だけが届く
    const sent = vi
      .mocked(ipc.poseSolve)
      .mock.calls.map((c) => c[1]?.pressure)
      .filter((p) => p !== undefined);
    expect(sent.length).toBeGreaterThan(0);
    expect(sent.every((p) => p === 0.8)).toBe(true);
  });

  it("範囲の外を指定しても0.0〜1.0に収まる", async () => {
    useAppStore.getState().setSoft({ soft_pressure: 5, soft_stiffness: -1 });
    expect(useAppStore.getState().display.soft_pressure).toBe(1);
    expect(useAppStore.getState().display.soft_stiffness).toBe(0);
    await wait(WAIT_MS);
  });
});

describe("たわみの設定の保存(SIM-015)", () => {
  it("作品ごとの設定として保存される(頂点の位置は保存しない)", async () => {
    useAppStore.getState().setSoft({ soft_enabled: true, soft_pressure: 0.3 });
    await wait(WAIT_MS);
    const calls = vi.mocked(ipc.editApply).mock.calls;
    const op = calls.length > 0 ? calls[calls.length - 1][0] : undefined;
    if (!op || op.type !== "SetDisplay") throw new Error("SetDisplayが送られていない");
    expect(op.display.soft_enabled).toBe(true);
    expect(op.display.soft_pressure).toBe(0.3);
    // 頂点の位置そのものは作品へ入らない
    expect(JSON.stringify(op.display)).not.toContain("positions");
  });

  it("「この形で仕上げる」でたわみのパラメータも一緒に確定する", async () => {
    vi.mocked(ipc.sequenceApply).mockImplementation(async () => makeView(makeDoc()));
    // つまみを動かした直後(まだ書き込み待ち)に仕上げても取りこぼさない
    useAppStore.getState().setSoft({ soft_enabled: true, soft_pressure: 0.4 });
    useAppStore.setState({
      hinges: new Set([1]),
      drivers: new Map([[1, 90]]),
      poseAngles: new Map(),
      playing: false,
    });
    await useAppStore.getState().recordPoseStep();
    const calls = vi.mocked(ipc.editApply).mock.calls;
    const op = calls.length > 0 ? calls[calls.length - 1][0] : undefined;
    if (!op || op.type !== "SetDisplay") throw new Error("SetDisplayが送られていない");
    expect(op.display.soft_enabled).toBe(true);
    expect(op.display.soft_pressure).toBe(0.4);
    expect(ipc.sequenceApply).toHaveBeenCalled();
    await wait(WAIT_MS);
  });

  it("作品をまだ開いていないときは画面の表示だけ変える(送らない)", async () => {
    useAppStore.setState({ doc: null });
    useAppStore.getState().setSoft({ soft_enabled: true });
    await wait(WAIT_MS);
    expect(ipc.editApply).not.toHaveBeenCalled();
    expect(useAppStore.getState().display.soft_enabled).toBe(true);
  });
});
