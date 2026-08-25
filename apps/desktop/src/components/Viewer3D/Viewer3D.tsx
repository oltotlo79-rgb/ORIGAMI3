// 3Dビュー区画: Three.jsのシーンをcanvasに描き、ヒンジのクリック選択と
// 「折る」ツールの折り線描画を受け付ける。
// Three.jsのオブジェクトはストアに入れずrefで保持する(要件§2: 状態はストア1本)。
//
// 更新の分担:
//   - 展開図(doc/faces/hinges)が変わったとき: 三角形分割と添字を作り直す
//   - 立体形状(frame3d)が変わったとき: 頂点座標の上書きだけ(作り直さない)

import { useCallback, useEffect, useRef, type ReactNode } from "react";
import * as THREE from "three";
import {
  canFoldNow,
  isSpatialFoldFrame,
  pullBlockedOf,
  type AlignCpPick,
  type SpatialFoldDrag,
  useAppStore,
} from "../../store/appStore";
import { SELECTABLE_3D_EDGE_TARGETS, viewerHint } from "../../lib/viewerHint";
import { measureGuide } from "../../lib/measureGuide";
import {
  hingeAnglesFromFrame,
  planPull,
  pullDeltaDeg,
  type PullPlan,
} from "../../lib/grabDrive";
import { paperExtent, snap } from "../CpEditor/snap";
import {
  foldedAlignPoint,
  SNAP_RADIUS_PX,
  TOOL_KIND,
} from "../CpEditor/interaction";
import { planeRadius, screenToPlane } from "../../lib/planeProject";
import { CONSTRUCT_STEPS, constructLines } from "../../lib/construct";
import { CURVE_STEPS, curvePolyline } from "../../lib/curve";
import type {
  EdgeKind,
  Face,
  FoldDirection,
  Frame3D,
  Vec2,
} from "../../lib/types";
import {
  facesAtPoint,
  foldLayers,
  foldPreviewSegments,
  keepSidePoint,
  snapFoldPoint,
} from "./foldDraw";
import {
  buildTopology,
  contentBoundingBox,
  createContent,
  createScene,
  createSoftContent,
  updateFrame,
  updateSoftContent,
  type SoftContent,
  type HighlightSegment,
  type Viewer3DScene,
} from "./sceneBuilder";
import { softSignature } from "./softMesh";
import {
  buildSoftHighlightMap,
  projectHighlightSegmentsToSoftSurface,
  type SoftHighlightMap,
} from "./softHighlight";
import { twistPreviewSegments } from "../../lib/twistPolygon";
import { ALIGN_STEPS, type AlignTarget } from "../../lib/alignFold";
import { nearestAlignPoint } from "../../lib/alignPick";
import { planGrabFold, type GrabMode } from "./grabFold";
import {
  pickFace,
  pickHingeSegment,
  pickPaper,
  type HingeSegment,
  type PaperPickSurface,
} from "./hingePicker";
import {
  deriveSelectedEdgeHighlights,
  pointInPolygon,
  type FacePlacement,
} from "./edgeHighlight";
import {
  buildCpFaceIndex,
  cpMarkSegments,
  cpPointOnFacePlane,
  isBorderVertex,
  pickCpFromPixel,
  placementOf,
  type CpFaceIndex,
  type CpPick3D,
} from "./cpPick3d";
import { ViewerOperationHint } from "./ViewerOperationHint";
import { PaperActionTip } from "./PaperActionTip";
import { FoldDirectionTip } from "./FoldDirectionTip";
import { ViewerOverlayStack } from "./ViewerOverlayStack";
import { ViewCube, type ViewCubeCameraControl } from "./ViewCube.jsx";
import { trackedOrbitTarget } from "./viewCube";

/** 畳み平面の線分列を強調表示用の線分へ(紙より少しだけ浮かせる) */
function toHighlight(segments: [Vec2, Vec2][]): HingeSegment[] {
  return segments.map(([a, b]) => ({
    edgeId: -1,
    a: new THREE.Vector3(a[0], a[1], PREVIEW_LIFT),
    b: new THREE.Vector3(b[0], b[1], PREVIEW_LIFT),
  }));
}

const SPATIAL_PREVIEW_EPS = 1e-9;

/** 面の頂点順を保った材質表側の法線。退化面では向きを変えない。 */
function materialNormal(
  frame: Frame3D | null,
  faceId: number,
): THREE.Vector3 | null {
  const polygon = frame?.faces.find((face) => face.face === faceId)?.polygon;
  if (!polygon || polygon.length < 3) return null;
  const normal = new THREE.Vector3();
  for (let i = 0; i < polygon.length; i++) {
    const a = polygon[i];
    const b = polygon[(i + 1) % polygon.length];
    normal.x += (a[1] - b[1]) * (a[2] + b[2]);
    normal.y += (a[2] - b[2]) * (a[0] + b[0]);
    normal.z += (a[0] - b[0]) * (a[1] + b[1]);
  }
  return normal.lengthSq() > SPATIAL_PREVIEW_EPS ** 2 ? normal.normalize() : null;
}

/**
 * 180°では終点の形だけから山谷を区別できないため、ドラッグ途中で材質の
 * 表側へ動いたか裏側へ動いたかを保持する。面内だけの移動では従来値を保つ。
 */
function spatialDragDirection(
  frame: Frame3D | null,
  faceId: number,
  from: SpatialFoldDrag["from"],
  to: SpatialFoldDrag["to"],
  previous: FoldDirection,
): FoldDirection {
  const normal = materialNormal(frame, faceId);
  if (!normal) return previous;
  const travel = new THREE.Vector3(...to).sub(new THREE.Vector3(...from));
  const towardFront = normal.dot(travel);
  if (Math.abs(towardFront) <= SPATIAL_PREVIEW_EPS) return previous;
  return towardFront > 0 ? "Up" : "Down";
}

function clipSpatialPolygon(
  polygon: readonly [number, number, number][],
  origin: THREE.Vector3,
  normal: THREE.Vector3,
  movingSign: number,
): THREE.Vector3[] {
  const clipped: THREE.Vector3[] = [];
  for (let i = 0; i < polygon.length; i++) {
    const a = new THREE.Vector3(...polygon[i]);
    const b = new THREE.Vector3(...polygon[(i + 1) % polygon.length]);
    const da = movingSign * normal.dot(a.clone().sub(origin));
    const db = movingSign * normal.dot(b.clone().sub(origin));
    const aInside = da >= -SPATIAL_PREVIEW_EPS;
    const bInside = db >= -SPATIAL_PREVIEW_EPS;
    if (aInside) clipped.push(a);
    if (aInside !== bInside) {
      const t = da / (da - db);
      clipped.push(a.lerp(b, t));
    }
  }
  return clipped;
}

function spatialCrease(
  polygon: readonly [number, number, number][],
  origin: THREE.Vector3,
  normal: THREE.Vector3,
): [THREE.Vector3, THREE.Vector3] | null {
  const crossings: THREE.Vector3[] = [];
  const add = (point: THREE.Vector3) => {
    if (!crossings.some((candidate) => candidate.distanceToSquared(point) <= 1e-18)) {
      crossings.push(point);
    }
  };
  for (let i = 0; i < polygon.length; i++) {
    const a = new THREE.Vector3(...polygon[i]);
    const b = new THREE.Vector3(...polygon[(i + 1) % polygon.length]);
    const da = normal.dot(a.clone().sub(origin));
    const db = normal.dot(b.clone().sub(origin));
    if (Math.abs(da) <= SPATIAL_PREVIEW_EPS) add(a);
    if (da * db < -(SPATIAL_PREVIEW_EPS ** 2)) add(a.lerp(b, da / (da - db)));
  }
  if (crossings.length < 2) return null;
  let pair: [THREE.Vector3, THREE.Vector3] = [crossings[0], crossings[1]];
  let farthest = pair[0].distanceToSquared(pair[1]);
  for (let i = 0; i < crossings.length; i++) {
    for (let j = i + 1; j < crossings.length; j++) {
      const distance = crossings[i].distanceToSquared(crossings[j]);
      if (distance > farthest) {
        farthest = distance;
        pair = [crossings[i], crossings[j]];
      }
    }
  }
  return farthest > SPATIAL_PREVIEW_EPS ** 2 ? pair : null;
}

/** 立体用計算と同じ「つかんだ面から同じ側へ共有辺でつながる面」。 */
function spatialPreviewFaces(
  frame: Frame3D,
  faces: readonly Face[],
  grab: Extract<GrabState, { spatial: true }>,
  origin: THREE.Vector3,
  normal: THREE.Vector3,
  movingSign: number,
): Set<number> {
  const frameById = new Map(frame.faces.map((face) => [face.face, face]));
  const faceById = new Map(faces.map((face) => [face.id, face]));
  const owners = new Map<number, number[]>();
  for (const face of faces) {
    for (const edge of face.edges) {
      const list = owners.get(edge) ?? [];
      list.push(face.id);
      owners.set(edge, list);
    }
  }
  const reachesSide = (faceId: number) =>
    frameById
      .get(faceId)
      ?.polygon.some(
        (point) =>
          movingSign *
            normal.dot(new THREE.Vector3(...point).sub(origin)) >
          SPATIAL_PREVIEW_EPS,
      ) ?? false;
  if (!reachesSide(grab.face)) return new Set();
  const selected = new Set([grab.face]);
  const queue = [grab.face];
  while (queue.length > 0) {
    const faceId = queue.shift()!;
    const face = faceById.get(faceId);
    const face3d = frameById.get(faceId);
    if (!face || !face3d || face.vertices.length !== face3d.polygon.length) continue;
    for (let edgeIndex = 0; edgeIndex < face.edges.length; edgeIndex++) {
      const a = new THREE.Vector3(...face3d.polygon[edgeIndex]);
      const b = new THREE.Vector3(
        ...face3d.polygon[(edgeIndex + 1) % face3d.polygon.length],
      );
      const edgeReaches = [a, b].some(
        (point) =>
          movingSign * normal.dot(point.clone().sub(origin)) > SPATIAL_PREVIEW_EPS,
      );
      if (!edgeReaches) continue;
      for (const neighbor of owners.get(face.edges[edgeIndex]) ?? []) {
        if (!selected.has(neighbor) && reachesSide(neighbor)) {
          selected.add(neighbor);
          queue.push(neighbor);
        }
      }
    }
  }
  return selected;
}

