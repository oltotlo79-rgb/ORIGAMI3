// 3Dビューのトポロジと表示content。Reactやscene lifecycleには依存しない。
import * as THREE from "three";
import type {
  DisplaySettings,
  Document,
  Face,
  Frame3D,
  SoftMesh,
  Vec2,
} from "../../lib/types";
import { stackLifts, type Vec3 } from "../../lib/layerOffset";
import {
  buildSoftLayout,
  fillSoftPositions,
  softSignature,
  type SoftLayout,
} from "./softMesh";
import { paperExtent } from "../CpEditor/snap";
import type { HingeSegment } from "./hingePicker";
import {
  createSurfaceOwnerBinding,
  createSurfaceOwnerCodes,
  createSurfaceOwnerSurface,
  updateSurfaceOwnerFaceRanks,
  updateSurfaceOwnerFaceLayers,
  updateSurfaceOwnerTriangleLayers,
  type SurfaceOwnerBinding,
  type SurfaceOwnerSurface,
} from "./surfaceOwner";
import {
  createSurfaceOwnerOutlineGeometry,
  filterMaterialBySurfaceOwnerAttribute,
  filterOutlineMaterialBySurfaceOwner,
  updateSurfaceOwnerOutlineGeometry,
  type SurfaceOwnerOutlineGeometry,
} from "./surfaceOwnerShader";

/** 面の境界線の色 */
const OUTLINE_COLOR = 0x1a1a1a;
const PAPER_LONG_SIDE = 1;
/**
 * MSAA本描画だけが拾うsilhouette画素を、owner背景側から補う最小実測半径。
 * 真横fixtureでは欠けが r0=3468, r1=577, ..., r9=2, r10=0画素だった。
 * shaderは実際に紙geometryが描いたfragmentだけで動き、中心がforeign ownerなら
 * 探索前に必ずfalseなので、10px内の別の紙を許す処理ではない。
 */
const PAPER_OWNER_RADIUS_PX = 10;

// ---------------------------------------------------------------------------
// トポロジ(展開図が変わったときだけ作り直す部分)
// ---------------------------------------------------------------------------

/** 1つの面が使う頂点バッファ上の範囲 */
export interface FaceSlot {
  /** 先頭の頂点番号 */
  offset: number;
  /** 頂点数(面の多角形の頂点数と同じ) */
  count: number;
}

/** ヒンジ(折れる辺)1本に対応する頂点番号の組 */
export interface HingeSlot {
  edgeId: number;
  /** 共有辺を最初の面だけに潰さず、この表示線分が属する面を残す。 */
  faceId: number;
  ia: number;
  ib: number;
  /** 境界辺へ接する三角形の第3頂点。太い線の面内方向を決める。 */
  ip: number;
}

/** 頂点座標を除いた、形の組み立て情報 */
export interface Topology {
  /** 面ID → 頂点バッファ上の範囲 */
  slots: Map<number, FaceSlot>;
  /** 頂点の総数 */
  vertexCount: number;
  /** 三角形の添字(表裏で共用する) */
  indices: number[];
  /** 三角形の通し番号 → 面ID(Task 2-5の3D折り線描画のraycastで使う) */
  triangleFaceIds: number[];
  /** 頂点の通し番号 → 面ID(surface owner用。面ごとに頂点を複製している) */
  vertexFaceIds: number[];
  /** 境界線の添字(2つで1本) */
  lineIndices: number[];
  /** 各境界辺に接する三角形の第3頂点(内向きowner照合用)。 */
  lineProbeIndices: number[];
  /** ヒンジの線分に対応する頂点番号 */
  hingeSlots: HingeSlot[];
  /** 折る前(平ら)の頂点座標。立体形状がまだ無い面の表示に使う */
  flatPositions: Float32Array;
}

/** 面ID・2D頂点列・境界辺IDの組 */
interface FacePolygon {
  id: number;
  points: Vec2[];
  edges: number[];
}

/** 面を2D頂点列に直す。面がまだ取れていないときは紙の外形1枚として扱う */
function facePolygons(doc: Document, faces: Face[]): FacePolygon[] {
  if (faces.length === 0) {
    const [w, h] = paperExtent(doc);
    return [
      {
        id: 0,
        points: [
          [0, 0],
          [w, 0],
          [w, h],
          [0, h],
        ],
        edges: [],
      },
    ];
  }
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const out: FacePolygon[] = [];
  for (const f of faces) {
    const points: Vec2[] = [];
    for (const id of f.vertices) {
      const p = pos.get(id);
      if (!p) break;
      points.push(p);
    }
    // 参照切れで頂点が欠けた面は境界との対応が取れないので描かない
    if (points.length === f.vertices.length) {
      out.push({ id: f.id, points, edges: f.edges });
    }
  }
  return out;
}

