// 作図補助ツールの操作(クリックで点や線を指定 → 補助線を引く)と、
// 平らに畳めない点のホバー検出のテスト。

import { describe, expect, it, vi } from "vitest";
import {
  acceptLinePoint,
  activateKeyboardCursor,
  constructDone,
  cursorFor,
  curveDraft,
  initialEphemeralState,
  KEYBOARD_CURSOR_FAST_STEP_PX,
  KEYBOARD_CURSOR_STEP_PX,
  onKeyDown,
  onKeyUp,
  onKeyboardLineKey,
  onMouseDown,
  onMouseMove,
  onMouseUp,
  onWheel,
  scrollView,
  wheelAction,
  wheelHint,
  zoomView,
  type InteractionCtx,
} from "./interaction";
import { DEFAULT_CONSTRUCT, type ConstructOptions } from "../../lib/construct";
import { DEFAULT_CURVE, type CurveOptions } from "../../lib/curve";
import { intersectDirectionRayWithSegment } from "../../lib/directionSnap";
import { paperMirrorLine } from "../../lib/mirror";
import type { Document, EdgeKind, EditOp, Vec2 } from "../../lib/types";
import { screenToWorld } from "./renderer";

/** 1辺1.0の正方形(輪郭だけ)の作品 */
function squareDoc(): Document {
  const vertices = [
    { id: 0, pos: [0, 0] as Vec2 },
    { id: 1, pos: [1, 0] as Vec2 },
    { id: 2, pos: [1, 1] as Vec2 },
    { id: 3, pos: [0, 1] as Vec2 },
  ];
  return {
    schema_version: 1,
    paper: { width_mm: 150, height_mm: 150 },
    cp: {
      vertices,
      edges: [0, 1, 2, 3].map((i) => ({
        id: i,
        v0: i,
        v1: (i + 1) % 4,
        kind: "Border" as const,
      })),
      next_vertex_id: 4,
      next_edge_id: 4,
    },
    sequence: [],
    display: { front_color: [237, 28, 36], back_color: [255, 255, 255], grid_divisions: 8 },
  };
}

/**
 * 以前はBorderだけの正方形の角を移動成功例に使い、CPE-001と逆の動作を
 * 誤って固定していた。点移動の成功例は、補助線につながる内部頂点で検査する。
 */
function movableVertexDoc(): Document {
  const doc = squareDoc();
  doc.cp.vertices.push(
    { id: 4, pos: [0.5, 0.5] },
    { id: 5, pos: [0.75, 0.5] },
  );
  doc.cp.edges.push({ id: 4, v0: 4, v1: 5, kind: "Aux" });
  doc.cp.next_vertex_id = 6;
  doc.cp.next_edge_id = 5;
  return doc;
}

const CRANE_BASE_INTERSECTION_X = 0.20710678118654752;

/** 左下の45°線と左辺が作る67.5°方向、および中央横線を持つ作品。 */
function directionGridConflictDoc(gridDivisions = 4): Document {
  const doc = squareDoc();
  doc.display.grid_divisions = gridDivisions;
  doc.cp.vertices.push(
    { id: 4, pos: [0, 0.5] },
    { id: 5, pos: [1, 0.5] },
  );
  doc.cp.edges.push(
    { id: 4, v0: 0, v1: 2, kind: "Aux" },
    { id: 5, v0: 4, v1: 5, kind: "Aux" },
  );
  doc.cp.next_vertex_id = 6;
  doc.cp.next_edge_id = 6;
  return doc;
}

/** 画面→正規化座標の丸めが12px境界をまたぐ方向吸着用の作品。 */
function directionRoundingBoundaryDoc(): Document {
  const doc = squareDoc();
  const startY = 52 / 127;
  const intersection = 26 / 127;
  doc.cp.vertices.push(
    { id: 4, pos: [0, startY] },
    { id: 5, pos: [0.1, startY - 0.1] },
    { id: 6, pos: [0.1, intersection] },
    { id: 7, pos: [0.4, intersection] },
  );
  doc.cp.edges.push(
    { id: 4, v0: 4, v1: 5, kind: "Aux" },
    { id: 5, v0: 6, v1: 7, kind: "Aux" },
  );
  doc.cp.next_vertex_id = 8;
  doc.cp.next_edge_id = 6;
  return doc;
}

/** 高倍率・中央座標で交点計算の丸めが12px境界をまたぐ作品。 */
function highScaleDirectionRoundingBoundaryDoc(): Document {
  const doc = squareDoc();
  doc.display.grid_divisions = 1024;
  doc.cp.vertices.push(
    { id: 4, pos: [0.24356623889179896, 0.5024259923957288] },
    { id: 5, pos: [0.34356623889179894, 0.5024259923957288] },
    { id: 6, pos: [0.643566238891799, 0.40242599239572885] },
    { id: 7, pos: [0.643566238891799, 0.6024259923957288] },
  );
  doc.cp.edges.push(
    { id: 4, v0: 4, v1: 5, kind: "Aux" },
    { id: 5, v0: 6, v1: 7, kind: "Aux" },
  );
  doc.cp.next_vertex_id = 8;
  doc.cp.next_edge_id = 6;
  return doc;
}

/** 正規化座標 → 画面座標(scale=500、y軸反転) */
const toScreen = (p: Vec2): Vec2 => [p[0] * 500, 500 - p[1] * 500];

/** 任意の拡大率での正規化座標 → 画面座標。 */
const toScreenAtScale = (p: Vec2, scale: number): Vec2 => [
  p[0] * scale,
  scale - p[1] * scale,
];

const polar = (deg: number, radius: number): Vec2 => {
  const rad = (deg * Math.PI) / 180;
  return [Math.cos(rad) * radius, Math.sin(rad) * radius];
};

function makeCtx(
  construct: Partial<ConstructOptions> = {},
  violations: number[] = [],
  curve: Partial<CurveOptions> = {},
) {
  const doc = squareDoc();
  const applyEdit = vi.fn<(op: EditOp | EditOp[]) => void>();
  const drawSegment = vi.fn<(a: Vec2, b: Vec2, kind: EdgeKind) => void>();
  const drawCurve = vi.fn<(points: Vec2[], kind: EdgeKind) => void>();
  const beginFoldDraft = vi.fn<(line: [Vec2, Vec2], source: "2d" | "3d") => void>();
  const pickAlignTarget = vi.fn();
  const setOperationStage = vi.fn<(stage: number) => void>();
  const ctx: InteractionCtx = {
    doc,
    view: { scale: 500, offsetX: 0, offsetY: 500 },
    tool: "construct",
    selection: { edgeIds: [], vertexIds: [] },
    alignDraft: null,
    finalDoc: doc,
    faces: [],
    frame3d: null,
    construct: { ...DEFAULT_CONSTRUCT, ...construct },
    curve: { ...DEFAULT_CURVE, ...curve },
    measureMode: "angle",
    wheelBehavior: "scroll",
    violations,
    mirrorAxis: paperMirrorLine(doc.paper, "paperVertical"),
    state: initialEphemeralState(),
    lineInputStart: null,
    setView: vi.fn(),
    setLineInputStart: vi.fn(),
    setOperationStage,
    applyEdit,
    drawSegment,
    drawCurve,
    setSelection: vi.fn(),
    beginFoldDraft,
    pickAlignTarget,
    pickMeasureEdge: vi.fn(),
    pickMeasurePoint: vi.fn(),
  };
  ctx.setLineInputStart = vi.fn((start) => {
    ctx.lineInputStart = start;
  });
  return {
    ctx,
    applyEdit,
    drawSegment,
    drawCurve,
    beginFoldDraft,
    pickAlignTarget,
    setOperationStage,
  };
}

