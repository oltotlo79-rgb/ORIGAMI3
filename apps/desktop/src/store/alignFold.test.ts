// 「合わせて折る」のストア側のテスト:
//  - 選択が順に進み、そろった時点で折り線が求まること
//  - 求まった折り線がそのままFoldThroughの引数になること(動かす側も含めて)
//  - 解が2つあるときの切り替え・1つ戻す・やめる

import { beforeEach, describe, expect, it, vi } from "vitest";

const solveAlignCounter = vi.hoisted(() => ({ count: 0 }));

vi.mock("../lib/alignFold", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/alignFold")>();
  return {
    ...actual,
    solveAlign: (...args: Parameters<typeof actual.solveAlign>) => {
      solveAlignCounter.count += 1;
      return actual.solveAlign(...args);
    },
  };
});

import {
  ALIGN_STEPS,
  solveAlign,
  type AlignMode,
  type AlignTarget,
  type FoldLine,
} from "../lib/alignFold";
import type {
  SpatialAlignTarget,
  SpatialFoldTarget,
  SpatialSupportPlane,
} from "../lib/spatialAlignTypes";
import type { DocumentView, ReplayResult, Vec2 } from "../lib/types";
import type { SpatialMaterialForMovingSide } from "./slices/documentSlice";

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
  proposalProgress: vi.fn(),
  proposalControl: vi.fn(),
}));

import * as ipc from "../ipc/client";
import { automaticMovingSide, initialMovingSide, useAppStore } from "./appStore";

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
const TOP_AXIS = { kind: "line" as const, a: [0, 1] as Vec2, b: [1, 1] as Vec2 };
const SPATIAL_SUPPORT: SpatialSupportPlane = {
  point: [0, 0, 0],
  normal: [0, 0, 1],
};
const SPATIAL_XAXIS: SpatialAlignTarget = {
  kind: "line",
  aWorld: [0, 0, 0],
  bWorld: [1, 0, 0],
  supportPlanes: [SPATIAL_SUPPORT],
  foldedLine: null,
};
const SPATIAL_YAXIS: SpatialAlignTarget = {
  kind: "line",
  aWorld: [0, 0, 0],
  bWorld: [0, 1, 0],
  supportPlanes: [SPATIAL_SUPPORT],
  foldedLine: null,
};
const SPATIAL_LINE_SOLUTIONS: [SpatialFoldTarget, SpatialFoldTarget] = [
  {
    lineWorld: [
      [-1, -1, 0],
      [1, 1, 0],
    ],
    keepWorldForMovingSide: {
      left: [-0.25, 0.25, 0],
      right: [0.25, -0.25, 0],
    },
    foldedPlane: null,
    sideForFirstPick: { automatic: null, initial: "right" },
  },
  {
    lineWorld: [
      [-1, 1, 0],
      [1, -1, 0],
    ],
    keepWorldForMovingSide: {
      left: [0.25, 0.25, 0],
      right: [-0.25, -0.25, 0],
    },
    foldedPlane: null,
    sideForFirstPick: { automatic: null, initial: "right" },
  },
];
const SPATIAL_MATERIAL_LINES: [FoldLine, FoldLine] = [
  [
    [0, 0.2],
    [1, 0.2],
  ],
  [
    [0.8, 0],
    [0.8, 1],
  ],
];
const SPATIAL_MATERIAL_SOLUTIONS: [
  SpatialMaterialForMovingSide,
  SpatialMaterialForMovingSide,
] = [
  {
    left: {
      materialLine: SPATIAL_MATERIAL_LINES[0],
      materialKeepSidePoint: [0.5, 0.6],
    },
    right: {
      materialLine: SPATIAL_MATERIAL_LINES[0],
      materialKeepSidePoint: [0.5, 0],
    },
  },
  {
    left: {
      materialLine: SPATIAL_MATERIAL_LINES[1],
      materialKeepSidePoint: [0.4, 0.5],
    },
    right: {
      materialLine: SPATIAL_MATERIAL_LINES[1],
      materialKeepSidePoint: [0.95, 0.5],
    },
  },
];

