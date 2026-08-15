// 新規作成の紙の指定(PAP-001)・紙の色と方眼(PAP-003 / CPE-003)・
// 2D/3Dの分割比(UI-004)・手順の並べ替え(SEQ-005)のテスト。

import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  Document,
  DocumentView,
  EdgeKind,
  EditOp,
  FoldStep,
  Vec2,
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
  recoveryCheck: vi.fn(),
  recoveryRestore: vi.fn(),
  proposalGenerate: vi.fn(),
}));

import * as ipc from "../ipc/client";
import { DEFAULT_NEW_PAPER, useAppStore } from "./appStore";
import {
  DEFAULT_CONTEXT_PANEL_RATIO,
  DEFAULT_DISPLAY,
  MAX_CONTEXT_PANEL_RATIO,
  MIN_CONTEXT_PANEL_RATIO,
} from "../lib/displayPrefs";
import { DEFAULT_CONSTRUCT, type ConstructOptions } from "../lib/construct";
import { DEFAULT_CURVE } from "../lib/curve";
import { paperMirrorLine } from "../lib/mirror";
import {
  initialEphemeralState,
  onMouseDown,
  type InteractionCtx,
} from "../components/CpEditor/interaction";

function step(id: number): FoldStep {
  return { id, kind: "Simple", drivers: [], layer_order: null, note: `手順${id}` };
}

function makeDoc(sequence: FoldStep[]): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: { vertices: [], edges: [], next_vertex_id: 0, next_edge_id: 0 },
    sequence,
    display: DEFAULT_DISPLAY,
  };
}

function makeView(doc: Document): DocumentView {
  return {
    doc,
    faces: [],
    warnings: [],
    violations: [],
    frame: null,
    skipped: [],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({
    doc: null,
    newDialogOpen: false,
    newPaperDraft: DEFAULT_NEW_PAPER,
    display: DEFAULT_DISPLAY,
    splitRatio: 0.5,
    contextPanelRatio: DEFAULT_CONTEXT_PANEL_RATIO,
    errorMessage: null,
    currentStep: null,
    mirrorDraw: false,
    mirrorAxis: { kind: "paperVertical" },
    mirrorAxisNotice: null,
    selection: { edgeIds: [], vertexIds: [] },
    faces: [],
    wheelBehavior: "scroll",
  });
});

describe("新規作成の紙の指定", () => {
  it("正方形なら縦は横に合わせ、長方形なら別々に指定できる", async () => {
    vi.mocked(ipc.documentNew).mockResolvedValue(makeView(makeDoc([])));
    const s = useAppStore.getState();
    s.openNewDialog();
    expect(useAppStore.getState().newDialogOpen).toBe(true);

    s.setNewPaperDraft({ widthMm: 200, heightMm: 90 });
    await useAppStore.getState().confirmNewDocument();
    expect(ipc.documentNew).toHaveBeenLastCalledWith({
      width_mm: 200,
      height_mm: 200,
    });
    // 作りはじめたらダイアログは閉じる
    expect(useAppStore.getState().newDialogOpen).toBe(false);

    s.setNewPaperDraft({ square: false });
    await useAppStore.getState().confirmNewDocument();
    expect(ipc.documentNew).toHaveBeenLastCalledWith({
      width_mm: 200,
      height_mm: 90,
    });
  });

  it("0以下の大きさは作らずに理由を出す", async () => {
    useAppStore.getState().setNewPaperDraft({ widthMm: 0 });
    await useAppStore.getState().confirmNewDocument();
    expect(ipc.documentNew).not.toHaveBeenCalled();
    expect(useAppStore.getState().errorMessage).toContain("0より大きい");
  });
});

