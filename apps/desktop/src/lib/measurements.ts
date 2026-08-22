// 測定モードの純粋な数値計算と表示モデル。
// 作品・選択状態は受け取らず、渡された座標から結果を返すだけにする。

import type { Vec2 } from "./types";

export type Vec3 = readonly [number, number, number];
export type Segment2 = readonly [Vec2, Vec2];
export type Segment3 = readonly [Vec3, Vec3];

/**
 * 復元対象にする既約分数の分母上限。方眼の最大1024等分と、角度の4桁小数
 * (分母10000)をどちらも含める。
 */
export const RATIONAL_MAX_DENOMINATOR = 10_000;

/**
 * f64計算で生じた丸めだけを吸収する有理数復元の許容差。
 *
 * 分母がともにQ以下の異なる既約分数の距離は1/Q²以上である。
 * Q=10000では 1/Q²=1e-8。一方、ここで使う両側の許容幅は
 * 2*1e-10=2e-10で、その1/50であるため、許容範囲に入る分数は高々1個。
 * 0/22.5/45/67.5/90度を実際のatan2経路で測った最大誤差は
 * 3.552713678800501e-15度で、許容差はその約2.8万倍の余裕を持つ。
 * 等号は受理せず、この不等式を満たさない引数でも復元しない。
 */
export const RATIONAL_RECOVERY_TOLERANCE = 1e-10;

/** 近似表示で残す小数桁。角度の有限小数判定も同じ4桁まで。 */
export const MEASUREMENT_DECIMAL_PLACES = 4;

const U64_MAX = (1n << 64n) - 1n;
const POLLARD_ATTEMPTS = 32;
const POLLARD_ITERATIONS = 50_000;

export interface Rational {
  numerator: bigint;
  denominator: bigint;
}

export interface RationalRecovery extends Rational {
  /** 入力したf64と復元した分数との差。 */
  error: number;
}

export interface DecimalDisplay {
  text: string;
  /** trueならtextの先頭は必ず「およそ」。 */
  approximate: boolean;
}

export interface MeasurementDisplay {
  primary: string;
  secondary: string | null;
  approximate: boolean;
}

export interface ExactRadical {
  /** coefficient * sqrt(radicand)。radicandは平方因子を持たない。 */
  coefficient: Rational;
  radicand: bigint;
  text: string;
}

export interface ExactLengthMeasurement {
  kind: "exact";
  valueMm: number;
  exact: ExactRadical;
  exactText: string;
  decimal: DecimalDisplay;
  maxRecoveryError: number;
}

export interface ApproxLengthMeasurement {
  kind: "approx";
  valueMm: number;
  decimal: DecimalDisplay;
}

export type LengthMeasurement = ExactLengthMeasurement | ApproxLengthMeasurement;

export interface ExactAngleMeasurement {
  kind: "exact";
  valueDeg: number;
  fraction: Rational;
  fractionText: string;
  decimal: DecimalDisplay;
  /** 4桁以内の有限小数ならdecimal、それ以外(1/3°等)はfraction。 */
  defaultKind: "decimal" | "fraction";
  recoveryError: number;
}

export interface ApproxAngleMeasurement {
  kind: "approx";
  valueDeg: number;
  decimal: DecimalDisplay;
}

export type AngleMeasurement = ExactAngleMeasurement | ApproxAngleMeasurement;

function absBigInt(value: bigint): bigint {
  return value < 0n ? -value : value;
}

function gcdBigInt(a: bigint, b: bigint): bigint {
  let x = absBigInt(a);
  let y = absBigInt(b);
  while (y !== 0n) {
    [x, y] = [y, x % y];
  }
  return x;
}

function rational(numerator: bigint, denominator = 1n): Rational | null {
  if (denominator === 0n) return null;
  const sign = denominator < 0n ? -1n : 1n;
  const numeratorWithSign = numerator * sign;
  const positiveDenominator = denominator * sign;
  const divisor = gcdBigInt(numeratorWithSign, positiveDenominator);
  return {
    numerator: numeratorWithSign / divisor,
    denominator: positiveDenominator / divisor,
  };
}

