// 層のずらし表示(UI-010 / SIM-004)の計算。Three.jsには依存しない純粋な計算。
//
// 平らに畳んだ紙は全ての層がz=0に重なるため、そのまま描くと紙が1枚に見えてしまい、
// 「どの層を選んでいるのか」が画面から読み取れない(要件の設計原則3b: 直感的に触れること)。
// そこで平らな状態のときだけ、層ごとに微小な高さを付けて重なりを見せる。
//
// ここで作る値は表示専用で、作品データ(Frame3D)そのものには一切反映しない。

import type { Frame3D } from "./types";

/** 層1枚あたりのずらし量(紙の長辺に対する割合) */
export const LAYER_STEP_RATIO = 0.01;

/** 重なり全体の厚みの上限(紙の長辺に対する割合)。層が多くても分厚く見せない */
export const MAX_STACK_RATIO = 0.03;

/** これ以下の高さは計算誤差とみなして「平ら」と扱う(foldDrawの判定と揃える) */
const FLAT_EPS = 1e-6;

/**
 * 層ごとのずらし量(下から0番)を返す。
 * 間隔は「紙の長辺 × LAYER_STEP_RATIO」を基本とし、層が多いときは重なり全体の
 * 厚みが MAX_STACK_RATIO を超えないよう間隔を詰める。
 */
export function layerOffsets(layerCount: number, paperScale: number): number[] {
  const count = Math.floor(layerCount);
  if (!Number.isFinite(count) || count <= 0) return [];
  const scale = Number.isFinite(paperScale) && paperScale > 0 ? paperScale : 0;
  const gaps = count - 1;
  const step =
    gaps === 0
      ? 0
      : Math.min(LAYER_STEP_RATIO, MAX_STACK_RATIO / gaps) * scale;
  const out = new Array<number>(count);
  for (let i = 0; i < count; i++) out[i] = i * step;
  return out;
}

/**
 * 平らに畳んだ状態か(全ての面が同じ平面=z≒0に乗っているか)。
 * 折り途中や立体的な形にずらしを掛けると形が歪むので、この判定を通ったときだけ使う。
 */
export function isFlatFrame(frame: Frame3D, eps = FLAT_EPS): boolean {
  if (frame.faces.length === 0) return false;
  return frame.faces.every((f) => f.polygon.every((p) => Math.abs(p[2]) <= eps));
}

/** 重なりの枚数(層番号の最大+1。層は下から0) */
export function frameLayerCount(frame: Frame3D): number {
  let max = -1;
  for (const f of frame.faces) {
    if (f.layer > max) max = f.layer;
  }
  return max + 1;
}