/** 立体の折り平面と、反射後の動く紙の輪郭を3D線で下見する。 */
function spatialFoldPreview(
  frame: Frame3D | null,
  faces: readonly Face[],
  grab: Extract<GrabState, { spatial: true }>,
): HighlightSegment[] {
  if (!frame) return [];
  const from = new THREE.Vector3(...grab.a);
  const to = new THREE.Vector3(...grab.b);
  const normal = to.clone().sub(from);
  if (normal.lengthSq() <= SPATIAL_PREVIEW_EPS ** 2) return [];
  normal.normalize();
  const origin = from.clone().add(to).multiplyScalar(0.5);
  const signed = normal.dot(from.clone().sub(origin));
  const movingSign = Math.abs(signed) > SPATIAL_PREVIEW_EPS ? Math.sign(signed) : -1;
  const selected = spatialPreviewFaces(frame, faces, grab, origin, normal, movingSign);
  const segments: HighlightSegment[] = [];
  for (const face of frame.faces) {
    if (!selected.has(face.face)) continue;
    const crease = spatialCrease(face.polygon, origin, normal);
    if (crease) {
      segments.push({ edgeId: -1, a: crease[0], b: crease[1], role: "reference" });
    }
    const moving = clipSpatialPolygon(face.polygon, origin, normal, movingSign).map(
      (point) => point.sub(normal.clone().multiplyScalar(2 * normal.dot(point.clone().sub(origin)))),
    );
    for (let i = 0; i < moving.length; i++) {
      segments.push({
        edgeId: -1,
        a: moving[i],
        b: moving[(i + 1) % moving.length],
        role: "active",
      });
    }
  }
  return segments;
}

/** たわみONでは、見えている細分網をowner/pickerの両方で使う。 */
function displayedPickSurface(scene: Viewer3DScene): PaperPickSurface | null {
  if (scene.pickSurface) return scene.pickSurface;
  const content = scene.content;
  if (!content) return null;
  return {
    mesh: content.mesh,
    triangleFaceIds: content.topology.triangleFaceIds,
    triangleLayers: content.owner?.triangleLayers ??
      new Array(content.topology.triangleFaceIds.length).fill(0),
    faceSurfaceRanks: content.owner?.faceSurfaceRanks ?? new Map<number, number>(),
  };
}

/** 指定した中心の位置を示す小さな十字(ねじり折りの下見) */
function centerMark(c: Vec2, r = CENTER_MARK): [Vec2, Vec2][] {
  return [
    [
      [c[0] - r, c[1]],
      [c[0] + r, c[1]],
    ],
    [
      [c[0], c[1] - r],
      [c[0], c[1] + r],
    ],
  ];
}

/** 2点の距離(展開図・畳み平面のどちらの座標でも使う) */
function distance2(a: Vec2, b: Vec2): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1]);
}

/** 修飾キーから「何枚の紙を動かすか」を決める(説明は常にヒント行に出す) */
function grabMode(e: { shiftKey: boolean; altKey: boolean }): GrabMode {
  if (e.shiftKey) return "all";
  if (e.altKey) return "single";
  return "flap";
}

/** これ以上動かしたら「クリック」ではなく視点操作とみなす(px) */
const CLICK_MOVE_PX = 4;
/** 折り線の端点を紙の点・輪郭へ吸着させる距離(px) */
const FOLD_SNAP_PX = 14;
/** 平面へ投影できないときの吸着半径(正規化座標) */
const FOLD_SNAP_FALLBACK = 0.02;
/** これ未満の長さの折り線は引かなかったことにする(正規化座標) */
const MIN_FOLD_LENGTH = 1e-4;
/** 合わせて折るときに、点・線を拾う許容距離(px) */
const ALIGN_PICK_PX = 16;
/** 技法のフラップ選択で、層の輪郭からこの距離以内なら「その場所にある」とみなす
 * (クリック位置は紙の点・輪郭へ吸着するので、境界ちょうどを指しても拾えるようにする) */
const FLAP_PICK_EPS = 1e-3;
/** ねじり折りの中心を示す十字の腕の長さ(正規化座標) */
const CENTER_MARK = 0.02;
/** プレビュー線を紙より少しだけ上に浮かせる高さ(重なりのちらつき防止) */
const PREVIEW_LIFT = 0.002;
/** 折った結果の下見(半透明の面)を浮かせる高さ。層のずらし表示より上に置く */
const PREVIEW_FILL_LIFT = 0.045;

type GrabState = {
  face: number;
  mode: GrabMode;
} & (
  | {
      spatial: false;
      a: Vec2;
      b: Vec2;
    }
  | {
      spatial: true;
      a: SpatialFoldDrag["from"];
      b: SpatialFoldDrag["to"];
      origin: THREE.Vector3;
      ndc: THREE.Vector3;
      x: number;
      y: number;
      direction: FoldDirection;
    }
);

/** 合わせて折るで1つ選んだ結果(そのままpickAlignTargetへ渡せる形) */
interface AlignPick {
  target: AlignTarget;
  /** 解を並べ替える基準になるクリック位置(畳み平面座標)。無ければnull */
  cursor: Vec2 | null;
  /** 展開図側の識別子。展開図の頂点・辺として拾えたときだけ入る */
  cpPick: AlignCpPick | null;
}

interface Props {
  /** 「全体表示」用: 親が current を呼ぶと紙全体が見える位置にカメラを戻す */
  fitRef: React.RefObject<(() => void) | null>;
  /** 3D区画上側の通知。狭い画面では他の案内と同じ列へ入れて重なりを防ぐ。 */
  statusOverlays?: ReactNode;
}