describe("紙の色と方眼・重なり防止/食い込み検出・区画の分割比", () => {
  it("色・方眼・2種類の防止設定を作品ごとの設定として保存する", async () => {
    // Rust側は受け取った見た目をそのまま作品へ入れて返す
    vi.mocked(ipc.editApply).mockImplementation(async (op) =>
      makeView({
        ...makeDoc([]),
        display: op.type === "SetDisplay" ? op.display : DEFAULT_DISPLAY,
      }),
    );
    useAppStore.setState({ doc: makeDoc([]) });

    await useAppStore.getState().setDisplay({ front_color: [0, 128, 255] });
    expect(vi.mocked(ipc.editApply).mock.calls[0][0]).toEqual({
      type: "SetDisplay",
      display: { ...DEFAULT_DISPLAY, front_color: [0, 128, 255] },
    });
    // 作品にも画面側の写しにも入る(保存すれば.ori3へ、相手にも同じ色で伝わる)
    expect(useAppStore.getState().doc?.display.front_color).toEqual([0, 128, 255]);
    expect(useAppStore.getState().display.front_color).toEqual([0, 128, 255]);

    // 範囲外は上限(1024)に丸めてから送る
    await useAppStore.getState().setDisplay({ grid_divisions: 2048 });
    const last = vi.mocked(ipc.editApply).mock.calls[1][0];
    if (last.type !== "SetDisplay") throw new Error("SetDisplayでない");
    expect(last.display.grid_divisions).toBe(1024);
    expect(useAppStore.getState().doc?.display.grid_divisions).toBe(1024);

    await useAppStore
      .getState()
      .setDisplay({ overlap_prevention_enabled: false });
    const overlap = vi.mocked(ipc.editApply).mock.calls[2][0];
    if (overlap.type !== "SetDisplay") throw new Error("SetDisplayでない");
    expect(overlap.display.overlap_prevention_enabled).toBe(false);
    expect(useAppStore.getState().doc?.display.overlap_prevention_enabled).toBe(false);

    await useAppStore
      .getState()
      .setDisplay({ penetration_prevention_enabled: false });
    const penetration = vi.mocked(ipc.editApply).mock.calls[3][0];
    if (penetration.type !== "SetDisplay") throw new Error("SetDisplayでない");
    expect(penetration.display.penetration_prevention_enabled).toBe(false);
    expect(useAppStore.getState().doc?.display.penetration_prevention_enabled).toBe(false);
  });

  it("作品をまだ開いていないときは画面の見た目だけ変える(送らない)", async () => {
    useAppStore.setState({ doc: null });
    await useAppStore.getState().setDisplay({ grid_divisions: 16 });
    expect(ipc.editApply).not.toHaveBeenCalled();
    expect(useAppStore.getState().display.grid_divisions).toBe(16);
  });

  it("分割比は狭くしすぎないように収める", () => {
    useAppStore.getState().setSplitRatio(0.01);
    expect(useAppStore.getState().splitRatio).toBeCloseTo(0.2);
    useAppStore.getState().setSplitRatio(0.35);
    expect(useAppStore.getState().splitRatio).toBeCloseTo(0.35);
  });

  it("下部パネルの比率は25%〜55%に収め、中間値はそのまま使う", () => {
    useAppStore.getState().setContextPanelRatio(0.1);
    expect(useAppStore.getState().contextPanelRatio).toBeCloseTo(
      MIN_CONTEXT_PANEL_RATIO,
    );
    useAppStore.getState().setContextPanelRatio(0.4);
    expect(useAppStore.getState().contextPanelRatio).toBeCloseTo(0.4);
    useAppStore.getState().setContextPanelRatio(0.9);
    expect(useAppStore.getState().contextPanelRatio).toBeCloseTo(
      MAX_CONTEXT_PANEL_RATIO,
    );
    expect(ipc.editApply).not.toHaveBeenCalled();
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
  });

  it("区画の広さを初期化しても作品データは変更しない", () => {
    useAppStore.setState({
      doc: makeDoc([]),
      splitRatio: 0.72,
      contextPanelRatio: 0.51,
    });

    useAppStore.getState().resetPaneSizes();

    expect(useAppStore.getState().splitRatio).toBeCloseTo(0.5);
    expect(useAppStore.getState().contextPanelRatio).toBeCloseTo(
      DEFAULT_CONTEXT_PANEL_RATIO,
    );
    expect(ipc.editApply).not.toHaveBeenCalled();
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
    expect(ipc.documentSave).not.toHaveBeenCalled();
  });
});

type MirrorAxisTestCase = {
  label: string;
  kind: "paperVertical" | "paperHorizontal" | "selectedLine";
  sourceId: number;
  mirrorId: number;
  mirrored: [Vec2, Vec2];
  onAxis: [Vec2, Vec2];
};

const DRAW_SEGMENT: [Vec2, Vec2] = [
  [0.125, 0.25],
  [0.25, 0.375],
];

