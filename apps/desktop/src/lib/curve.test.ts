import { describe, expect, it } from "vitest";
import {
  arcPolyline,
  arcSegmentCount,
  cubicPolyline,
  curveHint,
  curvePolyline,
  DEFAULT_CURVE_TOL,
  firstCrossing,
  MAX_CURVE_SEGMENTS,
  rulingLines,
} from "./curve";
import type { Document, Vec2 } from "./types";

/** 点pから折れ線までの最短距離 */
function distToPolyline(p: Vec2, pts: Vec2[]): number {
  let best = Infinity;
  for (let i = 0; i + 1 < pts.length; i++) {
    const [a, b] = [pts[i], pts[i + 1]];
    const ab: Vec2 = [b[0] - a[0], b[1] - a[1]];
    const l2 = ab[0] * ab[0] + ab[1] * ab[1];
    const t = l2 === 0 ? 0 : Math.max(0, Math.min(1, ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2));
    best = Math.min(best, Math.hypot(p[0] - (a[0] + ab[0] * t), p[1] - (a[1] + ab[1] * t)));
  }
  return best;
}

describe("円弧の折れ線", () => {
  it("指定した誤差以内になる", () => {
    for (const tol of [0.02, 0.005, 0.001]) {
      const pts = arcPolyline([0, 0], [0.5, 0.4], [1, 0], tol);
      const fine = arcPolyline([0, 0], [0.5, 0.4], [1, 0], 1e-7);
      const worst = Math.max(...fine.map((q) => distToPolyline(q, pts)));
      expect(worst).toBeLessThanOrEqual(tol);
    }
  });

  it("細かさを上げると分割数が増え、上限で頭打ちになる", () => {
    expect(arcSegmentCount(0.5, Math.PI, 0.001)).toBeGreaterThan(
      arcSegmentCount(0.5, Math.PI, 0.02),
    );
    const n = arcSegmentCount(0.5, Math.PI, DEFAULT_CURVE_TOL);
    expect(n).toBeGreaterThanOrEqual(8);
    expect(n).toBeLessThanOrEqual(24);
    expect(arcSegmentCount(0.5, Math.PI, 1e-12)).toBe(MAX_CURVE_SEGMENTS);
  });

  it("端点は指定した座標そのもので、通過点のそばを通る", () => {
    const pts = arcPolyline([0.1, 0.2], [0.5, 0.9], [0.8, 0.3], DEFAULT_CURVE_TOL);
    expect(pts[0]).toEqual([0.1, 0.2]);
    expect(pts[pts.length - 1]).toEqual([0.8, 0.3]);
    expect(distToPolyline([0.5, 0.9], pts)).toBeLessThanOrEqual(DEFAULT_CURVE_TOL);
  });

  it("一直線の3点はただの線分になる", () => {
    expect(arcPolyline([0, 0], [0.5, 0.5], [1, 1], DEFAULT_CURVE_TOL)).toEqual([
      [0, 0],
      [1, 1],
    ]);
  });

  it("分割数を指定すればその数になる", () => {
    expect(arcPolyline([0, 0], [0.5, 0.4], [1, 0], 0.005, 5)).toHaveLength(6);
    expect(arcPolyline([0, 0], [0.5, 0.4], [1, 0], 0.005, 9999)).toHaveLength(
      MAX_CURVE_SEGMENTS + 1,
    );
  });
});

describe("ベジェの折れ線", () => {
  it("指定した誤差以内になる", () => {
    const [p0, c1, c2, p1]: Vec2[] = [
      [0, 0],
      [0, 1],
      [1, -0.5],
      [1, 0.5],
    ];
    for (const tol of [0.02, 0.005, 0.001]) {
      const pts = cubicPolyline(p0, c1, c2, p1, tol);
      const fine = cubicPolyline(p0, c1, c2, p1, 1e-7);
      const worst = Math.max(...fine.map((q) => distToPolyline(q, pts)));
      expect(worst).toBeLessThanOrEqual(tol);
    }
  });

  it("制御点が一直線上ならただの線分になる", () => {
    expect(cubicPolyline([0, 0], [0.25, 0.25], [0.75, 0.75], [1, 1], 0.005)).toEqual([
      [0, 0],
      [1, 1],
    ]);
  });
});