export function Viewer3D({ fitRef, statusOverlays }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sceneRef = useRef<Viewer3DScene | null>(null);
  /** OrbitControlsが内部に持つ注視点を、公開済みcameraの変化から追跡する。 */
  const orbitTargetRef = useRef(new THREE.Vector3());
  /** 表示中のたわみの網(Three.jsの資源なのでストアには入れずrefで持つ) */
  const softRef = useRef<SoftContent | null>(null);
  /** CP上の物理辺を、表示中のたわみ網へ厳密に写す対応。網の形が変わるまで再利用する。 */
  const softHighlightRef = useRef<SoftHighlightMap | null>(null);
  /** 実際に表示中の全辺。通常選択と合わせ入力が同じ線分・表面判定を共有する。 */
  const selectableEdgeSegmentsRef = useRef<HingeSegment[]>([]);
  const downPosRef = useRef<{ x: number; y: number } | null>(null);
  /** 折り線を引いている最中の2点(表示専用の一時状態なのでrefで持つ) */
  const drawingRef = useRef<{ a: Vec2; b: Vec2 } | null>(null);
  /** 展開図の頂点・辺と面の対応。展開図が変わるまで作り直さない */
  const cpIndexRef = useRef<CpFaceIndex | null>(null);
  /** 山・谷・補助で1クリック目に決めた点(展開図座標) */
  const pendingCpPointRef = useRef<Vec2 | null>(null);
  /** 曲線モードでクリック済みの点(展開図座標) */
  const curvePointsRef = useRef<Vec2[]>([]);
  /** 作図でクリック済みの点と線(展開図座標) */
  const constructRef = useRef<{ points: Vec2[]; seg: [Vec2, Vec2] | null }>({
    points: [],
    seg: null,
  });
  /** 折るツールで2クリック目を待っている折り線の始点(畳み平面座標) */
  const foldClickRef = useRef<Vec2 | null>(null);
  /** 3Dで点をつかんで動かしている最中(離すまで展開図は変えない) */
  const vertexDragRef = useRef<{
    id: number;
    faceId: number;
    from: Vec2;
    to: Vec2;
  } | null>(null);
  /** つかめる・選べるものの上にカーソルがあるので視点回転を止めているか。
   * 同じ指定を何度も出さないための覚え書き */
  const hoverLockRef = useRef(false);
  /** 合わせて折るで、押した瞬間に決まった選択。離した位置ではなく押した位置で決める
   * (押してから離すまでに手がぶれても、選ぼうとしたものが選ばれるようにする) */
  const alignPressRef = useRef<AlignPick | null>(null);
  /** 測定で押した瞬間に既存の逆写像が返した点・辺。 */
  const measurePressRef = useRef<CpPick3D | null>(null);
  /** 紙をつかんで動かしている最中のつかんだ点・今の点・つかんだ面・対象の枚数 */
  const grabRef = useRef<GrabState | null>(null);
  /** 紙を引いている最中の、つかんだ点(世界座標とその画面位置)と駆動する折り線。
   * ドラッグ中に形が変わっても基準はつかんだ瞬間のまま保つ(手が形に追われない) */
  const pullRef = useRef<{
    plan: PullPlan;
    origin: THREE.Vector3;
    ndc: THREE.Vector3;
    x: number;
    y: number;
  } | null>(null);

  // 購読は更新の合図として使う(値の読み出しはgetStateで行う)
  const doc = useAppStore((s) => s.doc);
  const faces = useAppStore((s) => s.faces);
  const hinges = useAppStore((s) => s.hinges);
  const frame3d = useAppStore((s) => s.frame3d);
  const softMesh = useAppStore((s) => s.softMesh);
  const selection = useAppStore((s) => s.selection);
  const hoveredHinge = useAppStore((s) => s.hoveredHinge);
  const suspectHinges = useAppStore((s) => s.suspectHinges);
  const pinnedFolds = useAppStore((s) => s.pinnedFolds);
  const foldAllActive = useAppStore((s) => s.foldAllPreview !== null);
  const activeAngleIntent = useAppStore((s) => s.activeAngleIntent);
  const docEpoch = useAppStore((s) => s.docEpoch);
  const activeTool = useAppStore((s) => s.activeTool);
  const measureDraft = useAppStore((s) => s.measureDraft);
  const uiTheme = useAppStore((s) => s.uiTheme);
  const foldDraft = useAppStore((s) => s.foldDraft);
  const pendingFoldThrough = useAppStore((s) => s.pendingFoldThrough);
  const alignDraft = useAppStore((s) => s.alignDraft);
  const techniqueDraft = useAppStore((s) => s.techniqueDraft);
  const foldReady = useAppStore(
    (s) =>
      canFoldNow(s) && !s.foldThroughBusy && s.pendingFoldThrough === null,
  );
  const pullHinge = useAppStore((s) => s.pullHinge);
  const pullMirrorHinge = useAppStore((s) => s.pullMirrorHinge);
  const pullBlocked = useAppStore(pullBlockedOf);
  // 「今どのモードで何ができるか」を1行で常に出す(UI-009)。
  // 文字列を返す選択なので、内容が変わらない限り再描画は起きない
  const hint = useAppStore((s) => {
    if (s.foldAllPreview !== null) {
      return s.foldAllPreview.returning
        ? "いつもの表示に戻しています。少し待ってください"
        : "下の「折る割合」を動かすと、全部の折り目が同じ割合で動きます";
    }
    if (s.foldThroughBusy) return "折り方を確認しています。少し待ってください";
    if (s.pendingFoldThrough) {
      return "追加折り目の位置を確認し、下のパネルで折り方を選んでください";
    }
    if (s.activeTool === "measure") {
      return measureGuide(s.measureDraft.mode, s.measureDraft.picks.length);
    }
    return viewerHint({
      pullBlocked: pullBlockedOf(s),
      pulling: s.pullHinge !== null,
      pullMirrored: s.pullMirrorHinge !== null,
      hasDoc: s.doc !== null,
      playing: s.playing,
      playT: s.playT,
      driverCount: s.drivers.size,
      currentStep: s.currentStep,
      stepCount: s.doc?.sequence.length ?? 0,
      tool: s.activeTool,
      hasFoldDraft: s.foldDraft !== null,
      hasTechnique: s.techniqueDraft !== null,
      techniqueFlapCount: s.techniqueDraft?.flap.length ?? 0,
      techniqueCandidateCount: s.techniqueDraft?.flapCandidates.length ?? 0,
      hasTechniqueLine: s.techniqueDraft?.line != null,
      techniqueKind: s.techniqueDraft?.kind ?? null,
      techniqueVertexCount: s.techniqueDraft?.polygon.length ?? 0,
      techniqueHasCenter: s.techniqueDraft?.center != null,
      techniqueHasReference: s.techniqueDraft?.referencePoint != null,
      alignMode: s.alignDraft?.mode ?? null,
      alignPickCount: s.alignDraft?.picks.length ?? 0,
      alignSolutionCount: s.alignDraft?.solutions.length ?? 0,
      alignReason: s.alignDraft?.reason ?? null,
    });
  });
  // 「折る」と「技法」はどちらも紙の上に折り線を引く(左ドラッグを線引きに使う)
  const foldMode = activeTool === "fold" || activeTool === "technique";
  // 「引く」は紙をつかんで動かす(左ドラッグを紙の操作に使う)
  const pullMode = activeTool === "pull";

  // シーンの初期化と破棄
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const scene = createScene(canvas);
    sceneRef.current = scene;
    scene.resize(canvas.clientWidth, canvas.clientHeight);
    const rememberOrbitTarget = () => {
      // OrbitControlsのnative listenerが同じイベントを処理し終えた姿勢を読む。
      queueMicrotask(() => {
        if (sceneRef.current !== scene) return;
        orbitTargetRef.current.copy(
          trackedOrbitTarget(
            orbitTargetRef.current,
            scene.camera.position,
            scene.camera.getWorldDirection(new THREE.Vector3()),
          ),
        );
      });
    };
    canvas.addEventListener("pointermove", rememberOrbitTarget);
    canvas.addEventListener("wheel", rememberOrbitTarget);
    return () => {
      canvas.removeEventListener("pointermove", rememberOrbitTarget);
      canvas.removeEventListener("wheel", rememberOrbitTarget);
      sceneRef.current = null;
      softRef.current = null;
      softHighlightRef.current = null;
      scene.dispose();
    };
  }, []);

  // テーマ変更後のCSS変数を読み直して、WebGL背景も直ちに描き替える。
  useEffect(() => {
    sceneRef.current?.syncTheme();
  }, [uiTheme]);

  // 展開図が変わったときだけ、三角形分割・境界線・ヒンジ対応を作り直す
  useEffect(() => {
    const scene = sceneRef.current;
    softHighlightRef.current = null;
    if (!scene || !doc) return;
    const content = createContent(
      buildTopology(doc, faces, hinges),
      doc.display,
      scene.ownerBinding,
    );
    softRef.current = null; // setContentがたわみの表示物を捨てるので参照も外す
    scene.setContent(content);
    updateFrame(content, useAppStore.getState().frame3d);
    scene.render();
  }, [doc, faces, hinges]);

  // 立体形状が変わったら頂点座標を上書きするだけ(手順再生でもここだけ動く)
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene?.content) return;
    updateFrame(scene.content, frame3d);
    scene.render();
  }, [frame3d]);

  // 紙のたわみ(SIM-012)。網が届いている間は細かい三角形の網を描き、
  // 切ったら従来の描き方へ戻す。網の形が同じ間は座標の書き換えだけで済ませる
  // (膨らみのつまみを動かしても毎回作り直さない)
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene) return;
    const s = useAppStore.getState();
    if (!softMesh || !s.doc) {
      softRef.current = null;
      softHighlightRef.current = null;
      scene.setSoft(null);
      return;
    }
    let content = softRef.current;
    let replaced = false;
    if (!content || content.signature !== softSignature(softMesh)) {
      content = createSoftContent(
        softMesh,
        s.doc.display,
        scene.ownerBinding,
        scene.content?.owner.ownerCodes,
      );
      softRef.current = content;
      scene.setSoft(content);
      replaced = true;
    }
    updateSoftContent(content, softMesh, s.frame3d);
    if (replaced) {
      softHighlightRef.current = buildSoftHighlightMap(s.doc, s.faces, softMesh, content);
    }
    scene.render();
  }, [softMesh, doc, faces, hinges]);

  /** 展開図の頂点・辺と面の対応。同じ展開図の間は作り直さない */
  const cpIndex = useCallback((): CpFaceIndex | null => {
    const s = useAppStore.getState();
    if (!s.doc) return null;
    const cached = cpIndexRef.current;
    if (cached && cached.doc === s.doc && cached.faces === s.faces) return cached;
    const built = buildCpFaceIndex(s.doc, s.faces);
    cpIndexRef.current = built;
    return built;
  }, []);

  /** 面の現在位置(展開図↔3Dの写像)。点を動かしている間の追従に使う */
  const facePlacementOf = useCallback(
    (faceId: number): FacePlacement | null => {
      const scene = sceneRef.current;
      const index = cpIndex();
      if (!scene?.content || !index) return null;
      return placementOf(
        index,
        faceId,
        scene.content.topology.slots,
        scene.content.positions,
      );
    },
    [cpIndex],
  );

  /**
   * 展開図の点を、いま3Dで見えている位置の十字として描く線分にする。
   * 面が傾いていても面の上に貼り付くので、立体姿勢でも点の位置が分かる。
   * どの面に載せるかを指定しなければ、その点を含む面のうち最初に見つかったものへ載せる。
   */
  const cpPointHighlight = useCallback(
    (cp: Vec2, faceId?: number): HighlightSegment[] => {
      const index = cpIndex();
      if (!index) return [];
      const faceIds =
        faceId === undefined ? index.faces.map((face) => face.id) : [faceId];
      for (const id of faceIds) {
        if (faceId === undefined) {
          const polygon = index.polygons.get(id);
          if (!polygon || !pointInPolygon(polygon, cp)) continue;
        }
        const placement = facePlacementOf(id);
        if (!placement) continue;
        // 紙の面から PREVIEW_LIFT(0.002)だけ浮かせる。強調線の太さ0.006より小さいので
        // 見た目の位置は動かず、面と同じ高さに置いたときのちらつきだけが消える。
        // 色は選択中の折り線と同じ黄色(既定)にする。紙の赤に近い色だと見分けられない。
        const marks = cpMarkSegments(placement, cp, CENTER_MARK, PREVIEW_LIFT);
        if (marks.length === 0) continue;
        return marks.map(([a, b]) => ({
          edgeId: -1,
          a: new THREE.Vector3(...a),
          b: new THREE.Vector3(...b),
        }));
      }
      return [];
    },
    [cpIndex, facePlacementOf],
  );

  /** 強調表示を描き直す: 折り線を引いている間はその線と動く層、それ以外は選択中の折り線 */
  const drawHighlight = useCallback(() => {
    const scene = sceneRef.current;
    if (!scene?.content) return;
    const s = useAppStore.getState();
    const physicalEdgeSegments: HighlightSegment[] = s.doc
      ? deriveSelectedEdgeHighlights(
          s.doc,
          s.faces,
          scene.content.topology.slots,
          scene.content.positions,
          s.hinges,
          s.doc.cp.edges.map((edge) => edge.id),
          scene.content.topology.lineProbeIndices,
        ).map((target) => ({
          edgeId: target.edgeId,
          ownerFace: target.ownerFace,
          role: target.role,
          a: new THREE.Vector3(...target.a),
          b: new THREE.Vector3(...target.b),
          ...(target.surfaceProbe === undefined
            ? {}
            : { surfaceProbe: new THREE.Vector3(...target.surfaceProbe) }),
        }))
      : [];
    // pickerへ渡す線と、基本outlineに含まれない線の表示は同じrigid/soft投影を使う。
    // ownerの無いfallback線は紙面との対応を確認できず表示もしないため、hit対象にも入れない。
    const displayedEdgeSegments = projectHighlightSegmentsToSoftSurface(
      physicalEdgeSegments,
      softHighlightRef.current,
    ).filter((segment) => segment.ownerFace !== undefined);
    selectableEdgeSegmentsRef.current = displayedEdgeSegments;
    const outlinedEdgeIds = new Set(s.faces.flatMap((face) => face.edges));
    const selectedIds = new Set(s.selection.edgeIds);
    scene.setSupplementalEdges(
      displayedEdgeSegments.filter(
        (segment) =>
          !outlinedEdgeIds.has(segment.edgeId) && !selectedIds.has(segment.edgeId),
      ),
    );
    // 一斉表示は通常姿勢の固定と貫通候補を計算入力に使わないため、古い色を重ねない。
    const normalPoseMarksVisible = s.foldAllPreview === null;
    const suspectIds = new Set(normalPoseMarksVisible ? s.suspectHinges : []);
    const suspectSegments: HighlightSegment[] = scene.content.hingeSegments
      .filter((segment) => suspectIds.has(segment.edgeId))
      .map((segment) => ({ ...segment, role: "suspect" as const }));
    const activeIds = new Set(s.activeAngleIntent?.hinges ?? []);
    if (s.pullHinge !== null) activeIds.add(s.pullHinge);
    if (s.pullMirrorHinge !== null) activeIds.add(s.pullMirrorHinge);
    const activeSegments: HighlightSegment[] = scene.content.hingeSegments
      .filter((segment) => activeIds.has(segment.edgeId) && !suspectIds.has(segment.edgeId))
      .map((segment) => ({ ...segment, role: "active" as const }));
    // 角度を固定した折り目は、選んでいなくても光らせる(どれを固定したかが
    // 選び直さなくても分かるように)。いま動かしている折り目・食い込みの
    // 原因候補は、そちらの色を優先する。
    const pinnedSegments: HighlightSegment[] = scene.content.hingeSegments
      .filter(
        (segment) =>
          normalPoseMarksVisible &&
          s.pinnedFolds.has(segment.edgeId) &&
          !suspectIds.has(segment.edgeId) &&
          !activeIds.has(segment.edgeId),
      )
      .map((segment) => ({ ...segment, role: "pinned" as const }));
    const hoveredSegments: HighlightSegment[] =
      s.hoveredHinge !== null && !s.selection.edgeIds.includes(s.hoveredHinge)
        ? scene.content.hingeSegments
            .filter(
              (segment) =>
                segment.edgeId === s.hoveredHinge && !suspectIds.has(segment.edgeId),
            )
            .map((segment) => ({ ...segment, role: "focus" as const }))
        : [];
    // 3Dで指した点を、面の上に貼り付く十字で示す。選択中の頂点・引きかけの点・
    // 作図で選んだ点が「どこを指しているか」を、3Dだけを見て確かめられるようにする。
    const cpMarks: HighlightSegment[] = [];
    if (s.doc) {
      const cpPositions = new Map(s.doc.cp.vertices.map((v) => [v.id, v.pos]));
      if (s.activeTool !== "measure") {
        for (const id of s.selection.vertexIds) {
          const pos = cpPositions.get(id);
          if (pos) cpMarks.push(...cpPointHighlight(pos));
        }
      }
      for (const pick of s.measureDraft.picks) {
        if (pick.kind === "point") {
          cpMarks.push(
            ...cpPointHighlight(
              pick.cp,
              pick.faceId === null ? undefined : pick.faceId,
            ),
          );
        }
      }
      const vertexDrag = vertexDragRef.current;
      if (vertexDrag) cpMarks.push(...cpPointHighlight(vertexDrag.to, vertexDrag.faceId));
      if (pendingCpPointRef.current) {
        cpMarks.push(...cpPointHighlight(pendingCpPointRef.current));
      }
      for (const p of curvePointsRef.current) cpMarks.push(...cpPointHighlight(p));
      for (const p of constructRef.current.points) cpMarks.push(...cpPointHighlight(p));
      for (const p of constructRef.current.seg ?? []) cpMarks.push(...cpPointHighlight(p));
    }
    // 折り線の始点だけは畳み平面の座標で決まるので、従来の折り線表示と同じ面に出す
    if (foldClickRef.current) cpMarks.push(...toHighlight(centerMark(foldClickRef.current)));
    const setHighlight = (segments: HighlightSegment[]) => {
      const shownIds = new Set(segments.map((segment) => segment.edgeId));
      const physicalSegments = [
        ...suspectSegments,
        // 選択中の折り目は選択の色を優先する(同じ線を二重に描かない)。
        // 選んでいない固定の折り目だけを、固定の色で足す。
        ...pinnedSegments.filter((segment) => !shownIds.has(segment.edgeId)),
        ...segments.filter((segment) => !suspectIds.has(segment.edgeId)),
        ...hoveredSegments,
        ...activeSegments,
        ...cpMarks,
      ];
      scene.setHighlight(
        projectHighlightSegmentsToSoftSurface(physicalSegments, softHighlightRef.current),
      );
    };
    const drawing = drawingRef.current;
    // つかんで動かしている間は「折った結果の形」を半透明で重ねて見せる(UI-008)
    const grab = grabRef.current;
    if (grab && s.doc) {
      if (grab.spatial) {
        // setPreviewはz=0専用なので消し、立体では折り平面と反射後の輪郭を
        // 3Dの強調線で示す。現在の紙と重ねることで動く側と結果形が分かる。
        scene.setPreview([], PREVIEW_FILL_LIFT);
        setHighlight(spatialFoldPreview(s.frame3d, s.faces, grab));
        return;
      }
      const plan = planGrabFold(
        foldLayers(s.frame3d, s.doc, s.faces),
        s.faces,
        grab.a,
        grab.b,
        grab.mode,
        grab.face,
      );
      scene.setPreview(plan.ok ? plan.plan.preview : [], PREVIEW_FILL_LIFT);
      setHighlight(plan.ok ? toHighlight(plan.plan.segments) : []);
      return;
    }
    scene.setPreview([], PREVIEW_FILL_LIFT);
    // 巻き込み用の追加折り目。Rustが現在の畳み平面へ写した線を、既存の
    // 参照線ハイライト(水色)で示す。展開図側は別のCP座標を使う。
    if (s.pendingFoldThrough) {
      setHighlight(
        toHighlight([s.pendingFoldThrough.proposal.folded_line]).map((segment) => ({
          ...segment,
          role: "reference" as const,
        })),
      );
      return;
    }
    // 合わせて折る: 選んだ点(十字)・線を光らせ、求まった折り線は下見に重ねる
    if (s.activeTool === "fold" && s.alignDraft && s.doc) {
      const segments: [Vec2, Vec2][] = [];
      for (const t of s.alignDraft.picks) {
        if (t.kind === "point") segments.push(...centerMark(t.p));
        else segments.push([t.a, t.b]);
      }
      if (s.foldDraft) {
        segments.push(
          ...foldPreviewSegments(
            foldLayers(s.frame3d, s.doc, s.faces),
            s.foldDraft.line,
            keepSidePoint(s.foldDraft.line, s.foldDraft.movingSide),
            s.foldDraft.target === "top",
          ),
        );
      }
      setHighlight(toHighlight(segments));
      return;
    }
    // 技法では、選んだフラップ(重なった層)の輪郭も光らせる
    if (s.activeTool === "technique" && s.techniqueDraft && s.doc) {
      const draft = s.techniqueDraft;
      const layers = foldLayers(s.frame3d, s.doc, s.faces);
      const segments: [Vec2, Vec2][] = [];
      // 層操作で追加済みのReflect軸も、現在選択中の軸と一緒に表示する。
      if (draft.kind === "Simple") {
        for (const part of draft.motionParts) {
          if (part.transform !== "Stay") segments.push(...part.transform.Reflect);
        }
      }
      // ねじり折り: 指した中央多角形と、そこから出るひだの折り線を下見する
      if (draft.kind === "Twist" && draft.polygon.length > 0) {
        segments.push(...twistPreviewSegments(draft.polygon, draft.center));
        if (draft.center) segments.push(...centerMark(draft.center));
      }
      // Ctrl+クリックで明示した基準点。「こちら側／反対側」より優先される位置を
      // 適用前に確認できるよう、折り線と同じ黄色の十字で見せる。
      if (draft.referencePoint) segments.push(...centerMark(draft.referencePoint));
      const shown = drawing ? [drawing.a, drawing.b] : draft.line;
      if (shown) segments.push([shown[0], shown[1]]);
      const selectedMotionLayers = new Set([
        ...draft.flap,
        ...(draft.kind === "Simple"
          ? draft.motionParts.flatMap((part) => part.layers)
          : []),
      ]);
      for (const l of layers.filter((l) => selectedMotionLayers.has(l.face))) {
        for (let i = 0; i < l.polygon.length; i++) {
          segments.push([l.polygon[i], l.polygon[(i + 1) % l.polygon.length]]);
        }
      }
      setHighlight(toHighlight(segments));
      return;
    }
    const line: [Vec2, Vec2] | null = drawing
      ? [drawing.a, drawing.b]
      : s.activeTool === "fold" && s.foldDraft
        ? s.foldDraft.line
        : null;
    if (line && s.doc) {
      // 引いている最中はどちら側を動かすかまだ決まっていない(線だけ出す)
      const keep =
        drawing || !s.foldDraft
          ? null
          : keepSidePoint(line, s.foldDraft.movingSide);
      const segments = foldPreviewSegments(
        foldLayers(s.frame3d, s.doc, s.faces),
        line,
        keep,
        s.foldDraft?.target === "top",
      );
      setHighlight(toHighlight(segments));
      return;
    }
    // 引いている間は、いま角度を変えている折り線だけを色で示す(UI-007)。
    // 左右同時のときは対称の相手にも同じ色を付け、両方動くことを見せる
    if (s.pullHinge !== null) {
      setHighlight([]);
      return;
    }
    // 2Dで選んだ辺は種類を問わず現在の3D位置へ写す。ヒンジは黄色、
    // 折る操作の対象にならない縁・補助線・非ヒンジ折り線は水色で区別する。
    if (!s.doc) {
      setHighlight([]);
      return;
    }
    setHighlight(
      physicalEdgeSegments
        .filter((segment) => selectedIds.has(segment.edgeId))
        .map((segment) => ({
          ...segment,
          role: segment.edgeId === s.hoveredHinge ? ("focus" as const) : segment.role,
        })),
    );
  }, [cpPointHighlight]);

  // 選択・折り線プレビューの強調(上の効果で線分が更新された後に走る)
  useEffect(() => {
    drawHighlight();
  }, [
    selection,
    hoveredHinge,
    suspectHinges,
    pinnedFolds,
    foldAllActive,
    activeAngleIntent,
    doc,
    faces,
    hinges,
    frame3d,
    foldDraft,
    pendingFoldThrough,
    alignDraft,
    techniqueDraft,
    activeTool,
    measureDraft,
    pullHinge,
    pullMirrorHinge,
    softMesh,
    drawHighlight,
  ]);

  // 折る・引くツールの間は左ドラッグを紙の操作に使うので、視点の回転を止める。
  // 引くツールでは代わりに右ドラッグで回せるようにする(色々な向きから引くため)
  useEffect(() => {
    // 選べるものの上にカーソルがある間だけ止めていた分も、道具が替わったらここで元へ戻す
    hoverLockRef.current = false;
    sceneRef.current?.setDrawMode(
      !foldAllActive &&
        ((foldMode && foldReady && !alignDraft) ||
          (pullMode && pullBlocked === null)),
      !foldAllActive && pullMode,
    );
  }, [foldMode, foldReady, pullMode, pullBlocked, alignDraft, activeTool, foldAllActive]);

  /**
   * つかめる・選べるものの上に来た間だけ、左ドラッグの視点回転を先に止める。
   * 押してから止めても、視点回転を始める処理がcanvasの入力を先に受け取るので
   * 間に合わない。紙・線・点の上でない場所では止めないので、視点は今までどおり回せる。
   */
  const setHoverLock = useCallback((locked: boolean) => {
    if (hoverLockRef.current === locked) return;
    hoverLockRef.current = locked;
    sceneRef.current?.setDrawMode(locked);
  }, []);

  // 折るツールから離れたら、引きかけの線とつかみかけの紙を捨てる
  useEffect(() => {
    if (!foldMode) {
      drawingRef.current = null;
      grabRef.current = null;
    }
  }, [foldMode]);

  // ねじり折りで中央多角形を置いている間のキー操作(Escでやめる・
  // Backspaceで直前の角を取り消す)。入力欄を打っている間は邪魔しない
  useEffect(() => {
    if (activeTool !== "technique") return;
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLElement && e.target.tagName === "INPUT") return;
      const s = useAppStore.getState();
      if (!s.techniqueDraft) return;
      if (e.key === "Escape") {
        s.cancelTechnique();
      } else if (e.key === "Backspace" && s.techniqueDraft.polygon.length > 0) {
        e.preventDefault();
        s.undoTechniqueVertex();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [activeTool]);

  // 合わせて折る途中のキー操作(Escでやめる・Backspaceで直前の選択を取り消す)。
  // 入力欄を打っている間は邪魔しない
  useEffect(() => {
    if (activeTool !== "fold") return;
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLElement && e.target.tagName === "INPUT") return;
      const s = useAppStore.getState();
      if (!s.alignDraft) return;
      if (e.key === "Escape") {
        s.cancelAlign();
      } else if (e.key === "Backspace" && s.alignDraft.picks.length > 0) {
        e.preventDefault();
        s.undoAlignPick();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [activeTool]);

  // 引くツールから離れたら、引きかけの状態を捨てる
  useEffect(() => {
    if (!pullMode) {
      pullRef.current = null;
      useAppStore.getState().endPull();
    }
  }, [pullMode]);

  /**
   * 左上に重ねている案内の札の下端(canvasの上からのCSS px)。
   * 札は紙の上に重なるので、視点合わせのときにここより下へ紙を逃がす。
   * 札が無い・まだ大きさが決まっていないときは0(=避けない)。
   */
  const hintBottomPx = useCallback((): number => {
    const canvas = canvasRef.current;
    const hint = canvas?.parentElement?.querySelector(".viewer-operation-hint");
    if (!canvas || !hint) return 0;
    const canvasTop = canvas.getBoundingClientRect().top;
    const bottom = hint.getBoundingClientRect().bottom - canvasTop;
    return Number.isFinite(bottom) && bottom > 0 ? bottom : 0;
  }, []);

  /**
   * 立体全体が見える斜め上の位置へカメラを戻す。
   *
   * 基準は展開図の大きさ((0,0)〜(紙の幅,紙の高さ))ではなく、いま実際に
   * 表示している立体の頂点座標そのものから求めた範囲にする。折る・技法で
   * 座標は展開図の範囲から離れて動くため、展開図の大きさを基準にすると
   * 立体の一部が画面の外へ出ることがある(「視点を戻す」を押したときに実際に発生)。
   * 立体がまだ無い(3D内容を作る前)ときだけ、展開図の平らな大きさへ戻す。
   */
  const fitCamera = useCallback(() => {
    const scene = sceneRef.current;
    const current = useAppStore.getState().doc;
    if (!scene || !current) return;
    const modelBox = scene.content ? contentBoundingBox(scene.content) : null;
    const box =
      modelBox && !modelBox.isEmpty()
        ? modelBox
        : (() => {
            const [w, h] = paperExtent(current);
            return new THREE.Box3(
              new THREE.Vector3(0, 0, 0),
              new THREE.Vector3(w, h, 0),
            );
          })();
    scene.resetCamera(box, hintBottomPx());
    orbitTargetRef.current.copy(box.getCenter(new THREE.Vector3()));
  }, [hintBottomPx]);

  // 新規作成・ファイルを開いた直後は紙全体が見える位置へカメラを戻す
  useEffect(() => {
    fitCamera();
  }, [docEpoch, fitCamera]);

  // 「全体表示」を親(ツールレール)から呼べるように登録する
  useEffect(() => {
    fitRef.current = fitCamera;
    return () => {
      fitRef.current = null;
    };
  }, [fitRef, fitCamera]);

  const getViewCubeCamera = useCallback(
    () => sceneRef.current?.camera ?? null,
    [],
  );

  const prepareViewCubeCamera = useCallback((): ViewCubeCameraControl | null => {
    const scene = sceneRef.current;
    if (!scene) return null;
    orbitTargetRef.current.copy(
      trackedOrbitTarget(
        orbitTargetRef.current,
        scene.camera.position,
        scene.camera.getWorldDirection(new THREE.Vector3()),
      ),
    );
    return {
      camera: scene.camera,
      target: orbitTargetRef.current.clone(),
      canvasHeight: Math.max(canvasRef.current?.clientHeight ?? 1, 1),
    };
  }, []);

  const renderViewCubeCamera = useCallback(() => {
    sceneRef.current?.render();
  }, []);

  // 区画サイズの変化に追従
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const observer = new ResizeObserver(() => {
      sceneRef.current?.resize(
        canvas.clientWidth,
        canvas.clientHeight,
        hintBottomPx(),
      );
    });
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [hintBottomPx]);

  /** canvas上の位置を畳み平面(z=0)の点へ直し、紙の点・輪郭へ吸着させる */
  const planePoint = useCallback(
    (rect: DOMRect, x: number, y: number): Vec2 | null => {
      const scene = sceneRef.current;
      const s = useAppStore.getState();
      if (!scene || !s.doc) return null;
      const p = screenToPlane(scene.camera, rect.width, rect.height, x, y);
      if (!p) return null;
      const radius = planeRadius(
        scene.camera,
        rect.width,
        rect.height,
        x,
        y,
        FOLD_SNAP_PX,
        FOLD_SNAP_FALLBACK,
      );
      return snapFoldPoint(foldLayers(s.frame3d, s.doc, s.faces), p, radius);
    },
    [],
  );

  /** canvas上の位置を畳み平面の点へ直すだけ(吸着しない)。つかむ操作に使う */
  const rawPoint = useCallback(
    (rect: DOMRect, x: number, y: number): Vec2 | null => {
      const scene = sceneRef.current;
      if (!scene) return null;
      return screenToPlane(scene.camera, rect.width, rect.height, x, y);
    },
    [],
  );

  /**
   * 3Dのクリック画素から、展開図の頂点ID・辺ID・面内座標を1本の逆写像で受け取る。
   * 点を指す道具はすべてこの入口を通す(道具ごとに別の当て方を足さない)。
   */
  const cpPickAt = useCallback(
    (rect: DOMRect, x: number, y: number, thresholdPx?: number): CpPick3D | null => {
      const scene = sceneRef.current;
      const index = cpIndex();
      if (!scene?.content || !index) return null;
      const surface = displayedPickSurface(scene);
      if (!surface) return null;
      return pickCpFromPixel({
        index,
        slots: scene.content.topology.slots,
        positions: scene.content.positions,
        surface,
        camera: scene.camera,
        widthPx: rect.width,
        heightPx: rect.height,
        x,
        y,
        ...(thresholdPx === undefined ? {} : { thresholdPx }),
      });
    },
    [cpIndex],
  );

  /**
   * 3Dで拾った点へ、展開図と同じ頂点・交点・方眼の吸着を適用する。
   * クリックから展開図へ戻す入口は `pickCpFromPixel` のまま増やさず、返ったcpを
   * 既存の `snap` へ通す。12pxを現在面の正規化距離へ直しているため、拡大率や
   * 面の傾きが変わっても吸着の画面上の広さが大きく変わらない。
   */
  const measurePointFromPick = useCallback(
    (pick: CpPick3D, rect: DOMRect, x: number, y: number) => {
      const s = useAppStore.getState();
      if (!s.doc) return null;
      const placement = facePlacementOf(pick.faceId);
      const center = placement
        ? cpPointOnFacePlane(
            placement,
            sceneRef.current!.camera,
            rect.width,
            rect.height,
            x,
            y,
          )
        : null;
      const offsets = placement
        ? [
            cpPointOnFacePlane(
              placement,
              sceneRef.current!.camera,
              rect.width,
              rect.height,
              x + SNAP_RADIUS_PX,
              y,
            ),
            cpPointOnFacePlane(
              placement,
              sceneRef.current!.camera,
              rect.width,
              rect.height,
              x,
              y + SNAP_RADIUS_PX,
            ),
          ]
        : [];
      const radii = offsets.flatMap((point) =>
        point && center
          ? [Math.hypot(point[0] - center[0], point[1] - center[1])]
          : [],
      );
      const radius = radii.length > 0 ? Math.max(...radii) : 0;
      const candidate = radius > 0 ? snap(s.doc, pick.cp, radius) : null;
      const polygon = cpIndex()?.polygons.get(pick.faceId);
      const accepted =
        candidate &&
        candidate.kind !== "edge" &&
        polygon &&
        pointInPolygon(polygon, candidate.pos)
          ? candidate
          : null;
      const cp: Vec2 = accepted ? [accepted.pos[0], accepted.pos[1]] : pick.cp;
      const vertexId =
        accepted?.kind === "vertex"
          ? (s.doc.cp.vertices.find(
              (vertex) =>
                Math.hypot(vertex.pos[0] - cp[0], vertex.pos[1] - cp[1]) <=
                1e-12,
            )?.id ?? null)
          : pick.vertexId;
      return { cp, faceId: pick.faceId, vertexId };
    },
    [cpIndex, facePlacementOf],
  );

  /**
   * 合わせて折る途中に、その画素で何を選んだことになるかを決める。
   * カーソルの形・視点回転を止めるかの判定・実際の選択が、
   * すべてこの1本の結果を使う(場所によって当たり方が変わることを無くす)。
   *
   * 次に選ぶのが点なら、まず展開図の頂点として拾う(展開図区画で選んだときと
   * 同じ頂点IDが付く)。頂点でない場所は畳み平面の候補から拾う。
   * 次に選ぶのが線なら、3Dで見えている辺から拾う。
   */
  const resolveAlignPick = useCallback(
    (rect: DOMRect, x: number, y: number): AlignPick | null => {
      const s = useAppStore.getState();
      const scene = sceneRef.current;
      if (!scene?.content || !s.doc || !s.alignDraft || s.activeTool !== "fold") {
        return null;
      }
      const steps = ALIGN_STEPS[s.alignDraft.mode];
      const at = s.alignDraft.picks.length % steps.length;
      if (steps[at] === "point") {
        const pick = cpPickAt(rect, x, y, ALIGN_PICK_PX);
        const vertexId = pick?.vertexId ?? null;
        const folded =
          vertexId === null
            ? null
            : foldedAlignPoint(s.doc, s.faces, s.frame3d, vertexId);
        if (vertexId !== null && folded) {
          return {
            target: { kind: "point", p: folded },
            cursor: folded,
            cpPick: { kind: "vertex", id: vertexId },
          };
        }
        const p = rawPoint(rect, x, y);
        if (!p) return null;
        const hit = nearestAlignPoint(
          foldLayers(s.frame3d, s.doc, s.faces),
          p,
          planeRadius(
            scene.camera,
            rect.width,
            rect.height,
            x,
            y,
            ALIGN_PICK_PX,
            FOLD_SNAP_FALLBACK,
          ),
        );
        return hit ? { target: { kind: "point", p: hit }, cursor: p, cpPick: null } : null;
      }
      const hit = pickHingeSegment(
        selectableEdgeSegmentsRef.current,
        scene.camera,
        rect.width,
        rect.height,
        x,
        y,
        ALIGN_PICK_PX,
        displayedPickSurface(scene) ?? undefined,
      );
      if (!hit) return null;
      return {
        target: { kind: "line", a: [hit.a.x, hit.a.y], b: [hit.b.x, hit.b.y] },
        cursor: rawPoint(rect, x, y),
        cpPick: { kind: "edge", id: hit.edgeId },
      };
    },
    [cpPickAt, rawPoint],
  );

  /** 引きかけ・選びかけの3D入力を捨てる(道具を替えたときなど) */
  const clearCpDrafts = useCallback(() => {
    pendingCpPointRef.current = null;
    curvePointsRef.current = [];
    constructRef.current = { points: [], seg: null };
    foldClickRef.current = null;
    vertexDragRef.current = null;
    measurePressRef.current = null;
  }, []);

  // 道具を替えたとき・展開図が入れ替わったときは、選びかけの点や作図を捨てる
  // (前の道具の途中が残ると、次の1クリックで思わぬ線が引かれるため)
  useEffect(() => {
    clearCpDrafts();
    drawHighlight();
  }, [activeTool, measureDraft.mode, docEpoch, clearCpDrafts, drawHighlight]);

  // Escで3Dの選びかけを取り消す(展開図区画のEscと同じ扱い)。
  // 入力欄を打っている間は邪魔しない
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (e.target instanceof HTMLElement && e.target.tagName === "INPUT") return;
      clearCpDrafts();
      const s = useAppStore.getState();
      if (s.activeTool === "measure") s.clearMeasurement();
      drawHighlight();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [clearCpDrafts, drawHighlight]);

  /**
   * 山・谷・補助の3Dクリックを1回分受け取る。
   * 直線は2クリック、曲線モードは描き方ごとに決まった数の点がそろったところで引く
   * (展開図区画の `onCurveClick` と同じ数え方)。
   */
  const addCpLinePoint = useCallback((cp: Vec2, kind: EdgeKind) => {
    const s = useAppStore.getState();
    if (!s.doc) return;
    if (s.curve.enabled) {
      const points = curvePointsRef.current;
      // 始点と同じところをもう一度押しても線にならないので受け付けない
      if (points.length === 1 && distance2(points[0], cp) <= 1e-9) return;
      points.push(cp);
      if (points.length < CURVE_STEPS[s.curve.shape]) return;
      const line = curvePolyline(s.curve.shape, points, {
        segments: s.curve.segments,
      });
      curvePointsRef.current = [];
      if (line && line.length >= 2) void s.drawCurve(line, kind);
      return;
    }
    const start = pendingCpPointRef.current;
    if (!start) {
      pendingCpPointRef.current = cp;
      return;
    }
    pendingCpPointRef.current = null;
    if (distance2(start, cp) > 1e-9) void s.drawSegment(start, cp, kind);
  }, []);

  /** 作図の3Dクリックを1回分受け取る。必要な点・線がそろったら補助線を引く */
  const addConstructPick = useCallback((pick: CpPick3D) => {
    const s = useAppStore.getState();
    const doc = s.doc;
    if (!doc) return;
    const draft = constructRef.current;
    const steps = CONSTRUCT_STEPS[s.construct.kind];
    const done = draft.points.length + (draft.seg ? 1 : 0);
    if (steps[Math.min(done, steps.length - 1)] === "line") {
      const edge =
        pick.edgeId === null
          ? undefined
          : doc.cp.edges.find((one) => one.id === pick.edgeId);
      const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
      const a = edge && byId.get(edge.v0);
      const b = edge && byId.get(edge.v1);
      if (!a || !b) return; // 線に当たらなければ何もしない(案内は出したまま)
      draft.seg = [a, b];
    } else {
      draft.points.push(pick.cp);
    }
    if (draft.points.length + (draft.seg ? 1 : 0) < steps.length) return;
    const lines = constructLines(s.construct.kind, draft.points, draft.seg, {
      divisions: s.construct.divisions,
      stepDeg: s.construct.stepDeg,
      paper: paperExtent(doc),
    });
    constructRef.current = { points: [], seg: null };
    // 角度線のように何本もまとめて引く作図でも、元に戻す1回で作る前へ戻れるよう
    // 1回の要求として渡す(展開図区画の作図と同じ扱い)
    if (lines.length > 0) {
      void s.applyEdit(
        lines.map(([a, b]) => ({ type: "AddSegment", a, b, kind: "Aux" }) as const),
      );
    }
  }, []);

  /**
   * 指している対象に合わせてカーソルだけを直接変える。
   * hoverは表示専用なのでZustandへ頻繁に書かず、CpEditorと同じくDOMへ反映する。
   */
  const updateHoverCursor = useCallback(
    (canvas: HTMLCanvasElement, x: number, y: number, ctrlKey = false) => {
      const s = useAppStore.getState();
      const scene = sceneRef.current;
      if (!scene?.content) {
        canvas.style.cursor = "default";
        return;
      }
      const rect = canvas.getBoundingClientRect();
      const surface = displayedPickSurface(scene);
      if (s.activeTool === "pull") {
        if (pullBlockedOf(s) !== null) {
          canvas.style.cursor = "not-allowed";
          return;
        }
        const hit = surface && pickPaper(
          surface.mesh,
          surface.triangleFaceIds,
          scene.camera,
          rect.width,
          rect.height,
          x,
          y,
          surface.faceSurfaceRanks,
        );
        const plan =
          hit &&
          s.doc &&
          planPull(
            s.doc,
            s.faces,
            s.frame3d,
            hit.face,
            [hit.point.x, hit.point.y, hit.point.z],
            [0, 0, 0],
            s.pullMirror,
          );
        canvas.style.cursor = hit && plan ? "grab" : "default";
        return;
      }
      if (s.activeTool === "fold") {
        if (!canFoldNow(s) || s.foldThroughBusy || s.pendingFoldThrough) {
          canvas.style.cursor = "not-allowed";
          return;
        }
        if (s.alignDraft && s.doc) {
          // 選べるもの(点でも線・辺でも)の上では、指で押す形にして視点回転を止める。
          // そうしないと押した瞬間から視点が回り、少しでも手がぶれると選べない。
          // 選べるものが無い場所では止めないので、視点は今までどおり回せる。
          const hit = resolveAlignPick(rect, x, y) !== null;
          setHoverLock(hit);
          canvas.style.cursor = hit ? "pointer" : "default";
          return;
        }
        if (ctrlKey) {
          canvas.style.cursor = "crosshair";
          return;
        }
        const face = surface && pickFace(
          surface.mesh,
          surface.triangleFaceIds,
          scene.camera,
          rect.width,
          rect.height,
          x,
          y,
          surface.faceSurfaceRanks,
        );
        canvas.style.cursor = face == null ? "default" : "grab";
        return;
      }
      if (s.activeTool === "technique") {
        if (!canFoldNow(s)) {
          canvas.style.cursor = "not-allowed";
          return;
        }
        if (s.techniqueDraft?.kind === "Simple" && s.techniqueDraft.motionMode === "reflect") {
          const axis = pickHingeSegment(
            scene.content.hingeSegments,
            scene.camera,
            rect.width,
            rect.height,
            x,
            y,
            undefined,
            surface ?? undefined,
          );
          canvas.style.cursor = axis ? "pointer" : "crosshair";
          return;
        }
        canvas.style.cursor = "crosshair";
        return;
      }
      // 展開図を直接編集する道具(山・谷・補助・作図)は、3Dでも同じ十字カーソルにする
      const drawsOnCp = TOOL_KIND[s.activeTool] !== undefined || s.activeTool === "construct";
      // 点を使わない道具(削除など)では逆写像を通さない(そのぶん当たり判定を省く)
      const cpPick =
        drawsOnCp || s.activeTool === "select" || s.activeTool === "measure"
          ? cpPickAt(rect, x, y)
          : null;
      if (s.activeTool === "measure") {
        const target =
          s.measureDraft.mode === "distance"
            ? cpPick !== null && (cpPick.onPaper || cpPick.vertexId !== null)
            : cpPick?.edgeId !== null && cpPick?.edgeId !== undefined;
        setHoverLock(target);
        canvas.style.cursor = target ? "pointer" : "default";
        return;
      }
      // 「選択」で動かせる点の上に来たら、左ドラッグの視点回転を止めておく。
      // 押した瞬間から点をつかめるようにするための先回り(押してから止めても、
      // 視点回転を始める処理はcanvasの入力を先に受け取っているので間に合わない)。
      const draggableVertex =
        s.activeTool === "select" &&
        s.doc !== null &&
        cpPick?.vertexId != null &&
        !isBorderVertex(s.doc, cpPick.vertexId);
      setHoverLock(draggableVertex);
      if (draggableVertex) {
        canvas.style.cursor = "move";
        return;
      }
      if (drawsOnCp) {
        canvas.style.cursor = cpPick ? "crosshair" : "default";
        return;
      }
      if (cpPick?.vertexId != null) {
        canvas.style.cursor = "pointer";
        return;
      }
      const edgeId = pickHingeSegment(
        selectableEdgeSegmentsRef.current,
        scene.camera,
        rect.width,
        rect.height,
        x,
        y,
        undefined,
        surface ?? undefined,
      )?.edgeId ?? null;
      if (edgeId !== null) {
        canvas.style.cursor = "pointer";
        return;
      }
      const paper = surface && pickPaper(
        surface.mesh,
        surface.triangleFaceIds,
        scene.camera,
        rect.width,
        rect.height,
        x,
        y,
        surface.faceSurfaceRanks,
      );
      canvas.style.cursor = paper ? "pointer" : "default";
    },
    [cpPickAt, resolveAlignPick, setHoverLock],
  );

  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      const rect = e.currentTarget.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const s = useAppStore.getState();
      if (s.foldAllPreview !== null) {
        // 一斉形は作品編集の入力にしない。OrbitControlsのnative listenerは残し、
        // 視点操作だけを許す。途中だった紙操作のrefもここで確実に破棄する。
        pullRef.current = null;
        grabRef.current = null;
        drawingRef.current = null;
        vertexDragRef.current = null;
        alignPressRef.current = null;
        measurePressRef.current = null;
        downPosRef.current = { x, y };
        if (e.button === 0 || e.button === 2) e.currentTarget.style.cursor = "grabbing";
        return;
      }
      const scene0 = sceneRef.current;
      // 紙をつかんで引く(UI-007): つかんだ面から根までの経路のうち、
      // その点をいちばんよく動かす折り線を選び、その角度だけを動かして
      // 残りはソルバーにつじつまを合わせてもらう
      if (
        e.button === 0 &&
        s.activeTool === "pull" &&
        pullBlockedOf(s) === null &&
        s.doc &&
        scene0?.content
      ) {
        const surface = displayedPickSurface(scene0);
        const hit = surface && pickPaper(
          surface.mesh,
          surface.triangleFaceIds,
          scene0.camera,
          rect.width,
          rect.height,
          x,
          y,
          surface.faceSurfaceRanks,
        );
        const plan =
          hit &&
          planPull(
            s.doc,
            s.faces,
            s.frame3d,
            hit.face,
            [hit.point.x, hit.point.y, hit.point.z],
            [0, 0, 0],
            s.pullMirror,
          );
        if (hit && plan) {
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.style.cursor = "grabbing";
          s.setOperationStage(1);
          pullRef.current = {
            plan,
            origin: hit.point,
            ndc: hit.point.clone().project(scene0.camera),
            x,
            y,
          };
          s.beginPull(
            plan.hinge,
            hingeAnglesFromFrame(s.doc, s.faces, s.frame3d),
            plan.mirrorHinge,
          );
        }
        return;
      }
      const drawTool = s.activeTool === "fold" || s.activeTool === "technique";
      // 「折る」の主操作は紙をつかんで動かすこと(UI-007)。
      // 位置をきっちり指定したいときだけ、Ctrl+ドラッグで折り線を引く(補助操作)
      const scene = sceneRef.current;
      if (
        e.button === 0 &&
        s.activeTool === "fold" &&
        !e.ctrlKey &&
        !s.alignDraft &&
        !s.foldThroughBusy &&
        !s.pendingFoldThrough &&
        canFoldNow(s) &&
        scene?.content
      ) {
        const surface = displayedPickSurface(scene);
        const hit = surface && pickPaper(
          surface.mesh,
          surface.triangleFaceIds,
          scene.camera,
          rect.width,
          rect.height,
          x,
          y,
          surface.faceSurfaceRanks,
        );
        const spatial = isSpatialFoldFrame(s.frame3d);
        const p = hit && !spatial ? rawPoint(rect, x, y) : null;
        if (hit && (spatial || p)) {
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.style.cursor = "grabbing";
          s.setOperationStage(1);
          if (spatial) {
            const a: SpatialFoldDrag["from"] = [hit.point.x, hit.point.y, hit.point.z];
            grabRef.current = {
              spatial: true,
              a,
              b: [...a],
              face: hit.face,
              mode: grabMode(e),
              origin: hit.point.clone(),
              ndc: hit.point.clone().project(scene.camera),
              x,
              y,
              direction: "Up",
            };
          } else {
            grabRef.current = {
              spatial: false,
              a: p!,
              b: p!,
              face: hit.face,
              mode: grabMode(e),
            };
          }
          drawHighlight();
        }
        return;
      }
      if (
        e.button === 0 &&
        drawTool &&
        !s.alignDraft &&
        !s.foldThroughBusy &&
        !s.pendingFoldThrough &&
        canFoldNow(s)
      ) {
        const p = planePoint(rect, x, y);
        if (p) {
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.style.cursor = "crosshair";
          s.setOperationStage(1);
          drawingRef.current = { a: p, b: p };
          drawHighlight();
        }
        return;
      }
      if (e.button === 0 && s.activeTool === "measure") {
        const pick = cpPickAt(rect, x, y);
        const valid =
          s.measureDraft.mode === "distance"
            ? pick !== null && (pick.onPaper || pick.vertexId !== null)
            : pick?.edgeId !== null && pick?.edgeId !== undefined;
        measurePressRef.current = valid ? pick : null;
        if (valid) {
          // 押した瞬間に選べる対象だと分かったら、離すまで視点を回さない。
          setHoverLock(true);
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.style.cursor = "pointer";
          downPosRef.current = { x, y };
          return;
        }
      }
      // 「選択」で点の上を押したら、その点を動かす操作として始める(展開図区画と同じ)。
      // 動かさずに離せばただの選択になる
      if (e.button === 0 && s.activeTool === "select" && s.doc && !e.ctrlKey && !e.metaKey) {
        const pick = cpPickAt(rect, x, y);
        const vertexId = pick?.vertexId ?? null;
        if (pick && vertexId !== null && !isBorderVertex(s.doc, vertexId)) {
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.style.cursor = "grabbing";
          vertexDragRef.current = {
            id: vertexId,
            faceId: pick.faceId,
            from: pick.cp,
            to: pick.cp,
          };
          s.setSelection({ edgeIds: [], vertexIds: [vertexId] });
          drawHighlight();
          return;
        }
      }
      // 合わせて折るは、押した瞬間に「何を選んだか」を決めてしまう。
      // 離すまでに手がぶれても、押した場所にあったものが選ばれる
      alignPressRef.current =
        e.button === 0 && s.activeTool === "fold" && s.alignDraft
          ? resolveAlignPick(rect, x, y)
          : null;
      if (alignPressRef.current) {
        // マウスを動かさずに押した場合(ペン・指など)でも、ここから先は視点を回さない
        setHoverLock(true);
        e.currentTarget.setPointerCapture(e.pointerId);
        e.currentTarget.style.cursor = "pointer";
        downPosRef.current = { x, y };
        return;
      }
      downPosRef.current = { x, y };
      if (e.button === 0 || e.button === 2) e.currentTarget.style.cursor = "grabbing";
    },
    [planePoint, rawPoint, drawHighlight, cpPickAt, resolveAlignPick, setHoverLock],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      if (useAppStore.getState().foldAllPreview !== null) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const pull = pullRef.current;
      const scene = sceneRef.current;
      if (pull && scene) {
        // 画面上の動きを、つかんだ点を通る画面と平行な面の上のベクトルへ直す
        const dx = e.clientX - rect.left - pull.x;
        const dy = e.clientY - rect.top - pull.y;
        const moved = new THREE.Vector3(
          pull.ndc.x + (dx * 2) / rect.width,
          pull.ndc.y - (dy * 2) / rect.height,
          pull.ndc.z,
        ).unproject(scene.camera);
        const drag = moved.sub(pull.origin);
        const delta = pullDeltaDeg(pull.plan.velocity, [drag.x, drag.y, drag.z]);
        if (Math.hypot(dx, dy) > CLICK_MOVE_PX) {
          useAppStore.getState().setOperationStage(2);
        }
        e.currentTarget.style.cursor = "grabbing";
        useAppStore.getState().pullTo(pull.plan.baseDeg + delta);
        return;
      }
      const grab = grabRef.current;
      if (grab) {
        if (grab.spatial) {
          if (!scene) return;
          const dx = e.clientX - rect.left - grab.x;
          const dy = e.clientY - rect.top - grab.y;
          const moved = new THREE.Vector3(
            grab.ndc.x + (dx * 2) / rect.width,
            grab.ndc.y - (dy * 2) / rect.height,
            grab.ndc.z,
          ).unproject(scene.camera);
          grab.b = [moved.x, moved.y, moved.z];
          grab.direction = spatialDragDirection(
            useAppStore.getState().frame3d,
            grab.face,
            grab.a,
            grab.b,
            grab.direction,
          );
        } else {
          const p = rawPoint(rect, e.clientX - rect.left, e.clientY - rect.top);
          if (!p) return;
          grab.b = p;
        }
        useAppStore.getState().setOperationStage(2);
        e.currentTarget.style.cursor = "grabbing";
        grab.mode = grabMode(e); // 途中で修飾キーを押しても下見に反映する
        drawHighlight();
        return;
      }
      // 点を動かしている間は、その点が載っている面の平面をたどる。
      // 面の外へカーソルが出ても同じ面の座標系で追えるので、手が形に追われない
      const vertexDrag = vertexDragRef.current;
      if (vertexDrag && scene) {
        const placement = facePlacementOf(vertexDrag.faceId);
        const at =
          placement &&
          cpPointOnFacePlane(
            placement,
            scene.camera,
            rect.width,
            rect.height,
            e.clientX - rect.left,
            e.clientY - rect.top,
          );
        if (at) {
          vertexDrag.to = at;
          useAppStore.getState().setOperationStage(2);
          e.currentTarget.style.cursor = "grabbing";
          drawHighlight();
        }
        return;
      }
      const drawing = drawingRef.current;
      if (!drawing) {
        updateHoverCursor(
          e.currentTarget,
          e.clientX - rect.left,
          e.clientY - rect.top,
          e.ctrlKey,
        );
        return;
      }
      const p = planePoint(rect, e.clientX - rect.left, e.clientY - rect.top);
      if (!p) return;
      drawing.b = p;
      useAppStore.getState().setOperationStage(2);
      e.currentTarget.style.cursor = "crosshair";
      drawHighlight();
    },
    [planePoint, rawPoint, drawHighlight, updateHoverCursor, facePlacementOf],
  );

  /** クリック(視点操作でない)なら最寄りのヒンジを選ぶ。折り線を引いていたら確定する */
  const handlePointerUp = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      if (useAppStore.getState().foldAllPreview !== null) {
        pullRef.current = null;
        grabRef.current = null;
        drawingRef.current = null;
        vertexDragRef.current = null;
        alignPressRef.current = null;
        measurePressRef.current = null;
        downPosRef.current = null;
        e.currentTarget.style.cursor = "grab";
        return;
      }
      if (pullRef.current) {
        // 引いた形はそのまま残る(角度指定として保持される)。色付けだけ消す
        pullRef.current = null;
        useAppStore.getState().endPull();
        useAppStore.getState().setOperationStage(2);
        updateHoverCursor(
          e.currentTarget,
          e.clientX - e.currentTarget.getBoundingClientRect().left,
          e.clientY - e.currentTarget.getBoundingClientRect().top,
          e.ctrlKey,
        );
        return;
      }
      const grab = grabRef.current;
      if (grab) {
        grabRef.current = null;
        useAppStore.getState().setOperationStage(2);
        drawHighlight(); // 下見を消してから、実際に折る
        void useAppStore
          .getState()
          .foldByDrag(
            grab.a,
            grab.b,
            grabMode(e),
            grab.face,
            grab.spatial ? grab.direction : "Up",
          );
        return;
      }
      const drawing = drawingRef.current;
      if (drawing) {
        drawingRef.current = null;
        const [a, b] = [drawing.a, drawing.b];
        const s = useAppStore.getState();
        const drawn = Math.hypot(b[0] - a[0], b[1] - a[1]) >= MIN_FOLD_LENGTH;
        if (s.activeTool === "technique" && s.techniqueDraft && s.doc) {
          const techniqueDoc = s.doc;
          const selectTechniqueFlap = () =>
            s.setTechniqueFlap(
              facesAtPoint(
                foldLayers(s.frame3d, techniqueDoc, s.faces),
                a,
                FLAP_PICK_EPS,
              ),
            );
          if (s.techniqueDraft.kind === "Twist" && !drawn) {
            // ねじり折り: 通常クリック=中央多角形の角、Ctrl=中心、Shift=対象層。
            // 通常クリックが頂点専用でも、既存の層ピッカーを全て使えるようにする。
            if (e.ctrlKey) s.setTechniqueCenter(a);
            else if (e.shiftKey) selectTechniqueFlap();
            else s.addTechniqueVertex(a);
          } else if (
            !drawn &&
            e.ctrlKey &&
            s.techniqueDraft.kind !== "Simple"
          ) {
            // 名前付き技法の任意基準点。Swivelの寄せ先など、自動の左右点では
            // 表せない位置を直接指す。SimpleはCtrlも従来どおり層/既存軸選択。
            s.setTechniqueReferencePoint(a);
          } else if (drawn) {
            // 既存折り目の開閉は、目分量のドラッグ軸を受け取らない。クリックで
            // sceneのヒンジ線分を選び、現在形と完全に同じ座標を使う。
            if (
              !(
                s.techniqueDraft.kind === "Simple" &&
                s.techniqueDraft.motionMode === "reflect"
              )
            ) {
              s.setTechniqueLine([a, b]);
            }
          } else {
            // 層操作の開閉では、既存ヒンジをクリックすると表示中の正確な線分を
            // Reflect軸へ使う。ヒンジ以外の紙面クリックは従来どおり層選択。
            const rect = e.currentTarget.getBoundingClientRect();
            const scene = sceneRef.current;
            const axis =
              s.techniqueDraft.kind === "Simple" &&
              s.techniqueDraft.motionMode === "reflect" &&
              scene?.content
                ? pickHingeSegment(
                    scene.content.hingeSegments,
                    scene.camera,
                    rect.width,
                    rect.height,
                    e.clientX - rect.left,
                    e.clientY - rect.top,
                    undefined,
                    displayedPickSurface(scene) ?? undefined,
                  )
                : null;
            if (axis) {
              s.setLayerMotionAxis(axis.edgeId, [
                [axis.a.x, axis.a.y],
                [axis.b.x, axis.b.y],
              ]);
            } else {
              selectTechniqueFlap();
            }
          }
        } else if (drawn) {
          s.beginFoldDraft([a, b], "3d");
          s.setOperationStage(1);
        } else if (s.activeTool === "fold") {
          // ドラッグしにくい場所でも折り線を決められるよう、Ctrl+クリック2回でも
          // 同じ折り線を引けるようにする。1回目が始点、2回目で確定する
          const rect = e.currentTarget.getBoundingClientRect();
          const pick = cpPickAt(
            rect,
            e.clientX - rect.left,
            e.clientY - rect.top,
            ALIGN_PICK_PX,
          );
          // 折り線は畳み平面の座標で決まるので、当たった紙の位置のxyを使う
          const at: Vec2 | null = pick ? [pick.world[0], pick.world[1]] : null;
          const start = foldClickRef.current;
          if (at && !start) {
            foldClickRef.current = at;
          } else if (at && start) {
            foldClickRef.current = null;
            if (distance2(start, at) >= MIN_FOLD_LENGTH) {
              s.beginFoldDraft([start, at], "3d");
              s.setOperationStage(1);
            }
          }
        }
        drawHighlight();
        return;
      }
      // 点をつかんで動かし終えたところで、展開図の点の位置を確定する
      // (1ドラッグ=1回の編集。途中の位置は履歴に残さない)
      const vertexDrag = vertexDragRef.current;
      if (vertexDrag) {
        vertexDragRef.current = null;
        const s = useAppStore.getState();
        if (distance2(vertexDrag.from, vertexDrag.to) > 1e-9) {
          void s.applyEdit({
            type: "MoveVertex",
            id: vertexDrag.id,
            to: vertexDrag.to,
          });
        }
        drawHighlight();
        const rect = e.currentTarget.getBoundingClientRect();
        updateHoverCursor(
          e.currentTarget,
          e.clientX - rect.left,
          e.clientY - rect.top,
          e.ctrlKey,
        );
        return;
      }
      const down = downPosRef.current;
      downPosRef.current = null;
      // 合わせて折る: 押した瞬間に決まった選択をそのまま確定する。
      // 押した場所で決めているので、離すまでにどれだけ手がぶれても選択は成立する
      const pressed = alignPressRef.current;
      alignPressRef.current = null;
      const measured = measurePressRef.current;
      measurePressRef.current = null;
      const scene = sceneRef.current;
      if (!down || !scene?.content || e.button !== 0) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const st = useAppStore.getState();
      if (pressed) {
        if (st.activeTool === "fold" && st.alignDraft && st.doc) {
          st.pickAlignTarget(pressed.target, pressed.cursor, pressed.cpPick);
        }
        updateHoverCursor(e.currentTarget, x, y, e.ctrlKey);
        return;
      }
      if (st.activeTool === "measure") {
        if (measured) {
          if (st.measureDraft.mode === "distance") {
            const point = measurePointFromPick(
              measured,
              rect,
              down.x,
              down.y,
            );
            if (point) st.pickMeasurePoint(point);
          } else if (measured.edgeId !== null) {
            st.pickMeasureEdge(measured.edgeId);
          }
        }
        drawHighlight();
        updateHoverCursor(e.currentTarget, x, y, e.ctrlKey);
        return;
      }
      if (Math.hypot(x - down.x, y - down.y) > CLICK_MOVE_PX) return; // 視点の回転・移動
      if (st.activeTool === "fold" && st.alignDraft && st.doc) {
        // 選べるものが無い場所を押して離した(何も選ばない)
        return;
      }
      // 3Dのクリック画素を、展開図の頂点・辺・面内座標へ1本の逆写像で直す。
      // 点を使う道具はすべてここから受け取る(道具ごとに別の当て方を足さない)
      const cpPick = cpPickAt(rect, x, y);
      const lineKind = TOOL_KIND[st.activeTool];
      if (lineKind) {
        if (cpPick) addCpLinePoint(cpPick.cp, lineKind);
        drawHighlight();
        updateHoverCursor(e.currentTarget, x, y, e.ctrlKey);
        return;
      }
      if (st.activeTool === "construct") {
        if (cpPick) addConstructPick(cpPick);
        drawHighlight();
        updateHoverCursor(e.currentTarget, x, y, e.ctrlKey);
        return;
      }
      const toggle = e.ctrlKey || e.metaKey;
      // 展開図区画と同じく、点は線より先に拾う
      const vertexId = st.activeTool === "select" ? (cpPick?.vertexId ?? null) : null;
      if (vertexId !== null) {
        st.setSelection(
          toggle
            ? {
                edgeIds: st.selection.edgeIds,
                vertexIds: st.selection.vertexIds.includes(vertexId)
                  ? st.selection.vertexIds.filter((id) => id !== vertexId)
                  : [...st.selection.vertexIds, vertexId],
              }
            : { edgeIds: [], vertexIds: [vertexId] },
        );
        st.hidePaperActionTip();
        updateHoverCursor(e.currentTarget, x, y, e.ctrlKey);
        return;
      }
      const edgeId = pickHingeSegment(
        selectableEdgeSegmentsRef.current,
        scene.camera,
        rect.width,
        rect.height,
        x,
        y,
        undefined,
        displayedPickSurface(scene) ?? undefined,
      )?.edgeId ?? null;
      if (toggle && edgeId === null) {
        // Ctrl/Command+空白は現在の複数選択を保つ。
        updateHoverCursor(e.currentTarget, x, y, e.ctrlKey);
        return;
      }
      const edgeIds =
        toggle && edgeId !== null
          ? st.selection.edgeIds.includes(edgeId)
            ? st.selection.edgeIds.filter((id) => id !== edgeId)
            : [...st.selection.edgeIds, edgeId]
          : edgeId !== null
            ? [edgeId]
            : [];
      st.setSelection({ edgeIds, vertexIds: [] });
      if (st.activeTool === "select" && edgeId === null) {
        const surface = displayedPickSurface(scene);
        const paper = surface && pickPaper(
          surface.mesh,
          surface.triangleFaceIds,
          scene.camera,
          rect.width,
          rect.height,
          x,
          y,
          surface.faceSurfaceRanks,
        );
        if (paper) st.showPaperActionTip();
        else st.hidePaperActionTip();
      } else if (st.activeTool === "select") {
        st.hidePaperActionTip();
      }
      updateHoverCursor(e.currentTarget, x, y, e.ctrlKey);
    },
    [
      drawHighlight,
      updateHoverCursor,
      cpPickAt,
      measurePointFromPick,
      addCpLinePoint,
      addConstructPick,
    ],
  );

  return (
    <>
      <canvas
        ref={canvasRef}
        className="viewer3d-canvas"
        aria-label={
          foldAllActive
            ? "全部の折り目を同じ割合で動かした形。ドラッグで視点を回せます"
            : undefined
        }
        style={{
          cursor:
            foldAllActive
              ? "grab"
              : activeTool === "measure"
              ? "crosshair"
              : pullMode && pullBlocked === null
              ? "grab"
              : !foldMode || !foldReady
                ? "default"
                : activeTool === "fold" && !alignDraft
                  ? "grab"
                  : "crosshair",
        }}
        data-tooltip={
          foldAllActive
            ? "形を見る間は、ドラッグで視点を回し、ホイールで拡大縮小できます"
            : activeTool === "measure"
            ? measureDraft.mode === "angle"
              ? "紙の辺を2本クリックして角度を測ります(Escで選び直し)"
              : measureDraft.mode === "length"
                ? "紙の線を1本クリックして長さを測ります(Escで選び直し)"
                : "紙の上の点を2つクリックして距離を測ります(Escで選び直し)"
            : pullMode
            ? "紙をドラッグして全体を連動させます。右ドラッグで視点を回します"
            : activeTool === "technique"
            ? techniqueDraft?.kind === "Simple"
              ? "紙面をクリックして対象層を選び、開閉なら既存の折り目をクリックして軸を選びます"
              : techniqueDraft?.kind === "Twist"
              ? "中央の形の角を3つ以上、順に選びます"
              : "紙の層を選び、ドラッグで折り線を引きます"
            : activeTool === "fold" && alignDraft
              ? `点または${SELECTABLE_3D_EDGE_TARGETS}をクリックして選びます`
              : foldMode
                ? "紙をドラッグして折ります。Ctrl+ドラッグ、またはCtrl+クリック2回で折り線を指定します"
                : TOOL_KIND[activeTool] !== undefined
                  ? "紙の上の点をクリックして線を引きます(Escで中止)"
                  : activeTool === "construct"
                    ? "紙の上の点や線をクリックして作図します(Escで中止)"
                    : `ドラッグで回転、ホイールで拡大縮小。点・${SELECTABLE_3D_EDGE_TARGETS}をクリックして選びます。点はドラッグで動かせます`
        }
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={() => {
          downPosRef.current = null;
          alignPressRef.current = null;
          measurePressRef.current = null;
          drawingRef.current = null;
          grabRef.current = null;
          vertexDragRef.current = null;
          pullRef.current = null;
          useAppStore.getState().endPull();
          useAppStore.getState().setOperationStage(0);
          drawHighlight();
        }}
        onPointerLeave={(e) => {
          if (
            !pullRef.current &&
            !grabRef.current &&
            !drawingRef.current &&
            !vertexDragRef.current
          ) {
            e.currentTarget.style.cursor = "default";
          }
        }}
        onContextMenu={(e) => e.preventDefault()}
      />
      <ViewerOverlayStack>
        {statusOverlays}
        <ViewerOperationHint
          hint={hint}
          blocked={(foldMode && !foldReady) || (pullMode && pullBlocked !== null)}
          aligning={alignDraft !== null}
        />
        <PaperActionTip />
        <FoldDirectionTip />
      </ViewerOverlayStack>
      <ViewCube
        getCamera={getViewCubeCamera}
        prepareCameraControl={prepareViewCubeCamera}
        requestRender={renderViewCubeCamera}
      />
      {/* 立体だけを最初の視点へ戻す小さなボタン(ツールレールは増やさない)。
          上端は警告バッジが使うので、区画の右下の隅に置く */}
      <button
        type="button"
        className="viewer-reset"
        data-floating-ui="viewer-reset"
        data-tooltip="3Dを紙全体が見える視点へ戻します"
        onClick={fitCamera}
      >
        視点を戻す
      </button>
    </>
  );
}
