// Viewer camera/framing boundary (C6).
// Scene/WebGL resources remain owned by sceneFacade; this hook only coordinates
// the existing camera callbacks and their React lifetime.

import { useCallback, useEffect, type RefObject } from "react";
import * as THREE from "three";
import { paperExtent } from "../CpEditor/snap";
import { useAppStore } from "../../store/appStore";
import {
  contentBoundingBox,
  type Viewer3DScene,
} from "./sceneBuilder";
import type { ViewCubeCameraControl } from "./ViewCube.jsx";
import { trackedOrbitTarget } from "./viewCube";

export interface ViewerCameraRefs {
  canvasRef: RefObject<HTMLCanvasElement | null>;
  sceneRef: RefObject<Viewer3DScene | null>;
  orbitTargetRef: RefObject<THREE.Vector3>;
}

export interface UseViewerCameraOptions extends ViewerCameraRefs {
  fitRef: RefObject<(() => void) | null>;
  docEpoch: number;
}

export function useViewerCamera({
  canvasRef,
  sceneRef,
  orbitTargetRef,
  fitRef,
  docEpoch,
}: UseViewerCameraOptions) {
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
  }, [canvasRef]);

  /**
   * 立体全体が見える斜め上の位置へカメラを戻す。
   * 基準は展開図の大きさではなく、いま表示している立体の頂点範囲にする。
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
  }, [hintBottomPx, orbitTargetRef, sceneRef]);

  // 新規作成・ファイルを開いた直後は紙全体が見える位置へカメラを戻す。
  useEffect(() => {
    fitCamera();
  }, [docEpoch, fitCamera]);

  // 「全体表示」を親(ツールレール)から呼べるように登録する。
  useEffect(() => {
    fitRef.current = fitCamera;
    return () => {
      fitRef.current = null;
    };
  }, [fitRef, fitCamera]);

  const getViewCubeCamera = useCallback(
    () => sceneRef.current?.camera ?? null,
    [sceneRef],
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
  }, [canvasRef, orbitTargetRef, sceneRef]);

  const renderViewCubeCamera = useCallback(() => {
    sceneRef.current?.render();
  }, [sceneRef]);

  // 区画サイズの変化に追従する。
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
  }, [canvasRef, hintBottomPx, sceneRef]);

  return {
    fitCamera,
    getViewCubeCamera,
    prepareViewCubeCamera,
    renderViewCubeCamera,
  };
}
