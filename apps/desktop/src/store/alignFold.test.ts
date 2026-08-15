// 「合わせて折る」のストア側のテスト:
//  - 選択が順に進み、そろった時点で折り線が求まること
//  - 求まった折り線がそのままFoldThroughの引数になること(動かす側も含めて)
//  - 解が2つあるときの切り替え・1つ戻す・やめる

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentView, ReplayResult, Vec2 } from "../lib/types";

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
  recoveryCheck: vi.fn(),
  recoveryRestore: vi.fn(),
  proposalGenerate: vi.fn(),
}));

import * as ipc from "../ipc/client";
import { useAppStore } from "./appStore";

/** 単位正方形1枚・手順1つの、平らに畳んだ状態(折る操作ができる状態) */
function seedFlat(): void {
  const doc: DocumentView["doc"] = {
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
      ],
      next_vertex_id: 4,
      next_edge_id: 4,
    },
    sequence: [
      { id: 1, kind: "Simple", drivers: [], layer_order: null, note: "" },
    ],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
  useAppStore.setState({
    doc,
    faces: [{ id: 0, vertices: [0, 1, 2, 3], edges: [0, 1, 2, 3] }],
    hinges: new Set<number>(),
    activeTool: "fold",
    currentStep: null,
    playT: 1,
    playing: false,
    drivers: new Map(),
    foldDraft: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    alignDraft: null,
    errorMessage: null,
    frame3d: {
      faces: [
        {
          face: 0,
          polygon: [
            [0, 0, 0],
            [1, 0, 0],
            [1, 1, 0],
            [0, 1, 0],
          ],
          layer: 0,
        },
      ],
      warnings: [],
    },
  });
}

const XAXIS = { kind: "line" as const, a: [0, 0] as Vec2, b: [1, 0] as Vec2 };
const YAXIS = { kind: "line" as const, a: [0, 0] as Vec2, b: [0, 1] as Vec2 };

beforeEach(() => {
  vi.clearAllMocks();
  seedFlat();
});

describe("選択の進行", () => {
  it("点と点: 2つ選ぶまで折り線は出ず、そろうと垂直二等分線になる", () => {
    const s = useAppStore.getState();
    s.beginAlign("pointPoint");
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(0);

    s.pickAlignTarget({ kind: "point", p: [0, 0] });
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(1);
    expect(useAppStore.getState().foldDraft).toBeNull();

    s.pickAlignTarget({ kind: "point", p: [1, 1] });
    const draft = useAppStore.getState().alignDraft!;
    expect(draft.picks).toHaveLength(2);
    expect(draft.solutions).toHaveLength(1);
    // 折り線は y = 1 - x(両端点で x + y = 1)
    for (const p of useAppStore.getState().foldDraft!.line) {
      expect(p[0] + p[1]).toBeCloseTo(1, 9);
    }
  });

  it("種類の違う対象は受け付けない(点を選ぶところで線を選んでも進まない)", () => {
    const s = useAppStore.getState();
    s.beginAlign("pointPoint");
    s.pickAlignTarget(XAXIS);
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(0);
  });

  it("選び終えたあとにもう一度選ぶと1つ目から選び直す", () => {
    const s = useAppStore.getState();
    s.beginAlign("pointPoint");
    s.pickAlignTarget({ kind: "point", p: [0, 0] });
    s.pickAlignTarget({ kind: "point", p: [1, 1] });
    s.pickAlignTarget({ kind: "point", p: [0.5, 0] });
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(1);
    expect(useAppStore.getState().foldDraft).toBeNull();
  });

  it("同じ合わせ方をもう一度選ぶとやめる", () => {
    const s = useAppStore.getState();
    s.beginAlign("lineLine");
    expect(useAppStore.getState().alignDraft).not.toBeNull();
    useAppStore.getState().beginAlign("lineLine");
    expect(useAppStore.getState().alignDraft).toBeNull();
  });

  it("2点を通る: 選んだ2点を正確に通る折り線を作る", () => {
    const s = useAppStore.getState();
    s.beginAlign("throughTwoPoints");
    s.pickAlignTarget({ kind: "point", p: [0.125, 0.25] });
    s.pickAlignTarget({ kind: "point", p: [0.875, 0.75] });
    const line = useAppStore.getState().foldDraft!.line;
    const distance = (p: Vec2) => {
      const dx = line[1][0] - line[0][0];
      const dy = line[1][1] - line[0][1];
      return Math.abs(dx * (p[1] - line[0][1]) - dy * (p[0] - line[0][0])) /
        Math.hypot(dx, dy);
    };
    expect(distance([0.125, 0.25])).toBeLessThan(1e-12);
    expect(distance([0.875, 0.75])).toBeLessThan(1e-12);
  });

  it("点を通り垂直: 点→線の2選択で厳密な垂線を作る", () => {
    const s = useAppStore.getState();
    s.beginAlign("pointPerpendicularLine");
    s.pickAlignTarget({ kind: "point", p: [0.25, 0.5] });
    s.pickAlignTarget(XAXIS);
    const line = useAppStore.getState().foldDraft!.line;
    expect(line[0][0]).toBeCloseTo(0.25, 12);
    expect(line[1][0]).toBeCloseTo(0.25, 12);
  });

  it("2組を同時に合わせる: 4選択がそろうまで待ち、最大3解を保持する", () => {
    const s = useAppStore.getState();
    s.beginAlign("pointToLinePointToLine");
    s.pickAlignTarget({ kind: "point", p: [0, 0] });
    s.pickAlignTarget({ kind: "line", a: [1, -1], b: [1, 2] });
    s.pickAlignTarget({ kind: "point", p: [-1, 1] });
    expect(useAppStore.getState().foldDraft).toBeNull();
    s.pickAlignTarget({ kind: "line", a: [1, 0], b: [2, 1] });
    const draft = useAppStore.getState().alignDraft!;
    expect(draft.picks).toHaveLength(4);
    expect(draft.solutions.length).toBeGreaterThan(0);
    expect(draft.solutions.length).toBeLessThanOrEqual(3);
    expect(useAppStore.getState().foldDraft).not.toBeNull();
  });

  it("点→線+垂直と既存線は、それぞれ3選択・1選択で折り線になる", () => {
    const s = useAppStore.getState();
    s.beginAlign("pointLinePerpendicular");
    s.pickAlignTarget({ kind: "point", p: [0, 2] });
    s.pickAlignTarget(XAXIS);
    s.pickAlignTarget(YAXIS);
    let line = useAppStore.getState().foldDraft!.line;
    expect(line[0][1]).toBeCloseTo(1, 12);
    expect(line[1][1]).toBeCloseTo(1, 12);

    useAppStore.getState().beginAlign("existingLine");
    useAppStore.getState().pickAlignTarget({
      kind: "line",
      a: [0, 0.375],
      b: [1, 0.375],
    });
    line = useAppStore.getState().foldDraft!.line;
    expect(line[0][1]).toBeCloseTo(0.375, 12);
    expect(line[1][1]).toBeCloseTo(0.375, 12);
  });
});