/** 三角形を表(+z)向き=反時計回りに揃える */
function orient(
  points: Vec2[],
  a: number,
  b: number,
  c: number,
): [number, number, number] {
  const cross =
    (points[b][0] - points[a][0]) * (points[c][1] - points[a][1]) -
    (points[b][1] - points[a][1]) * (points[c][0] - points[a][0]);
  return cross < 0 ? [a, c, b] : [a, b, c];
}

/**
 * 2Dの多角形を三角形に分ける。スリット(行き止まりの折り線)でできる凹んだ面にも
 * 対応するため耳切り法(ShapeUtils.triangulateShape)を使う。
 * 分割できない形(自己接触など)は扇形分割で代用する。
 */
export function triangulate(points: Vec2[]): [number, number, number][] {
  const contour = points.map((p) => new THREE.Vector2(p[0], p[1]));
  let raw: number[][];
  try {
    raw = THREE.ShapeUtils.triangulateShape(contour, []);
  } catch {
    raw = [];
  }
  const out: [number, number, number][] = [];
  for (const t of raw) {
    if (t.length !== 3) continue;
    if (t.some((i) => i < 0 || i >= points.length)) continue;
    out.push(orient(points, t[0], t[1], t[2]));
  }
  if (out.length === 0) {
    for (let i = 1; i + 1 < points.length; i++) out.push(orient(points, 0, i, i + 1));
  }
  return out;
}

/** 展開図から、頂点座標を除いた組み立て情報を作る */
export function buildTopology(
  doc: Document,
  faces: Face[],
  hinges: ReadonlySet<number>,
): Topology {
  const slots = new Map<number, FaceSlot>();
  const indices: number[] = [];
  const triangleFaceIds: number[] = [];
  const vertexFaceIds: number[] = [];
  const lineIndices: number[] = [];
  const lineProbeIndices: number[] = [];
  const hingeSlots: HingeSlot[] = [];
  const flat: number[] = [];
  let offset = 0;

  for (const poly of facePolygons(doc, faces)) {
    const n = poly.points.length;
    if (n < 3) continue;
    slots.set(poly.id, { offset, count: n });
    for (const p of poly.points) {
      flat.push(p[0], p[1], 0);
      vertexFaceIds.push(poly.id);
    }
    const triangles = triangulate(poly.points);
    for (const t of triangles) {
      indices.push(offset + t[0], offset + t[1], offset + t[2]);
      triangleFaceIds.push(poly.id);
    }
    for (let i = 0; i < n; i++) {
      const j = (i + 1) % n;
      lineIndices.push(offset + i, offset + j);
      const adjacent = triangles.find(
        (triangle) => triangle.includes(i) && triangle.includes(j),
      );
      // 正常な単純多角形では境界辺に接する三角形は必ず1枚。対応不能なら
      // probeを端点へ退化させ、shaderをcenter-onlyへfail closedさせる。
      const probe = adjacent?.find((vertex) => vertex !== i && vertex !== j) ?? i;
      lineProbeIndices.push(offset + probe);
      // 境界辺ID列は頂点列と同順(edges[i] が vertices[i]→vertices[i+1])
      const edgeId = poly.edges[i];
      if (edgeId !== undefined && hinges.has(edgeId)) {
        // 共有ヒンジは両面のコピーを残す。surface ownerが、画素ごとに実際に
        // 見えている面へ属するコピーだけを通す。
        hingeSlots.push({
          edgeId,
          faceId: poly.id,
          ia: offset + i,
          ib: offset + j,
          ip: offset + probe,
        });
      }
    }
    offset += n;
  }
  return {
    slots,
    vertexCount: offset,
    indices,
    triangleFaceIds,
    vertexFaceIds,
    lineIndices,
    lineProbeIndices,
    hingeSlots,
    flatPositions: new Float32Array(flat),
  };
}

// ---------------------------------------------------------------------------
// 表示物(トポロジから1度だけ作り、以後は座標だけ書き換える)
// ---------------------------------------------------------------------------