function sameRational(a: Rational, b: Rational): boolean {
  return a.numerator === b.numerator && a.denominator === b.denominator;
}

function addRational(a: Rational, b: Rational): Rational | null {
  return rational(
    a.numerator * b.denominator + b.numerator * a.denominator,
    a.denominator * b.denominator,
  );
}

function multiplyRational(a: Rational, b: Rational): Rational | null {
  return rational(a.numerator * b.numerator, a.denominator * b.denominator);
}

function squareRational(value: Rational): Rational | null {
  return multiplyRational(value, value);
}

/**
 * f64を、分母上限内で一意に決まる分数へ復元する。
 * 全分母を調べるので連分数の最終半収束を取りこぼさず、候補は最後に残差を再検算する。
 */
export function recoverRational(
  value: number,
  maxDenominator = RATIONAL_MAX_DENOMINATOR,
  tolerance = RATIONAL_RECOVERY_TOLERANCE,
): RationalRecovery | null {
  if (
    !Number.isFinite(value) ||
    !Number.isInteger(maxDenominator) ||
    maxDenominator < 1 ||
    !(tolerance > 0) ||
    !Number.isFinite(tolerance) ||
    !(2 * tolerance < 1 / (maxDenominator * maxDenominator))
  ) {
    return null;
  }

  for (let denominator = 1; denominator <= maxDenominator; denominator += 1) {
    const scaled = value * denominator;
    if (!Number.isSafeInteger(Math.round(scaled))) continue;
    const numerator = Math.round(scaled);
    const candidateValue = numerator / denominator;
    const error = Math.abs(value - candidateValue);
    // 境界ちょうどは、計算機ごとの最下位桁で判定が逆転しないよう受理しない。
    if (!(error < tolerance)) continue;
    const normalized = rational(BigInt(numerator), BigInt(denominator));
    if (!normalized) return null;
    return { ...normalized, error };
  }
  return null;
}

function modPow(base: bigint, exponent: bigint, modulus: bigint): bigint {
  let result = 1n;
  let factor = base % modulus;
  let power = exponent;
  while (power > 0n) {
    if ((power & 1n) === 1n) result = (result * factor) % modulus;
    factor = (factor * factor) % modulus;
    power >>= 1n;
  }
  return result;
}

/** 64bit整数に対して決定的なMiller-Rabin素数判定。 */
function isPrime64(value: bigint): boolean {
  if (value < 2n) return false;
  for (const prime of [2n, 3n, 5n, 7n, 11n, 13n, 17n, 19n, 23n, 29n, 31n, 37n]) {
    if (value === prime) return true;
    if (value % prime === 0n) return false;
  }
  let odd = value - 1n;
  let shifts = 0;
  while ((odd & 1n) === 0n) {
    odd >>= 1n;
    shifts += 1;
  }
  // Jim Sinclairの7底集合。n < 2^64で決定的。
  for (const base of [2n, 325n, 9375n, 28178n, 450775n, 9780504n, 1795265022n]) {
    if (base % value === 0n) continue;
    let witness = modPow(base, odd, value);
    if (witness === 1n || witness === value - 1n) continue;
    let composite = true;
    for (let r = 1; r < shifts; r += 1) {
      witness = (witness * witness) % value;
      if (witness === value - 1n) {
        composite = false;
        break;
      }
    }
    if (composite) return false;
  }
  return true;
}

function pollardFactor(value: bigint): bigint | null {
  if (value % 2n === 0n) return 2n;
  if (value % 3n === 0n) return 3n;
  for (let attempt = 1n; attempt <= BigInt(POLLARD_ATTEMPTS); attempt += 1n) {
    const step = (x: bigint) => (x * x + attempt) % value;
    let slow = 2n;
    let fast = 2n;
    let divisor = 1n;
    for (let iteration = 0; iteration < POLLARD_ITERATIONS && divisor === 1n; iteration += 1) {
      slow = step(slow);
      fast = step(step(fast));
      divisor = gcdBigInt(slow - fast, value);
    }
    if (divisor > 1n && divisor < value) return divisor;
  }
  return null;
}

