import {
  useCallback,
  useEffect,
  useRef,
  type MouseEvent as ReactMouseEvent,
  type MouseEventHandler,
  type PointerEvent as ReactPointerEvent,
  type PointerEventHandler,
  type RefObject,
} from "react";
import * as THREE from "three";
import {
  canFoldNow,
  isSpatialFoldFrame,
  pullBlockedOf,
  type AlignDraft,
  type MeasureDraft,
  type SpatialFoldDrag,
  type ToolId,
  useAppStore,
} from "../../store/appStore";
import type { SpatialAlignPickResult } from "../../store/slices/documentSlice";
import { viewerHint } from "../../lib/viewerHint";
import { measureGuide } from "../../lib/measureGuide";
import {
  hingeAnglesFromFrame,
  planPull,
  pullDeltaDeg,
  type PullPlan,
} from "../../lib/grabDrive";
import { paperExtent } from "../CpEditor/snap";
import { TOOL_KIND } from "../CpEditor/interaction";
import { CONSTRUCT_STEPS, constructLines } from "../../lib/construct";
import { CURVE_STEPS, curvePolyline } from "../../lib/curve";
import type {
  EdgeKind,
  FoldDirection,
  Frame3D,
  Vec2,
} from "../../lib/types";
import { facesAtPoint, foldLayers } from "./foldDraw";
import type { FacePlacement } from "./edgeHighlight";
import { type GrabMode } from "./grabFold";
import {
  pickFace,
  pickHingeSegment,
  pickPaper,
  type HingeSegment,
} from "./hingePicker";
import {
  cpPointOnFacePlane,
  isBorderVertex,
  type CpPick3D,
} from "./cpPick3d";
import type { Viewer3DScene } from "./sceneBuilder";
import type { GrabState } from "./viewerHighlight";
import {
  ALIGN_PICK_PX,
  displayedPickSurface,
  type AlignPick,
  type ViewerPickingApi,
} from "./viewerPicking";
import { ALIGN_STEPS } from "../../lib/alignFold";
import { solveSpatialAlignOnCommonPlane } from "./spatialAlign";

/** これ以上動かしたら「クリック」ではなく視点操作とみなす(px) */
const CLICK_MOVE_PX = 4;
/** これ未満の長さの折り線は引かなかったことにする(正規化座標) */
const MIN_FOLD_LENGTH = 1e-4;
/** 技法のフラップ選択で、層の輪郭からこの距離以内なら「その場所にある」とみなす */
const FLAP_PICK_EPS = 1e-3;
const SPATIAL_PREVIEW_EPS = 1e-9;

export interface ViewerDrawingState {
  a: Vec2;
  b: Vec2;
}

export interface ViewerConstructDraft {
  points: Vec2[];
  seg: [Vec2, Vec2] | null;
}

export interface ViewerVertexDragState {
  id: number;
  faceId: number;
  from: Vec2;
  to: Vec2;
}

export interface ViewerPullState {
  plan: PullPlan;
  origin: THREE.Vector3;
  ndc: THREE.Vector3;
  x: number;
  y: number;
}

/** C8が所有する、表示専用・入力途中の一時値。WebGL資源は含まない。 */
export interface ViewerPointerRefs {
  readonly downPosRef: RefObject<{ x: number; y: number } | null>;
  readonly drawingRef: RefObject<ViewerDrawingState | null>;
  readonly pendingCpPointRef: RefObject<Vec2 | null>;
  readonly curvePointsRef: RefObject<Vec2[]>;
  readonly constructRef: RefObject<ViewerConstructDraft>;
  readonly foldClickRef: RefObject<Vec2 | null>;
  readonly vertexDragRef: RefObject<ViewerVertexDragState | null>;
  readonly hoverLockRef: RefObject<boolean>;
  readonly alignPressRef: RefObject<AlignPick | null>;
  readonly measurePressRef: RefObject<CpPick3D | null>;
  readonly grabRef: RefObject<GrabState | null>;
  readonly pullRef: RefObject<ViewerPullState | null>;
}

/** C9が下見を描くために読むrefだけを公開した境界。 */
export type ViewerPointerVisualRefs = Pick<
  ViewerPointerRefs,
  | "drawingRef"
  | "pendingCpPointRef"
  | "curvePointsRef"
  | "constructRef"
  | "foldClickRef"
  | "vertexDragRef"
  | "grabRef"
>;

