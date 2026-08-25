import { useCallback, type RefObject } from "react";
import {
  type AlignCpPick,
  useAppStore,
} from "../../store/appStore";
import { snap } from "../CpEditor/snap";
import {
  foldedAlignPoint,
  SNAP_RADIUS_PX,
} from "../CpEditor/interaction";
import { planeRadius, screenToPlane } from "../../lib/planeProject";
import type { Vec2 } from "../../lib/types";
import { ALIGN_STEPS, type AlignTarget } from "../../lib/alignFold";
import { nearestAlignPoint } from "../../lib/alignPick";
import { foldLayers, snapFoldPoint } from "./foldDraw";
import {
  pickHingeSegment,
  type HingeSegment,
  type PaperPickSurface,
} from "./hingePicker";
import {
  pointInPolygon,
  type FacePlacement,
} from "./edgeHighlight";
import {
  buildCpFaceIndex,
  cpPointOnFacePlane,
  pickCpFromPixel,
  placementOf,
  type CpFaceIndex,
  type CpPick3D,
} from "./cpPick3d";
import type { Viewer3DScene } from "./sceneBuilder";

/** 折り線の端点を紙の点・輪郭へ吸着させる距離(px) */
const FOLD_SNAP_PX = 14;
/** 平面へ投影できないときの吸着半径(正規化座標) */
const FOLD_SNAP_FALLBACK = 0.02;
/** 合わせて折るときに、点・線を拾う許容距離(px) */
export const ALIGN_PICK_PX = 16;

/** たわみONでは、見えている細分網をowner/pickerの両方で使う。 */
export function displayedPickSurface(
  scene: Viewer3DScene,
): PaperPickSurface | null {
  if (scene.pickSurface) return scene.pickSurface;
  const content = scene.content;
  if (!content) return null;
  return {
    mesh: content.mesh,
    triangleFaceIds: content.topology.triangleFaceIds,
    triangleLayers:
      content.owner?.triangleLayers ??
      new Array(content.topology.triangleFaceIds.length).fill(0),
    faceSurfaceRanks:
      content.owner?.faceSurfaceRanks ?? new Map<number, number>(),
  };
}

/** 合わせて折るで1つ選んだ結果(そのままpickAlignTargetへ渡せる形) */
export interface AlignPick {
  target: AlignTarget;
  /** 解を並べ替える基準になるクリック位置(畳み平面座標)。無ければnull */
  cursor: Vec2 | null;
  /** 展開図側の識別子。展開図の頂点・辺として拾えたときだけ入る */
  cpPick: AlignCpPick | null;
}

/** 測定へ渡す、3D上の点を展開図へ逆写像した結果。 */
export interface MeasurePointPick {
  cp: Vec2;
  faceId: number;
  vertexId: number | null;
}

export interface ViewerPickingOptions {
  sceneRef: RefObject<Viewer3DScene | null>;
  cpIndexRef: RefObject<CpFaceIndex | null>;
  selectableEdgeSegmentsRef: RefObject<HingeSegment[]>;
}

export interface ViewerPickingApi {
  cpIndex: () => CpFaceIndex | null;
  facePlacementOf: (faceId: number) => FacePlacement | null;
  planePoint: (rect: DOMRect, x: number, y: number) => Vec2 | null;
  rawPoint: (rect: DOMRect, x: number, y: number) => Vec2 | null;
  cpPickAt: (
    rect: DOMRect,
    x: number,
    y: number,
    thresholdPx?: number,
  ) => CpPick3D | null;
  measurePointFromPick: (
    pick: CpPick3D,
    rect: DOMRect,
    x: number,
    y: number,
  ) => MeasurePointPick | null;
  resolveAlignPick: (
    rect: DOMRect,
    x: number,
    y: number,
  ) => AlignPick | null;
}

/**
 * 3D画素から畳み平面・展開図・合わせ入力へ写す入口をまとめる。
 * sceneとCP索引はViewerが所有するrefを借り、Zustandの状態は従来どおり
 * useAppStore.getState()からその都度読む。
 */
export function useViewerPicking({
  sceneRef,
  cpIndexRef,
  selectableEdgeSegmentsRef,
}: ViewerPickingOptions): ViewerPickingApi {
  /** 展開図の頂点・辺と面の対応。同じ展開図の間は作り直さない */
  const cpIndex = useCallback((): CpFaceIndex | null => {
    const s = useAppStore.getState();
    if (!s.doc) return null;
    const cached = cpIndexRef.current;
    if (cached && cached.doc === s.doc && cached.faces === s.faces) return cached;
    const built = buildCpFaceIndex(s.doc, s.faces);
    cpIndexRef.current = built;
    return built;
  }, [cpIndexRef]);

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
    [cpIndex, sceneRef],
  );

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
    [sceneRef],
  );

  /** canvas上の位置を畳み平面の点へ直すだけ(吸着しない)。つかむ操作に使う */
  const rawPoint = useCallback(
    (rect: DOMRect, x: number, y: number): Vec2 | null => {
      const scene = sceneRef.current;
      if (!scene) return null;
      return screenToPlane(scene.camera, rect.width, rect.height, x, y);
    },
    [sceneRef],
  );

  /**
   * 3Dのクリック画素から、展開図の頂点ID・辺ID・面内座標を1本の逆写像で受け取る。
   * 点を指す道具はすべてこの入口を通す(道具ごとに別の当て方を足さない)。
   */
  const cpPickAt = useCallback(
    (
      rect: DOMRect,
      x: number,
      y: number,
      thresholdPx?: number,
    ): CpPick3D | null => {
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
    [cpIndex, sceneRef],
  );

  /**
   * 3Dで拾った点へ、展開図と同じ頂点・交点・方眼の吸着を適用する。
   * クリックから展開図へ戻す入口は `pickCpFromPixel` のまま増やさず、返ったcpを
   * 既存の `snap` へ通す。12pxを現在面の正規化距離へ直しているため、拡大率や
   * 面の傾きが変わっても吸着の画面上の広さが大きく変わらない。
   */
  const measurePointFromPick = useCallback(
    (
      pick: CpPick3D,
      rect: DOMRect,
      x: number,
      y: number,
    ): MeasurePointPick | null => {
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
    [cpIndex, facePlacementOf, sceneRef],
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
        return hit
          ? { target: { kind: "point", p: hit }, cursor: p, cpPick: null }
          : null;
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
        target: {
          kind: "line",
          a: [hit.a.x, hit.a.y],
          b: [hit.b.x, hit.b.y],
        },
        cursor: rawPoint(rect, x, y),
        cpPick: { kind: "edge", id: hit.edgeId },
      };
    },
    [cpPickAt, rawPoint, sceneRef, selectableEdgeSegmentsRef],
  );

  return {
    cpIndex,
    facePlacementOf,
    planePoint,
    rawPoint,
    cpPickAt,
    measurePointFromPick,
    resolveAlignPick,
  };
}
