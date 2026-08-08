// @vitest-environment jsdom
// 「この形で仕上げる」ボタン(SIM-009)の画面テスト。
// 押せないときもボタンは消さず、理由を日本語で見せることを確かめる。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { ContextPanel } from "./ContextPanel";
import { resetPoseThrottle, useAppStore } from "../store/appStore";
import type { Document } from "../lib/types";
import { DEFAULT_CURVE } from "../lib/curve";

vi.mock("../ipc/client", () => ({
  sequenceApply: vi.fn(),
  sequenceReplay: vi.fn(),
  poseSolve: vi.fn(),
}));

import * as ipc from "../ipc/client";

/** 対角線(辺ID 5)が折り線の正方形 */
function makeDoc(): Document {
  return {
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
}

/** 折り線を1本選んだ状態にする(角度は呼び出し側で足す) */
function seed(drivers: Map<number, number>, poseAngles = new Map<number, number>()) {
  useAppStore.setState({
    doc: makeDoc(),
    faces: [
      { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
      { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
    ],
    hinges: new Set([5]),
    selection: { edgeIds: [5], vertexIds: [] },
    drivers,
    poseAngles,
    currentStep: null,
    playing: false,
    playT: 1,
    foldDraft: null,
    pendingFoldThrough: null,
    foldThroughBusy: false,
    techniqueDraft: null,
    warnings: [],
    poseWarnings: [],
    replayWarnings: [],
    errorMessage: null,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  resetPoseThrottle();
  vi.mocked(ipc.poseSolve).mockResolvedValue({
    frame: { faces: [], warnings: [] },
    converged: true,
    angles: {},
    iterations: 1,
    soft: null,
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    doc: null,
    drivers: new Map(),
    poseAngles: new Map(),
    hoveredHinge: null,
  });
});

describe("複数の折り目の角度(SIM-001)", () => {
  function seedMultiple() {
    seed(
      new Map([
        [5, 30],
        [7, -20],
      ]),
      new Map([[9, 75]]),
    );
    const doc = makeDoc();
    doc.cp.edges.push(
      { id: 7, v0: 1, v1: 3, kind: "Valley" },
      { id: 9, v0: 0, v1: 1, kind: "Mountain" },
    );
    useAppStore.setState({
      doc,
      hinges: new Set([5, 7, 9]),
      selection: { edgeIds: [9, 5, 7], vertexIds: [2] },
    });
  }

  it("複数選択では一括スライダーを先頭に置き、各折り目のスライダーを縦に出す", () => {
    seedMultiple();
    render(<ContextPanel />);

    expect(screen.getByText("折り目を3本選択中")).not.toBeNull();
    expect(screen.getByLabelText("選択した折り目をまとめて動かす")).not.toBeNull();
    expect(screen.getByText("角度はばらばら")).not.toBeNull();
    for (const hinge of [5, 7, 9]) {
      expect(screen.getByLabelText(`折り目 #${hinge}の角度`)).not.toBeNull();
    }
    expect(screen.getByLabelText("選択した折り目ごとの角度").children).toHaveLength(3);
    const controls = document.querySelector(".fold-controls") as HTMLElement;
    const sliders = within(controls).getAllByRole("slider");
    expect(sliders[0]).toBe(
      screen.getByLabelText("選択した折り目をまとめて動かす"),
    );
  });

  it("一括スライダーは選択中の全折り目を同じ絶対角度へそろえる", () => {
    seedMultiple();
    render(<ContextPanel />);

    fireEvent.change(screen.getByLabelText("選択した折り目をまとめて動かす"), {
      target: { value: "45" },
    });
    const drivers = useAppStore.getState().drivers;
    expect([drivers.get(5), drivers.get(7), drivers.get(9)]).toEqual([45, 45, 45]);
    expect(screen.getAllByText("45°")).toHaveLength(4);
  });

  it("各行を指すと該当する折り目IDを2D/3D強調用にストアへ置く", () => {
    seedMultiple();
    render(<ContextPanel />);
    const row = screen.getByLabelText("折り目 #7の角度設定");

    fireEvent.mouseEnter(row);
    expect(useAppStore.getState().hoveredHinge).toBe(7);
    fireEvent.mouseLeave(row);
    expect(useAppStore.getState().hoveredHinge).toBeNull();
  });

  it("1本選択では従来どおり個別の折り角度だけを出す", () => {
    seed(new Map([[5, 90]]));
    render(<ContextPanel />);

    expect(screen.getByText("折り角度")).not.toBeNull();
    expect(screen.getByLabelText("折り目 #5の角度")).not.toBeNull();
    expect(screen.queryByLabelText("選択した折り目をまとめて動かす")).toBeNull();
    expect(screen.getByRole("button", { name: "この折り線の角度を解除" })).not.toBeNull();
  });

  it("最小ウィンドウでも折り角度を最上部に置き、操作手順はその下で閉じておく", () => {
    seed(new Map([[5, 90]]));
    render(<ContextPanel />);

    const panel = document.querySelector(".context-selection")!;
    const controls = panel.querySelector(".fold-controls") as HTMLElement;
    const operationSteps = panel.querySelector(".operation-steps") as HTMLElement;
    expect(panel.firstElementChild).toBe(controls);
    expect(controls.nextElementSibling).toBe(operationSteps);
    expect(within(controls).getByRole("slider", { name: "折り目 #5の角度" })).toBeTruthy();
    expect(operationSteps.querySelector("details")?.open).toBe(false);
  });
});

describe("コンテキストパネルの主操作順", () => {
  it("何も選んでいないときは現在ツールの1行ガイドを最初に出す", () => {
    seed(new Map());
    useAppStore.setState({
      activeTool: "select",
      selection: { edgeIds: [], vertexIds: [] },
    });
    render(<ContextPanel />);

    const panel = document.querySelector(".context-selection") as HTMLElement;
    expect(panel.firstElementChild?.classList.contains("operation-steps")).toBe(true);
    expect(panel.querySelector("details")?.open).toBe(false);
  });

  it.each([
    {
      label: "辺",
      selection: { edgeIds: [0], vertexIds: [] },
      heading: "線を1本選択中",
    },
    {
      label: "頂点",
      selection: { edgeIds: [], vertexIds: [2] },
      heading: "点を1個選択中",
    },
  ])("$labelでは対象の操作をガイドより先に出す", ({ selection, heading }) => {
    seed(new Map());
    useAppStore.setState({ activeTool: "select", selection });
    render(<ContextPanel />);

    const panel = document.querySelector(".context-selection") as HTMLElement;
    expect(panel.firstElementChild?.textContent).toContain(heading);
    expect(panel.lastElementChild?.classList.contains("operation-steps")).toBe(true);
  });

  it("手順を選んだときは手順設定をガイドより先に出す", () => {
    seed(new Map());
    const doc = makeDoc();
    doc.sequence = [
      { id: 1, kind: "Simple", drivers: [], layer_order: null, note: "" },
    ];
    useAppStore.setState({
      doc,
      activeTool: "select",
      currentStep: 1,
      selection: { edgeIds: [], vertexIds: [] },
    });
    render(<ContextPanel />);

    const panel = document.querySelector(".context-selection") as HTMLElement;
    expect(panel.firstElementChild?.textContent).toContain("手順1");
    expect(panel.lastElementChild?.classList.contains("operation-steps")).toBe(true);
  });
});

describe("この形で仕上げる(SIM-009)", () => {
  it("角度が付いていなければ、ボタンは残したまま理由を見せる", () => {
    seed(new Map());
    render(<ContextPanel />);

    const button = screen.getByRole("button", { name: "この形で仕上げる" });
    expect(button).toHaveProperty("disabled", true);
    expect(screen.getAllByText(/まだ角度が付いていません/).length).toBeGreaterThan(0);
  });

  it("角度が付いていれば押せて、手順として送られる", async () => {
    seed(new Map([[5, 90]]));
    vi.mocked(ipc.sequenceApply).mockResolvedValue({
      doc: makeDoc(),
      faces: [],
      warnings: [],
      violations: [],
      frame: null,
      skipped: [],
    });
    render(<ContextPanel />);

    const button = screen.getByRole("button", { name: "この形で仕上げる" });
    expect(button).toHaveProperty("disabled", false);
    fireEvent.click(button);
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(vi.mocked(ipc.sequenceApply)).toHaveBeenCalledTimes(1);
    const op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type === "PushStep" && op.step.kind).toBe("Pose");
  });
});

describe("巻き込み折り目の提案", () => {
  function seedProposal() {
    seed(new Map());
    useAppStore.setState({
      pendingFoldThrough: {
        proposal: {
          folded_line: [
            [0.25, 0.5],
            [0.75, 0.5],
          ],
          crease_segments: [
            [
              [0.2, 0.4],
              [0.8, 0.4],
            ],
          ],
          message: "重なりの縁に沿って巻き込むと、紙の突き抜けを避けられます。",
        },
        operation: {
          type: "FoldThrough",
          up_to: 0,
          line: [
            [0.5, 0],
            [0.5, 1],
          ],
          keep_side_point: [0.25, 0.5],
          target_layers: null,
          direction: "Up",
        },
        docEpoch: useAppStore.getState().docEpoch,
        stepCount: 0,
      },
    });
  }

  function resolvedView() {
    return {
      doc: makeDoc(),
      faces: [],
      warnings: [],
      violations: [],
      frame: null,
      skipped: [],
    };
  }

  it("作業を止めるダイアログではなく、下部パネルへ日本語の選択肢を出す", () => {
    seedProposal();
    render(<ContextPanel />);

    expect(screen.getByLabelText("巻き込み折り目の提案")).toBeTruthy();
    expect(screen.getByText("指定した場所以外に、ここへ折り目がつきます")).toBeTruthy();
    expect(screen.getByText(/展開図の橙色の破線.*3D表示の水色/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "追加折り目を入れて折る" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "追加せず折る（警告のみ）" })).toBeTruthy();
  });

  it("承諾なら追加折り目あり、拒否なら追加なしで元の折りを送る", async () => {
    vi.mocked(ipc.sequenceApply).mockResolvedValue(resolvedView());
    seedProposal();
    render(<ContextPanel />);

    fireEvent.click(screen.getByRole("button", { name: "追加折り目を入れて折る" }));
    await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(1));
    let op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type === "FoldThrough" && op.accept_additional_crease).toBe(true);
    await waitFor(() => expect(useAppStore.getState().pendingFoldThrough).toBeNull());

    cleanup();
    vi.mocked(ipc.sequenceApply).mockClear();
    seedProposal();
    render(<ContextPanel />);
    fireEvent.click(screen.getByRole("button", { name: "追加せず折る（警告のみ）" }));
    await waitFor(() => expect(ipc.sequenceApply).toHaveBeenCalledTimes(1));
    op = vi.mocked(ipc.sequenceApply).mock.calls[0][0];
    expect(op.type === "FoldThrough" && op.accept_additional_crease).toBe(false);
    await waitFor(() => expect(useAppStore.getState().pendingFoldThrough).toBeNull());
  });
});

