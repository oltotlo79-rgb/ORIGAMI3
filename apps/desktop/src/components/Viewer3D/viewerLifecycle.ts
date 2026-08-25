// Viewer scene lifetime boundary (C5).
// WebGL resources are created by sceneBuilder's sceneFacade-compatible entry
// and become scene-owned after setContent/setSoft. This hook only coordinates
// the existing React lifetime and never disposes transferred resources itself.

import { useEffect, type RefObject } from "react";
import type { UiTheme } from "../../lib/displayPrefs";
import type {
  Document,
  Face,
  Frame3D,
  SoftMesh,
} from "../../lib/types";
import { useAppStore } from "../../store/appStore";
import {
  buildTopology,
  createContent,
  createScene,
  createSoftContent,
  updateFrame,
  updateSoftContent,
  type SoftContent,
  type Viewer3DScene,
} from "./sceneBuilder";
import { softSignature } from "./softMesh";
import {
  buildSoftHighlightMap,
  type SoftHighlightMap,
} from "./softHighlight";
import { trackedOrbitTarget } from "./viewCube";
import * as THREE from "three";

export interface ViewerLifecycleState {
  readonly doc: Document | null;
  readonly faces: Face[];
  readonly hinges: ReadonlySet<number>;
  readonly frame3d: Frame3D | null;
  readonly softMesh: SoftMesh | null;
  readonly uiTheme: UiTheme;
}

export interface UseViewerLifecycleOptions extends ViewerLifecycleState {
  readonly canvasRef: RefObject<HTMLCanvasElement | null>;
  readonly sceneRef: RefObject<Viewer3DScene | null>;
  readonly orbitTargetRef: RefObject<THREE.Vector3>;
  readonly softRef: RefObject<SoftContent | null>;
  readonly softHighlightRef: RefObject<SoftHighlightMap | null>;
}

export function useViewerLifecycle({
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
}: UseViewerLifecycleOptions): void {
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
  }, [canvasRef, orbitTargetRef, sceneRef, softHighlightRef, softRef]);

  // テーマ変更後のCSS変数を読み直して、WebGL背景も直ちに描き替える。
  useEffect(() => {
    sceneRef.current?.syncTheme();
  }, [sceneRef, uiTheme]);

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
  }, [doc, faces, hinges, sceneRef, softHighlightRef, softRef]);

  // 立体形状が変わったら頂点座標を上書きするだけ(手順再生でもここだけ動く)
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene?.content) return;
    updateFrame(scene.content, frame3d);
    scene.render();
  }, [frame3d, sceneRef]);

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
      softHighlightRef.current = buildSoftHighlightMap(
        s.doc,
        s.faces,
        softMesh,
        content,
      );
    }
    scene.render();
  }, [
    doc,
    faces,
    hinges,
    sceneRef,
    softHighlightRef,
    softMesh,
    softRef,
  ]);
}
