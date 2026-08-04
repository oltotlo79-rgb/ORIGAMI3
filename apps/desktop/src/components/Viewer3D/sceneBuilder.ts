// 3Dビューのシーン構築(Three.js)。Reactの外側でThree.jsの資源を管理し、
// 作り直すたびに古いジオメトリ・マテリアルを必ず破棄する。
// 座標系: 展開図の(x, y)がそのまま3Dの(x, y)、紙の法線が+z(表side)。

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { DisplaySettings, Document, Face, Frame3D } from "../../lib/types";
import { paperExtent } from "../CpEditor/snap";

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

export interface Viewer3DScene {
  readonly camera: THREE.PerspectiveCamera;
  /** 面と境界線を入れる入れ物(作り直しのたびに中身を破棄する) */
  readonly contentGroup: THREE.Group;
  /** 選択中ヒンジの強調を入れる入れ物 */
  readonly highlightGroup: THREE.Group;
  render(): void;
  resize(widthPx: number, heightPx: number): void;
  /** 紙全体が見える斜め上の初期位置へカメラを戻す */
  resetCamera(paperWidth: number, paperHeight: number): void;
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

  const render = () => renderer.render(scene, camera);
  controls.addEventListener("change", render);

  return {
    camera,
    contentGroup,
    highlightGroup,
    render,
    resize(widthPx, heightPx) {
      if (widthPx === 0 || heightPx === 0) return;
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
    dispose() {
      controls.removeEventListener("change", render);
      controls.dispose();
      clearGroup(contentGroup);
      clearGroup(highlightGroup);
      renderer.dispose();
    },
  };
}

/** 0〜255のRGBをThree.jsの色へ */
function toColor(rgb: [number, number, number]): THREE.Color {
  return new THREE.Color(rgb[0] / 255, rgb[1] / 255, rgb[2] / 255);
}

/** 頂点列と三角形の添字からジオメトリを作る */
function makeGeometry(positions: number[], indices: number[]): THREE.BufferGeometry {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  return geometry;
}

/**
 * Frame3Dから面(表・裏)と境界線を作り直す。
 * 表裏はDoubleSideを使わず、巻き方向を逆にした2枚のメッシュで描く
 * (表=front_color、裏=back_color を確実に塗り分けるため)。
 *
 * 既知の制限(v1): 多角形は扇形分割(頂点0から扇状に三角形化)する。
 * 凹多角形では正しく塗られない場合があるが、実用上の展開図では稀なため許容する。
 */
export function buildContent(
  scene: Viewer3DScene,
  frame: Frame3D,
  display: DisplaySettings,
): void {
  clearGroup(scene.contentGroup);

  const positions: number[] = [];
  const frontIndices: number[] = [];
  const backIndices: number[] = [];
  const linePositions: number[] = [];
  for (const face of frame.faces) {
    const poly = face.polygon;
    if (poly.length < 3) continue;
    const base = positions.length / 3;
    for (const p of poly) positions.push(p[0], p[1], p[2]);
    for (let i = 1; i + 1 < poly.length; i++) {
      frontIndices.push(base, base + i, base + i + 1);
      backIndices.push(base, base + i + 1, base + i); // 巻き方向を反転=裏面
    }
    for (let i = 0; i < poly.length; i++) {
      const a = poly[i];
      const b = poly[(i + 1) % poly.length];
      linePositions.push(a[0], a[1], a[2], b[0], b[1], b[2]);
    }
  }
  if (positions.length === 0) {
    scene.render();
    return;
  }

  // 境界線が面に埋もれないよう、面を少しだけ奥へずらして描く
  const faceMaterial = (rgb: [number, number, number]) =>
    new THREE.MeshLambertMaterial({
      color: toColor(rgb),
      side: THREE.FrontSide,
      polygonOffset: true,
      polygonOffsetFactor: 1,
      polygonOffsetUnits: 1,
    });

  scene.contentGroup.add(
    new THREE.Mesh(
      makeGeometry(positions, frontIndices),
      faceMaterial(display.front_color),
    ),
    new THREE.Mesh(
      makeGeometry([...positions], backIndices),
      faceMaterial(display.back_color),
    ),
    new THREE.LineSegments(
      new THREE.BufferGeometry().setAttribute(
        "position",
        new THREE.Float32BufferAttribute(linePositions, 3),
      ),
      new THREE.LineBasicMaterial({ color: OUTLINE_COLOR }),
    ),
  );
  scene.render();
}

/** 選択中ヒンジを黄色の太線(円柱)で強調する。segmentsが空なら強調を消す */
export function buildHighlight(
  scene: Viewer3DScene,
  segments: { a: THREE.Vector3; b: THREE.Vector3 }[],
): void {
  clearGroup(scene.highlightGroup);
  for (const seg of segments) {
    const dir = new THREE.Vector3().subVectors(seg.b, seg.a);
    const length = dir.length();
    if (length < 1e-9) continue;
    const geometry = new THREE.CylinderGeometry(
      HIGHLIGHT_RADIUS,
      HIGHLIGHT_RADIUS,
      length,
      8,
    );
    geometry.translate(0, length / 2, 0); // 原点を端点aに合わせる
    const mesh = new THREE.Mesh(
      geometry,
      // 紙に隠れても見えるように深度判定を切る
      new THREE.MeshBasicMaterial({ color: HIGHLIGHT_COLOR, depthTest: false }),
    );
    mesh.position.copy(seg.a);
    mesh.quaternion.setFromUnitVectors(
      new THREE.Vector3(0, 1, 0),
      dir.normalize(),
    );
    mesh.renderOrder = 1;
    scene.highlightGroup.add(mesh);
  }
  scene.render();
}

/** 折る前(平ら)のFrame3D。ソルバー結果がまだ無いときの表示に使う */
export function flatFrame(doc: Document, faces: Face[]): Frame3D {
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  if (faces.length === 0) {
    // 面がまだ取れていない場合は紙の外形だけを平らに描く
    const [w, h] = paperExtent(doc);
    return {
      faces: [
        {
          face: 0,
          polygon: [
            [0, 0, 0],
            [w, 0, 0],
            [w, h, 0],
            [0, h, 0],
          ],
          layer: 0,
        },
      ],
      warnings: [],
    };
  }
  return {
    faces: faces.map((f) => ({
      face: f.id,
      polygon: f.vertices.flatMap((id): [number, number, number][] => {
        const p = pos.get(id);
        return p ? [[p[0], p[1], 0]] : [];
      }),
      layer: 0,
    })),
    warnings: [],
  };
}