describe("引くツールの左右同時の切替(UI-007)", () => {
  it("引くツールを選ぶと切替が出て、押すと設定が変わる", () => {
    seed(new Map());
    // 何も選んでいない状態で「引く」ツールにする(常設UIは増やさない)
    useAppStore.setState({
      activeTool: "pull",
      selection: { edgeIds: [], vertexIds: [] },
      pullMirror: true,
    });
    render(<ContextPanel />);

    const box = screen.getByLabelText("左右対称に動かす") as HTMLInputElement;
    expect(box.checked).toBe(true); // 既定はオン(作品はほとんど左右対称なので)
    expect(screen.getAllByText(/鶴の両羽が一緒に開きます/).length).toBe(1);

    fireEvent.click(box);
    expect(useAppStore.getState().pullMirror).toBe(false);
    expect(screen.getAllByText(/つかんだ側の折り線だけが動きます/).length).toBe(1);
  });

  it("他のツールでは出さない(下部パネルの内容を増やしすぎない)", () => {
    seed(new Map());
    useAppStore.setState({
      activeTool: "select",
      selection: { edgeIds: [], vertexIds: [] },
    });
    render(<ContextPanel />);
    expect(screen.queryByLabelText("左右対称に動かす")).toBeNull();
  });
});

describe("ねじり折りの中央多角形(TEC-009)", () => {
  /** ねじり折りを選び、角をcount個置いた状態にする */
  function seedTwist(count: number, center: [number, number] | null = null) {
    seed(new Map());
    const pts: [number, number][] = [
      [0.2, 0.2],
      [0.8, 0.2],
      [0.5, 0.9],
      [0.3, 0.5],
    ];
    useAppStore.setState({
      activeTool: "technique",
      selection: { edgeIds: [], vertexIds: [] },
      techniqueDraft: {
        kind: "Twist",
        flap: [],
        line: null,
        movingSide: "right",
        widthMm: 10,
        polygon: pts.slice(0, count),
        center,
        twistDeg: 30,
        docEpoch: 0,
        stepCount: 0,
        upTo: 0,
      },
    });
  }

  it("角が足りないうちは、何をすればよいかを見せて適用できない", () => {
    seedTwist(2);
    render(<ContextPanel />);

    expect(screen.getAllByText(/角を2個指定/).length).toBe(1);
    expect(screen.getAllByText(/あと3個以上必要/).length).toBe(1);
    expect(screen.getAllByText(/角を順にクリック/).length).toBeGreaterThan(0);
    const apply = screen.getByRole("button", { name: "適用" });
    expect(apply).toHaveProperty("disabled", true);
  });

  it("3つ以上そろえば、層を選ばなくても適用できる", () => {
    seedTwist(3);
    render(<ContextPanel />);

    expect(screen.getAllByText(/3角形/).length).toBe(1);
    expect(screen.getByRole("button", { name: "適用" })).toHaveProperty(
      "disabled",
      false,
    );
    // ねじる角は数値で決められる(既定30度)
    const deg = screen.getByLabelText("ねじる角(度)") as HTMLInputElement;
    expect(deg.value).toBe("30");
  });

  it("角を1つ戻す・中心を重心へ戻すが効く", () => {
    seedTwist(3, [0.4, 0.4]);
    render(<ContextPanel />);

    expect(screen.getAllByText(/中心は指定した点/).length).toBe(1);
    fireEvent.click(screen.getByRole("button", { name: "中心を重心へ戻す" }));
    expect(useAppStore.getState().techniqueDraft?.center).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "角を1つ戻す" }));
    expect(useAppStore.getState().techniqueDraft?.polygon).toHaveLength(2);
  });
});

