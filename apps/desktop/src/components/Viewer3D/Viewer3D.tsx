// 3Dビュー区画: Three.jsのシーンをcanvasに描き、ヒンジのクリック選択を受け付ける。
// Three.jsのオブジェクトはストアに入れずrefで保持する(要件§2: 状態はストア1本)。
//
// 更新の分担:
//   - 展開図(doc/faces/hinges)が変わったとき: 三角形分割と添字を作り直す
//   - 立体形状(frame3d)が変わったとき: 頂点座標の上書きだけ(作り直さない)

import { useCallback, useEffect, useRef } from "react";
import { useAppStore } from "../../store/appStore";
import { paperExtent } from "../CpEditor/snap";
import {
  buildTopology,
  createContent,
  createScene,
  updateFrame,
  type Viewer3DScene,
} from "./sceneBuilder";
import { pickHinge } from "./hingePicker";

/** これ以上動かしたら「クリック」ではなく視点操作とみなす(px) */
const CLICK_MOVE_PX = 4;

interface Props {
  /** 「全体表示」用: 親が current を呼ぶと紙全体が見える位置にカメラを戻す */
  fitRef: React.RefObject<(() => void) | null>;
}

export function Viewer3D({ fitRef }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sceneRef = useRef<Viewer3DScene | null>(null);
  const downPosRef = useRef<{ x: number; y: number } | null>(null);

  // 購読は更新の合図として使う(値の読み出しはgetStateで行う)
  const doc = useAppStore((s) => s.doc);
  const faces = useAppStore((s) => s.faces);
  const hinges = useAppStore((s) => s.hinges);
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

  // 展開図が変わったときだけ、三角形分割・境界線・ヒンジ対応を作り直す
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene || !doc) return;
    const content = createContent(buildTopology(doc, faces, hinges), doc.display);
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

  // 選択中のヒンジを黄色で強調する(上の効果で線分が更新された後に走る)
  useEffect(() => {
    const scene = sceneRef.current;
    if (!scene?.content) return;
    const selected = new Set(selection.edgeIds);
    scene.setHighlight(
      scene.content.hingeSegments.filter((s) => selected.has(s.edgeId)),
    );
  }, [selection, doc, faces, hinges, frame3d]);

  /** 紙全体が見える斜め上の位置へカメラを戻す */
  const fitCamera = useCallback(() => {
    const scene = sceneRef.current;
    const current = useAppStore.getState().doc;
    if (!scene || !current) return;
    const [w, h] = paperExtent(current);
    scene.resetCamera(w, h);
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
    if (!down || !scene?.content || e.button !== 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    if (Math.hypot(x - down.x, y - down.y) > CLICK_MOVE_PX) return; // 視点の回転・移動
    const edgeId = pickHinge(
      scene.content.hingeSegments,
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
