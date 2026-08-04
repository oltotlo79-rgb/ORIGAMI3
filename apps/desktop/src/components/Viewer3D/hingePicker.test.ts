// 3Dビューのヒンジ拾い上げ(純関数)のテスト。
// 面の境界辺(face.edges[i])と立体の頂点(polygon[i])の対応、
// クリック位置のしきい値、重なったときの手前優先を確かめる。

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { collectHingeSegments, pickHinge, type HingeSegment } from "./hingePicker";
import type { Document, Face, Frame3D } from "../../lib/types";

/** 対角線(辺5、山折り)で2つの面に分かれた正方形 */
function makeDoc(): Document {
  return {
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
        { id: 0, v0: 0, v1: 1, kind: "Border" },
        { id: 1, v0: 1, v1: 2, kind: "Border" },
        { id: 2, v0: 2, v1: 3, kind: "Border" },
        { id: 3, v0: 3, v1: 0, kind: "Border" },
        { id: 5, v0: 0, v1: 2, kind: "Mountain" },
      ],
      next_vertex_id: 4,
      next_edge_id: 6,
    },
    sequence: [],
    display: {
      front_color: [237, 28, 36],
      back_color: [255, 255, 255],
      grid_divisions: 8,
    },
  };
}

const FACES: Face[] = [
  { id: 0, vertices: [0, 1, 2], edges: [0, 1, 5] },
  { id: 1, vertices: [0, 2, 3], edges: [5, 2, 3] },
];

/** 平らなまま(z=0)のFrame3D。面1は畳んだ位置(z)に置くこともできる */
function makeFrame(z = 0): Frame3D {
  return {
    faces: [
      {
        face: 0,
        polygon: [
          [0, 0, 0],
          [1, 0, 0],
          [1, 1, 0],
        ],
        layer: 0,
      },
      {
        face: 1,
        polygon: [
          [0, 0, 0],
          [1, 1, 0],
          [0, 1, z],
        ],
        layer: 0,
      },
    ],
    warnings: [],
  };
}

describe("collectHingeSegments", () => {
  it("折り線の線分を、面の境界辺と立体の頂点の対応どおりに取り出す", () => {
    const segments = collectHingeSegments(makeDoc(), FACES, makeFrame());
    expect(segments.map((s) => s.edgeId)).toEqual([5]);
    // 面0のedges[2]=辺5は vertices[2]→vertices[0](=(1,1)→(0,0))
    expect(segments[0].a.toArray()).toEqual([1, 1, 0]);
    expect(segments[0].b.toArray()).toEqual([0, 0, 0]);
  });

  it("輪郭や補助線は拾わず、頂点が欠けた面は無視する", () => {
    const doc = makeDoc();
    doc.cp.edges[4].kind = "Aux";
    expect(collectHingeSegments(doc, FACES, makeFrame())).toEqual([]);

    // 立体の頂点数が面の頂点数と合わない(参照切れ)場合は対応が取れない
    const broken = makeFrame();
    broken.faces[0].polygon = [
      [0, 0, 0],
      [1, 0, 0],
    ];
    broken.faces[1].polygon = [
      [0, 0, 0],
      [1, 1, 0],
    ];
    expect(collectHingeSegments(makeDoc(), FACES, broken)).toEqual([]);
  });
});

/** 原点を正面から見るカメラ(画面200×200px) */
function makeCamera(): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);
  camera.position.set(0, 0, 2);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  return camera;
}

function segment(edgeId: number, a: number[], b: number[]): HingeSegment {
  return {
    edgeId,
    a: new THREE.Vector3(a[0], a[1], a[2]),
    b: new THREE.Vector3(b[0], b[1], b[2]),
  };
}

describe("pickHinge", () => {
  const camera = makeCamera();
  // 画面中央を通る横向きの線分(z=0)
  const segments = [segment(1, [-0.5, 0, 0], [0.5, 0, 0])];

  it("しきい値の内側をクリックすると拾い、外側なら拾わない", () => {
    expect(pickHinge(segments, camera, 200, 200, 100, 104)).toBe(1);
    expect(pickHinge(segments, camera, 200, 200, 100, 130)).toBeNull();
  });

  it("線分の外側(端点より先)は距離が離れるので拾わない", () => {
    // 線分は画面のごく一部にしか映らないため、右端は端点から遠い
    expect(pickHinge(segments, camera, 200, 200, 199, 100)).toBeNull();
  });

  it("重なって見える折り線は手前(カメラに近い方)を選ぶ", () => {
    const far = segment(1, [-0.5, 0, 0], [0.5, 0, 0]);
    const near = segment(2, [-0.5, 0, 0.5], [0.5, 0, 0.5]); // カメラ側
    expect(pickHinge([far, near], camera, 200, 200, 100, 100)).toBe(2);
    // 並び順に関わらず同じ結果になる
    expect(pickHinge([near, far], camera, 200, 200, 100, 100)).toBe(2);
  });

  it("カメラの後ろにある折り線は拾わない", () => {
    const behind = [segment(3, [-0.5, 0, 5], [0.5, 0, 5])];
    expect(pickHinge(behind, camera, 200, 200, 100, 100)).toBeNull();
  });
});
