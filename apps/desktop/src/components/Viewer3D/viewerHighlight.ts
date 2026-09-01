import { useCallback, useEffect, type RefObject } from "react";
import * as THREE from "three";
import {
  type SpatialFoldDrag,
  useAppStore,
} from "../../store/appStore";
import { twistPreviewSegments } from "../../lib/twistPolygon";
import type {
  Document as OrigamiDocument,
  Face,
  FoldDirection,
  Frame3D,
  SelfIntersectionPair,
  Vec2,
} from "../../lib/types";
import type { Viewer3DInteractionCapture } from "../../captureApi";
import type {
  SpatialAlignTarget,
  SpatialFoldTarget,
} from "../../lib/spatialAlignTypes";
import {
  foldLayers,
  foldPreviewSegments,
  keepSidePoint,
} from "./foldDraw";
import {
  type HighlightSegment,
  type Viewer3DScene,
} from "./sceneBuilder";
import {
  projectHighlightSegmentsToSoftSurface,
  type SoftHighlightMap,
} from "./softHighlight";
import { planGrabFold, type GrabMode } from "./grabFold";
import type { HingeSegment } from "./hingePicker";
import {
  deriveSelectedEdgeHighlights,
  pointInPolygon,
  type FacePlacement,
} from "./edgeHighlight";
import {
  cpMarkSegments,
  type CpFaceIndex,
} from "./cpPick3d";
import { SPATIAL_REPROJECTION_EPS } from "./spatialAlign";

/** 畳み平面の線分列を強調表示用の線分へ(紙より少しだけ浮かせる) */
function toHighlight(segments: [Vec2, Vec2][]): HingeSegment[] {
  return segments.map(([a, b]) => ({
    edgeId: -1,
    a: new THREE.Vector3(a[0], a[1], PREVIEW_LIFT),
    b: new THREE.Vector3(b[0], b[1], PREVIEW_LIFT),
  }));
}

interface SpatialMarkPlane {
  normal: THREE.Vector3;
  offset: number;
}

function normalizedMarkPlane(
  plane: Extract<SpatialAlignTarget, { kind: "point" }>["supportPlanes"][number],
): SpatialMarkPlane | null {
  const normal = new THREE.Vector3(...plane.normal);
  const point = new THREE.Vector3(...plane.point);
  const length = normal.length();
  if (!Number.isFinite(length) || length <= 1e-18 || !point.toArray().every(Number.isFinite)) {
    return null;
  }
  normal.multiplyScalar(1 / length);
  const components = normal.toArray();
  let largestAxis = 0;
  for (let axis = 1; axis < 3; axis++) {
    if (Math.abs(components[axis]) > Math.abs(components[largestAxis])) largestAxis = axis;
  }
  if (components[largestAxis] < 0) normal.multiplyScalar(-1);
  const offset = normal.dot(point);
  return Number.isFinite(offset) ? { normal, offset } : null;
}

function markPlaneForPoint(
  target: Extract<SpatialAlignTarget, { kind: "point" }>,
  foldTarget: SpatialFoldTarget | null,
): SpatialMarkPlane | null {
  const world = new THREE.Vector3(...target.world);
  const candidates = target.supportPlanes
    .map(normalizedMarkPlane)
    .filter((plane): plane is SpatialMarkPlane => plane !== null)
    .filter((plane) => {
      if (Math.abs(plane.normal.dot(world) - plane.offset) > SPATIAL_REPROJECTION_EPS) {
        return false;
      }
      return (
        foldTarget === null ||
        foldTarget.lineWorld.every(
          (point) =>
            Math.abs(plane.normal.dot(new THREE.Vector3(...point)) - plane.offset) <=
            SPATIAL_REPROJECTION_EPS,
        )
      );
    });
  if (candidates.length === 0) return null;
  const equivalent = (a: SpatialMarkPlane, b: SpatialMarkPlane): boolean =>
    a.normal.distanceTo(b.normal) <= SPATIAL_REPROJECTION_EPS &&
    Math.abs(a.offset - b.offset) <= SPATIAL_REPROJECTION_EPS;
  for (let a = 0; a < candidates.length; a++) {
    for (let b = a + 1; b < candidates.length; b++) {
      if (!equivalent(candidates[a], candidates[b])) return null;
    }
  }
  return [...candidates].sort((a, b) => {
    for (const axis of ["x", "y", "z"] as const) {
      if (a.normal[axis] !== b.normal[axis]) return a.normal[axis] - b.normal[axis];
    }
    return a.offset - b.offset;
  })[0];
}

