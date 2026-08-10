// 技法で対象にする層の選び方。facesAtPoint が返す「奥→手前」の順を
// そのまま保ち、枚数指定・奥行き指定を面IDの部分集合へ変換する。

import type { TechniqueKind } from "./types";

export type TechniqueLayerPreset = "all" | "front" | "back" | "frontNth";

/** Rust側で `open_to_back` を参照する技法。ほかの技法では指定を送らない。 */
export function techniqueUsesOpenToBack(kind: TechniqueKind): boolean {
  return kind === "Squash" || kind === "Petal" || kind === "Swivel" || kind === "Twist";
}

/**
 * 技法を画面から適用するために最低限選ぶ層数。
 * 空=その領域の全層、というRust側の規約を持つ技法は0枚でよい。
 */
export function minimumTechniqueFlap(kind: TechniqueKind): number {
  if (kind === "InsideReverse" || kind === "OutsideReverse") return 2;
  if (kind === "Squash" || kind === "Petal") return 1;
  return 0;
}

/** 数値欄の枚数・奥行きを、候補層に使える整数へ丸める。 */
export function clampTechniqueLayerCount(value: number, candidateCount: number): number {
  const max = Math.max(1, candidateCount);
  const rounded = Number.isFinite(value) ? Math.round(value) : 1;
  return Math.max(1, Math.min(max, rounded));
}

/**
 * 奥→手前の候補から、ボタンで指定した部分集合を同じ順序で返す。
 * `frontNth` は「手前からN枚目」1枚だけを選ぶ。
 */
export function techniqueFlapForPreset(
  candidates: readonly number[],
  preset: TechniqueLayerPreset,
  count: number,
): number[] {
  if (candidates.length === 0) return [];
  if (preset === "all") return [...candidates];
  const n = clampTechniqueLayerCount(count, candidates.length);
  if (preset === "front") return candidates.slice(candidates.length - n);
  if (preset === "back") return candidates.slice(0, n);
  return [candidates[candidates.length - n]];
}

/** 候補1枚のチェックを切り替え、候補の奥→手前順に並べ直す。 */
export function toggleTechniqueFlap(
  candidates: readonly number[],
  selected: readonly number[],
  face: number,
): number[] {
  const chosen = new Set(selected.filter((id) => candidates.includes(id)));
  if (chosen.has(face)) chosen.delete(face);
  else if (candidates.includes(face)) chosen.add(face);
  return candidates.filter((id) => chosen.has(id));
}
