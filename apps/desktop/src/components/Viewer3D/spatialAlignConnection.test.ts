// Viewerの3D pickを、raw world解・同index材料解・3D強調線のままstoreへ渡す境界。
// 非平坦点をlegacy z=0 gateで落とさず、global XYへfallbackしない。

import { describe, expect, it } from "vitest";
import type { AlignDraft } from "../../store/appStore";
import type {
  SpatialAlignTarget,
  SpatialFoldTarget,
} from "../../lib/spatialAlignTypes";
import type { Document, Face, Frame3D, Vec2 } from "../../lib/types";
import type { FacePlacement } from "./edgeHighlight";
import type { CpFaceIndex, CpPick3D } from "./cpPick3d";
import {
  alignPointPickFromCp,
  foldedEvidenceOnSelectedFace,
  materialAlignLineTarget,
  type AlignPick,
} from "./viewerPicking";
import { buildSpatialAlignPickResult } from "./viewerPointer";
import { spatialAlignHighlightSegments } from "./viewerHighlight";

const VERTICAL: FacePlacement = {
  faceId: 17,
  polygon: [
    [0.5, 0],
    [1, 0],
    [1, 1],
    [0.5, 1],
  ],
  p0: [0.5, 0],
  e1: [0.5, 0],
  e2: [0, 1],
  q0: [2, 0, 0],
  f1: [0, 0, 0.5],
  f2: [0, 1, 0],
  det: 0.5,
};

const PLANE = { point: [2, 0, 0] as [number, number, number], normal: [1, 0, 0] as [number, number, number] };
const POINT_TARGET: SpatialAlignTarget = {
  kind: "point",
  world: [2, 0.25, 0.25],
  supportPlanes: [PLANE],
  foldedPoint: null,
};
const LINE_TARGET: SpatialAlignTarget = {
  kind: "line",
  aWorld: [2, 0.125, 0.125],
  bWorld: [2, 0.875, 0.125],
  supportPlanes: [PLANE],
  foldedLine: null,
};

function draftWithFirstPoint(): AlignDraft {
  return {
    mode: "pointPerpendicularLine",
    picks: [{ kind: "point", p: [0.75, 0.25] }],
    cpPicks: [{ kind: "vertex", id: 7 }],
    spatialPicks: [POINT_TARGET],
    solutions: [],
    solutionIndex: 0,
    reason: null,
  };
}