export interface Viewer3DContent {
  topology: Topology;
  /** 面(表裏2組の描画指定を持つ1つのジオメトリ) */
  mesh: THREE.Mesh;
  /** 面の境界線(edge固有probe用に複製し、meshの動的座標から同期する) */
  line: THREE.LineSegments;
  /** edge固有probeを持つ非indexed境界線geometryと、その更新対応。 */
  outline: SurfaceOwnerOutlineGeometry;
  /** 頂点座標。updateFrameで書き換える実体 */
  positions: Float32Array;
  /** 現在の立体形状におけるヒンジの線分(選択の当たり判定・強調表示に使う) */
  hingeSegments: HingeSegment[];
  /** 紙面と線を同じ画素所有者へ絞る前処理の表示物。 */
  owner: SurfaceOwnerSurface;
}

/**
 * 表示中の立体が実際に占める範囲(頂点座標そのものから求める)。
 *
 * 「視点を戻す」は展開図の大きさ・中心ではなく、必ずこの実測の範囲を基準にする。
 * 折る・技法で頂点は展開図の(0,0)〜(紙の幅,紙の高さ)から離れた場所へ動くため、
 * 展開図の大きさを基準にすると立体の一部が画面の外へ出ることがある。
 * 頂点が1つも無いとき(面が無い等)は空のBox3を返す(呼び出し側でフォールバックする)。
 */
export function contentBoundingBox(content: Viewer3DContent): THREE.Box3 {
  const box = new THREE.Box3();
  const positions = content.positions;
  const v = new THREE.Vector3();
  for (let i = 0; i + 2 < positions.length; i += 3) {
    box.expandByPoint(v.set(positions[i], positions[i + 1], positions[i + 2]));
  }
  return box;
}

/** 0〜255のRGBをThree.jsの色へ */
function toColor(rgb: [number, number, number]): THREE.Color {
  return new THREE.Color(rgb[0] / 255, rgb[1] / 255, rgb[2] / 255);
}

/**
 * 面のマテリアル。紙面の実際の深度をそのまま書き込む。
 *
 * 面を奥へずらすpolygon offsetは、0.0002しかない層間隔を打ち消し、裏の強調線を
 * 露出させ得るので使わない。同じ深度で表と裏が重なったときだけ、後から描く裏面を
 * strict lessにして、先に見えている表面を塗りつぶさない。
 */
function faceMaterial(
  rgb: [number, number, number],
  side: THREE.Side,
  ownerBinding?: SurfaceOwnerBinding,
) {
  const material = new THREE.MeshLambertMaterial({
    color: toColor(rgb),
    side,
    polygonOffset: false,
    depthFunc: side === THREE.BackSide ? THREE.LessDepth : THREE.LessEqualDepth,
  });
  if (ownerBinding) {
    filterMaterialBySurfaceOwnerAttribute(material, ownerBinding, PAPER_OWNER_RADIUS_PX);
  }
  return material;
}

/**
 * トポロジから面・境界線の表示物を作る。頂点座標は折る前(平ら)で始め、
 * updateFrameで立体形状に差し替える。
 * 表と裏は同じ三角形を2回描いて塗り分ける(裏はBackSide指定でThree.jsが法線を
 * 反転するため、裏返った面も暗くならない)。
 */
