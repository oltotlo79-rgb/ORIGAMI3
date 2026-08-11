// 任意の基準線で対称にそろえる計算のテスト(CPE-010)。

import { describe, expect, it } from "vitest";
import {
  MIRROR_EPS,
  isOnMirrorAxis,
  isSameSegment,
  isValidMirrorLine,
  mirrorAxisX,
  mirrorLineForChoice,
  mirrorLineInsidePaper,
  mirrorLineThrough,
  mirrorPoint,
  mirrorSegment,
  mirrorSegments,
  normalizedPaperSize,
  paperMirrorLine,
  rebindSelectedMirrorAxis,
  type MirrorLine,
  type Segment,
} from "./mirror";
import type { Document, Vec2 } from "./types";
import { DEFAULT_DISPLAY } from "./displayPrefs";

const SQUARE = { width_mm: 150, height_mm: 150 };
const VERTICAL = paperMirrorLine(SQUARE, "paperVertical");
const HORIZONTAL = paperMirrorLine(SQUARE, "paperHorizontal");
const DIAGONAL: MirrorLine = { p: [0, 0], d: [2, 2] };

function expectPoint(actual: Vec2, expected: Vec2) {
  expect(actual[0]).toBeCloseTo(expected[0], 12);
  expect(actual[1]).toBeCloseTo(expected[1], 12);
}

function documentWithSegments(
  segments: { id: number; a: Vec2; b: Vec2; kind?: "Mountain" | "Valley" | "Aux" | "Border" }[],
): Document {
  const vertices: Document["cp"]["vertices"] = [];
  const edges: Document["cp"]["edges"] = [];
  let nextVertexId = 0;
  for (const segment of segments) {
    const v0 = nextVertexId++;
    const v1 = nextVertexId++;
    vertices.push(
      { id: v0, pos: segment.a },
      { id: v1, pos: segment.b },
    );
    edges.push({
      id: segment.id,
      v0,
      v1,
      kind: segment.kind ?? "Aux",
    });
  }
  return {
    schema_version: 1,
    paper: SQUARE,
    cp: {
      vertices,
      edges,
      next_vertex_id: nextVertexId,
      next_edge_id: Math.max(0, ...segments.map((segment) => segment.id + 1)),
    },
    sequence: [],
    display: DEFAULT_DISPLAY,
  };
}