/** 12個の一時refをC8へまとめる。作成・破棄するWebGL資源はない。 */
export function useViewerPointerRefs(): ViewerPointerRefs {
  const downPosRef = useRef<{ x: number; y: number } | null>(null);
  const drawingRef = useRef<ViewerDrawingState | null>(null);
  const pendingCpPointRef = useRef<Vec2 | null>(null);
  const curvePointsRef = useRef<Vec2[]>([]);
  const constructRef = useRef<ViewerConstructDraft>({ points: [], seg: null });
  const foldClickRef = useRef<Vec2 | null>(null);
  const vertexDragRef = useRef<ViewerVertexDragState | null>(null);
  const hoverLockRef = useRef(false);
  const alignPressRef = useRef<AlignPick | null>(null);
  const measurePressRef = useRef<CpPick3D | null>(null);
  const grabRef = useRef<GrabState | null>(null);
  const pullRef = useRef<ViewerPullState | null>(null);
  return {
    downPosRef,
    drawingRef,
    pendingCpPointRef,
    curvePointsRef,
    constructRef,
    foldClickRef,
    vertexDragRef,
    hoverLockRef,
    alignPressRef,
    measurePressRef,
    grabRef,
    pullRef,
  };
}

export type ViewerPointerPickingPort = Pick<
  ViewerPickingApi,
  | "planePoint"
  | "rawPoint"
  | "cpPickAt"
  | "measurePointFromPick"
  | "resolveAlignPick"
  | "facePlacementOf"
  | "facePlacements"
>;

export interface UseViewerPointerArgs {
  readonly sceneRef: RefObject<Viewer3DScene | null>;
  readonly selectableEdgeSegmentsRef: RefObject<HingeSegment[]>;
  readonly refs: ViewerPointerRefs;
  readonly picking: ViewerPointerPickingPort;
  readonly view: ViewerPointerViewState;
  /** C6と共有する更新番号。Viewerで1回だけ購読して渡す。 */
  readonly docEpoch: number;
  /** C9のstructural port。C8からC9をruntime importしない。 */
  readonly drawHighlight: () => void;
  readonly setHoverLock: (locked: boolean) => void;
}

export interface UseViewerPointerPreludeArgs {
  readonly sceneRef: RefObject<Viewer3DScene | null>;
  readonly refs: ViewerPointerRefs;
  readonly view: ViewerPointerViewState;
}

export interface ViewerPointerPreludeApi {
  readonly setHoverLock: (locked: boolean) => void;
}

export interface ViewerPointerViewState {
  readonly foldAllActive: boolean;
  readonly activeTool: ToolId;
  readonly measureDraft: MeasureDraft;
  readonly alignDraft: AlignDraft | null;
  readonly foldReady: boolean;
  readonly pullBlocked: ReturnType<typeof pullBlockedOf>;
  readonly foldMode: boolean;
  readonly pullMode: boolean;
  readonly hint: string;
}

export interface ViewerPointerCanvasHandlers {
  readonly onPointerDown: PointerEventHandler<HTMLCanvasElement>;
  readonly onPointerMove: PointerEventHandler<HTMLCanvasElement>;
  readonly onPointerUp: PointerEventHandler<HTMLCanvasElement>;
  readonly onPointerCancel: PointerEventHandler<HTMLCanvasElement>;
  readonly onPointerLeave: PointerEventHandler<HTMLCanvasElement>;
  readonly onContextMenu: MouseEventHandler<HTMLCanvasElement>;
}

export interface ViewerPointerApi {
  readonly handlers: ViewerPointerCanvasHandlers;
  readonly view: ViewerPointerViewState;
}

/** C8とC9/JSXが共有する7つの購読。effectを持たないので先に呼べる。 */
export function useViewerPointerState(): ViewerPointerViewState {
  const foldAllActive = useAppStore((s) => s.foldAllPreview !== null);
  const activeTool = useAppStore((s) => s.activeTool);
  const measureDraft = useAppStore((s) => s.measureDraft);
  const alignDraft = useAppStore((s) => s.alignDraft);
  const foldReady = useAppStore(
    (s) =>
      canFoldNow(s) && !s.foldThroughBusy && s.pendingFoldThrough === null,
  );
  const pullBlocked = useAppStore(pullBlockedOf);
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
      driverAngles: [...s.drivers.values()],
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
  const foldMode = activeTool === "fold" || activeTool === "technique";
  const pullMode = activeTool === "pull";
  return {
    foldAllActive,
    activeTool,
    measureDraft,
    alignDraft,
    foldReady,
    pullBlocked,
    foldMode,
    pullMode,
    hint,
  };
}