describe("展開図で測る", () => {
  it.each([
    ["angle", [0.5, 0.004] as Vec2, 0],
    ["length", [0.996, 0.5] as Vec2, 1],
  ] as const)("%sは既存の辺選択で線を拾う", (mode, point, edgeId) => {
    const { ctx } = makeCtx();
    ctx.tool = "measure";
    ctx.measureMode = mode;

    onMouseDown(ctx, toScreen(point), 0);

    expect(ctx.pickMeasureEdge).toHaveBeenCalledWith(edgeId);
    expect(ctx.pickMeasurePoint).not.toHaveBeenCalled();
  });

  it("2点の距離は頂点、方眼の順に吸着し、線上だけには吸着しない", () => {
    const { ctx } = makeCtx();
    ctx.tool = "measure";
    ctx.measureMode = "distance";

    onMouseDown(ctx, toScreen([0.01, 0.01]), 0);
    expect(ctx.pickMeasurePoint).toHaveBeenLastCalledWith({
      cp: [0, 0],
      faceId: null,
      vertexId: 0,
    });

    onMouseDown(ctx, toScreen([0.255, 0.255]), 0);
    expect(ctx.pickMeasurePoint).toHaveBeenLastCalledWith({
      cp: [0.25, 0.25],
      faceId: null,
      vertexId: null,
    });

    // 下辺は近いが、頂点・方眼は12pxの外。線上へは寄せず押した点を保つ。
    onMouseDown(ctx, toScreen([0.31, 0.01]), 0);
    expect(ctx.pickMeasurePoint).toHaveBeenLastCalledWith({
      cp: [0.31, 0.01],
      faceId: null,
      vertexId: null,
    });
  });

  it("紙の外は測定点にせず、測定中のカーソルは十字にする", () => {
    const { ctx } = makeCtx();
    ctx.tool = "measure";
    ctx.measureMode = "distance";

    onMouseDown(ctx, toScreen([1.1, 0.5]), 0);

    expect(ctx.pickMeasurePoint).not.toHaveBeenCalled();
    expect(cursorFor("measure", ctx.state)).toBe("crosshair");
  });
});

/** applyEditは1回の入力ぶんをまとめて受け取るので、送られた編集を平らに並べ直す */
function sentEdits(applyEdit: { mock: { calls: [EditOp | EditOp[]][] } }): EditOp[] {
  return applyEdit.mock.calls.flatMap(([op]) => (Array.isArray(op) ? op : [op]));
}

describe("作図補助の操作", () => {
  it("作図の点指定は実交点より方眼を優先する", () => {
    const { ctx } = makeCtx({ kind: "bisector" });
    ctx.doc = directionGridConflictDoc();
    const cursor: Vec2 = [(CRANE_BASE_INTERSECTION_X + 0.25) / 2, 0.5];

    onMouseDown(ctx, toScreen(cursor), 0);
    expect(ctx.state.constructPoints).toEqual([[0.25, 0.5]]);
    expect(ctx.state.directionSnap).toBeNull();
  });

  it("角度線は1回のクリックで、刻みの数だけ補助線を1回の編集として引く", () => {
    const { ctx, applyEdit } = makeCtx({ kind: "angle", stepDeg: 45 });
    onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
    // 元に戻す1回で作る前へ戻せるよう、4本を1回の要求として送る(不具合D05)
    expect(applyEdit).toHaveBeenCalledTimes(1);
    const edits = sentEdits(applyEdit);
    expect(edits).toHaveLength(4);
    for (const op of edits) {
      expect(op.type).toBe("AddSegment");
      expect(op.type === "AddSegment" && op.kind).toBe("Aux");
    }
    // 引き終わったら次の作図のためにクリックの記録を空にする
    expect(constructDone(ctx.state)).toBe(0);
  });

  it("二等分線は3回クリックするまで線を引かない", () => {
    const { ctx, applyEdit } = makeCtx({ kind: "bisector" });
    onMouseDown(ctx, toScreen([0.9, 0.5]), 0);
    onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
    expect(applyEdit).not.toHaveBeenCalled();
    expect(constructDone(ctx.state)).toBe(2);
    onMouseDown(ctx, toScreen([0.5, 0.9]), 0);
    expect(applyEdit).toHaveBeenCalledTimes(1);
  });

  it("Escでクリックの記録を捨てる", () => {
    const { ctx, applyEdit } = makeCtx({ kind: "bisector" });
    onMouseDown(ctx, toScreen([0.9, 0.5]), 0);
    onKeyDown(ctx, "Escape");
    expect(constructDone(ctx.state)).toBe(0);
    expect(applyEdit).not.toHaveBeenCalled();
  });

  it("垂線は先に線をクリックする(線に当たらなければ進まない)", () => {
    const { ctx, applyEdit } = makeCtx({ kind: "perpendicular" });
    onMouseDown(ctx, toScreen([0.5, 0.5]), 0); // 紙の真ん中には線が無い
    expect(constructDone(ctx.state)).toBe(0);
    onMouseDown(ctx, toScreen([0.5, 0.0]), 0); // 下の輪郭線
    expect(constructDone(ctx.state)).toBe(1);
    onMouseDown(ctx, toScreen([0.3, 0.6]), 0);
    expect(applyEdit).toHaveBeenCalledTimes(1);
  });
});

describe("平らに畳めない点", () => {
  it("その点に近づくとホバーとして覚える", () => {
    const { ctx } = makeCtx({}, [2]);
    onMouseMove(ctx, toScreen([1, 1]));
    expect(ctx.state.hoverViolation).toBe(2);
    onMouseMove(ctx, toScreen([0.5, 0.5]));
    expect(ctx.state.hoverViolation).toBeNull();
  });
});

describe("方眼吸着の画面距離", () => {
  it("1024等分でもズーム倍率によらず画面上12px以内だけ吸着する", () => {
    for (const scale of [32768, 65536]) {
      const { ctx } = makeCtx();
      ctx.doc.display.grid_divisions = 1024;
      ctx.view = { scale, offsetX: 0, offsetY: scale };
      const centerScreen: Vec2 = [scale / 2, scale / 2];

      onMouseMove(ctx, [centerScreen[0] + 11, centerScreen[1]]);
      expect(ctx.state.hoverSnap).toEqual({ pos: [0.5, 0.5], kind: "grid" });

      onMouseMove(ctx, [centerScreen[0] + 13, centerScreen[1]]);
      expect(ctx.state.hoverSnap).toBeNull();
    }
  });
});

describe("Ctrl+クリック・矩形での複数選択", () => {
  /** 選択ツールで、修飾キーを押したまま1回クリックする。 */
  function ctrlClick(ctx: InteractionCtx, world: Vec2): void {
    const screen = toScreen(world);
    onMouseDown(ctx, screen, 0, false, true);
    onMouseUp(ctx, screen, 0, true);
  }

  it("Ctrl+クリックで辺を追加し、同じ辺をもう一度クリックすると解除する", () => {
    const { ctx } = makeCtx();
    ctx.tool = "select";
    ctx.selection = { edgeIds: [0], vertexIds: [] };

    // 右辺(id=1)の中央。頂点から離して、辺として拾わせる。
    ctrlClick(ctx, [1, 0.5]);
    expect(ctx.setSelection).toHaveBeenLastCalledWith({ edgeIds: [0, 1], vertexIds: [] });

    // テスト用ctxはモックのsetSelectionで自動更新されないため、画面の次状態を反映する。
    ctx.selection = { edgeIds: [0, 1], vertexIds: [] };
    vi.mocked(ctx.setSelection).mockClear();
    ctrlClick(ctx, [1, 0.5]);
    expect(ctx.setSelection).toHaveBeenCalledWith({ edgeIds: [0], vertexIds: [] });
  });

  it("Ctrl+空白クリックでは現在の選択を維持する", () => {
    const { ctx } = makeCtx();
    ctx.tool = "select";
    ctx.selection = { edgeIds: [0, 1], vertexIds: [3] };

    ctrlClick(ctx, [0.5, 0.5]);

    // 変更が無いのでストアへ新しい選択を送らない。
    expect(ctx.setSelection).not.toHaveBeenCalled();
    expect(ctx.selection).toEqual({ edgeIds: [0, 1], vertexIds: [3] });
  });

  it("押下後にCtrlを離しても追加選択として確定する", () => {
    const { ctx } = makeCtx();
    ctx.tool = "select";
    ctx.selection = { edgeIds: [0], vertexIds: [] };
    const screen = toScreen([1, 0.5]);

    onMouseDown(ctx, screen, 0, false, true);
    onMouseUp(ctx, screen, 0, false);

    expect(ctx.setSelection).toHaveBeenCalledWith({ edgeIds: [0, 1], vertexIds: [] });
  });

  it("Ctrl+矩形選択は範囲内の辺・点を既存選択へ足す", () => {
    const { ctx } = makeCtx();
    ctx.tool = "select";
    ctx.selection = { edgeIds: [0], vertexIds: [0] };

    // 右辺(id=1)と、その両端の点1・2だけを囲む。
    const start = toScreen([0.8, -0.1]);
    const end = toScreen([1.1, 1.1]);
    onMouseDown(ctx, start, 0, false, true);
    onMouseMove(ctx, end);
    onMouseUp(ctx, end, 0, true);

    expect(ctx.setSelection).toHaveBeenCalledWith({
      edgeIds: [0, 1],
      vertexIds: [0, 1, 2],
    });
  });
});

