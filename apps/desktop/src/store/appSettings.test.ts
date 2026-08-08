// 新規作成の紙の指定(PAP-001)・紙の色と方眼(PAP-003 / CPE-003)・
// 2D/3Dの分割比(UI-004)・手順の並べ替え(SEQ-005)のテスト。

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Document, DocumentView, FoldStep } from "../lib/types";

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
import { DEFAULT_NEW_PAPER, useAppStore } from "./appStore";
import { DEFAULT_DISPLAY } from "../lib/displayPrefs";

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
    errorMessage: null,
    currentStep: null,
    mirrorDraw: false,
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

describe("紙の色と方眼・分割比", () => {
  it("色と方眼の数は作品ごとの設定として保存する(SetDisplayを送る)", async () => {
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

    // 範囲外は上限(64)に丸めてから送る
    await useAppStore.getState().setDisplay({ grid_divisions: 100 });
    const last = vi.mocked(ipc.editApply).mock.calls[1][0];
    if (last.type !== "SetDisplay") throw new Error("SetDisplayでない");
    expect(last.display.grid_divisions).toBe(64);
    expect(useAppStore.getState().doc?.display.grid_divisions).toBe(64);
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
});

describe("左右対称に線を引く", () => {
  /** 線を1本引く準備(正方形の紙・編集は毎回成功する) */
  function ready(mirrorDraw: boolean) {
    const doc = makeDoc([]);
    useAppStore.setState({ doc, mirrorDraw });
    vi.mocked(ipc.editApply).mockResolvedValue(makeView(doc));
  }

  const calls = () => vi.mocked(ipc.editApply).mock.calls.map((c) => c[0]);

  it("入れておくと、中心線の反対側にも同じ線が引かれる", async () => {
    ready(true);
    await useAppStore.getState().drawSegment([0.25, 0], [0.375, 1], "Mountain");
    expect(calls()).toEqual([
      { type: "AddSegment", a: [0.25, 0], b: [0.375, 1], kind: "Mountain" },
      { type: "AddSegment", a: [0.75, 0], b: [0.625, 1], kind: "Mountain" },
    ]);
  });

  it("中心線に重なる線・もともと左右対称な線は1本だけになる", async () => {
    ready(true);
    await useAppStore.getState().drawSegment([0.5, 0], [0.5, 1], "Valley");
    await useAppStore.getState().drawSegment([0.25, 0.5], [0.75, 0.5], "Valley");
    expect(calls()).toEqual([
      { type: "AddSegment", a: [0.5, 0], b: [0.5, 1], kind: "Valley" },
      { type: "AddSegment", a: [0.25, 0.5], b: [0.75, 0.5], kind: "Valley" },
    ]);
  });

  it("切ってあるときは引いた線だけを引く", async () => {
    ready(false);
    await useAppStore.getState().drawSegment([0.25, 0], [0.375, 1], "Aux");
    expect(calls()).toEqual([
      { type: "AddSegment", a: [0.25, 0], b: [0.375, 1], kind: "Aux" },
    ]);
  });
});

describe("左右対称に消す・線種を変える", () => {
  /** 縦の中心線を対称軸とする展開図(辺10と辺11が左右の対、辺12は相手なし) */
  function symmetricDoc(): Document {
    const doc = makeDoc([]);
    doc.cp = {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [1, 1] },
        { id: 3, pos: [0, 1] },
        { id: 4, pos: [0.5, 0] },
        { id: 5, pos: [0.5, 1] },
        { id: 8, pos: [0.2, 0.4] },
        { id: 9, pos: [0.2, 0.6] },
      ],
      edges: [
        { id: 0, v0: 0, v1: 4, kind: "Border" },
        { id: 1, v0: 4, v1: 1, kind: "Border" },
        { id: 2, v0: 1, v1: 2, kind: "Border" },
        { id: 3, v0: 2, v1: 5, kind: "Border" },
        { id: 4, v0: 5, v1: 3, kind: "Border" },
        { id: 5, v0: 3, v1: 0, kind: "Border" },
        { id: 10, v0: 0, v1: 5, kind: "Mountain" },
        { id: 11, v0: 1, v1: 5, kind: "Mountain" },
        { id: 12, v0: 8, v1: 9, kind: "Aux" },
      ],
      next_vertex_id: 10,
      next_edge_id: 13,
    };
    return doc;
  }

  function ready(mirrorDraw: boolean) {
    const doc = symmetricDoc();
    useAppStore.setState({
      doc,
      mirrorDraw,
      faces: [
        { id: 0, vertices: [0, 4, 5], edges: [0, 4, 10] },
        { id: 1, vertices: [4, 1, 5], edges: [1, 11, 4] },
        { id: 2, vertices: [0, 5, 3], edges: [10, 4, 5] },
        { id: 3, vertices: [1, 2, 5], edges: [2, 3, 11] },
      ],
    });
    vi.mocked(ipc.editApply).mockResolvedValue(makeView(doc));
  }

  const lastCall = () => {
    const c = vi.mocked(ipc.editApply).mock.calls;
    return c[c.length - 1][0];
  };

  it("削除は鏡映の相手も一緒に消す", async () => {
    ready(true);
    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [10] });
    const op = lastCall();
    if (op.type !== "RemoveEdges") throw new Error("RemoveEdgesでない");
    expect([...op.ids].sort()).toEqual([10, 11]);
  });

  it("線種の変更も鏡映の相手に効く", async () => {
    ready(true);
    await useAppStore
      .getState()
      .applyEdit({ type: "SetEdgeKind", ids: [11], kind: "Valley" });
    const op = lastCall();
    if (op.type !== "SetEdgeKind") throw new Error("SetEdgeKindでない");
    expect([...op.ids].sort()).toEqual([10, 11]);
    expect(op.kind).toBe("Valley");
  });

  it("鏡映の相手がいない線は、その線だけを消す(警告は出さない)", async () => {
    ready(true);
    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [12] });
    const op = lastCall();
    if (op.type !== "RemoveEdges") throw new Error("RemoveEdgesでない");
    expect(op.ids).toEqual([12]);
    expect(useAppStore.getState().errorMessage).toBeNull();
  });

  it("切ってあるときは選んだ線だけが変わる", async () => {
    ready(false);
    await useAppStore.getState().applyEdit({ type: "RemoveEdges", ids: [10] });
    const op = lastCall();
    if (op.type !== "RemoveEdges") throw new Error("RemoveEdgesでない");
    expect(op.ids).toEqual([10]);
  });

  it("線を引く以外の編集(点を動かす等)はそのまま送る", async () => {
    ready(true);
    await useAppStore.getState().applyEdit({ type: "MoveVertex", id: 8, to: [0.3, 0.4] });
    expect(lastCall()).toEqual({ type: "MoveVertex", id: 8, to: [0.3, 0.4] });
  });
});

describe("手順の並べ替え", () => {
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
