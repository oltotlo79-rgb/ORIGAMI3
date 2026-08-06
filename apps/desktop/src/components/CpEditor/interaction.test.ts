// 作図補助ツールの操作(クリックで点や線を指定 → 補助線を引く)と、
// 平らに畳めない点のホバー検出のテスト。

import { describe, expect, it, vi } from "vitest";
import {
  constructDone,
  initialEphemeralState,
  onKeyDown,
  onMouseDown,
  onMouseMove,
  onMouseUp,
  type InteractionCtx,
} from "./interaction";
import { DEFAULT_CONSTRUCT, type ConstructOptions } from "../../lib/construct";
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

function makeCtx(construct: Partial<ConstructOptions> = {}, violations: number[] = []) {
  const applyEdit = vi.fn<(op: EditOp) => void>();
  const drawSegment = vi.fn<(a: Vec2, b: Vec2, kind: EdgeKind) => void>();
  const ctx: InteractionCtx = {
    doc: squareDoc(),
    view: { scale: 500, offsetX: 0, offsetY: 500 },
    tool: "construct",
    selection: { edgeIds: [], vertexIds: [] },
    construct: { ...DEFAULT_CONSTRUCT, ...construct },
    violations,
    state: initialEphemeralState(),
    setView: vi.fn(),
    applyEdit,
    drawSegment,
    setSelection: vi.fn(),
    beginFoldDraft: vi.fn(),
  };
  return { ctx, applyEdit, drawSegment };
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
});
