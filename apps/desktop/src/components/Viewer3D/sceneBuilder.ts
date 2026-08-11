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
/** 選択中ヒンジの太さ(紙の長辺=1.0を基準にした半径) */
const HIGHLIGHT_RADIUS = 0.006;
/** カメラ画角(度) */
const CAMERA_FOV = 45;
/** 初期カメラの向き(紙の中心から見た方向。斜め上=手前上から見下ろす) */
const CAMERA_DIR = new THREE.Vector3(0.35, -0.85, 0.95).normalize();
/** 紙全体が収まるための距離の余裕 */
const CAMERA_MARGIN = 1.35;
/** 強調表示の円柱の分割数 */
const HIGHLIGHT_SEGMENTS = 8;
/** 円柱の伸ばす向き(強調表示の回転計算に使う) */
const AXIS_Y = new THREE.Vector3(0, 1, 0);
/** 紙の長辺(展開図は長辺=1.0の正規化座標。層のずらし量の基準にする) */
const PAPER_LONG_SIDE = 1;

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
  ia: number;
  ib: number;
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
  /** 境界線の添字(2つで1本) */
  lineIndices: number[];
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
  const lineIndices: number[] = [];
  const hingeSlots: HingeSlot[] = [];
  const flat: number[] = [];
  const seen = new Set<number>();
  let offset = 0;

  for (const poly of facePolygons(doc, faces)) {
    const n = poly.points.length;
    if (n < 3) continue;
    slots.set(poly.id, { offset, count: n });
    for (const p of poly.points) flat.push(p[0], p[1], 0);
    for (const t of triangulate(poly.points)) {
      indices.push(offset + t[0], offset + t[1], offset + t[2]);
      triangleFaceIds.push(poly.id);
    }
    for (let i = 0; i < n; i++) {
      const j = (i + 1) % n;
      lineIndices.push(offset + i, offset + j);
      // 境界辺ID列は頂点列と同順(edges[i] が vertices[i]→vertices[i+1])
      const edgeId = poly.edges[i];
      if (edgeId !== undefined && hinges.has(edgeId) && !seen.has(edgeId)) {
        seen.add(edgeId);
        hingeSlots.push({ edgeId, ia: offset + i, ib: offset + j });
      }
    }
    offset += n;
  }
  return {
    slots,
    vertexCount: offset,
    indices,
    triangleFaceIds,
    lineIndices,
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
  /** 面の境界線(頂点座標をmeshと共有する) */
  line: THREE.LineSegments;
  /** 頂点座標。updateFrameで書き換える実体 */
  positions: Float32Array;
  /** 現在の立体形状におけるヒンジの線分(選択の当たり判定・強調表示に使う) */
  hingeSegments: HingeSegment[];
}

/** 0〜255のRGBをThree.jsの色へ */
function toColor(rgb: [number, number, number]): THREE.Color {
  return new THREE.Color(rgb[0] / 255, rgb[1] / 255, rgb[2] / 255);
}

/** 面を奥へずらす量(境界線が面に埋もれないように) */
export const FACE_OFFSET_UNITS = 1;
/**
 * 裏面を表面より更に奥へずらす量。
 *
 * 表と裏は同じ三角形を2回描くので、Three.jsは必ず表→裏の順に描く。深度判定は
 * 「同値なら通す」なので、紙が完全に重なって深度が同値になると、後から描かれる
 * 裏面が表の色を塗りつぶし、表を向いた面まで裏の白になってしまう(実機で発生)。
 * 裏面だけ深度を余分に奥へずらし、同値のときは表の色が残るようにする。
 */
export const BACK_OFFSET_UNITS = 3;

