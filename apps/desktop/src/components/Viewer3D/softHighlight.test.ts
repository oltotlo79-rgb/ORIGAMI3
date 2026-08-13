import { describe, expect, it } from "vitest";
import * as THREE from "three";
import type { Vec3 } from "../../lib/layerOffset";
import type { Document, Face, SoftMesh } from "../../lib/types";
import type { HighlightSegment, SoftContent } from "./sceneBuilder";
import {
  buildSoftHighlightMap,
  projectHighlightSegmentsToSoftSurface,
} from "./softHighlight";
import { buildSoftLayout, fillSoftPositions } from "./softMesh";

const QUAD_DOC: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [1, 0] },
      { id: 2, pos: [1, 1] },
      { id: 3, pos: [0, 1] },
    ],
    edges: [
      { id: 10, v0: 0, v1: 1, kind: "Border" },
      { id: 11, v0: 1, v1: 2, kind: "Border" },
      { id: 12, v0: 2, v1: 3, kind: "Border" },
      { id: 13, v0: 3, v1: 0, kind: "Border" },
    ],
    next_vertex_id: 4,
    next_edge_id: 14,
  },
  sequence: [],
  display: {
    front_color: [230, 90, 60],
    back_color: [245, 245, 245],
    grid_divisions: 8,
  },
};

const QUAD_FACE: Face = {
  id: 0,
  vertices: [0, 1, 2, 3],
  edges: [10, 11, 12, 13],
};

/** Rustのear_clipが正方形を[3,0,1],[1,2,3]の順に切り、各辺を2分する結果。 */
function bowedQuad(): SoftMesh {
  return {
    positions: [
      [0, 1, 0],
      [0.5, 0.5, 0.24],
      [1, 0, 0],
      [0, 0.5, 0.08],
      [0.5, 0, 0.18],
      [0, 0, 0],
      [1, 0.5, 0.08],
      [0.5, 1, 0.12],
      [1, 1, 0],
    ],
    triangles: [
      [0, 3, 1],
      [3, 4, 1],
      [1, 4, 2],
      [3, 5, 4],
      [2, 6, 1],
      [6, 7, 1],
      [1, 7, 0],
      [6, 8, 7],
    ],
    triangle_faces: Array(8).fill(0),
    triangle_layers: Array(8).fill(0),
    warnings: [],
  };
}

function manyBowedQuads(count: number): {
  doc: Document;
  faces: Face[];
  soft: SoftMesh;
} {
  const vertices: Document["cp"]["vertices"] = [];
  const edges: Document["cp"]["edges"] = [];
  const faces: Face[] = [];
  const positions: SoftMesh["positions"] = [];
  const triangles: SoftMesh["triangles"] = [];
  const triangleFaces: number[] = [];
  const local = bowedQuad();
  for (let face = 0; face < count; face++) {
    const vertexBase = face * 4;
    const edgeBase = face * 4;
    const sourceBase = face * local.positions.length;
    const x = face * 2;
    vertices.push(
      { id: vertexBase, pos: [x, 0] },
      { id: vertexBase + 1, pos: [x + 1, 0] },
      { id: vertexBase + 2, pos: [x + 1, 1] },
      { id: vertexBase + 3, pos: [x, 1] },
    );
    edges.push(
      { id: edgeBase, v0: vertexBase, v1: vertexBase + 1, kind: "Border" },
      { id: edgeBase + 1, v0: vertexBase + 1, v1: vertexBase + 2, kind: "Border" },
      { id: edgeBase + 2, v0: vertexBase + 2, v1: vertexBase + 3, kind: "Border" },
      { id: edgeBase + 3, v0: vertexBase + 3, v1: vertexBase, kind: "Border" },
    );
    faces.push({
      id: face,
      vertices: [vertexBase, vertexBase + 1, vertexBase + 2, vertexBase + 3],
      edges: [edgeBase, edgeBase + 1, edgeBase + 2, edgeBase + 3],
    });
    positions.push(
      ...local.positions.map(
        (point): [number, number, number] => [point[0] + x, point[1], point[2]],
      ),
    );
    triangles.push(
      ...local.triangles.map(
        (triangle) =>
          triangle.map((source) => source + sourceBase) as [number, number, number],
      ),
    );
    triangleFaces.push(...Array(local.triangles.length).fill(face));
  }
  return {
    doc: {
      ...QUAD_DOC,
      cp: {
        vertices,
        edges,
        next_vertex_id: count * 4,
        next_edge_id: count * 4,
      },
    },
    faces,
    soft: {
      positions,
      triangles,
      triangle_faces: triangleFaces,
      triangle_layers: Array(triangles.length).fill(0),
      warnings: [],
    },
  };
}

