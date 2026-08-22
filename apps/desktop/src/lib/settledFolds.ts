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

/**
 * 固定した折り目が動いたことを知らせる最小のずれ(度)。
 *
 * 固定は「そのままでいてほしい」と利用者が明示したものなので、
 * 譲り合いの許容(`DEVIATION_NOTICE_EPS_DEG` = 2.0°)ではなく、
 * 画面が既に「前の折り目が追従した」と知らせている下限と同じ細かさで知らせる。
 * その下限は `store/appStore.ts` の `RELAX_NOTICE_EPS_DEG` = 0.1°(既存の実測に
 * 基づく値)であり、ここでも同じ 0.1° を使う。
 * 循環参照を避けるため値をここへ置き、食い違いは検査で見張る。
 */
export const PIN_RELEASE_NOTICE_EPS_DEG = 0.1;

/**
 * 「この折り目が原因で成り立たない」とみなす最小のずれ(度)。
 *
 * 全部を希望にして解いた結果が、固定した角度からこの値以上ずれている折り目だけを
 * 「外す候補」にする。ずれていない折り目を外しても解けるようにはならないため。
 * 1e-6 は追従計算が診断へ載せる下限(`crates/ori3-rigid/src/solver.rs` の
 * `RELAXATION_EPS_DEG`)と同じで、これより細かい値は そもそも返ってこない。
 */
export const PIN_CONFLICT_EPS_DEG = 1e-6;

/**
 * 紙が裂けたとみなす閉じ残り。これを超える形は表示に使わない。
 *
 * 追従計算が「裂けた」とみなす値(`crates/ori3-rigid/src/motion.rs` の
 * `SEAM_TEAR_TOLERANCE` = 1e-6)と同じ値にする。実測では、
 * 折り目を固定しすぎて成り立たなくなった形の閉じ残りは 2.86e-1、
 * ふつうに解けた形は 3.6e-16〜1.0e-15 だった
 * (`crates/ori3-soft/tests/live2_pose_diagnosis.rs`)。1e-6 はその間で、
 * 解けた形の10桁上・裂けた形の5桁下にある。
 */
export const CLOSURE_TEAR_LIMIT = 1e-6;

/**
 * 固定を外す順番。小さいほど先に外す。
 *
 * 折り切った折り目(0°/±180°)を後ろへ回すのは実測が根拠である。
 * 折り切った折り目が 0.268° 開いただけで、重なった紙の束が 2.473e-3 ばらけ
 * (3D表示が同じ高さとみなせる幅 2.78e-5 の89倍)、本来見えないはずの紙の裏が
 * 最大 38.5% の画素に出た(ファイル冒頭の実測)。途中の角度の折り目が数度動いても
 * 重なり順は幾何から決まるので、そちらを先に譲る。
 */
export const PIN_RELEASE_ORDER = {
  /** 利用者が固定した、折り切っていない折り目。いちばん先に外す。 */
  pinned: 1,
  /** 自動で守っている折り切り(利用者は固定していない)。 */
  autoSettled: 2,
  /** 利用者が固定した折り切り。表示も壊れ、利用者の意思もあるので最後。 */
  pinnedSettled: 3,
} as const;

/**
 * 動かしている最中の1コマで解き直してよい回数。
 *
 * 打ち切りは必ず**回数**で決める。ミリ秒で打ち切ると、速い計算機と遅い計算機で
 * 外れる本数が変わり、同じ操作を繰り返しても同じ形にならなくなる。
 *
 * 2回にする根拠: 面400・辺840の形で、いまと同じ「希望つき」の追従計算1回が
 * release で 8.8〜12.7ms(`crates/ori3-rigid/tests/perf_miura.rs` の計測記録)。
 * 画面は16msごとに要求を出しているので、1コマ2回(最大約25ms)までなら
 * 待ち行列が最新1件へまとまるだけで、操作は止まらない。
 * 外す判断は次のコマへ持ち越すので、成り立たない本数kは約 k/60 秒で落ち着く。
 */
export const MAX_PIN_SOLVES_WHILE_MOVING = 2;

/**
 * 指を離したときに、外す折り目を1本ずつ探してよい回数。
 *
 * 4本で打ち切ると、最悪の解き直しは 1 + 2×4 + 1(まとめて外す) + 1(全部外す)
 * = 11回。上の実測(1回あたり最大12.7ms)から約140msで、指を離した後の1回だけ
 * 起きる。ここでも時間では打ち切らない。
 */
