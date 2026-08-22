// sceneBuilderのテスト。
// (1) 資源の破棄(dispose)の回帰テスト: 表示を作り替えるときに古いジオメトリと
//     マテリアルを必ず1回ずつ破棄することを、偽の資源で回数を数えて確かめる。
//     取りこぼすとGPU資源が積み上がるため、作り替えのたびに監視する。
// (2) トポロジ(三角形分割・添字・ヒンジ対応・三角形→面IDの対応表)の確認。
// (3) 立体形状の更新が「その場書き換え」で、資源を作り直さないことの確認。
// (4) 平らに畳んだときだけ層をずらして描くこと(UI-010 / SIM-004)の確認。
// (5) 強調表示が紙の表裏を無視せず、食い込み原因だけ例外にすることの確認。

import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import { Line2 } from "three/examples/jsm/lines/Line2.js";
import {
  FOCUS_HIGHLIGHT_WIDTH_PX,
  HIGHLIGHT_WIDTH_PX,
  PIN_MARK_MAX_LENGTH,
  PIN_MARK_MIN_LENGTH,
  PIN_MARK_RATIO,
  PIN_MARK_WIDTH_PX,
  SUSPECT_HIGHLIGHT_WIDTH_PX,
  withPinMarks,
  buildTopology,
  clearGroup,
  createContent,
  createHighlightGeometry,
  createHighlightLayer,
  createHighlightMaterials,
  createSupplementalEdgeLayer,
  createSoftContent,
  highlightAppearance,
  updateFrame,
  updateSoftContent,
} from "./sceneBuilder";
import { COLORS as CP_COLORS } from "../CpEditor/renderer";
import {
  LAYER_STEP_RATIO,
  MAX_STACK_RATIO,
  layerOffsets,
} from "../../lib/layerOffset";
import type { Document, Face, Frame3D } from "../../lib/types";
import { orderSurfaceOwner, ownerCodeBytes } from "./surfaceOwner";

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