function spatialPointMark(
  target: Extract<SpatialAlignTarget, { kind: "point" }>,
  foldTarget: SpatialFoldTarget | null,
): HingeSegment[] {
  const plane = markPlaneForPoint(target, foldTarget);
  if (!plane) return [];
  const normal = plane.normal;
  const reference =
    Math.abs(normal.x) < 0.9
      ? new THREE.Vector3(1, 0, 0)
      : new THREE.Vector3(0, 1, 0);
  const u = new THREE.Vector3().crossVectors(normal, reference).normalize();
  const v = new THREE.Vector3().crossVectors(normal, u).normalize();
  const center = new THREE.Vector3(...target.world);
  return [u, v].map((axis) => ({
    edgeId: -1,
    a: center.clone().addScaledVector(axis, -CENTER_MARK),
    b: center.clone().addScaledVector(axis, CENTER_MARK),
  }));
}

/** spatial cycleの選択値と解を、global XYへ落とさずworld線のまま表示する。 */
export function spatialAlignHighlightSegments(
  picks: readonly (SpatialAlignTarget | null)[],
  foldTarget: SpatialFoldTarget | null,
): HingeSegment[] {
  const segments: HingeSegment[] = [];
  for (const target of picks) {
    if (target === null) continue;
    if (target.kind === "point") {
      segments.push(...spatialPointMark(target, foldTarget));
    } else {
      segments.push({
        edgeId: -1,
        a: new THREE.Vector3(...target.aWorld),
        b: new THREE.Vector3(...target.bWorld),
      });
    }
  }
  if (foldTarget !== null) {
    segments.push({
      edgeId: -1,
      a: new THREE.Vector3(...foldTarget.lineWorld[0]),
      b: new THREE.Vector3(...foldTarget.lineWorld[1]),
    });
  }
  return segments;
}

const SPATIAL_PREVIEW_EPS = 1e-9;

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
  if (grab.mode === "single") return new Set([grab.face]);
  if (grab.mode === "all") {
    return new Set(faces.filter((face) => reachesSide(face.id)).map((face) => face.id));
  }
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
export interface SpatialFoldFacePreview {
  readonly face: number;
  readonly segments: readonly HighlightSegment[];
}

export interface SpatialFoldPreviewPlan {
  readonly faces: readonly SpatialFoldFacePreview[];
  readonly segments: readonly HighlightSegment[];
}

const emptySpatialFoldPreview = (): SpatialFoldPreviewPlan => ({
  faces: [],
  segments: [],
});

/**
 * 画面で光らせる面と線を同じ計算結果として返す。
 * selectedLayerCount はこの faces.length を使い、表示と枚数を別々に推測しない。
 */
