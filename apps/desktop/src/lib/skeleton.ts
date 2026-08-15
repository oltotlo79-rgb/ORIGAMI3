// 骨格(PRO-001)の編集とプレビュー配置の計算。純関数だけを置く。
// 胴から出た部分の先へ、任意の深さと分岐でさらに出っぱりを足せる。

import type { Skeleton, SkeletonNode, TipPos2d, Vec2 } from "./types";

/** 出っぱりの本数の上下限(Rust側のMAX_LEAVESと揃える) */
export const MIN_LIMBS = 1;
export const MAX_LIMBS = 12;

/** 長さ・太さのスライダーの範囲と刻み */
export const LENGTH_RANGE = { min: 0.2, max: 3, step: 0.1 } as const;
export const WIDTH_RANGE = { min: 0.3, max: 2, step: 0.1 } as const;

/**
 * 完成形での先端の位置に指定できる範囲(Rust側のTIP_POS_MIN/TIP_POS_MAXと揃える)。
 * 原点(0,0)が胴の中心で、1.0は完成形を囲む正方形の中心から辺までの長さ。
 */
export const TIP_POS_MIN = -1;
export const TIP_POS_MAX = 1;

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

/** 本当の先端だけを、nodesに保存された順のまま取り出す */
export function leafNodes(s: Skeleton): SkeletonNode[] {
  const parentIds = new Set(
    s.nodes.flatMap((n) => (n.parent === null ? [] : [n.parent])),
  );
  return s.nodes.filter((n) => n.parent !== null && !parentIds.has(n.id));
}

/** 既存の呼び出し名との互換用。本数として数える出っぱりは本当の先端だけ。 */
export function limbs(s: Skeleton): SkeletonNode[] {
  return leafNodes(s);
}

function childrenByParent(s: Skeleton): Map<number, SkeletonNode[]> {
  const children = new Map<number, SkeletonNode[]>();
  for (const node of s.nodes) {
    if (node.parent === null) continue;
    const siblings = children.get(node.parent) ?? [];
    siblings.push(node);
    children.set(node.parent, siblings);
  }
  return children;
}

function rootNode(s: Skeleton): SkeletonNode | undefined {
  return (
    s.nodes.find((n) => n.id === ROOT_ID && n.parent === null) ??
    s.nodes.find((n) => n.parent === null)
  );
}

/** 画面の1行分。depthは胴を0とした深さなので、胴のすぐ先は1。 */
export interface SkeletonRow {
  node: SkeletonNode;
  depth: number;
  label: string;
}

/**
 * 親の直後へ子を並べた画面表示用の順序。
 * 胴のすぐ先の呼び名は、その先へ何本足しても変わらない。
 */
export function skeletonRows(s: Skeleton): SkeletonRow[] {
  const root = rootNode(s);
  if (!root) return [];

  const children = childrenByParent(s);
  const rows: SkeletonRow[] = [];
  const visited = new Set<number>([root.id]);
  const pending: SkeletonRow[] = [];
  const rootChildren = children.get(root.id) ?? [];
  for (let i = rootChildren.length - 1; i >= 0; i -= 1) {
    pending.push({ node: rootChildren[i], depth: 1, label: limbLabel(i) });
  }

  while (pending.length > 0) {
    const row = pending.pop();
    if (!row || visited.has(row.node.id)) continue;
    visited.add(row.node.id);
    rows.push(row);

    const childNodes = children.get(row.node.id) ?? [];
    for (let i = childNodes.length - 1; i >= 0; i -= 1) {
      pending.push({
        node: childNodes[i],
        depth: row.depth + 1,
        label: `その先${i + 1}`,
      });
    }
  }
  return rows;
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

function subtreeIds(s: Skeleton, id: number): Set<number> {
  const children = childrenByParent(s);
  const found = new Set<number>();
  const pending = [id];
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === undefined || found.has(current)) continue;
    found.add(current);
    for (const child of children.get(current) ?? []) pending.push(child.id);
  }
  return found;
}

/** 指定した場所の先へ足しても、先端が上限12本以内か。 */
export function canAddLimb(s: Skeleton, parentId = ROOT_ID): boolean {
  const parent = s.nodes.find((n) => n.id === parentId);
  if (!parent) return false;

  const parentIsLeaf =
    parent.parent !== null && !s.nodes.some((n) => n.parent === parentId);
  const nextLeafCount = leafNodes(s).length + (parentIsLeaf ? 0 : 1);
  return nextLeafCount <= MAX_LIMBS;
}

/**
 * 指定した場所の先へ出っぱりを1本足す。
 * 追加先を省略した従来の操作では、これまでどおり胴から足す。
 */
