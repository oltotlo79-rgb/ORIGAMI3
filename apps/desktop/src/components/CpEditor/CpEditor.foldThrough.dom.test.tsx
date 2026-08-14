// @vitest-environment jsdom
// 巻き込み用の追加折り目が、RustのCP座標のまま2D描画へ渡ることを確かめる。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type React from "react";
import { ALIGN_LABELS, type AlignMode, type AlignTarget } from "../../lib/alignFold";
import type { Document, Vec2 } from "../../lib/types";

const held = vi.hoisted(() => ({
  overlay: null as unknown,
  document: null as unknown,
  selection: null as unknown,
}));

vi.mock("./renderer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./renderer")>();
  return {
    ...actual,
    render: vi.fn((...args: unknown[]) => {
      held.document = args[4];
      held.selection = args[6];
      held.overlay = args[7];
    }),
  };
});

vi.mock("../../ipc/client", () => ({}));

import { useAppStore, type AlignCpPick, type Selection } from "../../store/appStore";
import { ContextPanel } from "../ContextPanel";
import { CpEditor } from "./CpEditor";
import type { RenderOverlay } from "./renderer";

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
    ],
    next_vertex_id: 4,
    next_edge_id: 4,
  },
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

interface AlignClick {
  screen: Vec2;
  target: AlignTarget;
  cpPick: AlignCpPick;
}

interface Align2dCase {
  mode: AlignMode;
  label: string;
  clicks: AlignClick[];
  solutionCount: number;
  highlighted: Selection;
}

const point = (id: number, p: Vec2, at: Vec2): AlignClick => ({
  screen: at,
  target: { kind: "point", p },
  cpPick: { kind: "vertex", id },
});
const line = (id: number, a: Vec2, b: Vec2, at: Vec2): AlignClick => ({
  screen: at,
  target: { kind: "line", a, b },
  cpPick: { kind: "edge", id },
});

const BL = point(0, [0, 0], [20, 380]);
const BR = point(1, [1, 0], [380, 380]);
const TR = point(2, [1, 1], [380, 20]);
const TL = point(3, [0, 1], [20, 20]);
const BOTTOM = line(0, [0, 0], [1, 0], [200, 380]);
const RIGHT = line(1, [1, 0], [1, 1], [380, 200]);
const LEFT = line(3, [0, 1], [0, 0], [20, 200]);

const ALIGN_2D_CASES: Align2dCase[] = [
  {
    mode: "throughTwoPoints",
    label: "2点を通る",
    clicks: [BL, TR],
    solutionCount: 1,
    highlighted: { edgeIds: [], vertexIds: [0, 2] },
  },
  {
    mode: "pointPoint",
    label: "点と点を合わせる",
    clicks: [BL, TR],
    solutionCount: 1,
    highlighted: { edgeIds: [], vertexIds: [0, 2] },
  },
  {
    mode: "lineLine",
    label: "線と線を合わせる",
    clicks: [BOTTOM, LEFT],
    solutionCount: 2,
    highlighted: { edgeIds: [0, 3], vertexIds: [] },
  },
  {
    mode: "pointPerpendicularLine",
    label: "点を通り線と垂直に",
    clicks: [TR, BOTTOM],
    solutionCount: 1,
    highlighted: { edgeIds: [0], vertexIds: [2] },
  },
  {
    mode: "pointLineThrough",
    label: "点を線に合わせる(折り目が通る点を指定)",
    clicks: [TL, BOTTOM, BL],
    solutionCount: 2,
    highlighted: { edgeIds: [0], vertexIds: [3, 0] },
  },
  {
    mode: "pointToLinePointToLine",
    label: "2組を同時に合わせる",
    clicks: [BL, RIGHT, BR, LEFT],
    solutionCount: 1,
    highlighted: { edgeIds: [1, 3], vertexIds: [0, 1] },
  },
  {
    mode: "pointLinePerpendicular",
    label: "点を線に合わせて別の線と垂直に",
    clicks: [BL, RIGHT, BOTTOM],
    solutionCount: 1,
    highlighted: { edgeIds: [1, 0], vertexIds: [0] },
  },
  {
    mode: "existingLine",
    label: "既存の線に沿って折る",
    clicks: [BOTTOM],
    solutionCount: 1,
    highlighted: { edgeIds: [0], vertexIds: [] },
  },
];

