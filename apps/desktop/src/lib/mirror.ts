// 左右対称に線を引くための計算(CPE-010)。
// 折り紙の作品は左右対称のものが多いので、片側に引いた線を紙の縦の中心線で
// 折り返した線も一緒に引けるようにする。ここは画面もIPCも触らない純粋な計算。
// v1の対称軸は「紙の縦の中心線」だけ(斜め・横の軸は扱わない)。

import type { Paper, Vec2 } from "./types";

/** 同じ場所とみなす距離(正規化座標。紙の長辺=1.0) */
export const MIRROR_EPS = 1e-6;

/** 線分(始点・終点) */
export type Segment = [Vec2, Vec2];

/** 紙の縦の中心線のx座標(正規化座標。長辺=1.0) */
export function mirrorAxisX(paper: Paper): number {
  const long = Math.max(paper.width_mm, paper.height_mm);
  return long > 0 ? paper.width_mm / long / 2 : 0;
}

/** 中心線をはさんで反対側へ移した点 */
export function mirrorPoint(p: Vec2, axisX: number): Vec2 {
  return [2 * axisX - p[0], p[1]];
}

/** 中心線をはさんで反対側へ移した線分 */
export function mirrorSegment(seg: Segment, axisX: number): Segment {
  return [mirrorPoint(seg[0], axisX), mirrorPoint(seg[1], axisX)];
}

/** その線分が中心線の上に乗っているか(両端のxが中心線と同じ) */
export function isOnMirrorAxis(
  seg: Segment,
  axisX: number,
  eps = MIRROR_EPS,
): boolean {
  return Math.abs(seg[0][0] - axisX) <= eps && Math.abs(seg[1][0] - axisX) <= eps;
}

function samePoint(a: Vec2, b: Vec2, eps: number): boolean {
  return Math.abs(a[0] - b[0]) <= eps && Math.abs(a[1] - b[1]) <= eps;
}

/** 同じ線分か(向きが逆でも同じとみなす) */
export function isSameSegment(x: Segment, y: Segment, eps = MIRROR_EPS): boolean {
  return (
    (samePoint(x[0], y[0], eps) && samePoint(x[1], y[1], eps)) ||
    (samePoint(x[0], y[1], eps) && samePoint(x[1], y[0], eps))
  );
}

/**
 * 実際に引く線分を返す。左右対称のときは元の線と鏡映した線の2本、
 * 中心線の上の線や、もともと左右対称な線は同じ線を二重に引かないよう1本だけ。
 */
export function mirrorSegments(
  seg: Segment,
  axisX: number,
  eps = MIRROR_EPS,
): Segment[] {
  const other = mirrorSegment(seg, axisX);
  if (isOnMirrorAxis(seg, axisX, eps) || isSameSegment(seg, other, eps)) {
    return [seg];
  }
  return [seg, other];
}
