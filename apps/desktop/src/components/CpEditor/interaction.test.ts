// 作図補助ツールの操作(クリックで点や線を指定 → 補助線を引く)と、
// 平らに畳めない点のホバー検出のテスト。

import { describe, expect, it, vi } from "vitest";
import {
  constructDone,
  cursorFor,
  curveDraft,
  initialEphemeralState,
  onKeyDown,
  onKeyUp,
  onMouseDown,
  onMouseMove,
  onMouseUp,
  type InteractionCtx,
} from "./interaction";
import { DEFAULT_CONSTRUCT, type ConstructOptions } from "../../lib/construct";
import { DEFAULT_CURVE, type CurveOptions } from "../../lib/curve";
import type { Document, EdgeKind, EditOp, Vec2 } from "../../lib/types";

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

/** 正規化座標 → 画面座標(scale=500、y軸反転) */
const toScreen = (p: Vec2): Vec2 => [p[0] * 500, 500 - p[1] * 500];

const polar = (deg: number, radius: number): Vec2 => {
  const rad = (deg * Math.PI) / 180;
  return [Math.cos(rad) * radius, Math.sin(rad) * radius];
};

function makeCtx(
  construct: Partial<ConstructOptions> = {},
  violations: number[] = [],
  curve: Partial<CurveOptions> = {},
) {
  const applyEdit = vi.fn<(op: EditOp) => void>();
  const drawSegment = vi.fn<(a: Vec2, b: Vec2, kind: EdgeKind) => void>();
  const drawCurve = vi.fn<(points: Vec2[], kind: EdgeKind) => void>();
  const ctx: InteractionCtx = {
    doc: squareDoc(),
    view: { scale: 500, offsetX: 0, offsetY: 500 },
    tool: "construct",
    selection: { edgeIds: [], vertexIds: [] },
    construct: { ...DEFAULT_CONSTRUCT, ...construct },
    curve: { ...DEFAULT_CURVE, ...curve },
    violations,
    state: initialEphemeralState(),
    setView: vi.fn(),
    applyEdit,
    drawSegment,
    drawCurve,
    setSelection: vi.fn(),
    beginFoldDraft: vi.fn(),
  };
  return { ctx, applyEdit, drawSegment, drawCurve };
}

describe("作図補助の操作", () => {
  it("角度線は1回のクリックで、刻みの数だけ補助線を引く", () => {
    const { ctx, applyEdit } = makeCtx({ kind: "angle", stepDeg: 45 });
    onMouseDown(ctx, toScreen([0.5, 0.5]), 0);
    expect(applyEdit).toHaveBeenCalledTimes(4);
    for (const [op] of applyEdit.mock.calls) {
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

describe("点のドラッグ移動(選択ツール)", () => {
  /** 選択ツールで頂点2(1,1)を押さえた状態を作る */
  function grabCorner() {
    const made = makeCtx();
    made.ctx.tool = "select";
    onMouseDown(made.ctx, toScreen([1, 1]), 0);
    return made;
  }

  it("押している間はプレビューだけで、離したときに動かす", () => {
    const { ctx, applyEdit } = grabCorner();
    expect(ctx.state.vertexDrag?.id).toBe(2);
    expect(ctx.setSelection).toHaveBeenCalledWith({ edgeIds: [], vertexIds: [2] });

    onMouseMove(ctx, toScreen([0.63, 0.63]));
    // 動かしている途中では編集を送らない(1回のドラッグ=1回の編集)
    expect(applyEdit).not.toHaveBeenCalled();
    expect(ctx.state.vertexDrag?.to[0]).toBeCloseTo(0.625, 3); // 8等分の目盛りに吸着
    expect(ctx.state.marqueeEnd).toBeNull(); // 矩形選択にはならない

    onMouseUp(ctx, toScreen([0.63, 0.63]), 0);
    expect(applyEdit).toHaveBeenCalledWith({
      type: "MoveVertex",
      id: 2,
      to: [0.625, 0.625],
    });
    expect(ctx.state.vertexDrag).toBeNull();
  });

  it("動かさずに離したときは選択のままで編集しない", () => {
    const { ctx, applyEdit } = grabCorner();
    onMouseUp(ctx, toScreen([1, 1]), 0);
    expect(applyEdit).not.toHaveBeenCalled();
  });

  it("Escでやめれば元の位置のまま", () => {
    const { ctx, applyEdit } = grabCorner();
    onMouseMove(ctx, toScreen([0.5, 0.5]));
    onKeyDown(ctx, "Escape");
    expect(ctx.state.vertexDrag).toBeNull();
    onMouseUp(ctx, toScreen([0.5, 0.5]), 0);
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

  it("既存頂点への点吸着を方向吸着より優先する", () => {
    const { ctx, drawSegment } = makeCtx();
    ctx.tool = "valley";
    ctx.doc.cp.vertices.push({ id: 4, pos: [0.4, 0.43] });
    ctx.doc.cp.next_vertex_id = 5;

    onMouseDown(ctx, toScreen([0, 0]), 0);
    onMouseMove(ctx, toScreen([0.4, 0.43]));
    expect(ctx.state.hoverSnap).toEqual({ pos: [0.4, 0.43], kind: "vertex" });
    expect(ctx.state.directionSnap).toBeNull();
    onMouseDown(ctx, toScreen([0.4, 0.43]), 0);
    expect(drawSegment.mock.calls[0][1]).toEqual([0.4, 0.43]);
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
});

describe("曲線の折り目を描く", () => {
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

  it("曲線モードを切っていれば今までどおり2クリックの直線になる", () => {
    const { ctx, drawSegment, drawCurve } = makeCtx();
    ctx.tool = "valley";
    onMouseDown(ctx, toScreen([0.1, 0.2]), 0);
    onMouseDown(ctx, toScreen([0.9, 0.2]), 0);
    expect(drawSegment).toHaveBeenCalledTimes(1);
    expect(drawCurve).not.toHaveBeenCalled();
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
    expect(ctx.state.pendingStart).toBeNull(); // 線引きは始まっていない
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