/** WebGL資源を作らず、SoftContentの対応表とlive表示座標だけを本番と同じ関数で作る。 */
function displayContent(
  soft: SoftMesh,
  lifts: ReadonlyMap<number, Vec3> = new Map(),
): SoftContent {
  const layout = buildSoftLayout(soft);
  const positions = new Float32Array(layout.vertexCount * 3);
  fillSoftPositions(soft, layout, lifts, positions);
  return { layout, positions } as unknown as SoftContent;
}

function physicalSegment(
  overrides: Partial<HighlightSegment> = {},
): HighlightSegment {
  return {
    edgeId: 10,
    ownerFace: 0,
    role: "hinge",
    a: new THREE.Vector3(0, 0, 0),
    b: new THREE.Vector3(1, 0, 0),
    ...overrides,
  };
}

describe("たわみ面への強調線の正確な対応", () => {
  it("Rustと同じ正方形div=2のsource・三角形順と完全一致した候補だけを採用する", () => {
    const soft = bowedQuad();
    const content = displayContent(soft);

    const map = buildSoftHighlightMap(QUAD_DOC, [QUAD_FACE], soft, content);

    expect(map).not.toBeNull();
    expect(map?.division).toBe(2);
    expect(map?.livePositions).toBe(content.positions);
    expect(map?.materialPositions).toEqual([
      [0, 1],
      [0.5, 0.5],
      [1, 0],
      [0, 0.5],
      [0.5, 0],
      [0, 0],
      [1, 0.5],
      [0.5, 1],
      [1, 1],
    ]);
    expect(map?.trianglesByFace.get(0)?.map((triangle) => triangle.sources)).toEqual(
      soft.triangles,
    );
  });

  it("400面3200三角形でも個数恒等式でdiv=2だけを選び、完全topologyを復元する", () => {
    const fixture = manyBowedQuads(400);
    const content = displayContent(fixture.soft);

    const map = buildSoftHighlightMap(
      fixture.doc,
      fixture.faces,
      fixture.soft,
      content,
    );

    expect(fixture.soft.triangles).toHaveLength(3200);
    expect(map?.division).toBe(2);
    expect(map?.materialPositions).toHaveLength(3600);
    expect(map?.trianglesByFace.size).toBe(400);
  });

  it("物理辺を細分三角形ごとの連続片へ写し、両端と更新後の膨らみ中点を通す", () => {
    const soft = bowedQuad();
    const content = displayContent(soft);
    const map = buildSoftHighlightMap(QUAD_DOC, [QUAD_FACE], soft, content);
    expect(map).not.toBeNull();
    if (!map) return;

    // map構築後の座標更新もコピーなしで追従することを固定する。
    const midpointDisplay = map.displayByFaceSource.get(0)?.get(4);
    expect(midpointDisplay).toBeDefined();
    if (midpointDisplay === undefined) return;
    content.positions[midpointDisplay * 3 + 2] = 0.27;

    const result = projectHighlightSegmentsToSoftSurface([physicalSegment()], map);

    expect(result).toHaveLength(2);
    expect(result[0].a.toArray()).toEqual([0, 0, 0]);
    expect(result[0].b.x).toBeCloseTo(0.5);
    expect(result[0].b.y).toBeCloseTo(0);
    expect(result[0].b.z).toBeCloseTo(0.27);
    expect(result[1].a.toArray()).toEqual(result[0].b.toArray());
    expect(result[1].b.toArray()).toEqual([1, 0, 0]);
    expect(result.every((segment) => segment.surfaceProbe instanceof THREE.Vector3)).toBe(true);
    expect(result.every((segment) => Number.isFinite(segment.surfaceProbe?.z))).toBe(true);
  });

  it("edgeId・ownerFace・layerと赤いsuspect役割を全ての曲面片に保つ", () => {
    const soft = bowedQuad();
    const map = buildSoftHighlightMap(
      QUAD_DOC,
      [QUAD_FACE],
      soft,
      displayContent(soft),
    );
    const source = physicalSegment({ edgeId: 10, ownerFace: 0, layer: 4, role: "suspect" });

    const result = projectHighlightSegmentsToSoftSurface([source], map);

    expect(result.length).toBeGreaterThanOrEqual(2);
    expect(
      result.every(
        (segment) =>
          segment.edgeId === 10 &&
          segment.ownerFace === 0 &&
          segment.layer === 4 &&
          segment.role === "suspect",
      ),
    ).toBe(true);
  });

  it("ownerFaceなしはany判定を変えず、同じ入力オブジェクトをそのまま返す", () => {
    const soft = bowedQuad();
    const map = buildSoftHighlightMap(
      QUAD_DOC,
      [QUAD_FACE],
      soft,
      displayContent(soft),
    );
    const source = physicalSegment({ ownerFace: undefined, role: "reference" });

    const result = projectHighlightSegmentsToSoftSurface([source], map);

    expect(result).toHaveLength(1);
    expect(result[0]).toBe(source);
    expect(result[0].a).toBe(source.a);
    expect(result[0].b).toBe(source.b);
  });

  it("三角形indexが1つでも違えばmapを作らず、入力配列・線分参照を保つ", () => {
    const soft = bowedQuad();
    soft.triangles = soft.triangles.map((triangle) => [...triangle]);
    soft.triangles[3][1] = 4;
    const map = buildSoftHighlightMap(
      QUAD_DOC,
      [QUAD_FACE],
      soft,
      displayContent(soft),
    );
    const source = physicalSegment();
    const input = [source];

    const result = projectHighlightSegmentsToSoftSurface(input, map);

    expect(map).toBeNull();
    expect(result).toBe(input);
    expect(result[0]).toBe(source);
  });

  it("退化した物理辺と非finiteなlive座標は最近傍へ吸着せず元線へ戻す", () => {
    const soft = bowedQuad();
    const degenerateDoc: Document = {
      ...QUAD_DOC,
      cp: {
        ...QUAD_DOC.cp,
        edges: [
          ...QUAD_DOC.cp.edges,
          { id: 99, v0: 0, v1: 0, kind: "Aux" },
        ],
      },
    };
    const content = displayContent(soft);
    const map = buildSoftHighlightMap(degenerateDoc, [QUAD_FACE], soft, content);
    expect(map).not.toBeNull();
    if (!map) return;

    const degenerate = physicalSegment({ edgeId: 99 });
    const degenerateResult = projectHighlightSegmentsToSoftSurface([degenerate], map);
    expect(degenerateResult).toEqual([degenerate]);
    expect(degenerateResult[0]).toBe(degenerate);

    content.positions.fill(Number.NaN);
    const nonFinite = physicalSegment();
    const nonFiniteResult = projectHighlightSegmentsToSoftSurface([nonFinite], map);
    expect(nonFiniteResult).toEqual([nonFinite]);
    expect(nonFiniteResult[0]).toBe(nonFinite);
  });

  it("共有辺の面別copyは、同じsourceでも各面のdisplay位置とliftを使う", () => {
    const doc: Document = {
      ...QUAD_DOC,
      cp: {
        ...QUAD_DOC.cp,
        edges: [
          { id: 0, v0: 0, v1: 1, kind: "Border" },
          { id: 1, v0: 1, v1: 2, kind: "Border" },
          { id: 2, v0: 2, v1: 3, kind: "Border" },
          { id: 3, v0: 3, v1: 0, kind: "Border" },
          { id: 5, v0: 0, v1: 2, kind: "Mountain" },
        ],
      },
    };
    const faces: Face[] = [
      { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
      { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
    ];
    const soft: SoftMesh = {
      positions: [
        [0, 0, 0],
        [1, 1, 0],
        [1, 0, 0],
        [0, 1, 0],
      ],
      triangles: [
        [0, 2, 1],
        [0, 1, 3],
      ],
      triangle_faces: [0, 1],
      triangle_layers: [0, 1],
      warnings: [],
    };
    const content = displayContent(
      soft,
      new Map<number, Vec3>([
        [0, [0, 0, 0.1]],
        [1, [0, 0, 0.3]],
      ]),
    );
    const map = buildSoftHighlightMap(doc, faces, soft, content);
    expect(map?.division).toBe(1);
    const lower = physicalSegment({ edgeId: 5, ownerFace: 0 });
    const upper = physicalSegment({ edgeId: 5, ownerFace: 1 });

    const result = projectHighlightSegmentsToSoftSurface([lower, upper], map);

    expect(result).toHaveLength(2);
    expect(result[0].ownerFace).toBe(0);
    expect(result[0].a.z).toBeCloseTo(0.1);
    expect(result[0].b.z).toBeCloseTo(0.1);
    expect(result[0].surfaceProbe?.z).toBeCloseTo(0.1);
    expect(result[1].ownerFace).toBe(1);
    expect(result[1].a.z).toBeCloseTo(0.3);
    expect(result[1].b.z).toBeCloseTo(0.3);
    expect(result[1].surfaceProbe?.z).toBeCloseTo(0.3);
  });

  it("存在しない辺・面は近い曲面を探さず、それぞれ元オブジェクトを保つ", () => {
    const soft = bowedQuad();
    const map = buildSoftHighlightMap(
      QUAD_DOC,
      [QUAD_FACE],
      soft,
      displayContent(soft),
    );
    const missingEdge = physicalSegment({ edgeId: 404 });
    const missingFace = physicalSegment({ ownerFace: 404 });

    const result = projectHighlightSegmentsToSoftSurface(
      [missingEdge, missingFace],
      map,
    );

    expect(result).toEqual([missingEdge, missingFace]);
    expect(result[0]).toBe(missingEdge);
    expect(result[1]).toBe(missingFace);
  });
});