/** 合わせて折るの下ごしらえ(選び終えて折り線が求まった状態)にする */
function seedAlign(mode: "pointPoint" | "lineLine") {
  seed(new Map());
  useAppStore.setState({
    activeTool: "fold",
    selection: { edgeIds: [], vertexIds: [] },
    alignDraft: null,
    foldDraft: null,
  });
  const s = useAppStore.getState();
  s.beginAlign(mode);
  if (mode === "pointPoint") {
    s.pickAlignTarget({ kind: "point", p: [0, 0] });
    s.pickAlignTarget({ kind: "point", p: [1, 1] });
  } else {
    s.pickAlignTarget({ kind: "line", a: [0, 0], b: [1, 0] }, [1, 1]);
    s.pickAlignTarget({ kind: "line", a: [0, 0], b: [0, 1] }, [1, 1]);
  }
}

describe("合わせて折る(パネル)", () => {
  it("折るツールのときだけ3つの合わせ方を出す", () => {
    seed(new Map());
    useAppStore.setState({ activeTool: "fold" });
    render(<ContextPanel />);
    expect(screen.getByRole("button", { name: "点と点を合わせる" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "線と線を合わせる" })).toBeTruthy();
    cleanup();

    useAppStore.setState({ activeTool: "select" });
    render(<ContextPanel />);
    expect(screen.queryByRole("button", { name: "点と点を合わせる" })).toBeNull();
  });

  it("選択の途中経過を出し、そろうと既存の折り確定UI(山谷・折る)が出る", () => {
    seed(new Map());
    useAppStore.setState({ activeTool: "fold", alignDraft: null, foldDraft: null });
    useAppStore.getState().beginAlign("pointPoint");
    render(<ContextPanel />);
    expect(screen.getByText(/選択 0 \/ 2/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "折る" })).toBeNull();

    cleanup();
    seedAlign("pointPoint");
    render(<ContextPanel />);
    expect(screen.getByText(/選択 2 \/ 2/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "折る" })).toBeTruthy();
    expect(screen.getByText(/手前へ折る/)).toBeTruthy();
    // 解が1本しかないときは切り替えボタンを出さない
    expect(screen.queryByRole("button", { name: /別の解/ })).toBeNull();
  });

  it("解が2つあるときは「別の解」で切り替えられる", () => {
    seedAlign("lineLine");
    render(<ContextPanel />);
    const button = screen.getByRole("button", { name: "別の解(1/2)" });
    const before = useAppStore.getState().foldDraft!.line;
    fireEvent.click(button);
    expect(useAppStore.getState().alignDraft?.solutionIndex).toBe(1);
    expect(useAppStore.getState().foldDraft!.line).not.toEqual(before);
    expect(screen.getByRole("button", { name: "別の解(2/2)" })).toBeTruthy();
  });

  it("「1つ戻す」と「合わせるのをやめる」で取り消せる", () => {
    seedAlign("pointPoint");
    render(<ContextPanel />);
    fireEvent.click(screen.getByRole("button", { name: "1つ戻す" }));
    expect(useAppStore.getState().alignDraft?.picks).toHaveLength(1);
    expect(useAppStore.getState().foldDraft).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "合わせるのをやめる" }));
    expect(useAppStore.getState().alignDraft).toBeNull();
  });

  it("折り線が求まらないときは理由を見せる", () => {
    seed(new Map());
    useAppStore.setState({ activeTool: "fold", alignDraft: null, foldDraft: null });
    const s = useAppStore.getState();
    s.beginAlign("pointLineThrough");
    s.pickAlignTarget({ kind: "point", p: [0, 1] });
    s.pickAlignTarget({ kind: "line", a: [0, 0], b: [1, 0] });
    s.pickAlignTarget({ kind: "point", p: [0, 3] });
    render(<ContextPanel />);
    expect(screen.getByText(/届きません/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "折る" })).toBeNull();
  });
});