describe("強調表示の太さ・深度判定", () => {
  it("画面上の太さを保ちながら3Dでは紙を貫く半径を持たない", () => {
    const geometry = createHighlightGeometry();
    try {
      geometry.computeBoundingBox();
      const size = geometry.boundingBox!.getSize(new THREE.Vector3());

      // 中心線はy方向の長さ1だけ。LineMaterialが画面方向へ太く描くため、
      // 世界座標の幅・奥行きは厳密に0で、重なり全体0.001より小さい。
      expect(size.toArray()).toEqual([0, 1, 0]);
      expect(Math.max(size.x, size.z)).toBe(0);
      expect(Math.max(size.x, size.z)).toBeLessThan(MAX_STACK_RATIO);
      // 紙の層間隔を広げて直す退行も同時に防ぐ。
      expect(LAYER_STEP_RATIO).toBe(0.0002);
      expect(MAX_STACK_RATIO).toBe(0.001);
    } finally {
      geometry.dispose();
    }
  });

  it("7種類とも世界単位でなくCSSピクセルの見える太さを使う", () => {
    const materials = createHighlightMaterials();
    try {
      expect(Object.keys(materials)).toHaveLength(7);
      expect({
        selected: materials.highlightMaterial.linewidth,
        reference: materials.referenceHighlightMaterial.linewidth,
        focus: materials.focusHighlightMaterial.linewidth,
        suspect: materials.suspectHighlightMaterial.linewidth,
        active: materials.activeHighlightMaterial.linewidth,
        pinned: materials.pinnedHighlightMaterial.linewidth,
        pinMark: materials.pinMarkMaterial.linewidth,
      }).toEqual({
        selected: HIGHLIGHT_WIDTH_PX,
        reference: HIGHLIGHT_WIDTH_PX,
        focus: FOCUS_HIGHLIGHT_WIDTH_PX,
        suspect: SUSPECT_HIGHLIGHT_WIDTH_PX,
        active: HIGHLIGHT_WIDTH_PX,
        pinned: HIGHLIGHT_WIDTH_PX,
        pinMark: PIN_MARK_WIDTH_PX,
      });
      expect(HIGHLIGHT_WIDTH_PX).toBe(4);
      expect(FOCUS_HIGHLIGHT_WIDTH_PX).toBe(6);
      expect(SUSPECT_HIGHLIGHT_WIDTH_PX).toBe(8);
      for (const material of Object.values(materials)) {
        expect(material.worldUnits).toBe(false);
        expect(material.depthFunc).toBe(THREE.LessEqualDepth);
        expect(material.depthWrite).toBe(false);
      }
    } finally {
      for (const material of Object.values(materials)) material.dispose();
    }
  });

  it("選択・参照・フォーカス・操作中・固定の折り目は紙の裏側なら隠す", () => {
    const materials = createHighlightMaterials();
    try {
      expect({
        selected: materials.highlightMaterial.depthTest,
        reference: materials.referenceHighlightMaterial.depthTest,
        focus: materials.focusHighlightMaterial.depthTest,
        active: materials.activeHighlightMaterial.depthTest,
        pinned: materials.pinnedHighlightMaterial.depthTest,
      }).toEqual({
        selected: true,
        reference: true,
        focus: true,
        active: true,
        pinned: true,
      });
    } finally {
      for (const material of Object.values(materials)) material.dispose();
    }
  });

  it("食い込み原因候補だけは紙の内側でも見つけられるよう隠さない", () => {
    const materials = createHighlightMaterials();
    try {
      expect(materials.suspectHighlightMaterial.depthTest).toBe(false);
      expect(materials.suspectHighlightMaterial.color.getHex()).toBe(0xff2038);
    } finally {
      for (const material of Object.values(materials)) material.dispose();
    }
  });

  it("7役割と省略時を実際の材質・描画順へ漏れなく対応付ける", () => {
    const materials = createHighlightMaterials();
    try {
      expect([
        highlightAppearance(materials, undefined),
        highlightAppearance(materials, "hinge"),
        highlightAppearance(materials, "reference"),
        highlightAppearance(materials, "focus"),
        highlightAppearance(materials, "pinned"),
        highlightAppearance(materials, "pinMark"),
        highlightAppearance(materials, "active"),
        highlightAppearance(materials, "suspect"),
      ]).toEqual([
        { material: materials.highlightMaterial, renderOrder: 5 },
        { material: materials.highlightMaterial, renderOrder: 5 },
        { material: materials.referenceHighlightMaterial, renderOrder: 5 },
        { material: materials.focusHighlightMaterial, renderOrder: 5 },
        // 固定の印は、いま操作している折り目(6)・食い込み(7)より下に描く
        { material: materials.pinnedHighlightMaterial, renderOrder: 5 },
        { material: materials.pinMarkMaterial, renderOrder: 5 },
        { material: materials.activeHighlightMaterial, renderOrder: 6 },
        { material: materials.suspectHighlightMaterial, renderOrder: 7 },
      ]);
    } finally {
      for (const material of Object.values(materials)) material.dispose();
    }
  });

  it("本番と同じLine2プールを6役割＋省略経路で更新・再利用・破棄する", () => {
    const layer = createHighlightLayer();
    let disposed = false;
    try {
      const roles = [
        undefined,
        "hinge",
        "reference",
        "focus",
        "pinned",
        "active",
        "suspect",
      ] as const;
      const segments = roles.map((role, index) => ({
        edgeId: index,
        a: new THREE.Vector3(index, 0, 0),
        b: new THREE.Vector3(index, index + 1, 0),
        role,
      }));

      layer.setSegments(segments);
      // 固定した折り目には中点の丸が1つ足されるので、線6本+丸1つ=7本…
      // ではなく、7役割ぶんの線(省略・hinge・reference・focus・pinned・active・
      // suspect)に丸が1つ足されて8本になる。
      expect(layer.group.children).toHaveLength(8);
      const firstPool = [...layer.group.children] as Line2[];
      const expected = [
        highlightAppearance(layer.materials, undefined),
        highlightAppearance(layer.materials, "hinge"),
        highlightAppearance(layer.materials, "reference"),
        highlightAppearance(layer.materials, "focus"),
        highlightAppearance(layer.materials, "pinned"),
        highlightAppearance(layer.materials, "active"),
        highlightAppearance(layer.materials, "suspect"),
        highlightAppearance(layer.materials, "pinMark"),
      ];
      for (let i = 0; i < firstPool.length; i++) {
        const line = firstPool[i];
        expect(line).toBeInstanceOf(Line2);
        expect(line.geometry).toBe(layer.geometry);
        expect(line.material).toBe(expected[i].material);
        expect(line.renderOrder).toBe(expected[i].renderOrder);
        expect(line.frustumCulled).toBe(false);
        expect(line.visible).toBe(true);
        if (i < roles.length) {
          expect(line.position.toArray()).toEqual([i, 0, 0]);
          expect(line.scale.toArray()).toEqual([1, i + 1, 1]);
        } else {
          // 最後の1本は固定の印。役割 "pinned" の線(x=4、長さ5)の中点にあり、
          // 世界座標の伸びは PIN_MARK_LENGTH だけ。
          const pinnedIndex = roles.indexOf("pinned");
          expect(line.position.x).toBe(pinnedIndex);
          const pinnedLength = pinnedIndex + 1;
          const markLength = Math.min(
            pinnedLength * PIN_MARK_RATIO,
            PIN_MARK_MAX_LENGTH,
          );
          expect(line.position.y).toBeCloseTo(pinnedLength / 2 - markLength / 2, 9);
          expect(line.scale.y).toBeCloseTo(markLength, 12);
        }
      }

      layer.setSegments(segments.slice(0, 2));
      expect(layer.group.children).toEqual(firstPool);
      expect(firstPool.map((line) => line.visible)).toEqual([
        true,
        true,
        false,
        false,
        false,
        false,
        false,
        false,
      ]);
      layer.setSegments(segments);
      expect(layer.group.children).toEqual(firstPool);
      expect(firstPool.every((line) => line.visible)).toBe(true);

      const geometryDispose = vi.spyOn(layer.geometry, "dispose");
      const materialDisposes = Object.values(layer.materials).map((material) =>
        vi.spyOn(material, "dispose"),
      );
      layer.dispose();
      disposed = true;
      expect(layer.group.children).toEqual([]);
      expect(geometryDispose).toHaveBeenCalledTimes(1);
      for (const dispose of materialDisposes) expect(dispose).toHaveBeenCalledTimes(1);
    } finally {
      if (!disposed) layer.dispose();
    }
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

describe("createSupplementalEdgeLayer(面境界へ入らない既存線)", () => {
  it("rigidの黒outline材質とowner tokenを共有し、owner不明の裏線候補は描かない", () => {
    const doc = makeDoc();
    const content = createContent(buildTopology(doc, FACES, HINGES), doc.display);
    const material = content.line.material as THREE.LineBasicMaterial;
    const layer = createSupplementalEdgeLayer();
    const probe = new THREE.Vector3(0.5, 0.25, 0);

    layer.setSegments(
      [
        {
          edgeId: 20,
          ownerFace: 0,
          a: new THREE.Vector3(0.2, 0.2, 0),
          b: new THREE.Vector3(0.8, 0.2, 0),
          surfaceProbe: probe,
        },
        {
          edgeId: 21,
          ownerFace: 999,
          a: new THREE.Vector3(0.2, 0.3, 0),
          b: new THREE.Vector3(0.8, 0.3, 0),
        },
        {
          edgeId: 22,
          a: new THREE.Vector3(0.2, 0.4, 0),
          b: new THREE.Vector3(0.8, 0.4, 0),
        },
      ],
      material,
      content.owner.ownerCodes,
    );

    expect(layer.group.children).toHaveLength(1);
    const line = layer.group.children[0] as THREE.LineSegments;
    expect(line.material).toBe(material);
    expect(material.userData.surfaceOwnerFilter).toBe("outline-inward");
    expect(line.renderOrder).toBe(content.line.renderOrder);
    expect(line.frustumCulled).toBe(false);
    const geometry = line.geometry;
    expect(geometry.getAttribute("position").count).toBe(2);
    expect(geometry.getAttribute("surfaceOwnerOther").count).toBe(2);
    expect(geometry.getAttribute("surfaceOwnerProbe").count).toBe(2);
    expect(geometry.getAttribute("surfaceOwnerProbe").getX(0)).toBeCloseTo(probe.x, 9);
    expect(geometry.getAttribute("surfaceOwnerProbe").getY(1)).toBeCloseTo(probe.y, 9);
    const token = geometry.getAttribute("surfaceOwnerToken");
    const code = content.owner.ownerCodes.get(0)!;
    expect(Array.from((token.array as Uint8Array).slice(0, 4))).toEqual(ownerCodeBytes(code));

    layer.dispose();
  });

  it("線分更新では借りたoutline材質を破棄せず、自分のgeometryだけを片付ける", () => {
    const doc = makeDoc();
    const content = createContent(buildTopology(doc, FACES, HINGES), doc.display);
    const material = content.line.material as THREE.LineBasicMaterial;
    const materialDispose = vi.spyOn(material, "dispose");
    const layer = createSupplementalEdgeLayer();
    const segment = {
      edgeId: 20,
      ownerFace: 0,
      a: new THREE.Vector3(0.2, 0.2, 0),
      b: new THREE.Vector3(0.8, 0.2, 0),
    };
    layer.setSegments([segment], material, content.owner.ownerCodes);
    const first = (layer.group.children[0] as THREE.LineSegments).geometry;
    const firstDispose = vi.spyOn(first, "dispose");

    layer.setSegments([segment], material, content.owner.ownerCodes);
    expect(firstDispose).toHaveBeenCalledTimes(1);
    expect(materialDispose).not.toHaveBeenCalled();
    const second = (layer.group.children[0] as THREE.LineSegments).geometry;
    const secondDispose = vi.spyOn(second, "dispose");

    layer.dispose();
    expect(secondDispose).toHaveBeenCalledTimes(1);
    expect(materialDispose).not.toHaveBeenCalled();
    expect(layer.group.children).toEqual([]);
  });
});

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
    expect(topo.lineProbeIndices).toHaveLength(topo.lineIndices.length / 2);
    for (let edge = 0; edge < topo.lineProbeIndices.length; edge++) {
      const a = topo.lineIndices[edge * 2];
      const b = topo.lineIndices[edge * 2 + 1];
      const probe = topo.lineProbeIndices[edge];
      const adjacent = Array.from({ length: topo.indices.length / 3 }, (_, triangle) =>
        topo.indices.slice(triangle * 3, triangle * 3 + 3),
      ).filter((indices) => indices.includes(a) && indices.includes(b));
      expect(adjacent).toHaveLength(1);
      expect(adjacent[0]).toContain(probe);
      expect(probe).not.toBe(a);
      expect(probe).not.toBe(b);
    }
    // 共有ヒンジは両面のコピーと所有面を残す。owner判定で見える側だけを出す。
    expect(topo.hingeSlots).toEqual([
      { edgeId: 5, faceId: 0, ia: 2, ib: 0, ip: 1 },
      { edgeId: 5, faceId: 1, ia: 3, ib: 4, ip: 5 },
    ]);
    expect(topo.vertexFaceIds).toEqual([0, 0, 0, 1, 1, 1]);
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
    // 各edge固有の内向きprobeを補間せず持たせるため、境界線だけは非indexedで
    // 2頂点ずつ複製する。座標の出所はmeshの動的positionのまま。
    const linePosition = content.line.geometry.getAttribute("position");
    expect(content.line.geometry.getIndex()).toBeNull();
    expect(linePosition).not.toBe(content.mesh.geometry.getAttribute("position"));
    expect(linePosition.count).toBe(content.topology.lineIndices.length);
    expect(content.line.geometry.getAttribute("surfaceOwnerOther").count).toBe(
      linePosition.count,
    );
    expect(content.line.geometry.getAttribute("surfaceOwnerProbe").count).toBe(
      linePosition.count,
    );
    expect(content.outline.sourcePosition).toBe(
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
          surface_rank: 0,
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

  it("立体更新後も黒outlineの端点・反対端・内向きprobeを同じsourceへ同期する", () => {
    const doc = makeDoc();
    const content = createContent(buildTopology(doc, FACES, HINGES), doc.display);
    updateFrame(content, {
      faces: [
        {
          face: 0,
          polygon: [
            [0.1, 0.2, 0.3],
            [1.1, 0.2, 0.4],
            [1.1, 1.2, 0.5],
          ],
          layer: 0,
          surface_rank: 0,
        },
        {
          face: 1,
          polygon: [
            [0.6, 0.7, 0.8],
            [1.6, 1.7, 0.9],
            [0.6, 1.7, 1.0],
          ],
          layer: 0,
          surface_rank: 0,
        },
      ],
      warnings: [],
    });

    const source = content.outline.sourcePosition;
    const endpoint = content.line.geometry.getAttribute("position");
    const other = content.line.geometry.getAttribute("surfaceOwnerOther");
    const probe = content.line.geometry.getAttribute("surfaceOwnerProbe");
    const expectSource = (
      attribute: THREE.BufferAttribute | THREE.InterleavedBufferAttribute,
      at: number,
      sourceAt: number,
    ) => {
      expect([attribute.getX(at), attribute.getY(at), attribute.getZ(at)]).toEqual([
        source.getX(sourceAt),
        source.getY(sourceAt),
        source.getZ(sourceAt),
      ]);
    };
    for (let at = 0; at < content.outline.endpointSources.length; at++) {
      expectSource(endpoint, at, content.outline.endpointSources[at]);
      expectSource(other, at, content.outline.otherSources[at]);
      expectSource(probe, at, content.outline.probeSources[at]);
    }
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
    expect(
      content.hingeSegments.map(({ edgeId, ownerFace }) => ({ edgeId, ownerFace })),
    ).toEqual([
      { edgeId: 5, ownerFace: 0 },
      { edgeId: 5, ownerFace: 1 },
    ]);
    expect(content.hingeSegments[0].a.toArray()).toEqual([1, 1, 0]);
    expect(content.hingeSegments[0].b.toArray()).toEqual([0, 0, 0]);
    expect(content.hingeSegments[0].surfaceProbe?.toArray()).toEqual([1, 0, 0]);
    expect(content.hingeSegments[1].a.toArray()).toEqual([0, 0, 0]);
    expect(content.hingeSegments[1].b.toArray()).toEqual([1, 1, 0]);
    expect(content.hingeSegments[1].surfaceProbe?.toArray()).toEqual([0, 1, 0]);
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
          surface_rank: 0,
        },
        {
          face: 1,
          polygon: [
            [0, 0, 0],
            [1, 1, 0],
            [0, 1, 0],
          ],
          layer: 1,
          surface_rank: 1,
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
          surface_rank: 1,
        },
      ],
      warnings: [],
    });
    expect(content.positions[3 * 3 + 2]).toBe(0); // 層1でも持ち上げない
    expect(content.positions[5 * 3 + 2]).toBe(0.5);
  });

  it("紙が完全に重なっても、表を向いた面が裏の色で塗りつぶされない", () => {
    // 実機で見つかった不具合の再現。角度スライダー・紙を引く操作(pose_solve)の
    // 結果は全ての面が層0なので、面を離して描くことができず深度が同値になる。
    // owner順を現在のworld法線で決めると、谷折りで裏windingの面がownerに
    // なり、別faceの表fragmentを先に捨てて裏色(白)だけを残していた。
    const doc = makeDoc();
    const content = createContent(buildTopology(doc, FACES, HINGES), doc.display);
    // 山折りを180度まで折った形: 面1が面0の上に完全に重なり、裏返っている
    updateFrame(content, {
      faces: [
        {
          face: 0,
          polygon: [
            [0, 0, 0],
            [1, 0, 0],
            [1, 1, 0],
          ],
          layer: 0,
          surface_rank: 0,
          mirrored: false,
        },
        {
          face: 1,
          polygon: [
            [0, 0, 0],
            [1, 1, 0],
            [1, 0, 0],
          ],
          layer: 0,
          surface_rank: 0,
          mirrored: true,
        },
      ],
      warnings: [],
    });

    // 層が同じなので面は離れず、深度は同値のまま(離して隠すのではなく色で決める)
    const normal = content.mesh.geometry.getAttribute("normal");
    expect(normal.getZ(0)).toBeCloseTo(1, 6); // 面0は表(+z)向き=赤で描かれる
    expect(normal.getZ(3)).toBeCloseTo(-1, 6); // 面1は裏返り=白で描かれる
    expect(content.positions[2]).toBe(content.positions[3 * 3 + 2]);
    expect([...content.owner.faceSurfaceRanks]).toEqual([[0, 0], [1, 0]]);

    const ownerFaceFrom = (z: number) => {
      const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 100);
      camera.position.set(0.5, 0.5, z);
      camera.lookAt(0.5, 0.5, 0);
      camera.updateMatrixWorld(true);
      orderSurfaceOwner(content.owner, camera);
      const index = content.owner.geometry.getIndex();
      if (!index) throw new Error("surface owner index is missing");
      return content.topology.vertexFaceIds[index.getX(index.count - 1)];
    };

    // 180°山折りの正面では+z表法線の面0を選ぶ。裏へ回しても面1の表へ
    // 乗り換えず、同じ面0をBackSide材質で見るため裏の白になる。
    expect(ownerFaceFrom(2)).toBe(0);
    expect(ownerFaceFrom(-2)).toBe(0);

    // 面の実際の深度は動かさない。表と裏が同値のときだけ、裏面のstrict lessで
    // 先に見えている表を守る。境界線は同値を通すLEQUALで面より後に描く。
    const materials = content.mesh.material as THREE.MeshLambertMaterial[];
    expect(materials[0].side).toBe(THREE.FrontSide);
    expect(materials[1].side).toBe(THREE.BackSide);
    expect(materials[0].color.toArray()).toEqual(
      doc.display.front_color.map((channel) => channel / 255),
    );
    expect(materials[1].color.toArray()).toEqual(
      doc.display.back_color.map((channel) => channel / 255),
    );
    for (const m of materials) {
      expect(m.depthTest).toBe(true);
      expect(m.depthWrite).toBe(true);
      expect(m.polygonOffset).toBe(false);
    }
    expect(materials[0].depthFunc).toBe(THREE.LessEqualDepth);
    expect(materials[1].depthFunc).toBe(THREE.LessDepth);
    expect(content.mesh.renderOrder).toBe(0);
    expect(content.line.renderOrder).toBe(1);
  });

  it("立体・折り途中で重なった面も層の上下を保って離れる", () => {
    // x=0の平面に重なった2枚(平坦判定は偽)。zへ足しても離れないため
    // 平面の法線(±x)方向へ離す
    const doc = makeDoc();
    const content = createContent(buildTopology(doc, FACES, HINGES), doc.display);
    const wall = (face: number, layer: number): Frame3D["faces"][number] => ({
      face,
      polygon: [
        [0, 0, 0],
        [0, 1, 0],
        [0, 1, 1],
      ],
      layer,
      surface_rank: layer,
    });
    updateFrame(content, { faces: [wall(0, 0), wall(1, 1)], warnings: [] });

    const step = layerOffsets(2, 1)[1];
    // 下の層(面0、頂点0〜2)は動かず、上の層(面1、頂点3〜5)だけxへ離れる
    for (let i = 0; i < 3; i++) expect(content.positions[i * 3]).toBeCloseTo(0, 6);
    for (let i = 3; i < 6; i++) {
      expect(Math.abs(content.positions[i * 3])).toBeCloseTo(step, 6);
    }
  });

  it("頂点数が合わない面(参照切れ)は前の座標のままにする", () => {
    const doc = makeDoc();
    const content = createContent(
      buildTopology(doc, FACES, HINGES),
      doc.display,
    );
    updateFrame(content, null);
    updateFrame(content, {
      faces: [{ face: 0, polygon: [[9, 9, 9]], layer: 0, surface_rank: 0 }],
      warnings: [],
    });
    expect([...content.positions.slice(0, 3)]).toEqual([0, 0, 0]);
  });
});

