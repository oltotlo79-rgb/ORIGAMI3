import { describe, expect, it } from "vitest";
import {
  formatAngleMeasurement,
  formatLengthMeasurement,
  measureAngleDegrees,
  measurePlanarAngle,
  measurePlanarDistance,
  measureSegmentLength,
  measureSpatialAngle,
  measureSpatialDistance,
  RATIONAL_MAX_DENOMINATOR,
  RATIONAL_RECOVERY_TOLERANCE,
  recoverRational,
  simplifyRationalSquareRoot,
} from "./measurements";

describe("有理数の復元", () => {
  it("選んだ許容差では分母上限内の候補が高々1個になる", () => {
    const minimumSeparation = 1 / RATIONAL_MAX_DENOMINATOR ** 2;
    expect(minimumSeparation).toBe(1e-8);
    expect(2 * RATIONAL_RECOVERY_TOLERANCE).toBeLessThan(minimumSeparation);
  });

  it("f64の丸めを含む整数と分数を復元する", () => {
    expect(recoverRational(45 + Number.EPSILON * 32)).toMatchObject({
      numerator: 45n,
      denominator: 1n,
    });
    expect(recoverRational(22.5 + Number.EPSILON * 16)).toMatchObject({
      numerator: 45n,
      denominator: 2n,
    });
    expect(recoverRational(30.125 + Number.EPSILON * 16)).toMatchObject({
      numerator: 241n,
      denominator: 8n,
    });
    expect(recoverRational(1 / 3)).toMatchObject({ numerator: 1n, denominator: 3n });
    expect(recoverRational(1 / 10_000)).toMatchObject({
      numerator: 1n,
      denominator: 10_000n,
    });
  });

  it("分母上限外や正確に復元できない小数を分数にしない", () => {
    expect(recoverRational(1 / 10_001)).toBeNull();
    expect(recoverRational(45 + 2 * RATIONAL_RECOVERY_TOLERANCE)).toBeNull();
    for (const value of [Math.SQRT2, Math.PI, Math.sqrt(2 + Math.sqrt(3))]) {
      expect(recoverRational(value)).toBeNull();
    }
  });

  it("一意性を保証できない設定や有限でない値を受理しない", () => {
    expect(recoverRational(1 / 2, 10_000, 5e-9)).toBeNull();
    const boundaryValue = 45 + 2 ** -34;
    const boundaryError = Math.abs(boundaryValue - 45);
    expect(recoverRational(boundaryValue, 10_000, boundaryError)).toBeNull();
    expect(recoverRational(Number.NaN)).toBeNull();
    expect(recoverRational(Number.POSITIVE_INFINITY)).toBeNull();
  });
});

describe("長さと距離", () => {
  it("有理数の平方根を既約な係数と平方因子のない根号へ簡約する", () => {
    expect(simplifyRationalSquareRoot({ numerator: 450n, denominator: 1n })?.text)
      .toBe("15√2");
    expect(simplifyRationalSquareRoot({ numerator: 8n, denominator: 9n })?.text)
      .toBe("2√2/3");
    expect(simplifyRationalSquareRoot({ numerator: 1n, denominator: 2n })?.text)
      .toBe("√2/2");
    expect(simplifyRationalSquareRoot({ numerator: 72n, denominator: 50n })?.text)
      .toBe("6/5");
  });

  it("150mm正方形の対角線を150√2として小数も添える", () => {
    const measured = measurePlanarDistance([0, 0], [1, 1], 150);
    expect(measured).not.toBeNull();
    expect(measured?.kind).toBe("exact");
    if (!measured || measured.kind !== "exact") return;

    expect(measured.exactText).toBe("150√2 mm");
    expect(measured.valueMm).toBeCloseTo(150 * Math.SQRT2, 12);
    expect(measured.decimal).toEqual({
      text: "およそ 212.1320 mm",
      approximate: true,
    });
    expect(formatLengthMeasurement(measured)).toEqual({
      primary: "150√2 mm",
      secondary: "およそ 212.1320 mm",
      approximate: false,
    });
  });

  it("有理数になる線分の長さも正確に表示する", () => {
    const measured = measureSegmentLength([[0, 0], [3, 4]], 1);
    expect(measured?.kind).toBe("exact");
    if (!measured || measured.kind !== "exact") return;
    expect(measured.exactText).toBe("5 mm");
    expect(measured.decimal).toEqual({ text: "5 mm", approximate: false });
    expect(formatLengthMeasurement(measured).secondary).toBe("5 mm");
  });

  it("座標差を有理数に復元できない3標本は近似のままにする", () => {
    const samples = [Math.SQRT2, Math.PI, Math.sqrt(2 + Math.sqrt(3))];
    for (const x of samples) {
      const measured = measurePlanarDistance([0, 0], [x, 0], 1);
      expect(measured?.kind).toBe("approx");
      expect(measured?.decimal.text.startsWith("およそ ")).toBe(true);
    }
  });

  it("立体距離は平らで整数になる場合も常に近似にする", () => {
    const measured = measureSpatialDistance([0, 0, 0], [3, 4, 0], 1);
    expect(measured).toEqual({
      kind: "approx",
      valueMm: 5,
      decimal: { text: "およそ 5.0000 mm", approximate: true },
    });
  });

  it("平らな状態では展開図と3D図の距離が誤差1e-9未満で一致する", () => {
    const planar = measurePlanarDistance([0.125, 0.25], [0.875, 0.75], 150);
    const spatial = measureSpatialDistance(
      [0.125, 0.25, 0],
      [0.875, 0.75, 0],
      150,
    );
    expect(planar).not.toBeNull();
    expect(spatial).not.toBeNull();
    if (!planar || !spatial) return;
    expect(Math.abs(planar.valueMm - spatial.valueMm)).toBeLessThan(1e-9);
  });

  it("紙尺度または座標が不正なら結果を作らない", () => {
    expect(measurePlanarDistance([0, 0], [1, 1], 0)).toBeNull();
    expect(measurePlanarDistance([0, 0], [Number.NaN, 1], 150)).toBeNull();
    expect(measureSpatialDistance([0, 0, 0], [1, 1, 1], Number.POSITIVE_INFINITY))
      .toBeNull();
  });
});

