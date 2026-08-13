// たわみの三角形の網(SIM-012)を3D表示用の頂点配列へ組み替える計算。
// Three.jsには依存しない純粋な計算なので、そのまま単体テストできる。
//
// ori3-soft の網は、折り目の上の頂点を隣り合う面どうしで共有している(共有しないと
// 紙が破れて見えるため)。一方、画面側は「層のずらし表示」で面ごとに別の高さへ
// 持ち上げる必要があり、共有したままだと紙が引き伸ばされてしまう。
// そこで表示用に頂点を面ごとに複製する。複製の単位は面なので、面の中では頂点が
// 共有されたまま(法線がなめらかに繋がって紙の丸みが出る)、折り目のところだけ
// 分かれる(折り目は本来くっきり折れているので、これで正しい)。

import type { SoftMesh } from "../../lib/types";
import type { Vec3 } from "../../lib/layerOffset";

/** 面ごとに複製したあとの頂点の並び。網の形が変わったときだけ作り直す */
export interface SoftLayout {
  /** 複製後の頂点数 */
  vertexCount: number;
  /** 複製後の頂点 → 元の網の頂点番号 */
  source: Int32Array;
  /** 複製後の頂点 → その頂点が属する面ID(ずらし量を引くのに使う) */
  faceOf: Int32Array;
  /** 三角形の添字(複製後の番号) */
  indices: number[];
  /** 面の境目の線(2つで1本)。面の中で1回しか使われていない辺=面の輪郭 */
  lineIndices: number[];
  /** 各輪郭辺に接する唯一の三角形の第3頂点。画面上の紙の内向きを決める。 */
  lineProbeIndices: number[];
  /** 三角形の通し番号 → 面ID(当たり判定で使う。網のtriangle_facesと同じ並び) */
  triangleFaceIds: number[];
  /** 三角形の通し番号 → 層番号(surface ownerと当たり判定のtie-breakで使う) */
  triangleLayers: number[];
  /** 表示三角形 → SoftMesh.triangles上の番号(座標更新時にlayerを取り直す)。 */
  triangleSources: number[];
}

/** 辺の鍵。複製後の番号は面ごとに固有なので、この鍵は面をまたいで衝突しない */
const EDGE_STRIDE = 1 << 22;

function edgeKey(a: number, b: number): number {
  return a < b ? a * EDGE_STRIDE + b : b * EDGE_STRIDE + a;
}

/**
 * 網の形が同じかどうかを見分ける短い文字列。
 * 中身が同じ形なら頂点座標を書き換えるだけで済み、組み立て直さずに済む。
 */
export function softSignature(soft: SoftMesh): string {
  return `${soft.positions.length}:${soft.triangles.length}`;
}

/** 面ごとに頂点を複製した並びを作る */
export function buildSoftLayout(soft: SoftMesh): SoftLayout {
  const source: number[] = [];
  const faceOf: number[] = [];
  const indices: number[] = [];
  const triangleFaceIds: number[] = [];
  const triangleLayers: number[] = [];
  const triangleSources: number[] = [];
  // 面ID → (元の頂点番号 → 複製後の番号)
  const perFace = new Map<number, Map<number, number>>();
  const edgeUse = new Map<number, { count: number; probe: number }>();

  for (let t = 0; t < soft.triangles.length; t++) {
    const tri = soft.triangles[t];
    const face = soft.triangle_faces[t];
    if (!tri || face === undefined) continue;
    let table = perFace.get(face);
    if (!table) {
      table = new Map<number, number>();
      perFace.set(face, table);
    }
    const local: number[] = [];
    let broken = false;
    for (const v of tri) {
      if (v < 0 || v >= soft.positions.length) {
        broken = true;
        break;
      }
      let idx = table.get(v);
      if (idx === undefined) {
        idx = source.length;
        source.push(v);
        faceOf.push(face);
        table.set(v, idx);
      }
      local.push(idx);
    }
    // 参照切れの三角形は描かない(網が壊れていても画面は止めない)
    if (broken) continue;
    indices.push(local[0], local[1], local[2]);
    triangleFaceIds.push(face);
    triangleLayers.push(soft.triangle_layers[t] ?? 0);
    triangleSources.push(t);
    for (let i = 0; i < 3; i++) {
      const key = edgeKey(local[i], local[(i + 1) % 3]);
      const used = edgeUse.get(key);
      if (used) used.count += 1;
      else edgeUse.set(key, { count: 1, probe: local[(i + 2) % 3] });
    }
  }

  // 1つの三角形からしか使われていない辺が、その面の輪郭(=境界線)
  const lineIndices: number[] = [];
  const lineProbeIndices: number[] = [];
  for (const [key, use] of edgeUse) {
    if (use.count !== 1) continue;
    lineIndices.push(Math.floor(key / EDGE_STRIDE), key % EDGE_STRIDE);
    lineProbeIndices.push(use.probe);
  }

  return {
    vertexCount: source.length,
    source: Int32Array.from(source),
    faceOf: Int32Array.from(faceOf),
    indices,
    lineIndices,
    lineProbeIndices,
    triangleFaceIds,
    triangleLayers,
    triangleSources,
  };
}

/**
 * 複製後の頂点座標を書き込む。面ごとのずらし量(層の重なりを見せるための
 * 微小な持ち上げ)を足すので、たわみを入れても重なりの見え方は今までどおり。
 */
export function fillSoftPositions(
  soft: SoftMesh,
  layout: SoftLayout,
  lifts: ReadonlyMap<number, Vec3>,
  out: Float32Array,
): void {
  for (let i = 0; i < layout.vertexCount; i++) {
    const p = soft.positions[layout.source[i]];
    const lift = lifts.get(layout.faceOf[i]);
    const k = i * 3;
    if (!p) {
      out[k] = 0;
      out[k + 1] = 0;
      out[k + 2] = 0;
      continue;
    }
    out[k] = p[0] + (lift ? lift[0] : 0);
    out[k + 1] = p[1] + (lift ? lift[1] : 0);
    out[k + 2] = p[2] + (lift ? lift[2] : 0);
  }
}
