// 展開図を拡大したときに出す、右端・下端の位置バーの幾何テスト。

import { describe, expect, it, vi } from "vitest";
import type { Document, Vec2 } from "../../lib/types";
import { paperExtent } from "./snap";
import {
  COLORS,
  deriveAxisPositionBar,
  deriveViewportPositionBars,
  drawGrid,
  drawKeyboardCursor,
  drawPinnedMarks,
  fitView,
  gridDrawStride,
  PIN_MARK_RADIUS,
  PIN_MARK_RELEASED_DASH,
  render as renderCp,
  worldToScreen,
  type RenderOverlay,
  type ViewTransform,
} from "./renderer";

function paperDoc(widthMm = 150, heightMm = 150): Document {
  const height = heightMm / Math.max(widthMm, heightMm);
  const width = widthMm / Math.max(widthMm, heightMm);
  const vertices: Vec2[] = [
    [0, 0],
    [width, 0],
    [width, height],
    [0, height],
  ];
  return {
    schema_version: 1,
    paper: { width_mm: widthMm, height_mm: heightMm },
    cp: {
      vertices: vertices.map((pos, id) => ({ id, pos })),
      edges: [0, 1, 2, 3].map((id) => ({
        id,
        v0: id,
        v1: (id + 1) % 4,
        kind: "Border" as const,
      })),
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
}

/** 指定した倍率で紙を表示区画の中央へ置く。 */
function centeredView(doc: Document, widthPx: number, heightPx: number, scale: number) {
  const [paperWidth, paperHeight] = paperExtent(doc);
  return {
    scale,
    offsetX: (widthPx - paperWidth * scale) / 2,
    offsetY: (heightPx + paperHeight * scale) / 2,
  } satisfies ViewTransform;
}

function gridContext() {
  const beginPath = vi.fn();
  const moveTo = vi.fn();
  const lineTo = vi.fn();
  const stroke = vi.fn();
  const ctx = {
    beginPath,
    moveTo,
    lineTo,
    stroke,
    setLineDash: vi.fn(),
    strokeStyle: "",
    lineWidth: 0,
  } as unknown as CanvasRenderingContext2D;
  return { ctx, beginPath, moveTo, lineTo, stroke };
}

describe("高密度方眼の間引き", () => {
  it("線間隔2pxを境に1→8→64本ごとへ切り替える", () => {
    expect(gridDrawStride(1024, 2048)).toBe(1);
    expect(gridDrawStride(1024, 2047)).toBe(8);
    expect(gridDrawStride(1024, 256)).toBe(8);
    expect(gridDrawStride(1024, 255)).toBe(64);
  });

  it("1024等分を1 Path・1 strokeで描き、拡大すると全細線を戻す", () => {
    const doc = paperDoc();
    doc.display.grid_divisions = 1024;

    const thinned = gridContext();
    drawGrid(thinned.ctx, doc, { scale: 1024, offsetX: 0, offsetY: 1024 }, [255, 255, 255]);
    expect(thinned.beginPath).toHaveBeenCalledTimes(1);
    expect(thinned.moveTo).toHaveBeenCalledTimes(254);
    expect(thinned.lineTo).toHaveBeenCalledTimes(254);
    expect(thinned.stroke).toHaveBeenCalledTimes(1);

    const zoomed = gridContext();
    drawGrid(zoomed.ctx, doc, { scale: 2048, offsetX: 0, offsetY: 2048 }, [255, 255, 255]);
    expect(zoomed.beginPath).toHaveBeenCalledTimes(1);
    expect(zoomed.moveTo).toHaveBeenCalledTimes(2046);
    expect(zoomed.lineTo).toHaveBeenCalledTimes(2046);
    expect(zoomed.stroke).toHaveBeenCalledTimes(1);
  });
});

describe("位置バー1軸の導出", () => {
  it("表示割合をつまみの長さ、スクロール進捗を位置にする", () => {
    // 紙1000px・表示400pxなのでつまみはトラックの40%。
    // 紙の先頭が-300pxなら、移動可能な600pxのちょうど中央を見ている。
    const bar = deriveAxisPositionBar(-300, 1000, 400, 10, 200);
    expect(bar.trackStart).toBe(10);
    expect(bar.trackLength).toBe(200);
    expect(bar.thumbLength).toBeCloseTo(80);
    expect(bar.thumbStart).toBeCloseTo(70);
  });

  it("紙を範囲外まで動かしてもつまみはトラック両端で止まる", () => {
    const before = deriveAxisPositionBar(100, 1000, 400, 10, 200);
    const after = deriveAxisPositionBar(-1000, 1000, 400, 10, 200);
    expect(before.thumbStart).toBe(10);
    expect(after.thumbStart).toBeCloseTo(130); // 10 + (200 - 80)
  });

  it("紙が軸方向に収まるときはトラック全長で全範囲を表す", () => {
    const bar = deriveAxisPositionBar(-80, 300, 400, 10, 200);
    expect(bar.thumbStart).toBe(10);
    expect(bar.thumbLength).toBe(200);
  });

  it("大きく拡大してもつまみを見失わない最小長に収める", () => {
    const bar = deriveAxisPositionBar(-4800, 10000, 400, 10, 200);
    expect(bar.thumbLength).toBe(20);
    expect(bar.thumbStart).toBeCloseTo(100); // 中央の進捗
  });
});

describe("展開図の位置バー", () => {
  it("全体表示の倍率1.0以下では出さない", () => {
    const doc = paperDoc();
    const fit = fitView(doc, 400, 400);
    expect(deriveViewportPositionBars(doc, fit, 400, 400)).toBeNull();
    expect(
      deriveViewportPositionBars(doc, { ...fit, scale: fit.scale * 0.8 }, 400, 400),
    ).toBeNull();
  });

  it("拡大した紙を中央表示すると、横・縦のつまみも中央に来る", () => {
    const doc = paperDoc();
    const scale = fitView(doc, 400, 400).scale * 2;
    const bars = deriveViewportPositionBars(
      doc,
      centeredView(doc, 400, 400, scale),
      400,
      400,
    );
    expect(bars).not.toBeNull();
    for (const bar of [bars!.horizontal, bars!.vertical]) {
      expect(bar.thumbLength / bar.trackLength).toBeCloseTo(400 / 720);
      expect(bar.thumbStart + bar.thumbLength / 2).toBeCloseTo(
        bar.trackStart + bar.trackLength / 2,
      );
    }
  });

  it("拡大中でも紙が収まる軸はトラック全長になる", () => {
    const doc = paperDoc(300, 150);
    const scale = fitView(doc, 400, 400).scale * 1.2;
    const bars = deriveViewportPositionBars(
      doc,
      centeredView(doc, 400, 400, scale),
      400,
      400,
    );
    expect(bars).not.toBeNull();
    expect(bars!.horizontal.thumbLength).toBeLessThan(bars!.horizontal.trackLength);
    expect(bars!.vertical.thumbStart).toBe(bars!.vertical.trackStart);
    expect(bars!.vertical.thumbLength).toBe(bars!.vertical.trackLength);
  });
});

/** 印の描画だけを見るための、arc呼び出しを記録する偽の描画先。 */
function markContext() {
  const arcs: { x: number; y: number; r: number; dash: number[]; style: string }[] = [];
  const fills: { x: number; y: number; r: number }[] = [];
  let dash: number[] = [];
  let pending: { x: number; y: number; r: number } | null = null;
  const ctx = {
    save: vi.fn(),
    restore: vi.fn(),
    beginPath: vi.fn(),
    arc: vi.fn((x: number, y: number, r: number) => {
      pending = { x, y, r };
    }),
    stroke: vi.fn(() => {
      if (pending) arcs.push({ ...pending, dash: [...dash], style: ctx.strokeStyle });
    }),
    fill: vi.fn(() => {
      if (pending) fills.push({ ...pending });
    }),
    setLineDash: vi.fn((next: number[]) => {
      dash = next;
    }),
    strokeStyle: "",
    fillStyle: "",
    lineWidth: 0,
  } as unknown as CanvasRenderingContext2D & { strokeStyle: string };
  return { ctx, arcs, fills };
}

describe("キーボードで指している現在位置", () => {
  it("正しい画面座標へ白い縁と青い輪・十字を重ねて描く", () => {
    const strokes: { style: string; width: number }[] = [];
    const arc = vi.fn();
    const moveTo = vi.fn();
    const lineTo = vi.fn();
    const rawContext = {
      save: vi.fn(),
      restore: vi.fn(),
      setLineDash: vi.fn(),
      beginPath: vi.fn(),
      arc,
      moveTo,
      lineTo,
      stroke: vi.fn(),
      strokeStyle: "",
      lineWidth: 0,
    };
    rawContext.stroke.mockImplementation(() =>
      strokes.push({ style: rawContext.strokeStyle, width: rawContext.lineWidth }),
    );
    const ctx = rawContext as unknown as CanvasRenderingContext2D;
    const view: ViewTransform = { scale: 200, offsetX: 10, offsetY: 210 };
    const cursor: Vec2 = [0.25, 0.75];
    const [sx, sy] = worldToScreen(view, cursor);

    drawKeyboardCursor(ctx, view, cursor);

    expect(arc).toHaveBeenCalledWith(sx, sy, 8, 0, Math.PI * 2);
    expect(moveTo.mock.calls).toEqual([
      [sx - 13, sy],
      [sx, sy - 13],
    ]);
    expect(lineTo.mock.calls).toEqual([
      [sx + 13, sy],
      [sx, sy + 13],
    ]);
    expect(strokes).toEqual([
      { style: COLORS.keyboardCursorHalo, width: 5 },
      { style: COLORS.keyboardCursor, width: 2 },
    ]);
    expect(rawContext.save).toHaveBeenCalledTimes(1);
    expect(rawContext.restore).toHaveBeenCalledTimes(1);
  });

  it("吸着中は青い現在位置を先に描き、その上へ緑の吸着輪を描く", () => {
    const strokeStyles: string[] = [];
    const rawContext = {
      canvas: {} as HTMLCanvasElement,
      arc: vi.fn(),
      beginPath: vi.fn(),
      fill: vi.fn(),
      fillRect: vi.fn(),
      fillText: vi.fn(),
      lineTo: vi.fn(),
      measureText: vi.fn(() => ({ width: 0 }) as TextMetrics),
      moveTo: vi.fn(),
      restore: vi.fn(),
      save: vi.fn(),
      setLineDash: vi.fn(),
      setTransform: vi.fn(),
      stroke: vi.fn(),
      strokeRect: vi.fn(),
      fillStyle: "",
      font: "",
      lineWidth: 0,
      shadowBlur: 0,
      shadowColor: "",
      strokeStyle: "",
      textBaseline: "alphabetic" as CanvasTextBaseline,
    };
    rawContext.stroke.mockImplementation(() => strokeStyles.push(rawContext.strokeStyle));
    const ctx = rawContext as unknown as CanvasRenderingContext2D;
    const doc = paperDoc();
    const overlay: RenderOverlay = {
      hoverSnap: { pos: [0.5, 0.5], kind: "grid" },
      preview: null,
      keyboardCursor: [0.5, 0.5],
      directionGuide: null,
      mirrorAxis: null,
      mirrorPreview: null,
      previewPaths: [],
      marquee: null,
      violations: [],
      constructPoints: [],
      hint: null,
      tooltip: null,
      vertexDrag: null,
    };

    renderCp(
      ctx,
      400,
      400,
      1,
      doc,
      fitView(doc, 400, 400),
      { edgeIds: [], vertexIds: [] },
      overlay,
    );

    const cursorStyles = new Set<string>([
      COLORS.keyboardCursorHalo,
      COLORS.keyboardCursor,
      COLORS.snapMarker,
    ]);
    expect(strokeStyles.filter((style) => cursorStyles.has(style))).toEqual([
      COLORS.keyboardCursorHalo,
      COLORS.keyboardCursor,
      COLORS.snapMarker,
    ]);
  });
});

/** 紙の中央を横切る折り線を1本足した展開図(辺ID 4)。 */
function docWithCrease(): Document {
  const doc = paperDoc();
  doc.cp.vertices.push({ id: 4, pos: [0, 0.5] }, { id: 5, pos: [1, 0.5] });
  doc.cp.edges.push({ id: 4, v0: 4, v1: 5, kind: "Mountain" });
  doc.cp.next_vertex_id = 6;
  doc.cp.next_edge_id = 5;
  return doc;
}

describe("固定した折り目の印(2D)", () => {
  it("固定していなければ何も描かない", () => {
    const { ctx, arcs } = markContext();
    const doc = docWithCrease();
    drawPinnedMarks(ctx, doc, fitView(doc, 400, 400), [], []);
    expect(arcs).toEqual([]);
  });

  it("選んでいなくても、固定した折り目に印を描く", () => {
    const { ctx, arcs, fills } = markContext();
    const doc = docWithCrease();
    const view = fitView(doc, 400, 400);
    // 選択も、指している折り目も、操作中の折り目も渡さない。
    drawPinnedMarks(ctx, doc, view, [4], []);
    // 白い縁取りと本体で2重、中心の点で1つ。
    expect(arcs).toHaveLength(2);
    expect(fills).toHaveLength(1);
    const center = worldToScreen(view, [0.5, 0.5]);
    for (const arc of arcs) {
      expect(arc.x).toBeCloseTo(center[0], 6);
      expect(arc.y).toBeCloseTo(center[1], 6);
      expect(arc.r).toBe(PIN_MARK_RADIUS);
    }
    // 固定中は実線
    expect(arcs[1].dash).toEqual([]);
    expect(arcs[1].style).toBe(COLORS.pinned);
  });

  it("固定が外れている折り目は破線の輪にし、中心の点を打たない", () => {
    const { ctx, arcs, fills } = markContext();
    const doc = docWithCrease();
    drawPinnedMarks(ctx, doc, fitView(doc, 400, 400), [4], [4]);
    expect(arcs).toHaveLength(2);
    expect(arcs[1].dash).toEqual([...PIN_MARK_RELEASED_DASH]);
    expect(fills).toEqual([]);
  });

  it("線の色を増やしていない(印は既存の4色とは別の仕組み)", () => {
    // 2Dで線に重なる強調は 食い込み・指している・選択・操作中 の4つのまま。
    // 固定は5色目を足さず、輪の印で示す。
    const lineEmphasisColors = [
      COLORS.suspectGlow,
      COLORS.hingeHover,
      COLORS.selection,
      COLORS.active,
    ];
    expect(new Set(lineEmphasisColors).size).toBe(4);
    expect(lineEmphasisColors).not.toContain(COLORS.pinned);
  });

  it("参照切れの線には印を置かない", () => {
    const { ctx, arcs } = markContext();
    const doc = docWithCrease();
    doc.cp.edges.push({ id: 9, v0: 90, v1: 91, kind: "Mountain" });
    drawPinnedMarks(ctx, doc, fitView(doc, 400, 400), [9], []);
    expect(arcs).toEqual([]);
  });
});