/** 面のマテリアル。境界線が面に埋もれないよう面を少しだけ奥へずらして描く */
function faceMaterial(rgb: [number, number, number], side: THREE.Side) {
  return new THREE.MeshLambertMaterial({
    color: toColor(rgb),
    side,
    polygonOffset: true,
    polygonOffsetFactor: 1,
    polygonOffsetUnits:
      side === THREE.BackSide ? BACK_OFFSET_UNITS : FACE_OFFSET_UNITS,
  });
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

  const lineGeometry = new THREE.BufferGeometry();
  lineGeometry.setAttribute("position", position); // 座標は面と共有する
  lineGeometry.setIndex(topology.lineIndices);

  const mesh = new THREE.Mesh(faceGeometry, [
    faceMaterial(display.front_color, THREE.FrontSide),
    faceMaterial(display.back_color, THREE.BackSide),
  ]);
  const line = new THREE.LineSegments(
    lineGeometry,
    new THREE.LineBasicMaterial({ color: OUTLINE_COLOR }),
  );
  // 形が毎フレーム変わるので、範囲の当たり判定による省略は行わない
  mesh.frustumCulled = false;
  line.frustumCulled = false;

  return {
    topology,
    mesh,
    line,
    positions,
    hingeSegments: topology.hingeSlots.map((s) => ({
      edgeId: s.edgeId,
      a: new THREE.Vector3(),
      b: new THREE.Vector3(),
    })),
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
  if (frame === null) {
    positions.set(topology.flatPositions);
  } else {
    // 重なった面はその平面の法線方向へ離す(向きが+zとは限らないのでベクトル)
    const lifts = stackLifts(frame, PAPER_LONG_SIDE);
    for (let i = 0; i < frame.faces.length; i++) {
      const f = frame.faces[i];
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
  geometry.computeVertexNormals();
  geometry.computeBoundingSphere();

  for (let i = 0; i < topology.hingeSlots.length; i++) {
    const slot = topology.hingeSlots[i];
    const seg = content.hingeSegments[i];
    seg.a.fromArray(positions, slot.ia * 3);
    seg.b.fromArray(positions, slot.ib * 3);
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
  positions: Float32Array;
}

/** たわみの網から表示物を作る(座標は updateSoftContent で入れる) */
export function createSoftContent(
  soft: SoftMesh,
  display: DisplaySettings,
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

  const lineGeometry = new THREE.BufferGeometry();
  lineGeometry.setAttribute("position", position); // 座標は面と共有する
  lineGeometry.setIndex(layout.lineIndices);

  const mesh = new THREE.Mesh(geometry, [
    faceMaterial(display.front_color, THREE.FrontSide),
    faceMaterial(display.back_color, THREE.BackSide),
  ]);
  const line = new THREE.LineSegments(
    lineGeometry,
    new THREE.LineBasicMaterial({ color: OUTLINE_COLOR }),
  );
  mesh.frustumCulled = false;
  line.frustumCulled = false;
  return { signature: softSignature(soft), layout, mesh, line, positions };
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
  if (frame) {
    const values = stackLifts(frame, PAPER_LONG_SIDE);
    for (let i = 0; i < frame.faces.length; i++) {
      lifts.set(frame.faces[i].face, values[i]);
    }
  }
  fillSoftPositions(soft, content.layout, lifts, content.positions);
  const geometry = content.mesh.geometry;
  geometry.getAttribute("position").needsUpdate = true;
  geometry.computeVertexNormals();
  geometry.computeBoundingSphere();
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
  /** 表示中の面・線。展開図が変わるまで作り替えない */
  content: Viewer3DContent | null;
  /** 次の描画機会に1回だけ描く(1フレーム1回にまとめる) */
  render(): void;
  /** 現在のテーマのCSS変数から背景色を読み直して描画する。 */
  syncTheme(): void;
  resize(widthPx: number, heightPx: number): void;
  /** 紙全体が見える斜め上の初期位置へカメラを戻す */
  resetCamera(paperWidth: number, paperHeight: number): void;
  /** 面と線を差し替える(古い資源は破棄する) */
  setContent(content: Viewer3DContent): void;
  /**
   * たわみの網を表示する(SIM-012)。渡している間は面ごとの多角形の代わりに
   * 細かい三角形の網を描く。nullで従来の描き方へ戻る。
   *
   * 元の面(content.mesh)は入れ物から外すだけで捨てない。当たり判定(どの面を
   * つかんだか・どの折り線を選んだか)は今までどおり剛体折りの多角形で行うので、
   * たわみを入れても折る・つかむ操作がそのまま使える。
   */
  setSoft(soft: SoftContent | null): void;
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
  role?: "hinge" | "reference" | "focus" | "suspect" | "active";
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

/** canvasにレンダラ・カメラ・軌道操作・照明を用意する */
export function createScene(canvas: HTMLCanvasElement): Viewer3DScene {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio || 1);

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
  const highlightGroup = new THREE.Group();
  // 折った結果の下見。紙に隠れず常に見えるよう深度判定を切って最後に描く
  const previewMaterial = new THREE.MeshBasicMaterial({
    color: PREVIEW_COLOR,
    transparent: true,
    opacity: PREVIEW_OPACITY,
    side: THREE.DoubleSide,
    depthTest: false,
  });
  const previewMesh = new THREE.Mesh(new THREE.BufferGeometry(), previewMaterial);
  previewMesh.renderOrder = 2;
  previewMesh.frustumCulled = false;
  previewMesh.visible = false;
  scene.add(contentGroup, highlightGroup, previewMesh);

  const controls = new OrbitControls(camera, canvas);
  controls.enableDamping = false; // 常時描画ループを持たない(変化時だけ描く)

  // 描画は1フレームに1回だけ。連続した変化(座標更新・選択・視点操作)が
  // 同じフレームに重なっても描画は1回にまとまる
  let frameHandle: number | null = null;
  const draw = () => {
    frameHandle = null;
    renderer.render(scene, camera);
  };
  const render = () => {
    if (frameHandle === null) frameHandle = requestAnimationFrame(draw);
  };
  controls.addEventListener("change", render);

  // 描画資源が失われて復帰したときは描き直す(復帰直後は画面が空になるため)
  const onContextRestored = () => render();
  canvas.addEventListener("webglcontextrestored", onContextRestored);

  // 強調表示は長さ1の円柱1個を使い回し、位置・向き・伸ばし方だけ変える
  const highlightGeometry = new THREE.CylinderGeometry(
    HIGHLIGHT_RADIUS,
    HIGHLIGHT_RADIUS,
    1,
    HIGHLIGHT_SEGMENTS,
  );
  highlightGeometry.translate(0, 0.5, 0); // 原点を端点aに合わせる
  const highlightMaterial = new THREE.MeshBasicMaterial({
    color: HIGHLIGHT_COLOR,
    depthTest: false, // 紙に隠れても見えるように深度判定を切る
  });
  const referenceHighlightMaterial = new THREE.MeshBasicMaterial({
    color: REFERENCE_HIGHLIGHT_COLOR,
    depthTest: false,
  });
  const focusHighlightMaterial = new THREE.MeshBasicMaterial({
    color: FOCUS_HIGHLIGHT_COLOR,
    depthTest: false,
  });
  const suspectHighlightMaterial = new THREE.MeshBasicMaterial({
    color: SUSPECT_HIGHLIGHT_COLOR,
    depthTest: false,
  });
  const activeHighlightMaterial = new THREE.MeshBasicMaterial({
    color: ACTIVE_HIGHLIGHT_COLOR,
    depthTest: false,
  });
  const dir = new THREE.Vector3();

  /** 表示中のたわみの網(null なら従来の面の描き方) */
  let soft: SoftContent | null = null;

  const api: Viewer3DScene = {
    camera,
    contentGroup,
    highlightGroup,
    content: null,
    render,
    syncTheme() {
      scene.background = new THREE.Color(canvas3dBackgroundColor(canvas));
      render();
    },
    resize(widthPx, heightPx) {
      if (widthPx === 0 || heightPx === 0) return;
      // 画面の細かさは移動先の画面で変わることがあるので毎回合わせ直す
      renderer.setPixelRatio(window.devicePixelRatio || 1);
      renderer.setSize(widthPx, heightPx, false);
      camera.aspect = widthPx / heightPx;
      camera.updateProjectionMatrix();
      render();
    },
    resetCamera(paperWidth, paperHeight) {
      const center = new THREE.Vector3(paperWidth / 2, paperHeight / 2, 0);
      const extent = Math.max(paperWidth, paperHeight);
      const dist =
        (extent / (2 * Math.tan((CAMERA_FOV * Math.PI) / 360))) * CAMERA_MARGIN;
      camera.position.copy(center).addScaledVector(CAMERA_DIR, dist);
      controls.target.copy(center);
      controls.update();
      render();
    },
    setContent(content) {
      api.setSoft(null); // たわみの表示物を片付け、外していた面・線を入れ物へ戻す
      clearGroup(contentGroup);
      api.content = content;
      contentGroup.add(content.mesh, content.line);
      render();
    },
    setSoft(next) {
      if (soft !== null && soft !== next) {
        contentGroup.remove(soft.mesh, soft.line);
        disposeDrawable(soft.mesh);
        disposeDrawable(soft.line);
      }
      soft = next;
      const base = api.content;
      if (next !== null) {
        if (base) contentGroup.remove(base.mesh, base.line);
        if (next.mesh.parent !== contentGroup) contentGroup.add(next.mesh, next.line);
      } else if (base && base.mesh.parent !== contentGroup) {
        contentGroup.add(base.mesh, base.line);
      }
      render();
    },
    setHighlight(segments) {
      const pool = highlightGroup.children;
      let used = 0;
      for (const seg of segments) {
        dir.subVectors(seg.b, seg.a);
        const length = dir.length();
        if (length < 1e-9) continue;
        let mesh = pool[used] as THREE.Mesh | undefined;
        if (!mesh) {
          mesh = new THREE.Mesh(highlightGeometry, highlightMaterial);
          mesh.frustumCulled = false;
          highlightGroup.add(mesh);
        }
        mesh.material =
          seg.role === "suspect"
            ? suspectHighlightMaterial
            : seg.role === "active"
              ? activeHighlightMaterial
              : seg.role === "reference"
                ? referenceHighlightMaterial
                : seg.role === "focus"
                  ? focusHighlightMaterial
                  : highlightMaterial;
        // 食い込みの赤を最優先し、操作中は水色で示す。
        mesh.renderOrder = seg.role === "suspect" ? 7 : seg.role === "active" ? 6 : 5;
        mesh.position.copy(seg.a);
        mesh.quaternion.setFromUnitVectors(AXIS_Y, dir.normalize());
        const thickness =
          seg.role === "suspect"
            ? 2
            : seg.role === "focus"
              ? 1.45
              : 1;
        mesh.scale.set(thickness, length, thickness);
        mesh.visible = true;
        used++;
      }
      for (let i = used; i < pool.length; i++) pool[i].visible = false;
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
      previewMesh.geometry.dispose();
      previewMaterial.dispose();
      if (frameHandle !== null) cancelAnimationFrame(frameHandle);
      canvas.removeEventListener("webglcontextrestored", onContextRestored);
      controls.removeEventListener("change", render);
      controls.dispose();
      api.setSoft(null); // たわみの表示物も片付ける(外していた面・線が入れ物へ戻る)
      clearGroup(contentGroup);
      api.content = null;
      highlightGroup.clear(); // 形と材質は共有しているのでここでは壊さない
      highlightGeometry.dispose();
      highlightMaterial.dispose();
      referenceHighlightMaterial.dispose();
      focusHighlightMaterial.dispose();
      suspectHighlightMaterial.dispose();
      activeHighlightMaterial.dispose();
      renderer.dispose();
    },
  };
  return api;
}