describe("点のドラッグ移動(選択ツール)", () => {
  /** 選択ツールで補助線につながる内部頂点4を押さえた状態を作る。 */
  function grabInternalVertex() {
    const made = makeCtx();
    made.ctx.doc = movableVertexDoc();
    made.ctx.tool = "select";
    onMouseDown(made.ctx, toScreen([0.5, 0.5]), 0);
    return made;
  }

  it("紙の四隅id=0..3は内側へドラッグしても編集しない", () => {
    const cases: Array<{ id: number; from: Vec2; to: Vec2 }> = [
      { id: 0, from: [0, 0], to: [0.25, 0.25] },
      { id: 1, from: [1, 0], to: [0.75, 0.25] },
      { id: 2, from: [1, 1], to: [0.75, 0.75] },
      { id: 3, from: [0, 1], to: [0.25, 0.75] },
    ];

    for (const { id, from, to } of cases) {
      const { ctx, applyEdit } = makeCtx();
      ctx.tool = "select";
      onMouseDown(ctx, toScreen(from), 0);
      expect(ctx.state.vertexDrag, `輪郭の角id=${id}`).toBeNull();
      onMouseMove(ctx, toScreen(to));
      onMouseUp(ctx, toScreen(to), 0);
      expect(applyEdit, `輪郭の角id=${id}`).toHaveBeenCalledTimes(0);
    }
  });

  it("輪郭辺の途中にある頂点もMoveVertexを送らない", () => {
    const { ctx, applyEdit } = makeCtx();
    ctx.tool = "select";
    ctx.doc.cp.vertices.push({ id: 4, pos: [0.5, 0] });
    ctx.doc.cp.edges = [
      { id: 0, v0: 0, v1: 4, kind: "Border" },
      { id: 4, v0: 4, v1: 1, kind: "Border" },
      ...ctx.doc.cp.edges.filter((edge) => edge.id !== 0),
    ];
    ctx.doc.cp.next_vertex_id = 5;
    ctx.doc.cp.next_edge_id = 5;

    onMouseDown(ctx, toScreen([0.5, 0]), 0);
    expect(ctx.state.vertexDrag).toBeNull();
    onMouseMove(ctx, toScreen([0.5, 0.25]));
    onMouseUp(ctx, toScreen([0.5, 0.25]), 0);

    expect(applyEdit).toHaveBeenCalledTimes(0);
  });

  it("押している間はプレビューだけで、離したときに動かす", () => {
    const { ctx, applyEdit } = grabInternalVertex();
    expect(ctx.state.vertexDrag?.id).toBe(4);
    expect(ctx.setSelection).toHaveBeenCalledWith({ edgeIds: [], vertexIds: [4] });

    const destination: Vec2 = [0.68, 0.61];
    onMouseMove(ctx, toScreen(destination));
    // 動かしている途中では編集を送らない(1回のドラッグ=1回の編集)
    expect(applyEdit).not.toHaveBeenCalled();
    expect(ctx.state.vertexDrag?.to).toEqual(destination);
    expect(ctx.state.marqueeEnd).toBeNull(); // 矩形選択にはならない

    onMouseUp(ctx, toScreen(destination), 0);
    expect(applyEdit).toHaveBeenCalledWith({
      type: "MoveVertex",
      id: 4,
      to: destination,
    });
    expect(ctx.state.vertexDrag).toBeNull();
  });

  it("点移動の正確な候補が近くになければ従来どおり方眼へ吸着する", () => {
    const { ctx, applyEdit } = makeCtx();
    ctx.doc = movableVertexDoc();
    ctx.doc.display.grid_divisions = 4;
    ctx.tool = "select";
    const cursor: Vec2 = [(CRANE_BASE_INTERSECTION_X + 0.25) / 2, 0.5];

    onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
    onMouseMove(ctx, toScreen(cursor));
    expect(ctx.state.vertexDrag?.to).toEqual([0.25, 0.5]);
    onMouseUp(ctx, toScreen(cursor), 0);
    expect(applyEdit).toHaveBeenCalledWith({ type: "MoveVertex", id: 4, to: [0.25, 0.5] });
  });

  it("点をつかんで、縦中心で折り返した正しい位置へ1e-12未満で動かせる", () => {
    const target: Vec2 = [0.20710678118654752, 0.5];
    const { ctx, applyEdit } = makeCtx();
    ctx.tool = "select";
    ctx.doc.display.grid_divisions = 4;
    ctx.doc.cp.vertices.push(
      { id: 4, pos: [0.25, 0.5] },
      { id: 5, pos: [0.7928932188134525, 0.5] },
    );
    ctx.doc.cp.next_vertex_id = 6;
    const cursor: Vec2 = [(target[0] + 0.25) / 2, target[1]];

    onMouseDown(ctx, toScreen([0.25, 0.5]), 0);
    onMouseMove(ctx, toScreen(cursor));
    const preview = ctx.state.vertexDrag?.to;
    expect(Math.abs((preview?.[0] ?? Number.NaN) - target[0])).toBeLessThan(1e-12);
    expect(Math.abs((preview?.[1] ?? Number.NaN) - target[1])).toBeLessThan(1e-12);

    onMouseUp(ctx, toScreen(cursor), 0);
    expect(applyEdit).toHaveBeenCalledTimes(1);
    const edit = sentEdits(applyEdit)[0];
    expect(edit.type).toBe("MoveVertex");
    if (edit.type !== "MoveVertex") throw new Error("MoveVertexではない編集が送られた");
    expect(edit.id).toBe(4);
    expect(Math.abs(edit.to[0] - target[0])).toBeLessThan(1e-12);
    expect(Math.abs(edit.to[1] - target[1])).toBeLessThan(1e-12);
  });

  it("クリックだけでは候補を作らず、移動開始時と表示倍率変更時に12pxへ準備する", () => {
    const { ctx } = makeCtx();
    ctx.tool = "select";
    ctx.doc.cp.vertices.push({ id: 4, pos: [0.25, 0.5] });
    ctx.doc.cp.next_vertex_id = 5;

    onMouseDown(ctx, toScreen([0.25, 0.5]), 0);
    const before = ctx.state.vertexDrag?.snap;
    expect(before).toBeNull();

    onMouseMove(ctx, toScreen([0.4, 0.5]));
    const prepared = ctx.state.vertexDrag?.snap;
    expect(prepared?.radiusNorm).toBe(12 / 500);

    ctx.view.scale = 250;
    onMouseMove(ctx, toScreenAtScale([0.4, 0.5], 250));
    expect(ctx.state.vertexDrag?.snap).not.toBe(prepared);
    expect(ctx.state.vertexDrag?.snap?.radiusNorm).toBe(12 / 250);
  });

  it("動かさずに離したときは選択のままで編集しない", () => {
    const { ctx, applyEdit } = grabInternalVertex();
    onMouseUp(ctx, toScreen([0.5, 0.5]), 0);
    expect(applyEdit).not.toHaveBeenCalled();
  });

  it("Escでやめれば元の位置のまま", () => {
    const { ctx, applyEdit } = grabInternalVertex();
    onMouseMove(ctx, toScreen([0.68, 0.61]));
    onKeyDown(ctx, "Escape");
    expect(ctx.state.vertexDrag).toBeNull();
    onMouseUp(ctx, toScreen([0.68, 0.61]), 0);
    expect(applyEdit).not.toHaveBeenCalled();
  });

  it("点のない所を押したときはこれまで通り矩形選択になる", () => {
    const { ctx } = makeCtx();
    ctx.tool = "select";
    onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
    expect(ctx.state.vertexDrag).toBeNull();
    onMouseMove(ctx, toScreen([0.8, 0.8]));
    expect(ctx.state.marqueeEnd).not.toBeNull();
  });
});

