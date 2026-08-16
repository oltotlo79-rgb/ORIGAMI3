// 3D→展開図の逆写像の検査。
// 「順写像で3Dへ写した点を逆写像で戻すと元に返る」ことと、
// 立体姿勢(どの面もz=0に乗っていない)でも点の候補が0件にならないことを見る。

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import type { Document, Face, Frame3D, Vec2 } from "../../lib/types";
import { facePlacement, mapPoint, unmapPoint } from "./edgeHighlight";
import { alignVertexCandidates } from "../../lib/alignPick";
import { foldLayers } from "./foldDraw";
import {
  buildCpFaceIndex,
  cpPointCandidates,
  pickCpFromPixel,
  placementOf,
} from "./cpPick3d";
import { buildTopology, createContent, updateFrame } from "./sceneBuilder";

/** 正方形を中央の縦線で2枚に分けた展開図。 */
const DOC: Document = {
  schema_version: 1,
  paper: { width_mm: 150, height_mm: 150 },
  cp: {
    vertices: [
      { id: 0, pos: [0, 0] },
      { id: 1, pos: [0.5, 0] },
      { id: 2, pos: [1, 0] },
      { id: 3, pos: [1, 1] },
      { id: 4, pos: [0.5, 1] },
      { id: 5, pos: [0, 1] },
    ],
    edges: [
      { id: 0, v0: 0, v1: 1, kind: "Border" },
      { id: 1, v0: 1, v1: 2, kind: "Border" },
      { id: 2, v0: 2, v1: 3, kind: "Border" },
      { id: 3, v0: 3, v1: 4, kind: "Border" },
      { id: 4, v0: 4, v1: 5, kind: "Border" },
      { id: 5, v0: 5, v1: 0, kind: "Border" },
      { id: 6, v0: 1, v1: 4, kind: "Valley" },
    ],
    next_vertex_id: 6,
    next_edge_id: 7,
  },
  sequence: [],
  display: { front_color: [230, 90, 60], back_color: [245, 245, 245], grid_divisions: 8 },
};

const FACES: Face[] = [
  { id: 0, vertices: [0, 1, 4, 5], edges: [0, 6, 4, 5] },
  { id: 1, vertices: [1, 2, 3, 4], edges: [1, 2, 3, 6] },
];

/**
 * どの面もz=0に乗っていない立体姿勢。
 * 左半分は傾いた板、右半分は真上へ立てた板。
 */
const TILTED_FRAME: Frame3D = {
  faces: [
    {
      face: 0,
      polygon: [
        [0, 0, 0.3],
        [0.5, 0, 0],
        [0.5, 1, 0],
        [0, 1, 0.3],
      ],
      layer: 0,
      surface_rank: 0,
    },
    {
      face: 1,
      polygon: [
        [0.5, 0, 0],
        [0.5, 0, 0.5],
        [0.5, 1, 0.5],
        [0.5, 1, 0],
      ],
      layer: 1,
      surface_rank: 1,
    },
  ],
  warnings: [],
};

/** 展開図と立体形状から、3Dビューと同じ頂点バッファを作る。 */
function contentFor(frame: Frame3D | null) {
  const content = createContent(buildTopology(DOC, FACES, new Set([6])), DOC.display);
  updateFrame(content, frame);
  return content;
}