const MIRROR_AXIS_CASES: MirrorAxisTestCase[] = [
  {
    label: "紙の縦の中心線",
    kind: "paperVertical",
    sourceId: 10,
    mirrorId: 11,
    mirrored: [
      [0.875, 0.25],
      [0.75, 0.375],
    ],
    onAxis: [
      [0.5, 0.125],
      [0.5, 0.875],
    ],
  },
  {
    label: "紙の横の中心線",
    kind: "paperHorizontal",
    sourceId: 20,
    mirrorId: 21,
    mirrored: [
      [0.125, 0.75],
      [0.25, 0.625],
    ],
    onAxis: [
      [0.125, 0.5],
      [0.875, 0.5],
    ],
  },
  {
    label: "選んだ斜めの線",
    kind: "selectedLine",
    sourceId: 30,
    mirrorId: 31,
    mirrored: [
      [0.25, 0.125],
      [0.375, 0.25],
    ],
    onAxis: [
      [0.125, 0.125],
      [0.875, 0.875],
    ],
  },
];

function addMirrorTestEdge(
  doc: Document,
  id: number,
  a: Vec2,
  b: Vec2,
  kind: EdgeKind = "Mountain",
): void {
  const v0 = doc.cp.next_vertex_id++;
  const v1 = doc.cp.next_vertex_id++;
  doc.cp.vertices.push({ id: v0, pos: a }, { id: v1, pos: b });
  doc.cp.edges.push({ id, v0, v1, kind });
  doc.cp.next_edge_id = Math.max(doc.cp.next_edge_id, id + 1);
}

/** 縦・横・斜めの各基準について、元の線と対になる線を持つ正方形の展開図。 */
function mirrorTestDoc(): Document {
  const doc = makeDoc([]);
  addMirrorTestEdge(doc, 10, DRAW_SEGMENT[0], DRAW_SEGMENT[1]);
  addMirrorTestEdge(doc, 11, [0.875, 0.25], [0.75, 0.375]);
  addMirrorTestEdge(doc, 20, DRAW_SEGMENT[0], DRAW_SEGMENT[1]);
  addMirrorTestEdge(doc, 21, [0.125, 0.75], [0.25, 0.625]);
  addMirrorTestEdge(doc, 30, DRAW_SEGMENT[0], DRAW_SEGMENT[1]);
  addMirrorTestEdge(doc, 31, [0.25, 0.125], [0.375, 0.25]);
  // 選択基準は左下から右上へ通る補助線(y=x)。
  addMirrorTestEdge(doc, 100, [0, 0], [1, 1], "Aux");
  // 縦中心で反対側に相手が無い線。
  addMirrorTestEdge(doc, 90, [0.625, 0.5], [0.75, 0.625], "Aux");
  return doc;
}

/** 対称編集の準備。IPCは編集前と同じ展開図を成功結果として返す。 */
function readyMirrorTest(mirrorDraw: boolean): Document {
  const doc = mirrorTestDoc();
  useAppStore.setState({
    doc,
    mirrorDraw,
    mirrorAxis: { kind: "paperVertical" },
    mirrorAxisNotice: null,
    selection: { edgeIds: [], vertexIds: [] },
    faces: [],
  });
  vi.mocked(ipc.editApply).mockResolvedValue(makeView(doc));
  vi.mocked(ipc.editApplyBatch).mockResolvedValue(makeView(doc));
  return doc;
}

function chooseMirrorAxis(axis: MirrorAxisTestCase): void {
  const store = useAppStore.getState();
  if (axis.kind === "selectedLine") {
    store.setSelection({ edgeIds: [100], vertexIds: [] });
    useAppStore.getState().setSelectedLineAsMirrorAxis();
  } else {
    store.setMirrorAxisPreset(axis.kind);
  }
}

/** 送った編集を、1件送りとまとめ送りの区別なく送った順に並べる。
 * 左右対称の2本は「元に戻す1回ぶん」としてまとめて送られる(不具合D05)。 */
const editCalls = (): EditOp[] => {
  const single = vi.mocked(ipc.editApply).mock;
  const batch = vi.mocked(ipc.editApplyBatch).mock;
  const sent: { at: number; ops: EditOp[] }[] = [
    ...single.calls.map(([op], i) => ({
      at: single.invocationCallOrder[i],
      ops: [op],
    })),
    ...batch.calls.map(([ops], i) => ({
      at: batch.invocationCallOrder[i],
      ops: [...ops],
    })),
  ];
  return sent.sort((a, b) => a.at - b.at).flatMap((one) => one.ops);
};

const lastEditCall = () => {
  const calls = editCalls();
  return calls[calls.length - 1];
};

