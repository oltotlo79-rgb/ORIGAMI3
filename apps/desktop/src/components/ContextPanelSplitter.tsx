// 作業画面と下部の「今できる操作」の境目。既存のPaneSplitterと同じく、
// ドラッグ・矢印キー・Zustand経由の端末保存を1つの取っ手へまとめる。
// 区画そのものは増やさず、4区画の境界だけを操作できるようにする。

import { useRef } from "react";
import {
  MAX_CONTEXT_PANEL_RATIO,
  MIN_CONTEXT_PANEL_RATIO,
} from "../lib/displayPrefs";
import { useAppStore } from "../store/appStore";

/** App.cssの.context-panel-splitterの高さと合わせる。 */
const HANDLE_PX = 10;
/** キー操作1回で動かす割合。既存の左右仕切りと同じ2%。 */
const KEY_STEP = 0.02;

export function ContextPanelSplitter() {
  const ratio = useAppStore((s) => s.contextPanelRatio);
  const setContextPanelRatio = useAppStore((s) => s.setContextPanelRatio);
  // ドラッグ中だけ必要な一時情報なので、永続状態にはせずrefで持つ。
  const draggingRef = useRef(false);

  /** 画面上のy座標を、下部パネルが作業領域に占める割合へ直す。 */
  const ratioAt = (clientY: number, el: HTMLElement): number | null => {
    const main = el.previousElementSibling as HTMLElement | null;
    const panel = el.nextElementSibling as HTMLElement | null;
    if (!main || !panel) return null;
    const mainRect = main.getBoundingClientRect();
    const panelRect = panel.getBoundingClientRect();
    const usable = mainRect.height + panelRect.height;
    if (usable <= 0) return null;
    const bottom = panelRect.top + panelRect.height;
    // ポインターを取っ手の中央に置いたとき、現在値から跳ねないよう半分を引く。
    return (bottom - clientY - HANDLE_PX / 2) / usable;
  };

  return (
    <div
      className="context-panel-splitter"
      role="separator"
      aria-orientation="horizontal"
      aria-label="作業画面と今できる操作の広さを変える"
      aria-controls="context-panel"
      aria-valuenow={Math.round(ratio * 100)}
      aria-valuemin={Math.round(MIN_CONTEXT_PANEL_RATIO * 100)}
      aria-valuemax={Math.round(MAX_CONTEXT_PANEL_RATIO * 100)}
      aria-valuetext={`今できる操作は画面の${Math.round(ratio * 100)}%`}
      tabIndex={0}
      data-tooltip="上下にドラッグして、下の操作欄の広さを変えます"
      onPointerDown={(e) => {
        e.preventDefault();
        draggingRef.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onPointerMove={(e) => {
        if (!draggingRef.current) return;
        const next = ratioAt(e.clientY, e.currentTarget);
        if (next !== null) setContextPanelRatio(next);
      }}
      onPointerUp={(e) => {
        draggingRef.current = false;
        e.currentTarget.releasePointerCapture(e.pointerId);
      }}
      onPointerCancel={() => {
        draggingRef.current = false;
      }}
      onLostPointerCapture={() => {
        draggingRef.current = false;
      }}
      onKeyDown={(e) => {
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setContextPanelRatio(ratio + KEY_STEP);
        } else if (e.key === "ArrowDown") {
          e.preventDefault();
          setContextPanelRatio(ratio - KEY_STEP);
        }
      }}
    />
  );
}