describe("紙のたわみの表示(SIM-012)", () => {
  /** 面0(三角形1枚)と面1(三角形1枚)が辺を共有する最小の網 */
  const SOFT = {
    positions: [
      [0, 0, 0],
      [1, 0, 0],
      [1, 1, 0],
      [0, 1, 0],
    ] as [number, number, number][],
    triangles: [
      [0, 1, 2],
      [0, 2, 3],
    ] as [number, number, number][],
    triangle_faces: [0, 1],
    triangle_layers: [0, 1],
    warnings: [],
  };

  it("表裏の色分け・境界線つきの網を作る(面ごとに頂点を分ける)", () => {
    const content = createSoftContent(SOFT, makeDoc().display);
    // 共有していた頂点0・2が面ごとに複製され、3+3=6頂点になる
    expect(content.layout.vertexCount).toBe(6);
    expect(Array.isArray(content.mesh.material)).toBe(true);
    expect(content.mesh.geometry.groups.length).toBe(2); // 表と裏
    expect(content.line.geometry.getIndex()).toBeNull();
    expect(content.line.geometry.getAttribute("position").count).toBe(3 * 2 * 2);
    expect(content.line.geometry.getAttribute("surfaceOwnerProbe").count).toBe(3 * 2 * 2);
  });

  it("補足線もsoftの黒outline材質とowner codeをそのまま共有する", () => {
    const content = createSoftContent(SOFT, makeDoc().display);
    const material = content.line.material as THREE.LineBasicMaterial;
    const layer = createSupplementalEdgeLayer();
    layer.setSegments(
      [
        {
          edgeId: 20,
          ownerFace: 1,
          a: new THREE.Vector3(0.2, 0.8, 0),
          b: new THREE.Vector3(0.8, 0.8, 0),
          surfaceProbe: new THREE.Vector3(0.5, 0.7, 0),
        },
      ],
      material,
      content.owner.ownerCodes,
    );

    const line = layer.group.children[0] as THREE.LineSegments;
    expect(line.material).toBe(material);
    expect(material.userData.surfaceOwnerFilter).toBe("outline-inward");
    const token = line.geometry.getAttribute("surfaceOwnerToken");
    expect(Array.from((token.array as Uint8Array).slice(0, 4))).toEqual(
      ownerCodeBytes(content.owner.ownerCodes.get(1)!),
    );
    layer.dispose();
  });

  it("層のずらし表示が三角形の網にも効く(重なった紙が見分けられる)", () => {
    const content = createSoftContent(SOFT, makeDoc().display);
    updateSoftContent(content, SOFT, {
      faces: [
        {
          face: 0,
          polygon: [[0, 0, 0], [1, 0, 0], [1, 1, 0]],
          layer: 0,
          surface_rank: 0,
          mirrored: false,
        },
        {
          face: 1,
          polygon: [[0, 0, 0], [1, 1, 0], [0, 1, 0]],
          layer: 1,
          surface_rank: 1,
          mirrored: true,
        },
      ],
      warnings: [],
    });
    const step = layerOffsets(2, 1)[1];
    // 面0の3頂点はz=0のまま、面1の3頂点だけ層のぶん持ち上がる
    for (let i = 0; i < 3; i++) expect(content.positions[i * 3 + 2]).toBeCloseTo(0, 6);
    for (let i = 3; i < 6; i++) {
      expect(Math.abs(content.positions[i * 3 + 2])).toBeCloseTo(step, 6);
    }
    expect([...content.owner.faceSurfaceRanks]).toEqual([[0, 0], [1, 1]]);
  });
});