describe("基準線を選んで対称に線を引く", () => {
  for (const axis of MIRROR_AXIS_CASES) {
    it(`${axis.label}を基準に、反対側にも同じ線を引く`, async () => {
      readyMirrorTest(true);
      chooseMirrorAxis(axis);

      await useAppStore
        .getState()
        .drawSegment(DRAW_SEGMENT[0], DRAW_SEGMENT[1], "Mountain");

      const calls = editCalls();
      expect(calls).toHaveLength(2);
      expect(calls[0]).toEqual({
        type: "AddSegment",
        a: DRAW_SEGMENT[0],
        b: DRAW_SEGMENT[1],
        kind: "Mountain",
      });
      const reflected = calls[1];
      if (reflected.type !== "AddSegment") throw new Error("AddSegmentでない");
      expect(reflected.kind).toBe("Mountain");
      if (axis.kind !== "selectedLine") {
        // 紙の縦・横中心は二進数で正確な座標なので、従来どおり完全一致を守る。
        expect(reflected).toEqual({
          type: "AddSegment",
          a: axis.mirrored[0],
          b: axis.mirrored[1],
          kind: "Mountain",
        });
      } else {
        // 斜線への垂線計算はf64の丸めだけを明示的な桁数で許容する。
        expect(reflected.a[0]).toBeCloseTo(axis.mirrored[0][0], 12);
        expect(reflected.a[1]).toBeCloseTo(axis.mirrored[0][1], 12);
        expect(reflected.b[0]).toBeCloseTo(axis.mirrored[1][0], 12);
        expect(reflected.b[1]).toBeCloseTo(axis.mirrored[1][1], 12);
      }
    });

    it(`${axis.label}の上に引いた線は1本だけになる`, async () => {
      readyMirrorTest(true);
      chooseMirrorAxis(axis);

      await useAppStore
        .getState()
        .drawSegment(axis.onAxis[0], axis.onAxis[1], "Valley");

      expect(editCalls()).toEqual([
        {
          type: "AddSegment",
          a: axis.onAxis[0],
          b: axis.onAxis[1],
          kind: "Valley",
        },
      ]);
    });
  }

  it("もともと縦中心をまたいで対称な線も1本だけになる", async () => {
    readyMirrorTest(true);
    useAppStore.getState().setMirrorAxisPreset("paperVertical");

    await useAppStore.getState().drawSegment([0.25, 0.5], [0.75, 0.5], "Valley");

    expect(editCalls()).toEqual([
      { type: "AddSegment", a: [0.25, 0.5], b: [0.75, 0.5], kind: "Valley" },
    ]);
  });

  it("対称描画を切ってあるときは引いた線だけを引く", async () => {
    readyMirrorTest(false);
    useAppStore.getState().setMirrorAxisPreset("paperHorizontal");

    await useAppStore
      .getState()
      .drawSegment(DRAW_SEGMENT[0], DRAW_SEGMENT[1], "Aux");

    expect(editCalls()).toEqual([
      { type: "AddSegment", a: DRAW_SEGMENT[0], b: DRAW_SEGMENT[1], kind: "Aux" },
    ]);
  });
});

