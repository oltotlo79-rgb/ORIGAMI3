import { describe, expect, it } from "vitest";
import { defaultSkeleton, setTipPos } from "./skeleton";
import {
  clientPointToPaperPosition,
  paperBounds,
  paperEditorViewBox,
  paperPositionLabelBounds,
  paperPositionLabelLayout,
  paperPositionLabelLayouts,
  paperPointToPosition,
  paperPositionToPoint,
  paperPositionsFromCandidate,
  skeletonForPaperPositions,
} from "./paperPosition";
import type { CreasePattern, ProposalCandidate } from "./types";

function rectangleCp(width = 1, height = 0.6): CreasePattern {
  return {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [width, 0] },
      { id: 2, pos: [width, height] },
      { id: 3, pos: [0, height] },
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

function candidateWithCenters(): ProposalCandidate {
  return {
    cp: rectangleCp(),
    scale: 0.4,
    violations: 0,
    warnings: [],
    sites: [
      {
        circle: { leaf_id: 2, circle_index: 1, center: [0.8, 0.48], radius: 0.1 },
        vertex: null,
        molecules: [],
      },
      {
        circle: { leaf_id: 1, circle_index: 0, center: [0.2, 0.12], radius: 0.1 },
        vertex: null,
        molecules: [],
      },
    ],
  };
}

describe("紙の上の場所の純粋な変換", () => {
  it("紙の点と一時入力を441点で往復してもずれない", () => {
    const bounds = paperBounds(rectangleCp());
    let worst = 0;
    for (let x = 0; x <= 20; x += 1) {
      for (let y = 0; y <= 20; y += 1) {
        const point: [number, number] = [x / 20, (y / 20) * 0.6];
        const back = paperPositionToPoint(
          paperPointToPosition(point, bounds),
          bounds,
        );
        worst = Math.max(
          worst,
          Math.abs(back[0] - point[0]),
          Math.abs(back[1] - point[1]),
        );
      }
    }
    expect(worst).toBeLessThanOrEqual(1.2e-16);
    expect(worst).toBeLessThan(1e-9);
  });

  it("画面の余白を戻し、紙の外へ引っぱっても縁で止める", () => {
    const bounds = paperBounds(rectangleCp());
    const viewBox = paperEditorViewBox(bounds);
    // viewBox 1.12×0.72を同じ縦横比の560×360へ描く。
    const rect = { left: 10, top: 20, width: 560, height: 360 };
    const at = (x: number, y: number) => ({
      clientX: rect.left + ((x - viewBox.x) / viewBox.width) * rect.width,
      clientY: rect.top + ((-y - viewBox.y) / viewBox.height) * rect.height,
    });
    const center = at(0.75, 0.45);
    const got = clientPointToPaperPosition(
      [center.clientX, center.clientY],
      rect,
      bounds,
    );
    expect(Math.abs(got.x - 0.5)).toBeLessThan(1e-12);
    expect(Math.abs(got.y - 0.3)).toBeLessThan(1e-12);

    const outside = clientPointToPaperPosition([-10_000, 10_000], rect, bounds);
    expect(outside).toEqual({ x: -1, y: -0.6 });
  });

  it("紙の四隅と細長い紙でも、長い先端名の字面を見本から切らさない", () => {
    for (const cp of [rectangleCp(1, 0.6), rectangleCp(0.05, 1)]) {
      const bounds = paperBounds(cp);
      const viewBox = paperEditorViewBox(bounds);
      const label = "頭のその先1のその先1のその先1";
      for (const point of [
        [bounds.minX, bounds.minY],
        [bounds.minX, bounds.maxY],
        [bounds.maxX, bounds.minY],
        [bounds.maxX, bounds.maxY],
      ] as const) {
        const layout = paperPositionLabelLayout(
          [point[0], point[1]],
          viewBox,
          label,
          bounds.longSide * 0.036,
          bounds.longSide * 0.028,
        );
        const width = layout.fontSize * [...label].length;
        const left =
          layout.textAnchor === "start"
            ? layout.x
            : layout.textAnchor === "end"
              ? layout.x - width
              : layout.x - width / 2;
        const right = left + width;
        const top = layout.y - layout.fontSize - layout.strokeWidth / 2;
        const bottom =
          layout.y + layout.fontSize * 0.25 + layout.strokeWidth / 2;
        expect(left).toBeGreaterThanOrEqual(viewBox.x - 1e-12);
        expect(right).toBeLessThanOrEqual(viewBox.x + viewBox.width + 1e-12);
        expect(top).toBeGreaterThanOrEqual(viewBox.y - 1e-12);
        expect(bottom).toBeLessThanOrEqual(viewBox.y + viewBox.height + 1e-12);
      }
    }
  });

  it("同一点の12本・細長い紙・深い名前でも、見えている先端名を重ねない", () => {
    let hiddenInImpossibleCase = 0;
    for (const [width, height] of [
      [1, 0.6],
      [1, 0.05],
      [0.05, 1],
    ] as const) {
      const bounds = paperBounds(rectangleCp(width, height));
      const viewBox = paperEditorViewBox(bounds);
      const inputs = Array.from({ length: 12 }, (_, index) => ({
        id: index + 1,
        point: [width / 2, height / 2] as [number, number],
        label: Array.from(
          { length: 12 },
          (_, depth) => `その先${depth + 1}`,
        ).join("の"),
      }));

      for (const priority of [null, ...inputs.map((input) => input.id)]) {
        const layouts = paperPositionLabelLayouts(
          priority === null ? [...inputs].reverse() : inputs,
          viewBox,
          bounds.longSide * 0.036,
          bounds.longSide * 0.028,
          priority,
        );
        const labelById = new Map(inputs.map((input) => [input.id, input.label]));
        const visible = layouts.filter((layout) => layout.visible);
        const boxes = visible.map((layout) =>
          paperPositionLabelBounds(layout, labelById.get(layout.id) ?? ""),
        );

        expect(layouts).toHaveLength(12);
        if (priority !== null) {
          expect(layouts.find((layout) => layout.id === priority)?.visible).toBe(true);
        }
        for (const box of boxes) {
          expect(box.left).toBeGreaterThanOrEqual(viewBox.x - 1e-12);
          expect(box.right).toBeLessThanOrEqual(viewBox.x + viewBox.width + 1e-12);
          expect(box.top).toBeGreaterThanOrEqual(viewBox.y - 1e-12);
          expect(box.bottom).toBeLessThanOrEqual(viewBox.y + viewBox.height + 1e-12);
        }
        for (let left = 0; left < boxes.length; left += 1) {
          for (let right = left + 1; right < boxes.length; right += 1) {
            const a = boxes[left];
            const b = boxes[right];
            const overlap =
              a.left < b.right &&
              b.left < a.right &&
              a.top < b.bottom &&
              b.top < a.bottom;
            expect(overlap).toBe(false);
          }
        }
        if (height === 0.05 && priority === null) {
          hiddenInImpossibleCase = layouts.filter((layout) => !layout.visible).length;
        }
      }
    }
    expect(hiddenInImpossibleCase).toBeGreaterThan(0);
  });

  it("通常の4×3配置では12本すべての先端名を表示する", () => {
    const bounds = paperBounds(rectangleCp(1, 0.6));
    const viewBox = paperEditorViewBox(bounds);
    const fontSize = bounds.longSide * 0.036;
    const strokeWidth = fontSize * 0.15;
    const collisionGap = fontSize * 0.12;
    const handleObstacleRadius = bounds.longSide * 0.028 * 1.8;
    const columnSpacing = 0.8 / 3;
    const rowSpacing = 0.2;
    // 実測式の字面は最長の「先端10」で4文字。縁取りと間隔を足しても
    // 横0.15372 < 列間0.266666…、縦0.05472 < 行間0.2で、12本を隠す理由はない。
    const widestLabelWithStroke = fontSize * 4 + strokeWidth;
    const labelHeightWithStroke = fontSize * 1.25 + strokeWidth;
    expect(viewBox.width).toBeCloseTo(1.12, 12);
    expect(viewBox.height).toBeCloseTo(0.72, 12);
    expect(widestLabelWithStroke + collisionGap).toBeCloseTo(0.15372, 12);
    expect(labelHeightWithStroke + collisionGap).toBeCloseTo(0.05472, 12);
    expect(columnSpacing - widestLabelWithStroke - collisionGap).toBeCloseTo(
      0.11294666666666667,
      12,
    );
    expect(rowSpacing - labelHeightWithStroke - collisionGap).toBeCloseTo(
      0.14528,
      12,
    );
    expect(handleObstacleRadius).toBeCloseTo(0.0504, 12);
    expect(columnSpacing).toBeGreaterThan(handleObstacleRadius * 2);
    expect(rowSpacing).toBeGreaterThan(handleObstacleRadius * 2);
    const inputs = Array.from({ length: 12 }, (_, index) => ({
      id: index + 1,
      point: [
        0.1 + (index % 4) * (0.8 / 3),
        0.1 + Math.floor(index / 4) * 0.2,
      ] as [number, number],
      label: `先端${index + 1}`,
    }));
    const layouts = paperPositionLabelLayouts(
      inputs,
      viewBox,
      fontSize,
      bounds.longSide * 0.028,
    );
    expect(layouts.filter((layout) => layout.visible)).toHaveLength(12);
    const inputById = new Map(inputs.map((input) => [input.id, input]));
    const maximumVerticalTravel = Math.max(
      ...layouts.map((layout) => {
        const input = inputById.get(layout.id)!;
        return Math.abs(layout.y + input.point[1]);
      }),
    );
    // 境界駆動の候補は最寄りから採用し、560px表示で約33.21px以内に名前を置く。
    expect(maximumVerticalTravel).toBeCloseTo(0.066420036, 8);
    expect(maximumVerticalTravel * 500).toBeLessThanOrEqual(33.22);
    const labelById = new Map(inputs.map((input) => [input.id, input.label]));
    const boxes = layouts.map((layout) =>
      paperPositionLabelBounds(layout, labelById.get(layout.id) ?? ""),
    );
    for (const box of boxes) {
      expect(box.left).toBeGreaterThanOrEqual(viewBox.x - 1e-12);
      expect(box.right).toBeLessThanOrEqual(viewBox.x + viewBox.width + 1e-12);
      expect(box.top).toBeGreaterThanOrEqual(viewBox.y - 1e-12);
      expect(box.bottom).toBeLessThanOrEqual(viewBox.y + viewBox.height + 1e-12);
    }
    for (let left = 0; left < boxes.length; left += 1) {
      for (let right = left + 1; right < boxes.length; right += 1) {
        const a = boxes[left];
        const b = boxes[right];
        const separated =
          a.right + collisionGap <= b.left + 1e-12 ||
          b.right + collisionGap <= a.left + 1e-12 ||
          a.bottom + collisionGap <= b.top + 1e-12 ||
          b.bottom + collisionGap <= a.top + 1e-12;
        expect(separated).toBe(true);
      }
    }
    const handleObstacles = inputs.map((input) => ({
      left: input.point[0] - handleObstacleRadius,
      top: -input.point[1] - handleObstacleRadius,
      right: input.point[0] + handleObstacleRadius,
      bottom: -input.point[1] + handleObstacleRadius,
    }));
    for (const box of boxes) {
      for (const handle of handleObstacles) {
        const separated =
          box.right + collisionGap <= handle.left + 1e-12 ||
          handle.right + collisionGap <= box.left + 1e-12 ||
          box.bottom + collisionGap <= handle.top + 1e-12 ||
          handle.bottom + collisionGap <= box.top + 1e-12;
        expect(separated).toBe(true);
      }
    }
  });

  it("候補の対応を葉ID順の独立した紙位置へ直す", () => {
    const positions = paperPositionsFromCandidate(candidateWithCenters());
    expect(positions.map((entry) => entry.leaf_id)).toEqual([1, 2]);
    expect(positions[0].position).toEqual({ x: -0.6, y: -0.36 });
    expect(positions[1].position).toEqual({ x: 0.6000000000000001, y: 0.36 });
  });

  it("送信用の複製だけへ紙位置を載せ、完成形の位置は壊さない", () => {
    let completion = defaultSkeleton();
    completion = setTipPos(completion, 1, { x: 0.75, y: -0.5 });
    const before = JSON.stringify(completion);
    const paperPositions = [
      { leaf_id: 1, position: { x: -0.4, y: 0.3 } },
      { leaf_id: 2, position: { x: 0.6, y: -0.2 } },
    ];
    const request = skeletonForPaperPositions(completion, paperPositions);

    expect(JSON.stringify(completion)).toBe(before);
    expect(completion.nodes.find((node) => node.id === 1)?.tip_pos_2d).toEqual({
      x: 0.75,
      y: -0.5,
    });
    for (const expected of paperPositions) {
      const sent = request.nodes.find((node) => node.id === expected.leaf_id)
        ?.tip_pos_2d;
      expect(sent).not.toBeNull();
      expect(Math.abs((sent?.x ?? 0) - expected.position.x)).toBeLessThan(1e-12);
      expect(Math.abs((sent?.y ?? 0) - expected.position.y)).toBeLessThan(1e-12);
    }
    // 紙位置を渡さなかった葉へ、完成形の指定を混ぜない。
    expect(request.nodes.find((node) => node.id === 3)?.tip_pos_2d).toBeUndefined();
  });
});