describe("対称操作の基準線と計算", () => {
  it("既定の縦中心線は従来と同じ位置で、横中心線も紙の中央を通る", () => {
    expect(mirrorAxisX(SQUARE)).toBeCloseTo(0.5);
    expect(mirrorAxisX({ width_mm: 100, height_mm: 200 })).toBeCloseTo(0.25);
    expect(mirrorAxisX({ width_mm: 200, height_mm: 100 })).toBeCloseTo(0.5);
    expect(paperMirrorLine(SQUARE, "paperVertical")).toEqual({
      p: [0.5, 0.5],
      d: [0, 1],
    });
    expect(paperMirrorLine(SQUARE, "paperHorizontal")).toEqual({
      p: [0.5, 0.5],
      d: [1, 0],
    });
    expect(normalizedPaperSize({ width_mm: Number.POSITIVE_INFINITY, height_mm: 100 })).toEqual([
      0,
      0,
    ]);
  });

  it("縦長・横長の紙でも中心線と画面ガイドが正規化した紙の中央を通る", () => {
    const portrait = { width_mm: 100, height_mm: 200 };
    expect(
      mirrorLineInsidePaper(
        portrait,
        paperMirrorLine(portrait, "paperVertical"),
      ),
    ).toEqual([[0.25, 0], [0.25, 1]]);
    expect(
      mirrorLineInsidePaper(
        portrait,
        paperMirrorLine(portrait, "paperHorizontal"),
      ),
    ).toEqual([[0, 0.5], [0.5, 0.5]]);

    const landscape = { width_mm: 200, height_mm: 100 };
    expect(
      mirrorLineInsidePaper(
        landscape,
        paperMirrorLine(landscape, "paperVertical"),
      ),
    ).toEqual([[0.5, 0], [0.5, 0.5]]);
    expect(
      mirrorLineInsidePaper(
        landscape,
        paperMirrorLine(landscape, "paperHorizontal"),
      ),
    ).toEqual([[0, 0.25], [1, 0.25]]);
  });

  it("縦・横・選んだ斜め線の3種類で、垂線の足をはさんだ位置へ移る", () => {
    expectPoint(mirrorPoint([0.2, 0.7], VERTICAL), [0.8, 0.7]);
    expectPoint(mirrorPoint([0.2, 0.3], HORIZONTAL), [0.2, 0.7]);
    // d=[2,2]のように単位長でない向きでも、y=xを基準にxとyが入れ替わる。
    expectPoint(mirrorPoint([0.2, 0.7], DIAGONAL), [0.7, 0.2]);
    const twice = mirrorPoint(mirrorPoint([0.17, 0.82], DIAGONAL), DIAGONAL);
    expectPoint(twice, [0.17, 0.82]);
  });

  it("基準線上と許容誤差内の点は動かさず、誤差を超えた点だけ反対側へ移す", () => {
    const on = [0.5, 0.3] as Vec2;
    expect(mirrorPoint(on, VERTICAL)).toEqual(on);
    const within = [0.5 + MIRROR_EPS / 2, 0.4] as Vec2;
    expect(mirrorPoint(within, VERTICAL)).toEqual(within);
    expectPoint(mirrorPoint([0.5 + MIRROR_EPS * 2, 0.4], VERTICAL), [
      0.5 - MIRROR_EPS * 2,
      0.4,
    ]);
  });

  it("任意方向の基準線上にある線が分かる", () => {
    expect(isOnMirrorAxis([[0.2, 0.2], [0.8, 0.8]], DIAGONAL)).toBe(true);
    expect(isOnMirrorAxis([[0.2, 0.2], [0.8, 0.7]], DIAGONAL)).toBe(false);
  });

  it("同じ線分は向きが逆でも同じと分かる", () => {
    const a: Segment = [[0.1, 0.2], [0.9, 0.8]];
    const b: Segment = [[0.9, 0.8], [0.1, 0.2]];
    expect(isSameSegment(a, b)).toBe(true);
    expect(isSameSegment(a, [[0.1, 0.2], [0.8, 0.8]])).toBe(false);
  });

  it("普通の線は2本、基準線上・もともと対称な線は1本だけになる", () => {
    expect(mirrorSegments([[0.1, 0], [0.4, 1]], VERTICAL)).toEqual([
      [[0.1, 0], [0.4, 1]],
      [[0.9, 0], [0.6, 1]],
    ]);
    expect(mirrorSegments([[0.5, 0], [0.5, 1]], VERTICAL)).toHaveLength(1);
    expect(mirrorSegments([[0.2, 0.5], [0.8, 0.5]], VERTICAL)).toHaveLength(1);
    expect(mirrorSegments([[0.2, 0.2], [0.8, 0.8]], DIAGONAL)).toHaveLength(1);
  });

  it("退化した基準線はNaNを出さず、元の点・線をそのまま保つ", () => {
    const invalid: MirrorLine = { p: [0.5, 0.5], d: [0, 0] };
    expect(mirrorPoint([0.2, 0.7], invalid)).toEqual([0.2, 0.7]);
    expect(mirrorSegment([[0, 0], [1, 1]], invalid)).toEqual([[0, 0], [1, 1]]);
    expect(mirrorSegments([[0, 0], [1, 1]], invalid)).toHaveLength(1);
    expect(mirrorLineThrough([0.2, 0.2], [0.2, 0.2])).toBeNull();
  });

  it("向きベクトルの倍率に依存せず、極小倍率でも同じ直線として扱う", () => {
    const tiny: MirrorLine = { p: [0, 0], d: [1e-12, 1e-12] };
    const huge: MirrorLine = { p: [0, 0], d: [1e200, 1e200] };
    for (const axis of [tiny, huge]) {
      expect(isValidMirrorLine(axis)).toBe(true);
      expectPoint(mirrorPoint([0.2, 0.7], axis), [0.7, 0.2]);
      expect(mirrorLineInsidePaper(SQUARE, axis)).toEqual([[0, 0], [1, 1]]);
    }
  });

  it("非有限・0方向の基準線と不正な許容誤差を安全に扱う", () => {
    const point: Vec2 = [0.2, 0.7];
    for (const invalid of [
      { p: [Number.POSITIVE_INFINITY, 0], d: [1, 0] },
      { p: [0, 0], d: [Number.NaN, 1] },
      { p: [0, 0], d: [0, 0] },
    ] as MirrorLine[]) {
      expect(isValidMirrorLine(invalid)).toBe(false);
      expect(mirrorPoint(point, invalid)).toEqual(point);
      expect(mirrorLineInsidePaper(SQUARE, invalid)).toBeNull();
    }

    const within: Vec2 = [0.5 + MIRROR_EPS / 2, 0.4];
    expect(mirrorPoint(within, VERTICAL, Number.NaN)).toEqual(within);
    expect(mirrorPoint(within, VERTICAL, -1)).toEqual(within);
    expect(
      mirrorSegments([[0.5, 0], [0.5, 1]], VERTICAL, -1),
    ).toHaveLength(1);
  });

  it("選んだ折り線から基準を作り、紙の輪郭まで斜めガイドを延ばせる", () => {
    const doc: Document = {
      schema_version: 1,
      paper: SQUARE,
      cp: {
        vertices: [
          { id: 0, pos: [0, 0] },
          { id: 1, pos: [1, 1] },
        ],
        edges: [{ id: 7, v0: 0, v1: 1, kind: "Aux" }],
        next_vertex_id: 2,
        next_edge_id: 8,
      },
      sequence: [],
      display: DEFAULT_DISPLAY,
    };
    const axis = mirrorLineForChoice(doc, { kind: "selectedLine", edgeId: 7 });
    expect(axis).not.toBeNull();
    expect(mirrorLineInsidePaper(doc.paper, axis!)).toEqual([[0, 0], [1, 1]]);
  });

  it("選んだ基準辺のIDが残っていれば、そのまま維持する", () => {
    const previous = documentWithSegments([
      { id: 7, a: [0, 0], b: [1, 1] },
    ]);
    const next = documentWithSegments([
      { id: 7, a: [0, 0], b: [0.5, 0.5] },
      { id: 8, a: [0.5, 0.5], b: [1, 1] },
    ]);
    expect(
      rebindSelectedMirrorAxis(previous, next, {
        kind: "selectedLine",
        edgeId: 7,
      }),
    ).toEqual({ kind: "selectedLine", edgeId: 7 });
  });

  it("交差追加で基準辺IDが消えても、分割片が全長を覆えば片のIDへ結び直す", () => {
    const previous = documentWithSegments([
      { id: 7, a: [0, 0], b: [1, 1] },
    ]);
    const next = documentWithSegments([
      { id: 9, a: [0, 0], b: [0.4, 0.4] },
      { id: 8, a: [0.4, 0.4], b: [1, 1] },
      { id: 10, a: [0, 1], b: [1, 0], kind: "Mountain" },
    ]);
    expect(
      rebindSelectedMirrorAxis(previous, next, {
        kind: "selectedLine",
        edgeId: 7,
      }),
    ).toEqual({ kind: "selectedLine", edgeId: 9 });
  });

  it("分割片の継ぎ目は許容誤差内だけ連続とみなす", () => {
    const previous = documentWithSegments([
      { id: 7, a: [0, 0], b: [1, 0] },
    ]);
    const choice = { kind: "selectedLine", edgeId: 7 } as const;
    const within = documentWithSegments([
      { id: 8, a: [0, 0], b: [0.5, 0] },
      { id: 9, a: [0.5 + MIRROR_EPS / 2, 0], b: [1, 0] },
    ]);
    expect(rebindSelectedMirrorAxis(previous, within, choice)).toEqual({
      kind: "selectedLine",
      edgeId: 8,
    });

    const missing = documentWithSegments([
      { id: 8, a: [0, 0], b: [0.5, 0] },
      { id: 9, a: [0.5 + MIRROR_EPS * 2, 0], b: [1, 0] },
    ]);
    expect(rebindSelectedMirrorAxis(previous, missing, choice)).toBeNull();
  });

  it("基準線が削除・一部欠落・輪郭化したときは結び直さない", () => {
    const previous = documentWithSegments([
      { id: 7, a: [0, 0], b: [1, 1] },
    ]);
    const choice = { kind: "selectedLine", edgeId: 7 } as const;
    expect(rebindSelectedMirrorAxis(previous, documentWithSegments([]), choice)).toBeNull();
    expect(
      rebindSelectedMirrorAxis(
        previous,
        documentWithSegments([{ id: 8, a: [0, 0], b: [0.4, 0.4] }]),
        choice,
      ),
    ).toBeNull();
    expect(
      rebindSelectedMirrorAxis(
        previous,
        documentWithSegments([
          { id: 8, a: [0, 0], b: [1, 1], kind: "Border" },
        ]),
        choice,
      ),
    ).toBeNull();
  });

  it("紙の中心線プリセットは作品更新時もそのまま返す", () => {
    const empty = documentWithSegments([]);
    expect(
      rebindSelectedMirrorAxis(empty, empty, { kind: "paperHorizontal" }),
    ).toEqual({ kind: "paperHorizontal" });
  });
});
