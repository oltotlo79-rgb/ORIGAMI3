import { describe, expect, it } from "vitest";
import {
  LENGTH_RANGE,
  ROOT_ID,
  WIDTH_RANGE,
  addLimb,
  defaultSkeleton,
  leafNodes,
  skeletonRows,
} from "./skeleton";
import {
  calculatePreviewFrame,
  calculateProposalLayout,
} from "./proposalLayout";
import type { Skeleton } from "./types";

function withLargestParts(skeleton: Skeleton): Skeleton {
  return {
    nodes: skeleton.nodes.map((node) =>
      node.parent === null
        ? node
        : {
            ...node,
            length: LENGTH_RANGE.max,
            width_factor: WIDTH_RANGE.max,
          },
    ),
  };
}

function twelveRadialTips(): Skeleton {
  let skeleton = defaultSkeleton();
  while (leafNodes(skeleton).length < 12) {
    skeleton = addLimb(skeleton, ROOT_ID);
  }
  return withLargestParts(skeleton);
}

function depthTwelveChain(): Skeleton {
  return withLargestParts({
    nodes: [
      { id: ROOT_ID, parent: null, length: 0, width_factor: 1 },
      ...Array.from({ length: 12 }, (_, index) => ({
        id: index + 1,
        parent: index,
        length: 1,
        width_factor: 1,
      })),
    ],
  });
}

function deepTwelveTipBranch(): Skeleton {
  let skeleton = defaultSkeleton();
  skeleton = addLimb(skeleton, 1);
  const secondLevel = skeleton.nodes.find((node) => node.parent === 1)!;
  skeleton = addLimb(skeleton, secondLevel.id);
  const thirdLevel = skeleton.nodes.find(
    (node) => node.parent === secondLevel.id,
  )!;
  for (let index = 0; index < 9; index += 1) {
    skeleton = addLimb(skeleton, thirdLevel.id);
  }
  return withLargestParts(skeleton);
}

function numericValues(value: unknown): number[] {
  if (typeof value === "number") return [value];
  if (Array.isArray(value)) return value.flatMap(numericValues);
  if (value !== null && typeof value === "object") {
    return Object.values(value).flatMap(numericValues);
  }
  return [];
}

describe("提案画面の純粋な配置計算", () => {
  it("1000×700では込み入った形も横へ1pxもはみ出さない", () => {
    const complex = deepTwelveTipBranch();
    expect(leafNodes(complex)).toHaveLength(12);
    expect(Math.max(...skeletonRows(complex).map((row) => row.depth))).toBe(4);
    const samples = [
      ["初期4本", defaultSkeleton()],
      ["胴から12本", twelveRadialTips()],
      ["深さ12の一本道", depthTwelveChain()],
      ["深さ4・先端12本", complex],
    ] as const;

    for (const [name, skeleton] of samples) {
      const layout = calculateProposalLayout(skeleton, {
        width: 1_000,
        height: 700,
      });
      expect(layout.horizontalExcessPx, name).toBeLessThanOrEqual(0);
      expect(layout.horizontalOverflowPx, name).toBe(0);
      expect(layout.dialog, name).toEqual({ left: 140, width: 720, right: 860 });
      expect(layout.preview, name).toEqual({ left: 166, width: 200, right: 366 });
      expect(layout.list, name).toEqual({ left: 380, width: 437, right: 817 });
      expect(layout.horizontalExcessPx, name).toBe(-140);
      expect(layout.dialogMaxHeight, name).toBe(616);
      expect(layout.rows, name).toHaveLength(skeleton.nodes.length - 1);
      for (const span of [
        layout.dialog,
        layout.preview,
        layout.list,
        ...layout.rows,
      ]) {
        expect(span.left, name).toBeGreaterThanOrEqual(0);
        expect(span.right, name).toBeLessThanOrEqual(1_000);
        expect(span.width, name).toBeGreaterThanOrEqual(0);
      }
      expect(numericValues(layout).every(Number.isFinite), name).toBe(true);
    }
  });

  it("SVGの全ての線・接続位置・文字位置を見本枠の内側に置く", () => {
    const frame = calculatePreviewFrame(deepTwelveTipBranch());
    const minX = frame.viewBox.x;
    const maxX = frame.viewBox.x + frame.viewBox.width;

    for (const part of frame.parts) {
      const linePadding = part.strokeWidth / 2;
      expect(Math.min(part.start[0], part.end[0]) - linePadding).toBeGreaterThanOrEqual(
        minX,
      );
      expect(Math.max(part.start[0], part.end[0]) + linePadding).toBeLessThanOrEqual(
        maxX,
      );
      expect(part.start[0] - part.connectionRadius).toBeGreaterThanOrEqual(minX);
      expect(part.start[0] + part.connectionRadius).toBeLessThanOrEqual(maxX);
      expect(part.labelPosition[0]).toBeGreaterThanOrEqual(minX);
      expect(part.labelPosition[0]).toBeLessThanOrEqual(maxX);
    }
  });
});
