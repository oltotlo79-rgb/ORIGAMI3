// 2D展開図エディタ: canvas要素とイベント接続、ストア購読で再描画。
// ズーム/パン・描画途中などの一時状態はコンポーネント内に保持する(表示専用)。

import { useCallback, useEffect, useRef } from "react";
import type { Vec2 } from "../../lib/types";
import { useAppStore, type AlignDraft, type Selection } from "../../store/appStore";
import { clipToPaper, CONSTRUCT_STEPS, constructHint } from "../../lib/construct";
import {
  curveHint,
  firstCrossing,
  rulingLines,
  type CurveOptions,
} from "../../lib/curve";
import { violationReason } from "../../lib/flatFoldHint";
import { documentForCpStep, violationsForCpStep } from "../../lib/cpHistory";
import {
  mirrorAxisLabel,
  mirrorLineForChoice,
  mirrorLineInsidePaper,
  mirrorPoint,
  paperMirrorLine,
} from "../../lib/mirror";
import { isEditableTarget } from "../../lib/keyboard";
import type { Document, EdgeKind } from "../../lib/types";
import {
  constructDone,
  cursorFor,
  curveDraft,
  initialEphemeralState,
  isSpaceKey,
  onKeyDown,
  onKeyUp,
  onMouseDown,
  onMouseMove,
  onMouseUp,
  onWheel,
  panHint,
  previewKind,
  type InteractionCtx,
} from "./interaction";
import { fitView, render, type RenderOverlay, type ViewTransform } from "./renderer";
import { paperExtent } from "./snap";
import { CpOperationHint } from "./CpOperationHint";

interface Props {
  /** 「全体表示」用: 親が current を呼ぶと紙全体が収まる表示に戻す */
  fitRef: React.RefObject<(() => void) | null>;
}

/** ホバー中の「平らに畳めない点」に添える説明(カーソルの近くに出す) */
function violationTooltip(doc: Document, vertexId: number | null) {
  if (vertexId === null) return null;
  const v = doc.cp.vertices.find((x) => x.id === vertexId);
  return v ? { pos: v.pos, text: violationReason(doc, vertexId) } : null;
}

/** 選んだ点・線を、既存の展開図の選択強調へ対応付ける。 */
function alignSelection(draft: AlignDraft): Selection {
  const vertexIds = new Set<number>();
  const edgeIds = new Set<number>();
  for (const pick of draft.cpPicks ?? []) {
    if (pick?.kind === "vertex") {
      vertexIds.add(pick.id);
    } else if (pick?.kind === "edge") {
      edgeIds.add(pick.id);
    }
  }
  return { edgeIds: [...edgeIds], vertexIds: [...vertexIds] };
}

/**
 * 描いている最中の曲線と、確定したときに一緒に入る「紙が曲がるための線」を
 * まとめて返す(確定前に何が入るかを見せるため。設計原則3b)。
 * 曲がるための線は、折り目の両側で曲がる向きが逆になるので線種を分ける。
 */
function curvePreviewPaths(
  doc: Document,
  points: Vec2[],
  kind: EdgeKind,
  curve: CurveOptions,
): { points: Vec2[]; kind: EdgeKind }[] {
  const paths = [{ points, kind }];
  if (!curve.rulings || kind === "Aux") return paths;
  const long = Math.max(doc.paper.width_mm, doc.paper.height_mm);
  const paper: Vec2 = [doc.paper.width_mm / long, doc.paper.height_mm / long];
  const opposite: EdgeKind = kind === "Mountain" ? "Valley" : "Mountain";
  for (const r of rulingLines(points, paper)) {
    paths.push({ points: [r.at, firstCrossing(doc, r.at, r.concave)], kind: opposite });
    paths.push({ points: [r.at, firstCrossing(doc, r.at, r.convex)], kind });
  }
  return paths;
}