export function addLimb(s: Skeleton, parentId = ROOT_ID): Skeleton {
  if (!canAddLimb(s, parentId)) return s;

  const parent = s.nodes.find((n) => n.id === parentId)!;
  const parentWasLeaf =
    parent.parent !== null && !s.nodes.some((n) => n.parent === parentId);
  const id = s.nodes.reduce((max, n) => Math.max(max, n.id), -1) + 1;
  const descendants = subtreeIds(s, parentId);
  let insertAfter = -1;
  s.nodes.forEach((node, index) => {
    if (descendants.has(node.id)) insertAfter = index;
  });
  const node: SkeletonNode = {
    id,
    parent: parentId,
    length: 1,
    // 先端を延ばすときは、そこで決めていた太さを新しい先端へ受け継ぐ。
    // 既に途中になっている場所から分岐するときは、新しい枝の既定値に戻す。
    width_factor: parentWasLeaf ? parent.width_factor : 1,
  };
  return {
    nodes: [
      ...s.nodes.slice(0, insertAfter + 1),
      node,
      ...s.nodes.slice(insertAfter + 1),
    ],
  };
}

/** 指定部分とその先を消した後にも、先端が1本以上残るか。 */
export function canRemoveLimb(s: Skeleton, id: number): boolean {
  const target = s.nodes.find((n) => n.id === id);
  if (!target || target.parent === null) return false;
  const removed = subtreeIds(s, id);
  const nodes = s.nodes.filter((n) => !removed.has(n.id));
  return leafNodes({ nodes }).length >= MIN_LIMBS;
}

/** 指定部分を、その先に足されたものも含めて一緒に消す。 */
export function removeLimb(s: Skeleton, id: number): Skeleton {
  if (!canRemoveLimb(s, id)) return s;
  const removed = subtreeIds(s, id);
  return { nodes: s.nodes.filter((n) => !removed.has(n.id)) };
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

/**
 * 完成形での先端の位置を範囲内へ収める(Rust側の`TipPos2d::clamped`と同じ規則)。
 * 画面から先端を引っぱって動かすとき用で、数値にならない値は原点の成分0へ寄せる。
 */
export function clampTipPos(pos: TipPos2d): TipPos2d {
  const fit = (v: number): number =>
    Number.isNaN(v) ? 0 : Math.min(TIP_POS_MAX, Math.max(TIP_POS_MIN, v));
  return { x: fit(pos.x), y: fit(pos.y) };
}

/**
 * 1本の先端に、完成形での位置を書き込む。nullを渡すと「指定なし」へ戻す。
 * 位置は範囲内へ収めてから書き込む。
 */
export function setTipPos(
  s: Skeleton,
  id: number,
  pos: TipPos2d | null,
): Skeleton {
  return {
    nodes: s.nodes.map((n) => {
      if (n.id !== id) return n;
      if (pos === null) {
        const cleared: SkeletonNode = { ...n };
        delete cleared.tip_pos_2d;
        return cleared;
      }
      return { ...n, tip_pos_2d: clampTipPos(pos) };
    }),
  };
}

/**
 * 完成形での位置が指定されている先端を、IDの小さい順に取り出す。
 * 指定のない先端と、先へ枝を足して先端でなくなった節点は入らない
 * (Rust側の`Skeleton::leaf_tip_positions`と同じ)。
 */
export function leafTipPositions(
  s: Skeleton,
): { id: number; pos: TipPos2d }[] {
  return leafNodes(s)
    .flatMap((n) =>
      n.tip_pos_2d ? [{ id: n.id, pos: n.tip_pos_2d }] : [],
    )
    .sort((a, b) => a.id - b.id);
}

/** プレビュー1本分: 親側の始点から先端までの線と表示名 */
export interface LimbLayout {
  id: number;
  start: Vec2;
  end: Vec2;
  /** 先端の丸(=角の膨らみ)の半径。長さ×太さ係数 */
  radius: number;
  label: string;
}

/**
 * 骨格の2Dプレビュー配置。胴のすぐ先は真上から時計回りに等間隔で置き、
 * その先は親の終点から外向きに伸ばす。分岐したときだけ扇状に開く。
 * 座標は骨格の長さの単位そのまま(表示側で紙の枠に収める)。
 */
export function previewLayout(s: Skeleton): LimbLayout[] {
  const root = rootNode(s);
  if (!root) return [];

  const children = childrenByParent(s);
  const rows = skeletonRows(s);
  const positions = new Map<number, Vec2>([[root.id, [0, 0]]]);
  const angles = new Map<number, number>();

  return rows.map(({ node, label }) => {
    const parentId = node.parent ?? root.id;
    const start = positions.get(parentId) ?? ([0, 0] as Vec2);
    const siblings = children.get(parentId) ?? [];
    const index = siblings.indexOf(node);
    let angle: number;

    if (parentId === root.id) {
      angle = Math.PI / 2 - (2 * Math.PI * index) / siblings.length;
    } else {
      const parentAngle = angles.get(parentId) ?? Math.PI / 2;
      if (siblings.length <= 1) {
        angle = parentAngle;
      } else {
        const spread = Math.min(
          (2 * Math.PI) / 3,
          ((siblings.length - 1) * Math.PI) / 6,
        );
        angle = parentAngle + spread / 2 - (spread * index) / (siblings.length - 1);
      }
    }

    const end: Vec2 = [
      start[0] + node.length * Math.cos(angle),
      start[1] + node.length * Math.sin(angle),
    ];
    positions.set(node.id, end);
    angles.set(node.id, angle);
    return {
      id: node.id,
      start,
      end,
      radius: node.length * node.width_factor,
      label,
    };
  });
}
