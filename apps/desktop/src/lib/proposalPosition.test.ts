import { describe, expect, it } from "vitest";
import { defaultSkeleton, setTipPos } from "./skeleton";
import { clientPointToPaperPosition, paperBounds } from "./paperPosition";
import {
  PAPER_POSITION_MATCH_TOLERANCE,
  PAPER_POSITION_ONE_PIXEL_MEASURED,
  completionPositionsOnPaper,
  proposalLeafPositionStates,
  proposalRequestSkeleton,
} from "./proposalPosition";
import type { CreasePattern, Paper, Skeleton } from "./types";

const SQUARE: Paper = { width_mm: 150, height_mm: 150 };

function squareCp(): CreasePattern {
  return {
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
  };
}

function tip(request: Skeleton, id: number) {
  return request.nodes.find((node) => node.id === id)?.tip_pos_2d ?? null;
}

describe("完成形と紙の上の場所を葉ごとにまとめる", () => {
  it("560pxの大画面で1pxの入力差は0.004、許容差は実測の80%", () => {
    const bounds = paperBounds(squareCp());
    const rect = { left: 20, top: 30, width: 560, height: 560 };
    const center = clientPointToPaperPosition([300, 310], rect, bounds);
    const onePixelRight = clientPointToPaperPosition([301, 310], rect, bounds);
    const measured = Math.abs(onePixelRight.x - center.x);
    expect(Math.abs(measured - PAPER_POSITION_ONE_PIXEL_MEASURED)).toBeLessThan(
      1e-12,
    );
    expect(PAPER_POSITION_ONE_PIXEL_MEASURED).toBe(0.004);
    expect(PAPER_POSITION_MATCH_TOLERANCE).toBe(0.0032);
    expect(PAPER_POSITION_MATCH_TOLERANCE).toBeLessThan(measured);
  });

  it("実測から決めた許容差の内側は同じ場所、外側は違う場所と判定する", () => {
    let skeleton = defaultSkeleton();
    skeleton = setTipPos(skeleton, 1, { x: -0.5, y: 0 });
    skeleton = setTipPos(skeleton, 2, { x: 0.5, y: 0 });
    const automatic = completionPositionsOnPaper(skeleton, SQUARE).find(
      (entry) => entry.leaf_id === 1,
    )!.position;

    const inside = proposalLeafPositionStates(
      skeleton,
      [
        {
          leaf_id: 1,
          position: { x: automatic.x + 0.0031, y: automatic.y },
        },
      ],
      [{ leaf_id: 1, source: "completion" }],
      SQUARE,
    ).find((state) => state.leaf_id === 1)!;
    const outside = proposalLeafPositionStates(
      skeleton,
      [
        {
          leaf_id: 1,
          position: { x: automatic.x + 0.0033, y: automatic.y },
        },
      ],
      [{ leaf_id: 1, source: "completion" }],
      SQUARE,
    ).find((state) => state.leaf_id === 1)!;

    expect(inside.kind).toBe("both");
    expect(inside.used).toBe("paper");
    expect(outside.kind).toBe("different");
    expect(outside.used).toBe("completion");
  });

  it("完成だけ・紙だけ・両方・違う場所の4状態で決めた側を1つずつ選ぶ", () => {
    let skeleton = defaultSkeleton();
    skeleton = setTipPos(skeleton, 1, { x: -0.8, y: -0.6 });
    skeleton = setTipPos(skeleton, 3, { x: 0.7, y: 0.5 });
    skeleton = setTipPos(skeleton, 4, { x: 0.3, y: -0.4 });
    const automatic = new Map(
      completionPositionsOnPaper(skeleton, SQUARE).map((entry) => [
        entry.leaf_id,
        entry.position,
      ]),
    );
    const paper = [
      { leaf_id: 2, position: { x: 0.2, y: -0.3 } },
      { leaf_id: 3, position: { ...automatic.get(3)! } },
      { leaf_id: 4, position: { x: -0.7, y: 0.8 } },
    ];
    const states = proposalLeafPositionStates(
      skeleton,
      paper,
      [{ leaf_id: 4, source: "completion" }],
      SQUARE,
    );
    expect(states.map(({ leaf_id, kind, used }) => ({ leaf_id, kind, used }))).toEqual([
      { leaf_id: 1, kind: "completion-only", used: "completion" },
      { leaf_id: 2, kind: "paper-only", used: "paper" },
      { leaf_id: 3, kind: "both", used: "paper" },
      { leaf_id: 4, kind: "different", used: "completion" },
    ]);

    const request = proposalRequestSkeleton(
      skeleton,
      paper,
      [{ leaf_id: 4, source: "completion" }],
      SQUARE,
    );
    expect(tip(request, 1)).toEqual({ x: -0.8, y: -0.6 });
    expect(tip(request, 2)).toEqual({ x: 0.2, y: -0.3 });
    expect(tip(request, 3)).toEqual(automatic.get(3));
    expect(tip(request, 4)).toEqual({ x: 0.3, y: -0.4 });
  });

  it("違う場所では、完成形の後なら完成形、紙の後なら紙を使う", () => {
    let skeleton = defaultSkeleton();
    skeleton = setTipPos(skeleton, 1, { x: 0.75, y: -0.5 });
    const paper = [{ leaf_id: 1, position: { x: -0.6, y: 0.7 } }];

    const completionLast = proposalRequestSkeleton(
      skeleton,
      paper,
      [{ leaf_id: 1, source: "completion" }],
      SQUARE,
    );
    const paperLast = proposalRequestSkeleton(
      skeleton,
      paper,
      [{ leaf_id: 1, source: "paper" }],
      SQUARE,
    );
    expect(tip(completionLast, 1)).toEqual({ x: 0.75, y: -0.5 });
    expect(tip(paperLast, 1)).toEqual({ x: -0.6, y: 0.7 });
  });

  it("1つの葉は完成形、別の葉は紙の上という選び方を同じ要求へまとめる", () => {
    let skeleton = defaultSkeleton();
    skeleton = setTipPos(skeleton, 1, { x: -0.4, y: 0.2 });
    skeleton = setTipPos(skeleton, 2, { x: 0.5, y: -0.1 });
    const paper = [
      { leaf_id: 1, position: { x: 0.8, y: 0.6 } },
      { leaf_id: 2, position: { x: -0.7, y: -0.5 } },
    ];
    const request = proposalRequestSkeleton(
      skeleton,
      paper,
      [
        { leaf_id: 1, source: "completion" },
        { leaf_id: 2, source: "paper" },
      ],
      SQUARE,
    );
    expect(tip(request, 1)).toEqual({ x: -0.4, y: 0.2 });
    expect(tip(request, 2)).toEqual({ x: -0.7, y: -0.5 });
  });
});