describe("線ツール", () => {
  it("2回クリックで線を引く(左右対称にするかはストアが決める)", () => {
    const { ctx, drawSegment, applyEdit } = makeCtx();
    ctx.tool = "mountain";
    onMouseDown(ctx, toScreen([0.2, 0.2]), 0);
    onMouseDown(ctx, toScreen([0.8, 0.6]), 0);
    expect(drawSegment).toHaveBeenCalledTimes(1);
    expect(drawSegment.mock.calls[0][2]).toBe("Mountain");
    // 線の追加はdrawSegment経由に一本化する(直接の編集要求は出さない)
    expect(applyEdit).not.toHaveBeenCalled();
  });

  it("紙の四隅の少し外側を押しても、角へ吸着して受け付ける", () => {
    // 吸着半径は画面12px。scale=500なので正規化座標で0.024。
    // 斜めに0.01ずつ外した点は角から0.0142で、吸着範囲の内側にある。
    const corners: Vec2[] = [
      [0, 0],
      [1, 0],
      [1, 1],
      [0, 1],
    ];
    let accepted = 0;
    for (const corner of corners) {
      const { ctx } = makeCtx();
      ctx.tool = "mountain";
      const outside: Vec2 = [
        corner[0] === 0 ? -0.01 : 1.01,
        corner[1] === 0 ? -0.01 : 1.01,
      ];

      onMouseDown(ctx, toScreen(outside), 0);

      expect(ctx.state.hoverSnap, `角${corner}`).toEqual({ pos: corner, kind: "vertex" });
      expect(ctx.lineInputStart, `角${corner}`).toEqual(corner);
      expect(ctx.state.lineInputHint, `角${corner}`).toBeNull();
      accepted += 1;
    }
    expect(accepted).toBe(4);
  });

  it("紙の輪郭の少し外側を押しても、輪郭上の点として受け付ける", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.tool = "valley";

    // 左辺の外0.01(=5px)。方眼の交点0.125/0.25からは0.0625離しておく。
    onMouseDown(ctx, toScreen([-0.01, 0.1875]), 0);
    expect(ctx.state.hoverSnap).toEqual({ pos: [0, 0.1875], kind: "edge" });
    expect(ctx.lineInputStart).toEqual([0, 0.1875]);
    expect(ctx.state.lineInputHint).toBeNull();

    onMouseDown(ctx, toScreen([1.01, 0.8125]), 0);
    expect(drawSegment).toHaveBeenCalledTimes(1);
    const [start, end] = drawSegment.mock.calls[0];
    expect(start).toEqual([0, 0.1875]);
    expect(end[0]).toBeCloseTo(1, 12);
    expect(ctx.state.lineInputHint).toBeNull();
  });

  it("輪郭上の方眼の交点も、少し外側から吸着して受け付ける", () => {
    const { ctx } = makeCtx();
    ctx.tool = "aux";
    // 8等分の方眼で、左辺上の交点(0, 0.25)。角からは0.25離れているので点吸着より方眼が効く。
    onMouseDown(ctx, toScreen([-0.01, 0.25]), 0);

    expect(ctx.state.hoverSnap).toEqual({ pos: [0, 0.25], kind: "grid" });
    expect(ctx.lineInputStart).toEqual([0, 0.25]);
    expect(ctx.state.lineInputHint).toBeNull();
  });

  it("山・谷・補助の各ツールで紙外の2点を押しても線を引かない", () => {
    for (const tool of ["mountain", "valley", "aux"] as const) {
      const { ctx, drawSegment } = makeCtx();
      ctx.tool = tool;

      // 吸着半径(12px=0.024)の外なので、輪郭にも方眼にも吸い付かない。
      onMouseDown(ctx, toScreen([-0.05, 0.2]), 0);
      onMouseDown(ctx, toScreen([1.05, 0.8]), 0);

      expect(drawSegment, tool).toHaveBeenCalledTimes(0);
      expect(ctx.lineInputStart, tool).toBeNull();
      expect(ctx.state.lineInputHint, tool).toBe(
        "紙の外には線を引けません。紙の上をクリックしてください",
      );
      onKeyDown(ctx, "Escape");
      expect(ctx.state.lineInputHint, `${tool}: Esc`).toBeNull();
    }
  });

  it("紙上の1点目の後に紙外を押すと始点と案内を保ち、次の紙上で1本だけ引く", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.tool = "mountain";
    const start: Vec2 = [0.25, 0.25];
    const end: Vec2 = [0.75, 0.75];

    onMouseDown(ctx, toScreen(start), 0);
    onMouseDown(ctx, toScreen([1.05, 0.5]), 0);

    expect(drawSegment).toHaveBeenCalledTimes(0);
    expect(ctx.lineInputStart).toEqual(start);
    expect(ctx.state.lineInputHint).toBe(
      "紙の外には線を引けません。紙の上をクリックしてください",
    );

    onMouseDown(ctx, toScreen(end), 0);
    expect(drawSegment).toHaveBeenCalledTimes(1);
    expect(drawSegment).toHaveBeenCalledWith(start, end, "Mountain");
    expect(ctx.lineInputStart).toBeNull();
    expect(ctx.state.lineInputHint).toBeNull();
  });

  it("紙外の1点目は捨て、次の紙上の点を1点目として保つ", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.tool = "valley";
    const start: Vec2 = [0.25, 0.25];

    onMouseDown(ctx, toScreen([-0.05, 0.5]), 0);
    onMouseDown(ctx, toScreen(start), 0);

    expect(drawSegment).toHaveBeenCalledTimes(0);
    expect(ctx.lineInputStart).toEqual(start);
    expect(ctx.state.lineInputHint).toBeNull();
  });

  it("輪郭上の2点は有効で、丸め誤差だけを輪郭へ戻して1本引く", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.tool = "aux";

    onMouseDown(ctx, toScreen([-5e-13, 0.25]), 0);
    onMouseDown(ctx, toScreen([1 + 5e-13, 0.75]), 0);

    expect(drawSegment).toHaveBeenCalledTimes(1);
    expect(drawSegment).toHaveBeenCalledWith([0, 0.25], [1, 0.75], "Aux");
    expect(ctx.lineInputStart).toBeNull();
  });

  it("「折る」の点指定は実交点より方眼を優先する", () => {
    const { ctx, beginFoldDraft } = makeCtx();
    ctx.doc = directionGridConflictDoc();
    ctx.tool = "fold";
    const cursor: Vec2 = [(CRANE_BASE_INTERSECTION_X + 0.25) / 2, 0.5];

    onMouseDown(ctx, toScreen([0, 0]), 0);
    onMouseDown(ctx, toScreen(cursor), 0);
    expect(beginFoldDraft).toHaveBeenCalledWith([[0, 0], [0.25, 0.5]], "2d");
    expect(ctx.state.directionSnap).toBeNull();
  });

  it("点吸着候補が無ければ角の二等分方向へ向きだけを吸着する", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.tool = "mountain";
    const cursor = polar(48, 0.57);

    onMouseDown(ctx, toScreen([0, 0]), 0);
    onMouseMove(ctx, toScreen(cursor));
    expect(ctx.state.hoverSnap).toBeNull();
    expect(ctx.state.directionSnap?.kind).toBe("bisector");
    expect(ctx.state.directionSnap?.pos[0]).toBeCloseTo(0.57 * Math.SQRT1_2, 10);
    expect(ctx.state.directionSnap?.pos[1]).toBeCloseTo(0.57 * Math.SQRT1_2, 10);

    // 確定時にもプレビューと同じ計算を使い、長さはカーソルまでの距離を保つ。
    onMouseDown(ctx, toScreen(cursor), 0);
    const [, end] = drawSegment.mock.calls[0];
    expect(end[0]).toBeCloseTo(0.57 * Math.SQRT1_2, 10);
    expect(end[1]).toBeCloseTo(0.57 * Math.SQRT1_2, 10);
  });

  it("方向を保ったまま既存頂点の軸への投影点へ吸着する", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.tool = "valley";
    ctx.doc.cp.vertices.push({ id: 4, pos: [0.4, 0.43] });
    ctx.doc.cp.next_vertex_id = 5;

    onMouseDown(ctx, toScreen([0, 0]), 0);
    onMouseMove(ctx, toScreen([0.4, 0.43]));
    expect(ctx.state.hoverSnap?.kind).toBe("vertex");
    expect(ctx.state.hoverSnap?.pos[0]).toBeCloseTo(0.415, 12);
    expect(ctx.state.hoverSnap?.pos[1]).toBeCloseTo(0.415, 12);
    expect(ctx.state.directionSnap?.kind).toBe("bisector");
    onMouseDown(ctx, toScreen([0.4, 0.43]), 0);
    expect(drawSegment.mock.calls[0][1][0]).toBeCloseTo(0.415, 12);
    expect(drawSegment.mock.calls[0][1][1]).toBeCloseTo(0.415, 12);
  });

  it("方向を保ったまま既存線分との交点へ吸着する", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.tool = "mountain";
    ctx.doc.cp.vertices.push(
      { id: 4, pos: [0.6, 0.2] },
      { id: 5, pos: [0.6, 0.8] },
    );
    ctx.doc.cp.edges.push({ id: 4, v0: 4, v1: 5, kind: "Valley" });

    onMouseDown(ctx, toScreen([0, 0]), 0);
    // 線分上ではあるが交点からは吸着半径(0.024)より離れた位置。
    onMouseMove(ctx, toScreen([0.6, 0.64]));
    expect(ctx.state.hoverSnap?.kind).toBe("edge");
    expect(ctx.state.hoverSnap?.pos[0]).toBeCloseTo(0.6, 12);
    expect(ctx.state.hoverSnap?.pos[1]).toBeCloseTo(0.6, 12);
    expect(ctx.state.directionSnap?.kind).toBe("bisector");
    expect(ctx.state.directionSnap?.pos[0]).toBeCloseTo(0.6, 12);
    expect(ctx.state.directionSnap?.pos[1]).toBeCloseTo(0.6, 12);

    onMouseDown(ctx, toScreen([0.6, 0.64]), 0);
    expect(drawSegment.mock.calls[0][1][0]).toBeCloseTo(0.6, 12);
    expect(drawSegment.mock.calls[0][1][1]).toBeCloseTo(0.6, 12);
  });

  it("67.5°方向で中央横線の鶴の基本形に必要な位置へ確定する", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.doc = directionGridConflictDoc();
    ctx.tool = "mountain";
    const cursor: Vec2 = [(CRANE_BASE_INTERSECTION_X + 0.25) / 2, 0.5];

    onMouseDown(ctx, toScreen([0, 0]), 0);
    onMouseMove(ctx, toScreen(cursor));
    expect(ctx.state.directionSnap?.kind).toBe("bisector");
    expect(ctx.state.hoverSnap?.kind).toBe("edge");
    expect(
      Math.abs((ctx.state.hoverSnap?.pos[0] ?? Number.NaN) - CRANE_BASE_INTERSECTION_X),
    ).toBeLessThan(1e-12);
    expect(Math.abs((ctx.state.hoverSnap?.pos[1] ?? Number.NaN) - 0.5)).toBeLessThan(1e-12);

    onMouseDown(ctx, toScreen(cursor), 0);
    const [, end] = drawSegment.mock.calls[0];
    expect(Math.abs(end[0] - CRANE_BASE_INTERSECTION_X)).toBeLessThan(1e-12);
    expect(Math.abs(end[1] - 0.5)).toBeLessThan(1e-12);
  });

  it("拡大率が変わっても実交点までの画面距離12pxを基準にする", () => {
    for (const scale of [256, 1024]) {
      const { ctx } = makeCtx();
      ctx.doc = directionGridConflictDoc(32);
      ctx.tool = "aux";
      ctx.view = { scale, offsetX: 0, offsetY: scale };

      onMouseDown(ctx, toScreenAtScale([0, 0], scale), 0);
      onMouseMove(
        ctx,
        toScreenAtScale([CRANE_BASE_INTERSECTION_X + 11 / scale, 0.5], scale),
      );
      expect(ctx.state.hoverSnap?.kind, `${scale}倍表示・11px`).toBe("edge");
      expect(
        Math.abs((ctx.state.hoverSnap?.pos[0] ?? Number.NaN) - CRANE_BASE_INTERSECTION_X),
      ).toBeLessThan(1e-12);

      onMouseMove(
        ctx,
        toScreenAtScale([CRANE_BASE_INTERSECTION_X + 13 / scale, 0.5], scale),
      );
      expect(ctx.state.hoverSnap?.kind, `${scale}倍表示・13px`).toBe("grid");
    }
  });

  it("画面では12px以内の実交点を正規化座標の丸めで落とさない", () => {
    const scale = 128;
    const start: Vec2 = [0, 52 / 127];
    const intersection: Vec2 = [26 / 127, 26 / 127];
    const cursorScreen: Vec2 = [34.690005783687255, 110.28055696478988];
    const intersectionScreen = toScreenAtScale(intersection, scale);
    expect(
      Math.hypot(
        cursorScreen[0] - intersectionScreen[0],
        cursorScreen[1] - intersectionScreen[1],
      ),
    ).toBeLessThan(12);

    const { ctx } = makeCtx();
    ctx.doc = directionRoundingBoundaryDoc();
    ctx.tool = "aux";
    ctx.view = { scale, offsetX: 0, offsetY: scale };
    const cursorWorld = screenToWorld(ctx.view, cursorScreen);
    const calculatedIntersection = intersectDirectionRayWithSegment(
      start,
      [1, -1],
      [0.1, 26 / 127],
      [0.4, 26 / 127],
    );
    expect(calculatedIntersection).not.toBeNull();
    expect(
      Math.hypot(
        cursorWorld[0] - (calculatedIntersection?.[0] ?? Number.NaN),
        cursorWorld[1] - (calculatedIntersection?.[1] ?? Number.NaN),
      ),
    ).toBeGreaterThan(12 / scale);

    onMouseDown(ctx, toScreenAtScale(start, scale), 0);
    onMouseMove(ctx, cursorScreen);

    expect(ctx.state.directionSnap?.kind).toBe("extension");
    expect(ctx.state.hoverSnap?.kind).toBe("edge");
    expect(ctx.state.hoverSnap?.pos[0]).toBeCloseTo(intersection[0], 12);
    expect(ctx.state.hoverSnap?.pos[1]).toBeCloseTo(intersection[1], 12);
  });

  it("高倍率・中央座標でも画面12px以内の実交点を丸めで落とさない", () => {
    const scale = 10000;
    const start: Vec2 = [0.24356623889179896, 0.5024259923957288];
    const intersection: Vec2 = [0.643566238891799, 0.5024259923957288];
    const cursorScreen: Vec2 = [6446.937264917439, 4971.631892600235];
    const intersectionScreen = toScreenAtScale(intersection, scale);
    expect(
      Math.hypot(
        cursorScreen[0] - intersectionScreen[0],
        cursorScreen[1] - intersectionScreen[1],
      ),
    ).toBeLessThan(12);

    const { ctx } = makeCtx();
    ctx.doc = highScaleDirectionRoundingBoundaryDoc();
    ctx.tool = "aux";
    ctx.view = { scale, offsetX: 0, offsetY: scale };
    const cursorWorld = screenToWorld(ctx.view, cursorScreen);
    expect(Math.hypot(cursorWorld[0] - intersection[0], cursorWorld[1] - intersection[1])).toBeGreaterThan(
      12 / scale,
    );

    onMouseDown(ctx, toScreenAtScale(start, scale), 0);
    onMouseMove(ctx, cursorScreen);

    expect(ctx.state.directionSnap?.kind).toBe("extension");
    expect(ctx.state.hoverSnap?.kind).toBe("edge");
    expect(ctx.state.hoverSnap?.pos[0]).toBeCloseTo(intersection[0], 12);
    expect(ctx.state.hoverSnap?.pos[1]).toBeCloseTo(intersection[1], 12);
  });

  it("方向の許容角度外では従来どおり既存頂点を優先する", () => {
    const { ctx } = makeCtx();
    ctx.tool = "valley";
    ctx.doc.cp.vertices.push({ id: 4, pos: [0.4, 0.49] });

    onMouseDown(ctx, toScreen([0, 0]), 0);
    onMouseMove(ctx, toScreen([0.4, 0.49]));
    expect(ctx.state.hoverSnap).toEqual({ pos: [0.4, 0.49], kind: "vertex" });
    expect(ctx.state.directionSnap).toBeNull();
  });

  it("Shift中は方向吸着を解除するが、従来の点吸着は残す", () => {
    const free = makeCtx();
    free.ctx.tool = "aux";
    const cursor = polar(48, 0.57);
    onMouseDown(free.ctx, toScreen([0, 0]), 0);
    onKeyDown(free.ctx, "Shift");
    onMouseMove(free.ctx, toScreen(cursor));
    expect(free.ctx.state.directionSnap).toBeNull();
    onMouseDown(free.ctx, toScreen(cursor), 0);
    expect(free.drawSegment.mock.calls[0][1][0]).toBeCloseTo(cursor[0], 10);
    expect(free.drawSegment.mock.calls[0][1][1]).toBeCloseTo(cursor[1], 10);

    const point = makeCtx();
    point.ctx.tool = "aux";
    point.ctx.doc.cp.vertices.push({ id: 4, pos: [0.4, 0.43] });
    onMouseDown(point.ctx, toScreen([0, 0]), 0);
    onKeyDown(point.ctx, "Shift");
    onMouseMove(point.ctx, toScreen([0.4, 0.43]));
    expect(point.ctx.state.hoverSnap?.kind).toBe("vertex");
  });

  it("方向吸着中にShiftを押すと、動かさなくても従来の点吸着へ戻る", () => {
    const { ctx } = makeCtx();
    ctx.tool = "aux";
    ctx.doc.cp.vertices.push({ id: 4, pos: [0.4, 0.43] });

    onMouseDown(ctx, toScreen([0, 0]), 0);
    onMouseMove(ctx, toScreen([0.4, 0.43]));
    expect(ctx.state.directionSnap?.kind).toBe("bisector");
    expect(ctx.state.hoverSnap?.pos[0]).toBeCloseTo(0.415, 12);

    onKeyDown(ctx, "Shift");
    expect(ctx.state.directionSnap).toBeNull();
    expect(ctx.state.hoverSnap).toEqual({ pos: [0.4, 0.43], kind: "vertex" });

    onKeyUp(ctx, "Shift");
    expect(ctx.state.directionSnap?.kind).toBe("bisector");
    expect(ctx.state.hoverSnap?.pos[0]).toBeCloseTo(0.415, 12);
  });

  it("方向を解除した通常直線は実交点より方眼を優先する", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.doc = directionGridConflictDoc();
    ctx.tool = "valley";
    const cursor: Vec2 = [(CRANE_BASE_INTERSECTION_X + 0.25) / 2, 0.5];

    onMouseDown(ctx, toScreen([0, 0]), 0);
    onKeyDown(ctx, "Shift");
    onMouseDown(ctx, toScreen(cursor), 0);
    expect(drawSegment.mock.calls[0][1]).toEqual([0.25, 0.5]);
  });
});

