// 2D展開図エディタ: canvas要素とイベント接続、ストア購読で再描画。
// ズーム/パン・描画途中などの一時状態はコンポーネント内に保持する(表示専用)。

import { useCallback, useEffect, useRef } from "react";
import type { Vec2 } from "../../lib/types";
import { useAppStore } from "../../store/appStore";
import { constructHint } from "../../lib/construct";
import {
  curveHint,
  firstCrossing,
  rulingLines,
  type CurveOptions,
} from "../../lib/curve";
import { violationReason } from "../../lib/flatFoldHint";
import { mirrorAxisX, mirrorPoint } from "../../lib/mirror";
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
  const selection = useAppStore((s) => s.selection);
  const activeTool = useAppStore((s) => s.activeTool);
  const docEpoch = useAppStore((s) => s.docEpoch);
  const violations = useAppStore((s) => s.violations);
  const construct = useAppStore((s) => s.construct);
  const curve = useAppStore((s) => s.curve);
  const mirrorDraw = useAppStore((s) => s.mirrorDraw);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const { doc, selection, activeTool, violations, construct, curve, mirrorDraw } =
      useAppStore.getState();
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
    const st = stateRef.current;
    const kind = previewKind(activeTool);
    // 左右対称のときは対称軸を薄く出し、引いている最中の線も反対側に見せる
    const axisX = mirrorDraw ? mirrorAxisX(doc.paper) : null;
    const curveMode = kind !== undefined && curve.enabled && activeTool !== "fold";
    const preview =
      kind && !curveMode && st.pendingStart && st.cursorWorld
        ? { a: st.pendingStart, b: st.hoverSnap?.pos ?? st.cursorWorld, kind }
        : null;
    // 曲線モードでは、確定したときに入るのと同じ折れ線をそのまま見せる(設計原則3b)
    const draft = curveMode ? curveDraft(st, curve) : null;
    const previewPaths = draft && kind ? curvePreviewPaths(doc, draft, kind, curve) : [];
    const overlay: RenderOverlay = {
      hoverSnap: kind ? st.hoverSnap : null,
      preview,
      mirrorAxis: axisX,
      mirrorPreview:
        axisX !== null && preview
          ? {
              a: mirrorPoint(preview.a, axisX),
              b: mirrorPoint(preview.b, axisX),
              kind: preview.kind,
            }
          : null,
      previewPaths,
      marquee:
        st.marqueeStart && st.marqueeEnd ? { a: st.marqueeStart, b: st.marqueeEnd } : null,
      violations,
      constructPoints:
        activeTool === "construct" ? st.constructPoints : curveMode ? st.curvePoints : [],
      // 作図補助では次にすることを常に1行で出す(設計原則3b)
      // つかんで動かしている間は、その案内を他より優先して出す
      hint:
        panHint(st) ??
        (st.vertexDrag
          ? "点を動かしています(離すと決まります。Escでやめる)"
          : activeTool === "construct"
          ? constructHint(construct.kind, constructDone(st), construct.divisions)
          : curveMode
            ? curveHint(curve.shape, st.curvePoints.length, curve.rulings)
            : // 今どの描き方かが画面の上で分かるようにする(設計原則3b)
              mirrorDraw
              ? "左右対称に描いています(線を引くときだけ効きます)"
              : null),
      tooltip: violationTooltip(doc, st.hoverViolation),
      vertexDrag: st.vertexDrag
        ? { id: st.vertexDrag.id, to: st.vertexDrag.to }
        : null,
    };
    const ctx2d = canvas.getContext("2d");
    if (ctx2d) {
      render(ctx2d, w, h, dpr, doc, viewRef.current, selection, overlay);
    }
  }, []);

  /** 操作ハンドラへ渡す文脈を組み立てる(表示前はnull) */
  const makeCtx = useCallback((): InteractionCtx | null => {
    const s = useAppStore.getState();
    if (!s.doc || !viewRef.current) return null;
    return {
      doc: s.doc,
      view: viewRef.current,
      tool: s.activeTool,
      selection: s.selection,
      construct: s.construct,
      curve: s.curve,
      violations: s.violations,
      state: stateRef.current,
      setView: (v) => {
        viewRef.current = v;
      },
      applyEdit: s.applyEdit,
      drawSegment: (a, b, kind) => void s.drawSegment(a, b, kind),
      drawCurve: (points, kind) => void s.drawCurve(points, kind),
      setSelection: s.setSelection,
      beginFoldDraft: s.beginFoldDraft,
    };
  }, []);

  // ストアの変化(線の追加・選択・ツール切替・畳めない点・作図の選び方)で再描画
  useEffect(() => {
    draw();
  }, [doc, selection, activeTool, violations, construct, curve, mirrorDraw, draw]);

  // 新規作成・ファイルを開いた直後は紙全体が見える表示に戻す
  useEffect(() => {
    viewRef.current = null; // 次のdrawが全体表示から作り直す
    draw();
  }, [docEpoch, draw]);

  // ツール切替時は描画途中・選択途中の一時状態を破棄する
  // (山折りの1点目を谷折りに引き継ぐ、といった取り違えを防ぐ)
  useEffect(() => {
    const st = stateRef.current;
    st.pendingStart = null;
    st.downScreen = null;
    st.marqueeStart = null;
    st.marqueeEnd = null;
    st.constructPoints = [];
    st.constructSeg = null;
    st.curvePoints = [];
    st.vertexDrag = null;
    draw();
  }, [activeTool, draw]);

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
    const isInput = (t: EventTarget | null) =>
      t instanceof HTMLElement && t.tagName === "INPUT";
    const down = (e: KeyboardEvent) => {
      if (isInput(e.target)) return;
      // スペースは画面のスクロールに使われるので、つかむ操作のために止める
      if (isSpaceKey(e.key)) e.preventDefault();
      const ctx = makeCtx();
      if (ctx) {
        onKeyDown(ctx, e.key);
        draw();
      }
    };
    const up = (e: KeyboardEvent) => {
      if (isInput(e.target)) return;
      const ctx = makeCtx();
      if (ctx) {
        onKeyUp(ctx, e.key);
        draw();
      }
    };
    // 別の窓へ移ったときにスペースを押しっぱなしと誤解しないよう解除する
    const blur = () => {
      stateRef.current.spaceHeld = false;
      stateRef.current.panLast = null;
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

  return (
    <canvas
      ref={canvasRef}
      className="cp-canvas"
      onPointerDown={(e) => {
        e.preventDefault();
        // ポインタ捕捉: canvas外へ出てもmove/upが届き、ドラッグ状態が残留しない
        e.currentTarget.setPointerCapture(e.pointerId);
        withCtx((ctx) => onMouseDown(ctx, screenPos(e), e.button));
      }}
      onPointerMove={(e) => withCtx((ctx) => onMouseMove(ctx, screenPos(e)))}
      onPointerUp={(e) => withCtx((ctx) => onMouseUp(ctx, screenPos(e), e.button))}
      onPointerLeave={() => {
        // 捕捉中はleaveが飛ばないため、ここに来るのはドラッグしていない時だけ
        stateRef.current.hoverSnap = null;
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
        st.vertexDrag = null;
        draw();
      }}
      onWheel={(e) => withCtx((ctx) => onWheel(ctx, screenPos(e), e.deltaY))}
      onContextMenu={(e) => e.preventDefault()}
    />
  );
}