describe("曲線の折り目の設定(CPE-011)", () => {
  /** 線ツールを選んで何も選択していない状態 */
  function seedLineTool() {
    seed(new Map());
    useAppStore.setState({
      activeTool: "valley",
      selection: { edgeIds: [], vertexIds: [] },
      alignDraft: null,
      curve: DEFAULT_CURVE,
    });
  }

  it("線ツールのときだけ曲線の切り替えが出る", () => {
    seedLineTool();
    render(<ContextPanel />);
    expect(screen.getByLabelText("曲線で描く")).toBeTruthy();
    cleanup();
    useAppStore.setState({ activeTool: "select" });
    render(<ContextPanel />);
    expect(screen.queryByLabelText("曲線で描く")).toBeNull();
  });

  it("曲線に切り替えると描き方・分割・曲がるための線を選べる", () => {
    seedLineTool();
    render(<ContextPanel />);
    // 切る前は細かい設定を出さない(画面を混ませない)
    expect(screen.queryByLabelText("紙が曲がるための線も引く")).toBeNull();
    fireEvent.click(screen.getByLabelText("曲線で描く"));
    expect(useAppStore.getState().curve.enabled).toBe(true);
    fireEvent.change(screen.getByLabelText("描き方"), { target: { value: "bezier" } });
    expect(useAppStore.getState().curve.shape).toBe("bezier");
    // 分割は既定で自動、指定に切り替えられる
    expect(useAppStore.getState().curve.segments).toBeNull();
    fireEvent.click(screen.getByLabelText("分割の細かさを自分で決める"));
    expect(useAppStore.getState().curve.segments).toBe(16);
    // 曲がるための線は既定でオン(これが無いと曲線折りは折れない)
    const rulings = screen.getByLabelText("紙が曲がるための線も引く");
    expect((rulings as HTMLInputElement).checked).toBe(true);
    fireEvent.click(rulings);
    expect(useAppStore.getState().curve.rulings).toBe(false);
    expect(screen.getByText(/このままでは折れません/)).toBeTruthy();
  });
});
