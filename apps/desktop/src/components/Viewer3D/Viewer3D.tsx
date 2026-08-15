// 3Dビュー区画: Three.jsのシーンをcanvasに描き、ヒンジのクリック選択と
// 「折る」ツールの折り線描画を受け付ける。
// Three.jsのオブジェクトはストアに入れずrefで保持する(要件§2: 状態はストア1本)。
//
// 更新の分担:
//   - 展開図(doc/faces/hinges)が変わったとき: 三角形分割と添字を作り直す
//   - 立体形状(frame3d)が変わったとき: 頂点座標の上書きだけ(作り直さない)

import { useCallback, useEffect, useRef } from "react";
import * as THREE from "three";
import {
  canFoldNow,
  isSpatialFoldFrame,
  pullBlockedOf,
  type SpatialFoldDrag,
  useAppStore,
} from "../../store/appStore";
import { SELECTABLE_3D_EDGE_TARGETS, viewerHint } from "../../lib/viewerHint";
import {
  hingeAnglesFromFrame,
  planPull,
  pullDeltaDeg,
  type PullPlan,
} from "../../lib/grabDrive";
import { paperExtent } from "../CpEditor/snap";
import { planeRadius, screenToPlane } from "../../lib/planeProject";
import type { Face, FoldDirection, Frame3D, Vec2 } from "../../lib/types";
import {
  facesAtPoint,
  foldLayers,
  foldPreviewSegments,
  keepSidePoint,
  snapFoldPoint,
} from "./foldDraw";
import {
  buildTopology,
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
import { ALIGN_STEPS } from "../../lib/alignFold";
import { nearestAlignPoint } from "../../lib/alignPick";
import { planGrabFold, type GrabMode } from "./grabFold";
import {
  pickFace,
  pickHingeSegment,
  pickPaper,
  type HingeSegment,
  type PaperPickSurface,
} from "./hingePicker";
import { deriveSelectedEdgeHighlights } from "./edgeHighlight";
import { ViewerOperationHint } from "./ViewerOperationHint";
import { PaperActionTip } from "./PaperActionTip";
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

interface Props {
  /** 「全体表示」用: 親が current を呼ぶと紙全体が見える位置にカメラを戻す */
  fitRef: React.RefObject<(() => void) | null>;
}

export function Viewer3D({ fitRef }: Props) {
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
  const activeAngleIntent = useAppStore((s) => s.activeAngleIntent);
  const docEpoch = useAppStore((s) => s.docEpoch);
  const activeTool = useAppStore((s) => s.activeTool);
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
    if (s.foldThroughBusy) return "折り方を確認しています。少し待ってください";
    if (s.pendingFoldThrough) {
      return "追加折り目の位置を確認し、下のパネルで折り方を選んでください";
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
    const suspectIds = new Set(s.suspectHinges);
    const suspectSegments: HighlightSegment[] = scene.content.hingeSegments
      .filter((segment) => suspectIds.has(segment.edgeId))
      .map((segment) => ({ ...segment, role: "suspect" as const }));
    const activeIds = new Set(s.activeAngleIntent?.hinges ?? []);
    if (s.pullHinge !== null) activeIds.add(s.pullHinge);
    if (s.pullMirrorHinge !== null) activeIds.add(s.pullMirrorHinge);
    const activeSegments: HighlightSegment[] = scene.content.hingeSegments
      .filter((segment) => activeIds.has(segment.edgeId) && !suspectIds.has(segment.edgeId))
      .map((segment) => ({ ...segment, role: "active" as const }));
    const hoveredSegments: HighlightSegment[] =
      s.hoveredHinge !== null && !s.selection.edgeIds.includes(s.hoveredHinge)
        ? scene.content.hingeSegments
            .filter(
              (segment) =>
                segment.edgeId === s.hoveredHinge && !suspectIds.has(segment.edgeId),
            )
            .map((segment) => ({ ...segment, role: "focus" as const }))
        : [];
    const setHighlight = (segments: HighlightSegment[]) => {
      const physicalSegments = [
        ...suspectSegments,
        ...segments.filter((segment) => !suspectIds.has(segment.edgeId)),
        ...hoveredSegments,
        ...activeSegments,
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
  }, []);

  // 選択・折り線プレビューの強調(上の効果で線分が更新された後に走る)
  useEffect(() => {
    drawHighlight();
  }, [
    selection,
    hoveredHinge,
    suspectHinges,
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
    pullHinge,
    pullMirrorHinge,
    softMesh,
    drawHighlight,
  ]);

  // 折る・引くツールの間は左ドラッグを紙の操作に使うので、視点の回転を止める。
  // 引くツールでは代わりに右ドラッグで回せるようにする(色々な向きから引くため)
  useEffect(() => {
    sceneRef.current?.setDrawMode(
      (foldMode && foldReady && !alignDraft) || (pullMode && pullBlocked === null),
      pullMode,
    );
  }, [foldMode, foldReady, pullMode, pullBlocked, alignDraft]);

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

  /** 紙全体が見える斜め上の位置へカメラを戻す */
  const fitCamera = useCallback(() => {
    const scene = sceneRef.current;
    const current = useAppStore.getState().doc;
    if (!scene || !current) return;
    const [w, h] = paperExtent(current);
    scene.resetCamera(w, h);
    orbitTargetRef.current.set(w / 2, h / 2, 0);
  }, []);

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
      sceneRef.current?.resize(canvas.clientWidth, canvas.clientHeight);
    });
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);

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
          const steps = ALIGN_STEPS[s.alignDraft.mode];
          const at = s.alignDraft.picks.length % steps.length;
          const p = screenToPlane(scene.camera, rect.width, rect.height, x, y);
          const hit =
            steps[at] === "point"
              ? p &&
                nearestAlignPoint(
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
                )
              : pickHingeSegment(
                  selectableEdgeSegmentsRef.current,
                  scene.camera,
                  rect.width,
                  rect.height,
                  x,
                  y,
                  ALIGN_PICK_PX,
                  surface ?? undefined,
                );
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
    [],
  );

  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      const rect = e.currentTarget.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const s = useAppStore.getState();
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
      downPosRef.current = { x, y };
      if (e.button === 0 || e.button === 2) e.currentTarget.style.cursor = "grabbing";
    },
    [planePoint, rawPoint, drawHighlight],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
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
    [planePoint, rawPoint, drawHighlight, updateHoverCursor],
  );

  /** クリック(視点操作でない)なら最寄りのヒンジを選ぶ。折り線を引いていたら確定する */
  const handlePointerUp = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
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
        }
        drawHighlight();
        return;
      }
      const down = downPosRef.current;
      downPosRef.current = null;
      const scene = sceneRef.current;
      if (!down || !scene?.content || e.button !== 0) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      if (Math.hypot(x - down.x, y - down.y) > CLICK_MOVE_PX) return; // 視点の回転・移動
      // 合わせて折る: 次に選ぶべき種類(点/線)に合わせて、近い方へ吸着して拾う
      const st = useAppStore.getState();
      if (st.activeTool === "fold" && st.alignDraft && st.doc) {
        const steps = ALIGN_STEPS[st.alignDraft.mode];
        const at = st.alignDraft.picks.length % steps.length;
        if (steps[at] === "point") {
          const p = rawPoint(rect, x, y);
          if (!p) return;
          const hit = nearestAlignPoint(
            foldLayers(st.frame3d, st.doc, st.faces),
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
          if (hit) st.pickAlignTarget({ kind: "point", p: hit }, p);
        } else {
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
          if (hit) {
            st.pickAlignTarget(
              {
                kind: "line",
                a: [hit.a.x, hit.a.y],
                b: [hit.b.x, hit.b.y],
              },
              rawPoint(rect, x, y),
              { kind: "edge", id: hit.edgeId },
            );
          }
        }
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
      const toggle = e.ctrlKey || e.metaKey;
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
    [drawHighlight, rawPoint, updateHoverCursor],
  );

  return (
    <>
      <canvas
        ref={canvasRef}
        className="viewer3d-canvas"
        style={{
          cursor:
            pullMode && pullBlocked === null
              ? "grab"
              : !foldMode || !foldReady
                ? "default"
                : activeTool === "fold" && !alignDraft
                  ? "grab"
                  : "crosshair",
        }}
        data-tooltip={
          pullMode
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
                ? "紙をドラッグして折ります。Ctrl+ドラッグで折り線を指定します"
                : `ドラッグで回転、ホイールで拡大縮小。${SELECTABLE_3D_EDGE_TARGETS}をクリックして選びます`
        }
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={() => {
          downPosRef.current = null;
          drawingRef.current = null;
          grabRef.current = null;
          pullRef.current = null;
          useAppStore.getState().endPull();
          useAppStore.getState().setOperationStage(0);
          drawHighlight();
        }}
        onPointerLeave={(e) => {
          if (!pullRef.current && !grabRef.current && !drawingRef.current) {
            e.currentTarget.style.cursor = "default";
          }
        }}
        onContextMenu={(e) => e.preventDefault()}
      />
      <ViewerOperationHint
        hint={hint}
        blocked={(foldMode && !foldReady) || (pullMode && pullBlocked !== null)}
        aligning={alignDraft !== null}
      />
      <PaperActionTip />
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
