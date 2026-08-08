// 3Dビュー区画: Three.jsのシーンをcanvasに描き、ヒンジのクリック選択と
// 「折る」ツールの折り線描画を受け付ける。
// Three.jsのオブジェクトはストアに入れずrefで保持する(要件§2: 状態はストア1本)。
//
// 更新の分担:
//   - 展開図(doc/faces/hinges)が変わったとき: 三角形分割と添字を作り直す
//   - 立体形状(frame3d)が変わったとき: 頂点座標の上書きだけ(作り直さない)

import { useCallback, useEffect, useRef } from "react";
import * as THREE from "three";
import { canFoldNow, pullBlockReason, useAppStore } from "../../store/appStore";
import { viewerHint } from "../../lib/viewerHint";
import {
  hingeAnglesFromFrame,
  planPull,
  pullDeltaDeg,
  type PullPlan,
} from "../../lib/grabDrive";
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
  createSoftContent,
  updateFrame,
  updateSoftContent,
  type SoftContent,
  type Viewer3DScene,
} from "./sceneBuilder";
import { softSignature } from "./softMesh";
import { twistPreviewSegments } from "../../lib/twistPolygon";
import { ALIGN_STEPS } from "../../lib/alignFold";
import { nearestAlignLine, nearestAlignPoint } from "../../lib/alignPick";
import { planGrabFold, type GrabMode } from "./grabFold";
import { pickFace, pickHinge, pickPaper, type HingeSegment } from "./hingePicker";
import { deriveSelectedEdgeHighlights } from "./edgeHighlight";
import { ViewerOperationHint } from "./ViewerOperationHint";
import { PaperActionTip } from "./PaperActionTip";