/** 正方形(輪郭4辺のみ)のドキュメント */
function squareDoc(extraEdges: Document["cp"]["edges"] = [], extraVertices: Document["cp"]["vertices"] = []): Document {
  return {
    schema_version: 1,
    paper: { width_mm: 100, height_mm: 100 },
    cp: {
      vertices: [
        { id: 0, pos: [0, 0] },
        { id: 1, pos: [1, 0] },
        { id: 2, pos: [1, 1] },
        { id: 3, pos: [0, 1] },
        ...extraVertices,
      ],
      edges: [
        { id: 0, v0: 0, v1: 1, kind: "Border" },
        { id: 1, v0: 1, v1: 2, kind: "Border" },
        { id: 2, v0: 2, v1: 3, kind: "Border" },
        { id: 3, v0: 3, v1: 0, kind: "Border" },
        ...extraEdges,
      ],
      next_vertex_id: 4 + extraVertices.length,
      next_edge_id: 4 + extraEdges.length,
    },
    sequence: [],
    display: { front_color: [237, 28, 36], back_color: [255, 255, 255], grid_divisions: 8 },
  };
}

describe("曲がるための線", () => {
  it("曲線に直角で、へこむ側と膨らむ側へ伸びる", () => {
    const pts = arcPolyline([0, 0.1], [0.5, 0.4], [1, 0.1], DEFAULT_CURVE_TOL);
    const rulings = rulingLines(pts, [1, 1]);
    expect(rulings).toHaveLength(pts.length - 2);
    rulings.forEach((r, i) => {
      const tan: Vec2 = [pts[i + 2][0] - pts[i][0], pts[i + 2][1] - pts[i][1]];
      const dir: Vec2 = [r.convex[0] - r.concave[0], r.convex[1] - r.concave[1]];
      const dot = (tan[0] * dir[0] + tan[1] * dir[1]) / (Math.hypot(...tan) * Math.hypot(...dir));
      expect(Math.abs(dot)).toBeLessThan(1e-9);
      // 上に膨らむ弧なので、へこむ側(円の中心側)は下
      expect(r.concave[1]).toBeLessThan(r.at[1]);
      expect(r.convex[1]).toBeGreaterThan(r.at[1]);
    });
  });

  it("既にある折り目に突き当たったらそこで止まる", () => {
    // y=0.5 の水平な山折りがある紙で、(0.5,0.2)から真上へ伸ばすと y=0.5 で止まる
    const doc = squareDoc(
      [{ id: 4, v0: 4, v1: 5, kind: "Mountain" }],
      [
        { id: 4, pos: [0, 0.5] },
        { id: 5, pos: [1, 0.5] },
      ],
    );
    const hit = firstCrossing(doc, [0.5, 0.2], [0.5, 1]);
    expect(hit[0]).toBeCloseTo(0.5, 9);
    expect(hit[1]).toBeCloseTo(0.5, 9);
    // 何にも当たらなければ終点のまま
    expect(firstCrossing(doc, [0.5, 0.2], [0.5, 0])).toEqual([0.5, 0]);
  });
});

describe("描いている最中の形と案内", () => {
  it("点が足りないうちは直線、そろったら曲線になる", () => {
    const two = curvePolyline("arc", [[0, 0], [1, 0]], { segments: null });
    expect(two).toEqual([[0, 0], [1, 0]]);
    const three = curvePolyline("arc", [[0, 0], [1, 0], [0.5, 0.3]], { segments: null });
    expect((three ?? []).length).toBeGreaterThan(2);
    expect(curvePolyline("arc", [[0, 0]], { segments: null })).toBeNull();
    const bez = curvePolyline("bezier", [[0, 0], [1, 0], [0, 1], [1, 1]], { segments: null });
    expect((bez ?? []).length).toBeGreaterThan(2);
  });

  it("次にすることを1行で知らせる", () => {
    expect(curveHint("arc", 0, true)).toContain("始点");
    expect(curveHint("arc", 2, true)).toContain("通したい点");
    expect(curveHint("bezier", 3, true)).toContain("終点側");
    expect(curveHint("arc", 0, false)).toContain("曲がるための線なし");
  });
});