describe("固定した折り目の強調(3D)", () => {
  it("2D展開図の印と同じ色にする", () => {
    const materials = createHighlightMaterials();
    try {
      // CpEditor/renderer.ts の COLORS.pinned("#1b2430")と同じ色。
      // 2Dと3Dで別の色にすると、同じものだと分からなくなる。
      expect(materials.pinnedHighlightMaterial.color.getHex()).toBe(0x1b2430);
      expect(CP_COLORS.pinned.toLowerCase()).toBe("#1b2430");
    } finally {
      for (const material of Object.values(materials)) material.dispose();
    }
  });

  it("世界座標の太さを持たない(紙を突き抜けない)", () => {
    // 強調線が円柱だったころ、半径0.006の線が厚み0.001の紙の重なりを
    // 幾何的に貫通し、裏側の折り目が手前へ突き出て見えた。固定の印でも
    // 同じことが起きないよう、画面上の太さだけを使うことを主張する。
    const materials = createHighlightMaterials();
    try {
      expect(materials.pinnedHighlightMaterial.worldUnits).toBe(false);
      const geometry = createHighlightGeometry();
      try {
        const positions = geometry.getAttribute("instanceStart").array;
        // 中心線は原点から+y方向の線分1本だけ(x/z方向の幅を持たない)
        expect([...positions]).toEqual([0, 0, 0, 0, 1, 0]);
      } finally {
        geometry.dispose();
      }
    } finally {
      for (const material of Object.values(materials)) material.dispose();
    }
  });
});