/** 8つの合わせ方を、折り線が一意に求まる代表入力で漏れなく走査する。 */
const AUTOMATIC_SIDE_CASES: { mode: AlignMode; picks: AlignTarget[] }[] = [
  {
    mode: "throughTwoPoints",
    picks: [
      { kind: "point", p: [0, 0] },
      { kind: "point", p: [1, 0] },
    ],
  },
  {
    mode: "pointPoint",
    picks: [
      { kind: "point", p: [0, 0] },
      { kind: "point", p: [1, 0] },
    ],
  },
  { mode: "lineLine", picks: [XAXIS, TOP_AXIS] },
  {
    mode: "pointPerpendicularLine",
    picks: [{ kind: "point", p: [0.25, 0.5] }, XAXIS],
  },
  {
    mode: "pointLineThrough",
    picks: [
      { kind: "point", p: [0, 2] },
      XAXIS,
      { kind: "point", p: [0, 1] },
    ],
  },
  {
    mode: "pointToLinePointToLine",
    picks: [
      { kind: "point", p: [0, 1] },
      XAXIS,
      { kind: "point", p: [0, 0] },
      TOP_AXIS,
    ],
  },
  {
    mode: "pointLinePerpendicular",
    picks: [{ kind: "point", p: [0, 1] }, XAXIS, YAXIS],
  },
  { mode: "existingLine", picks: [XAXIS] },
];

