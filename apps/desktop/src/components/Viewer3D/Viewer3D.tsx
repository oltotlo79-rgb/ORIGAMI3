// 3Dビュー区画: Three.jsのシーンをcanvasに描き、ヒンジのクリック選択と
// 「折る」ツールの折り線描画を受け付ける。
// Three.jsのオブジェクトはストアに入れずrefで保持する(要件§2: 状態はストア1本)。
//
// 更新の分担:
//   - 展開図(doc/faces/hinges)が変わったとき: 三角形分割と添字を作り直す
//   - 立体形状(frame3d)が変わったとき: 頂点座標の上書きだけ(作り直さない)

import { useCallback, useEffect, useRef } from "react";
import * as THREE from "three";
import { canFoldNow, useAppStore } from "../../store/appStore";
import { paperExtent } from "../CpEditor/snap";
import { planeRadius, screenToPlane } from "../../lib/planeProject";
import type { Vec2 } from "../../lib/types";
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
  updateFrame,
  type Viewer3DScene,
} from "./sceneBuilder";
import { pickHinge } from "./hingePicker";

/** これ以上動かしたら「クリック」ではなく視点操作とみなす(px) */
const CLICK_MOVE_PX = 4;
/** 折り線の端点を紙の点・輪郭へ吸着させる距離(px) */
const FOLD_SNAP_PX = 14;
/** 平面へ投影できないときの吸着半径(正規化座標) */
const FOLD_SNAP_FALLBACK = 0.02;
/** これ未満の長さの折り線は引かなかったことにする(正規化座標) */
const MIN_FOLD_LENGTH = 1e-4;
/** 技法のフラップ選択で、層の輪郭からこの距離以内なら「その場所にある」とみなす
 * (クリック位置は紙の点・輪郭へ吸着するので、境界ちょうどを指しても拾えるようにする) */
const FLAP_PICK_EPS = 1e-3;
/** プレビュー線を紙より少しだけ上に浮かせる高さ(重なりのちらつき防止) */
const PREVIEW_LIFT = 0.002;

interface Props {
  /** 「全体表示」用: 親が current を呼ぶと紙全体が見える位置にカメラを戻す */
  fitRef: React.RefObject<(() => void) | null>;
}