describe("基準線を選んで対称に消す・線種を変える", () => {
  for (const axis of MIRROR_AXIS_CASES) {
    it(`${axis.label}を基準に、削除を反対側の線にも効かせる`, async () => {
      readyMirrorTest(true);
      chooseMirrorAxis(axis);

      await useAppStore
        .getState()
        .applyEdit({ type: "RemoveEdges", ids: [axis.sourceId] });

      const op = lastEditCall();
      if (op.type !== "RemoveEdges") throw new Error("RemoveEdgesでない");
      expect([...op.ids].sort((a, b) => a - b)).toEqual([
        axis.sourceId,
        axis.mirrorId,
      ]);
    });

    it(`${axis.label}を基準に、線種変更を反対側の線にも効かせる`, async () => {
      readyMirrorTest(true);
      chooseMirrorAxis(axis);

      await useAppStore
        .getState()
        .applyEdit({ type: "SetEdgeKind", ids: [axis.sourceId], kind: "Valley" });

      const op = lastEditCall();
      if (op.type !== "SetEdgeKind") throw new Error("SetEdgeKindでない");
      expect([...op.ids].sort((a, b) => a - b)).toEqual([
        axis.sourceId,
        axis.mirrorId,
      ]);
      expect(op.kind).toBe("Valley");
    });
  }

  it("反対側に相手がいない線は、その線だけを消す(警告は出さない)", async () => {
    readyMirrorTest(true);
    useAppStore.getState().setMirrorAxisPreset("paperVertical");

    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [90] });

    const op = lastEditCall();
    if (op.type !== "RemoveEdges") throw new Error("RemoveEdgesでない");
    expect(op.ids).toEqual([90]);
    expect(useAppStore.getState().errorMessage).toBeNull();
    expect(useAppStore.getState().mirrorAxisNotice).toBeNull();
  });

  it("対称描画を切ってあるときは選んだ線だけが変わる", async () => {
    readyMirrorTest(false);
    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [10] });
    const op = lastEditCall();
    if (op.type !== "RemoveEdges") throw new Error("RemoveEdgesでない");
    expect(op.ids).toEqual([10]);
  });

  it("線を引く以外の編集(点を動かす等)はそのまま送る", async () => {
    readyMirrorTest(true);
    await useAppStore.getState().applyEdit({ type: "MoveVertex", id: 0, to: [0.3, 0.4] });
    expect(lastEditCall()).toEqual({ type: "MoveVertex", id: 0, to: [0.3, 0.4] });
  });

  it("選んだ基準線が交点で分割されても、同じ斜め基準を使い続ける", async () => {
    readyMirrorTest(true);
    chooseMirrorAxis(MIRROR_AXIS_CASES[2]);
    const splitAxis = mirrorTestDoc();
    splitAxis.cp.edges = splitAxis.cp.edges.filter((edge) => edge.id !== 100);
    addMirrorTestEdge(splitAxis, 101, [0, 0], [0.5, 0.5], "Aux");
    addMirrorTestEdge(splitAxis, 102, [0.5, 0.5], [1, 1], "Aux");
    vi.mocked(ipc.editApply).mockResolvedValue(makeView(splitAxis));
    vi.mocked(ipc.editApplyBatch).mockResolvedValue(makeView(splitAxis));

    // 基準線を横切る線を追加すると、実際の編集結果では基準辺が交点で2片に分かれる。
    await useAppStore.getState().drawSegment([0.125, 0.5], [0.875, 0.5], "Aux");

    expect(useAppStore.getState().mirrorAxis).toMatchObject({ kind: "selectedLine" });
    expect(useAppStore.getState().mirrorAxisNotice).toBeNull();

    // 次の操作でも、縦中心ではなく選んだ斜線(y=x)を使うことをIPC引数で確かめる。
    const callsBeforeSecondDraw = editCalls().length;
    await useAppStore
      .getState()
      .drawSegment(DRAW_SEGMENT[0], DRAW_SEGMENT[1], "Mountain");
    const secondDraw = editCalls().slice(callsBeforeSecondDraw);
    expect(secondDraw).toHaveLength(2);
    expect(secondDraw[0]).toEqual({
      type: "AddSegment",
      a: DRAW_SEGMENT[0],
      b: DRAW_SEGMENT[1],
      kind: "Mountain",
    });
    const reflected = secondDraw[1];
    if (reflected.type !== "AddSegment") throw new Error("AddSegmentでない");
    expect(reflected.kind).toBe("Mountain");
    expect(reflected.a[0]).toBeCloseTo(0.25, 12);
    expect(reflected.a[1]).toBeCloseTo(0.125, 12);
    expect(reflected.b[0]).toBeCloseTo(0.375, 12);
    expect(reflected.b[1]).toBeCloseTo(0.25, 12);
  });

  it("選んだ基準線を削除すると縦中心へ戻し、理由を完全一致で通知する", async () => {
    const doc = readyMirrorTest(true);
    const withoutAxis: Document = {
      ...doc,
      cp: {
        ...doc.cp,
        edges: doc.cp.edges.filter((edge) => edge.id !== 100),
      },
    };
    chooseMirrorAxis(MIRROR_AXIS_CASES[2]);
    vi.mocked(ipc.editApply).mockResolvedValue(makeView(withoutAxis));
    vi.mocked(ipc.editApplyBatch).mockResolvedValue(makeView(withoutAxis));

    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [100] });

    const op = lastEditCall();
    if (op.type !== "RemoveEdges") throw new Error("RemoveEdgesでない");
    expect(op.ids).toEqual([100]);
    const state = useAppStore.getState();
    expect(state.mirrorAxis).toEqual({ kind: "paperVertical" });
    expect(state.selection.edgeIds).toEqual([]);
    expect(state.mirrorAxisNotice).toBe(
      "基準にしていた線が無くなったので、紙の縦の中心線に戻しました",
    );
  });
});

