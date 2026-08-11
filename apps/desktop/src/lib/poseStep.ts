// SIM-009「仕上げの角度」: 今つけている立体的な形を、手順(FoldStep)として残す。
//
// 角度スライダーで作った形は画面の一時的な状態でしかなく、展開図を編集すると
// 平らな計算結果へ戻ってしまう。そこで全ての折り線の「今の角度」を
// DriverLine(展開図座標の線分+角度)の列にして、kind="Pose" の手順として
// 手順一覧へ積む。再生(Rust側 replay)は線分から折り線を引き当て直すので、
// 展開図を編集して辺IDが変わっても同じ立体形状に戻る。

import type { Document, DriverLine, FoldStep } from "./types";

/** これ未満の角度は「平ら」とみなす(度)。計算誤差を立体と誤解しないため */
export const POSE_MIN_DEG = 0.5;

/**
 * 今の折り角度(折り線の辺ID → 度)。
 * 追従計算の実角 → 利用者の希望値 → 0度(平ら)の順に決める。
 * 指定していない折り線もソルバーが求めた角度で埋めるので、画面に見えている
 * 形がそのまま手順になる。
 */
export function currentAngles(
  hinges: ReadonlySet<number>,
  drivers: ReadonlyMap<number, number>,
  poseAngles: ReadonlyMap<number, number>,
): Map<number, number> {
  const out = new Map<number, number>();
  for (const hinge of hinges) {
    out.set(hinge, poseAngles.get(hinge) ?? drivers.get(hinge) ?? 0);
  }
  return out;
}

/** 角度のついた折り線が1本でもあるか(平らなら残す意味がない) */
export function hasPoseAngle(angles: ReadonlyMap<number, number>): boolean {
  for (const deg of angles.values()) {
    if (Number.isFinite(deg) && Math.abs(deg) >= POSE_MIN_DEG) return true;
  }
  return false;
}

/**
 * 今の形を表す「仕上げの角度」の手順を組み立てる。
 * 折り線の両端は展開図の頂点座標で書き出す(辺IDは編集で変わるため)。
 * 平坦にならない手順なので layer_order は null(直前の層の重なりを保つ)。
 *
 * 角度は**丸めずにそのまま**書き出す。角度どうしは頂点まわりのループが閉じる
 * 関係で結ばれていて、少しでも丸めるとその関係が崩れ、再生のたびに
 * 「追従計算が収束していません」の警告が出てしまう(ソルバーの収束判定は
 * 残差RMS 1e-13。小数9桁に丸めても桁が足りない)。.ori3はJSONなので
 * f64をそのまま書いてもファイルの大きさはほとんど変わらない。
 */
export function buildPoseStep(
  doc: Document,
  angles: ReadonlyMap<number, number>,
): FoldStep {
  const pos = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const drivers: DriverLine[] = [];
  for (const e of doc.cp.edges) {
    const deg = angles.get(e.id);
    if (deg === undefined || !Number.isFinite(deg)) continue;
    const a = pos.get(e.v0);
    const b = pos.get(e.v1);
    if (!a || !b) continue;
    drivers.push({
      a: [a[0], a[1]],
      b: [b[0], b[1]],
      target_angle_deg: deg,
    });
  }
  return {
    id: nextStepId(doc),
    kind: "Pose",
    drivers,
    layer_order: null,
    note: "",
  };
}

/** 次に使える手順ID(Rust側 next_step_id と同じ決め方) */
export function nextStepId(doc: Document): number {
  return doc.sequence.reduce((max, s) => Math.max(max, s.id + 1), 0);
}