export function Viewer3D({ fitRef }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sceneRef = useRef<Viewer3DScene | null>(null);
  const downPosRef = useRef<{ x: number; y: number } | null>(null);
  /** 折り線を引いている最中の2点(表示専用の一時状態なのでrefで持つ) */
  const drawingRef = useRef<{ a: Vec2; b: Vec2 } | null>(null);

  // 購読は更新の合図として使う(値の読み出しはgetStateで行う)
  const doc = useAppStore((s) => s.doc);
  const faces = useAppStore((s) => s.faces);
  const hinges = useAppStore((s) => s.hinges);
  const frame3d = useAppStore((s) => s.frame3d);
  const selection = useAppStore((s) => s.selection);
  const docEpoch = useAppStore((s) => s.docEpoch);
  const activeTool = useAppStore((s) => s.activeTool);
  const foldDraft = useAppStore((s) => s.foldDraft);
  const techniqueDraft = useAppStore((s) => s.techniqueDraft);
  const foldReady = useAppStore(canFoldNow);
  // 「折る」と「技法」はどちらも紙の上に折り線を引く(左ドラッグを線引きに使う)
  const foldMode = activeTool === "fold" || activeTool === "technique";

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

  /** 強調表示を描き直す: 折り線を引いている間はその線と動く層、それ以外は選択中の折り線 */
  const drawHighlight = useCallback(() => {
    const scene = sceneRef.current;
    if (!scene?.content) return;
    const s = useAppStore.getState();
    const drawing = drawingRef.current;
    // 技法では、選んだフラップ(重なった層)の輪郭も光らせる
    if (s.activeTool === "technique" && s.techniqueDraft && s.doc) {
      const draft = s.techniqueDraft;
      const layers = foldLayers(s.frame3d, s.doc, s.faces);
      const segments: [Vec2, Vec2][] = [];
      const shown = drawing ? [drawing.a, drawing.b] : draft.line;
      if (shown) segments.push([shown[0], shown[1]]);
      for (const l of layers.filter((l) => draft.flap.includes(l.face))) {
        for (let i = 0; i < l.polygon.length; i++) {
          segments.push([l.polygon[i], l.polygon[(i + 1) % l.polygon.length]]);
        }
      }
      scene.setHighlight(
        segments.map(([a, b]) => ({
          edgeId: -1,
          a: new THREE.Vector3(a[0], a[1], PREVIEW_LIFT),
          b: new THREE.Vector3(b[0], b[1], PREVIEW_LIFT),
        })),
      );
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
      scene.setHighlight(
        segments.map(([a, b]) => ({
          edgeId: -1,
          a: new THREE.Vector3(a[0], a[1], PREVIEW_LIFT),
          b: new THREE.Vector3(b[0], b[1], PREVIEW_LIFT),
        })),
      );
      return;
    }
    const selected = new Set(s.selection.edgeIds);
    scene.setHighlight(
      scene.content.hingeSegments.filter((seg) => selected.has(seg.edgeId)),
    );
  }, []);

  // 選択・折り線プレビューの強調(上の効果で線分が更新された後に走る)
  useEffect(() => {
    drawHighlight();
  }, [
    selection,
    doc,
    faces,
    hinges,
    frame3d,
    foldDraft,
    techniqueDraft,
    activeTool,
    drawHighlight,
  ]);

  // 折るツールの間は左ドラッグを線引きに使うので、視点の回転を止める
  useEffect(() => {
    sceneRef.current?.setDrawMode(foldMode && foldReady);
  }, [foldMode, foldReady]);

  // 折るツールから離れたら引きかけの線を捨てる
  useEffect(() => {
    if (!foldMode) drawingRef.current = null;
  }, [foldMode]);

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

  const handlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      const rect = e.currentTarget.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const s = useAppStore.getState();
      const drawTool = s.activeTool === "fold" || s.activeTool === "technique";
      if (e.button === 0 && drawTool && canFoldNow(s)) {
        const p = planePoint(rect, x, y);
        if (p) {
          e.currentTarget.setPointerCapture(e.pointerId);
          drawingRef.current = { a: p, b: p };
          drawHighlight();
        }
        return;
      }
      downPosRef.current = { x, y };
    },
    [planePoint, drawHighlight],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      const drawing = drawingRef.current;
      if (!drawing) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const p = planePoint(rect, e.clientX - rect.left, e.clientY - rect.top);
      if (!p) return;
      drawing.b = p;
      drawHighlight();
    },
    [planePoint, drawHighlight],
  );

  /** クリック(視点操作でない)なら最寄りのヒンジを選ぶ。折り線を引いていたら確定する */
  const handlePointerUp = useCallback(
    (e: React.PointerEvent<HTMLCanvasElement>) => {
      const drawing = drawingRef.current;
      if (drawing) {
        drawingRef.current = null;
        const [a, b] = [drawing.a, drawing.b];
        const s = useAppStore.getState();
        const drawn = Math.hypot(b[0] - a[0], b[1] - a[1]) >= MIN_FOLD_LENGTH;
        if (s.activeTool === "technique" && s.techniqueDraft && s.doc) {
          if (drawn) {
            s.setTechniqueLine([a, b]);
          } else {
            // ドラッグせずクリックしただけ: その場所に重なっている層を選ぶ
            s.setTechniqueFlap(
              facesAtPoint(
                foldLayers(s.frame3d, s.doc, s.faces),
                a,
                FLAP_PICK_EPS,
              ),
            );
          }
        } else if (drawn) {
          s.beginFoldDraft([a, b], "3d");
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
    },
    [drawHighlight],
  );

  return (
    <>
      <canvas
        ref={canvasRef}
        className="viewer3d-canvas"
        style={{ cursor: foldMode && foldReady ? "crosshair" : "default" }}
        title={
          activeTool === "technique"
            ? "紙をクリックして層を選び、ドラッグして折り線を引く(平らに畳んだ状態で使える)"
            : foldMode
              ? "紙の上をドラッグして折り線を引く(平らに畳んだ状態で使える)"
              : "ドラッグで回転、ホイールで拡大縮小、折り線をクリックで選択"
        }
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={() => {
          downPosRef.current = null;
          drawingRef.current = null;
          drawHighlight();
        }}
        onContextMenu={(e) => e.preventDefault()}
      />
      {foldMode && !foldReady && (
        <div className="viewer-notice">平らに畳んだ状態で使えます</div>
      )}
    </>
  );
}