/** 2点の距離(展開図・畳み平面のどちらの座標でも使う) */
function distance2(a: Vec2, b: Vec2): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1]);
}

/**
 * 1回のpointer-upでstore第4引数へ渡すraw spatial結果を作る。
 * 未完成pickはtargetだけ、完成pickはsolverのnull slotを圧縮せずそのまま渡す。
 */
export function buildSpatialAlignPickResult(
  draft: AlignDraft,
  pressed: AlignPick,
  placements: readonly FacePlacement[],
): SpatialAlignPickResult {
  const stepCount = ALIGN_STEPS[draft.mode].length;
  const restarting = draft.picks.length >= stepCount;
  const previousSpatialPicks = restarting
    ? []
    : (draft.spatialPicks ?? draft.picks.map(() => null));
  const picks = [...previousSpatialPicks, pressed.spatialTarget];
  if (picks.length < stepCount) {
    return {
      target: pressed.spatialTarget,
      solutions: [],
      materialSolutions: [],
      reason: null,
    };
  }
  if (picks.some((pick) => pick === null)) {
    return {
      target: pressed.spatialTarget,
      solutions: [],
      materialSolutions: [],
      reason: "3Dで選んだ点・線を同じ支持面の値として説明できません",
    };
  }
  const solved = solveSpatialAlignOnCommonPlane({
    mode: draft.mode,
    picks: picks.filter((pick): pick is NonNullable<typeof pick> => pick !== null),
    cursorWorld: pressed.spatialCursorWorld,
    placements,
  });
  return {
    target: pressed.spatialTarget,
    solutions: solved.solutions,
    materialSolutions: solved.materialSolutions,
    reason: solved.reason,
  };
}

/** 修飾キーから「何枚の紙を動かすか」を決める(説明は常にヒント行に出す) */
function grabMode(e: { shiftKey: boolean; altKey: boolean }): GrabMode {
  if (e.shiftKey) return "all";
  if (e.altKey) return "single";
  return "flap";
}

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
  return normal.lengthSq() > SPATIAL_PREVIEW_EPS ** 2
    ? normal.normalize()
    : null;
}

/** 180°でもドラッグ途中の材質表裏から山谷を保つ。 */
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

/** C9直後・C6直前に置き、従来のeffect登録順を保つ前半境界。 */
export function useViewerPointerPrelude({
  sceneRef,
  refs,
  view,
}: UseViewerPointerPreludeArgs): ViewerPointerPreludeApi {
  const { drawingRef, grabRef, hoverLockRef, pullRef } = refs;
  const {
    foldAllActive,
    activeTool,
    alignDraft,
    foldReady,
    pullBlocked,
    foldMode,
    pullMode,
  } = view;

  useEffect(() => {
    hoverLockRef.current = false;
    sceneRef.current?.setDrawMode(
      !foldAllActive &&
        ((foldMode && foldReady && !alignDraft) ||
          (pullMode && pullBlocked === null)),
      !foldAllActive && pullMode,
    );
  }, [
    foldMode,
    foldReady,
    pullMode,
    pullBlocked,
    alignDraft,
    activeTool,
    foldAllActive,
    hoverLockRef,
    sceneRef,
  ]);

  const setHoverLock = useCallback(
    (locked: boolean) => {
      if (hoverLockRef.current === locked) return;
      hoverLockRef.current = locked;
      sceneRef.current?.setDrawMode(locked);
    },
    [hoverLockRef, sceneRef],
  );

  useEffect(() => {
    if (!foldMode) {
      drawingRef.current = null;
      grabRef.current = null;
    }
  }, [drawingRef, foldMode, grabRef]);

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

  useEffect(() => {
    if (!pullMode) {
      pullRef.current = null;
      useAppStore.getState().endPull();
    }
  }, [pullMode, pullRef]);

  return { setHoverLock };
}

/**
 * Viewerのポインタ入力をまとめる。Zustandの状態は従来どおりgetStateで都度読み、
 * scene・表示線・C7/C9のAPIは所有せず借りる。
 */
