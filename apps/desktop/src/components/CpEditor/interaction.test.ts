// 作図補助ツールの操作(クリックで点や線を指定 → 補助線を引く)と、
// 平らに畳めない点のホバー検出のテスト。

import { describe, expect, it, vi } from "vitest";
import {
  constructDone,
  initialEphemeralState,
  onKeyDown,
  onMouseDown,
  onMouseMove,
  type InteractionCtx,
} from "./interaction";
import { DEFAULT_CONSTRUCT, type ConstructOptions } from "../../lib/construct";
import type { Document, EditOp, Vec2 } from "../../lib/types";

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
    setSelection: vi.fn(),
    beginFoldDraft: vi.fn(),
  };
  return { ctx, applyEdit };
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
