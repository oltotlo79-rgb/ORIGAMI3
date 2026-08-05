// sceneBuilderのテスト。
// (1) 資源の破棄(dispose)の回帰テスト: 表示を作り替えるときに古いジオメトリと
//     マテリアルを必ず1回ずつ破棄することを、偽の資源で回数を数えて確かめる。
//     取りこぼすとGPU資源が積み上がるため、作り替えのたびに監視する。
// (2) トポロジ(三角形分割・添字・ヒンジ対応・三角形→面IDの対応表)の確認。
// (3) 立体形状の更新が「その場書き換え」で、資源を作り直さないことの確認。
// (4) 平らに畳んだときだけ層をずらして描くこと(UI-010 / SIM-004)の確認。

import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import {
  buildTopology,
  clearGroup,
  createContent,
  updateFrame,
} from "./sceneBuilder";
import { layerOffsets } from "../../lib/layerOffset";
import type { Document, Face, Frame3D } from "../../lib/types";

/** dispose回数を数える偽の資源(Three.jsの実体は使わない) */
function fake() {
  // Meshの生成時にmorphAttributesを読むので最低限の形だけ持たせる
  return { dispose: vi.fn(), morphAttributes: {} };
}

/** 偽の資源をThree.jsの型として渡すための変換 */
function asGeometry(f: ReturnType<typeof fake>): THREE.BufferGeometry {
  return f as unknown as THREE.BufferGeometry;
}
function asMaterial(f: ReturnType<typeof fake>): THREE.Material {
  return f as unknown as THREE.Material;
}

