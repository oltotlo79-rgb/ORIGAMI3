// 平らに畳めない点の説明文(表示の言葉選びだけを受け持つ)。
//
// 「どの点が畳めないか」はRust側(crates/ori3-cp/src/flatfold.rs)が決め、
// DocumentView.violations で届く。ここではその点をホバーしたときに
// 「山と谷の本数」と「向かい合う角の和」のどちらが合っていないかを言い分ける。
// 判定そのものはやり直さない(違反かどうかを画面側で決めることはない)。

import type { Document } from "./types";

/** 角の和の許容誤差(ラジアン)。Rust側の判定と同じ値 */
const ANGLE_TOL = 1e-6;

/** 山と谷の本数が合わないときの説明 */
export const REASON_COUNTS = "山と谷の本数が合いません";
/** 一周の角が半分ずつに分かれないときの説明 */
export const REASON_ANGLES = "向かい合う角の和が合いません";

/** 違反している点の理由を1行で返す(先頭に「平らに畳めません」を付けた文) */
export function violationReason(doc: Document, vertexId: number): string {
  const byId = new Map(doc.cp.vertices.map((v) => [v.id, v.pos]));
  const center = byId.get(vertexId);
  const reasons: string[] = [];
  if (center) {
    const dirs: number[] = [];
    let mountain = 0;
    let valley = 0;
    for (const e of doc.cp.edges) {
      if (e.kind === "Aux") continue; // 補助線は折りに関与しない
      const other = e.v0 === vertexId ? byId.get(e.v1) : e.v1 === vertexId ? byId.get(e.v0) : null;
      if (!other) continue;
      const d: [number, number] = [other[0] - center[0], other[1] - center[1]];
      if (Math.hypot(d[0], d[1]) < 1e-9) continue;
      dirs.push(Math.atan2(d[1], d[0]));
      if (e.kind === "Mountain") mountain++;
      if (e.kind === "Valley") valley++;
    }
    if (Math.abs(mountain - valley) !== 2) reasons.push(REASON_COUNTS);
    if (!alternatingAnglesHalf(dirs)) reasons.push(REASON_ANGLES);
  }
  if (reasons.length === 0) return "平らに畳めないかもしれません";
  return `平らに畳めません: ${reasons.join("、")}`;
}

/** 一周の角を1つおきに足した和が180°か(川崎定理)。本数が奇数なら分けられない */
function alternatingAnglesHalf(dirs: number[]): boolean {
  if (dirs.length < 2 || dirs.length % 2 !== 0) return false;
  const sorted = [...dirs].sort((a, b) => a - b);
  let alt = 0;
  for (let i = 0; i < sorted.length; i += 2) {
    let gap = sorted[i + 1] - sorted[i];
    if (gap < 0) gap += Math.PI * 2;
    alt += gap;
  }
  return Math.abs(alt - Math.PI) <= ANGLE_TOL;
}