export function CpEditor({ fitRef }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const viewRef = useRef<ViewTransform | null>(null);
  const stateRef = useRef(initialEphemeralState());

  // 購読はdrawの再実行トリガーとして使う(値の読み出しはgetStateで行う)
  const doc = useAppStore((s) => s.doc);
  const currentStep = useAppStore((s) => s.currentStep);
  const selection = useAppStore((s) => s.selection);
  const hoveredHinge = useAppStore((s) => s.hoveredHinge);
  const suspectHinges = useAppStore((s) => s.suspectHinges);
  const pinnedFolds = useAppStore((s) => s.pinnedFolds);
  const releasedPins = useAppStore((s) => s.releasedPins);
  const activeAngleIntent = useAppStore((s) => s.activeAngleIntent);
  const activeTool = useAppStore((s) => s.activeTool);
  const measureDraft = useAppStore((s) => s.measureDraft);
  const docEpoch = useAppStore((s) => s.docEpoch);
  const violations = useAppStore((s) => s.violations);
  const stepCreases = useAppStore((s) => s.stepCreases);
  const construct = useAppStore((s) => s.construct);
  const curve = useAppStore((s) => s.curve);
  const mirrorDraw = useAppStore((s) => s.mirrorDraw);
  const mirrorAxisChoice = useAppStore((s) => s.mirrorAxis);
  const wheelBehavior = useAppStore((s) => s.wheelBehavior);
  const uiTheme = useAppStore((s) => s.uiTheme);
  const pendingFoldThrough = useAppStore((s) => s.pendingFoldThrough);
  const alignDraft = useAppStore((s) => s.alignDraft);
  const foldDraft = useAppStore((s) => s.foldDraft);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const {
      doc,
      currentStep,
      stepCreases,
      selection,
      activeTool,
      measureDraft,
      violations,
      construct,
      curve,
      mirrorDraw,
      mirrorAxis,
      hoveredHinge,
      suspectHinges,
      pinnedFolds,
      releasedPins,
      activeAngleIntent,
      alignDraft,
      foldDraft,
    } = useAppStore.getState();
    if (!canvas) return;
    // カーソルの形は表示専用なので、再描画を起こさずcanvasへ直接反映する
    canvas.style.cursor = cursorFor(activeTool, stateRef.current);
    if (!doc) return;
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    if (w === 0 || h === 0) return;
    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
    }
    viewRef.current ??= fitView(doc, w, h);
    const cpDocument = documentForCpStep(doc, currentStep, stepCreases);
    // その手順の展開図に無い点の丸は出さない(将来の点の違反を過去へ出さない)
    const cpViolations = violationsForCpStep(cpDocument, violations);
    const st = stateRef.current;
    const captureClean = document.documentElement.hasAttribute(
      "data-origami3-capture-view",
    );
    const kind = previewKind(activeTool);
    // 左右対称のときは対称軸を薄く出し、引いている最中の線も反対側に見せる
    const axis = mirrorDraw
      ? mirrorLineForChoice(doc, mirrorAxis) ??
        paperMirrorLine(doc.paper, "paperVertical")
      : null;
    const axisSegment = axis ? mirrorLineInsidePaper(doc.paper, axis) : null;
    const curveMode = kind !== undefined && curve.enabled && activeTool !== "fold";
    const directionSnap =
      kind && !curveMode && st.pendingStart ? st.directionSnap : null;
    // 未折りなら展開図と畳み平面が同じなので、求まった折り線も2Dで下見できる。
    // 手順後は座標系が異なるため、既存の3D下見だけに出す。
    const alignPreview =
      activeTool === "fold" && alignDraft && foldDraft && doc.sequence.length === 0
        ? {
            a: foldDraft.line[0],
            b: foldDraft.line[1],
            kind: foldDraft.direction === "Up" ? ("Valley" as const) : ("Mountain" as const),
          }
        : null;
    const preview =
      alignPreview ??
      (kind && !curveMode && st.pendingStart && st.cursorWorld
        ? {
            a: st.pendingStart,
            b: st.hoverSnap?.pos ?? directionSnap?.pos ?? st.cursorWorld,
            kind,
          }
        : null);
    const [paperWidth, paperHeight] = paperExtent(doc);
    const guideReach = 2 * Math.max(paperWidth, paperHeight);
    const directionGuide =
      directionSnap && st.pendingStart
        ? clipToPaper(
            [
              [
                st.pendingStart[0] - directionSnap.direction[0] * guideReach,
                st.pendingStart[1] - directionSnap.direction[1] * guideReach,
              ],
              [
                st.pendingStart[0] + directionSnap.direction[0] * guideReach,
                st.pendingStart[1] + directionSnap.direction[1] * guideReach,
              ],
            ],
            paperWidth,
            paperHeight,
          )
        : null;
    // 曲線モードでは、確定したときに入るのと同じ折れ線をそのまま見せる(設計原則3b)
    const draft = curveMode ? curveDraft(st, curve) : null;
    const previewPaths = draft && kind ? curvePreviewPaths(doc, draft, kind, curve) : [];
    const directionHint = directionSnap
      ? directionSnap.kind === "bisector"
        ? "二等分方向に吸着中(Shiftで解除)"
        : "辺・折り目の延長方向に吸着中(Shiftで解除)"
      : null;
    const toolHint =
      directionHint ??
      (activeTool === "construct"
        ? constructHint(construct.kind, constructDone(st), construct.divisions)
        : curveMode
          ? curveHint(curve.shape, st.curvePoints.length, curve.rulings)
          : mirrorDraw
            ? `対称にそろえています（基準: ${mirrorAxisLabel(mirrorAxis)}。引く・消す・線種変更）`
            : null);
    const overlay: RenderOverlay = {
      // 作図も通常線と同じhoverSnapを渡し、renderer共通の緑丸で吸着を知らせる。
      hoverSnap:
        kind !== undefined ||
        activeTool === "construct" ||
        (activeTool === "measure" && measureDraft.mode === "distance")
          ? st.hoverSnap
          : null,
      preview,
      directionGuide,
      mirrorAxis: axisSegment,
      mirrorPreview:
        axis !== null && preview && activeTool !== "fold"
          ? {
              a: mirrorPoint(preview.a, axis),
              b: mirrorPoint(preview.b, axis),
              kind: preview.kind,
            }
          : null,
      previewPaths,
      marquee:
        st.marqueeStart && st.marqueeEnd ? { a: st.marqueeStart, b: st.marqueeEnd } : null,
      violations: cpViolations,
      constructPoints:
        activeTool === "measure" && measureDraft.mode === "distance"
          ? measureDraft.picks.flatMap((pick) =>
              // 頂点は既存の選択印で描かれる。IDを持たない方眼・任意点だけを
              // 作図用の点表示へ渡し、同じ位置へ印を二重描画しない。
              pick.kind === "point" && pick.vertexId === null ? [pick.cp] : [],
            )
          : activeTool === "construct"
            ? st.constructPoints
            : curveMode
              ? st.curvePoints
              : [],
      // 作図補助では次にすることを常に1行で出す(設計原則3b)
      // つかんで動かしている間は、その案内を他より優先して出す
      hint:
        panHint(st) ??
        (st.vertexDrag
          ? "点を動かしています(離すと決まります。Escでやめる)"
          : st.lineInputHint ?? toolHint),
      tooltip: violationTooltip(doc, st.hoverViolation),
      vertexDrag: st.vertexDrag
        ? { id: st.vertexDrag.id, to: st.vertexDrag.to }
        : null,
      suggestedCreases: useAppStore.getState().pendingFoldThrough?.proposal
        .crease_segments,
      hoveredHinge,
      suspectHinges,
      activeHinges: activeAngleIntent?.hinges ?? [],
      // 固定した折り目は、選んでいなくても印を出す
      pinnedHinges: [...pinnedFolds.keys()],
      releasedPinHinges: releasedPins.map((pin) => pin.hinge),
    };
    if (captureClean) {
      // 撮影画像には作品そのものだけを残し、操作中だけの案内・強調を消す。
      overlay.hoverSnap = null;
      overlay.preview = null;
      overlay.directionGuide = null;
      overlay.mirrorAxis = null;
      overlay.mirrorPreview = null;
      overlay.previewPaths = [];
      overlay.marquee = null;
      overlay.violations = [];
      overlay.constructPoints = [];
      overlay.hint = null;
      overlay.tooltip = null;
      overlay.vertexDrag = null;
      overlay.suggestedCreases = undefined;
      overlay.hoveredHinge = null;
      overlay.suspectHinges = [];
      overlay.activeHinges = [];
      overlay.pinnedHinges = [];
      overlay.releasedPinHinges = [];
    }
    const ctx2d = canvas.getContext("2d");
    if (ctx2d) {
      const visibleSelection = alignDraft
        ? alignSelection(alignDraft)
        : selection;
      render(
        ctx2d,
        w,
        h,
        dpr,
        cpDocument,
        viewRef.current,
        captureClean ? { edgeIds: [], vertexIds: [] } : visibleSelection,
        overlay,
      );
    }
  }, []);

  /** 操作ハンドラへ渡す文脈を組み立てる(表示前はnull) */
  const makeCtx = useCallback((): InteractionCtx | null => {
    const s = useAppStore.getState();
    if (!s.doc || !viewRef.current) return null;
    const stepDoc = documentForCpStep(s.doc, s.currentStep, s.stepCreases);
    return {
      doc: stepDoc,
      finalDoc: s.doc,
      view: viewRef.current,
      tool: s.activeTool,
      selection: s.selection,
      alignDraft: s.alignDraft,
      faces: s.faces,
      frame3d: s.frame3d,
      construct: s.construct,
      curve: s.curve,
      measureMode: s.measureDraft.mode,
      wheelBehavior: s.wheelBehavior,
      violations: violationsForCpStep(stepDoc, s.violations),
      // 点移動の対称位置吸着は、対称描画のオン・オフに関係なく現在の基準を使う。
      // 選んだ線が編集直後に無効なら、画面表示・対称編集と同じ縦中心へ戻す。
      mirrorAxis:
        mirrorLineForChoice(s.doc, s.mirrorAxis) ??
        paperMirrorLine(s.doc.paper, "paperVertical"),
      state: stateRef.current,
      setView: (v) => {
        viewRef.current = v;
      },
      applyEdit: s.applyEdit,
      drawSegment: (a, b, kind) => void s.drawSegment(a, b, kind),
      drawCurve: (points, kind) => void s.drawCurve(points, kind),
      setSelection: s.setSelection,
      beginFoldDraft: s.beginFoldDraft,
      pickAlignTarget: s.pickAlignTarget,
      pickMeasureEdge: s.pickMeasureEdge,
      pickMeasurePoint: s.pickMeasurePoint,
    };
  }, []);

  // ストアの変化(線の追加・選択・ツール切替・畳めない点・作図の選び方)で再描画
  useEffect(() => {
    draw();
  }, [
    doc,
    currentStep,
    selection,
    hoveredHinge,
    suspectHinges,
    pinnedFolds,
    releasedPins,
    activeAngleIntent,
    activeTool,
    measureDraft,
    violations,
    stepCreases,
    construct,
    curve,
    mirrorDraw,
    mirrorAxisChoice,
    wheelBehavior,
    pendingFoldThrough,
    alignDraft,
    foldDraft,
    draw,
  ]);

  // 新規作成・ファイルを開いた直後は紙全体が見える表示に戻す
  useEffect(() => {
    viewRef.current = null; // 次のdrawが全体表示から作り直す
    draw();
  }, [docEpoch, draw]);

  // CSS変数の実効値を読み直し、テーマ切替と同じフレームで背景を描き替える。
  useEffect(() => {
    draw();
  }, [uiTheme, draw]);

  // ツール・作図方式・曲線の入切/形を変えたら、描画途中・選択途中を破棄する。
  // 別の入力規則へ古い点や線を引き継ぐ取り違えを防ぐ。
  useEffect(() => {
    const st = stateRef.current;
    st.pendingStart = null;
    st.lineInputHint = null;
    st.downScreen = null;
    st.marqueeStart = null;
    st.marqueeEnd = null;
    st.selectionToggle = false;
    st.constructPoints = [];
    st.constructSeg = null;
    st.curvePoints = [];
    st.vertexDrag = null;
    st.directionSnap = null;
    draw();
  }, [
    activeTool,
    alignDraft?.mode,
    construct.kind,
    curve.enabled,
    curve.shape,
    measureDraft.mode,
    draw,
  ]);

  // 区画サイズの変化に追従
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [draw]);

  // 「全体表示」を親(ツールレール)から呼べるように登録
  useEffect(() => {
    fitRef.current = () => {
      const canvas = canvasRef.current;
      const doc = useAppStore.getState().doc;
      if (canvas && doc) {
        viewRef.current = fitView(doc, canvas.clientWidth, canvas.clientHeight);
        draw();
      }
    };
    return () => {
      fitRef.current = null;
    };
  }, [fitRef, draw]);

  // Esc(描画中止)・Delete(選択線の削除)・スペース(押している間つかんで動かす)
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return;
      // スペースは画面のスクロールに使われるので、つかむ操作のために止める
      if (isSpaceKey(e.key)) e.preventDefault();
      const ctx = makeCtx();
      if (ctx) {
        onKeyDown(ctx, e.key);
        const s = useAppStore.getState();
        if (e.key === "Escape" && s.activeTool === "measure") {
          s.clearMeasurement();
        }
        draw();
      }
    };
    const up = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return;
      const ctx = makeCtx();
      if (ctx) {
        onKeyUp(ctx, e.key);
        draw();
      }
    };
    // 別の窓へ移ったときにスペースを押しっぱなしと誤解しないよう解除する
    const blur = () => {
      stateRef.current.spaceHeld = false;
      stateRef.current.shiftHeld = false;
      stateRef.current.panLast = null;
      stateRef.current.directionSnap = null;
      draw();
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, [makeCtx, draw]);

  const screenPos = (e: React.MouseEvent<HTMLCanvasElement>): Vec2 => {
    const rect = e.currentTarget.getBoundingClientRect();
    return [e.clientX - rect.left, e.clientY - rect.top];
  };

  const withCtx = (f: (ctx: InteractionCtx) => void) => {
    const ctx = makeCtx();
    if (ctx) {
      f(ctx);
      draw();
    }
  };

  const totalSteps = doc?.sequence.length ?? 0;
  const displayedStep =
    currentStep === null
      ? totalSteps
      : Math.max(0, Math.min(totalSteps, Math.trunc(currentStep)));

  return (
    <div className="cp-editor">
      <CpOperationHint />
      <canvas
        ref={canvasRef}
        className="cp-canvas"
        onPointerDown={(e) => {
          e.preventDefault();
          const hadStart = stateRef.current.pendingStart !== null;
          const constructBefore = constructDone(stateRef.current);
          const s = useAppStore.getState();
          if (
            (s.activeTool === "mountain" ||
              s.activeTool === "valley" ||
              s.activeTool === "aux" ||
              s.activeTool === "construct") &&
            s.operationStage === 2
          ) {
            s.setOperationStage(0);
          }
          // ポインタ捕捉: canvas外へ出てもmove/upが届き、ドラッグ状態が残留しない
          e.currentTarget.setPointerCapture(e.pointerId);
          withCtx((ctx) =>
            onMouseDown(
              ctx,
              screenPos(e),
              e.button,
              e.shiftKey,
              e.ctrlKey || e.metaKey,
            ),
          );
          if (
            s.activeTool === "mountain" ||
            s.activeTool === "valley" ||
            s.activeTool === "aux"
          ) {
            const hasStart = stateRef.current.pendingStart !== null;
            if (!hadStart && hasStart) s.setOperationStage(1);
            else if (hadStart && !hasStart) s.setOperationStage(2);
          } else if (s.activeTool === "construct" && e.button === 0) {
            const constructAfter = constructDone(stateRef.current);
            const required = CONSTRUCT_STEPS[s.construct.kind].length;
            if (constructAfter > 0) s.setOperationStage(1);
            else if (constructBefore > 0 || required === 1) s.setOperationStage(2);
          }
        }}
        onPointerMove={(e) =>
          withCtx((ctx) => onMouseMove(ctx, screenPos(e), e.shiftKey))
        }
        onPointerUp={(e) => {
          withCtx((ctx) =>
            onMouseUp(ctx, screenPos(e), e.button, e.ctrlKey || e.metaKey),
          );
        }}
        onPointerLeave={() => {
          // 捕捉中はleaveが飛ばないため、ここに来るのはドラッグしていない時だけ
          stateRef.current.hoverSnap = null;
          stateRef.current.directionSnap = null;
          stateRef.current.cursorWorld = null;
          stateRef.current.hoverViolation = null;
          draw();
        }}
        onPointerCancel={() => {
          // 捕捉が中断されたらドラッグ系の一時状態を破棄する
          const st = stateRef.current;
          st.downScreen = null;
          st.panLast = null;
          st.marqueeStart = null;
          st.marqueeEnd = null;
          st.selectionToggle = false;
          st.vertexDrag = null;
          st.directionSnap = null;
          draw();
        }}
        onWheel={(e) => {
          e.preventDefault();
          // DOM_DELTA_LINE / DOM_DELTA_PAGEも同じCSS px単位へそろえる。
          const unit =
            e.deltaMode === 1
              ? 16
              : e.deltaMode === 2
                ? e.currentTarget.clientHeight
                : 1;
          withCtx((ctx) =>
            onWheel(ctx, screenPos(e), {
              deltaX: e.deltaX * unit,
              deltaY: e.deltaY * unit,
              shiftKey: e.shiftKey,
              ctrlKey: e.ctrlKey,
            }),
          );
        }}
        onContextMenu={(e) => e.preventDefault()}
      />
      <div
        className="cp-step-indicator"
        data-floating-ui="cp-step-indicator"
        aria-label="展開図に表示している手順"
        data-tooltip="この手順までに付いた折り線を表示しています"
        tabIndex={0}
      >
        手順 {displayedStep} / {totalSteps}
      </div>
    </div>
  );
}
