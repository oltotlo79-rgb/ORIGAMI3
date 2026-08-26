// 3Dビュー区画: Three.jsのシーンをcanvasに描き、ヒンジのクリック選択と
// 「折る」ツールの折り線描画を受け付ける。
// Three.jsのオブジェクトはストアに入れずrefで保持する(要件§2: 状態はストア1本)。
//
// 更新の分担:
//   - 展開図(doc/faces/hinges)が変わったとき: 三角形分割と添字を作り直す
//   - 立体形状(frame3d)が変わったとき: 頂点座標の上書きだけ(作り直さない)

import { useEffect, useRef, type ReactNode } from "react";
import * as THREE from "three";
import { registerViewer3DInteractionReader } from "../../captureApi";
import { useAppStore } from "../../store/appStore";
import { SELECTABLE_3D_EDGE_TARGETS } from "../../lib/viewerHint";
import { TOOL_KIND } from "../CpEditor/interaction";
import {
  type SoftContent,
  type Viewer3DScene,
} from "./sceneBuilder";
import {
  type SoftHighlightMap,
} from "./softHighlight";
import type { HingeSegment } from "./hingePicker";
import type { CpFaceIndex } from "./cpPick3d";
import { ViewerOperationHint } from "./ViewerOperationHint";
import { PaperActionTip } from "./PaperActionTip";
import { FoldDirectionTip } from "./FoldDirectionTip";
import { ViewerOverlayStack } from "./ViewerOverlayStack";
import { ViewCube } from "./ViewCube.jsx";
import { useViewerCamera } from "./viewerCamera";
import { useViewerLifecycle } from "./viewerLifecycle";
import { useViewerPicking } from "./viewerPicking";
import {
  deriveViewer3DInteractionCapture,
  useViewerHighlight,
} from "./viewerHighlight";
import {
  useViewerPointer,
  useViewerPointerPrelude,
  useViewerPointerRefs,
  useViewerPointerState,
} from "./viewerPointer";
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
  /** 展開図の頂点・辺と面の対応。展開図が変わるまで作り直さない */
  const cpIndexRef = useRef<CpFaceIndex | null>(null);
  const pointerRefs = useViewerPointerRefs();
  const pointerView = useViewerPointerState();
  const {
    foldAllActive,
    activeTool,
    measureDraft,
    alignDraft,
    foldReady,
    pullBlocked,
    foldMode,
    pullMode,
    hint,
  } = pointerView;

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
  const activeAngleIntent = useAppStore((s) => s.activeAngleIntent);
  const docEpoch = useAppStore((s) => s.docEpoch);
  const uiTheme = useAppStore((s) => s.uiTheme);
  const foldDraft = useAppStore((s) => s.foldDraft);
  const pendingFoldThrough = useAppStore((s) => s.pendingFoldThrough);
  const techniqueDraft = useAppStore((s) => s.techniqueDraft);
  const pullHinge = useAppStore((s) => s.pullHinge);
  const pullMirrorHinge = useAppStore((s) => s.pullMirrorHinge);

  useEffect(
    () =>
      registerViewer3DInteractionReader(() => {
        const state = useAppStore.getState();
        return deriveViewer3DInteractionCapture({
          grab: pointerRefs.grabRef.current,
          frame: state.frame3d,
          doc: state.doc,
          faces: state.faces,
        });
      }),
    [pointerRefs.grabRef],
  );

  useViewerLifecycle({
    canvasRef,
    sceneRef,
    orbitTargetRef,
    softRef,
    softHighlightRef,
    doc,
    faces,
    hinges,
    frame3d,
    softMesh,
    uiTheme,
  });

  const {
    cpIndex,
    facePlacementOf,
    planePoint,
    rawPoint,
    cpPickAt,
    measurePointFromPick,
    resolveAlignPick,
  } = useViewerPicking({
    sceneRef,
    cpIndexRef,
    selectableEdgeSegmentsRef,
  });

  const { drawHighlight } = useViewerHighlight({
    sceneRef,
    softHighlightRef,
    selectableEdgeSegmentsRef,
    pendingCpPointRef: pointerRefs.pendingCpPointRef,
    curvePointsRef: pointerRefs.curvePointsRef,
    constructRef: pointerRefs.constructRef,
    foldClickRef: pointerRefs.foldClickRef,
    vertexDragRef: pointerRefs.vertexDragRef,
    drawingRef: pointerRefs.drawingRef,
    grabRef: pointerRefs.grabRef,
    cpIndex,
    facePlacementOf,
    refresh: {
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
    },
  });
  const { setHoverLock } = useViewerPointerPrelude({
    sceneRef,
    refs: pointerRefs,
    view: pointerView,
  });

  const {
    fitCamera,
    getViewCubeCamera,
    prepareViewCubeCamera,
    renderViewCubeCamera,
  } = useViewerCamera({
    canvasRef,
    sceneRef,
    orbitTargetRef,
    fitRef,
    docEpoch,
  });

  const pointer = useViewerPointer({
    sceneRef,
    selectableEdgeSegmentsRef,
    refs: pointerRefs,
    picking: {
      planePoint,
      rawPoint,
      cpPickAt,
      measurePointFromPick,
      resolveAlignPick,
      facePlacementOf,
    },
    view: pointerView,
    docEpoch,
    drawHighlight,
    setHoverLock,
  });

  return (
    <>
      <canvas
        ref={canvasRef}
        className="viewer3d-canvas"
        data-testid="viewer3d-canvas"
        tabIndex={0}
        aria-label={
          foldAllActive
            ? "全部の折り目を同じ割合で動かした形。ドラッグで視点を回せます"
            : "3D表示"
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
        onPointerDown={pointer.handlers.onPointerDown}
        onPointerMove={pointer.handlers.onPointerMove}
        onPointerUp={pointer.handlers.onPointerUp}
        onPointerCancel={pointer.handlers.onPointerCancel}
        onPointerLeave={pointer.handlers.onPointerLeave}
        onContextMenu={pointer.handlers.onContextMenu}
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
