// 左右対称の編集(CPE-010の拡張): 選んだ線の「鏡映の相手」を探す純粋な計算。
// 画面もIPCも触らない。線を引くときだけでなく、消すとき・種類を変えるときにも
// 左右対称を効かせるために使う。
//
// 対称軸は紙の形から決め打ちせず、展開図そのものから見つけたもの
// ([`buildMirrorIndex`]。作品ごとに1回だけ判定してキャッシュされる)を
// 効かせたい順に当てる。見つからなかったときの保険として、線を引くときと同じ
// 「紙の縦の中心線」も最後の候補に加える(折り目がまだ少ない展開図でも効く)。

import { buildMirrorIndex } from "./grabDrive";
import { MIRROR_EPS, isSameSegment, mirrorAxisX, type Segment } from "./mirror";
import { buildSegmentIndex, findSegment, reflectSegment, type MirrorLine } from "./symmetry";
import type { Document, Face } from "./types";

/** 紙の縦の中心線を対称軸として表したもの(線を引くときと同じ軸) */
function paperAxis(doc: Document): MirrorLine {
  return { p: [mirrorAxisX(doc.paper), 0], d: [0, 1] };
}

/**
 * 展開図の全ての辺(輪郭・補助線も含む)の索引。
 * [`buildMirrorIndex`] の索引は「両側に面がある折り線」だけなので、
 * 消す・種類を変える相手を探すにはこちらを使う。
 */
function allEdgeIndex(doc: Document) {
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const items: [number, Segment][] = [];
  for (const e of doc.cp.edges) {
    const a = pos.get(e.v0);
    const b = pos.get(e.v1);
    if (a && b) items.push([e.id, [a, b]]);
  }
  return buildSegmentIndex(items);
}

/**
 * その線と左右対称の位置にある線の辺ID(無ければnull)。
 * 軸の上に乗っている線・折り返しても同じ位置になる線は「相手がいない」とみなす。
 */
export function mirrorEdgeOf(
  doc: Document,
  faces: Face[],
  edgeId: number,
  eps = MIRROR_EPS,
): number | null {
  const ix = allEdgeIndex(doc);
  const seg = ix.items.find(([id]) => id === edgeId)?.[1];
  if (!seg) return null;
  for (const ax of [...buildMirrorIndex(doc, faces).axes, paperAxis(doc)]) {
    const other = reflectSegment(seg, ax);
    if (isSameSegment(seg, other, eps)) continue;
    const found = findSegment(ix, other, eps);
    if (found !== null && found !== edgeId) return found;
  }
  return null;
}

/**
 * 選んだ辺の集合に、それぞれの鏡映の相手を足したもの(重複は1つに)。
 * 相手が見つからない線はその線だけが残る(警告は出さない)。
 */
export function withMirrorEdges(
  doc: Document,
  faces: Face[],
  ids: readonly number[],
  eps = MIRROR_EPS,
): number[] {
  const out = new Set<number>(ids);
  for (const id of ids) {
    const other = mirrorEdgeOf(doc, faces, id, eps);
    if (other !== null) out.add(other);
  }
  return [...out];
}