async function switchMirrorTestDocument(kind: "open" | "new"): Promise<void> {
  const next = makeView(mirrorTestDoc());
  if (kind === "open") {
    vi.mocked(ipc.documentOpen).mockResolvedValue(next);
    await useAppStore.getState().openDocument("別の作品.ori3");
    return;
  }
  vi.mocked(ipc.documentNew).mockResolvedValue(next);
  await useAppStore.getState().newDocument({ width_mm: 150, height_mm: 150 });
}

describe("作品を切り替えたときの対称基準", () => {
  for (const kind of ["open", "new"] as const) {
    const operation = kind === "open" ? "別の作品を開く" : "新しい作品を作る";

    it(`${operation}と、選んだ線の基準だけ縦中心へ戻る`, async () => {
      readyMirrorTest(true);
      chooseMirrorAxis(MIRROR_AXIS_CASES[2]);
      expect(useAppStore.getState().mirrorAxis).toEqual({
        kind: "selectedLine",
        edgeId: 100,
      });

      await switchMirrorTestDocument(kind);

      const state = useAppStore.getState();
      expect(state.mirrorAxis).toEqual({ kind: "paperVertical" });
      expect(state.selection).toEqual({ edgeIds: [], vertexIds: [] });
      expect(state.mirrorAxisNotice).toBeNull();
    });

    it(`${operation}ときも、紙の横の中心線は維持する`, async () => {
      readyMirrorTest(true);
      useAppStore.getState().setMirrorAxisPreset("paperHorizontal");

      await switchMirrorTestDocument(kind);

      expect(useAppStore.getState().mirrorAxis).toEqual({ kind: "paperHorizontal" });
      expect(useAppStore.getState().mirrorAxisNotice).toBeNull();
    });
  }
});

/** 輪郭だけの正方形(1辺=1.0)。作図補助の点・線のクリック先にする。 */
function constructTestDoc(): Document {
  const doc = makeDoc([]);
  doc.cp = {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [1, 0] },
      { id: 2, pos: [1, 1] },
      { id: 3, pos: [0, 1] },
    ],
    edges: [0, 1, 2, 3].map((i) => ({
      id: i,
      v0: i,
      v1: (i + 1) % 4,
      kind: "Border" as const,
    })),
    next_vertex_id: 4,
    next_edge_id: 4,
  };
  return doc;
}

/** 正規化座標 → 画面座標(拡大率500、y軸反転)。 */
const toConstructScreen = (p: Vec2): Vec2 => [p[0] * 500, 500 - p[1] * 500];

/**
 * 作図補助を、実際の画面と同じクリックの順で1回ぶん操作する。
 * 送られた線の本数と、要求の回数(=積まれる履歴の件数)を返す。
 */
async function runConstruct(
  options: Partial<ConstructOptions>,
  clicks: Vec2[],
  mirrorDraw: boolean,
): Promise<{ lines: EditOp[]; requests: number }> {
  const doc = constructTestDoc();
  useAppStore.setState({
    doc,
    mirrorDraw,
    mirrorAxis: { kind: "paperVertical" },
    mirrorAxisNotice: null,
    selection: { edgeIds: [], vertexIds: [] },
    faces: [],
  });
  vi.mocked(ipc.editApply).mockReset();
  vi.mocked(ipc.editApplyBatch).mockReset();
  vi.mocked(ipc.editApply).mockResolvedValue(makeView(doc));
  vi.mocked(ipc.editApplyBatch).mockResolvedValue(makeView(doc));

  const pending: Promise<void>[] = [];
  const ctx: InteractionCtx = {
    doc,
    finalDoc: doc,
    view: { scale: 500, offsetX: 0, offsetY: 500 },
    tool: "construct",
    selection: { edgeIds: [], vertexIds: [] },
    alignDraft: null,
    faces: [],
    frame3d: null,
    construct: { ...DEFAULT_CONSTRUCT, ...options },
    curve: { ...DEFAULT_CURVE },
    wheelBehavior: "scroll",
    violations: [],
    mirrorAxis: paperMirrorLine(doc.paper, "paperVertical"),
    state: initialEphemeralState(),
    setView: vi.fn(),
    applyEdit: (op) => {
      pending.push(useAppStore.getState().applyEdit(op));
    },
    drawSegment: vi.fn(),
    drawCurve: vi.fn(),
    setSelection: vi.fn(),
    beginFoldDraft: vi.fn(),
    pickAlignTarget: vi.fn(),
  };
  for (const click of clicks) onMouseDown(ctx, toConstructScreen(click), 0);
  await Promise.all(pending);

  return {
    lines: editCalls(),
    requests:
      vi.mocked(ipc.editApply).mock.calls.length +
      vi.mocked(ipc.editApplyBatch).mock.calls.length,
  };
}

