// 「もう折り切ってある折り目」を、追従計算で譲らせないための判定と知らせ。
// Three.jsにもRustにも依存しない純粋な計算で、ストアと単体テストから使う。
//
// 背景(実測):
//   追従計算は、いま掴んでいない折り目を全て「なるべく保ちたい希望」として渡す。
//   すると、既に厳密に180°まで畳んである折り目まで 0.03〜0.27° 曲げて帳尻を
//   合わせることがある。実機で採取した姿勢では17本が最大 0.268° 開いており、
//   その結果、重なった紙の束が 2.473e-3 ばらけていた。
//   3D表示が「同じ高さの紙」とみなせる幅は 2.78e-5 なので、その89倍である。
//   束がばらけると重なり順が幾何から決まらなくなり、本来見えないはずの
//   紙の裏が見える(実測 23.7% の画素が裏色になっていた)。
//
//   同じ入力で、折り切ってある17本を「厳密に保つ」側へ回すと:
//     - 17本の誤差 0.268° → 0°
//     - 束のばらけ 2.473e-3 → 8.33e-16
//     - 利用者の視点から見える紙の裏 38.5% → 0.00%
//     - 紙の裂けは 3.58e-16(裂けたとみなす 1e-6 の10桁下)
//   となり、利用者が指定した角度(この例では −35.000°)にも厳密に到達する。
//   測定は `crates/ori3-soft/tests/live2_pose_diagnosis.rs` にある。

import type { Driver } from "./types";

/**
 * 「折り切ってある」とみなす角度のずれ(度)。
 *
 * 折り切った折り目の角度は、作品ファイルにも画面の指定にも厳密な
 * 0° / ±180° として入っている(実測では 0 に対して 3.1e-15 / 5.2e-15 程度の
 * 丸めしか無かった)。途中の角度と取り違えないよう、丸めだけを吸収する幅にする。
 * 例えば 178.265° は「折り切っていない」側に残さなければならない。実測では
 * この折り目を譲れる側に残すことで、利用者の要求どおりの形へ到達できた。
 */
export const SETTLED_ANGLE_EPS_DEG = 1e-6;

/**
 * 利用者へ「指定どおりにならなかった」と知らせる最小のずれ(度)。
 *
 * 折り目どうしはわずかに違う角度を取れることが、紙が破れないために必要である。
 * その譲り合いまで知らせると鳴りっぱなしになるので、下は実測でそこを外す。
 *
 * - 譲り合いの実測: 鶴の花弁折りで8本を同時に動かすと、実際の角度は要求から
 *   **1.6度以内**に収まる(`store/appStore.ts` の `splitDrivers` に残る実測)。
 * - 知らせたい実測: 折り切った折り目を厳密に保つようにしたあと、
 *   利用者の姿勢で指定どおりにならないのは辺19・31 の **33.3度** だけで、
 *   残り18本は **0.000度** だった(`crates/ori3-soft/tests/live2_pose_diagnosis.rs`)。
 *
 * 2.0° は譲り合いの上(1.6°)であり、知らせたい 33.3° の16分の1である。
 */
export const DEVIATION_NOTICE_EPS_DEG = 2.0;

/** 0° または ±180° まで折り切ってある角度か。 */
export function isSettledFold(deg: number): boolean {
  if (!Number.isFinite(deg)) return false;
  const magnitude = Math.abs(deg);
  return (
    magnitude <= SETTLED_ANGLE_EPS_DEG ||
    Math.abs(magnitude - 180) <= SETTLED_ANGLE_EPS_DEG
  );
}

/**
 * 「なるべく保ちたい希望」の一覧を、折り切ってある折り目とそれ以外へ分ける。
 *
 * 折り切ってある側は厳密に保つ指定へ回す。譲らせると、重なった紙が
 * ばらけて重なり順が決まらなくなるため(ファイル冒頭の実測)。
 */
export function splitSettledFolds(preferred: readonly Driver[]): {
  settled: Driver[];
  rest: Driver[];
} {
  const settled: Driver[] = [];
  const rest: Driver[] = [];
  for (const driver of preferred) {
    (isSettledFold(driver.target_angle_deg) ? settled : rest).push(driver);
  }
  return { settled, rest };
}

/** 1本の折り目について、指定と実際の角度のずれ。 */
export interface FoldDeviation {
  hinge: number;
  requested: number;
  actual: number;
  deviation: number;
}

/** 指定した角度と、実際になった角度のずれを大きい順に返す。 */
export function foldDeviations(
  requested: readonly Driver[],
  actual: ReadonlyMap<number, number>,
  epsDeg: number = DEVIATION_NOTICE_EPS_DEG,
): FoldDeviation[] {
  const out: FoldDeviation[] = [];
  for (const driver of requested) {
    const got = actual.get(driver.hinge);
    if (got === undefined || !Number.isFinite(got)) continue;
    const deviation = Math.abs(got - driver.target_angle_deg);
    if (!(deviation > epsDeg)) continue;
    out.push({
      hinge: driver.hinge,
      requested: driver.target_angle_deg,
      actual: got,
      deviation,
    });
  }
  return out.sort((a, b) => b.deviation - a.deviation || a.hinge - b.hinge);
}

/** 一覧に出す折り目の本数。多すぎると1行に収まらないので大きい順に絞る。 */
const NOTICE_LIST_LIMIT = 3;

/**
 * 指定どおりにならなかった折り目を、内部の用語を使わずに1行で知らせる。
 * 該当が無ければ null。
 */
export function foldDeviationNotice(
  deviations: readonly FoldDeviation[],
): string | null {
  if (deviations.length === 0) return null;
  const shown = deviations.slice(0, NOTICE_LIST_LIMIT);
  const parts = shown.map(
    (d) =>
      `折り目 #${d.hinge}(指定 ${d.requested.toFixed(1)}° → いま ${d.actual.toFixed(1)}°)`,
  );
  const hidden = deviations.length - shown.length;
  if (hidden > 0) parts.push(`ほか${hidden}本`);
  return (
    `指定した角度にならなかった折り目が${deviations.length}本あります: ${parts.join("、")}。` +
    `ほかの折り目と同時にはその角度にできない形なので、紙が裂けないいちばん近い形を表示しています。` +
    `動かしたい折り目以外の指定を減らすと、指定どおりに折れることがあります。`
  );
}