describe("角度", () => {
  it("手で計算できる5角度を誤差1e-9未満で度の分数へ復元する", () => {
    const cases = [
      { degrees: 0, fraction: "0°" },
      { degrees: 22.5, fraction: "45/2°" },
      { degrees: 45, fraction: "45°" },
      { degrees: 67.5, fraction: "135/2°" },
      { degrees: 90, fraction: "90°" },
    ];
    let maxError = 0;

    for (const { degrees, fraction } of cases) {
      const radians = (degrees * Math.PI) / 180;
      const measured = measurePlanarAngle(
        [[0, 0], [1, 0]],
        [[0, 0], [Math.cos(radians), Math.sin(radians)]],
      );
      expect(measured?.kind).toBe("exact");
      if (!measured || measured.kind !== "exact") continue;
      expect(measured.fractionText).toBe(fraction);
      expect(measured.valueDeg).toBeCloseTo(degrees, 9);
      maxError = Math.max(maxError, measured.recoveryError);
    }

    expect(maxError).toBeLessThan(1e-9);
  });

  it("4桁以内で割り切れる角度は小数を既定にする", () => {
    for (const value of [45, 22.5, 30.125, 0.0625, 0.0001]) {
      const measured = measureAngleDegrees(value);
      expect(measured?.kind).toBe("exact");
      if (!measured || measured.kind !== "exact") continue;
      expect(measured.defaultKind).toBe("decimal");
      expect(measured.decimal.approximate).toBe(false);
      expect(formatAngleMeasurement(measured).primary).toBe(`${value}°`);
    }
  });

  it("1/3度は分数を既定にして近似小数も持つ", () => {
    const measured = measureAngleDegrees(1 / 3);
    expect(measured?.kind).toBe("exact");
    if (!measured || measured.kind !== "exact") return;

    expect(measured.fractionText).toBe("1/3°");
    expect(measured.defaultKind).toBe("fraction");
    expect(formatAngleMeasurement(measured)).toEqual({
      primary: "1/3°",
      secondary: "およそ 0.3333°",
      approximate: false,
    });
  });

  it("100/7度も小数へ丸めず度の分数を既定にする", () => {
    const measured = measureAngleDegrees(100 / 7);
    expect(measured?.kind).toBe("exact");
    if (!measured || measured.kind !== "exact") return;
    expect(measured.fractionText).toBe("100/7°");
    expect(measured.defaultKind).toBe("fraction");
    expect(measured.decimal.text).toBe("およそ 14.2857°");
  });

  it("5桁を要する有限小数は分数、4桁なら小数を既定にする", () => {
    const fourPlaces = measureAngleDegrees(1 / 10_000);
    const fivePlaces = measureAngleDegrees(1 / 32);
    expect(fourPlaces?.kind === "exact" && fourPlaces.defaultKind).toBe("decimal");
    expect(fivePlaces?.kind === "exact" && fivePlaces.defaultKind).toBe("fraction");
    expect(fivePlaces?.kind === "exact" && fivePlaces.fractionText).toBe("1/32°");
  });

  it("分数に復元できない3角度をおよその小数にする", () => {
    const samples = [Math.SQRT2, Math.PI, Math.sqrt(2 + Math.sqrt(3))];
    for (const value of samples) {
      const measured = measureAngleDegrees(value);
      expect(measured?.kind).toBe("approx");
      expect(measured?.decimal.text.startsWith("およそ ")).toBe(true);
    }
  });

  it("立体座標から計算した角度は45度でも常に近似にする", () => {
    const measured = measureSpatialAngle(
      [[0, 0, 0], [1, 0, 0]],
      [[0, 0, 0], [1, 1, 0]],
    );
    expect(measured?.kind).toBe("approx");
    expect(measured?.decimal.text).toBe("およそ 45.0000°");
  });

  it("表示は1回の指定で小数と度の分数を切り替えられる", () => {
    const measured = measureAngleDegrees(22.5 + Number.EPSILON * 16);
    expect(measured?.kind).toBe("exact");
    if (!measured || measured.kind !== "exact") return;

    expect(formatAngleMeasurement(measured, "decimal").primary).toBe("22.5°");
    expect(formatAngleMeasurement(measured, "fraction")).toEqual({
      primary: "45/2°",
      secondary: "22.5°",
      approximate: false,
    });
  });

  it("整数角を度の分数表示にした場合も小数を省かない", () => {
    const measured = measureAngleDegrees(45);
    expect(measured?.kind).toBe("exact");
    if (!measured || measured.kind !== "exact") return;
    expect(formatAngleMeasurement(measured, "fraction")).toEqual({
      primary: "45°",
      secondary: "45°",
      approximate: false,
    });
  });

  it("範囲外または線の長さがゼロなら角度を作らない", () => {
    expect(measureAngleDegrees(-1)).toBeNull();
    expect(measureAngleDegrees(181)).toBeNull();
    expect(measurePlanarAngle([[0, 0], [0, 0]], [[0, 0], [1, 0]])).toBeNull();
  });
});