type ConstructMirrorCase = {
  label: string;
  options: Partial<ConstructOptions>;
  clicks: Vec2[];
  /** 左右対称OFFの本数。作図は鏡映の経路を通っていなかったので、
   *  これがそのまま「変更前に左右対称ONで引けていた本数」でもある(不具合D13)。 */
  alone: number;
  /** 左右対称ONで引けるべき本数。 */
  mirrored: number;
};

const DIVIDE_CLICKS: Vec2[] = [
  [0.125, 0.25],
  [0.375, 0.25],
];

/** 基準線(紙の縦の中心線)の片側だけで作図した場合。本数はちょうど倍になる。 */
const CONSTRUCT_MIRROR_CASES: ConstructMirrorCase[] = [
  {
    label: "二等分",
    options: { kind: "bisector" },
    clicks: [
      [0.125, 0.25],
      [0.25, 0.25],
      [0.25, 0.5],
    ],
    alone: 1,
    mirrored: 2,
  },
  {
    label: "垂線",
    options: { kind: "perpendicular" },
    clicks: [
      [0.25, 0],
      [0.25, 0.5],
    ],
    alone: 1,
    mirrored: 2,
  },
  ...[2, 3, 4, 5, 6, 7, 8].map((divisions) => ({
    label: `${divisions}等分`,
    options: { kind: "divide" as const, divisions },
    clicks: DIVIDE_CLICKS,
    alone: divisions - 1,
    mirrored: (divisions - 1) * 2,
  })),
  // 角度線は0°から刻みごとに180°未満まで並ぶ。水平の1本だけが基準線に対して
  // 自分自身と重なるので、反対側は刻みの数より1本少ない。
  ...[15, 22.5, 30, 45].map((stepDeg) => {
    const alone = Math.ceil(180 / stepDeg);
    return {
      label: `${stepDeg}度の角度線`,
      options: { kind: "angle" as const, stepDeg },
      clicks: [[0.25, 0.5] as Vec2],
      alone,
      mirrored: alone * 2 - 1,
    };
  }),
];

/** 基準線の上に乗る線・基準線をはさんで既に対になっている線を含む作図。 */
const CONSTRUCT_SELF_SYMMETRIC_CASES: ConstructMirrorCase[] = [
  {
    label: "基準線の上にできる二等分線",
    options: { kind: "bisector" },
    clicks: [
      [0.25, 0.25],
      [0.5, 0.5],
      [0.75, 0.25],
    ],
    alone: 1,
    mirrored: 1,
  },
  {
    label: "基準線をはさんで対になる4等分の目印",
    options: { kind: "divide", divisions: 4 },
    clicks: [
      [0.125, 0.25],
      [0.875, 0.25],
    ],
    alone: 3,
    mirrored: 3,
  },
  {
    label: "基準線の上の点から出す22.5度の角度線",
    options: { kind: "angle", stepDeg: 22.5 },
    clicks: [[0.5, 0.5]],
    alone: 8,
    mirrored: 8,
  },
];

describe("D13: 作図で作った線も左右対称に引く", () => {
  for (const one of [
    ...CONSTRUCT_MIRROR_CASES,
    ...CONSTRUCT_SELF_SYMMETRIC_CASES,
  ]) {
    it(`${one.label}は左右対称ONで${one.mirrored}本になり、履歴は1件`, async () => {
      const result = await runConstruct(one.options, one.clicks, true);
      const added = result.lines.filter((op) => op.type === "AddSegment");

      expect(result.lines).toHaveLength(one.mirrored);
      expect(added).toHaveLength(one.mirrored);
      expect(
        result.requests,
        `${one.mirrored}本が${result.requests}件の履歴に分裂している`,
      ).toBe(1);
    });

    it(`${one.label}は左右対称OFFなら${one.alone}本のまま`, async () => {
      const result = await runConstruct(one.options, one.clicks, false);

      expect(result.lines).toHaveLength(one.alone);
      expect(result.requests).toBe(1);
    });
  }

  it("左右対称ONの作図線は、元の線と反対側の線が対になっている", async () => {
    const result = await runConstruct(
      { kind: "divide", divisions: 4 },
      DIVIDE_CLICKS,
      true,
    );
    const added = result.lines.flatMap((op) =>
      op.type === "AddSegment" ? [[op.a, op.b] as [Vec2, Vec2]] : [],
    );
    expect(added).toHaveLength(6);
    for (let i = 0; i < 3; i += 1) {
      const source = added[i];
      const reflected = added[i + 3];
      expect(reflected[0][0]).toBeCloseTo(1 - source[0][0], 12);
      expect(reflected[1][0]).toBeCloseTo(1 - source[1][0], 12);
      expect(reflected[0][1]).toBeCloseTo(source[0][1], 12);
      expect(reflected[1][1]).toBeCloseTo(source[1][1], 12);
    }
  });
});

