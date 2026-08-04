// ヒンジ(折れる辺)の判定。3Dビュー・角度操作・ストアで同じ定義を使う。
//
// ヒンジ = 山折り/谷折りの辺のうち、ちょうど2つの異なる面に共有される辺。
// Rust側(ori3-rigid::tree::build_forest)の定義に合わせている:
//   - 輪郭(Border)は面が片側にしかないので折れない
//   - 補助線(Aux)は面抽出の対象外なので折れない
//   - 行き止まりの折り線(スリット)は同じ面の境界に2回現れるため折れない

import type { Document, Face } from "./types";

/** 面の境界に現れる辺ID → その辺を境界に持つ面IDの集合 */
function edgeOwners(faces: Face[]): Map<number, Set<number>> {
  const owners = new Map<number, Set<number>>();
  for (const f of faces) {
    for (const edgeId of f.edges) {
      const set = owners.get(edgeId);
      if (set) {
        set.add(f.id);
      } else {
        owners.set(edgeId, new Set([f.id]));
      }
    }
  }
  return owners;
}

/** 折り角度を指定できる辺(ヒンジ)のID集合 */
export function hingeEdgeIds(doc: Document, faces: Face[]): Set<number> {
  const owners = edgeOwners(faces);
  const ids = new Set<number>();
  for (const e of doc.cp.edges) {
    if (e.kind !== "Mountain" && e.kind !== "Valley") continue;
    if (owners.get(e.id)?.size === 2) ids.add(e.id);
  }
  return ids;
}