describe("求まった折り線でFoldThroughを送る", () => {
  it("1つ目に選んだ点がある側が動く(動かさない側の点は2つ目の点の側)", async () => {
    vi.mocked(ipc.sequenceApply).mockResolvedValue({
      doc: useAppStore.getState().doc!,
      faces: [],
      warnings: [],
      violations: [],
      frame: { faces: [], warnings: [] },
      skipped: [],
    });
    const s = useAppStore.getState();
    s.beginAlign("pointPoint");
    s.pickAlignTarget({ kind: "point", p: [0, 0] });
    s.pickAlignTarget({ kind: "point", p: [1, 1] });
    const line = useAppStore.getState().foldDraft!.line;

    await useAppStore.getState().commitFoldDraft();

    expect(vi.mocked(ipc.sequenceApply).mock.calls.map(([op]) => op.type)).toEqual([
      "PreviewFoldThrough",
      "FoldThrough",
    ]);
    const op = vi.mocked(ipc.sequenceApply).mock.calls[1][0];
    if (op.type !== "FoldThrough") throw new Error("FoldThroughでない");
    expect(op.up_to).toBe(1); // 手順1つの末尾へ足す
    expect(op.line).toEqual(line);
    expect(op.target_layers).toBeNull(); // 既定は全ての層
    expect(op.direction).toBe("Up");
    expect(op.alignment).toEqual({
      mode: "pointPoint",
      picks: [
        { kind: "point", p: [0, 0] },
        { kind: "point", p: [1, 1] },
      ],
    });
    // (0,0)が(1,1)へ重なるので、動かさない側は(1,1)の側
    expect(op.keep_side_point[0]).toBeGreaterThan(0.5);
    expect(op.keep_side_point[1]).toBeGreaterThan(0.5);
    // 折り終えたら合わせの途中経過も捨てる
    expect(useAppStore.getState().alignDraft).toBeNull();
    expect(useAppStore.getState().foldDraft).toBeNull();
  });
});