function factorize64(value: bigint): bigint[] | null {
  if (value < 1n || value > U64_MAX) return null;
  const factors: bigint[] = [];
  const pending: bigint[] = [value];
  while (pending.length > 0) {
    const current = pending.pop() as bigint;
    if (current === 1n) continue;
    if (isPrime64(current)) {
      factors.push(current);
      continue;
    }
    const factor = pollardFactor(current);
    if (!factor) return null;
    pending.push(factor, current / factor);
  }
  return factors.sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
}

function squareFreeParts(value: bigint): { outside: bigint; inside: bigint } | null {
  if (value < 1n) return null;
  const factors = factorize64(value);
  if (!factors) return null;
  let outside = 1n;
  let inside = 1n;
  for (let at = 0; at < factors.length; ) {
    const factor = factors[at];
    let count = 0;
    while (at < factors.length && factors[at] === factor) {
      count += 1;
      at += 1;
    }
    outside *= factor ** BigInt(Math.floor(count / 2));
    if (count % 2 === 1) inside *= factor;
  }
  return { outside, inside };
}

function formatRational(value: Rational): string {
  return value.denominator === 1n
    ? value.numerator.toString()
    : `${value.numerator}/${value.denominator}`;
}

function formatRadical(coefficient: Rational, radicand: bigint): string {
  if (radicand === 1n) return formatRational(coefficient);
  const numerator = coefficient.numerator;
  const root = `√${radicand}`;
  if (coefficient.denominator === 1n) {
    return numerator === 1n ? root : `${numerator}${root}`;
  }
  const top = numerator === 1n ? root : `${numerator}${root}`;
  return `${top}/${coefficient.denominator}`;
}

/** 正の有理数の平方根を、係数×平方因子なし根号へ厳密に簡約する。 */
export function simplifyRationalSquareRoot(value: Rational): ExactRadical | null {
  const normalized = rational(value.numerator, value.denominator);
  if (!normalized || normalized.numerator < 0n) return null;
  if (normalized.numerator === 0n) {
    return {
      coefficient: { numerator: 0n, denominator: 1n },
      radicand: 1n,
      text: "0",
    };
  }
  const numeratorParts = squareFreeParts(normalized.numerator);
  const denominatorParts = squareFreeParts(normalized.denominator);
  if (!numeratorParts || !denominatorParts) return null;
  const coefficient = rational(
    numeratorParts.outside,
    denominatorParts.outside * denominatorParts.inside,
  );
  if (!coefficient) return null;
  const radicand = numeratorParts.inside * denominatorParts.inside;

  // 表示式を二乗して元の有理数へ戻ることを完全一致で確かめる。
  const coefficientSquared = squareRational(coefficient);
  const squared = coefficientSquared
    ? multiplyRational(coefficientSquared, { numerator: radicand, denominator: 1n })
    : null;
  if (!squared || !sameRational(squared, normalized)) return null;
  return { coefficient, radicand, text: formatRadical(coefficient, radicand) };
}

function exactFiniteDecimal(value: Rational, maxPlaces: number): string | null {
  const normalized = rational(value.numerator, value.denominator);
  if (!normalized) return null;
  // 先に許容差内のf64丸め(45.000000000000004等)を有理復元しているため、
  // ここでは既約分母が2と5だけから成り、必要桁数が4以下かを整数演算で判定する。
  // 小数を丸めた結果が一致するかどうかだけでは有限小数と認定しない。
  let denominator = normalized.denominator;
  let twos = 0;
  let fives = 0;
  while (denominator % 2n === 0n) {
    denominator /= 2n;
    twos += 1;
  }
  while (denominator % 5n === 0n) {
    denominator /= 5n;
    fives += 1;
  }
  const places = Math.max(twos, fives);
  if (denominator !== 1n || places > maxPlaces) return null;
  if (places === 0) return normalized.numerator.toString();

  const scale = 10n ** BigInt(places);
  const scaled = (normalized.numerator * scale) / normalized.denominator;
  const negative = scaled < 0n;
  const digits = absBigInt(scaled).toString().padStart(places + 1, "0");
  const integer = digits.slice(0, -places);
  const fraction = digits.slice(-places).replace(/0+$/, "");
  return `${negative ? "-" : ""}${integer}${fraction ? `.${fraction}` : ""}`;
}

