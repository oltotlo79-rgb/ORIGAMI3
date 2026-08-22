import { beforeEach, describe, expect, it } from "vitest";
import type { Document } from "../lib/types";
import { useAppStore } from "./appStore";

const DOC: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [1, 0] },
      { id: 2, pos: [1, 1] },
    ],
    edges: [
      { id: 10, v0: 0, v1: 1, kind: "Border" },
      { id: 11, v0: 1, v1: 2, kind: "Border" },
    ],
    next_vertex_id: 3,
    next_edge_id: 12,
  },
  sequence: [],
  display: {
    front_color: [237, 28, 36],
    back_color: [255, 255, 255],
    grid_divisions: 8,
  },
};

beforeEach(() => {
  useAppStore.setState({
    doc: DOC,
    activeTool: "select",
    measureDraft: { mode: "angle", picks: [], display: null },
    selection: { edgeIds: [], vertexIds: [] },
  });
  useAppStore.getState().setTool("measure");
});

describe("測定の一時状態", () => {
  it("角度は辺2本を共有し、完成後の次の辺を新しい1本目にする", () => {
    const store = useAppStore.getState();
    store.pickMeasureEdge(10);
    store.pickMeasureEdge(11);

    expect(useAppStore.getState().measureDraft.picks).toEqual([
      { kind: "edge", edgeId: 10 },
      { kind: "edge", edgeId: 11 },
    ]);
    expect(useAppStore.getState().selection.edgeIds).toEqual([10, 11]);

    useAppStore.getState().pickMeasureEdge(10);
    expect(useAppStore.getState().measureDraft.picks).toEqual([
      { kind: "edge", edgeId: 10 },
    ]);
  });

  it("測り方の切り替えは途中指定・結果・表示指定を1回で消す", () => {
    const store = useAppStore.getState();
    store.pickMeasureEdge(10);
    store.setMeasureDisplay("exact");
    store.setMeasureMode("distance");

    expect(useAppStore.getState().measureDraft).toEqual({
      mode: "distance",
      picks: [],
      display: null,
    });
    expect(useAppStore.getState().selection).toEqual({ edgeIds: [], vertexIds: [] });
  });

  it("2点は面と展開図座標だけを持ち、worldや作品データを書き換えない", () => {
    const before = new TextEncoder().encode(JSON.stringify(useAppStore.getState().doc));
    const store = useAppStore.getState();
    store.setMeasureMode("distance");
    store.pickMeasurePoint({ cp: [0, 0], faceId: 4, vertexId: 0 });
    store.pickMeasurePoint({ cp: [0.25, 0.5], faceId: null, vertexId: null });

    expect(useAppStore.getState().measureDraft.picks).toEqual([
      { kind: "point", cp: [0, 0], faceId: 4, vertexId: 0 },
      { kind: "point", cp: [0.25, 0.5], faceId: null, vertexId: null },
    ]);
    expect("world" in useAppStore.getState().measureDraft.picks[0]).toBe(false);
    const after = new TextEncoder().encode(JSON.stringify(useAppStore.getState().doc));
    expect(after).toEqual(before);
  });

  it("Esc相当の解除は測り方を保ち、道具を離れると角度の初期状態へ戻す", () => {
    const store = useAppStore.getState();
    store.setMeasureMode("length");
    store.pickMeasureEdge(10);
    store.clearMeasurement();
    expect(useAppStore.getState().measureDraft).toEqual({
      mode: "length",
      picks: [],
      display: null,
    });

    useAppStore.getState().setTool("select");
    expect(useAppStore.getState().measureDraft).toEqual({
      mode: "angle",
      picks: [],
      display: null,
    });
  });
});