export const MAX_PIN_RELEASES_ON_SETTLE = 4;

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
  const { kept, rest } = splitKeptFolds(preferred, new Set(), new Set());
  return { settled: kept, rest };
}

/**
 * 「動かさない側」と「なるべく守る側」へ分ける。
 *
 * 動かさない側に入るのは、折り切ってある折り目(従来どおり)と、
 * 利用者が固定した折り目である。ただし `released` に入っている折り目は、
 * その角度のままでは紙が裂けると分かったものなので、守る側へ戻す。
 */
export function splitKeptFolds(
  preferred: readonly Driver[],
  pinnedHinges: ReadonlySet<number>,
  released: ReadonlySet<number>,
): { kept: Driver[]; rest: Driver[] } {
  const kept: Driver[] = [];
  const rest: Driver[] = [];
  for (const driver of preferred) {
    const hold =
      !released.has(driver.hinge) &&
      (pinnedHinges.has(driver.hinge) || isSettledFold(driver.target_angle_deg));
    (hold ? kept : rest).push(driver);
  }
  return { kept, rest };
}

/** 指定と実際の角度の差(度)。±180°をまたぐ回り方は近いほうで測る。 */
export function foldAngleDelta(actual: number, target: number): number {
  const raw = actual - target;
  if (raw >= -180 && raw <= 180) return raw;
  const wrapped = (((raw + 180) % 360) + 360) % 360 - 180;
  return wrapped === -180 && raw > 0 ? 180 : wrapped;
}

/** 固定を外す候補1本ぶん。 */
export interface PinReleaseCandidate {
  hinge: number;
  /** 動かさない側へ送っていた角度。 */
  target: number;
  /** 全部を守る側にして解いたときの角度。 */
  actual: number;
  /** その差の大きさ(度)。 */
  deviation: number;
  /** 外す順番の群(`PIN_RELEASE_ORDER`)。 */
  order: number;
}

/**
 * 「全部を守る側にして1回だけ解いた結果」から、外す候補を順番に並べる。
 *
 * 1本ずつ総当たりで試さずに済むのは、追従計算が折り目ごとの譲り量を
 * 既に返しているためである。並び順は
 * 「外す群が先のもの → ずれが大きいもの → 辺IDの小さいもの」で、
 * 同じ入力なら毎回同じ順になる(実行のたびに結果が変わらないため)。
 */
export function pinReleaseCandidates(
  kept: readonly Driver[],
  pinnedHinges: ReadonlySet<number>,
  actual: ReadonlyMap<number, number>,
): PinReleaseCandidate[] {
  const out: PinReleaseCandidate[] = [];
  for (const driver of kept) {
    const got = actual.get(driver.hinge);
    if (got === undefined || !Number.isFinite(got)) continue;
    const deviation = Math.abs(foldAngleDelta(got, driver.target_angle_deg));
    if (!(deviation >= PIN_CONFLICT_EPS_DEG)) continue;
    const pinned = pinnedHinges.has(driver.hinge);
    const settled = isSettledFold(driver.target_angle_deg);
    out.push({
      hinge: driver.hinge,
      target: driver.target_angle_deg,
      actual: got,
      deviation,
      order: settled
        ? pinned
          ? PIN_RELEASE_ORDER.pinnedSettled
          : PIN_RELEASE_ORDER.autoSettled
        : PIN_RELEASE_ORDER.pinned,
    });
  }
  return out.sort(
    (a, b) =>
      a.order - b.order || b.deviation - a.deviation || a.hinge - b.hinge,
  );
}

/** 固定を外した折り目1本ぶんの知らせ。 */
export interface ReleasedPin {
  hinge: number;
  /** 固定していた角度。 */
  pinned: number;
  /** 実際になった角度。 */
  actual: number;
  /** その差の大きさ(度)。 */
  deviation: number;
}

/**
 * 外した折り目のうち、実際に動いたものだけを大きい順に返す。
 * 外しても結局その角度のままだったものは、利用者から見て何も起きていない。
 */
