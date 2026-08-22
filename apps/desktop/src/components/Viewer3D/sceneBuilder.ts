// 3Dビューのシーン構築(Three.js)。Reactの外側でThree.jsの資源を管理する。
// 座標系: 展開図の(x, y)がそのまま3Dの(x, y)、紙の法線が+z(表side)。
//
// 手順再生(毎フレームの形の更新)に耐えるため、トポロジとジオメトリを分ける:
//   - 展開図(doc/faces)が変わったときだけ buildTopology + createContent で
//     三角形分割・添字・境界線・ヒンジ対応・三角形index→面IDの対応表を作り直す
//   - 立体形状(frame3d)が変わったときは updateFrame で頂点座標を上書きし
//     法線を計算し直すだけ(ジオメトリ・マテリアルの生成も破棄もしない)
// 表と裏は1つのジオメトリを2組の描画指定(addGroup)+マテリアル配列で塗り分ける。

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { Line2 } from "three/examples/jsm/lines/Line2.js";
import { LineGeometry } from "three/examples/jsm/lines/LineGeometry.js";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
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
import {
  cameraQuaternionLookingAt,
  orbitCameraOffset,
  transportCameraScreenUp,
} from "./viewCube";
import type { HingeSegment, PaperPickSurface } from "./hingePicker";
import {
  createSurfaceOwnerBinding,
  createSurfaceOwnerCodes,
  createSurfaceOwnerSurface,
  disposeSurfaceOwnerSurface,
  orderSurfaceOwner,
  ownerCodeBytes,
  ownerCodeVector,
  updateSurfaceOwnerFaceRanks,
  updateSurfaceOwnerFaceLayers,
  updateSurfaceOwnerTriangleLayers,
  type SurfaceOwnerBinding,
  type SurfaceOwnerSurface,
} from "./surfaceOwner";
import {
  createSurfaceOwnerOutlineGeometry,
  createSurfaceOwnerPassResources,
  disposeSurfaceOwnerPassResources,
  filterLineMaterialBySurfaceOwner,
  filterMaterialBySurfaceOwnerAttribute,
  filterOutlineMaterialBySurfaceOwner,
  resizeSurfaceOwnerPassResources,
  setLineSurfaceOwner,
  updateSurfaceOwnerOutlineGeometry,
  type FilteredLineMaterial,
  type SurfaceOwnerOutlineGeometry,
} from "./surfaceOwnerShader";

/** 面の境界線の色 */
const OUTLINE_COLOR = 0x1a1a1a;
/** 選択中ヒンジの強調色(黄色) */
const HIGHLIGHT_COLOR = 0xffd400;
/** 選択中だが折る操作の対象ではない縁・補助線などの強調色(水色) */
const REFERENCE_HIGHLIGHT_COLOR = 0x40cfff;
/** 複数スライダーのうち指している1本の強調色(POPのコーラル) */
const FOCUS_HIGHLIGHT_COLOR = 0xed5c70;
/** 補正後にも残る食い込みの原因候補。選択の黄色・水色と明確に分ける。 */
const SUSPECT_HIGHLIGHT_COLOR = 0xff2038;
/** いま利用者が角度を固定して動かしている折り目(水色)。 */
const ACTIVE_HIGHLIGHT_COLOR = 0x40cfff;
/**
 * 利用者が角度を固定した折り目(墨色)。
 *
 * 2D展開図の印(`CpEditor/renderer.ts` の `COLORS.pinned` = #1b2430)と同じ色にし、
 * 2Dと3Dで同じものだと分かるようにする。黄・水色・コーラル・赤のどれとも
 * 明度で見分けられるよう、色相を足さず濃い墨色にする。
 */
const PINNED_HIGHLIGHT_COLOR = 0x1b2430;
/** 固定の印(丸)の色。線と同じ墨色にして、色は増やさない。 */
const PIN_MARK_COLOR = PINNED_HIGHLIGHT_COLOR;
/** 折った結果の下見(実行前プレビュー)の色。動く紙と分かるよう青系にする */
const PREVIEW_COLOR = 0x2f8fff;
/** 下見の透け具合(下の紙が見える程度) */
const PREVIEW_OPACITY = 0.45;
/** CSSが読み込まれない単体テスト環境で使うPOPテーマの背景色。 */
const DEFAULT_BACKGROUND_COLOR = "#cfcbc2";

/** App.cssで選択中テーマの3D背景色を読む。 */
export function canvas3dBackgroundColor(canvas: HTMLCanvasElement): string {
  if (typeof getComputedStyle !== "function") return DEFAULT_BACKGROUND_COLOR;
  return (
    getComputedStyle(canvas).getPropertyValue("--color-canvas-3d").trim() ||
    DEFAULT_BACKGROUND_COLOR
  );
}
/**
 * 選択中ヒンジの画面上の太さ(CSS px)。
 *
 * 層間隔の1/4以下の円柱にすると半径は0.00005以下となり、400pxの既定表示では
 * 直径が約0.03pxで見えない。そこで3Dの半径を持たない画面上4 CSS pxの線へ替える。
 * ANGLE/D3D11の原寸オフスクリーン描画でも紙面上で連続して見えることを目視済み。
 * 紙の裏側は深度と画素単位のsurface owner判定で隠す。
 */
export const HIGHLIGHT_WIDTH_PX = 4;
/** 指している1本は通常線より見分けやすくする。 */
export const FOCUS_HIGHLIGHT_WIDTH_PX = 6;
/** 食い込み原因は警告として最も目立たせる。 */
export const SUSPECT_HIGHLIGHT_WIDTH_PX = 8;
/**
 * 固定した折り目の中点に打つ印の大きさ(CSS px)。
 *
 * 折り目の線(4px)や輪郭の線と見分けるには、線より明らかに太い丸が要る。
 * 実機で普通の大きさ(拡大なし)にして目で見て決めた: 濃い線だけでは輪郭の線と
 * 見分けにくかったため、2D展開図の印(直径12px)とそろえた16pxにする。
 */
export const PIN_MARK_WIDTH_PX = 16;
/**
 * 印にする「折り目の真ん中の太い部分」の長さ(折り目の長さに対する割合)。
 *
 * 印は**折り目そのものの一部を太く描いたもの**である。折り目に沿った向きにしか
 * 伸びないので、紙の面から外へは出ない(=紙の重なりを貫通できない)。
 *
 * | 量 | 値 | 向き |
 * |---|---:|---|
 * | 印の伸び(折り目に沿う) | 折り目の長さの **12%**(下限 1e-4・上限 0.02) | **紙の面の中**。面から外へ出ない |
 * | 印の太さ | **画面上で16px**(`PIN_MARK_WIDTH_PX`) | 世界座標の厚みを持たない |
 * | 紙の重なり全体の厚み `MAX_STACK_RATIO` | 0.001 | 紙の面に**垂直** |
 * | 層と層の間隔 `LAYER_STEP_RATIO` | 0.0002 | 紙の面に**垂直** |
 * | 事故を起こした円柱の強調線の半径(過去) | 0.006 | **全方向**(だから紙を貫通した) |
 *
 * 過去の事故(§10.7.8)は、強調線が**全方向に**半径0.006の円柱を持ち、
 * それが紙の重なり全体の厚み0.001の6倍あったために起きた。
 * この印は**面に垂直な厚みを1つも持たない**ので、同じことは起きない。
 * **紙の厚み(`LAYER_STEP_RATIO` / `MAX_STACK_RATIO`)は1つも変えていない。**
 */
export const PIN_MARK_RATIO = 0.12;
/**
 * 印の最小の長さ(世界座標)。
 *
 * 描画は32ビットの小数で行われる(相対精度 約1.2e-7)。長さが小さすぎると
 * 線の向きが数値の誤差に埋もれて印が消える。実機で 1e-6 にしたところ
 * **画面に何も出なかった**ため、桁を上げて 1e-4(誤差の約1000倍)にした。
 */