function pointerClick(canvas: HTMLCanvasElement, at: Vec2) {
  fireEvent.pointerDown(canvas, {
    button: 0,
    pointerId: 1,
    clientX: at[0],
    clientY: at[1],
  });
  fireEvent.pointerUp(canvas, {
    button: 0,
    pointerId: 1,
    clientX: at[0],
    clientY: at[1],
  });
}

function renderAlign(mode: AlignMode) {
  useAppStore.setState({
    activeTool: "fold",
    pendingFoldThrough: null,
    alignDraft: null,
    foldDraft: null,
    frame3d: null,
  });
  const fitRef = { current: null } as React.RefObject<(() => void) | null>;
  const view = render(
    <>
      <CpEditor fitRef={fitRef} />
      <ContextPanel />
    </>,
  );
  fireEvent.click(screen.getByRole("button", { name: ALIGN_LABELS[mode] }));
  const canvas = view.container.querySelector("canvas");
  if (!canvas) throw new Error("展開図のcanvasがない");
  return canvas;
}

beforeEach(() => {
  held.overlay = null;
  held.document = null;
  held.selection = null;
  Object.defineProperty(HTMLCanvasElement.prototype, "clientWidth", {
    configurable: true,
    value: 400,
  });
  Object.defineProperty(HTMLCanvasElement.prototype, "clientHeight", {
    configurable: true,
    value: 400,
  });
  HTMLCanvasElement.prototype.getContext = vi.fn(() => ({})) as never;
  Element.prototype.setPointerCapture = vi.fn();
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  useAppStore.setState({
    doc: DOC,
    faces: [],
    selection: { edgeIds: [], vertexIds: [] },
    activeTool: "select",
    violations: [],
    suspectHinges: [],
    relaxations: [],
    activeAngleIntent: null,
    pendingFoldThrough: {
      proposal: {
        // 3D用の畳み平面座標とは違う値にし、取り違えを見つける。
        folded_line: [
          [0.2, 0.1],
          [0.2, 0.9],
        ],
        crease_segments: [
          [
            [0.65, 0.25],
            [0.65, 0.75],
          ],
        ],
        message: "追加折り目の候補です。",
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
    foldThroughBusy: false,
    currentStep: null,
    playT: 1,
    alignDraft: null,
    foldDraft: null,
    frame3d: null,
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState({
    doc: null,
    pendingFoldThrough: null,
    suspectHinges: [],
    relaxations: [],
    activeAngleIntent: null,
    currentStep: null,
    alignDraft: null,
    foldDraft: null,
  });
});

describe("CpEditor 2D折りの下見", () => {
  it.each([
    ["OFF", false],
    ["ON", true],
  ] as const)("D14: 左右対称%sでも下見と確定は1本で一致する", async (_label, mirrorDraw) => {
    useAppStore.setState({
      activeTool: "fold",
      mirrorDraw,
      mirrorAxis: { kind: "paperVertical" },
      pendingFoldThrough: null,
      alignDraft: null,
      foldDraft: null,
      frame3d: null,
    });
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    const view = render(<CpEditor fitRef={fitRef} />);
    const canvas = view.container.querySelector("canvas");
    if (!canvas) throw new Error("展開図のcanvasがない");

    const first: Vec2 = [100, 100];
    const second: Vec2 = [100, 300];
    pointerClick(canvas, first);
    fireEvent.pointerMove(canvas, {
      pointerId: 1,
      clientX: second[0],
      clientY: second[1],
    });

    await waitFor(() => {
      const overlay = held.overlay as RenderOverlay;
      expect([overlay.preview, overlay.mirrorPreview].filter(Boolean)).toHaveLength(1);
    });
    const preview = (held.overlay as RenderOverlay).preview;
    if (!preview) throw new Error("2D折りの下見がない");
    const previewLine: [Vec2, Vec2] = [preview.a, preview.b];

    pointerClick(canvas, second);
    await waitFor(() => expect(useAppStore.getState().foldDraft?.line).toEqual(previewLine));
  });
});

describe("CpEditor 合わせて折る", () => {
  it.each(ALIGN_2D_CASES)(
    "$labelは2Dで順番どおり選ぶとalignDraftに入り下見が出る",
    async ({ mode, clicks, solutionCount, highlighted }) => {
      const canvas = renderAlign(mode);
      expect(useAppStore.getState().alignDraft?.mode).toBe(mode);

      // 方眼点でも、展開図の実在する点・線でなければ選択せず通常折りにも流さない。
      pointerClick(canvas, [200, 200]);
      expect(useAppStore.getState().alignDraft?.picks).toHaveLength(0);
      expect(useAppStore.getState().foldDraft).toBeNull();

      const expectedTargets: AlignTarget[] = [];
      const expectedCpPicks: AlignCpPick[] = [];
      for (const [index, click] of clicks.entries()) {
        pointerClick(canvas, click.screen);
        expectedTargets.push(click.target);
        expectedCpPicks.push(click.cpPick);
        const draft = useAppStore.getState().alignDraft;
        expect(draft?.picks).toEqual(expectedTargets);
        expect(draft?.cpPicks).toEqual(expectedCpPicks);
        if (index < clicks.length - 1) {
          expect(draft?.solutions).toEqual([]);
          expect(useAppStore.getState().foldDraft).toBeNull();
        }
      }

      const draft = useAppStore.getState().alignDraft;
      expect(draft?.reason).toBeNull();
      expect(draft?.solutions).toHaveLength(solutionCount);
      expect(useAppStore.getState().foldDraft?.line).toEqual(draft?.solutions[0]);
      // 製品の通常選択は変えず、描画時だけ既存の選択強調へ対応付ける。
      expect(useAppStore.getState().selection).toEqual({ edgeIds: [], vertexIds: [] });
      await waitFor(() => expect(held.selection).toEqual(highlighted));
      await waitFor(() => {
        const overlay = held.overlay as RenderOverlay;
        expect(overlay.preview).toEqual({
          a: draft?.solutions[0][0],
          b: draft?.solutions[0][1],
          kind: "Valley",
        });
      });

      act(() => useAppStore.getState().updateFoldDraft({ direction: "Down" }));
      await waitFor(() => {
        const overlay = held.overlay as RenderOverlay;
        expect(overlay.preview).toEqual({
          a: draft?.solutions[0][0],
          b: draft?.solutions[0][1],
          kind: "Mountain",
        });
      });
    },
  );

  it("1つ戻すと最後の強調だけ消え、やめると合わせ用の強調がすべて消える", async () => {
    const canvas = renderAlign("pointLineThrough");
    for (const click of [TL, BOTTOM, BL]) pointerClick(canvas, click.screen);
    await waitFor(() =>
      expect(held.selection).toEqual({ edgeIds: [0], vertexIds: [3, 0] }),
    );

    fireEvent.click(screen.getByRole("button", { name: "1つ戻す" }));
    await waitFor(() =>
      expect(held.selection).toEqual({ edgeIds: [0], vertexIds: [3] }),
    );

    fireEvent.click(screen.getByRole("button", { name: "合わせるのをやめる" }));
    await waitFor(() =>
      expect(held.selection).toEqual({ edgeIds: [], vertexIds: [] }),
    );
  });

  it("選び終えた後の次の2Dクリックは、通常折りへ落ちず1つ目から選び直す", () => {
    const canvas = renderAlign("lineLine");
    pointerClick(canvas, BOTTOM.screen);
    pointerClick(canvas, LEFT.screen);
    expect(useAppStore.getState().foldDraft).not.toBeNull();

    pointerClick(canvas, RIGHT.screen);
    expect(useAppStore.getState().alignDraft?.picks).toEqual([RIGHT.target]);
    expect(useAppStore.getState().alignDraft?.cpPicks).toEqual([RIGHT.cpPick]);
    expect(useAppStore.getState().foldDraft).toBeNull();
  });
});

describe("CpEditor 巻き込み折り目プレビュー", () => {
  it("展開図用のcrease_segmentsを描画オーバーレイへ渡す", async () => {
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    render(<CpEditor fitRef={fitRef} />);

    await waitFor(() => {
      const overlay = held.overlay as RenderOverlay | null;
      expect(overlay?.suggestedCreases).toEqual([
        [
          [0.65, 0.25],
          [0.65, 0.75],
        ],
      ]);
    });
  });
});

describe("CpEditor 食い込み候補の強調", () => {
  it("候補ヒンジを描画オーバーレイへ渡し、解消したら空にする", async () => {
    useAppStore.setState({ pendingFoldThrough: null, suspectHinges: [5] });
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    render(<CpEditor fitRef={fitRef} />);

    await waitFor(() => {
      const overlay = held.overlay as RenderOverlay | null;
      expect(overlay?.suspectHinges).toEqual([5]);
    });

    act(() => useAppStore.setState({ suspectHinges: [] }));

    await waitFor(() => {
      const overlay = held.overlay as RenderOverlay | null;
      expect(overlay?.suspectHinges).toEqual([]);
    });
  });
});

describe("CpEditor 自然追従の表示", () => {
  it("追従診断は描画へ渡さず、操作中と食い込み候補だけを強調する", async () => {
    useAppStore.setState({
      pendingFoldThrough: null,
      suspectHinges: [7],
      relaxations: [
        { hinge: 5, target_angle_deg: 90, actual_angle_deg: 72, delta_deg: -18 },
        { hinge: 6, target_angle_deg: 45, actual_angle_deg: 44.901, delta_deg: -0.099 },
      ],
      activeAngleIntent: { generation: 3, hinges: [9], fixAll: true },
    });
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    render(<CpEditor fitRef={fitRef} />);

    await waitFor(() => {
      const overlay = held.overlay as RenderOverlay | null;
      expect(overlay).not.toHaveProperty("relaxedHinges");
      expect(overlay?.activeHinges).toEqual([9]);
      expect(overlay?.suspectHinges).toEqual([7]);
    });
  });
});

describe("CpEditor 手順時点の展開図", () => {
  const HISTORY_DOC: Document = {
    ...DOC,
    cp: {
      ...DOC.cp,
      vertices: [
        ...DOC.cp.vertices,
        { id: 4, pos: [0.25, 0] },
        { id: 5, pos: [0.25, 1] },
        { id: 6, pos: [0.75, 0] },
        { id: 7, pos: [0.75, 1] },
      ],
      edges: [
        ...DOC.cp.edges,
        { id: 4, v0: 4, v1: 5, kind: "Mountain" },
        { id: 5, v0: 6, v1: 7, kind: "Valley" },
      ],
      next_vertex_id: 8,
      next_edge_id: 6,
    },
    sequence: [
      {
        id: 0,
        kind: "Simple",
        drivers: [{ a: [0.25, 0], b: [0.25, 1], target_angle_deg: 180 }],
        layer_order: null,
        note: "",
      },
      {
        id: 1,
        kind: "Simple",
        drivers: [{ a: [0.75, 0], b: [0.75, 1], target_angle_deg: -180 }],
        layer_order: null,
        note: "",
      },
    ],
  };

  it("戻る・進めるたびに、その手順までの折り線と現在番号を描く", async () => {
    useAppStore.setState({ doc: HISTORY_DOC, currentStep: 0 });
    const fitRef = { current: null } as React.RefObject<(() => void) | null>;
    const screen = render(<CpEditor fitRef={fitRef} />);

    await waitFor(() => {
      const drawn = held.document as Document;
      expect(drawn.cp.edges.map((edge) => edge.id)).toEqual([0, 1, 2, 3]);
      expect(screen.getByText("手順 0 / 2")).toBeTruthy();
    });

    act(() => useAppStore.setState({ currentStep: 1 }));
    await waitFor(() => {
      const drawn = held.document as Document;
      expect(drawn.cp.edges.map((edge) => edge.id)).toEqual([0, 1, 2, 3, 4]);
      expect(screen.getByText("手順 1 / 2")).toBeTruthy();
    });

    act(() => useAppStore.setState({ currentStep: null }));
    await waitFor(() => {
      const drawn = held.document as Document;
      expect(drawn.cp.edges.map((edge) => edge.id)).toEqual([0, 1, 2, 3, 4, 5]);
      expect(screen.getByText("手順 2 / 2")).toBeTruthy();
    });

    act(() => useAppStore.setState({ currentStep: 0 }));
    await waitFor(() => {
      const drawn = held.document as Document;
      expect(drawn.cp.edges.map((edge) => edge.id)).toEqual([0, 1, 2, 3]);
      expect(screen.getByText("手順 0 / 2")).toBeTruthy();
    });
  });
});