describe("線ツールのキーボード操作", () => {
  it.each([
    ["mountain", "Mountain"],
    ["valley", "Valley"],
    ["aux", "Aux"],
  ] as const)(
    "%sはポインタ・キーボード・共通入口で同じ一点とdrawSegmentを使う",
    (tool, kind) => {
      const start: Vec2 = [0.25, 0.25];
      const end: Vec2 = [0.75, 0.625];

      const pointer = makeCtx();
      pointer.ctx.tool = tool;
      onMouseDown(pointer.ctx, toScreen(start), 0);
      onMouseDown(pointer.ctx, toScreen(end), 0);

      const keyboard = makeCtx();
      keyboard.ctx.tool = tool;
      activateKeyboardCursor(keyboard.ctx);
      keyboard.ctx.state.keyboardCursorWorld = start;
      expect(onKeyboardLineKey(keyboard.ctx, "Enter", false)).toBe(true);
      keyboard.ctx.state.keyboardCursorWorld = end;
      expect(onKeyboardLineKey(keyboard.ctx, "Enter", false)).toBe(true);

      const direct = makeCtx();
      direct.ctx.tool = tool;
      expect(acceptLinePoint(direct.ctx, start)).toBe("started");
      expect(acceptLinePoint(direct.ctx, end)).toBe("completed");

      expect(pointer.drawSegment.mock.calls).toEqual([[[0.25, 0.25], [0.75, 0.625], kind]]);
      expect(keyboard.drawSegment.mock.calls).toEqual(pointer.drawSegment.mock.calls);
      expect(direct.drawSegment.mock.calls).toEqual(pointer.drawSegment.mock.calls);
      expect(pointer.applyEdit).not.toHaveBeenCalled();
      expect(keyboard.applyEdit).not.toHaveBeenCalled();
      expect(direct.applyEdit).not.toHaveBeenCalled();
    },
  );

  it("キーボードの始点とポインタの終点、その逆のどちらでも同じ1本になる", () => {
    const start: Vec2 = [0.25, 0.25];
    const end: Vec2 = [0.75, 0.625];

    const keyboardFirst = makeCtx();
    keyboardFirst.ctx.tool = "mountain";
    activateKeyboardCursor(keyboardFirst.ctx);
    keyboardFirst.ctx.state.keyboardCursorWorld = start;
    onKeyboardLineKey(keyboardFirst.ctx, "Enter", false);
    onMouseDown(keyboardFirst.ctx, toScreen(end), 0);

    const pointerFirst = makeCtx();
    pointerFirst.ctx.tool = "mountain";
    onMouseDown(pointerFirst.ctx, toScreen(start), 0);
    activateKeyboardCursor(pointerFirst.ctx);
    pointerFirst.ctx.state.keyboardCursorWorld = end;
    onKeyboardLineKey(pointerFirst.ctx, "Enter", false);

    expect(keyboardFirst.drawSegment.mock.calls).toEqual([
      [[0.25, 0.25], [0.75, 0.625], "Mountain"],
    ]);
    expect(pointerFirst.drawSegment.mock.calls).toEqual(keyboardFirst.drawSegment.mock.calls);
    expect(keyboardFirst.ctx.lineInputStart).toBeNull();
    expect(pointerFirst.ctx.lineInputStart).toBeNull();
  });

  it("有効な始点の後にEscapeを押すと線・作品・始点を変えず最初へ戻る", () => {
    const { ctx, drawSegment, setOperationStage } = makeCtx();
    ctx.tool = "valley";
    const before = structuredClone(ctx.doc);
    activateKeyboardCursor(ctx);
    ctx.state.keyboardCursorWorld = [0.25, 0.25];

    onKeyboardLineKey(ctx, "Enter", false);
    expect(ctx.lineInputStart).toEqual([0.25, 0.25]);
    expect(setOperationStage).toHaveBeenLastCalledWith(1);

    onKeyDown(ctx, "Escape");

    expect(drawSegment).not.toHaveBeenCalled();
    expect(ctx.doc).toEqual(before);
    expect(ctx.lineInputStart).toBeNull();
    expect(ctx.state.directionSnap).toBeNull();
    expect(setOperationStage).toHaveBeenLastCalledWith(0);
  });

  it("矢印は画面16px、Shift付きは160pxを4方向へ動かし、紙端で止まる", () => {
    const { ctx } = makeCtx();
    ctx.tool = "mountain";
    expect(KEYBOARD_CURSOR_STEP_PX).toBe(16);
    expect(KEYBOARD_CURSOR_FAST_STEP_PX).toBe(160);

    const cases = [
      ["ArrowLeft", false, [0.468, 0.5]],
      ["ArrowRight", false, [0.532, 0.5]],
      ["ArrowUp", false, [0.5, 0.532]],
      ["ArrowDown", false, [0.5, 0.468]],
      ["ArrowLeft", true, [0.18, 0.5]],
      ["ArrowRight", true, [0.82, 0.5]],
      ["ArrowUp", true, [0.5, 0.82]],
      ["ArrowDown", true, [0.5, 0.18]],
    ] as const;
    for (const [key, shiftHeld, expected] of cases) {
      activateKeyboardCursor(ctx);
      expect(onKeyboardLineKey(ctx, key, shiftHeld)).toBe(true);
      expect(ctx.state.keyboardCursorWorld?.[0]).toBeCloseTo(expected[0], 12);
      expect(ctx.state.keyboardCursorWorld?.[1]).toBeCloseTo(expected[1], 12);
    }

    ctx.state.keyboardCursorWorld = [0.99, 0.01];
    onKeyboardLineKey(ctx, "ArrowRight", false);
    onKeyboardLineKey(ctx, "ArrowDown", false);
    expect(ctx.state.keyboardCursorWorld).toEqual([1, 0]);

    ctx.view = { ...ctx.view, scale: 1000 };
    activateKeyboardCursor(ctx);
    onKeyboardLineKey(ctx, "ArrowRight", false);
    const moved = ctx.state.keyboardCursorWorld?.[0] ?? Number.NaN;
    expect((moved - 0.5) * ctx.view.scale).toBeCloseTo(16, 12);
  });

  it("吸着前の現在点を保ったまま頂点・方眼・線へ吸着し、元の候補から抜けられる", () => {
    const snappingCases: { raw: Vec2; pos: Vec2; kind: "vertex" | "grid" | "edge" }[] = [
      { raw: [0.01, 0.01], pos: [0, 0], kind: "vertex" },
      { raw: [0.251, 0.251], pos: [0.25, 0.25], kind: "grid" },
      { raw: [0.31, 0.01], pos: [0.31, 0], kind: "edge" },
    ];
    for (const expected of snappingCases) {
      const { ctx } = makeCtx();
      ctx.tool = "aux";
      activateKeyboardCursor(ctx);
      ctx.state.keyboardCursorWorld = expected.raw;
      onKeyboardLineKey(ctx, "Enter", false);
      expect(ctx.lineInputStart).toEqual(expected.pos);
      expect(ctx.state.hoverSnap).toEqual({ pos: expected.pos, kind: expected.kind });
      expect(ctx.state.keyboardCursorWorld).toEqual(expected.raw);
    }

    const { ctx } = makeCtx();
    ctx.tool = "aux";
    activateKeyboardCursor(ctx);
    ctx.state.keyboardCursorWorld = [0.47, 0.5];
    onKeyboardLineKey(ctx, "ArrowRight", false);
    expect(ctx.state.keyboardCursorWorld).toEqual([0.502, 0.5]);
    expect(ctx.state.hoverSnap).toEqual({ pos: [0.5, 0.5], kind: "grid" });
    onKeyboardLineKey(ctx, "ArrowRight", false);
    expect(ctx.state.keyboardCursorWorld).toEqual([0.534, 0.5]);
    expect(ctx.state.hoverSnap).toBeNull();
  });

  it("キーボードの終点も既存の方向吸着で角の二等分方向へ決まる", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.tool = "mountain";
    activateKeyboardCursor(ctx);
    ctx.state.keyboardCursorWorld = [0, 0];
    onKeyboardLineKey(ctx, "Enter", false);
    ctx.state.keyboardCursorWorld = polar(48, 0.57);
    onKeyboardLineKey(ctx, "Enter", false);

    expect(drawSegment).toHaveBeenCalledTimes(1);
    const [, end] = drawSegment.mock.calls[0];
    expect(end[0]).toBeCloseTo(0.57 * Math.SQRT1_2, 10);
    expect(end[1]).toBeCloseTo(0.57 * Math.SQRT1_2, 10);
  });
});

