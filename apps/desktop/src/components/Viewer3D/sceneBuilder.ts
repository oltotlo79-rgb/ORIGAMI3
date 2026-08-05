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
import type { DisplaySettings, Document, Face, Frame3D, Vec2 } from "../../lib/types";
import { paperExtent } from "../CpEditor/snap";
import type { HingeSegment } from "./hingePicker";

/** 面の境界線の色 */
const OUTLINE_COLOR = 0x1a1a1a;
/** 選択中ヒンジの強調色(黄色) */
const HIGHLIGHT_COLOR = 0xffd400;
/** 背景色(2D区画と揃える) */
const BACKGROUND_COLOR = 0xcaccd4;
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

/** 面のマテリアル。境界線が面に埋もれないよう面を少しだけ奥へずらして描く */
function faceMaterial(rgb: [number, number, number], side: THREE.Side) {
  return new THREE.MeshLambertMaterial({
    color: toColor(rgb),
    side,
    polygonOffset: true,
    polygonOffsetFactor: 1,
    polygonOffsetUnits: 1,
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
 */
export function updateFrame(content: Viewer3DContent, frame: Frame3D | null): void {
  const { positions, topology } = content;
  if (frame === null) {
    positions.set(topology.flatPositions);
  } else {
    for (const f of frame.faces) {
      const slot = topology.slots.get(f.face);
      // 頂点数が合わない面は対応が取れないので前の座標のままにする
      // (展開図を編集した直後など、立体形状の計算が届くまでは平らのまま)
      if (!slot || slot.count !== f.polygon.length) continue;
      let k = slot.offset * 3;
      for (const p of f.polygon) {
        positions[k++] = p[0];
        positions[k++] = p[1];
        positions[k++] = p[2];
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
// シーン(レンダラ・カメラ・照明・入れ物)
// ---------------------------------------------------------------------------

export interface Viewer3DScene {
  readonly camera: THREE.PerspectiveCamera;
  /** 面と境界線を入れる入れ物(作り直しのたびに中身を破棄する) */
  readonly contentGroup: THREE.Group;
  /** 選択中ヒンジの強調を入れる入れ物 */
  readonly highlightGroup: THREE.Group;
  /** 表示中の面・線。展開図が変わるまで作り替えない */
  content: Viewer3DContent | null;
  /** 次の描画機会に1回だけ描く(1フレーム1回にまとめる) */
  render(): void;
  resize(widthPx: number, heightPx: number): void;
  /** 紙全体が見える斜め上の初期位置へカメラを戻す */
  resetCamera(paperWidth: number, paperHeight: number): void;
  /** 面と線を差し替える(古い資源は破棄する) */
  setContent(content: Viewer3DContent): void;
  /** 選択中ヒンジの強調を更新する(形と材質は使い回す) */
  setHighlight(segments: HingeSegment[]): void;
  /** 折り線の描画中は左ドラッグの視点回転を止める(拡大縮小・平行移動は残す) */
  setDrawMode(enabled: boolean): void;
  dispose(): void;
}

/** グループの中身を破棄して空にする(ジオメトリ・マテリアルのリーク防止) */
export function clearGroup(group: THREE.Group): void {
  for (const child of [...group.children]) {
    group.remove(child);
    if (child instanceof THREE.Mesh || child instanceof THREE.LineSegments) {
      child.geometry.dispose();
      const material = child.material;
      if (Array.isArray(material)) {
        for (const m of material) m.dispose();
      } else {
        material.dispose();
      }
    }
  }
}

/** canvasにレンダラ・カメラ・軌道操作・照明を用意する */
export function createScene(canvas: HTMLCanvasElement): Viewer3DScene {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio || 1);

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(BACKGROUND_COLOR);

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
  scene.add(contentGroup, highlightGroup);

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
  const dir = new THREE.Vector3();

  const api: Viewer3DScene = {
    camera,
    contentGroup,
    highlightGroup,
    content: null,
    render,
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
      clearGroup(contentGroup);
      api.content = content;
      contentGroup.add(content.mesh, content.line);
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
          mesh.renderOrder = 1;
          mesh.frustumCulled = false;
          highlightGroup.add(mesh);
        }
        mesh.position.copy(seg.a);
        mesh.quaternion.setFromUnitVectors(AXIS_Y, dir.normalize());
        mesh.scale.set(1, length, 1);
        mesh.visible = true;
        used++;
      }
      for (let i = used; i < pool.length; i++) pool[i].visible = false;
      render();
    },
    setDrawMode(enabled) {
      controls.mouseButtons.LEFT = enabled ? null : THREE.MOUSE.ROTATE;
    },
    dispose() {
      if (frameHandle !== null) cancelAnimationFrame(frameHandle);
      canvas.removeEventListener("webglcontextrestored", onContextRestored);
      controls.removeEventListener("change", render);
      controls.dispose();
      clearGroup(contentGroup);
      api.content = null;
      highlightGroup.clear(); // 形と材質は共有しているのでここでは壊さない
      highlightGeometry.dispose();
      highlightMaterial.dispose();
      renderer.dispose();
    },
  };
  return api;
}