/** 畳み平面の線分列を強調表示用の線分へ(紙より少しだけ浮かせる) */
function toHighlight(segments: [Vec2, Vec2][]): HingeSegment[] {
  return segments.map(([a, b]) => ({
    edgeId: -1,
    a: new THREE.Vector3(a[0], a[1], PREVIEW_LIFT),
    b: new THREE.Vector3(b[0], b[1], PREVIEW_LIFT),
  }));
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

interface Props {
  /** 「全体表示」用: 親が current を呼ぶと紙全体が見える位置にカメラを戻す */
  fitRef: React.RefObject<(() => void) | null>;
}

/** 引く操作ができない理由(できるならnull)。ストアの状態から組み立てる */
function pullBlockedOf(s: ReturnType<typeof useAppStore.getState>): string | null {
  return pullBlockReason({
    doc: s.doc,
    playing: s.playing,
    playT: s.playT,
    hingeCount: s.hinges.size,
    currentStep: s.currentStep,
    stepCount: s.doc?.sequence.length ?? 0,
  });
}

export function Viewer3D({ fitRef }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sceneRef = useRef<Viewer3DScene | null>(null);
  /** 表示中のたわみの網(Three.jsの資源なのでストアには入れずrefで持つ) */
  const softRef = useRef<SoftContent | null>(null);
  const downPosRef = useRef<{ x: number; y: number } | null>(null);
  /** 折り線を引いている最中の2点(表示専用の一時状態なのでrefで持つ) */
  const drawingRef = useRef<{ a: Vec2; b: Vec2 } | null>(null);
  /** 紙をつかんで動かしている最中のつかんだ点・今の点・つかんだ面・対象の枚数 */
  const grabRef = useRef<{
    a: Vec2;
    b: Vec2;
    face: number | null;
    mode: GrabMode;
  } | null>(null);
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
  const docEpoch = useAppStore((s) => s.docEpoch);
  const activeTool = useAppStore((s) => s.activeTool);
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
      hasTechniqueLine: s.techniqueDraft?.line != null,
      techniqueKind: s.techniqueDraft?.kind ?? null,
      techniqueVertexCount: s.techniqueDraft?.polygon.length ?? 0,
      techniqueHasCenter: s.techniqueDraft?.center != null,
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
      scene.setSoft(null);
      return;
    }
    let content = softRef.current;
    if (!content || content.signature !== softSignature(softMesh)) {
      content = createSoftContent(softMesh, s.doc.display);
      softRef.current = content;
      scene.setSoft(content);
    }
    updateSoftContent(content, softMesh, s.frame3d);
    scene.render();
  }, [softMesh, doc, faces, hinges]);

  /** 強調表示を描き直す: 折り線を引いている間はその線と動く層、それ以外は選択中の折り線 */
  const drawHighlight = useCallback(() => {
    const scene = sceneRef.current;
    if (!scene?.content) return;
    const s = useAppStore.getState();
    const drawing = drawingRef.current;
    // つかんで動かしている間は「折った結果の形」を半透明で重ねて見せる(UI-008)
    const grab = grabRef.current;
    if (grab && s.doc) {
      const plan = planGrabFold(
        foldLayers(s.frame3d, s.doc, s.faces),
        s.faces,
        grab.a,
        grab.b,
        grab.mode,
        grab.face,
      );
      scene.setPreview(plan.ok ? plan.plan.preview : [], PREVIEW_FILL_LIFT);
      scene.setHighlight(plan.ok ? toHighlight(plan.plan.segments) : []);
      return;
    }
    scene.setPreview([], PREVIEW_FILL_LIFT);
    // 巻き込み用の追加折り目。Rustが現在の畳み平面へ写した線を、既存の
    // 参照線ハイライト(水色)で示す。展開図側は別のCP座標を使う。
    if (s.pendingFoldThrough) {
      scene.setHighlight(
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
      scene.setHighlight(toHighlight(segments));
      return;
    }
    // 技法では、選んだフラップ(重なった層)の輪郭も光らせる
    if (s.activeTool === "technique" && s.techniqueDraft && s.doc) {
      const draft = s.techniqueDraft;
      const layers = foldLayers(s.frame3d, s.doc, s.faces);
      const segments: [Vec2, Vec2][] = [];
      // ねじり折り: 指した中央多角形と、そこから出るひだの折り線を下見する
      if (draft.kind === "Twist" && draft.polygon.length > 0) {
        segments.push(...twistPreviewSegments(draft.polygon, draft.center));
        if (draft.center) segments.push(...centerMark(draft.center));
      }
      const shown = drawing ? [drawing.a, drawing.b] : draft.line;
      if (shown) segments.push([shown[0], shown[1]]);
      for (const l of layers.filter((l) => draft.flap.includes(l.face))) {
        for (let i = 0; i < l.polygon.length; i++) {
          segments.push([l.polygon[i], l.polygon[(i + 1) % l.polygon.length]]);
        }
      }
      scene.setHighlight(toHighlight(segments));
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
      scene.setHighlight(toHighlight(segments));
      return;
    }
    // 引いている間は、いま角度を変えている折り線だけを色で示す(UI-007)。
    // 左右同時のときは対称の相手にも同じ色を付け、両方動くことを見せる
    if (s.pullHinge !== null) {
      const selected = new Set(
        s.pullMirrorHinge !== null
          ? [s.pullHinge, s.pullMirrorHinge]
          : [s.pullHinge],
      );
      scene.setHighlight(
        scene.content.hingeSegments.filter((seg) => selected.has(seg.edgeId)),
      );
      return;
    }
    // 2Dで選んだ辺は種類を問わず現在の3D位置へ写す。ヒンジは黄色、
    // 折る操作の対象にならない縁・補助線・非ヒンジ折り線は水色で区別する。
    if (!s.doc) {
      scene.setHighlight([]);
      return;
    }
    scene.setHighlight(
      deriveSelectedEdgeHighlights(
        s.doc,
        s.faces,
        scene.content.topology.slots,
        scene.content.positions,
        s.hinges,
        s.selection.edgeIds,
      ).map((target) => ({
        edgeId: target.edgeId,
        role: target.role,
        a: new THREE.Vector3(...target.a),
        b: new THREE.Vector3(...target.b),
      })),
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
    pendingFoldThrough,
    alignDraft,
    techniqueDraft,
    activeTool,
    pullHinge,
    pullMirrorHinge,
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
      if (s.activeTool === "pull") {
        if (pullBlockedOf(s) !== null) {
          canvas.style.cursor = "not-allowed";
          return;
        }
        const hit = pickPaper(
          scene.content.mesh,
          scene.content.topology.triangleFaceIds,
          scene.camera,
          rect.width,
          rect.height,
          x,
          y,
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
          const p = screenToPlane(scene.camera, rect.width, rect.height, x, y);
          if (!p) {
            canvas.style.cursor = "default";
            return;
          }
          const steps = ALIGN_STEPS[s.alignDraft.mode];
          const at = s.alignDraft.picks.length % steps.length;
          const radius = planeRadius(
            scene.camera,
            rect.width,
            rect.height,
            x,
            y,
            ALIGN_PICK_PX,
            FOLD_SNAP_FALLBACK,
          );
          const layers = foldLayers(s.frame3d, s.doc, s.faces);
          const hit =
            steps[at] === "point"
              ? nearestAlignPoint(layers, p, radius)
              : nearestAlignLine(layers, p, radius);
          canvas.style.cursor = hit ? "pointer" : "default";
          return;
        }
        if (ctrlKey) {
          canvas.style.cursor = "crosshair";
          return;
        }
        const face = pickFace(
          scene.content.mesh,
          scene.content.topology.triangleFaceIds,
          scene.camera,
          rect.width,
          rect.height,
          x,
          y,
        );
        canvas.style.cursor = face === null ? "default" : "grab";
        return;
      }
      if (s.activeTool === "technique") {
        canvas.style.cursor = canFoldNow(s) ? "crosshair" : "not-allowed";
        return;
      }
      const edgeId = pickHinge(
        scene.content.hingeSegments,
        scene.camera,
        rect.width,
        rect.height,
        x,
        y,
      );
      if (edgeId !== null) {
        canvas.style.cursor = "pointer";
        return;
      }
      const paper = pickPaper(
        scene.content.mesh,
        scene.content.topology.triangleFaceIds,
        scene.camera,
        rect.width,
        rect.height,
        x,
        y,
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
        const hit = pickPaper(
          scene0.content.mesh,
          scene0.content.topology.triangleFaceIds,
          scene0.camera,
          rect.width,
          rect.height,
          x,
          y,
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
        const face = pickFace(
          scene.content.mesh,
          scene.content.topology.triangleFaceIds,
          scene.camera,
          rect.width,
          rect.height,
          x,
          y,
        );
        const p = face === null ? null : rawPoint(rect, x, y);
        if (p && face !== null) {
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.style.cursor = "grabbing";
          s.setOperationStage(1);
          grabRef.current = {
            a: p,
            b: p,
            face,
            mode: grabMode(e),
          };
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
        const p = rawPoint(rect, e.clientX - rect.left, e.clientY - rect.top);
        if (!p) return;
        grab.b = p;
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
          .foldByDrag(grab.a, grab.b, grabMode(e), grab.face);
        return;
      }
      const drawing = drawingRef.current;
      if (drawing) {
        drawingRef.current = null;
        const [a, b] = [drawing.a, drawing.b];
        const s = useAppStore.getState();
        const drawn = Math.hypot(b[0] - a[0], b[1] - a[1]) >= MIN_FOLD_LENGTH;
        if (s.activeTool === "technique" && s.techniqueDraft && s.doc) {
          if (s.techniqueDraft.kind === "Twist" && !drawn) {
            // ねじり折り: クリックで中央多角形の角を順に置く(Ctrlなら中心)
            if (e.ctrlKey) s.setTechniqueCenter(a);
            else s.addTechniqueVertex(a);
          } else if (drawn) {
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
        const p = rawPoint(rect, x, y);
        if (!p) return;
        const radius = planeRadius(
          scene.camera,
          rect.width,
          rect.height,
          x,
          y,
          ALIGN_PICK_PX,
          FOLD_SNAP_FALLBACK,
        );
        const layers = foldLayers(st.frame3d, st.doc, st.faces);
        if (steps[at] === "point") {
          const hit = nearestAlignPoint(layers, p, radius);
          if (hit) st.pickAlignTarget({ kind: "point", p: hit }, p);
        } else {
          const hit = nearestAlignLine(layers, p, radius);
          if (hit) st.pickAlignTarget({ kind: "line", a: hit[0], b: hit[1] }, p);
        }
        return;
      }
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
      if (st.activeTool === "select" && edgeId === null) {
        const paper = pickPaper(
          scene.content.mesh,
          scene.content.topology.triangleFaceIds,
          scene.camera,
          rect.width,
          rect.height,
          x,
          y,
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
        title={
          pullMode
            ? "紙をつかんでドラッグすると、折り線のつじつまを保ったまま全体が連動して動く(右ドラッグで視点を回す)"
            : activeTool === "technique"
            ? techniqueDraft?.kind === "Twist"
              ? "中央の形の角を順にクリックする(3つ以上)。Ctrl+クリックで中心、Backspaceで1つ戻す、Escでやめる"
              : "紙をクリックして層を選び、ドラッグして折り線を引く(平らに畳んだ状態で使える)"
            : foldMode
              ? "紙をつかんでドラッグすると折れる。Shiftで重なった紙を全部、Altで1枚だけ、Ctrl+ドラッグで折り線を引く(平らに畳んだ状態で使える)"
              : "ドラッグで回転、ホイールで拡大縮小、折り線をクリックで選択(展開図で選んだ縁・補助線は水色)"
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
      {/* 立体だけを最初の視点へ戻す小さなボタン(ツールレールは増やさない)。
          上端は警告バッジが使うので、区画の右下の隅に置く */}
      <button
        type="button"
        className="viewer-reset"
        title="紙全体が見える最初の視点に戻します(展開図の表示はそのまま)"
        onClick={fitCamera}
      >
        視点を戻す
      </button>
    </>
  );
}