describe("合わせて折るの2D選択", () => {
  it("折った後は、面の対応で点と補助線を現在の平らな位置へ写して記録する", () => {
    const { ctx, pickAlignTarget } = makeCtx();
    const doc = squareDoc();
    doc.cp.vertices.push(
      { id: 4, pos: [0.25, 0.5] },
      { id: 5, pos: [0.75, 0.5] },
    );
    // 補助線は面の境界辺には含めず、面内の点の対応で写せることを確かめる。
    doc.cp.edges.push({ id: 4, v0: 4, v1: 5, kind: "Aux" });
    doc.cp.next_vertex_id = 6;
    doc.cp.next_edge_id = 5;
    ctx.doc = doc;
    ctx.finalDoc = doc;
    ctx.faces = [{ id: 7, vertices: [0, 1, 2, 3], edges: [0, 1, 2, 3] }];
    ctx.frame3d = {
      faces: [
        {
          face: 7,
          polygon: [
            [2, 3, 0],
            [2, 2, 0],
            [1, 2, 0],
            [1, 3, 0],
          ],
          layer: 2,
        },
      ],
      warnings: [],
    };
    ctx.tool = "fold";
    ctx.alignDraft = { mode: "pointLineThrough", picks: [] };

    onMouseDown(ctx, toScreen([0.25, 0.5]), 0);
    expect(pickAlignTarget).toHaveBeenNthCalledWith(
      1,
      { kind: "point", p: [1.5, 2.75] },
      [1.5, 2.75],
      { kind: "vertex", id: 4 },
    );

    ctx.alignDraft.picks = [{ kind: "point", p: [1.5, 2.75] }];
    onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
    expect(pickAlignTarget).toHaveBeenNthCalledWith(
      2,
      { kind: "line", a: [1.5, 2.75], b: [1.5, 2.25] },
      [1.5, 2.5],
      { kind: "edge", id: 4 },
    );
  });

  it("対象のないクリックは合わせ途中のまま保ち、通常の2点折りへ流さない", () => {
    const { ctx, beginFoldDraft, pickAlignTarget } = makeCtx();
    ctx.tool = "fold";
    ctx.alignDraft = { mode: "lineLine", picks: [] };

    onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
    onMouseDown(ctx, toScreen([0.6, 0.6]), 0);

    expect(pickAlignTarget).not.toHaveBeenCalled();
    expect(beginFoldDraft).not.toHaveBeenCalled();
    expect(ctx.lineInputStart).toBeNull();
  });

  it("現在の手順で見えない線は、全体の作品に存在していても拾わない", () => {
    const { ctx, pickAlignTarget } = makeCtx();
    const full = directionGridConflictDoc();
    ctx.doc = squareDoc();
    ctx.finalDoc = full;
    ctx.tool = "fold";
    ctx.alignDraft = { mode: "existingLine", picks: [] };

    onMouseDown(ctx, toScreen([0.5, 0.5]), 0); // fullだけにある中央横線

    expect(pickAlignTarget).not.toHaveBeenCalled();
  });

  it("現在の手順で見えない線だけに属する点も拾わない", () => {
    const { ctx, pickAlignTarget } = makeCtx();
    const full = directionGridConflictDoc();
    ctx.doc = squareDoc();
    ctx.finalDoc = full;
    ctx.tool = "fold";
    ctx.alignDraft = { mode: "pointPoint", picks: [] };

    onMouseDown(ctx, toScreen([0, 0.5]), 0); // fullの中央横線だけにある頂点4

    expect(pickAlignTarget).not.toHaveBeenCalled();
  });
});

