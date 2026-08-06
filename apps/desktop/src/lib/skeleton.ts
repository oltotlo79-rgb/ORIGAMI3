// 骨格(PRO-001)の編集とプレビュー配置の計算。純関数だけを置く。
//
// v1の骨格は「胴の中心(根)から出っぱりが放射状に伸びる」形に限る。
// 木構造としては根1つ+葉N本で、Rust側のSkeletonにそのまま渡せる。

import type { Skeleton, SkeletonNode, Vec2 } from "./types";

/** 出っぱりの本数の上下限(Rust側のMAX_LEAVESと揃える) */
export const MIN_LIMBS = 1;
export const MAX_LIMBS = 12;

/** 長さ・太さのスライダーの範囲と刻み */
export const LENGTH_RANGE = { min: 0.2, max: 3, step: 0.1 } as const;
export const WIDTH_RANGE = { min: 0.3, max: 2, step: 0.1 } as const;

/** 折り紙でよくある出っぱりの呼び名。足りない分は番号で呼ぶ */
const LIMB_NAMES = [
  "頭",
  "尾",
  "右前足",
  "左前足",
  "右後足",
  "左後足",
  "右の羽",
  "左の羽",
];

/** i番目(0始まり)の出っぱりの表示名 */
export function limbLabel(i: number): string {
  return LIMB_NAMES[i] ?? `出っぱり${i + 1}`;
}

/** 根(胴の中心)のID。追加・削除しても変わらない */
export const ROOT_ID = 0;

/** 出っぱり(葉)だけを並び順のまま取り出す */
export function limbs(s: Skeleton): SkeletonNode[] {
  return s.nodes.filter((n) => n.parent !== null);
}

/** 初期の骨格: 頭・尾・足2本の4本立て(いかにも生きもの、を最初の見本にする) */
export function defaultSkeleton(): Skeleton {
  const root: SkeletonNode = {
    id: ROOT_ID,
    parent: null,
    length: 0,
    width_factor: 1,
  };
  const limb = (id: number): SkeletonNode => ({
    id,
    parent: ROOT_ID,
    length: 1,
    width_factor: 1,
  });
  return { nodes: [root, limb(1), limb(2), limb(3), limb(4)] };
}

/** 出っぱりを1本増やす(上限に達していればそのまま) */
export function addLimb(s: Skeleton): Skeleton {
  if (limbs(s).length >= MAX_LIMBS) return s;
  const id = Math.max(...s.nodes.map((n) => n.id)) + 1;
  const nodes = [...s.nodes, { id, parent: ROOT_ID, length: 1, width_factor: 1 }];
  return { nodes };
}

/** 出っぱりを1本減らす(下限に達していればそのまま) */
export function removeLimb(s: Skeleton, id: number): Skeleton {
  if (limbs(s).length <= MIN_LIMBS) return s;
  if (!s.nodes.some((n) => n.id === id && n.parent !== null)) return s;
  return { nodes: s.nodes.filter((n) => n.id !== id) };
}

/** 1本の出っぱりの長さ・太さを書き換える */
export function setLimb(
  s: Skeleton,
  id: number,
  patch: Partial<Pick<SkeletonNode, "length" | "width_factor">>,
): Skeleton {
  return {
    nodes: s.nodes.map((n) => (n.id === id ? { ...n, ...patch } : n)),
  };
}

/** プレビュー1本分: 根から先端までの線と、太さに応じた線幅 */
export interface LimbLayout {
  id: number;
  end: Vec2;
  /** 先端の丸(=角の膨らみ)の半径。長さ×太さ係数 */
  radius: number;
}

/**
 * 骨格の2Dプレビュー配置。根を原点、出っぱりを真上から時計回りに等間隔で置く。
 * 座標は骨格の長さの単位そのまま(表示側で紙の枠に収める)。
 */
export function previewLayout(s: Skeleton): LimbLayout[] {
  const list = limbs(s);
  return list.map((n, i) => {
    const a = Math.PI / 2 - (2 * Math.PI * i) / list.length;
    return {
      id: n.id,
      end: [n.length * Math.cos(a), n.length * Math.sin(a)] as Vec2,
      radius: n.length * n.width_factor,
    };
  });
}