function roundedDecimal(value: number): string {
  const rounded = value.toFixed(MEASUREMENT_DECIMAL_PLACES);
  return rounded === "-0.0000" ? "0.0000" : rounded;
}

function approximateDisplay(value: number, unit: "mm" | "°"): DecimalDisplay {
  const spacer = unit === "mm" ? " " : "";
  return {
    text: `およそ ${roundedDecimal(value)}${spacer}${unit}`,
    approximate: true,
  };
}

function exactDecimalDisplay(value: Rational, unit: "mm" | "°"): DecimalDisplay | null {
  const decimal = exactFiniteDecimal(value, MEASUREMENT_DECIMAL_PLACES);
  if (decimal === null) return null;
  const spacer = unit === "mm" ? " " : "";
  return { text: `${decimal}${spacer}${unit}`, approximate: false };
}

function validScale(scaleMm: number): boolean {
  return Number.isFinite(scaleMm) && scaleMm > 0;
}

function exactSquaredDistance(differences: readonly number[]): {
  squared: Rational;
  maxRecoveryError: number;
} | null {
  let squared: Rational = { numerator: 0n, denominator: 1n };
  let maxRecoveryError = 0;
  for (const difference of differences) {
    const recovered = recoverRational(difference);
    if (!recovered) return null;
    const term = squareRational(recovered);
    const sum = term ? addRational(squared, term) : null;
    if (!sum) return null;
    squared = sum;
    maxRecoveryError = Math.max(maxRecoveryError, recovered.error);
  }
  return { squared, maxRecoveryError };
}

function lengthFromDifferences(
  differencesMm: readonly number[],
  allowExact: boolean,
): LengthMeasurement | null {
  if (differencesMm.some((value) => !Number.isFinite(value))) return null;
  const valueMm = Math.hypot(...differencesMm);
  if (!Number.isFinite(valueMm)) return null;
  if (allowExact) {
    const exactSquared = exactSquaredDistance(differencesMm);
    const exact = exactSquared ? simplifyRationalSquareRoot(exactSquared.squared) : null;
    if (exact && exactSquared) {
      const exactText = `${exact.text} mm`;
      const rationalDecimal = exact.radicand === 1n
        ? exactDecimalDisplay(exact.coefficient, "mm")
        : null;
      return {
        kind: "exact",
        valueMm,
        exact,
        exactText,
        decimal: rationalDecimal ?? approximateDisplay(valueMm, "mm"),
        maxRecoveryError: exactSquared.maxRecoveryError,
      };
    }
  }
  return { kind: "approx", valueMm, decimal: approximateDisplay(valueMm, "mm") };
}

/** 展開図上の2点距離。正規化座標の差をmmへ直してから有理復元する。 */
export function measurePlanarDistance(
  first: Vec2,
  second: Vec2,
  paperScaleMm: number,
): LengthMeasurement | null {
  if (!validScale(paperScaleMm)) return null;
  return lengthFromDifferences(
    [
      (second[0] - first[0]) * paperScaleMm,
      (second[1] - first[1]) * paperScaleMm,
    ],
    true,
  );
}

/** 選んだ線分の紙上の長さ。 */
export function measureSegmentLength(
  segment: Segment2,
  paperScaleMm: number,
): LengthMeasurement | null {
  return measurePlanarDistance(segment[0], segment[1], paperScaleMm);
}

/** 立体上の2点距離。3D座標は数値で解いた値なので、平らな姿勢でも常に近似表示。 */
export function measureSpatialDistance(
  first: Vec3,
  second: Vec3,
  paperScaleMm: number,
): LengthMeasurement | null {
  if (!validScale(paperScaleMm)) return null;
  return lengthFromDifferences(
    [
      (second[0] - first[0]) * paperScaleMm,
      (second[1] - first[1]) * paperScaleMm,
      (second[2] - first[2]) * paperScaleMm,
    ],
    false,
  );
}