beforeEach(() => {
  vi.clearAllMocks();
  solveAlignCounter.count = 0;
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
    expect(draft.spatialPicks).toBeUndefined();
    expect(draft.spatialSolutions).toBeUndefined();
    expect(
      Object.prototype.hasOwnProperty.call(
        useAppStore.getState().foldDraft,
        "spatialTarget",
      ),
    ).toBe(false);
    // 折り線は y = 1 - x(両端点で x + y = 1)
    for (const p of useAppStore.getState().foldDraft!.line) {
      expect(p[0] + p[1]).toBeCloseTo(1, 9);
    }
  });

  it("第4引数を省略したlegacy cycleだけはstoreで1回解く", () => {
    const s = useAppStore.getState();
    s.beginAlign("existingLine");
    s.pickAlignTarget(XAXIS);
    expect(solveAlignCounter.count).toBe(1);
    expect(useAppStore.getState().foldDraft).not.toBeNull();
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
      contact_detected: false,
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

describe("折り返す紙の自動決定", () => {
  it("8方式を走査し、1つ目から決まる5方式と決められない3方式を区別する", () => {
    const coveredModes = AUTOMATIC_SIDE_CASES.map(({ mode }) => mode);
    const allModes = Object.keys(ALIGN_STEPS) as AlignMode[];
    // 合わせ方が増えたとき、検査表へ足さないまま合格させない。
    expect(new Set(coveredModes).size).toBe(AUTOMATIC_SIDE_CASES.length);
    expect([...coveredModes].sort()).toEqual([...allModes].sort());

    const determined: AlignMode[] = [];
    const undetermined: AlignMode[] = [];
    for (const { mode, picks } of AUTOMATIC_SIDE_CASES) {
      useAppStore.getState().beginAlign(mode);
      for (const pick of picks) useAppStore.getState().pickAlignTarget(pick);

      const draft = useAppStore.getState().foldDraft;
      expect(draft, `${mode}で折り線が求まる`).not.toBeNull();
      if (!draft) continue;
      const automatic = automaticMovingSide(draft.line, picks[0]);
      // 自動で決められないときは従来の中点判定を保ちつつ、黄色と説明を出す。
      expect(draft.movingSide, mode).toBe(initialMovingSide(draft.line, picks[0]));
      (automatic === null ? undetermined : determined).push(mode);
    }

    expect(determined).toHaveLength(5);
    expect(undetermined).toEqual([
      "throughTwoPoints",
      "pointPerpendicularLine",
      "existingLine",
    ]);
  });

  it("線が折り線をまたぐと決め付けず、片端だけが折り線上ならもう一端の側を使う", () => {
    const foldLine: [Vec2, Vec2] = [
      [-1, 0],
      [1, 0],
    ];
    expect(
      automaticMovingSide(foldLine, {
        kind: "line",
        a: [0, 1],
        b: [0, -1],
      }),
    ).toBeNull();
    expect(
      automaticMovingSide(foldLine, {
        kind: "line",
        a: [0, 0],
        b: [0, 1],
      }),
    ).toBe("left");
    expect(
      automaticMovingSide(foldLine, {
        kind: "line",
        a: [0, 0],
        b: [0, -1],
      }),
    ).toBe("right");

    // またぐ線は判定不能と説明する一方、既存の折り結果は旧来の中点側に保つ。
    const verticalFold: [Vec2, Vec2] = [
      [0, -1],
      [0, 1],
    ];
    const crossing = {
      kind: "line" as const,
      a: [-2, 0] as Vec2,
      b: [1, 0] as Vec2,
    };
    expect(automaticMovingSide(verticalFold, crossing)).toBeNull();
    expect(initialMovingSide(verticalFold, crossing)).toBe("left");
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

  it("spatial solve 1回だけで3D解と材料値を同じindexで往復する", () => {
    // 3D担当が共通chartで行う唯一のsolve。以後storeへ第4引数として渡す。
    expect(solveAlign("lineLine", [XAXIS, YAXIS], [1, 1]).lines).toHaveLength(2);
    expect(solveAlignCounter.count).toBe(1);
    const s = useAppStore.getState();
    s.beginAlign("lineLine");
    s.pickAlignTarget(XAXIS, [1, 1], null, {
      target: SPATIAL_XAXIS,
      solutions: [],
      materialSolutions: [],
      reason: null,
    });
    s.pickAlignTarget(YAXIS, [1, 1], null, {
      target: SPATIAL_YAXIS,
      solutions: SPATIAL_LINE_SOLUTIONS,
      materialSolutions: SPATIAL_MATERIAL_SOLUTIONS,
      reason: "共通3D平面の2解",
    });
    // storeは材料値を正本にし、傾斜面のlegacy XYをもう一度解かない。
    expect(solveAlignCounter.count).toBe(1);

    let align = useAppStore.getState().alignDraft!;
    let fold = useAppStore.getState().foldDraft!;
    expect(align.spatialPicks).toEqual([SPATIAL_XAXIS, SPATIAL_YAXIS]);
    expect(align.spatialSolutions).toEqual(SPATIAL_LINE_SOLUTIONS);
    expect(align.spatialMaterialSolutions).toEqual(SPATIAL_MATERIAL_SOLUTIONS);
    expect(align.solutions).toEqual(SPATIAL_MATERIAL_LINES);
    expect(align.spatialSolutionIndices).toEqual([0, 1]);
    expect(align.spatialReason).toBe("共通3D平面の2解");
    expect(fold.line).toEqual(SPATIAL_MATERIAL_LINES[0]);
    expect(fold.spatialTarget).toEqual(SPATIAL_LINE_SOLUTIONS[0]);
    expect(fold.spatialMaterialForMovingSide).toEqual(
      SPATIAL_MATERIAL_SOLUTIONS[0],
    );

    useAppStore.getState().updateFoldDraft({ direction: "Down", target: "top" });
    useAppStore.getState().nextAlignSolution();
    align = useAppStore.getState().alignDraft!;
    fold = useAppStore.getState().foldDraft!;
    expect(align.solutionIndex).toBe(1);
    expect(fold.line).toEqual(SPATIAL_MATERIAL_LINES[1]);
    expect(fold.spatialTarget).toEqual(SPATIAL_LINE_SOLUTIONS[1]);
    expect(fold.spatialMaterialForMovingSide).toEqual(
      SPATIAL_MATERIAL_SOLUTIONS[1],
    );
    expect(fold.direction).toBe("Down");
    expect(fold.target).toBe("top");

    useAppStore.getState().nextAlignSolution();
    fold = useAppStore.getState().foldDraft!;
    expect(useAppStore.getState().alignDraft?.solutionIndex).toBe(0);
    expect(fold.spatialTarget).toEqual(SPATIAL_LINE_SOLUTIONS[0]);
    expect(fold.spatialMaterialForMovingSide).toEqual(
      SPATIAL_MATERIAL_SOLUTIONS[0],
    );
  });

  it("raw spatialのnull slotを詰めず、dense解から同じraw indexへ対応する", () => {
    const s = useAppStore.getState();
    s.beginAlign("lineLine");
    s.pickAlignTarget(XAXIS, [1, 1], null, {
      target: SPATIAL_XAXIS,
      solutions: [],
      materialSolutions: [],
      reason: null,
    });
    const rawSolutions = [
      SPATIAL_LINE_SOLUTIONS[0],
      null,
      SPATIAL_LINE_SOLUTIONS[1],
    ];
    const rawMaterials = [
      SPATIAL_MATERIAL_SOLUTIONS[0],
      null,
      SPATIAL_MATERIAL_SOLUTIONS[1],
    ];
    s.pickAlignTarget(YAXIS, [1, 1], null, {
      target: SPATIAL_YAXIS,
      solutions: rawSolutions,
      materialSolutions: rawMaterials,
      reason: null,
    });

    const align = useAppStore.getState().alignDraft!;
    expect(align.spatialSolutions).toEqual(rawSolutions);
    expect(align.spatialMaterialSolutions).toEqual(rawMaterials);
    expect(align.solutions).toEqual(SPATIAL_MATERIAL_LINES);
    expect(align.spatialSolutionIndices).toEqual([0, 2]);
    expect(useAppStore.getState().foldDraft?.spatialTarget).toEqual(
      SPATIAL_LINE_SOLUTIONS[0],
    );

    useAppStore.getState().nextAlignSolution();
    expect(useAppStore.getState().alignDraft?.solutionIndex).toBe(1);
    expect(useAppStore.getState().foldDraft).toMatchObject({
      line: SPATIAL_MATERIAL_LINES[1],
      spatialTarget: SPATIAL_LINE_SOLUTIONS[1],
      spatialMaterialForMovingSide: SPATIAL_MATERIAL_SOLUTIONS[1],
    });
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

  it("3D入力がnullならlegacy XY解へfallbackせず材料照会も送らない", async () => {
    const s = useAppStore.getState();
    s.beginAlign("lineLine");
    s.pickAlignTarget(XAXIS, [1, 1], null, {
      target: SPATIAL_XAXIS,
      solutions: [],
      materialSolutions: [],
      reason: null,
    });
    s.pickAlignTarget(YAXIS, [1, 1], null, {
      target: null,
      // 呼出側が誤って解を添えても、null pickがあるcycleでは全て拒否する。
      solutions: SPATIAL_LINE_SOLUTIONS,
      materialSolutions: SPATIAL_MATERIAL_SOLUTIONS,
      reason: "支持面を一意に決められません",
    });

    const align = useAppStore.getState().alignDraft!;
    expect(align.solutions).toEqual([]);
    expect(align.spatialPicks).toEqual([SPATIAL_XAXIS, null]);
    expect(align.spatialSolutions).toEqual([null, null]);
    expect(align.spatialMaterialSolutions).toEqual([null, null]);
    expect(align.spatialSolutionIndices).toEqual([]);
    expect(align.spatialReason).toBe("支持面を一意に決められません");
    expect(useAppStore.getState().foldDraft).toBeNull();

    await useAppStore.getState().requestFoldTargetInfo();
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
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

  it("3D選択を戻す・完成後に選び直すとspatial一時値も同時に消す", () => {
    const s = useAppStore.getState();
    s.beginAlign("lineLine");
    s.pickAlignTarget(XAXIS, [1, 1], null, {
      target: SPATIAL_XAXIS,
      solutions: [],
      materialSolutions: [],
      reason: null,
    });
    s.pickAlignTarget(YAXIS, [1, 1], null, {
      target: SPATIAL_YAXIS,
      solutions: SPATIAL_LINE_SOLUTIONS,
      materialSolutions: SPATIAL_MATERIAL_SOLUTIONS,
      reason: "古い3D解",
    });

    useAppStore.getState().undoAlignPick();
    let draft = useAppStore.getState().alignDraft!;
    expect(draft.picks).toEqual([XAXIS]);
    expect(draft.spatialPicks).toEqual([SPATIAL_XAXIS]);
    expect(draft.spatialSolutions).toEqual([]);
    expect(draft.spatialMaterialSolutions).toEqual([]);
    expect(draft.spatialSolutionIndices).toEqual([]);
    expect(draft.spatialReason).toBeNull();
    expect(useAppStore.getState().foldDraft).toBeNull();

    useAppStore.getState().pickAlignTarget(YAXIS, [1, 1], null, {
      target: SPATIAL_YAXIS,
      solutions: SPATIAL_LINE_SOLUTIONS,
      materialSolutions: SPATIAL_MATERIAL_SOLUTIONS,
      reason: "新しい3D解",
    });
    expect(useAppStore.getState().alignDraft?.spatialReason).toBe("新しい3D解");

    useAppStore.getState().pickAlignTarget(TOP_AXIS, null, null, {
      target: {
        ...SPATIAL_XAXIS,
        aWorld: [0, 1, 0],
        bWorld: [1, 1, 0],
      },
      solutions: [],
      materialSolutions: [],
      reason: "新しい1件目",
    });
    draft = useAppStore.getState().alignDraft!;
    expect(draft.picks).toEqual([TOP_AXIS]);
    expect(draft.spatialPicks).toEqual([
      {
        ...SPATIAL_XAXIS,
        aWorld: [0, 1, 0],
        bWorld: [1, 1, 0],
      },
    ]);
    expect(draft.spatialSolutions).toEqual([]);
    expect(draft.spatialMaterialSolutions).toEqual([]);
    expect(draft.spatialSolutionIndices).toEqual([]);
    expect(draft.spatialReason).toBe("新しい1件目");
    expect(useAppStore.getState().foldDraft).toBeNull();
  });

  it("spatial pickを0件まで戻した後は純2D cycleとして再開する", () => {
    const s = useAppStore.getState();
    const point = { kind: "point" as const, p: [0, 0] as Vec2 };
    s.beginAlign("pointPoint");
    s.pickAlignTarget(point, null, null, {
      target: {
        kind: "point",
          world: [0, 0, 0],
          supportPlanes: [SPATIAL_SUPPORT],
          foldedPoint: null,
      },
      solutions: [],
      materialSolutions: [],
      reason: null,
    });
    expect(solveAlignCounter.count).toBe(0);

    useAppStore.getState().undoAlignPick();
    let draft = useAppStore.getState().alignDraft!;
    expect(draft.picks).toEqual([]);
    expect(draft.spatialPicks).toBeUndefined();
    expect(draft.spatialSolutions).toBeUndefined();
    expect(draft.spatialMaterialSolutions).toBeUndefined();
    expect(draft.spatialSolutionIndices).toBeUndefined();

    useAppStore.getState().pickAlignTarget(point);
    draft = useAppStore.getState().alignDraft!;
    expect(solveAlignCounter.count).toBe(1);
    expect(draft.spatialPicks).toBeUndefined();
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