export function useViewerPointer({
  sceneRef,
  selectableEdgeSegmentsRef,
  refs,
  picking,
  view,
  docEpoch,
  drawHighlight,
  setHoverLock,
}: UseViewerPointerArgs): ViewerPointerApi {
  const {
    downPosRef,
    drawingRef,
    pendingCpPointRef,
    curvePointsRef,
    constructRef,
    foldClickRef,
    vertexDragRef,
    alignPressRef,
    measurePressRef,
    grabRef,
    pullRef,
  } = refs;
  const {
    planePoint,
    rawPoint,
    cpPickAt,
    measurePointFromPick,
    resolveAlignPick,
    facePlacementOf,
    facePlacements,
  } = picking;
  const {
    activeTool,
    measureDraft,
  } = view;

  const clearCpDrafts = useCallback(() => {
    pendingCpPointRef.current = null;
    curvePointsRef.current = [];
    constructRef.current = { points: [], seg: null };
    foldClickRef.current = null;
    vertexDragRef.current = null;
    measurePressRef.current = null;
  }, [
    constructRef,
    curvePointsRef,
    foldClickRef,
    measurePressRef,
    pendingCpPointRef,
    vertexDragRef,
  ]);

  useEffect(() => {
    clearCpDrafts();
    drawHighlight();
  }, [activeTool, measureDraft.mode, docEpoch, clearCpDrafts, drawHighlight]);

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

  const addCpLinePoint = useCallback(
    (cp: Vec2, kind: EdgeKind) => {
      const s = useAppStore.getState();
      if (!s.doc) return;
      if (s.curve.enabled) {
        const points = curvePointsRef.current;
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
    },
    [curvePointsRef, pendingCpPointRef],
  );

  const addConstructPick = useCallback(
    (pick: CpPick3D) => {
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
        if (!a || !b) return;
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
      if (lines.length > 0) {
        void s.applyEdit(
          lines.map(
            ([a, b]) => ({ type: "AddSegment", a, b, kind: "Aux" }) as const,
          ),
        );
      }
    },
    [constructRef],
  );

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
        const hit =
          surface &&
          pickPaper(
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
          const hit = resolveAlignPick(rect, x, y) !== null;
          setHoverLock(hit);
          canvas.style.cursor = hit ? "pointer" : "default";
          return;
        }
        if (ctrlKey) {
          canvas.style.cursor = "crosshair";
          return;
        }
        const face =
          surface &&
          pickFace(
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
        if (
          s.techniqueDraft?.kind === "Simple" &&
          s.techniqueDraft.motionMode === "reflect"
        ) {
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
      const drawsOnCp =
        TOOL_KIND[s.activeTool] !== undefined || s.activeTool === "construct";
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
      const edgeId =
        pickHingeSegment(
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
      const paper =
        surface &&
        pickPaper(
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
    [
      cpPickAt,
      resolveAlignPick,
      sceneRef,
      selectableEdgeSegmentsRef,
      setHoverLock,
    ],
  );

  const handlePointerDown = useCallback(
    (e: ReactPointerEvent<HTMLCanvasElement>) => {
      const rect = e.currentTarget.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const s = useAppStore.getState();
      if (s.foldAllPreview !== null) {
        pullRef.current = null;
        grabRef.current = null;
        drawingRef.current = null;
        vertexDragRef.current = null;
        alignPressRef.current = null;
        measurePressRef.current = null;
        downPosRef.current = { x, y };
        if (e.button === 0 || e.button === 2) {
          e.currentTarget.style.cursor = "grabbing";
        }
        return;
      }
      const scene0 = sceneRef.current;
      if (
        e.button === 0 &&
        s.activeTool === "pull" &&
        pullBlockedOf(s) === null &&
        s.doc &&
        scene0?.content
      ) {
        const surface = displayedPickSurface(scene0);
        const hit =
          surface &&
          pickPaper(
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
        const hit =
          surface &&
          pickPaper(
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
            const a: SpatialFoldDrag["from"] = [
              hit.point.x,
              hit.point.y,
              hit.point.z,
            ];
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
          setHoverLock(true);
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.style.cursor = "pointer";
          downPosRef.current = { x, y };
          return;
        }
      }
      if (
        e.button === 0 &&
        s.activeTool === "select" &&
        s.doc &&
        !e.ctrlKey &&
        !e.metaKey
      ) {
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
      alignPressRef.current =
        e.button === 0 && s.activeTool === "fold" && s.alignDraft
          ? resolveAlignPick(rect, x, y)
          : null;
      if (alignPressRef.current) {
        setHoverLock(true);
        e.currentTarget.setPointerCapture(e.pointerId);
        e.currentTarget.style.cursor = "pointer";
        downPosRef.current = { x, y };
        return;
      }
      downPosRef.current = { x, y };
      if (e.button === 0 || e.button === 2) {
        e.currentTarget.style.cursor = "grabbing";
      }
    },
    [
      alignPressRef,
      cpPickAt,
      downPosRef,
      drawHighlight,
      drawingRef,
      grabRef,
      measurePressRef,
      planePoint,
      pullRef,
      rawPoint,
      resolveAlignPick,
      sceneRef,
      setHoverLock,
      vertexDragRef,
    ],
  );

  const handlePointerMove = useCallback(
    (e: ReactPointerEvent<HTMLCanvasElement>) => {
      if (useAppStore.getState().foldAllPreview !== null) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const pull = pullRef.current;
      const scene = sceneRef.current;
      if (pull && scene) {
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
        grab.mode = grabMode(e);
        drawHighlight();
        return;
      }
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
    [
      drawHighlight,
      drawingRef,
      facePlacementOf,
      grabRef,
      planePoint,
      pullRef,
      rawPoint,
      sceneRef,
      updateHoverCursor,
      vertexDragRef,
    ],
  );

  const handlePointerUp = useCallback(
    (e: ReactPointerEvent<HTMLCanvasElement>) => {
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
        drawHighlight();
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
            if (e.ctrlKey) s.setTechniqueCenter(a);
            else if (e.shiftKey) selectTechniqueFlap();
            else s.addTechniqueVertex(a);
          } else if (
            !drawn &&
            e.ctrlKey &&
            s.techniqueDraft.kind !== "Simple"
          ) {
            s.setTechniqueReferencePoint(a);
          } else if (drawn) {
            if (
              !(
                s.techniqueDraft.kind === "Simple" &&
                s.techniqueDraft.motionMode === "reflect"
              )
            ) {
              s.setTechniqueLine([a, b]);
            }
          } else {
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
          const rect = e.currentTarget.getBoundingClientRect();
          const pick = cpPickAt(
            rect,
            e.clientX - rect.left,
            e.clientY - rect.top,
            ALIGN_PICK_PX,
          );
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
          const spatial = buildSpatialAlignPickResult(
            st.alignDraft,
            pressed,
            facePlacements(),
          );
          st.pickAlignTarget(
            pressed.target,
            pressed.cursor,
            pressed.cpPick,
            spatial,
          );
        }
        updateHoverCursor(e.currentTarget, x, y, e.ctrlKey);
        return;
      }
      if (st.activeTool === "measure") {
        if (measured) {
          if (st.measureDraft.mode === "distance") {
            const point = measurePointFromPick(measured, rect, down.x, down.y);
            if (point) st.pickMeasurePoint(point);
          } else if (measured.edgeId !== null) {
            st.pickMeasureEdge(measured.edgeId);
          }
        }
        drawHighlight();
        updateHoverCursor(e.currentTarget, x, y, e.ctrlKey);
        return;
      }
      if (Math.hypot(x - down.x, y - down.y) > CLICK_MOVE_PX) return;
      if (st.activeTool === "fold" && st.alignDraft && st.doc) return;
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
      const vertexId =
        st.activeTool === "select" ? (cpPick?.vertexId ?? null) : null;
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
      const edgeId =
        pickHingeSegment(
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
        const paper =
          surface &&
          pickPaper(
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
      addConstructPick,
      addCpLinePoint,
      alignPressRef,
      cpPickAt,
      downPosRef,
      drawHighlight,
      drawingRef,
      foldClickRef,
      facePlacements,
      grabRef,
      measurePointFromPick,
      measurePressRef,
      pullRef,
      sceneRef,
      selectableEdgeSegmentsRef,
      updateHoverCursor,
      vertexDragRef,
    ],
  );

  const handlePointerCancel = useCallback(
    () => {
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
    },
    [
      alignPressRef,
      downPosRef,
      drawHighlight,
      drawingRef,
      grabRef,
      measurePressRef,
      pullRef,
      vertexDragRef,
    ],
  );

  const handlePointerLeave = useCallback(
    (e: ReactPointerEvent<HTMLCanvasElement>) => {
      if (
        !pullRef.current &&
        !grabRef.current &&
        !drawingRef.current &&
        !vertexDragRef.current
      ) {
        e.currentTarget.style.cursor = "default";
      }
    },
    [drawingRef, grabRef, pullRef, vertexDragRef],
  );

  const handleContextMenu = useCallback(
    (e: ReactMouseEvent<HTMLCanvasElement>) => e.preventDefault(),
    [],
  );

  return {
    handlers: {
      onPointerDown: handlePointerDown,
      onPointerMove: handlePointerMove,
      onPointerUp: handlePointerUp,
      onPointerCancel: handlePointerCancel,
      onPointerLeave: handlePointerLeave,
      onContextMenu: handleContextMenu,
    },
    view,
  };
}