describe("clearGroup(資源の破棄)", () => {
  it("面と線のジオメトリ・マテリアルを1回ずつ破棄して入れ物を空にする", () => {
    const group = new THREE.Group();
    const meshGeometry = fake();
    const meshMaterial = fake();
    const lineGeometry = fake();
    const lineMaterial = fake();
    group.add(
      new THREE.Mesh(asGeometry(meshGeometry), asMaterial(meshMaterial)),
      new THREE.LineSegments(asGeometry(lineGeometry), asMaterial(lineMaterial)),
    );

    clearGroup(group);

    expect(group.children).toEqual([]);
    for (const f of [meshGeometry, meshMaterial, lineGeometry, lineMaterial]) {
      expect(f.dispose).toHaveBeenCalledTimes(1);
    }
  });

  it("マテリアルが配列(表裏の塗り分け)でも全て破棄する", () => {
    const group = new THREE.Group();
    const geometry = fake();
    const front = fake();
    const back = fake();
    group.add(
      new THREE.Mesh(asGeometry(geometry), [asMaterial(front), asMaterial(back)]),
    );

    clearGroup(group);

    expect(group.children).toEqual([]);
    expect(geometry.dispose).toHaveBeenCalledTimes(1);
    expect(front.dispose).toHaveBeenCalledTimes(1);
    expect(back.dispose).toHaveBeenCalledTimes(1);
  });

  it("破棄の対象でない子(照明など)は取り除くだけで壊さない", () => {
    const group = new THREE.Group();
    const light = new THREE.AmbientLight(0xffffff, 1);
    const disposeSpy = vi.spyOn(light, "dispose");
    group.add(light);

    clearGroup(group);

    expect(group.children).toEqual([]);
    expect(disposeSpy).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------

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

const HINGES = new Set([5]);

/** 三角形の2Dでの符号付き面積(正なら反時計回り=表向き) */
function signedArea(
  positions: Float32Array,
  a: number,
  b: number,
  c: number,
): number {
  const p = (i: number): [number, number] => [positions[i * 3], positions[i * 3 + 1]];
  const [ax, ay] = p(a);
  const [bx, by] = p(b);
  const [cx, cy] = p(c);
  return ((bx - ax) * (cy - ay) - (by - ay) * (cx - ax)) / 2;
}

describe("buildTopology(展開図から作る組み立て情報)", () => {
  it("面ごとに頂点範囲・三角形・境界線・ヒンジ対応を作る", () => {
    const topo = buildTopology(makeDoc(), FACES, HINGES);

    expect(topo.vertexCount).toBe(6);
    expect(topo.slots.get(0)).toEqual({ offset: 0, count: 3 });
    expect(topo.slots.get(1)).toEqual({ offset: 3, count: 3 });
    // 三角形2枚(面ごとに1枚)と、その三角形がどの面か
    expect(topo.indices).toHaveLength(6);
    expect(topo.triangleFaceIds).toEqual([0, 1]);
    // 境界線は面の辺の数だけ(3+3本 = 添字12個)
    expect(topo.lineIndices).toHaveLength(12);
    // 面0のedges[2]=辺5は vertices[2]→vertices[0]
    expect(topo.hingeSlots).toEqual([{ edgeId: 5, ia: 2, ib: 0 }]);
  });

  it("折り線でない辺はヒンジにしない", () => {
    const topo = buildTopology(makeDoc(), FACES, new Set<number>());
    expect(topo.hingeSlots).toEqual([]);
  });

  it("頂点が欠けた面(参照切れ)は描かない", () => {
    const doc = makeDoc();
    doc.cp.vertices = doc.cp.vertices.filter((v) => v.id !== 3); // 面1が欠ける
    const topo = buildTopology(doc, FACES, HINGES);
    expect([...topo.slots.keys()]).toEqual([0]);
    expect(topo.triangleFaceIds).toEqual([0]);
  });

  it("面がまだ取れていないときは紙の外形1枚として組み立てる", () => {
    const topo = buildTopology(makeDoc(), [], HINGES);
    expect(topo.vertexCount).toBe(4);
    expect(topo.slots.get(0)).toEqual({ offset: 0, count: 4 });
    expect(topo.triangleFaceIds).toEqual([0, 0]);
    expect(topo.hingeSlots).toEqual([]);
  });

  it("凹んだ面(スリットでできるL字)を裏返さずに過不足なく分割する", () => {
    // L字。扇形分割だと余分な面積(4)になる並び順から始めている
    const doc = makeDoc();
    doc.cp.vertices = [
      { id: 0, pos: [2, 1] },
      { id: 1, pos: [1, 1] },
      { id: 2, pos: [1, 2] },
      { id: 3, pos: [0, 2] },
      { id: 4, pos: [0, 0] },
      { id: 5, pos: [2, 0] },
    ];
    const face: Face = {
      id: 7,
      vertices: [0, 1, 2, 3, 4, 5],
      edges: [10, 11, 12, 13, 14, 15],
    };
    const topo = buildTopology(doc, [face], new Set<number>());
    const content = createContent(topo, doc.display);
    updateFrame(content, null);

    expect(topo.indices).toHaveLength((6 - 2) * 3); // 三角形は 頂点数-2 枚
    let total = 0;
    for (let i = 0; i < topo.indices.length; i += 3) {
      const area = signedArea(
        content.positions,
        topo.indices[i],
        topo.indices[i + 1],
        topo.indices[i + 2],
      );
      expect(area).toBeGreaterThan(0); // 全て表(+z)向き
      total += area;
    }
    expect(total).toBeCloseTo(3, 9); // L字の面積(扇形分割なら4になる)
  });
});

describe("createContent / updateFrame(形の更新)", () => {
  it("表と裏を1つのジオメトリの2組の描画指定で塗り分ける", () => {
    const doc = makeDoc();
    const content = createContent(buildTopology(doc, FACES, HINGES), doc.display);
    const groups = content.mesh.geometry.groups;
    expect(groups).toHaveLength(2);
    expect(groups.map((g) => g.materialIndex)).toEqual([0, 1]);
    expect(Array.isArray(content.mesh.material)).toBe(true);
    // 境界線は面と同じ頂点座標を共有する(二重に更新しない)
    expect(content.line.geometry.getAttribute("position")).toBe(
      content.mesh.geometry.getAttribute("position"),
    );
  });

  it("立体形状の反映は同じバッファの書き換えだけで、資源を作り直さない", () => {
    const doc = makeDoc();
    const topo = buildTopology(doc, FACES, HINGES);
    const content = createContent(topo, doc.display);
    const positions = content.positions;
    const geometry = content.mesh.geometry;
    const attribute = geometry.getAttribute("position");
    const normal = geometry.getAttribute("normal");

    updateFrame(content, null);
    const folded: Frame3D = {
      faces: [
        {
          face: 1,
          polygon: [
            [0, 0, 0],
            [1, 1, 0],
            [0, 1, 0.5],
          ],
          layer: 0,
        },
      ],
      warnings: [],
    };
    updateFrame(content, folded);

    // 何度更新しても同じ入れ物のまま
    expect(content.positions).toBe(positions);
    expect(content.mesh.geometry).toBe(geometry);
    expect(geometry.getAttribute("position")).toBe(attribute);
    expect(geometry.getAttribute("normal")).toBe(normal);
    // 面1(頂点3〜5)の3点目だけがz=0.5に動いている
    expect([...positions.slice(15, 18)]).toEqual([0, 1, 0.5]);
    expect([...positions.slice(0, 3)]).toEqual([0, 0, 0]); // 面0は平らのまま
  });

  it("平らな面の法線は表(+z)を向き、ヒンジ線分も立体の座標から更新される", () => {
    const doc = makeDoc();
    const content = createContent(
      buildTopology(doc, FACES, HINGES),
      doc.display,
    );
    updateFrame(content, null);

    const normal = content.mesh.geometry.getAttribute("normal");
    expect(normal.getZ(0)).toBeCloseTo(1, 6);
    expect(normal.getZ(5)).toBeCloseTo(1, 6);
    expect(content.hingeSegments).toHaveLength(1);
    expect(content.hingeSegments[0].edgeId).toBe(5);
    expect(content.hingeSegments[0].a.toArray()).toEqual([1, 1, 0]);
    expect(content.hingeSegments[0].b.toArray()).toEqual([0, 0, 0]);
  });

  it("平らに畳んだ状態では層ごとに高さを付けて重なりを見せる(表示専用)", () => {
    const doc = makeDoc();
    const content = createContent(buildTopology(doc, FACES, HINGES), doc.display);
    const flat: Frame3D = {
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
            [0, 1, 0],
          ],
          layer: 1,
        },
      ],
      warnings: [],
    };
    updateFrame(content, flat);

    const step = layerOffsets(2, 1)[1];
    expect(step).toBeGreaterThan(0);
    // 下の層(面0)はz=0のまま、上の層(面1)だけ持ち上がる
    for (let i = 0; i < 3; i++) expect(content.positions[i * 3 + 2]).toBe(0);
    // 頂点座標は32bit実数なので比較は6桁で足りる
    for (let i = 3; i < 6; i++) expect(content.positions[i * 3 + 2]).toBeCloseTo(step, 6);
    // 元のFrame3Dの値は書き換えない
    expect(flat.faces[1].polygon[0][2]).toBe(0);
  });

  it("折り途中(高さのある形)には層のずらしを掛けない", () => {
    const doc = makeDoc();
    const content = createContent(buildTopology(doc, FACES, HINGES), doc.display);
    updateFrame(content, {
      faces: [
        {
          face: 1,
          polygon: [
            [0, 0, 0],
            [1, 1, 0],
            [0, 1, 0.5],
          ],
          layer: 1,
        },
      ],
      warnings: [],
    });
    expect(content.positions[3 * 3 + 2]).toBe(0); // 層1でも持ち上げない
    expect(content.positions[5 * 3 + 2]).toBe(0.5);
  });

  it("頂点数が合わない面(参照切れ)は前の座標のままにする", () => {
    const doc = makeDoc();
    const content = createContent(
      buildTopology(doc, FACES, HINGES),
      doc.display,
    );
    updateFrame(content, null);
    updateFrame(content, {
      faces: [{ face: 0, polygon: [[9, 9, 9]], layer: 0 }],
      warnings: [],
    });
    expect([...content.positions.slice(0, 3)]).toEqual([0, 0, 0]);
  });
});