describe("Viewer spatial align接続", () => {
  it("raw z=0証拠は選んだowner面だけを読み、別layer・Face ID順で補わない", () => {
    const doc: Document = {
      schema_version: 1,
      paper: { width_mm: 150, height_mm: 150 },
      cp: {
        vertices: [
          { id: 0, pos: [0, 0] },
          { id: 1, pos: [0.5, 0] },
          { id: 2, pos: [0.5, 1] },
          { id: 3, pos: [0, 1] },
          { id: 4, pos: [1, 0] },
          { id: 5, pos: [1, 1] },
        ],
        edges: [],
        next_vertex_id: 6,
        next_edge_id: 0,
      },
      sequence: [],
      display: {
        front_color: [237, 28, 36],
        back_color: [255, 255, 255],
        grid_divisions: 8,
      },
    };
    const faces: Face[] = [
      { id: 91, vertices: [0, 1, 2, 3], edges: [0, 1, 2, 3] },
      { id: 3, vertices: [1, 4, 5, 2], edges: [4, 5, 6, 1] },
    ];
    const frame: Frame3D = {
      faces: [
        {
          face: 91,
          layer: 9,
          polygon: [[0, 0, 0], [0.5, 0, 0], [0.5, 1, 0], [0, 1, 0]],
        },
        {
          face: 3,
          layer: 1,
          polygon: [
            [0.5, 0, 0.01],
            [1, 0, 0.01],
            [1, 1, 0.01],
            [0.5, 1, 0.01],
          ],
        },
      ],
      warnings: [],
    };
    const shared: Vec2[] = [[0.5, 0], [0.5, 1]];

    expect(foldedEvidenceOnSelectedFace(doc, faces, frame, 91, shared)).toEqual(
      shared,
    );
    expect(foldedEvidenceOnSelectedFace(doc, faces, frame, 3, shared)).toBeNull();
    expect(foldedEvidenceOnSelectedFace(doc, faces, frame, null, shared)).toBeNull();
    expect(
      foldedEvidenceOnSelectedFace(
        doc,
        faces,
        {
          ...frame,
          faces: [
            ...frame.faces,
            { face: 91, layer: 10, polygon: [[0, 0, 0], [0.5, 0, 0], [0, 1, 0]] },
          ],
        },
        91,
        shared,
      ),
    ).toBeNull();
  });

  it("垂直面のCpPick3DはfoldedAlignPointが無くても材料点とworld点を保つ", () => {
    const pick: CpPick3D = {
      faceId: 17,
      cp: [0.75, 0.25],
      world: [2, 0.25, 0.25],
      vertexId: 7,
      edgeId: null,
      onPaper: true,
    };
    const actual = alignPointPickFromCp(pick, null, [VERTICAL]);

    expect(actual).toMatchObject({
      target: { kind: "point", p: [0.75, 0.25] },
      cursor: [0.75, 0.25],
      cpPick: { kind: "vertex", id: 7 },
      spatialTarget: POINT_TARGET,
      spatialCursorWorld: [2, 0.25, 0.25],
    });
  });

  it("3D線の説明用2D値はworld XYでなく同じCP edgeの材料端点にする", () => {
    const index = {
      edges: new Map([
        [17, [{ id: 31, a: [0.625, 0.125] as Vec2, b: [0.625, 0.875] as Vec2 }]],
      ]),
    } as unknown as CpFaceIndex;

    expect(materialAlignLineTarget(index, 31)).toEqual({
      kind: "line",
      a: [0.625, 0.125],
      b: [0.625, 0.875],
    });
  });

  it("完成pickでraw world解と材料解を同じindexの第4引数へ作る", () => {
    const pressed: AlignPick = {
      target: { kind: "line", a: [0.625, 0.125], b: [0.625, 0.875] },
      cursor: [0.625, 0.5],
      cpPick: { kind: "edge", id: 31 },
      spatialTarget: LINE_TARGET,
      spatialCursorWorld: [2, 0.5, 0.125],
    };
    const result = buildSpatialAlignPickResult(draftWithFirstPoint(), pressed, [VERTICAL]);

    expect(result.target).toEqual(LINE_TARGET);
    expect(result.solutions.length).toBeGreaterThan(0);
    expect(result.materialSolutions).toHaveLength(result.solutions.length);
    result.solutions.forEach((solution, index) => {
      expect(solution).not.toBeNull();
      expect(result.materialSolutions?.[index]).not.toBeNull();
    });
    expect(result.reason).toBeNull();
  });

  it("非共面の完成pickはglobal XY解で補わず空配列にする", () => {
    const pressed: AlignPick = {
      target: { kind: "line", a: [0, 0], b: [1, 0] },
      cursor: [0, 0],
      cpPick: { kind: "edge", id: 31 },
      spatialTarget: {
        ...LINE_TARGET,
        aWorld: [2.25, 0.125, 0.125],
        bWorld: [2.25, 0.875, 0.125],
        supportPlanes: [{ point: [2.25, 0, 0], normal: [1, 0, 0] }],
      },
      spatialCursorWorld: [2.25, 0.5, 0.125],
    };
    const result = buildSpatialAlignPickResult(draftWithFirstPoint(), pressed, [VERTICAL]);

    expect(result.solutions).toEqual([]);
    expect(result.materialSolutions).toEqual([]);
    expect(result.reason).not.toBeNull();
  });

  it("spatial pickと解線はz=0へ落とさず同じ垂直面のworld強調線にする", () => {
    const foldTarget: SpatialFoldTarget = {
      lineWorld: [[2, 0.25, 0] as [number, number, number], [2, 0.25, 0.5] as [number, number, number]],
      keepWorldForMovingSide: { left: [2, 0.75, 0.25] as [number, number, number], right: [2, 0.1, 0.25] as [number, number, number] },
      foldedPlane: null,
      sideForFirstPick: { automatic: "left", initial: "left" },
    };
    const segments = spatialAlignHighlightSegments(
      [POINT_TARGET, LINE_TARGET],
      foldTarget,
    );

    expect(segments.length).toBeGreaterThanOrEqual(4);
    expect(
      segments.some(
        (segment) =>
          segment.a.toArray().join(",") === "2,0.25,0" &&
          segment.b.toArray().join(",") === "2,0.25,0.5",
      ),
    ).toBe(true);
    expect(segments.every((segment) => segment.a.x === 2 && segment.b.x === 2)).toBe(true);
    expect(spatialAlignHighlightSegments([], null)).toEqual([]);
  });

  it("複数支持面の点markは解線が一意に示す共通面だけを使い、配列順で選ばない", () => {
    const ambiguousPoint: SpatialAlignTarget = {
      ...POINT_TARGET,
      supportPlanes: [
        { point: [0, 0.25, 0], normal: [0, 1, 0] },
        PLANE,
      ],
    };
    const foldTarget: SpatialFoldTarget = {
      lineWorld: [[2, 0.1, 0.1], [2, 0.9, 0.1]],
      keepWorldForMovingSide: { left: [2, 0.5, 0.3], right: null },
      foldedPlane: null,
      sideForFirstPick: { automatic: null, initial: "right" },
    };

    expect(spatialAlignHighlightSegments([ambiguousPoint], null)).toEqual([]);
    const segments = spatialAlignHighlightSegments([ambiguousPoint], foldTarget);
    expect(segments).toHaveLength(3);
    expect(segments.slice(0, 2).every((segment) => segment.a.x === 2 && segment.b.x === 2)).toBe(true);
  });
});