describe("手順の並べ替え", () => {
  async function moveWhileUpdateIsPending(
    delayMs: number,
    patch: Partial<Pick<FoldStep, "note" | "kind">>,
  ): Promise<FoldStep[]> {
    let backend = makeDoc([step(1), step(2), step(3)]);
    let releaseUpdate!: () => void;
    const updateGate = new Promise<void>((resolve) => {
      releaseUpdate = resolve;
    });
    vi.mocked(ipc.sequenceApply).mockImplementation(async (op) => {
      if (op.type === "UpdateStep") {
        await updateGate;
        backend = {
          ...backend,
          sequence: backend.sequence.map((item) =>
            item.id === op.step.id ? structuredClone(op.step) : item,
          ),
        };
      } else if (op.type === "RemoveStep") {
        backend = {
          ...backend,
          sequence: backend.sequence.filter((item) => item.id !== op.id),
        };
      } else if (op.type === "InsertStep") {
        const sequence = [...backend.sequence];
        sequence.splice(op.index, 0, structuredClone(op.step));
        backend = { ...backend, sequence };
      }
      return makeView(structuredClone(backend));
    });
    vi.mocked(ipc.sequenceReplay).mockResolvedValue({
      frame: { faces: [], warnings: [] },
      skipped: [],
      warnings: [],
    });
    useAppStore.setState({ doc: structuredClone(backend) });

    const changed = { ...step(3), ...patch };
    const update = useAppStore
      .getState()
      .applySequenceOp({ type: "UpdateStep", step: changed });
    await new Promise((resolve) => setTimeout(resolve, delayMs));
    const move = useAppStore.getState().moveStep(3, -1);
    releaseUpdate();
    await Promise.all([update, move]);
    return useAppStore.getState().doc?.sequence ?? [];
  }

  it.each([0, 1, 100])(
    "D08: 覚え書きの確定から%dms後に前へ動かしても新しい内容を保つ",
    async (delayMs) => {
      const sequence = await moveWhileUpdateIsPending(delayMs, { note: "書き直した覚え書き" });
      expect(sequence.map((item) => item.id)).toEqual([1, 3, 2]);
      expect(sequence[1].note).toBe("書き直した覚え書き");
    },
  );

  it.each([0, 1, 100])(
    "D08: 折り方の確定から%dms後に前へ動かしても新しい種類を保つ",
    async (delayMs) => {
      const sequence = await moveWhileUpdateIsPending(delayMs, { kind: "InsideReverse" });
      expect(sequence.map((item) => item.id)).toEqual([1, 3, 2]);
      expect(sequence[1].kind).toBe("InsideReverse");
    },
  );

  it("選んだ手順を前へ動かすと、取り除いてから同じ手順を入れ直す", async () => {
    const doc = makeDoc([step(1), step(2), step(3)]);
    useAppStore.setState({ doc });
    vi.mocked(ipc.sequenceApply).mockResolvedValue(makeView(doc));
    vi.mocked(ipc.sequenceReplay).mockResolvedValue({
      frame: { faces: [], warnings: [] },
      skipped: [],
      warnings: [],
    });

    await useAppStore.getState().moveStep(3, -1);
    expect(vi.mocked(ipc.sequenceApply).mock.calls[0][0]).toEqual({
      type: "RemoveStep",
      id: 3,
    });
    expect(vi.mocked(ipc.sequenceApply).mock.calls[1][0]).toEqual({
      type: "InsertStep",
      index: 1,
      step: step(3),
    });
  });

  it("端の手順はそれ以上動かさない(要求も送らない)", async () => {
    useAppStore.setState({ doc: makeDoc([step(1), step(2)]) });
    await useAppStore.getState().moveStep(1, -1);
    await useAppStore.getState().moveStep(2, 1);
    expect(ipc.sequenceApply).not.toHaveBeenCalled();
  });
});