describe("曲線の折り目を描く", () => {
  it("曲線の点指定は実交点より方眼を優先する", () => {
    const { ctx } = makeCtx({}, [], { enabled: true });
    ctx.doc = directionGridConflictDoc();
    ctx.tool = "mountain";
    const cursor: Vec2 = [(CRANE_BASE_INTERSECTION_X + 0.25) / 2, 0.5];

    onMouseDown(ctx, toScreen(cursor), 0);
    expect(ctx.state.curvePoints).toEqual([[0.25, 0.5]]);
    expect(ctx.state.directionSnap).toBeNull();
  });

  it("円弧は3回クリックするまで引かれず、そろったら折れ線として引かれる", () => {
    const { ctx, drawCurve, drawSegment } = makeCtx({}, [], { enabled: true });
    ctx.tool = "valley";
    onMouseDown(ctx, toScreen([0.1, 0.2]), 0);
    onMouseDown(ctx, toScreen([0.9, 0.2]), 0);
    expect(drawCurve).not.toHaveBeenCalled();
    expect(drawSegment).not.toHaveBeenCalled(); // 直線として引いてしまわない
    onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
    expect(drawCurve).toHaveBeenCalledTimes(1);
    const [points, kind] = drawCurve.mock.calls[0];
    expect(kind).toBe("Valley");
    expect(points.length).toBeGreaterThan(2); // 折れ線になっている
    expect(points[0]).toEqual([0.1, 0.2]);
    expect(points[points.length - 1]).toEqual([0.9, 0.2]);
    expect(ctx.state.curvePoints).toEqual([]); // 次の曲線のために空に戻る
  });

  it("ベジェは4回クリックで引かれる", () => {
    const { ctx, drawCurve } = makeCtx({}, [], { enabled: true, shape: "bezier" });
    ctx.tool = "mountain";
    for (const p of [
      [0.1, 0.1],
      [0.9, 0.1],
      [0.2, 0.8],
      [0.8, 0.8],
    ] as Vec2[]) {
      onMouseDown(ctx, toScreen(p), 0);
    }
    expect(drawCurve).toHaveBeenCalledTimes(1);
    expect(drawCurve.mock.calls[0][1]).toBe("Mountain");
  });

  it("Escで描きかけの曲線をやめられる", () => {
    const { ctx, drawCurve } = makeCtx({}, [], { enabled: true });
    ctx.tool = "valley";
    onMouseDown(ctx, toScreen([0.1, 0.2]), 0);
    onMouseDown(ctx, toScreen([0.9, 0.2]), 0);
    onKeyDown(ctx, "Escape");
    expect(ctx.state.curvePoints).toEqual([]);
    onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
    expect(drawCurve).not.toHaveBeenCalled();
  });

  it("描いている最中はカーソルの位置で形が決まる(プレビュー)", () => {
    const { ctx } = makeCtx({}, [], { enabled: true });
    ctx.tool = "valley";
    expect(curveDraft(ctx.state, ctx.curve)).toBeNull(); // まだ何もない
    onMouseDown(ctx, toScreen([0.1, 0.2]), 0);
    onMouseDown(ctx, toScreen([0.9, 0.2]), 0);
    onMouseMove(ctx, toScreen([0.5, 0.6]));
    const draft = curveDraft(ctx.state, ctx.curve);
    expect((draft ?? []).length).toBeGreaterThan(2);
    // カーソルを動かすと形も変わる
    onMouseMove(ctx, toScreen([0.5, 0.3]));
    expect(curveDraft(ctx.state, ctx.curve)).not.toEqual(draft);
  });

  it.each([
    {
      shape: "arc" as const,
      fixed: [
        [0.125, 0.25],
        [0.875, 0.25],
      ] as Vec2[],
      cursor: [0.51, 0.625] as Vec2,
      snapped: [0.5, 0.625] as Vec2,
    },
    {
      shape: "bezier" as const,
      fixed: [
        [0.125, 0.25],
        [0.875, 0.25],
        [0.25, 0.75],
      ] as Vec2[],
      cursor: [0.76, 0.75] as Vec2,
      snapped: [0.75, 0.75] as Vec2,
    },
  ])("D12 $shapeの下見座標と吸着後の確定座標の差は0", ({ shape, fixed, cursor, snapped }) => {
    const { ctx, drawCurve } = makeCtx(
      {},
      [],
      { enabled: true, shape, segments: 16 },
    );
    ctx.tool = "valley";
    for (const point of fixed) onMouseDown(ctx, toScreen(point), 0);

    onMouseMove(ctx, toScreen(cursor));
    expect(ctx.state.hoverSnap?.pos).toEqual(snapped);
    const preview = curveDraft(ctx.state, ctx.curve);
    expect(preview).not.toBeNull();

    onMouseDown(ctx, toScreen(cursor), 0);
    expect(drawCurve).toHaveBeenCalledTimes(1);
    const confirmed = drawCurve.mock.calls[0][0];
    expect(confirmed).toHaveLength(preview?.length ?? -1);
    const maxCoordinateDifference = Math.max(
      ...confirmed.flatMap((point, index) => [
        Math.abs(point[0] - (preview?.[index][0] ?? Number.NaN)),
        Math.abs(point[1] - (preview?.[index][1] ?? Number.NaN)),
      ]),
    );
    expect(maxCoordinateDifference).toBe(0);
  });

  it("曲線モードを切っていれば今までどおり2クリックの直線になる", () => {
    const { ctx, drawSegment, drawCurve } = makeCtx();
    ctx.tool = "valley";
    onMouseDown(ctx, toScreen([0.1, 0.2]), 0);
    onMouseDown(ctx, toScreen([0.9, 0.2]), 0);
    expect(drawSegment).toHaveBeenCalledTimes(1);
    expect(drawCurve).not.toHaveBeenCalled();
  });
});

