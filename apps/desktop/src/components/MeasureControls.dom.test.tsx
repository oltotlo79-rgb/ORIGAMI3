// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import type { Document, Face, Frame3D } from "../lib/types";
import { useAppStore, type MeasureDraft } from "../store/appStore";
import { ContextPanel } from "./ContextPanel";
import { MeasureControls } from "./MeasureControls";

const initialStoreState = useAppStore.getState();
const angleRadians = (22.5 * Math.PI) / 180;
const approximateAngleRadians = (Math.SQRT2 * Math.PI) / 180;

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
        { id: 4, pos: [Math.cos(angleRadians), Math.sin(angleRadians)] },
        { id: 5, pos: [Math.SQRT2 / 150, 0] },
        {
          id: 6,
          pos: [Math.cos(approximateAngleRadians), Math.sin(approximateAngleRadians)],
        },
      ],
      edges: [
        { id: 10, v0: 0, v1: 1, kind: "Border" },
        { id: 11, v0: 0, v1: 2, kind: "Mountain" },
        { id: 12, v0: 0, v1: 4, kind: "Aux" },
        { id: 13, v0: 0, v1: 5, kind: "Aux" },
        { id: 14, v0: 0, v1: 6, kind: "Aux" },
        { id: 20, v0: 1, v1: 2, kind: "Border" },
        { id: 21, v0: 2, v1: 3, kind: "Border" },
        { id: 22, v0: 3, v1: 0, kind: "Border" },
      ],
      next_vertex_id: 7,
      next_edge_id: 23,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

const FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2], edges: [10, 20, 11] },
  { id: 1, vertices: [0, 2, 3], edges: [11, 21, 22] },
];

const FOLDED_FRAME: Frame3D = {
  faces: [
    {
      face: 0,
      polygon: [
        [0, 0, 0],
        [1, 0, 0],
        [1, 1, 0],
      ],
      layer: 0,
    },
    {
      face: 1,
      polygon: [
        [0, 0, 0],
        [1, 1, 0],
        [0, 1, 1],
      ],
      layer: 0,
    },
  ],
  warnings: [],
};