describe("固定した折り目の中点の印(3D)", () => {
  const seg = (edgeId: number, ax: number, bx: number, role?: string) => ({
    edgeId,
    a: new THREE.Vector3(ax, 0, 0),
    b: new THREE.Vector3(bx, 0, 0),
    ...(role ? { role: role as "pinned" } : {}),
    ownerFace: 3,
  });

  it("固定した折り目1本につき、中点へ丸を1つ打つ", () => {
    const out = withPinMarks([seg(5, 0, 2, "pinned")]);
    expect(out).toHaveLength(2);
    const mark = out[1];
    expect(mark.role).toBe("pinMark");
    expect(mark.edgeId).toBe(5);
    // 中点にあり、線分の長さは PIN_MARK_LENGTH
    expect(mark.a.clone().add(mark.b).multiplyScalar(0.5).x).toBeCloseTo(1, 12);
    // 印の長さは折り目(長さ2)の12%。ただし上限(0.02)で頭打ちになる
    expect(mark.a.distanceTo(mark.b)).toBeCloseTo(
      Math.min(2 * PIN_MARK_RATIO, PIN_MARK_MAX_LENGTH),
      12,
    );
    // 面の持ち主を引き継ぐ(紙の裏側なら線と同じように隠れる)
    expect(mark.ownerFace).toBe(3);
  });

  it("同じ折り目が面ごとに分かれていても、丸は1つだけ(いちばん長い線分の中点)", () => {
    const out = withPinMarks([
      seg(5, 0, 1, "pinned"),
      seg(5, 1, 4, "pinned"),
      seg(5, 4, 5, "pinned"),
    ]);
    const marks = out.filter((s) => s.role === "pinMark");
    expect(marks).toHaveLength(1);
    // いちばん長い線分(1→4)の中点
    expect(marks[0].a.clone().add(marks[0].b).multiplyScalar(0.5).x).toBeCloseTo(2.5, 12);
  });

  it("固定していない折り目には丸を打たない", () => {
    const out = withPinMarks([seg(5, 0, 2), seg(6, 0, 2, "active")]);
    expect(out.filter((s) => s.role === "pinMark")).toHaveLength(0);
    expect(out).toHaveLength(2);
  });

  it("印は折り目に沿った向きにしか伸びない(紙の面から外へ出ない)", () => {
    // 過去の事故: 強調線が**全方向に**半径0.006の円柱を持ち、それが紙の重なり
    // 全体の厚み0.001の6倍あったため、裏側の折り目が手前の紙を貫通した(§10.7.8)。
    // この印は折り目の向きにしか伸びず、面に垂直な厚みを持たない。
    const a = new THREE.Vector3(0, 0, 0.5);
    const b = new THREE.Vector3(1, 0, 0.5); // z=0.5 の面の上にある折り目
    const [, mark] = withPinMarks([
      { edgeId: 1, a, b, role: "pinned" as const },
    ]);
    // 印の両端は、折り目と同じ平面(z=0.5)から1つも外れていない
    expect(mark.a.z).toBe(0.5);
    expect(mark.b.z).toBe(0.5);
    // 折り目の向きと平行(外積が0)
    expect(
      mark.b.clone().sub(mark.a).cross(b.clone().sub(a)).length(),
    ).toBeCloseTo(0, 12);
    // 紙の厚みの値は1つも変えていない
    expect(LAYER_STEP_RATIO).toBe(0.0002);
    expect(MAX_STACK_RATIO).toBe(0.001);
  });

  it("印の長さは折り目の長さに応じて決め、上限と下限で挟む", () => {
    const long = withPinMarks([
      {
        edgeId: 1,
        a: new THREE.Vector3(0, 0, 0),
        b: new THREE.Vector3(10, 0, 0),
        role: "pinned" as const,
      },
    ])[1];
    expect(long.a.distanceTo(long.b)).toBeCloseTo(PIN_MARK_MAX_LENGTH, 12);
    const short = withPinMarks([
      {
        edgeId: 1,
        a: new THREE.Vector3(0, 0, 0),
        b: new THREE.Vector3(2e-4, 0, 0),
        role: "pinned" as const,
      },
    ])[1];
    // 32ビットの小数で向きが埋もれない下限(1e-4)まで伸ばす
    expect(short.a.distanceTo(short.b)).toBeCloseTo(PIN_MARK_MIN_LENGTH, 12);
  });

  it("印は線より太く、色は増やさない(墨色のまま)", () => {
    const materials = createHighlightMaterials();
    try {
      expect(PIN_MARK_WIDTH_PX).toBeGreaterThan(HIGHLIGHT_WIDTH_PX);
      expect(PIN_MARK_WIDTH_PX).toBeGreaterThan(SUSPECT_HIGHLIGHT_WIDTH_PX);
      expect(materials.pinMarkMaterial.color.getHex()).toBe(
        materials.pinnedHighlightMaterial.color.getHex(),
      );
      // 紙の裏側なら隠れる(手前へ無理に出さない)
      expect(materials.pinMarkMaterial.depthTest).toBe(true);
      expect(materials.pinMarkMaterial.worldUnits).toBe(false);
    } finally {
      for (const material of Object.values(materials)) material.dispose();
    }
  });

  it("本番の描画経路でも、固定した折り目には線と丸の2本が出る", () => {
    const layer = createHighlightLayer();
    try {
      layer.setSegments([seg(5, 0, 2, "pinned")]);
      const lines = layer.group.children as Line2[];
      expect(lines).toHaveLength(2);
      expect(lines[0].material).toBe(layer.materials.pinnedHighlightMaterial);
      expect(lines[1].material).toBe(layer.materials.pinMarkMaterial);
      expect(lines[1].visible).toBe(true);
      // 印は折り目の真ん中に置かれ、折り目に沿ってだけ伸びる
      const markLength = Math.min(2 * PIN_MARK_RATIO, PIN_MARK_MAX_LENGTH);
      expect(lines[1].position.x).toBeCloseTo(1 - markLength / 2, 12);
      expect(lines[1].scale.y).toBeCloseTo(markLength, 12);
    } finally {
      layer.dispose();
    }
  });
});
