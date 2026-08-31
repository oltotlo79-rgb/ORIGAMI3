// 画面の中で「字」ではなく「図」で出す小さな印。
//
// 探す・最初へ・一時停止・閉じるの4つは、以前はUnicodeの記号文字(⌕ ⏮ ⏸ ✕)で
// 出していた。しかし同梱している日本語フォントにも、その元になったNoto Sans JP
// 2.004-H2(9,589,900 B)にもこの4字は入っていないため、機械に入っているフォント
// 次第で形が変わり、記号フォントが無い機械では四角い箱になっていた。
// ここで図として描けば、どの機械でも同じ形で出る。
//
// 図は読み上げの対象にしない(aria-hidden)。押しボタンの名前は、隣に置く日本語の
// 文言か aria-label が持つ。文言は記号を外す前と同じものを使う。

import type { ReactNode } from "react";

/** 描ける印の種類。増やすときはここと ICON_SHAPES の両方へ足す。 */
export type UiIconName = "search" | "skip-to-start" | "pause" | "close";

/**
 * 16x16の升目で描く。線は太さ2の丸端、塗りは currentColor にして、
 * 隣の文字と同じ色・同じ大きさ(1em)で並ぶようにする。
 */
const ICON_SHAPES: Record<UiIconName, ReactNode> = {
  // 虫めがね: 輪と柄
  search: (
    <>
      <circle cx="7" cy="7" r="4.25" fill="none" stroke="currentColor" strokeWidth="1.6" />
      <path
        d="M10.4 10.4 L14 14"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </>
  ),
  // 最初へ: 縦棒と左向きの三角
  "skip-to-start": (
    <>
      <rect x="2.5" y="3" width="1.9" height="10" rx="0.6" fill="currentColor" />
      <path d="M13.5 3.4 L13.5 12.6 L6.2 8 Z" fill="currentColor" />
    </>
  ),
  // 一時停止: 縦棒2本
  pause: (
    <>
      <rect x="4.2" y="3" width="2.6" height="10" rx="0.8" fill="currentColor" />
      <rect x="9.2" y="3" width="2.6" height="10" rx="0.8" fill="currentColor" />
    </>
  ),
  // 閉じる: 斜めの2本
  close: (
    <path
      d="M4 4 L12 12 M12 4 L4 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
    />
  ),
};

/** 隣の文字と同じ大きさ・同じ色で並ぶ、読み上げ対象外の印を描く。 */
export function UiIcon({ name }: { name: UiIconName }) {
  return (
    <svg
      className="ui-icon"
      viewBox="0 0 16 16"
      width="1em"
      height="1em"
      aria-hidden="true"
      focusable="false"
    >
      {ICON_SHAPES[name]}
    </svg>
  );
}
