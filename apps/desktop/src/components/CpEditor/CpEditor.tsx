// 2D展開図エディタ: canvas要素とイベント接続、ストア購読で再描画。
// ズーム/パン・描画途中などの一時状態はコンポーネント内に保持する(表示専用)。

import { useCallback, useEffect, useRef } from "react";
import type { Vec2 } from "../../lib/types";
import { useAppStore } from "../../store/appStore";
import {
  initialEphemeralState,
  onKeyDown,
  onMouseDown,
  onMouseMove,
  onMouseUp,
  onWheel,
  TOOL_KIND,
  type InteractionCtx,
} from "./interaction";
import { fitView, render, type RenderOverlay, type ViewTransform } from "./renderer";

interface Props {
  /** 「全体表示」用: 親が current を呼ぶと紙全体が収まる表示に戻す */
  fitRef: React.RefObject<(() => void) | null>;
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

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const { doc, selection, activeTool } = useAppStore.getState();
    if (!canvas || !doc) return;
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
    const kind = TOOL_KIND[activeTool];
    const overlay: RenderOverlay = {
      hoverSnap: kind ? st.hoverSnap : null,
      preview:
        kind && st.pendingStart && st.cursorWorld
          ? { a: st.pendingStart, b: st.hoverSnap?.pos ?? st.cursorWorld, kind }
          : null,
      marquee:
        st.marqueeStart && st.marqueeEnd ? { a: st.marqueeStart, b: st.marqueeEnd } : null,
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
      state: stateRef.current,
      setView: (v) => {
        viewRef.current = v;
      },
      applyEdit: s.applyEdit,
      setSelection: s.setSelection,
    };
  }, []);

  // ストアの変化(線の追加・選択・ツール切替)で再描画
  useEffect(() => {
    draw();
  }, [doc, selection, activeTool, draw]);

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

  // Esc(描画中止)・Delete(選択線の削除)
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLElement && e.target.tagName === "INPUT") return;
      const ctx = makeCtx();
      if (ctx) {
        onKeyDown(ctx, e.key);
        draw();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
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
      style={{
        cursor:
          TOOL_KIND[activeTool] !== undefined || activeTool === "delete"
            ? "crosshair"
            : "default",
      }}
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
        draw();
      }}
      onWheel={(e) => withCtx((ctx) => onWheel(ctx, screenPos(e), e.deltaY))}
      onContextMenu={(e) => e.preventDefault()}
    />
  );
}
