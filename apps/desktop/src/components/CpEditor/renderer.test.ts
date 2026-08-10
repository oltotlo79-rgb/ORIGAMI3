// 展開図を拡大したときに出す、右端・下端の位置バーの幾何テスト。

import { describe, expect, it, vi } from "vitest";
import type { Document, Vec2 } from "../../lib/types";
import { paperExtent } from "./snap";
import {
  deriveAxisPositionBar,
  deriveViewportPositionBars,
  drawGrid,
  fitView,
  gridDrawStride,
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