export function createContent(
  topology: Topology,
  display: DisplaySettings,
  ownerBinding: SurfaceOwnerBinding = createSurfaceOwnerBinding(),
  ownerCodes: ReadonlyMap<number, number> = createSurfaceOwnerCodes(
    topology.triangleFaceIds,
  ),
): Viewer3DContent {
  const positions = new Float32Array(topology.flatPositions);
  const position = new THREE.BufferAttribute(positions, 3);
  position.setUsage(THREE.DynamicDrawUsage);
  const normal = new THREE.BufferAttribute(
    new Float32Array(topology.vertexCount * 3),
    3,
  );
  normal.setUsage(THREE.DynamicDrawUsage);

  const faceGeometry = new THREE.BufferGeometry();
  faceGeometry.setAttribute("position", position);
  faceGeometry.setAttribute("normal", normal);
  faceGeometry.setIndex(topology.indices);
  faceGeometry.addGroup(0, topology.indices.length, 0); // 表
  faceGeometry.addGroup(0, topology.indices.length, 1); // 裏

  const owner = createSurfaceOwnerSurface({
    position,
    vertexFaces: topology.vertexFaceIds,
    indices: topology.indices,
    triangleFaces: topology.triangleFaceIds,
    ownerCodes,
  });
  // createSurfaceOwnerSurfaceが作った正規化RGBA8属性を通常描画にも共有する。
  faceGeometry.setAttribute(
    "surfaceOwnerToken",
    owner.geometry.getAttribute("surfaceOwnerToken"),
  );
  const outline = createSurfaceOwnerOutlineGeometry({
    sourcePosition: position,
    sourceToken: owner.geometry.getAttribute("surfaceOwnerToken") as THREE.BufferAttribute,
    lineIndices: topology.lineIndices,
    lineProbeIndices: topology.lineProbeIndices,
  });

  const mesh = new THREE.Mesh(faceGeometry, [
    faceMaterial(display.front_color, THREE.FrontSide, ownerBinding),
    faceMaterial(display.back_color, THREE.BackSide, ownerBinding),
  ]);
  const outlineMaterial = new THREE.LineBasicMaterial({ color: OUTLINE_COLOR });
  filterOutlineMaterialBySurfaceOwner(outlineMaterial, ownerBinding);
  const line = new THREE.LineSegments(outline.geometry, outlineMaterial);
  // 面と同じ深度ではLEQUALで境界線を残す。面の深度自体は動かさない。
  line.renderOrder = 1;
  // 形が毎フレーム変わるので、範囲の当たり判定による省略は行わない
  mesh.frustumCulled = false;
  line.frustumCulled = false;

  return {
    topology,
    mesh,
    line,
    outline,
    positions,
    hingeSegments: topology.hingeSlots.map((s) => ({
      edgeId: s.edgeId,
      ownerFace: s.faceId,
      a: new THREE.Vector3(),
      b: new THREE.Vector3(),
      surfaceProbe: new THREE.Vector3(),
    })),
    owner,
  };
}

/**
 * 立体形状(Frame3D)を反映する。頂点座標をその場で書き換え、法線を計算し直す
 * だけで、ジオメトリ・マテリアルの生成も破棄もしない。
 * frameがnull(まだ折っていない)なら展開図どおりの平らな形に戻す。
 *
 * 同じ平面に重なった面のかたまりには、層ごとに微小な高さを足して重なりを見せる
 * (UI-010 / SIM-004)。平らに畳んだ状態に限らず、折り途中・立体・引っ張った状態でも
 * その平面の法線方向へ離すので、完全に重なった面が深度の取り合いで裏の色に
 * なることがない。層番号が同じ面は離さない(1枚の紙がばらけないように)。
 * 足す高さは表示専用で、frameの値そのものは変えない。
 */
export function updateFrame(content: Viewer3DContent, frame: Frame3D | null): void {
  const { positions, topology } = content;
  const faceLayers = new Map<number, number>();
  const faceSurfaceRanks = new Map<number, number>();
  if (frame === null) {
    positions.set(topology.flatPositions);
  } else {
    // 重なった面はその平面の法線方向へ離す(向きが+zとは限らないのでベクトル)
    const lifts = stackLifts(frame, PAPER_LONG_SIDE);
    for (let i = 0; i < frame.faces.length; i++) {
      const f = frame.faces[i];
      faceLayers.set(f.face, f.layer);
      faceSurfaceRanks.set(f.face, f.surface_rank ?? 0);
      const slot = topology.slots.get(f.face);
      // 頂点数が合わない面は対応が取れないので前の座標のままにする
      // (展開図を編集した直後など、立体形状の計算が届くまでは平らのまま)
      if (!slot || slot.count !== f.polygon.length) continue;
      const lift = lifts[i];
      let k = slot.offset * 3;
      for (const p of f.polygon) {
        positions[k++] = p[0] + lift[0];
        positions[k++] = p[1] + lift[1];
        positions[k++] = p[2] + lift[2];
      }
    }
  }
  const geometry = content.mesh.geometry;
  geometry.getAttribute("position").needsUpdate = true;
  updateSurfaceOwnerOutlineGeometry(content.outline);
  geometry.computeVertexNormals();
  geometry.computeBoundingSphere();
  updateSurfaceOwnerFaceLayers(content.owner, faceLayers);
  updateSurfaceOwnerFaceRanks(content.owner, faceSurfaceRanks);

  for (let i = 0; i < topology.hingeSlots.length; i++) {
    const slot = topology.hingeSlots[i];
    const seg = content.hingeSegments[i];
    seg.a.fromArray(positions, slot.ia * 3);
    seg.b.fromArray(positions, slot.ib * 3);
    seg.surfaceProbe?.fromArray(positions, slot.ip * 3);
    seg.layer = faceLayers.get(slot.faceId) ?? 0;
  }
}

