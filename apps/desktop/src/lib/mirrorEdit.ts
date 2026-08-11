// 対称編集(CPE-010): 選んだ線の「基準線をはさんだ相手」を探す純粋な計算。
// 描画・削除・線種変更が同じ明示的な基準線を使うよう、軸の自動探索は行わない。

import {
  MIRROR_EPS,
  isSameSegment,
  mirrorSegment,
  type MirrorLine,
  type Segment,
} from "./mirror";
import { buildSegmentIndex, findSegment } from "./symmetry";
import type { Document } from "./types";

/** 展開図の全ての辺(輪郭・補助線も含む)の索引。 */
function allEdgeIndex(doc: Document) {
  const pos = new Map(doc.cp.vertices.map((vertex) => [vertex.id, vertex.pos]));
  const items: [number, Segment][] = [];
  for (const edge of doc.cp.edges) {
    const a = pos.get(edge.v0);
    const b = pos.get(edge.v1);
    if (a && b) items.push([edge.id, [a, b]]);
  }
  return buildSegmentIndex(items);
}

/**
 * その線と、指定した基準線をはさんで対称な位置にある線の辺ID(無ければnull)。
 * 基準線の上に乗る線・折り返しても同じ位置になる線は相手なしとみなす。
 */
export function mirrorEdgeOf(
  doc: Document,
  edgeId: number,
  axis: MirrorLine,
  eps = MIRROR_EPS,
): number | null {
  const index = allEdgeIndex(doc);
  const segment = index.items.find(([id]) => id === edgeId)?.[1];
  if (!segment) return null;
  const other = mirrorSegment(segment, axis, eps);
  if (isSameSegment(segment, other, eps)) return null;
  const found = findSegment(index, other, eps);
  return found !== null && found !== edgeId ? found : null;
}

/** 選んだ辺の集合に、それぞれの対称な相手を足す(重複は1つにする)。 */
export function withMirrorEdges(
  doc: Document,
  ids: readonly number[],
  axis: MirrorLine,
  eps = MIRROR_EPS,
): number[] {
  const out = new Set<number>(ids);
  for (const id of ids) {
    const other = mirrorEdgeOf(doc, id, axis, eps);
    if (other !== null) out.add(other);
  }
  return [...out];
}