describe("解が2つあるとき", () => {
  it("線と線: 2本の解が出て、「別の解」で切り替わる", () => {
    const s = useAppStore.getState();
    s.beginAlign("lineLine");
    s.pickAlignTarget(XAXIS, [1, 1]);
    s.pickAlignTarget(YAXIS, [1, 1]);
    const draft = useAppStore.getState().alignDraft!;
    expect(draft.solutions).toHaveLength(2);
    expect(draft.solutionIndex).toBe(0);
    // カーソル(1,1)に近い y=x が既定
    const first = useAppStore.getState().foldDraft!.line;
    expect(first[0][0] - first[0][1]).toBeCloseTo(0, 9);

    useAppStore.getState().nextAlignSolution();
    expect(useAppStore.getState().alignDraft?.solutionIndex).toBe(1);
    // もう1本は y = -x
    const second = useAppStore.getState().foldDraft!.line;
    expect(second[0][0] + second[0][1]).toBeCloseTo(0, 9);

    // 一周して戻る
    useAppStore.getState().nextAlignSolution();
    expect(useAppStore.getState().alignDraft?.solutionIndex).toBe(0);
  });

  it("「別の解」でも、パネルで決めた向き・対象の層は引き継ぐ", () => {
    const s = useAppStore.getState();
    s.beginAlign("lineLine");
    s.pickAlignTarget(XAXIS, [1, 1]);
    s.pickAlignTarget(YAXIS, [1, 1]);
    useAppStore.getState().updateFoldDraft({ direction: "Down", target: "top" });
    useAppStore.getState().nextAlignSolution();
    expect(useAppStore.getState().foldDraft?.direction).toBe("Down");
    expect(useAppStore.getState().foldDraft?.target).toBe("top");
  });
});

describe("折れないとき・やり直し", () => {
  it("点を線に合わせる: 届かないときは理由を残し、折り線は出さない", () => {
    const s = useAppStore.getState();
    s.beginAlign("pointLineThrough");
    s.pickAlignTarget({ kind: "point", p: [0, 1] });
    s.pickAlignTarget(XAXIS);
    s.pickAlignTarget({ kind: "point", p: [0, 3] });
    const draft = useAppStore.getState().alignDraft!;
    expect(draft.solutions).toHaveLength(0);
    expect(draft.reason).toContain("届きません");
    expect(useAppStore.getState().foldDraft).toBeNull();
  });

  it("1つ戻すと直前の選択が消え、折り線も消える", () => {
    const s = useAppStore.getState();
    s.beginAlign("pointPoint");
    s.pickAlignTarget({ kind: "point", p: [0, 0] });
    s.pickAlignTarget({ kind: "point", p: [1, 1] });
    expect(useAppStore.getState().foldDraft).not.toBeNull();

    useAppStore.getState().undoAlignPick();
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(1);
    expect(useAppStore.getState().alignDraft?.solutions).toHaveLength(0);
    expect(useAppStore.getState().foldDraft).toBeNull();

    useAppStore.getState().undoAlignPick();
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(0);
    // 何も選んでいなければ何も起きない
    useAppStore.getState().undoAlignPick();
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(0);
  });

  it("やめると途中経過も折り線も消える", () => {
    const s = useAppStore.getState();
    s.beginAlign("pointPoint");
    s.pickAlignTarget({ kind: "point", p: [0, 0] });
    s.pickAlignTarget({ kind: "point", p: [1, 1] });
    useAppStore.getState().cancelAlign();
    expect(useAppStore.getState().alignDraft).toBeNull();
    expect(useAppStore.getState().foldDraft).toBeNull();
  });

  it("別のツールへ移ると合わせモードも解除される", () => {
    useAppStore.getState().beginAlign("pointPoint");
    useAppStore.getState().setTool("select");
    expect(useAppStore.getState().alignDraft).toBeNull();
  });

  it("手順を切り替えると途中選択を捨て、表示の更新中は古い位置で選び始めない", async () => {
    let finishReplay!: (view: ReplayResult) => void;
    vi.mocked(ipc.sequenceReplay).mockReturnValueOnce(
      new Promise<ReplayResult>((resolve) => {
        finishReplay = resolve;
      }),
    );
    const state = useAppStore.getState();
    state.beginAlign("lineLine");
    state.pickAlignTarget(XAXIS, null, { kind: "edge", id: 0 });
    expect(useAppStore.getState().alignDraft?.cpPicks).toEqual([
      { kind: "edge", id: 0 },
    ]);

    const switching = state.selectStepForCapture(0);
    expect(useAppStore.getState().alignDraft).toBeNull();
    expect(useAppStore.getState().foldDraft).toBeNull();

    useAppStore.getState().beginAlign("lineLine");
    expect(useAppStore.getState().alignDraft).toBeNull();
    expect(useAppStore.getState().errorMessage).toContain("表示を切り替えています");

    const current = useAppStore.getState();
    finishReplay({
      frame: current.frame3d!,
      warnings: [],
      skipped: [],
    });
    await switching;

    useAppStore.getState().beginAlign("lineLine");
    expect(useAppStore.getState().alignDraft?.mode).toBe("lineLine");
    expect(useAppStore.getState().errorMessage).toBeNull();
  });
});