// ---------------------------------------------------------------------------
// 紙のたわみ(SIM-012)。面ごとの多角形ではなく細かい三角形の網を描く
// ---------------------------------------------------------------------------

/** たわみの表示物。表裏の色分け・境界線は面の表示と同じ作りにそろえる */
export interface SoftContent {
  /** 網の形が同じかを見分ける文字列(同じなら座標だけ書き換える) */
  signature: string;
  layout: SoftLayout;
  mesh: THREE.Mesh;
  line: THREE.LineSegments;
  outline: SurfaceOwnerOutlineGeometry;
  positions: Float32Array;
  owner: SurfaceOwnerSurface;
}

/** たわみの網から表示物を作る(座標は updateSoftContent で入れる) */
export function createSoftContent(
  soft: SoftMesh,
  display: DisplaySettings,
  ownerBinding: SurfaceOwnerBinding = createSurfaceOwnerBinding(),
  ownerCodes?: ReadonlyMap<number, number>,
): SoftContent {
  const layout = buildSoftLayout(soft);
  const positions = new Float32Array(layout.vertexCount * 3);
  const position = new THREE.BufferAttribute(positions, 3);
  position.setUsage(THREE.DynamicDrawUsage);
  const normal = new THREE.BufferAttribute(
    new Float32Array(layout.vertexCount * 3),
    3,
  );
  normal.setUsage(THREE.DynamicDrawUsage);

  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", position);
  geometry.setAttribute("normal", normal);
  geometry.setIndex(layout.indices);
  geometry.addGroup(0, layout.indices.length, 0); // 表
  geometry.addGroup(0, layout.indices.length, 1); // 裏

  const codes = ownerCodes ?? createSurfaceOwnerCodes(layout.triangleFaceIds);
  const owner = createSurfaceOwnerSurface({
    position,
    vertexFaces: layout.faceOf,
    indices: layout.indices,
    triangleFaces: layout.triangleFaceIds,
    triangleLayers: layout.triangleLayers,
    ownerCodes: codes,
  });
  geometry.setAttribute(
    "surfaceOwnerToken",
    owner.geometry.getAttribute("surfaceOwnerToken"),
  );
  const outline = createSurfaceOwnerOutlineGeometry({
    sourcePosition: position,
    sourceToken: owner.geometry.getAttribute("surfaceOwnerToken") as THREE.BufferAttribute,
    lineIndices: layout.lineIndices,
    lineProbeIndices: layout.lineProbeIndices,
  });

  const mesh = new THREE.Mesh(geometry, [
    faceMaterial(display.front_color, THREE.FrontSide, ownerBinding),
    faceMaterial(display.back_color, THREE.BackSide, ownerBinding),
  ]);
  const outlineMaterial = new THREE.LineBasicMaterial({ color: OUTLINE_COLOR });
  filterOutlineMaterialBySurfaceOwner(outlineMaterial, ownerBinding);
  const line = new THREE.LineSegments(outline.geometry, outlineMaterial);
  line.renderOrder = 1;
  mesh.frustumCulled = false;
  line.frustumCulled = false;
  return { signature: softSignature(soft), layout, mesh, line, outline, positions, owner };
}

/**
 * たわみの網の座標を反映する。層のずらし表示(UI-010 / SIM-004)は剛体折りの
 * 結果から面ごとに求め、その面に属する三角形の頂点へ同じだけ足す。
 */
export function updateSoftContent(
  content: SoftContent,
  soft: SoftMesh,
  frame: Frame3D | null,
): void {
  const lifts = new Map<number, Vec3>();
  const faceSurfaceRanks = new Map<number, number>();
  if (frame) {
    const values = stackLifts(frame, PAPER_LONG_SIDE);
    for (let i = 0; i < frame.faces.length; i++) {
      const face = frame.faces[i];
      lifts.set(face.face, values[i]);
      faceSurfaceRanks.set(face.face, face.surface_rank ?? 0);
    }
  }
  fillSoftPositions(soft, content.layout, lifts, content.positions);
  updateSurfaceOwnerTriangleLayers(
    content.owner,
    content.layout.triangleSources.map((triangle) => soft.triangle_layers[triangle] ?? 0),
  );
  updateSurfaceOwnerFaceRanks(content.owner, faceSurfaceRanks);
  const geometry = content.mesh.geometry;
  geometry.getAttribute("position").needsUpdate = true;
  updateSurfaceOwnerOutlineGeometry(content.outline);
  geometry.computeVertexNormals();
  geometry.computeBoundingSphere();
}