export const PIN_MARK_MIN_LENGTH = 1e-4;
/** 印の最大の長さ(世界座標)。長い折り目でも印が線全体を覆わないようにする。 */
export const PIN_MARK_MAX_LENGTH = 0.02;
/** カメラ画角(度) */
const CAMERA_FOV = 45;
/** 初期カメラの向き(紙の中心から見た方向。斜め上=手前上から見下ろす) */
export const CAMERA_DIR = new THREE.Vector3(0.35, -0.85, 0.95).normalize();
/**
 * 直す前の「視点を戻す」が使っていた距離の係数。
 *
 * 紙の長辺/(2 tan(画角/2)) にこれを掛けた距離へカメラを置いていた。縦の画角しか
 * 見ていないので、区画が縦長になると紙が左右へはみ出した(実測: 区画200×600 CSS pxで
 * 左へ136px・右へ189px)。いまはこの式を「直す前より紙を小さくしない」下限としてだけ使う。
 */
const LEGACY_CAMERA_MARGIN = 1.35;
/** 強調表示の伸ばす向き(回転計算に使う) */
const AXIS_Y = new THREE.Vector3(0, 1, 0);
/** 紙の長辺(展開図は長辺=1.0の正規化座標。層のずらし量の基準にする) */
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
function triangulate(points: Vec2[]): [number, number, number][] {
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

// ---------------------------------------------------------------------------
// 視点のドラッグ回転
// ---------------------------------------------------------------------------

/** 注視点からカメラへ向かうずれと、そのときの画面の上向き。 */
export interface CameraOrbitPose {
  /** 注視点からカメラへ向かうずれ。長さは注視点までの距離。 */
  offset: THREE.Vector3;
  /** 画面の上向き(世界座標の単位ベクトル)。視線と直角を保つ。 */
  screenUp: THREE.Vector3;
}

/** カメラの向きが定まらないほど注視点に近いときの下限。 */
const MIN_ORBIT_RADIUS = 1e-9;
/**
 * 1段で回してよいドラッグ量の上限(画面高さに対する割合)。
 *
 * 回す量は「画面高さいっぱいで1回転(360度)」なので、1/8は45度にあたる。
 * orbitCameraOffset は上下の角を0〜180度へ収めるため、いまの向き(常に90度)から
 * 一度に90度を超えて動かすと頭打ちになる。素早いドラッグで1回の通知に
 * 大きな移動量がまとまっても頭打ちにしないよう、45度ずつに分けて回す。
 */
const MAX_ORBIT_STEP_RATIO = 1 / 8;

/** 現在の姿勢から読み取る画面の上向き。 */
export function cameraScreenUp(camera: THREE.Camera): THREE.Vector3 {
  return AXIS_Y.clone().applyQuaternion(camera.quaternion).normalize();
}

/**
 * ドラッグ量だけ視点を回した後のずれと画面の上向き。
 *
 * 回す量の決め方は視点立方体のドラッグと同じ viewCube.ts の orbitCameraOffset を、
 * 画面の上向きの持ち運びも同じ transportCameraScreenUp を使う。
 * 回し方を増やさないため、この関数以外に回転の計算を置かない。
 *
 * 違いは「どの枠で回すか」だけにした。世界の上下を軸にすると、真上・真下が
 * 可動域の端になり、そこで止まる(実測: 「視点を戻す」直後の向きからは
 * 下向きへ49.98度しか回らず、上向きへ130.02度で行き止まりになった)。
 * そこで、いまの画面の上向きが枠の上になるように移してから回す。
 * カメラは常に枠の赤道上(上下角90度)にいるので、どちらへ何周しても端に届かない。
 */
export function rotateCameraByDrag(
  offset: THREE.Vector3,
  screenUp: THREE.Vector3,
  dragX: number,
  dragY: number,
  canvasHeight: number,
): CameraOrbitPose {
  const height = Math.max(canvasHeight, 1);
  const limit = height * MAX_ORBIT_STEP_RATIO;
  const steps = Math.max(
    1,
    Math.ceil(Math.max(Math.abs(dragX), Math.abs(dragY)) / limit),
  );
  let currentOffset = offset.clone();
  let currentUp = screenUp.clone().normalize();
  for (let i = 0; i < steps; i += 1) {
    // 画面の上向きを枠の上へそろえる回転。枠を戻せば世界の向きに戻る。
    const toFrame = new THREE.Quaternion().setFromUnitVectors(currentUp, AXIS_Y);
    const fromFrame = toFrame.clone().invert();
    const nextOffset = orbitCameraOffset(
      currentOffset.clone().applyQuaternion(toFrame),
      dragX / steps,
      dragY / steps,
      height,
    ).applyQuaternion(fromFrame);
    currentUp = transportCameraScreenUp(currentOffset, nextOffset, currentUp);
    currentOffset = nextOffset;
    // 積み重ねた丸めで視線と直角からずれないよう、毎段そろえ直す。
    const forward = currentOffset.clone().normalize();
    const orthogonal = currentUp.clone().projectOnPlane(forward);
    if (orthogonal.lengthSq() > MIN_ORBIT_RADIUS ** 2) {
      currentUp = orthogonal.normalize();
    }
  }
  return { offset: currentOffset, screenUp: currentUp };
}

/**
 * 注視点を中心に、ドラッグ量だけカメラを回す。
 * 注視点と距離は変えないので、寄る・平行移動には影響しない。
 */
export function applyCameraDragRotation(
  camera: THREE.PerspectiveCamera,
  target: THREE.Vector3,
  dragX: number,
  dragY: number,
  canvasHeight: number,
): void {
  const offset = camera.position.clone().sub(target);
  if (offset.lengthSq() <= MIN_ORBIT_RADIUS ** 2) return;
  const pose = rotateCameraByDrag(
    offset,
    cameraScreenUp(camera),
    dragX,
    dragY,
    canvasHeight,
  );
  camera.position.copy(target).add(pose.offset);
  // 上向きを姿勢に合わせて持ち歩くと、寄る・平行移動のあとの
  // OrbitControls の向き直し(lookAt)でも画面の傾きが失われない。
  camera.up.copy(pose.screenUp);
  camera.quaternion.copy(
    cameraQuaternionLookingAt(camera.position, target, pose.screenUp),
  );
  camera.updateMatrixWorld(true);
}

/** OrbitControls のボタン割り当て(ツールごとに setDrawMode が入れ替える)。 */
export interface ViewRotationMouseButtons {
  LEFT?: THREE.MOUSE | null;
  MIDDLE?: THREE.MOUSE | null;
  RIGHT?: THREE.MOUSE | null;
}

/**
 * 押したボタンが視点回転を始めるかどうか。
 * OrbitControls の判定(回転ボタン+修飾キーなしで回転、
 * 平行移動ボタン+修飾キーで回転)をそのまま写す。
 */
export function viewRotationStarts(
  buttons: ViewRotationMouseButtons,
  button: number,
  panModifierHeld: boolean,
): boolean {
  const action =
    button === 0
      ? buttons.LEFT
      : button === 1
        ? buttons.MIDDLE
        : button === 2
          ? buttons.RIGHT
          : null;
  if (action === THREE.MOUSE.ROTATE) return !panModifierHeld;
  if (action === THREE.MOUSE.PAN) return panModifierHeld;
  return false;
}

// ---------------------------------------------------------------------------
// 紙を3D区画へ収める視点合わせ
// ---------------------------------------------------------------------------

/** 画面上の四角形(3D区画の左上を原点としたCSS px)。 */
export interface ScreenBounds {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

/** 視点合わせの結果。カメラの距離と、区画へ当てはめる投影の枠。 */
export interface PaperFraming {
  /** 紙の中心からカメラまでの距離 */
  distance: number;
  /** camera.setViewOffset へ渡す仮想の枠の大きさ */
  fullWidth: number;
  fullHeight: number;
  /** 仮想の枠の中で3D区画が占める位置 */
  offsetX: number;
  offsetY: number;
  /** このとき紙の四隅が画面上で作る四角形 */
  bounds: ScreenBounds;
}

/** 紙と3D区画の縁との間に必ず残す隙間(CSS px)。狭い区画では区画の5%まで詰める。 */
const VIEW_EDGE_PADDING_PX = 8;
/** 案内の札の下端から、さらに空けたい隙間(CSS px)。 */
const HINT_CLEARANCE_PX = 8;
/**
 * 案内の札の下から紙を出すためだけに許す縮小の下限(直す前の紙の高さに対する割合)。
 *
 * 実測(説明書を撮ったときの区画605×439 CSS px): 札の下端107pxより下へ紙の上端を
 * 逃がすには、紙の高さを326→316px(3.1%減)まで詰める必要があった。0.95を下限にすると
 * この区画では札を避けられる。札が区画の4割を占める605×261では、いくら詰めても
 * 避けられないので、この判定で「避けない」側に倒れ、紙は直す前の大きさのまま残る。
 */
const HINT_AVOID_MIN_HEIGHT_RATIO = 0.95;
/** 画角の半分の正接。距離と画面上の大きさを結ぶ係数。 */
const HALF_FOV_TAN = Math.tan((CAMERA_FOV * Math.PI) / 360);

/** 紙の四隅を、カメラの右向き・上向き・視線方向の成分へ分けた値。 */
interface PaperCorner {
  /** 視線方向の成分(手前ほど大きい)。距離から引くと奥行きになる */
  along: number;
  /** 画面の右向きの成分 */
  right: number;
  /** 画面の上向きの成分 */
  up: number;
}

/** 直す前の「視点を戻す」が置いていたカメラ距離。いまは紙の大きさの下限に使う。 */
export function legacyPaperDistance(paperWidth: number, paperHeight: number): number {
  return (
    (Math.max(paperWidth, paperHeight) / (2 * HALF_FOV_TAN)) * LEGACY_CAMERA_MARGIN
  );
}

/** 紙の四隅を、紙の中心を原点としてカメラの向きの成分へ分ける。 */
export function paperCorners(
  paperWidth: number,
  paperHeight: number,
  dir: THREE.Vector3,
  right: THREE.Vector3,
  up: THREE.Vector3,
): PaperCorner[] {
  const cx = paperWidth / 2;
  const cy = paperHeight / 2;
  const points: [number, number][] = [
    [0, 0],
    [paperWidth, 0],
    [paperWidth, paperHeight],
    [0, paperHeight],
  ];
  return points.map(([x, y]) => {
    const v = new THREE.Vector3(x - cx, y - cy, 0);
    return { along: v.dot(dir), right: v.dot(right), up: v.dot(up) };
  });
}

/**
 * カメラの軸を区画の上から axisY の高さに置くための、仮想の枠の高さ。
 * 区画がこの枠の中へ収まる最小の大きさにすると、画面上の倍率が一つに決まる。
 */
function framingFullHeight(axisY: number, viewHeight: number): number {
  return 2 * Math.max(axisY, viewHeight - axisY);
}

/** 仮想の枠の高さから、世界の長さ1が画面上で何画素になるかの係数を出す。 */
function framingScale(axisY: number, viewHeight: number): number {
  return framingFullHeight(axisY, viewHeight) / (2 * HALF_FOV_TAN);
}

/** 軸を axisY に置いたとき、四隅が区画へ隙間つきで収まる最小の距離。 */
function framingDistance(
  corners: readonly PaperCorner[],
  axisY: number,
  viewWidth: number,
  viewHeight: number,
  padding: number,
): number {
  const scale = framingScale(axisY, viewHeight);
  const roomX = Math.max(viewWidth / 2 - padding, 1e-6);
  const roomUp = Math.max(axisY - padding, 1e-6);
  const roomDown = Math.max(viewHeight - padding - axisY, 1e-6);
  let distance = 0;
  for (const c of corners) {
    distance = Math.max(distance, c.along + (Math.abs(c.right) * scale) / roomX);
    distance = Math.max(
      distance,
      c.along + (Math.abs(c.up) * scale) / (c.up > 0 ? roomUp : roomDown),
    );
  }
  return distance;
}

/** 軸を axisY・距離を distance にしたときの、紙の四隅が作る画面上の四角形。 */
function framingBounds(
  corners: readonly PaperCorner[],
  axisY: number,
  distance: number,
  viewWidth: number,
  viewHeight: number,
): ScreenBounds {
  const scale = framingScale(axisY, viewHeight);
  let left = Infinity;
  let right = -Infinity;
  let top = Infinity;
  let bottom = -Infinity;
  for (const c of corners) {
    const depth = Math.max(distance - c.along, 1e-6);
    const x = viewWidth / 2 + (c.right * scale) / depth;
    const y = axisY - (c.up * scale) / depth;
    left = Math.min(left, x);
    right = Math.max(right, x);
    top = Math.min(top, y);
    bottom = Math.max(bottom, y);
  }
  return { left, right, top, bottom };
}

/**
 * 紙の上端が targetTop まで下がるように軸を下げる。ただし紙の高さが minHeight を
 * 割る手前で止める。下げるほど紙は小さくなるので、二分探索で境目を求める。
 */
function lowerAxis(
  corners: readonly PaperCorner[],
  viewWidth: number,
  viewHeight: number,
  padding: number,
  minHeight: number,
  targetTop: number,
): number {
  let lo = viewHeight / 2;
  let hi = viewHeight - padding;
  for (let i = 0; i < 60; i++) {
    const mid = (lo + hi) / 2;
    const bounds = framingBounds(
      corners,
      mid,
      framingDistance(corners, mid, viewWidth, viewHeight, padding),
      viewWidth,
      viewHeight,
    );
    if (bounds.bottom - bounds.top >= minHeight && bounds.top <= targetTop) lo = mid;
    else hi = mid;
  }
  return lo;
}

/**
 * 紙全体が3D区画へ収まり、できるだけ大きく、できるだけ左上の案内の札に
 * 隠れない視点を求める。
 *
 * 直す前は縦の画角だけで距離を決めていたため、区画の縦横比を無視していた。
 * ここでは四隅を実際に投影して左右・上下の4方向すべてを見るので、区画が
 * 縦長でも横長でもはみ出さない。
 *
 * 札を避けるのは「紙を小さくしすぎない」範囲だけにする。札が区画の高さの
 * 大部分を占める低い区画では避けきれないので、そのときは直す前の大きさを保ったまま、
 * 空いている下側の余りぶんだけ紙を下げる。
 *
 * @param hintBottomPx 左上の案内の札の下端(区画の上からのCSS px)。0なら札なし。
 */
export function paperFraming(
  paperWidth: number,
  paperHeight: number,
  viewWidth: number,
  viewHeight: number,
  hintBottomPx: number,
  dir: THREE.Vector3,
  right: THREE.Vector3,
  up: THREE.Vector3,
): PaperFraming {
  const corners = paperCorners(paperWidth, paperHeight, dir, right, up);
  const padding = Math.min(
    VIEW_EDGE_PADDING_PX,
    viewWidth * 0.05,
    viewHeight * 0.05,
  );
  // 直す前の大きさ。ここを下回らないようにする
  const legacyBounds = framingBounds(
    corners,
    viewHeight / 2,
    legacyPaperDistance(paperWidth, paperHeight),
    viewWidth,
    viewHeight,
  );
  const legacyHeight = legacyBounds.bottom - legacyBounds.top;

  const centeredAxis = viewHeight / 2;
  const centeredDistance = framingDistance(
    corners,
    centeredAxis,
    viewWidth,
    viewHeight,
    padding,
  );
  const centeredBounds = framingBounds(
    corners,
    centeredAxis,
    centeredDistance,
    viewWidth,
    viewHeight,
  );
  const centeredHeight = centeredBounds.bottom - centeredBounds.top;
  // 札の下端より下へ紙の上端を置きたい。区画の半分より下げることはしない
  const targetTop = Math.min(hintBottomPx + HINT_CLEARANCE_PX, viewHeight / 2);

  let axisY = centeredAxis;
  if (centeredBounds.top < targetTop) {
    // まず少しだけ縮めてよい条件で避けられるか試す
    const withShrink = lowerAxis(
      corners,
      viewWidth,
      viewHeight,
      padding,
      Math.min(centeredHeight, legacyHeight * HINT_AVOID_MIN_HEIGHT_RATIO),
      targetTop,
    );
    const shrunkTop = framingBounds(
      corners,
      withShrink,
      framingDistance(corners, withShrink, viewWidth, viewHeight, padding),
      viewWidth,
      viewHeight,
    ).top;
    axisY =
      shrunkTop >= hintBottomPx
        ? withShrink // 縮めた甲斐があった(札の下から出た)
        : lowerAxis(
            // 避けきれないので縮めない。空いているぶんだけ下げる
            corners,
            viewWidth,
            viewHeight,
            padding,
            Math.min(centeredHeight, legacyHeight),
            targetTop,
          );
  }

  const distance = framingDistance(corners, axisY, viewWidth, viewHeight, padding);
  const fullHeight = framingFullHeight(axisY, viewHeight);
  return {
    distance,
    fullWidth: viewWidth,
    fullHeight,
    offsetX: 0,
    offsetY: fullHeight / 2 - axisY,
    bounds: framingBounds(corners, axisY, distance, viewWidth, viewHeight),
  };
}

/**
 * 求めた枠をカメラの投影へ入れる。
 * setViewOffset は「仮想の枠のうち、この四角形だけを区画へ描く」指定で、
 * 縦横比も同時に決まる。区画の大きさが変わるたびに入れ直す。
 */
export function applyPaperFraming(
  camera: THREE.PerspectiveCamera,
  framing: PaperFraming,
  viewWidth: number,
  viewHeight: number,
): void {
  camera.setViewOffset(
    framing.fullWidth,
    framing.fullHeight,
    framing.offsetX,
    framing.offsetY,
    viewWidth,
    viewHeight,
  );
}

// ---------------------------------------------------------------------------
// シーン(レンダラ・カメラ・照明・入れ物)
// ---------------------------------------------------------------------------

export interface Viewer3DScene {
  readonly camera: THREE.PerspectiveCamera;
  /** 面と境界線を入れる入れ物(作り直しのたびに中身を破棄する) */
  readonly contentGroup: THREE.Group;
  /** 選択中の辺や折り線プレビューの強調を入れる入れ物 */
  readonly highlightGroup: THREE.Group;
  /** 全surface-aware材質が共有するowner textureのuniform。 */
  readonly ownerBinding: SurfaceOwnerBinding;
  /** 表示中の面・線。展開図が変わるまで作り替えない */
  content: Viewer3DContent | null;
  /** 現在実際に表示しているrigid/soft面。pickerも同じ面を使う。 */
  pickSurface?: PaperPickSurface | null;
  /** 次の描画機会に1回だけ描く(1フレーム1回にまとめる) */
  render(): void;
  /** 現在のテーマのCSS変数から背景色を読み直して描画する。 */
  syncTheme(): void;
  /**
   * 3D区画の大きさが変わったことを伝える。hintBottomPx を渡すと、案内の札の
   * 高さが変わったぶんも合わせ直しに反映する。
   */
  resize(widthPx: number, heightPx: number, hintBottomPx?: number): void;
  /**
   * 紙全体が見える斜め上の初期位置へカメラを戻す。
   * hintBottomPx に左上の案内の札の下端(区画の上からのCSS px)を渡すと、
   * 紙を小さくしすぎない範囲で札の下から逃がす。
   */
  resetCamera(paperWidth: number, paperHeight: number, hintBottomPx?: number): void;
  /** 面と線を差し替える(古い資源は破棄する) */
  setContent(content: Viewer3DContent): void;
  /**
   * たわみの網を表示する(SIM-012)。渡している間は面ごとの多角形の代わりに
   * 細かい三角形の網を描く。nullで従来の描き方へ戻る。
   *
   * 元の面(content.mesh)は入れ物から外すだけで捨てない。当たり判定も表示中の
   * 細分網へ切り替え、owner passと同じ可視面を選ぶ。面IDは細分前のIDを保つため、
   * たわみを入れても折る・つかむ操作の意味は変わらない。
   */
  setSoft(soft: SoftContent | null): void;
  /**
   * 面境界へ入らない既存線を、表示中の紙と同じ黒い線・表面判定で描く。
   * 呼び出し側が選択中の線を除き、現在のrigid/soft座標へ写した線分を渡す。
   */
  setSupplementalEdges(segments: readonly HingeSegment[]): void;
  /** 選択中の辺の強調を更新する(形と材質は使い回す) */
  setHighlight(segments: HighlightSegment[]): void;
  /**
   * 折った結果の下見を半透明の面で重ねる(UI-008)。
   * 多角形は畳み平面(z=0)の座標で、liftだけ持ち上げて描く。
   * 空配列を渡すと消える。
   */
  setPreview(polygons: Vec2[][], lift: number): void;
  /**
   * 折り線の描画中・紙を引いている間は左ドラッグの視点回転を止める
   * (拡大縮小・平行移動は残す)。
   * rotateWithRightを立てると、代わりに右ドラッグで視点を回せる
   * (立体を色々な向きから見ながら引くため。平行移動は中ボタンへ移る)
   */
  setDrawMode(enabled: boolean, rotateWithRight?: boolean): void;
  dispose(): void;
}

/** 強調線分。role省略時は従来どおり操作対象の黄色で描く。 */
export interface HighlightSegment extends HingeSegment {
  role?: "hinge" | "reference" | "focus" | "suspect" | "active" | "pinned" | "pinMark";
}

/** @types/threeがまだ公開していないLineMaterialの実在するlinewidthアクセサ。 */
export type HighlightLineMaterial = FilteredLineMaterial & { linewidth: number };

/** 強調表示7種類の共有材質。 */
export interface HighlightMaterials {
  highlightMaterial: HighlightLineMaterial;
  referenceHighlightMaterial: HighlightLineMaterial;
  focusHighlightMaterial: HighlightLineMaterial;
  suspectHighlightMaterial: HighlightLineMaterial;
  activeHighlightMaterial: HighlightLineMaterial;
  pinnedHighlightMaterial: HighlightLineMaterial;
  /** 固定した折り目の中点に打つ丸。線と同じ材質の仕組みで、太さだけ変える。 */
  pinMarkMaterial: HighlightLineMaterial;
}

/** 世界単位の断面を作らず、画面上で一定の太さを保つ線材質を作る。 */
function createHighlightMaterial(
  color: number,
  linewidth: number,
  depthTest: boolean,
  ownerBinding: SurfaceOwnerBinding,
): HighlightLineMaterial {
  const material = new LineMaterial({
    color,
    worldUnits: false,
    alphaToCoverage: true,
    depthTest,
    depthFunc: THREE.LessEqualDepth,
    depthWrite: false,
  }) as HighlightLineMaterial;
  material.linewidth = linewidth;
  if (depthTest) filterLineMaterialBySurfaceOwner(material, ownerBinding);
  return material;
}

/** 強調表示7種類の材質を作る。太さ・深度・surface owner判定を検査できる形にまとめる。 */
export function createHighlightMaterials(
  ownerBinding: SurfaceOwnerBinding = createSurfaceOwnerBinding(),
): HighlightMaterials {
  const highlightMaterial = createHighlightMaterial(
    HIGHLIGHT_COLOR,
    HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  const referenceHighlightMaterial = createHighlightMaterial(
    REFERENCE_HIGHLIGHT_COLOR,
    HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  const focusHighlightMaterial = createHighlightMaterial(
    FOCUS_HIGHLIGHT_COLOR,
    FOCUS_HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  const suspectHighlightMaterial = createHighlightMaterial(
    SUSPECT_HIGHLIGHT_COLOR,
    SUSPECT_HIGHLIGHT_WIDTH_PX,
    // 食い込みは紙の内側で起きるため、隠れると原因の折り目を見つけられない。
    // これだけは意図的に手前へ描く。
    false,
    ownerBinding,
  );
  const activeHighlightMaterial = createHighlightMaterial(
    ACTIVE_HIGHLIGHT_COLOR,
    HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  // 固定の印は「いま操作している折り目」より控えめにし、紙に隠れる側も
  // 同じ規則(深度と紙面の持ち主で隠す)にする。手前へ無理に出さない。
  const pinnedHighlightMaterial = createHighlightMaterial(
    PINNED_HIGHLIGHT_COLOR,
    HIGHLIGHT_WIDTH_PX,
    true,
    ownerBinding,
  );
  const pinMarkMaterial = createHighlightMaterial(
    PIN_MARK_COLOR,
    PIN_MARK_WIDTH_PX,
    true,
    ownerBinding,
  );
  return {
    highlightMaterial,
    referenceHighlightMaterial,
    focusHighlightMaterial,
    suspectHighlightMaterial,
    activeHighlightMaterial,
    pinnedHighlightMaterial,
    pinMarkMaterial,
  };
}

/**
 * 強調表示の中心線。画面方向へだけ太くするLine2用なので、世界座標ではx/z方向の
 * 幅が0、すなわち紙の重なりを幾何的に貫通する半径を持たない。
 */
export function createHighlightGeometry(): LineGeometry {
  const geometry = new LineGeometry();
  geometry.setPositions([0, 0, 0, 0, 1, 0]);
  return geometry;
}

const HIGHLIGHT_RENDER_ORDER = 5;
const ACTIVE_HIGHLIGHT_RENDER_ORDER = 6;
const SUSPECT_HIGHLIGHT_RENDER_ORDER = 7;

/** 7役割を実際に使う材質・描画順へ対応付ける。role省略時はhingeと同じ。 */
export function highlightAppearance(
  materials: HighlightMaterials,
  role: HighlightSegment["role"],
): { material: HighlightLineMaterial; renderOrder: number } {
  switch (role) {
    case "suspect":
      return {
        material: materials.suspectHighlightMaterial,
        renderOrder: SUSPECT_HIGHLIGHT_RENDER_ORDER,
      };
    case "active":
      return {
        material: materials.activeHighlightMaterial,
        renderOrder: ACTIVE_HIGHLIGHT_RENDER_ORDER,
      };
    case "pinned":
      return {
        material: materials.pinnedHighlightMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
    case "pinMark":
      return {
        material: materials.pinMarkMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
    case "reference":
      return {
        material: materials.referenceHighlightMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
    case "focus":
      return {
        material: materials.focusHighlightMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
    case "hinge":
    case undefined:
      return {
        material: materials.highlightMaterial,
        renderOrder: HIGHLIGHT_RENDER_ORDER,
      };
  }
}

/** createSceneが実際に使う強調線の生成・プール更新・破棄をまとめた層。 */
export interface HighlightLayer {
  readonly group: THREE.Group;
  readonly geometry: LineGeometry;
  readonly materials: HighlightMaterials;
  setSegments(segments: HighlightSegment[]): void;
  setOwnerCodes(codes: ReadonlyMap<number, number>): void;
  dispose(): void;
}

/**
 * 固定した折り目に、真ん中の太い印を足した線分の並びを返す。
 *
 * 濃い線だけでは輪郭の線と見分けにくかった(実機で普通の大きさにして確認)ので、
 * 折り目1本につき印を1つ打つ。同じ折り目が複数の面に分かれているときは、
 * **いちばん長い線分の真ん中**に1つだけ打つ(面の数だけ印が並ばないように)。
 *
 * 印は折り目に沿った短い線分を画面上で太く描いたもので、
 * **紙の面から外へは出ない**(`PIN_MARK_RATIO` の表を参照)。
 */
export function withPinMarks(
  segments: readonly HighlightSegment[],
): HighlightSegment[] {
  const longest = new Map<number, { segment: HighlightSegment; length: number }>();
  for (const segment of segments) {
    if (segment.role !== "pinned") continue;
    const length = segment.a.distanceTo(segment.b);
    if (!Number.isFinite(length)) continue;
    const found = longest.get(segment.edgeId);
    if (found === undefined || length > found.length) {
      longest.set(segment.edgeId, { segment, length });
    }
  }
  if (longest.size === 0) return [...segments];
  const marks: HighlightSegment[] = [];
  for (const { segment, length } of longest.values()) {
    if (!(length > PIN_MARK_MIN_LENGTH)) continue; // 短すぎる折り目には打たない
    const markLength = Math.min(
      Math.max(length * PIN_MARK_RATIO, PIN_MARK_MIN_LENGTH),
      Math.min(PIN_MARK_MAX_LENGTH, length),
    );
    const middle = segment.a.clone().add(segment.b).multiplyScalar(0.5);
    const along = segment.b
      .clone()
      .sub(segment.a)
      .normalize()
      .multiplyScalar(markLength / 2);
    marks.push({
      ...segment,
      role: "pinMark",
      a: middle.clone().sub(along),
      b: middle.clone().add(along),
    });
  }
  return [...segments, ...marks];
}

/**
 * 強調線の実描画物を作る。補助関数だけを検査して本番が円柱へ戻る退行を防ぐため、
 * createSceneと単体検査はこの同じ経路を使う。
 */
export function createHighlightLayer(
  ownerBinding: SurfaceOwnerBinding = createSurfaceOwnerBinding(),
): HighlightLayer {
  const group = new THREE.Group();
  const geometry = createHighlightGeometry();
  const materials = createHighlightMaterials(ownerBinding);
  const dir = new THREE.Vector3();
  let ownerCodes: ReadonlyMap<number, number> = new Map();

  return {
    group,
    geometry,
    materials,
    setSegments(rawSegments) {
      // 固定した折り目には、線に加えて中点の丸い印も描く。
      const segments = withPinMarks(rawSegments);
      const pool = group.children;
      let used = 0;
      for (const seg of segments) {
        dir.subVectors(seg.b, seg.a);
        const length = dir.length();
        if (length < 1e-9) continue;
        let line = pool[used] as Line2 | undefined;
        if (!line) {
          line = new Line2(geometry, materials.highlightMaterial);
          line.frustumCulled = false;
          // Line2本来のcallbackは、worldUnits=falseの線幅計算に必要なviewport解像度を
          // materialへ入れる。owner uniformを足しても必ず先に呼び、上書きで失わない。
          const updateLineResolution = line.onBeforeRender.bind(line);
          line.onBeforeRender = (
            renderer: THREE.WebGLRenderer,
            _scene?: THREE.Scene,
            camera?: THREE.Camera,
          ) => {
            updateLineResolution(renderer);
            const mode = line!.userData.surfaceOwnerMode as
              | "bypass"
              | "exact"
              | "any";
            if (mode === "bypass") return;
            setLineSurfaceOwner(
              line!.material as HighlightLineMaterial,
              mode,
              line!.userData.surfaceOwnerExpected as THREE.Vector4,
              line!.userData.surfaceOwnerRadius as number,
              camera && line!.userData.surfaceOwnerProbe instanceof THREE.Vector3
                ? {
                    camera,
                    start: line!.userData.surfaceOwnerStart as THREE.Vector3,
                    end: line!.userData.surfaceOwnerEnd as THREE.Vector3,
                    inside: line!.userData.surfaceOwnerProbe as THREE.Vector3,
                  }
                : undefined,
            );
          };
          group.add(line);
        }
        const appearance = highlightAppearance(materials, seg.role);
        line.material = appearance.material;
        const code = seg.ownerFace === undefined ? undefined : ownerCodes.get(seg.ownerFace);
        line.userData.surfaceOwnerMode =
          seg.role === "suspect" ? "bypass" : code === undefined ? "any" : "exact";
        line.userData.surfaceOwnerExpected = ownerCodeVector(code ?? 0);
        line.userData.surfaceOwnerRadius = Math.ceil(appearance.material.linewidth / 2) + 1;
        const ownerStart =
          line.userData.surfaceOwnerStart instanceof THREE.Vector3
            ? (line.userData.surfaceOwnerStart as THREE.Vector3)
            : new THREE.Vector3();
        const ownerEnd =
          line.userData.surfaceOwnerEnd instanceof THREE.Vector3
            ? (line.userData.surfaceOwnerEnd as THREE.Vector3)
            : new THREE.Vector3();
        line.userData.surfaceOwnerStart = ownerStart.copy(seg.a);
        line.userData.surfaceOwnerEnd = ownerEnd.copy(seg.b);
        if (seg.surfaceProbe) {
          const ownerProbe =
            line.userData.surfaceOwnerProbe instanceof THREE.Vector3
              ? (line.userData.surfaceOwnerProbe as THREE.Vector3)
              : new THREE.Vector3();
          line.userData.surfaceOwnerProbe = ownerProbe.copy(seg.surfaceProbe);
        } else {
          line.userData.surfaceOwnerProbe = null;
        }
        // 紙と同じ深度の表面では強調線を見せるため、紙より後に描く。
        // 非赤4種類は深度とsurface ownerの両方で紙の裏側なら隠れ、食い込みの赤だけは
        // 両判定を通さず最後に描くため、内側でも原因を見つけられる。
        line.renderOrder = appearance.renderOrder;
        line.position.copy(seg.a);
        line.quaternion.setFromUnitVectors(AXIS_Y, dir.normalize());
        line.scale.set(1, length, 1);
        line.visible = true;
        used++;
      }
      for (let i = used; i < pool.length; i++) pool[i].visible = false;
    },
    setOwnerCodes(codes) {
      ownerCodes = codes;
    },
    dispose() {
      group.clear(); // 全Line2が共有する資源は以下でそれぞれ1回だけ破棄する
      geometry.dispose();
      for (const material of Object.values(materials)) material.dispose();
    },
  };
}

/**
 * Face.edgesへ入らない補助線・行き止まりの折り線を、基本outlineへ足す描画層。
 * materialは現在表示中のrigid/soft outlineそのものを借り、ここでは生成・破棄しない。
 */
export interface SupplementalEdgeLayer {
  readonly group: THREE.Group;
  setSegments(
    segments: readonly HingeSegment[],
    material: THREE.LineBasicMaterial,
    ownerCodes: ReadonlyMap<number, number>,
  ): void;
  clear(): void;
  dispose(): void;
}

function finiteVector3(point: THREE.Vector3): boolean {
  return Number.isFinite(point.x) && Number.isFinite(point.y) && Number.isFinite(point.z);
}

/**
 * 補足線を、既存outlineと同じ非indexed属性へ変換する。
 * ownerFaceが無い／現在のsurfaceに無い線は、裏線を漏らすany判定へ落とさず描かない。
 */
export function createSupplementalEdgeLayer(): SupplementalEdgeLayer {
  const group = new THREE.Group();
  let outline: SurfaceOwnerOutlineGeometry | null = null;

  const clear = () => {
    group.clear();
    outline?.geometry.dispose();
    outline = null;
  };

  return {
    group,
    setSegments(segments, material, ownerCodes) {
      clear();
      const positions: number[] = [];
      const tokens: number[] = [];
      const lineIndices: number[] = [];
      const lineProbeIndices: number[] = [];
      for (const segment of segments) {
        if (!finiteVector3(segment.a) || !finiteVector3(segment.b)) continue;
        if (segment.a.distanceToSquared(segment.b) < 1e-18) continue;
        const code =
          segment.ownerFace === undefined ? undefined : ownerCodes.get(segment.ownerFace);
        if (code === undefined) continue;
        const bytes = ownerCodeBytes(code);
        const base = positions.length / 3;
        const probe =
          segment.surfaceProbe && finiteVector3(segment.surfaceProbe)
            ? segment.surfaceProbe
            : segment.a;
        for (const point of [segment.a, segment.b, probe]) {
          positions.push(point.x, point.y, point.z);
          tokens.push(...bytes);
        }
        lineIndices.push(base, base + 1);
        lineProbeIndices.push(base + 2);
      }
      if (lineIndices.length === 0) return;

      const sourcePosition = new THREE.BufferAttribute(new Float32Array(positions), 3);
      const sourceToken = new THREE.Uint8BufferAttribute(new Uint8Array(tokens), 4, true);
      outline = createSurfaceOwnerOutlineGeometry({
        sourcePosition,
        sourceToken,
        lineIndices,
        lineProbeIndices,
      });
      const line = new THREE.LineSegments(outline.geometry, material);
      line.renderOrder = 1;
      line.frustumCulled = false;
      group.add(line);
    },
    clear,
    dispose: clear,
  };
}

/** 面・線1つ分の資源を破棄する */
function disposeDrawable(child: THREE.Object3D): void {
  if (!(child instanceof THREE.Mesh || child instanceof THREE.LineSegments)) return;
  child.geometry.dispose();
  const material = child.material;
  if (Array.isArray(material)) {
    for (const m of material) m.dispose();
  } else {
    material.dispose();
  }
}

/** グループの中身を破棄して空にする(ジオメトリ・マテリアルのリーク防止) */
export function clearGroup(group: THREE.Group): void {
  for (const child of [...group.children]) {
    group.remove(child);
    disposeDrawable(child);
  }
}

/** 折った結果の半透明下見。紙の内側まで見る用途なのでowner/depth判定を通さない。 */
export function createPreviewMaterial(): THREE.MeshBasicMaterial {
  return new THREE.MeshBasicMaterial({
    color: PREVIEW_COLOR,
    transparent: true,
    opacity: PREVIEW_OPACITY,
    side: THREE.DoubleSide,
    depthTest: false,
  });
}

/** canvasにレンダラ・カメラ・軌道操作・照明を用意する */
export function createScene(canvas: HTMLCanvasElement): Viewer3DScene {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio || 1);

  // 1画素につき最前面の面IDをRGBA8へ書く前処理。最前深度とowner色を分けるため
  // draw callは面数によらず2回だけ増える。全三角形はどちらも動的index 1本で描く。
  const ownerBinding = createSurfaceOwnerBinding();
  const ownerPass = createSurfaceOwnerPassResources(ownerBinding);
  const emptyOwnerGeometry = new THREE.BufferGeometry();
  const ownerMesh = new THREE.Mesh(emptyOwnerGeometry, ownerPass.colorMaterial);
  ownerMesh.frustumCulled = false;
  ownerMesh.visible = false;
  const ownerScene = new THREE.Scene();
  ownerScene.add(ownerMesh);
  let ownerSurface: SurfaceOwnerSurface | null = null;
  const drawingBufferSize = new THREE.Vector2();
  const savedClearColor = new THREE.Color();

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(canvas3dBackgroundColor(canvas));

  const camera = new THREE.PerspectiveCamera(CAMERA_FOV, 1, 0.01, 100);
  camera.position.set(0.5, -1.2, 1.4);

  // 折った面がどちら向きでも暗くなりすぎないよう、環境光+表裏2方向の平行光
  scene.add(new THREE.AmbientLight(0xffffff, 1.4));
  const key = new THREE.DirectionalLight(0xffffff, 1.6);
  key.position.set(0.4, 0.8, 1.0);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xffffff, 1.0);
  fill.position.set(-0.5, -0.8, -1.0);
  scene.add(fill);

  const contentGroup = new THREE.Group();
  const supplementalEdgeLayer = createSupplementalEdgeLayer();
  const highlightLayer = createHighlightLayer(ownerBinding);
  const highlightGroup = highlightLayer.group;
  // 折った結果の下見。紙に隠れず常に見えるよう深度判定を切って最後に描く
  const previewMaterial = createPreviewMaterial();
  const previewMesh = new THREE.Mesh(new THREE.BufferGeometry(), previewMaterial);
  previewMesh.renderOrder = 2;
  previewMesh.frustumCulled = false;
  previewMesh.visible = false;
  scene.add(contentGroup, supplementalEdgeLayer.group, highlightGroup, previewMesh);

  const controls = new OrbitControls(camera, canvas);
  controls.enableDamping = false; // 常時描画ループを持たない(変化時だけ描く)
  // 視点を回すのは下のドラッグ処理だけにする。OrbitControlsの回転は
  // 世界の上下を軸にするため真上・真下で行き止まりになる(利用者の指摘)。
  // 寄る・平行移動・ボタンの割り当て(setDrawMode)はOrbitControlsのまま使う。
  controls.enableRotate = false;

  // --- 紙を区画へ収める視点合わせ ------------------------------------------
  // 区画の大きさが変わったときにも合わせ直せるよう、材料を覚えておく。
  // 3Dの状態は保存しないので、この記憶はこの画面が生きている間だけのもの。
  let fitRequest: {
    paperWidth: number;
    paperHeight: number;
    hintBottomPx: number;
  } | null = null;
  let viewWidth = 0;
  let viewHeight = 0;
  const framingRight = new THREE.Vector3();
  const framingUp = new THREE.Vector3();
  const framingDir = new THREE.Vector3();
  const framingCenter = new THREE.Vector3();
  const framingOffset = new THREE.Vector3();

  /**
   * 覚えている紙の大きさと、渡された視線・画面の上向きから枠を求める。
   * 区画の大きさがまだ届いていない間は求めない(nullを返す)。
   */
  const framingFor = (dir: THREE.Vector3, up: THREE.Vector3): PaperFraming | null => {
    if (fitRequest === null || viewWidth <= 0 || viewHeight <= 0) return null;
    framingRight.crossVectors(up, dir);
    if (framingRight.lengthSq() <= MIN_ORBIT_RADIUS ** 2) return null;
    framingRight.normalize();
    framingUp.crossVectors(dir, framingRight).normalize();
    return paperFraming(
      fitRequest.paperWidth,
      fitRequest.paperHeight,
      viewWidth,
      viewHeight,
      fitRequest.hintBottomPx,
      dir,
      framingRight,
      framingUp,
    );
  };

  /** 枠を求められないとき(紙の大きさがまだ届いていない等)の素直な投影。 */
  const plainProjection = () => {
    if (viewWidth <= 0 || viewHeight <= 0) return;
    camera.clearViewOffset();
    camera.aspect = viewWidth / viewHeight;
    camera.updateProjectionMatrix();
  };

  /**
   * 区画の大きさが変わったあとに合わせ直す。向きと注視点はそのままにして、
   * 枠を作り直し、狭くなって紙が収まらなくなったぶんだけカメラを引く。
   * 利用者が寄せた分を勝手に戻さないよう、寄せる向きへは動かさない。
   */
  const refitToViewport = () => {
    framingDir.copy(camera.position).sub(controls.target);
    const framing =
      fitRequest === null || framingDir.lengthSq() <= MIN_ORBIT_RADIUS ** 2
        ? null
        : framingFor(framingDir.normalize(), cameraScreenUp(camera));
    if (framing === null || fitRequest === null) {
      plainProjection();
      return;
    }
    applyPaperFraming(camera, framing, viewWidth, viewHeight);
    camera.updateProjectionMatrix();
    framingCenter.set(fitRequest.paperWidth / 2, fitRequest.paperHeight / 2, 0);
    const current = framingOffset
      .copy(camera.position)
      .sub(framingCenter)
      .dot(framingDir);
    if (current < framing.distance) {
      camera.position.addScaledVector(framingDir, framing.distance - current);
      controls.update();
    }
  };

  // 描画は1フレームに1回だけ。連続した変化(座標更新・選択・視点操作)が
  // 同じフレームに重なっても描画は1回にまとまる
  let frameHandle: number | null = null;
  let disposed = false;
  const draw = () => {
    frameHandle = null;
    if (disposed) return;
    if (ownerSurface !== null) {
      orderSurfaceOwner(ownerSurface, camera);
      const previousTarget = renderer.getRenderTarget();
      renderer.getClearColor(savedClearColor);
      const previousClearAlpha = renderer.getClearAlpha();
      const previousOverrideMaterial = ownerScene.overrideMaterial;
      try {
        renderer.setClearColor(0x000000, 0);

        // Pass 1: 三角形分割や描画順に依存しない最前深度だけを確定する。
        ownerScene.overrideMaterial = ownerPass.depthMaterial;
        renderer.setRenderTarget(ownerPass.depthTarget);
        renderer.render(ownerScene, camera);

        // Pass 2: depth textureを読む別targetへ、最前深度とtieの面だけを
        // layer/面の鏡映偶奇/決定的fallback順で上書きする。描画中target自身は
        // 読まないためfeedbackにならない。
        ownerScene.overrideMaterial = null;
        renderer.setRenderTarget(ownerPass.colorTarget);
        renderer.render(ownerScene, camera);
      } finally {
        ownerScene.overrideMaterial = previousOverrideMaterial;
        renderer.setRenderTarget(previousTarget);
        renderer.setClearColor(savedClearColor, previousClearAlpha);
      }
    }
    renderer.render(scene, camera);
  };
  const render = () => {
    if (!disposed && frameHandle === null) frameHandle = requestAnimationFrame(draw);
  };
  controls.addEventListener("change", render);

  // --- 視点のドラッグ回転 -------------------------------------------------
  // 押した位置からの差分だけを毎回渡す。OrbitControlsの回転と同じ量になる。
  let rotateDrag: { pointerId: number; x: number; y: number } | null = null;

  /**
   * カメラのupを、いま画面が上と見なしている向きへそろえる。
   * 視点立方体で移った直後はupが取り残されるため、操作を始める前に必ず合わせる。
   * これをしないと、寄る・平行移動のときのOrbitControlsの向き直しで傾きが跳ぶ。
   */
  const syncCameraUp = () => {
    camera.up.copy(cameraScreenUp(camera));
  };

  const onCanvasPointerDown = (event: PointerEvent) => {
    syncCameraUp();
    if (rotateDrag !== null) {
      // 2本目の指が触れたらOrbitControlsの2本指操作へ譲る。
      rotateDrag = null;
      return;
    }
    if (!controls.enabled) return;
    const rotates =
      event.pointerType === "touch"
        ? controls.touches.ONE === THREE.TOUCH.ROTATE
        : viewRotationStarts(
            controls.mouseButtons,
            event.button,
            event.ctrlKey || event.metaKey || event.shiftKey,
          );
    if (!rotates) return;
    rotateDrag = { pointerId: event.pointerId, x: event.clientX, y: event.clientY };
  };

  const onDocumentPointerMove = (event: PointerEvent) => {
    if (rotateDrag === null || rotateDrag.pointerId !== event.pointerId) return;
    const dragX = event.clientX - rotateDrag.x;
    const dragY = event.clientY - rotateDrag.y;
    rotateDrag.x = event.clientX;
    rotateDrag.y = event.clientY;
    if (dragX === 0 && dragY === 0) return;
    applyCameraDragRotation(camera, controls.target, dragX, dragY, canvas.clientHeight);
    render();
  };

  const onDocumentPointerUp = (event: PointerEvent) => {
    if (rotateDrag !== null && rotateDrag.pointerId === event.pointerId) {
      rotateDrag = null;
    }
  };

  const documentOf = canvas.ownerDocument;
  canvas.addEventListener("pointerdown", onCanvasPointerDown);
  canvas.addEventListener("wheel", syncCameraUp, { capture: true });
  documentOf.addEventListener("pointermove", onDocumentPointerMove);
  documentOf.addEventListener("pointerup", onDocumentPointerUp);
  documentOf.addEventListener("pointercancel", onDocumentPointerUp);

  // 描画資源が失われて復帰したときは描き直す(復帰直後は画面が空になるため)
  const onContextRestored = () => render();
  canvas.addEventListener("webglcontextrestored", onContextRestored);

  /** 表示中のたわみの網(null なら従来の面の描き方) */
  let soft: SoftContent | null = null;

  const showOwner = (surface: SurfaceOwnerSurface | null) => {
    ownerSurface = surface;
    ownerMesh.visible = surface !== null;
    if (surface !== null) ownerMesh.geometry = surface.geometry;
    ownerBinding.enabled.value = surface === null ? 0 : 1;
  };

  const pickSurfaceOf = (
    mesh: THREE.Mesh,
    surface: SurfaceOwnerSurface,
  ): PaperPickSurface => ({
    mesh,
    triangleFaceIds: surface.triangleFaces,
    triangleLayers: surface.triangleLayers,
    faceSurfaceRanks: surface.faceSurfaceRanks,
  });

  const api: Viewer3DScene = {
    camera,
    contentGroup,
    highlightGroup,
    ownerBinding,
    content: null,
    pickSurface: null,
    render,
    syncTheme() {
      scene.background = new THREE.Color(canvas3dBackgroundColor(canvas));
      render();
    },
    resize(widthPx, heightPx, hintBottomPx) {
      if (widthPx === 0 || heightPx === 0) return;
      viewWidth = widthPx;
      viewHeight = heightPx;
      if (fitRequest !== null && hintBottomPx !== undefined) {
        fitRequest.hintBottomPx = hintBottomPx;
      }
      // 画面の細かさは移動先の画面で変わることがあるので毎回合わせ直す
      renderer.setPixelRatio(window.devicePixelRatio || 1);
      renderer.setSize(widthPx, heightPx, false);
      renderer.getDrawingBufferSize(drawingBufferSize);
      resizeSurfaceOwnerPassResources(
        ownerPass,
        ownerBinding,
        drawingBufferSize.x,
        drawingBufferSize.y,
      );
      // 区画の大きさが変わると収まり方も変わる。向きと注視点はそのままに、
      // 枠を作り直し、狭くなって収まらなくなったぶんだけカメラを引く。
      refitToViewport();
      render();
    },
    resetCamera(paperWidth, paperHeight, hintBottomPx = 0) {
      fitRequest = { paperWidth, paperHeight, hintBottomPx };
      const center = new THREE.Vector3(paperWidth / 2, paperHeight / 2, 0);
      // 回して傾いたままの上向きを持ち込まないよう、世界の上へ戻してから向き直す。
      camera.up.set(0, 1, 0);
      controls.target.copy(center);
      const framing = framingFor(CAMERA_DIR, camera.up);
      const distance =
        framing?.distance ?? legacyPaperDistance(paperWidth, paperHeight);
      camera.position.copy(center).addScaledVector(CAMERA_DIR, distance);
      if (framing === null) plainProjection();
      else applyPaperFraming(camera, framing, viewWidth, viewHeight);
      camera.updateProjectionMatrix();
      controls.update();
      render();
    },
    setContent(content) {
      api.setSoft(null); // たわみの表示物を片付け、外していた面・線を入れ物へ戻す
      if (api.content !== null && api.content !== content) {
        disposeSurfaceOwnerSurface(api.content.owner);
      }
      clearGroup(contentGroup);
      api.content = content;
      contentGroup.add(content.mesh, content.line);
      showOwner(content.owner);
      highlightLayer.setOwnerCodes(content.owner.ownerCodes);
      api.pickSurface = pickSurfaceOf(content.mesh, content.owner);
      render();
    },
    setSoft(next) {
      // 座標・owner token・共有materialの参照元が切り替わる。呼び出し側が現在表示中の
      // 面へ写した線分を直後に渡し直すまで、古い面の補足線は表示しない。
      supplementalEdgeLayer.clear();
      if (soft !== null && soft !== next) {
        contentGroup.remove(soft.mesh, soft.line);
        disposeDrawable(soft.mesh);
        disposeDrawable(soft.line);
        disposeSurfaceOwnerSurface(soft.owner);
      }
      soft = next;
      const base = api.content;
      if (next !== null) {
        if (base) contentGroup.remove(base.mesh, base.line);
        if (next.mesh.parent !== contentGroup) contentGroup.add(next.mesh, next.line);
        showOwner(next.owner);
        highlightLayer.setOwnerCodes(next.owner.ownerCodes);
        api.pickSurface = pickSurfaceOf(next.mesh, next.owner);
      } else if (base && base.mesh.parent !== contentGroup) {
        contentGroup.add(base.mesh, base.line);
        showOwner(base.owner);
        highlightLayer.setOwnerCodes(base.owner.ownerCodes);
        api.pickSurface = pickSurfaceOf(base.mesh, base.owner);
      } else if (!base) {
        showOwner(null);
        api.pickSurface = null;
      }
      render();
    },
    setSupplementalEdges(segments) {
      const displayed = soft ?? api.content;
      const material = displayed?.line.material;
      if (
        !displayed ||
        Array.isArray(material) ||
        !(material instanceof THREE.LineBasicMaterial)
      ) {
        supplementalEdgeLayer.clear();
      } else {
        supplementalEdgeLayer.setSegments(segments, material, displayed.owner.ownerCodes);
      }
      render();
    },
    setHighlight(segments) {
      highlightLayer.setSegments(segments);
      render();
    },
    setPreview(polygons, lift) {
      const points: number[] = [];
      const indices: number[] = [];
      for (const poly of polygons) {
        if (poly.length < 3) continue;
        const base = points.length / 3;
        for (const p of poly) points.push(p[0], p[1], lift);
        for (const t of triangulate(poly)) {
          indices.push(base + t[0], base + t[1], base + t[2]);
        }
      }
      // 形は毎回変わるので作り直す(前の形は必ず捨てる)
      previewMesh.geometry.dispose();
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute(
        "position",
        new THREE.BufferAttribute(new Float32Array(points), 3),
      );
      geometry.setIndex(indices);
      previewMesh.geometry = geometry;
      previewMesh.visible = indices.length > 0;
      render();
    },
    setDrawMode(enabled, rotateWithRight = false) {
      controls.mouseButtons.LEFT = enabled ? null : THREE.MOUSE.ROTATE;
      const swap = enabled && rotateWithRight;
      controls.mouseButtons.RIGHT = swap ? THREE.MOUSE.ROTATE : THREE.MOUSE.PAN;
      controls.mouseButtons.MIDDLE = swap ? THREE.MOUSE.PAN : THREE.MOUSE.DOLLY;
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      previewMesh.geometry.dispose();
      previewMaterial.dispose();
      if (frameHandle !== null) {
        cancelAnimationFrame(frameHandle);
        frameHandle = null;
      }
      canvas.removeEventListener("webglcontextrestored", onContextRestored);
      canvas.removeEventListener("pointerdown", onCanvasPointerDown);
      canvas.removeEventListener("wheel", syncCameraUp, { capture: true });
      documentOf.removeEventListener("pointermove", onDocumentPointerMove);
      documentOf.removeEventListener("pointerup", onDocumentPointerUp);
      documentOf.removeEventListener("pointercancel", onDocumentPointerUp);
      rotateDrag = null;
      controls.removeEventListener("change", render);
      controls.dispose();
      api.setSoft(null); // たわみの表示物も片付ける(外していた面・線が入れ物へ戻る)
      if (api.content !== null) disposeSurfaceOwnerSurface(api.content.owner);
      clearGroup(contentGroup);
      api.content = null;
      api.pickSurface = null;
      showOwner(null);
      supplementalEdgeLayer.dispose();
      highlightLayer.dispose();
      emptyOwnerGeometry.dispose();
      disposeSurfaceOwnerPassResources(ownerPass);
      renderer.dispose();
    },
  };
  return api;
}