export function releasedPins(
  released: readonly Driver[],
  actual: ReadonlyMap<number, number>,
  epsDeg: number = PIN_RELEASE_NOTICE_EPS_DEG,
): ReleasedPin[] {
  const out: ReleasedPin[] = [];
  for (const driver of released) {
    const got = actual.get(driver.hinge);
    if (got === undefined || !Number.isFinite(got)) continue;
    const deviation = Math.abs(foldAngleDelta(got, driver.target_angle_deg));
    if (!(deviation >= epsDeg)) continue;
    out.push({
      hinge: driver.hinge,
      pinned: driver.target_angle_deg,
      actual: got,
      deviation,
    });
  }
  return out.sort((a, b) => b.deviation - a.deviation || a.hinge - b.hinge);
}

/**
 * 固定した角度のままでは折れなかったことを、内部の用語を使わずに1行で知らせる。
 * 該当が無ければ null。出し口は既存の警告欄と同じで、区画は増やさない。
 */
export function pinReleaseNotice(released: readonly ReleasedPin[]): string | null {
  if (released.length === 0) return null;
  const shown = released.slice(0, NOTICE_LIST_LIMIT);
  const parts = shown.map(
    (pin) =>
      `折り目 #${pin.hinge}(固定 ${pin.pinned.toFixed(1)}° → いま ${pin.actual.toFixed(1)}°、差 ${pin.deviation.toFixed(1)}°)`,
  );
  const hidden = released.length - shown.length;
  if (hidden > 0) parts.push(`ほか${hidden}本`);
  const undoPart =
    released.length === 1 ? "この折り目の固定を外すか" : "これらの固定を外すか";
  return (
    `固定した折り目${released.length}本を、紙が裂けないように動かしました: ${parts.join("、")}。` +
    `${undoPart}、ほかの折り目の角度の指定を減らすと、固定した角度のまま折れることがあります。`
  );
}

/** 1回の追従計算から、固定の成否を判断するために読む値。 */
export interface KeptFoldOutcome {
  converged?: boolean;
  closure_rms?: number;
}

/**
 * 「動かさない側」に入れた折り目のせいで折れなかったか。
 *
 * 収束の報告だけでは足りない。実測では、収束したと報告しながら要求から
 * 17.5° 外していた例と、固定を欲張って紙が 2.86e-1 裂けた例がある
 * (`scratchpad/layer-rank-fix-report.md`)。閉じ残りも一緒に見る。
 */
export function keptFoldsFailed(outcome: KeptFoldOutcome): boolean {
  if (outcome.converged !== true) return true;
  const closure = outcome.closure_rms;
  if (closure === undefined) return false;
  return !Number.isFinite(closure) || closure > CLOSURE_TEAR_LIMIT;
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
    if (
      got === undefined ||
      !Number.isFinite(got) ||
      !Number.isFinite(driver.target_angle_deg)
    ) {
      continue;
    }
    const deviation = Math.abs(foldAngleDelta(got, driver.target_angle_deg));
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
      `折り目 #${d.hinge}(指定 ${d.requested.toFixed(1)}° → いま ${d.actual.toFixed(1)}°、差 ${d.deviation.toFixed(1)}°)`,
  );
  const hidden = deviations.length - shown.length;
  if (hidden > 0) parts.push(`ほか${hidden}本`);
  return (
    `指定した角度にならなかった折り目が${deviations.length}本あります: ${parts.join("、")}。` +
    `ほかの折り目と同時にはその角度にできない形なので、紙が裂けないいちばん近い形を表示しています。` +
    `動かしたい折り目以外の指定を減らすと、指定どおりに折れることがあります。`
  );
}

/**
 * 既存の警告へ、指定と実際の角度のずれを知らせる文を最大1件だけ加える。
 *
 * 姿勢計算・生の手順再生・DocumentView のどの結果にも同じ後処理を適用するための
 * 唯一の入口。角度を比較するだけで、解き直しや自己交差検査は行わない。
 * 追加する文が無い場合と、同じ文が既にある場合は、元の配列をそのまま返す。
 */
export function withFoldDeviationNotice(
  warnings: string[],
  requested: readonly Driver[],
  actual: ReadonlyMap<number, number>,
): string[] {
  const notice = foldDeviationNotice(foldDeviations(requested, actual));
  if (notice === null || warnings.includes(notice)) return warnings;
  return [...warnings, notice];
}
