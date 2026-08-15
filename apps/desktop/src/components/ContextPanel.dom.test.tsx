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
import { ALIGN_LABELS } from "../lib/alignFold";

vi.mock("../ipc/client", () => ({
  documentSave: vi.fn(),
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
    sequenceTargets: new Map(),
    relaxations: [],
    poseBestEffort: false,
    activeAngleIntent: null,
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
    flatFoldViolations: [],
    errorMessage: null,
    documentSavedPath: null,
    mirrorAxis: { kind: "paperVertical" },
    mirrorAxisNotice: null,
    contextHelpExpanded: true,
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
    sequenceTargets: new Map(),
    relaxations: [],
    flatFoldViolations: [],
    activeAngleIntent: null,
    hoveredHinge: null,
    documentSavedPath: null,
    mirrorAxis: { kind: "paperVertical" },
    mirrorAxisNotice: null,
    contextHelpExpanded: true,
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

  it("数値入力のchangeだけで個別・一括の角度をストアへ反映する", () => {
    seedMultiple();
    render(<ContextPanel />);

    const individual = screen.getByLabelText("折り目 #7の角度（数値）");
    fireEvent.change(individual, { target: { value: "-21" } });
    let drivers = useAppStore.getState().drivers;
    expect([drivers.get(5), drivers.get(7), drivers.get(9)]).toEqual([
      30,
      -21,
      undefined,
    ]);

    const bulk = screen.getByLabelText(
      "選択した折り目をまとめて動かす角度（数値）",
    );
    fireEvent.change(bulk, { target: { value: "46" } });
    drivers = useAppStore.getState().drivers;
    expect([drivers.get(5), drivers.get(7), drivers.get(9)]).toEqual([46, 46, 46]);
  });

  it("テーマ共通の上下ボタンと入力欄の矢印キーで角度を確定し、上下限を越えない", () => {
    seed(new Map([[5, 30]]));
    const original = useAppStore.getState().finishAngleIntent;
    const finishAngleIntent = vi.fn(async () => {});
    useAppStore.setState({ finishAngleIntent });
    render(<ContextPanel />);

    const input = screen.getByLabelText("折り目 #5の角度（数値）");
    const increase = screen.getByRole("button", {
      name: "折り目 #5の角度（数値）を増やす",
    });
    const decrease = screen.getByRole("button", {
      name: "折り目 #5の角度（数値）を減らす",
    });

    increase.focus();
    expect(document.activeElement).toBe(increase);
    expect(increase.getAttribute("data-tooltip")).toContain("Shift");
    expect(increase.hasAttribute("title")).toBe(false);

    fireEvent.click(increase);
    expect(useAppStore.getState().drivers.get(5)).toBe(31);
    expect(input).toHaveProperty("value", "31");

    fireEvent.click(decrease, { shiftKey: true });
    expect(useAppStore.getState().drivers.get(5)).toBe(21);
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(useAppStore.getState().drivers.get(5)).toBe(22);

    fireEvent.change(input, { target: { value: "180" } });
    fireEvent.click(increase);
    expect(useAppStore.getState().drivers.get(5)).toBe(180);
    expect(input).toHaveProperty("value", "180");

    fireEvent.change(input, { target: { value: "-180" } });
    fireEvent.click(decrease);
    expect(useAppStore.getState().drivers.get(5)).toBe(-180);
    expect(input).toHaveProperty("value", "-180");
    expect(finishAngleIntent).toHaveBeenCalledTimes(3);
    useAppStore.setState({ finishAngleIntent: original });
  });

  it("数値入力はEnterなしで反映し、Enterでは最終値を丸める", () => {
    seed(new Map([[5, 30]]));
    render(<ContextPanel />);
    const input = screen.getByLabelText("折り目 #5の角度（数値）");
    fireEvent.focus(input);

    // スピナー・矢印キー・直接入力はいずれもブラウザからchangeとして届く
    fireEvent.change(input, { target: { value: "30.6" } });
    expect(useAppStore.getState().drivers.get(5)).toBe(30.6);

    fireEvent.keyDown(input, { key: "Enter" });
    expect(useAppStore.getState().drivers.get(5)).toBe(31);
    expect(input).toHaveProperty("value", "31");

    fireEvent.change(input, { target: { value: "999" } });
    expect(useAppStore.getState().drivers.get(5)).toBe(31);
    fireEvent.blur(input);
    expect(useAppStore.getState().drivers.get(5)).toBe(180);
    expect(input).toHaveProperty("value", "180");
  });

  it.each(["", "-", "12."])(
    "入力途中の「%s」では角度を更新しない",
    (incomplete) => {
      seed(new Map([[5, 30]]));
      render(<ContextPanel />);
      const input = screen.getByLabelText("折り目 #5の角度（数値）");
      fireEvent.focus(input);

      // jsdomのnumber入力は「-」「12.」を空文字へ正規化するが、いずれも
      // ブラウザでchangeが届いた時点では不完全な入力として扱われる
      fireEvent.change(input, { target: { value: incomplete } });
      expect(useAppStore.getState().drivers.get(5)).toBe(30);
    },
  );

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

  it("折り角度は最上部に置き、詳しい手順を閉じても角度と現在操作を残す", () => {
    seed(new Map([[5, 90]]));
    render(<ContextPanel />);

    const panel = document.querySelector(".context-selection")!;
    const controls = panel.querySelector(".fold-controls") as HTMLElement;
    const operationSteps = panel.querySelector(".operation-steps") as HTMLElement;
    expect(panel.firstElementChild).toBe(controls);
    expect(controls.nextElementSibling).toBe(operationSteps);
    expect(within(controls).getByRole("slider", { name: "折り目 #5の角度" })).toBeTruthy();
    const close = within(operationSteps).getByRole("button", {
      name: "この道具の詳しい操作方法 ▲",
    });
    expect(close.getAttribute("aria-expanded")).toBe("true");

    fireEvent.click(close);
    expect(within(controls).getByRole("slider", { name: "折り目 #5の角度" })).toBeTruthy();
    expect(within(operationSteps).getByText("今できる操作")).toBeTruthy();
    expect(within(operationSteps).getByText("紙と折り線を選ぶ")).toBeTruthy();
    expect(within(operationSteps).queryByRole("listitem")).toBeNull();
    expect(screen.queryByText(/左のツールを選んで操作します/)).toBeNull();
  });

  it("希望90度を入力欄に保ち、譲った実角72度を横へ併記する", () => {
    seed(new Map([[5, 90]]), new Map([[5, 72]]));
    useAppStore.setState({
      relaxations: [
        { hinge: 5, target_angle_deg: 90, actual_angle_deg: 72, delta_deg: -18 },
      ],
    });
    render(<ContextPanel />);

    expect(screen.getByLabelText("折り目 #5の角度")).toHaveProperty("value", "90");
    expect(screen.getByLabelText("折り目 #5の角度（数値）")).toHaveProperty(
      "value",
      "90",
    );
    expect(screen.getAllByText("現在72.0°").length).toBeGreaterThan(0);
  });

  it("一括角度も希望値を保ち、選択中の実角を併記する", () => {
    seedMultiple();
    useAppStore.setState({
      drivers: new Map([
        [5, 90],
        [7, 90],
        [9, 90],
      ]),
      poseAngles: new Map([
        [5, 72],
        [7, 72],
        [9, 72],
      ]),
      relaxations: [5, 7, 9].map((hinge) => ({
        hinge,
        target_angle_deg: 90,
        actual_angle_deg: 72,
        delta_deg: -18,
      })),
    });
    render(<ContextPanel />);

    const group = screen.getByLabelText("選択した折り目の一括角度設定");
    expect(within(group).getByRole("slider")).toHaveProperty("value", "90");
    expect(within(group).getByText("現在72.0°")).toBeTruthy();
  });

  it("スライダーと数値入力を離したときに末尾の追従計算を確定する", () => {
    seed(new Map([[5, 30]]));
    const original = useAppStore.getState().finishAngleIntent;
    const finishAngleIntent = vi.fn(async () => {});
    useAppStore.setState({ finishAngleIntent });
    render(<ContextPanel />);

    const slider = screen.getByLabelText("折り目 #5の角度");
    fireEvent.pointerUp(slider);
    fireEvent.keyUp(slider, { key: "ArrowRight" });
    fireEvent.blur(slider);

    const number = screen.getByLabelText("折り目 #5の角度（数値）");
    fireEvent.change(number, { target: { value: "31" } });
    fireEvent.keyDown(number, { key: "Enter" });
    fireEvent.blur(number);

    expect(finishAngleIntent).toHaveBeenCalledTimes(5);
    useAppStore.setState({ finishAngleIntent: original });
  });
});

describe("前の折り目の自然追従(SIM-018)", () => {
  it("0.1度以上の追従だけを全て一覧へ表示する", () => {
    seed(new Map());
    useAppStore.setState({
      contextHelpExpanded: false,
      hinges: new Set([1, 2, 3, 4, 5, 6, 7]),
      relaxations: [
        ...[1, 2, 3, 4, 5, 6].map((hinge) => ({
          hinge,
          target_angle_deg: 90,
          actual_angle_deg: 89.9 - hinge,
          delta_deg: -0.1 - hinge,
        })),
        { hinge: 7, target_angle_deg: 90, actual_angle_deg: 89.901, delta_deg: -0.099 },
      ],
    });
    render(<ContextPanel />);

    const list = screen.getByLabelText("前の折り目の追従");
    expect(
      screen.getByRole("button", { name: "この道具の詳しい操作方法 ▼" }).getAttribute(
        "aria-expanded",
      ),
    ).toBe("false");
    expect(within(list).getAllByRole("button")).toHaveLength(6);
    expect(within(list).getByText(/折り目 #6:/)).toBeTruthy();
    expect(within(list).queryByText(/折り目 #7:/)).toBeNull();
    expect(document.querySelectorAll(".context-panel")).toHaveLength(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("通知行のホバーとクリックで該当する折り目を示す", () => {
    seed(new Map());
    useAppStore.setState({
      hinges: new Set([5, 9]),
      relaxations: [
        { hinge: 9, target_angle_deg: 90, actual_angle_deg: 72, delta_deg: -18 },
      ],
    });
    render(<ContextPanel />);
    const row = screen.getByRole("button", {
      name: "折り目 #9: 指定90.0° → 現在72.0°",
    });

    fireEvent.mouseEnter(row);
    expect(useAppStore.getState().hoveredHinge).toBe(9);
    fireEvent.click(row);
    expect(useAppStore.getState().selection).toEqual({ edgeIds: [9], vertexIds: [] });
    fireEvent.mouseLeave(row);
    expect(useAppStore.getState().hoveredHinge).toBeNull();
  });

  it.each([6, 12, 30])(
    "D20: 追従した折り目が%i本でも、全ての位置を確かめて選べる",
    (count) => {
      seed(new Map());
      const hinges = Array.from({ length: count }, (_, index) => index + 1);
      useAppStore.setState({
        contextHelpExpanded: false,
        hinges: new Set(hinges),
        relaxations: hinges.map((hinge) => ({
          hinge,
          target_angle_deg: 90,
          actual_angle_deg: 90 - hinge,
          delta_deg: -hinge,
        })),
      });
      render(<ContextPanel />);

      const list = screen.getByLabelText("前の折り目の追従");
      const rows = within(list).getAllByRole("button");
      expect(rows).toHaveLength(count);
      for (const [index, row] of rows.entries()) {
        const hinge = index + 1;
        fireEvent.focus(row);
        expect(useAppStore.getState().hoveredHinge).toBe(hinge);
        fireEvent.click(row);
        expect(useAppStore.getState().selection).toEqual({
          edgeIds: [hinge],
          vertexIds: [],
        });
        fireEvent.blur(row);
        expect(useAppStore.getState().hoveredHinge).toBeNull();
      }
    },
  );

  it("最良候補を表示中でも角度操作を無効化しない", () => {
    seed(new Map([[5, 90]]), new Map([[5, 72]]));
    useAppStore.setState({
      poseBestEffort: true,
      relaxations: [
        { hinge: 5, target_angle_deg: 90, actual_angle_deg: 72, delta_deg: -18 },
      ],
    });
    render(<ContextPanel />);

    expect(screen.getByLabelText("折り目 #5の角度")).toHaveProperty("disabled", false);
    expect(screen.getByLabelText("折り目 #5の角度（数値）")).toHaveProperty(
      "disabled",
      false,
    );
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
    expect(within(panel).getByText("今できる操作")).toBeTruthy();
    expect(within(panel).getByText("紙と折り線を選ぶ")).toBeTruthy();
    expect(
      within(panel).getByRole("button", { name: "この道具の詳しい操作方法 ▲" }).getAttribute(
        "aria-expanded",
      ),
    ).toBe("true");
  });

  it("全体の長い操作説明だけを畳み、現在操作と設定部品は残す", () => {
    seed(new Map());
    useAppStore.setState({
      activeTool: "mountain",
      selection: { edgeIds: [], vertexIds: [] },
      contextHelpExpanded: true,
    });
    render(<ContextPanel />);

    expect(screen.getByText(/山折り・谷折り・補助線は2回クリック/)).toBeTruthy();
    fireEvent.click(
      screen.getByRole("button", { name: "この道具の詳しい操作方法 ▲" }),
    );

    expect(screen.queryByText(/山折り・谷折り・補助線は2回クリック/)).toBeNull();
    expect(screen.getByText("今できる操作")).toBeTruthy();
    expect(screen.getByText("山折り線を引く")).toBeTruthy();
    expect(screen.getByLabelText("紙のたわみを表現する")).toBeTruthy();
    expect(screen.getByRole("button", { name: "表示の広さを初期に戻す" })).toBeTruthy();
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

describe("D18: 紙のふちを選んだときの編集操作", () => {
  it.each([
    ["山折りにする", "紙のふちは紙そのものなので、山折りには変えられません"],
    ["谷折りにする", "紙のふちは紙そのものなので、谷折りには変えられません"],
    ["補助線にする", "紙のふちは紙そのものなので、補助線には変えられません"],
    ["削除", "紙のふちは紙そのものなので、削除できません"],
  ])("%sは押す前に理由が分かり、ほかの線では押せる", (label, reason) => {
    seed(new Map());
    useAppStore.setState({
      activeTool: "select",
      selection: { edgeIds: [0], vertexIds: [] },
    });
    render(<ContextPanel />);

    const borderAction = screen.getByRole("button", { name: label }) as HTMLButtonElement;
    expect(borderAction.disabled).toBe(true);
    expect(borderAction.getAttribute("data-tooltip")).toBe(reason);

    cleanup();
    seed(new Map());
    useAppStore.setState({ activeTool: "select" });
    render(<ContextPanel />);

    const creaseAction = screen.getByRole("button", { name: label }) as HTMLButtonElement;
    expect(creaseAction.disabled).toBe(false);
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

describe("対称描画の選択線基準", () => {
  it("折り線を1本選ぶと、この線を基準に設定できる", () => {
    seed(new Map());
    useAppStore.setState({ activeTool: "select" });
    render(<ContextPanel />);

    const selected = screen.getByRole("button", {
      name: "この線を基準にする",
    }) as HTMLButtonElement;
    expect(selected.disabled).toBe(false);
    expect(selected.getAttribute("data-tooltip")).toContain(
      "展開図で選んだ折り線・補助線を基準にします",
    );
    expect(selected.getAttribute("data-tooltip")).toContain(
      "現在の基準: 紙の縦の中心線",
    );
    expect(selected.hasAttribute("title")).toBe(false);

    fireEvent.click(selected);

    expect(useAppStore.getState().mirrorAxis).toEqual({
      kind: "selectedLine",
      edgeId: 5,
    });
    expect(selected.getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByText("現在: 選んだ線")).not.toBeNull();
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
      mirrorAxis: { kind: "paperHorizontal" },
    });
    render(<ContextPanel />);

    const box = screen.getByLabelText("左右対称に動かす") as HTMLInputElement;
    expect(box.checked).toBe(true); // 既定はオン(作品はほとんど左右対称なので)
    expect(box.getAttribute("data-tooltip")).toContain(
      "展開図から対になる折り目を自動で見つけ",
    );
    expect(box.getAttribute("data-tooltip")).toContain(
      "線をそろえる現在の基準: 紙の横の中心線",
    );
    expect(box.hasAttribute("title")).toBe(false);
    expect(screen.queryByText(/鶴の両羽が一緒に開きます/)).toBeNull();

    fireEvent.click(box);
    expect(useAppStore.getState().pullMirror).toBe(false);
    expect(box.getAttribute("data-tooltip")).toContain("つかんだ側");
    expect(box.getAttribute("data-tooltip")).toContain(
      "線をそろえる現在の基準: 紙の横の中心線",
    );
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

describe("技法の数値プレビュー", () => {
  it("段の幅は完全な数値だけを入力中からドラフトへ反映する", () => {
    seed(new Map());
    useAppStore.getState().beginTechnique("Pleat");
    render(<ContextPanel />);
    const width = screen.getByLabelText("段の幅(mm)");
    fireEvent.focus(width);

    fireEvent.change(width, { target: { value: "14.5" } });
    expect(useAppStore.getState().techniqueDraft?.widthMm).toBe(14.5);
    fireEvent.change(width, { target: { value: "" } });
    expect(useAppStore.getState().techniqueDraft?.widthMm).toBe(14.5);
  });
});

describe("技法の開く側と対象層", () => {
  const LINE: [[number, number], [number, number]] = [
    [0.5, 0],
    [0.5, 1],
  ];

  function seedTechnique(kind: "InsideReverse" | "OutsideReverse" | "Squash" | "Petal" | "OpenSink" | "Swivel" | "Twist") {
    seed(new Map());
    useAppStore.getState().beginTechnique(kind);
    useAppStore.getState().setTechniqueLine(LINE);
  }

  it.each(["Squash", "Petal", "Swivel", "Twist"] as const)(
    "%sでは開く側を手前/向こうから選べる",
    (kind) => {
      seedTechnique(kind);
      render(<ContextPanel />);

      const front = screen.getByLabelText("開く側: 手前") as HTMLInputElement;
      const back = screen.getByLabelText("開く側: 向こう") as HTMLInputElement;
      expect(front.checked).toBe(true);
      expect(back.checked).toBe(false);
      fireEvent.click(back);
      expect(useAppStore.getState().techniqueDraft?.openToBack).toBe(true);
    },
  );

  it("沈め折りでは使わない開く側を出さない", () => {
    seedTechnique("OpenSink");
    render(<ContextPanel />);
    expect(screen.queryByLabelText("開く側: 手前")).toBeNull();
    expect(screen.queryByLabelText("開く側: 向こう")).toBeNull();
  });

  it("Ctrlで指定した基準点を表示し、自動へ戻せる", () => {
    seedTechnique("Swivel");
    useAppStore.getState().setTechniqueReferencePoint([0.8, 0.7]);
    render(<ContextPanel />);

    expect(screen.getByText("寄せる先: 指定した点")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "基準点を自動へ戻す" }));
    expect(useAppStore.getState().techniqueDraft?.referencePoint).toBeNull();
    expect(screen.getByText("寄せる先: 自動")).toBeTruthy();
  });

  it.each([
    { kind: "InsideReverse" as const, flap: [1], disabled: true },
    { kind: "OutsideReverse" as const, flap: [1], disabled: true },
    { kind: "Squash" as const, flap: [1], disabled: false },
    { kind: "Petal" as const, flap: [1], disabled: false },
    { kind: "OpenSink" as const, flap: [], disabled: false },
    { kind: "Swivel" as const, flap: [], disabled: false },
  ])("$kindは技法固有の最小層数で適用可否を決める", ({ kind, flap, disabled }) => {
    seedTechnique(kind);
    if (flap.length > 0) useAppStore.getState().setTechniqueFlap(flap);
    render(<ContextPanel />);
    expect(screen.getByRole("button", { name: "適用" })).toHaveProperty(
      "disabled",
      disabled,
    );
  });

  it("128層を全部・手前51枚・奥55枚・手前98枚・手前から128枚目で指定できる", () => {
    seedTechnique("Squash");
    const candidates = Array.from({ length: 128 }, (_, i) => i);
    useAppStore.getState().setTechniqueFlap(candidates);
    render(<ContextPanel />);

    expect(screen.getAllByText(/候補128枚/).length).toBe(1);
    expect(useAppStore.getState().techniqueDraft?.flap).toHaveLength(128);
    const count = screen.getByLabelText("N(枚数・奥行き)");

    fireEvent.change(count, { target: { value: "51" } });
    fireEvent.click(screen.getByRole("button", { name: "手前からN枚" }));
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual(candidates.slice(77));

    fireEvent.change(count, { target: { value: "55" } });
    fireEvent.click(screen.getByRole("button", { name: "奥からN枚" }));
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual(candidates.slice(0, 55));

    fireEvent.change(count, { target: { value: "98" } });
    fireEvent.click(screen.getByRole("button", { name: "手前からN枚" }));
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual(candidates.slice(30));

    fireEvent.change(count, { target: { value: "128" } });
    fireEvent.click(screen.getByRole("button", { name: "手前からN枚目" }));
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual([0]);

    // 個別チェックでも同じ候補順を使える。
    fireEvent.click(screen.getByLabelText(/奥から1枚目 \/ 手前から128枚目/));
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: "全部" }));
    expect(useAppStore.getState().techniqueDraft?.flap).toEqual(candidates);
  });
});

describe("層操作の開閉・重ね替え", () => {
  const LINE: [[number, number], [number, number]] = [
    [0, 0],
    [1, 1],
  ];

  it("既存折り目を軸にした部分とStayの部分を、同じ1手へ追加できる", () => {
    seed(new Map());
    useAppStore.getState().beginTechnique("Simple");
    useAppStore.getState().setTechniqueFlap([1]);
    useAppStore.getState().setLayerMotionAxis(5, LINE);
    render(<ContextPanel />);

    expect(screen.getByText("軸: 折り目5")).toBeTruthy();
    const apply = screen.getByRole("button", { name: "まとめて適用" });
    expect(apply).toHaveProperty("disabled", false);
    fireEvent.click(screen.getByRole("button", { name: "この部分を追加" }));
    expect(screen.getByText(/1\. 1層を折り目で開閉/)).toBeTruthy();

    fireEvent.click(screen.getByLabelText("動かさず重ね替え"));
    fireEvent.change(screen.getByLabelText("重ね方"), {
      target: { value: "Outside" },
    });
    fireEvent.click(screen.getByLabelText("奥側"));
    fireEvent.click(
      screen.getByLabelText("選択層だけ山谷反転(層順も反転)"),
    );

    expect(useAppStore.getState().techniqueDraft).toMatchObject({
      motionMode: "stay",
      motionTurn: "Outside",
      motionDirection: "Down",
      motionReverseLayers: true,
      motionParts: [{ transform: { Reflect: [LINE] } }],
    });
    expect(screen.getByRole("button", { name: "まとめて適用" })).toHaveProperty(
      "disabled",
      false,
    );
  });

  it("動かさず位置も保つ指定は、山谷反転を選ぶまで適用できない", () => {
    seed(new Map());
    useAppStore.getState().beginTechnique("Simple");
    render(<ContextPanel />);

    fireEvent.click(screen.getByLabelText("動かさず重ね替え"));
    expect(screen.getByRole("button", { name: "まとめて適用" })).toHaveProperty(
      "disabled",
      true,
    );
    fireEvent.click(
      screen.getByLabelText("選択層だけ山谷反転(層順も反転)"),
    );
    expect(screen.getByRole("button", { name: "まとめて適用" })).toHaveProperty(
      "disabled",
      false,
    );
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
        flapCandidates: [],
        flapPickCount: 1,
        line: null,
        movingSide: "right",
        widthMm: 10,
        polygon: pts.slice(0, count),
        center,
        referencePoint: null,
        twistDeg: 30,
        openToBack: false,
        motionMode: "reflect",
        motionTurn: "Keep",
        motionDirection: "Up",
        motionAnchor: 0,
        motionReverseLayers: false,
        motionAxisEdgeId: null,
        motionParts: [],
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
    expect(screen.getAllByText(/中央の角を3つ以上クリック/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/Shift\+クリックで対象層/).length).toBeGreaterThan(0);
    const apply = screen.getByRole("button", { name: "適用" });
    expect(apply).toHaveProperty("disabled", true);
    expect(apply.getAttribute("data-tooltip")).toContain("角を3つ以上クリック");
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

  it("ねじる角は完全な数値だけを入力中からドラフトへ反映する", () => {
    seedTwist(3);
    render(<ContextPanel />);
    const deg = screen.getByLabelText("ねじる角(度)");
    fireEvent.focus(deg);

    fireEvent.change(deg, { target: { value: "42" } });
    expect(useAppStore.getState().techniqueDraft?.twistDeg).toBe(42);
    fireEvent.change(deg, { target: { value: "" } });
    expect(useAppStore.getState().techniqueDraft?.twistDeg).toBe(42);
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
  it("折るツールのときだけ8つの合わせ方を出す", () => {
    seed(new Map());
    useAppStore.setState({ activeTool: "fold" });
    render(<ContextPanel />);
    const choices = screen.getByRole("group", { name: "折り目の決め方" });
    expect(within(choices).getAllByRole("button")).toHaveLength(8);
    for (const label of Object.values(ALIGN_LABELS)) {
      const button = within(choices).getByRole("button", { name: label });
      expect(button).toBeTruthy();
      expect(button.getAttribute("data-tooltip")).toContain("展開図または3D表示");
    }

    fireEvent.click(
      within(choices).getByRole("button", {
        name: ALIGN_LABELS.throughTwoPoints,
      }),
    );
    expect(useAppStore.getState().alignDraft?.mode).toBe("throughTwoPoints");
    cleanup();

    useAppStore.setState({ activeTool: "select" });
    render(<ContextPanel />);
    expect(screen.queryByRole("group", { name: "折り目の決め方" })).toBeNull();
  });

  it("選択の途中経過を出し、そろうと既存の折り確定UI(山谷・折る)が出る", () => {
    seed(new Map());
    useAppStore.setState({ activeTool: "fold", alignDraft: null, foldDraft: null });
    useAppStore.getState().beginAlign("pointPoint");
    render(<ContextPanel />);
    expect(screen.getByText(/選択 0 \/ 2/)).toBeTruthy();
    expect(screen.getByText(/次は点を展開図または3D表示でクリック/)).toBeTruthy();
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
    expect(rulings.getAttribute("data-tooltip")).toContain("このままでは3Dで曲線折りできません");
    expect(screen.queryByText(/このままでは折れません/)).toBeNull();
  });

  it("分割数は範囲内の完全な数値だけを入力中から反映する", () => {
    seedLineTool();
    useAppStore.setState({
      curve: { ...DEFAULT_CURVE, enabled: true, segments: 16 },
    });
    render(<ContextPanel />);
    const segments = document.querySelector("#curve-segments") as HTMLInputElement;
    fireEvent.focus(segments);

    fireEvent.change(segments, { target: { value: "24" } });
    expect(useAppStore.getState().curve.segments).toBe(24);
    fireEvent.change(segments, { target: { value: "" } });
    expect(useAppStore.getState().curve.segments).toBe(24);
    fireEvent.change(segments, { target: { value: "201" } });
    expect(useAppStore.getState().curve.segments).toBe(24);
  });
});

describe("平らに畳めない点の警告欄", () => {
  it("4点を重複なくまとめ、承認済みの文を1行だけ出す", () => {
    seed(new Map());
    useAppStore.setState({ flatFoldViolations: [9, 10, 11, 12, 9] });
    render(<ContextPanel />);

    const text =
      "この折り方では平らに畳めない点が4か所あります。場所は展開図の橙色の丸で確認してください。折り目を足すか、使う折り目を減らすと畳めるようになることがあります。";
    const rows = screen.getAllByText(text);
    expect(rows).toHaveLength(1);
    expect(rows[0].classList.contains("warning-text")).toBe(true);
    expect(rows[0].textContent).not.toContain("山と谷の本数");
    expect(rows[0].textContent).not.toContain("向かい合う角の和");
  });
});

describe("D25: 作品保存の知らせ", () => {
  it("保存できたファイル名を既存の通知欄へ表示する", async () => {
    seed(new Map());
    vi.mocked(ipc.documentSave).mockResolvedValue(undefined);
    render(<ContextPanel />);

    await useAppStore.getState().saveDocument("C:\\作品\\鶴.ori3");

    expect(screen.getByText("作品を「鶴.ori3」に保存しました")).toBeTruthy();
    expect(screen.queryByText(/保存できません/)).toBeNull();
  });

  it("次の保存に失敗したら前の成功表示を消し、失敗理由だけを出す", async () => {
    seed(new Map());
    vi.mocked(ipc.documentSave)
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce("保存先へ書き込めませんでした");
    render(<ContextPanel />);

    await useAppStore.getState().saveDocument("C:\\作品\\前の作品.ori3");
    expect(screen.getByText("作品を「前の作品.ori3」に保存しました")).toBeTruthy();

    await useAppStore.getState().saveDocument("C:\\作品\\新しい作品.ori3");

    expect(screen.queryByText(/作品を「.*」に保存しました/)).toBeNull();
    expect(screen.getByText("保存先へ書き込めませんでした")).toBeTruthy();
  });
});