describe("展開図と3Dを結ぶ逆写像", () => {
  it("順写像で3Dへ写した点は、逆写像で元の展開図座標へ戻る(立体姿勢)", () => {
    const content = contentFor(TILTED_FRAME);
    const index = buildCpFaceIndex(DOC, FACES);
    const placement = placementOf(index, 0, content.topology.slots, content.positions);
    expect(placement).not.toBeNull();
    if (!placement) return;
    const samples: Vec2[] = [
      [0, 0],
      [0.5, 0],
      [0.25, 0.5],
      [0.1, 0.9],
      [0.4, 0.2],
    ];
    for (const p of samples) {
      const q = mapPoint(placement, p);
      expect(q).not.toBeNull();
      if (!q) continue;
      // 傾いた面なのでzは0でない。zを捨てる作りなら、この検査は通らない。
      const back = unmapPoint(placement, q);
      expect(back).not.toBeNull();
      expect(back?.[0]).toBeCloseTo(p[0], 5);
      expect(back?.[1]).toBeCloseTo(p[1], 5);
    }
    // 左半分は傾いているので、面の上の点は必ず高さを持つ
    expect(Math.abs(mapPoint(placement, [0, 0.5])?.[2] ?? 0)).toBeGreaterThan(1e-6);
  });

  it("面の外の点も、面の平面へ下ろした足として展開図座標に直る", () => {
    const content = contentFor(TILTED_FRAME);
    const index = buildCpFaceIndex(DOC, FACES);
    const placement = placementOf(index, 1, content.topology.slots, content.positions);
    if (!placement) throw new Error("面1の位置が取れない");
    const on = mapPoint(placement, [0.75, 0.5]);
    if (!on) throw new Error("順写像に失敗");
    // 面(x=0.5の垂直面)から法線方向へ0.2だけ浮かせても、同じ面内座標へ戻る
    const lifted: [number, number, number] = [on[0] + 0.2, on[1], on[2]];
    const back = unmapPoint(placement, lifted);
    expect(back?.[0]).toBeCloseTo(0.75, 5);
    expect(back?.[1]).toBeCloseTo(0.5, 5);
  });

  it("立体姿勢でも点の候補が0件にならない(畳み平面の層だけを見る作りと比べる)", () => {
    const content = contentFor(TILTED_FRAME);
    const index = buildCpFaceIndex(DOC, FACES);
    const candidates = cpPointCandidates(
      index,
      content.topology.slots,
      content.positions,
    );
    // 面2枚 × 角4つ = 8件。面の共有点は面ごとに1件ずつ数える
    expect(candidates).toHaveLength(8);
    expect(candidates.every((one) => Number.isFinite(one.world[2]))).toBe(true);

    // 比較: 高さのある面を捨てる作り(畳み平面の層)では候補が0件になる
    const flatOnly = alignVertexCandidates(foldLayers(TILTED_FRAME, DOC, FACES));
    expect(flatOnly).toHaveLength(0);
  });

  it("展開図の頂点を面ごとに振り分ける(面の内側に落ちている点も含む)", () => {
    const withInner: Document = {
      ...DOC,
      cp: {
        ...DOC.cp,
        vertices: [...DOC.cp.vertices, { id: 6, pos: [0.25, 0.25] }],
        next_vertex_id: 7,
      },
    };
    const index = buildCpFaceIndex(withInner, FACES);
    expect(index.vertices.get(0)?.map((v) => v.id).sort()).toEqual([0, 1, 4, 5, 6]);
    expect(index.vertices.get(1)?.map((v) => v.id).sort()).toEqual([1, 2, 3, 4]);
  });

  it("立体姿勢の画素から、その面が持つ展開図の頂点を拾える", () => {
    const content = contentFor(TILTED_FRAME);
    const index = buildCpFaceIndex(DOC, FACES);
    // 立てた面も傾いた面も見える斜めの視点
    const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
    camera.position.set(-1.4, -1.6, 1.6);
    camera.lookAt(0.4, 0.5, 0.2);
    camera.updateMatrixWorld(true);
    camera.updateProjectionMatrix();
    const surface = {
      mesh: content.mesh,
      triangleFaceIds: content.topology.triangleFaceIds,
      triangleLayers: content.owner.triangleLayers,
      faceSurfaceRanks: content.owner.faceSurfaceRanks,
    };
    const size = 400;
    const project = (world: [number, number, number]) => {
      const ndc = new THREE.Vector3(...world).project(camera);
      return { x: ((ndc.x + 1) / 2) * size, y: ((1 - ndc.y) / 2) * size };
    };

    let picked = 0;
    const tried: { vertexId: number; world: [number, number, number] }[] = [
      { vertexId: 0, world: [0, 0, 0.3] },
      { vertexId: 5, world: [0, 1, 0.3] },
      { vertexId: 2, world: [0.5, 0, 0.5] },
      { vertexId: 3, world: [0.5, 1, 0.5] },
    ];
    for (const one of tried) {
      const at = project(one.world);
      const hit = pickCpFromPixel({
        index,
        slots: content.topology.slots,
        positions: content.positions,
        surface,
        camera,
        widthPx: size,
        heightPx: size,
        x: at.x,
        y: at.y,
      });
      if (hit?.vertexId === one.vertexId) picked += 1;
    }
    expect(picked).toBe(tried.length);
  });

  it("紙の角のすぐ外を押しても、その角の点を拾える(展開図区画と同じ拾い方)", () => {
    const content = contentFor(null);
    const index = buildCpFaceIndex(DOC, FACES);
    const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
    camera.position.set(0.5, 0.5, 2);
    camera.lookAt(0.5, 0.5, 0);
    camera.updateMatrixWorld(true);
    camera.updateProjectionMatrix();
    const surface = {
      mesh: content.mesh,
      triangleFaceIds: content.topology.triangleFaceIds,
      triangleLayers: content.owner.triangleLayers,
      faceSurfaceRanks: content.owner.faceSurfaceRanks,
    };
    const size = 400;
    const corner = new THREE.Vector3(0, 0, 0).project(camera);
    const at = { x: ((corner.x + 1) / 2) * size, y: ((1 - corner.y) / 2) * size };
    const hit = pickCpFromPixel({
      index,
      slots: content.topology.slots,
      positions: content.positions,
      surface,
      camera,
      widthPx: size,
      heightPx: size,
      // 紙の外へ5pxはみ出した位置
      x: at.x - 5,
      y: at.y + 5,
    });
    expect(hit?.vertexId).toBe(0);
    expect(hit?.onPaper).toBe(false);
    expect(hit?.cp).toEqual([0, 0]);
  });

  it("潰れた面では面内座標を返さない(勝手な値を作らない)", () => {
    const content = contentFor({
      faces: [
        {
          face: 0,
          polygon: [
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
          ],
          layer: 0,
          surface_rank: 0,
        },
      ],
      warnings: [],
    });
    const vertexPositions = new Map(DOC.cp.vertices.map((v) => [v.id, v.pos]));
    const placement = facePlacement(
      FACES[0],
      vertexPositions,
      content.topology.slots,
      content.positions,
    );
    if (!placement) throw new Error("面の位置が取れない");
    expect(unmapPoint(placement, [0, 0, 0])).toBeNull();
  });
});