export function spatialFoldPreviewPlan(
  frame: Frame3D | null,
  faces: readonly Face[],
  grab: Extract<GrabState, { spatial: true }>,
): SpatialFoldPreviewPlan {
  if (!frame) return emptySpatialFoldPreview();
  const from = new THREE.Vector3(...grab.a);
  const to = new THREE.Vector3(...grab.b);
  const normal = to.clone().sub(from);
  if (normal.lengthSq() <= SPATIAL_PREVIEW_EPS ** 2) {
    return emptySpatialFoldPreview();
  }
  normal.normalize();
  const origin = from.clone().add(to).multiplyScalar(0.5);
  const signed = normal.dot(from.clone().sub(origin));
  const movingSign = Math.abs(signed) > SPATIAL_PREVIEW_EPS ? Math.sign(signed) : -1;
  const selected = spatialPreviewFaces(frame, faces, grab, origin, normal, movingSign);
  const facePreviews: SpatialFoldFacePreview[] = [];
  const segments: HighlightSegment[] = [];
  for (const face of frame.faces) {
    if (!selected.has(face.face)) continue;
    const faceSegments: HighlightSegment[] = [];
    const crease = spatialCrease(face.polygon, origin, normal);
    if (crease) {
      faceSegments.push({ edgeId: -1, a: crease[0], b: crease[1], role: "reference" });
    }
    const moving = clipSpatialPolygon(face.polygon, origin, normal, movingSign).map(
      (point) => point.sub(normal.clone().multiplyScalar(2 * normal.dot(point.clone().sub(origin)))),
    );
    for (let i = 0; i < moving.length; i++) {
      faceSegments.push({
        edgeId: -1,
        a: moving[i],
        b: moving[(i + 1) % moving.length],
        role: "active",
      });
    }
    facePreviews.push({ face: face.face, segments: faceSegments });
    segments.push(...faceSegments);
  }
  return { faces: facePreviews, segments };
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

/** ねじり折りの中心を示す十字の腕の長さ(正規化座標) */
const CENTER_MARK = 0.02;
/** プレビュー線を紙より少しだけ上に浮かせる高さ(重なりのちらつき防止) */
const PREVIEW_LIFT = 0.002;
/** 折った結果の下見(半透明の面)を浮かせる高さ。層のずらし表示より上に置く */
const PREVIEW_FILL_LIFT = 0.045;

export type GrabState = {
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

const inactiveViewer3DInteraction = (): Viewer3DInteractionCapture => ({
  grab: {
    active: false,
    spatial: null,
    face: null,
    mode: null,
    selectedLayerCount: 0,
  },
  preview: {
    visible: false,
    polygonCount: 0,
    segmentCount: 0,
  },
});

/**
 * captureから要求された時だけ、現在の通常grabと実際に表示するpreviewを再計算する。
 * selectedLayerCountは選択層数であり、完全折りの表裏組を数える「ひだ数」ではない。
 */
export function deriveViewer3DInteractionCapture({
  grab,
  frame,
  doc,
  faces,
}: {
  readonly grab: GrabState | null;
  readonly frame: Frame3D | null;
  readonly doc: OrigamiDocument | null;
  readonly faces: Face[];
}): Viewer3DInteractionCapture {
  if (!grab) return inactiveViewer3DInteraction();

  const capturedGrab = {
    active: true,
    spatial: grab.spatial,
    face: grab.face,
    mode: grab.mode,
  } as const;

  if (grab.spatial) {
    const plan = spatialFoldPreviewPlan(frame, faces, grab);
    return {
      grab: { ...capturedGrab, selectedLayerCount: plan.faces.length },
      preview: {
        visible: plan.segments.length > 0,
        polygonCount: 0,
        segmentCount: plan.segments.length,
      },
    };
  }

  if (!doc) {
    return {
      grab: { ...capturedGrab, selectedLayerCount: 0 },
      preview: { visible: false, polygonCount: 0, segmentCount: 0 },
    };
  }
  const plan = planGrabFold(
    foldLayers(frame, doc, faces),
    faces,
    grab.a,
    grab.b,
    grab.mode,
    grab.face,
  );
  if (!plan.ok) {
    return {
      grab: { ...capturedGrab, selectedLayerCount: 0 },
      preview: { visible: false, polygonCount: 0, segmentCount: 0 },
    };
  }
  const polygonCount = plan.plan.preview.length;
  const segmentCount = plan.plan.segments.length;
  return {
    grab: {
      ...capturedGrab,
      selectedLayerCount: plan.plan.selectedLayerCount,
    },
    preview: {
      visible: polygonCount > 0 || segmentCount > 0,
      polygonCount,
      segmentCount,
    },
  };
}


export interface ViewerHighlightRefresh {
  readonly selection: unknown;
  readonly hoveredHinge: unknown;
  readonly suspectHinges: unknown;
  readonly penetrationDetectionEnabled: unknown;
  readonly selfIntersectionPairs: unknown;
  readonly focusedSelfIntersectionPairIndex: unknown;
  readonly pinnedFolds: unknown;
  readonly foldAllActive: unknown;
  readonly activeAngleIntent: unknown;
  readonly doc: unknown;
  readonly faces: unknown;
  readonly hinges: unknown;
  readonly frame3d: unknown;
  readonly foldDraft: unknown;
  readonly pendingFoldThrough: unknown;
  readonly alignDraft: unknown;
  readonly techniqueDraft: unknown;
  readonly activeTool: unknown;
  readonly measureDraft: unknown;
  readonly pullHinge: unknown;
  readonly pullMirrorHinge: unknown;
  readonly softMesh: unknown;
}

/** backendが返した順序付き面ペアの両輪郭だけを、既存の赤いsuspect線へ変える。 */
export function selfIntersectionFaceOutlines(
  faces: readonly Face[],
  physicalEdgeSegments: readonly HighlightSegment[],
  pairs: readonly SelfIntersectionPair[],
  focusedIndex: number,
): HighlightSegment[] {
  if (pairs.length === 0) return [];
  const pair = pairs[focusedIndex % pairs.length];
  const left = faces.find((face) => face.id === pair[0]);
  const right = faces.find((face) => face.id === pair[1]);
  if (left === undefined || right === undefined) return [];
  const edgeIdsByFace = new Map<number, ReadonlySet<number>>([
    [left.id, new Set(left.edges)],
    [right.id, new Set(right.edges)],
  ]);
  return physicalEdgeSegments
    .filter((segment) => {
      if (segment.ownerFace === undefined) return false;
      return edgeIdsByFace.get(segment.ownerFace)?.has(segment.edgeId) === true;
    })
    .map((segment) => ({ ...segment, role: "suspect" as const }));
}

export interface UseViewerHighlightArgs {
  readonly sceneRef: RefObject<Viewer3DScene | null>;
  readonly softHighlightRef: RefObject<SoftHighlightMap | null>;
  readonly selectableEdgeSegmentsRef: RefObject<HingeSegment[]>;
  readonly pendingCpPointRef: RefObject<Vec2 | null>;
  readonly curvePointsRef: RefObject<Vec2[]>;
  readonly constructRef: RefObject<{
    points: Vec2[];
    seg: [Vec2, Vec2] | null;
  }>;
  readonly foldClickRef: RefObject<Vec2 | null>;
  readonly vertexDragRef: RefObject<{
    id: number;
    faceId: number;
    from: Vec2;
    to: Vec2;
  } | null>;
  readonly drawingRef: RefObject<{ a: Vec2; b: Vec2 } | null>;
  readonly grabRef: RefObject<GrabState | null>;
  readonly cpIndex: () => CpFaceIndex | null;
  readonly facePlacementOf: (faceId: number) => FacePlacement | null;
  readonly refresh: ViewerHighlightRefresh;
}

export interface ViewerHighlightApi {
  readonly cpPointHighlight: (
    cp: Vec2,
    faceId?: number,
  ) => HighlightSegment[];
  readonly drawHighlight: () => void;
}

export function useViewerHighlight({
  sceneRef,
  softHighlightRef,
  selectableEdgeSegmentsRef,
  pendingCpPointRef,
  curvePointsRef,
  constructRef,
  foldClickRef,
  vertexDragRef,
  drawingRef,
  grabRef,
  cpIndex,
  facePlacementOf,
  refresh,
}: UseViewerHighlightArgs): ViewerHighlightApi {
  const {
    selection,
    hoveredHinge,
    suspectHinges,
    penetrationDetectionEnabled,
    selfIntersectionPairs,
    focusedSelfIntersectionPairIndex,
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
  } = refresh;

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
    const selfIntersectionSegments = penetrationDetectionEnabled
      ? selfIntersectionFaceOutlines(
          s.faces,
          physicalEdgeSegments,
          s.selfIntersectionPairs,
          s.focusedSelfIntersectionPairIndex,
        )
      : [];
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
        ...selfIntersectionSegments,
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
        setHighlight(spatialFoldPreviewPlan(s.frame3d, s.faces, grab).segments.slice());
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
      if (s.alignDraft.spatialPicks !== undefined) {
        const spatialTarget =
          s.foldDraft &&
          Object.prototype.hasOwnProperty.call(s.foldDraft, "spatialTarget")
            ? (s.foldDraft.spatialTarget ?? null)
            : null;
        const spatialSegments = spatialAlignHighlightSegments(
          s.alignDraft.spatialPicks,
          spatialTarget,
        );
        const foldedPlane = spatialTarget?.foldedPlane ?? null;
        const keep =
          foldedPlane && s.foldDraft
            ? foldedPlane.keepPointForMovingSide[s.foldDraft.movingSide]
            : null;
        if (foldedPlane && keep && s.foldDraft) {
          // raw Frame3Dでもz=0畳み平面へ一意に戻せたときだけ、従来の黄色い紙輪郭を足す。
          // 先頭の解線はworld表示済みなので除き、非平坦面をXYへfallbackしない。
          spatialSegments.push(
            ...toHighlight(
              foldPreviewSegments(
                foldLayers(s.frame3d, s.doc, s.faces),
                foldedPlane.line,
                keep,
                s.foldDraft.target === "top",
              ).slice(1),
            ),
          );
        }
        setHighlight(spatialSegments);
        return;
      }
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
  }, [
    constructRef,
    cpPointHighlight,
    curvePointsRef,
    drawingRef,
    foldClickRef,
    grabRef,
    pendingCpPointRef,
    penetrationDetectionEnabled,
    sceneRef,
    selectableEdgeSegmentsRef,
    softHighlightRef,
    vertexDragRef,
  ]);

  // 選択・折り線プレビューの強調(上の効果で線分が更新された後に走る)
  useEffect(() => {
    drawHighlight();
  }, [
    selection,
    hoveredHinge,
    suspectHinges,
    penetrationDetectionEnabled,
    selfIntersectionPairs,
    focusedSelfIntersectionPairIndex,
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

  return { cpPointHighlight, drawHighlight };
}
