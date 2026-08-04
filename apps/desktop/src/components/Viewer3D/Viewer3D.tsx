// 3Dビュー区画: Three.jsのシーンをcanvasに描き、ヒンジのクリック選択を受け付ける。
// Three.jsのオブジェクトはストアに入れずrefで保持する(要件§2: 状態はストア1本)。

import { useCallback, useEffect, useRef } from "react";
import { useAppStore } from "../../store/appStore";
import { paperExtent } from "../CpEditor/snap";
import {
  buildContent,
  buildHighlight,
  createScene,
  flatFrame,
  type Viewer3DScene,
} from "./sceneBuilder";
import { collectHingeSegments, pickHinge, type HingeSegment } from "./hingePicker";

/** これ以上動かしたら「クリック」ではなく視点操作とみなす(px) */
const CLICK_MOVE_PX = 4;

export function Viewer3D() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sceneRef = useRef<Viewer3DScene | null>(null);
  const segmentsRef = useRef<HingeSegment[]>([]);
  const downPosRef = useRef<{ x: number; y: number } | null>(null);

  // 購読は作り直しの合図として使う(値の読み出しはgetStateで行う)
  const doc = useAppStore((s) => s.doc);
  const faces = useAppStore((s) => s.faces);
  const frame3d = useAppStore((s) => s.frame3d);
  const selection = useAppStore((s) => s.selection);
  const docEpoch = useAppStore((s) => s.docEpoch);

  // シーンの初期化と破棄
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const scene = createScene(canvas);
    sceneRef.current = scene;
    scene.resize(canvas.clientWidth, canvas.clientHeight);
    return () => {
      sceneRef.current = null;
      scene.dispose();
    };
  }, []);

  // 展開図・立体形状が変わったら面と線を作り直す
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene || !doc) return;
    const frame = frame3d ?? flatFrame(doc, faces);
    buildContent(scene, frame, doc.display);
    segmentsRef.current = collectHingeSegments(doc, faces, frame);
  }, [doc, faces, frame3d]);

  // 選択中のヒンジを黄色で強調する(上の効果で線分一覧が更新された後に走る)
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene) return;
    const selected = new Set(selection.edgeIds);
    buildHighlight(
      scene,
      segmentsRef.current.filter((s) => selected.has(s.edgeId)),
    );
  }, [selection, doc, faces, frame3d]);

  // 新規作成・ファイルを開いた直後は紙全体が見える位置へカメラを戻す
  useEffect(() => {
    const scene = sceneRef.current;
    const current = useAppStore.getState().doc;
    if (!scene || !current) return;
    const [w, h] = paperExtent(current);
    scene.resetCamera(w, h);
  }, [docEpoch]);

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

  /** クリック(視点操作でない)なら最寄りのヒンジを選ぶ */
  const handlePointerUp = useCallback((e: React.PointerEvent<HTMLCanvasElement>) => {
    const down = downPosRef.current;
    downPosRef.current = null;
    const scene = sceneRef.current;
    if (!down || !scene || e.button !== 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    if (Math.hypot(x - down.x, y - down.y) > CLICK_MOVE_PX) return; // 視点の回転・移動
    const edgeId = pickHinge(
      segmentsRef.current,
      scene.camera,
      rect.width,
      rect.height,
      x,
      y,
    );
    useAppStore.getState().setSelection({
      edgeIds: edgeId !== null ? [edgeId] : [],
      vertexIds: [],
    });
  }, []);

  return (
    <canvas
      ref={canvasRef}
      className="viewer3d-canvas"
      title="ドラッグで回転、ホイールで拡大縮小、折り線をクリックで選択"
      onPointerDown={(e) => {
        const rect = e.currentTarget.getBoundingClientRect();
        downPosRef.current = { x: e.clientX - rect.left, y: e.clientY - rect.top };
      }}
      onPointerUp={handlePointerUp}
      onPointerCancel={() => {
        downPosRef.current = null;
      }}
      onContextMenu={(e) => e.preventDefault()}
    />
  );
}