describe("ホイールで表示位置と拡大率を変える", () => {
  it("既定のスクロール設定ではShiftが横、Ctrlが拡大縮小になる", () => {
    expect(wheelAction("scroll", { shiftKey: false, ctrlKey: false })).toBe("scroll-y");
    expect(wheelAction("scroll", { shiftKey: true, ctrlKey: false })).toBe("scroll-x");
    expect(wheelAction("scroll", { shiftKey: false, ctrlKey: true })).toBe("zoom");
    // ズーム中はShiftよりCtrlの役割を優先する
    expect(wheelAction("scroll", { shiftKey: true, ctrlKey: true })).toBe("zoom");
  });

  it("拡大縮小設定へ切り替えると、スクロールがCtrl+ホイールへ入れ替わる", () => {
    expect(wheelAction("zoom", { shiftKey: false, ctrlKey: false })).toBe("zoom");
    expect(wheelAction("zoom", { shiftKey: true, ctrlKey: false })).toBe("zoom");
    expect(wheelAction("zoom", { shiftKey: false, ctrlKey: true })).toBe("scroll-y");
    expect(wheelAction("zoom", { shiftKey: true, ctrlKey: true })).toBe("scroll-x");
    expect(wheelHint("zoom")).toContain("Ctrl+Shift+ホイール: 左右");
  });

  it("スクロール量をそのまま表示位置の移動へ変換する", () => {
    const view = { scale: 500, offsetX: 30, offsetY: 480 };
    expect(scrollView(view, "scroll-y", 0, 40)).toEqual({
      scale: 500,
      offsetX: 30,
      offsetY: 440,
    });
    expect(scrollView(view, "scroll-x", 0, -25)).toEqual({
      scale: 500,
      offsetX: 55,
      offsetY: 480,
    });
    // Shift時にOSが横量へ移し替える場合も同じ結果
    expect(scrollView(view, "scroll-x", 12, 0).offsetX).toBe(18);
  });

  it("拡大縮小してもカーソルが指す紙の位置は動かない", () => {
    const view = { scale: 500, offsetX: 0, offsetY: 500 };
    expect(zoomView(view, [250, 250], -100)).toEqual({
      scale: 550,
      offsetX: -25,
      offsetY: 525,
    });
  });

  it("設定に従って同じホイール入力の役割を切り替える", () => {
    const { ctx } = makeCtx();
    const gesture = {
      deltaX: 0,
      deltaY: 20,
      shiftKey: false,
      ctrlKey: false,
    };
    onWheel(ctx, [250, 250], gesture);
    expect(vi.mocked(ctx.setView)).toHaveBeenLastCalledWith({
      scale: 500,
      offsetX: 0,
      offsetY: 480,
    });

    ctx.wheelBehavior = "zoom";
    onWheel(ctx, [250, 250], gesture);
    const calls = vi.mocked(ctx.setView).mock.calls;
    expect(calls[calls.length - 1][0].scale).toBeLessThan(500);
  });
});

describe("展開図をつかんで動かす", () => {
  /** setViewに渡された最後の表示位置 */
  const lastView = (ctx: InteractionCtx) => {
    const calls = vi.mocked(ctx.setView).mock.calls;
    return calls.length > 0 ? calls[calls.length - 1][0] : null;
  };

  it("スペースを押しながらの左ドラッグで表示位置が動く", () => {
    const { ctx } = makeCtx();
    ctx.tool = "select";
    onKeyDown(ctx, " ");
    expect(cursorFor(ctx.tool, ctx.state)).toBe("grab");

    onMouseDown(ctx, [100, 100], 0);
    expect(ctx.state.marqueeStart).toBeNull(); // 選択は始まらない
    expect(cursorFor(ctx.tool, ctx.state)).toBe("grabbing");
    onMouseMove(ctx, [130, 80]);
    expect(lastView(ctx)).toEqual({ scale: 500, offsetX: 30, offsetY: 480 });

    onMouseUp(ctx, [130, 80], 0);
    onKeyUp(ctx, " ");
    expect(ctx.state.panLast).toBeNull();
    expect(cursorFor(ctx.tool, ctx.state)).toBe("default");
  });

  it("右ドラッグでも表示位置が動く(中ボタンの無い機器のため)", () => {
    const { ctx } = makeCtx();
    ctx.tool = "valley";
    onMouseDown(ctx, [100, 100], 2);
    onMouseMove(ctx, [90, 110]);
    expect(lastView(ctx)).toEqual({ scale: 500, offsetX: -10, offsetY: 510 });
    onMouseUp(ctx, [90, 110], 2);
    expect(ctx.state.panLast).toBeNull();
    expect(ctx.lineInputStart).toBeNull(); // 線引きは始まっていない
  });

  it("中ボタンドラッグは今までどおり動く", () => {
    const { ctx } = makeCtx();
    onMouseDown(ctx, [100, 100], 1);
    onMouseMove(ctx, [120, 100]);
    expect(lastView(ctx)).toEqual({ scale: 500, offsetX: 20, offsetY: 500 });
    onMouseUp(ctx, [120, 100], 1);
    expect(ctx.state.panLast).toBeNull();
  });

  it("スペースを押していない普通の左ドラッグは今までどおり選択になる", () => {
    const { ctx } = makeCtx();
    ctx.tool = "select";
    onMouseDown(ctx, toScreen([0.2, 0.2]), 0);
    onMouseMove(ctx, toScreen([0.8, 0.8]));
    expect(ctx.state.panLast).toBeNull();
    expect(ctx.state.marqueeEnd).not.toBeNull();
    onMouseUp(ctx, toScreen([0.8, 0.8]), 0);
    expect(vi.mocked(ctx.setSelection)).toHaveBeenCalled();
    expect(vi.mocked(ctx.setView)).not.toHaveBeenCalled();
  });

  it("スペースを離す・Escで、つかんでいる状態が解ける", () => {
    const { ctx } = makeCtx();
    onKeyDown(ctx, " ");
    onMouseDown(ctx, [10, 10], 0);
    onKeyDown(ctx, "Escape");
    expect(ctx.state.spaceHeld).toBe(false);
    expect(ctx.state.panLast).toBeNull();
  });
});