function seed(draft: MeasureDraft, frame3d: Frame3D | null = null) {
  useAppStore.setState({
    activeTool: "measure",
    doc: makeDoc(),
    faces: FACES,
    frame3d,
    measureDraft: draft,
    selection: { edgeIds: [], vertexIds: [] },
    currentStep: null,
    pendingFoldThrough: null,
    foldDraft: null,
    alignDraft: null,
    techniqueDraft: null,
  });
}

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("測る道具の下部パネル", () => {
  it("3つの測り方を押した1回で切り替え、前の指定を消す", () => {
    seed({
      mode: "angle",
      picks: [{ kind: "edge", edgeId: 10 }],
      display: "decimal",
    });
    render(<MeasureControls />);

    const group = screen.getByRole("group", { name: "測り方" });
    expect(within(group).getAllByRole("button")).toHaveLength(3);
    expect(within(group).getByRole("button", { name: "角度" }).getAttribute("aria-pressed"))
      .toBe("true");

    fireEvent.click(within(group).getByRole("button", { name: "線の長さ" }));
    expect(useAppStore.getState().measureDraft).toEqual({
      mode: "length",
      picks: [],
      display: null,
    });
    expect(
      within(group).getByRole("button", { name: "線の長さ" }).getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("1つ目の指定と残りを示し、画面のボタンでも選び直せる", () => {
    seed({ mode: "angle", picks: [{ kind: "edge", edgeId: 10 }], display: null });
    render(<MeasureControls />);

    expect(screen.getByText("あと1つの辺を指定してください")).toBeTruthy();
    expect(screen.getByText("選択 1 / 2")).toBeTruthy();
    expect(screen.getByText("Escでも途中の指定をやめられます")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "選び直す" }));
    expect(useAppStore.getState().measureDraft.picks).toEqual([]);
    expect(screen.getByText("2つの辺を指定してください")).toBeTruthy();
  });

  it("角度は小数を既定にし、1回押すと度の分数と小数を併記する", () => {
    seed({
      mode: "angle",
      picks: [
        { kind: "edge", edgeId: 10 },
        { kind: "edge", edgeId: 12 },
      ],
      display: null,
    });
    render(<MeasureControls />);

    const card = screen.getByRole("region", { name: "測定結果" });
    expect(within(card).getByText("22.5°")).toBeTruthy();
    const fraction = screen.getByRole("button", { name: "度を分数で" });
    expect(fraction).toHaveProperty("disabled", false);
    fireEvent.click(fraction);
    expect(useAppStore.getState().measureDraft.display).toBe("exact");
    expect(within(card).getByText("45/2°")).toBeTruthy();
    expect(within(card).getByText("22.5°")).toBeTruthy();
  });

  it("線の正確な長さを根号とおよその小数で示し、1回で小数へ切り替える", () => {
    seed({
      mode: "length",
      picks: [{ kind: "edge", edgeId: 11 }],
      display: null,
    });
    render(<MeasureControls />);

    const card = screen.getByRole("region", { name: "測定結果" });
    expect(within(card).getByText("150√2 mm")).toBeTruthy();
    expect(within(card).getByText("およそ 212.1320 mm")).toBeTruthy();
    const exact = screen.getByRole("button", { name: "正確な形" });
    expect(exact.getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(screen.getByRole("button", { name: "小数" }));
    expect(useAppStore.getState().measureDraft.display).toBe("decimal");
    expect(within(card).queryByText("150√2 mm")).toBeNull();
    expect(within(card).getByText("およそ 212.1320 mm")).toBeTruthy();
  });

  it.each([
    {
      name: "角度",
      draft: {
        mode: "angle",
        picks: [
          { kind: "edge", edgeId: 10 },
          { kind: "edge", edgeId: 14 },
        ],
        display: null,
      } satisfies MeasureDraft,
      button: "度を分数で",
      reason: "この角度は度を分数で正確に表せないため、小数で表示します。",
    },
    {
      name: "線の長さ",
      draft: {
        mode: "length",
        picks: [{ kind: "edge", edgeId: 13 }],
        display: null,
      } satisfies MeasureDraft,
      button: "正確な形",
      reason: "この長さは正確な形で表せないため、小数で表示します。",
    },
    {
      name: "2点の距離",
      draft: {
        mode: "distance",
        picks: [
          { kind: "point", cp: [0, 0], faceId: null, vertexId: 0 },
          {
            kind: "point",
            cp: [Math.SQRT2 / 150, 0],
            faceId: null,
            vertexId: null,
          },
        ],
        display: null,
      } satisfies MeasureDraft,
      button: "正確な形",
      reason: "展開図での距離は正確な形で表せないため、小数で表示します。",
    },
  ])("$nameを正確に表せないときは正確な表示を押せず、理由をその場に出す", ({
    draft,
    button,
    reason,
  }) => {
    seed(draft);
    render(<MeasureControls />);

    expect(screen.getByRole("button", { name: button })).toHaveProperty("disabled", true);
    expect(screen.getByText(reason)).toBeTruthy();
    expect(screen.getByRole("button", { name: "小数" }).getAttribute("aria-pressed")).toBe(
      "true",
    );
  });

  it("2点は展開図と元の立体座標の距離を同じmmで2行表示する", () => {
    seed(
      {
        mode: "distance",
        picks: [
          { kind: "point", cp: [1, 0], faceId: 0, vertexId: 1 },
          { kind: "point", cp: [0, 1], faceId: 1, vertexId: 3 },
        ],
        display: null,
      },
      FOLDED_FRAME,
    );
    render(<MeasureControls />);

    const card = screen.getByRole("region", { name: "測定結果" });
    expect(within(card).getByText("展開図での距離")).toBeTruthy();
    expect(within(card).getByText("150√2 mm")).toBeTruthy();
    expect(within(card).getByText("3D図での距離")).toBeTruthy();
    expect(within(card).getByText("およそ 259.8076 mm")).toBeTruthy();
    expect(screen.getByText("立体での距離は、およその値で表示します。")).toBeTruthy();
    expect(card.querySelectorAll("output[aria-live='polite']")).toHaveLength(2);
  });

  it("過去の手順を表示中でも測定を選んでいれば測定欄を優先する", () => {
    seed({ mode: "angle", picks: [], display: null });
    useAppStore.setState({ currentStep: 1 });
    render(<ContextPanel />);

    expect(screen.getByRole("region", { name: "測る" })).toBeTruthy();
    expect(screen.getByText("今できる操作")).toBeTruthy();
    expect(screen.queryByText("この手順はもうありません")).toBeNull();
  });
});
