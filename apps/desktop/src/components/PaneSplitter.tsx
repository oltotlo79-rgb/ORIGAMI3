// 2D区画と3D区画の境目(UI-004)。左右にドラッグすると広さが変わり、
// 決めた広さは次に起動したときも同じに戻る(ストア → localStorage)。
// 区画そのものは増やさない(4区画の境目に取っ手を置くだけ)。

import { useRef } from "react";
import { useAppStore } from "../store/appStore";
import { MAX_SPLIT_RATIO, MIN_SPLIT_RATIO } from "../lib/displayPrefs";

/** ツールレールの幅(px)。App.cssの.main-rowの1列目と合わせる */
const RAIL_PX = 64;
/** 取っ手の幅(px)。App.cssの.pane-splitterと合わせる */
const HANDLE_PX = 6;
/** キー操作1回で動かす割合 */
const KEY_STEP = 0.02;

export function PaneSplitter() {
  const ratio = useAppStore((s) => s.splitRatio);
  const setSplitRatio = useAppStore((s) => s.setSplitRatio);
  // ドラッグ中かどうかは見た目にも状態にも出さない一時情報なのでrefで持つ
  const draggingRef = useRef(false);

  /** 画面上のx座標を「2D区画の割合」に直す */
  const ratioAt = (clientX: number, el: HTMLElement): number | null => {
    const row = el.parentElement;
    if (!row) return null;
    const rect = row.getBoundingClientRect();
    const usable = rect.width - RAIL_PX - HANDLE_PX;
    if (usable <= 0) return null;
    return (clientX - rect.left - RAIL_PX) / usable;
  };

  return (
    <div
      className="pane-splitter"
      role="separator"
      aria-orientation="vertical"
      aria-label="展開図と立体の広さを変える"
      aria-valuenow={Math.round(ratio * 100)}
      aria-valuemin={Math.round(MIN_SPLIT_RATIO * 100)}
      aria-valuemax={Math.round(MAX_SPLIT_RATIO * 100)}
      tabIndex={0}
      data-tooltip="左右にドラッグして、展開図と3Dの広さを変えます"
      onPointerDown={(e) => {
        e.preventDefault();
        draggingRef.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onPointerMove={(e) => {
        if (!draggingRef.current) return;
        const next = ratioAt(e.clientX, e.currentTarget);
        if (next !== null) setSplitRatio(next);
      }}
      onPointerUp={(e) => {
        draggingRef.current = false;
        e.currentTarget.releasePointerCapture(e.pointerId);
      }}
      onPointerCancel={() => {
        draggingRef.current = false;
      }}
      onKeyDown={(e) => {
        if (e.key === "ArrowLeft") setSplitRatio(ratio - KEY_STEP);
        else if (e.key === "ArrowRight") setSplitRatio(ratio + KEY_STEP);
      }}
    />
  );
}