/** 既に求めた度値を、分数表示と既定表示を持つモデルへ変換する。 */
export function measureAngleDegrees(
  valueDeg: number,
  allowExact = true,
): AngleMeasurement | null {
  if (!Number.isFinite(valueDeg) || valueDeg < 0 || valueDeg > 180) return null;
  const recovered = allowExact ? recoverRational(valueDeg) : null;
  if (!recovered) {
    return { kind: "approx", valueDeg, decimal: approximateDisplay(valueDeg, "°") };
  }
  const fraction: Rational = {
    numerator: recovered.numerator,
    denominator: recovered.denominator,
  };
  const fractionText = `${formatRational(fraction)}°`;
  const exactDecimal = exactDecimalDisplay(fraction, "°");
  return {
    kind: "exact",
    valueDeg,
    fraction,
    fractionText,
    decimal: exactDecimal ?? approximateDisplay(valueDeg, "°"),
    defaultKind: exactDecimal ? "decimal" : "fraction",
    recoveryError: recovered.error,
  };
}

function planarLineAngle(first: Segment2, second: Segment2): number | null {
  const a: Vec2 = [first[1][0] - first[0][0], first[1][1] - first[0][1]];
  const b: Vec2 = [second[1][0] - second[0][0], second[1][1] - second[0][1]];
  const aLength = Math.hypot(a[0], a[1]);
  const bLength = Math.hypot(b[0], b[1]);
  if (!(aLength > 0) || !(bLength > 0)) return null;
  const cross = Math.abs(a[0] * b[1] - a[1] * b[0]);
  const dot = Math.abs(a[0] * b[0] + a[1] * b[1]);
  return (Math.atan2(cross, dot) * 180) / Math.PI;
}

/** 展開図上の、向きを持たない2直線の小さい方の角(0〜90°)。 */
export function measurePlanarAngle(
  first: Segment2,
  second: Segment2,
): AngleMeasurement | null {
  const degrees = planarLineAngle(first, second);
  return degrees === null ? null : measureAngleDegrees(degrees, true);
}

/** 立体上の2直線の小さい方の角。3D座標由来なので常に近似表示。 */
export function measureSpatialAngle(
  first: Segment3,
  second: Segment3,
): AngleMeasurement | null {
  const a: Vec3 = [
    first[1][0] - first[0][0],
    first[1][1] - first[0][1],
    first[1][2] - first[0][2],
  ];
  const b: Vec3 = [
    second[1][0] - second[0][0],
    second[1][1] - second[0][1],
    second[1][2] - second[0][2],
  ];
  const aLength = Math.hypot(...a);
  const bLength = Math.hypot(...b);
  if (!(aLength > 0) || !(bLength > 0)) return null;
  const cross: Vec3 = [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
  const dot = Math.abs(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]);
  const degrees = (Math.atan2(Math.hypot(...cross), dot) * 180) / Math.PI;
  return measureAngleDegrees(degrees, false);
}

export function formatLengthMeasurement(
  measurement: LengthMeasurement,
  mode: "default" | "exact" | "decimal" = "default",
): MeasurementDisplay {
  if (measurement.kind === "approx" || mode === "decimal") {
    return {
      primary: measurement.decimal.text,
      secondary: null,
      approximate: measurement.decimal.approximate,
    };
  }
  return {
    primary: measurement.exactText,
    // 正確な形を選んだときも、値の大きさをすぐ読める小数を必ず併記する。
    // 整数など両者が同じ文字列でも、「併記する」という画面契約を優先して省かない。
    secondary: measurement.decimal.text,
    approximate: false,
  };
}

export function formatAngleMeasurement(
  measurement: AngleMeasurement,
  mode: "default" | "fraction" | "decimal" = "default",
): MeasurementDisplay {
  if (measurement.kind === "approx" || mode === "decimal") {
    return {
      primary: measurement.decimal.text,
      secondary: null,
      approximate: measurement.decimal.approximate,
    };
  }
  const useFraction = mode === "fraction" || measurement.defaultKind === "fraction";
  if (!useFraction) {
    return {
      primary: measurement.decimal.text,
      secondary: null,
      approximate: measurement.decimal.approximate,
    };
  }
  return {
    primary: measurement.fractionText,
    // 度の分数へ切り替えた場合も、小数は必ず同じ結果欄に残す。
    secondary: measurement.decimal.text,
    approximate: false,
  };
}
