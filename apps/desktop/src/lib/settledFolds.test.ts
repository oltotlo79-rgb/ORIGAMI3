import { describe, expect, it } from "vitest";
import {
  DEVIATION_NOTICE_EPS_DEG,
  foldDeviationNotice,
  foldDeviations,
  isSettledFold,
  splitSettledFolds,
} from "./settledFolds";
import type { Driver } from "./types";

const driver = (hinge: number, target_angle_deg: number): Driver => ({
  hinge,
  target_angle_deg,
});

describe("isSettledFold", () => {
  it("0度と±180度は折り切っているとみなす", () => {
    for (const deg of [0, 180, -180]) {
      expect(isSettledFold(deg)).toBe(true);
    }
  });

  it("作品ファイルに入る丸め程度のずれは折り切っているとみなす", () => {
    // 実機で採取した値。0度の指定が -3.06e-15 / -5.23e-15 として入っていた。
    for (const deg of [-3.0622045845905385e-15, -5.233885113024099e-15]) {
      expect(isSettledFold(deg)).toBe(true);
    }
    expect(isSettledFold(179.9999999)).toBe(true);
  });

  it("折り切っていない角度は譲れる側に残す", () => {
    // 178.265度は実機の辺19・31の角度。ここを厳密に保つと、利用者が要求した
    // 辺35=-35度へ到達できなくなる(紙が閉じない)。
    for (const deg of [-178.265130385534_97, -35, -17.5, 1, 179, -179.5, 90]) {
      expect(isSettledFold(deg)).toBe(false);
    }
  });

  it("有限でない値は折り切っているとみなさない", () => {
    for (const deg of [Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(isSettledFold(deg)).toBe(false);
    }
  });
});

describe("splitSettledFolds", () => {
  it("実機の20本を、折り切った17本とそれ以外の3本へ分ける", () => {
    const preferred = [
      driver(17, -180),
      driver(18, -180),
      driver(19, -178.26513038553497),
      driver(20, 180),
      driver(21, 180),
      driver(22, -3.0622045845905385e-15),
      driver(23, 180),
      driver(24, 180),
      driver(25, 180),
      driver(26, 180),
      driver(27, -5.233885113024099e-15),
      driver(28, 180),
      driver(29, 180),
      driver(30, 179.99999999999997),
      driver(31, -178.26513038553497),
      driver(32, 180),
      driver(33, 180),
      driver(34, 180),
      driver(35, -35),
      driver(36, -180),
    ];
    const { settled, rest } = splitSettledFolds(preferred);
    expect(settled).toHaveLength(17);
    expect(rest.map((d) => d.hinge)).toEqual([19, 31, 35]);
  });

  it("空の希望は空のまま返す", () => {
    expect(splitSettledFolds([])).toEqual({ settled: [], rest: [] });
  });
});

describe("foldDeviations", () => {
  it("しきい値を超えたずれだけを大きい順に返す", () => {
    const requested = [driver(19, -178.265), driver(31, -178.265), driver(20, 180)];
    const actual = new Map([
      [19, -162.505],
      [31, -160.0],
      [20, 179.9999],
    ]);
    const out = foldDeviations(requested, actual);
    expect(out.map((d) => d.hinge)).toEqual([31, 19]);
    expect(out[0].deviation).toBeCloseTo(18.265, 3);
  });

  it("紙が破れないための譲り合い(実測1.6度以内)では知らせない", () => {
    // 鶴の花弁折りで8本を同時に動かしたときの実測。ここで鳴らすと鳴りっぱなしになる。
    const requested = [1, 2, 3, 4, 5, 6, 7, 8].map((h) => driver(h, 147));
    const actual = new Map([1, 2, 3, 4, 5, 6, 7, 8].map((h) => [h, 147 + 1.6]));
    expect(foldDeviations(requested, actual)).toEqual([]);
  });

  it("角度が返ってこない折り目は数えない", () => {
    expect(foldDeviations([driver(9, 180)], new Map())).toEqual([]);
    expect(
      foldDeviations([driver(9, 180)], new Map([[9, Number.NaN]])),
    ).toEqual([]);
  });

  it("しきい値ちょうどは知らせない", () => {
    const actual = new Map([[1, DEVIATION_NOTICE_EPS_DEG]]);
    expect(foldDeviations([driver(1, 0)], actual)).toEqual([]);
  });
});

describe("foldDeviationNotice", () => {
  it("該当が無ければ何も出さない", () => {
    expect(foldDeviationNotice([])).toBeNull();
  });

  it("折り目の番号と、指定・実際の角度を出す", () => {
    const notice = foldDeviationNotice(
      foldDeviations(
        [driver(19, -178.265), driver(31, -178.265)],
        new Map([
          [19, -162.505],
          [31, -162.486],
        ]),
      ),
    );
    expect(notice).not.toBeNull();
    expect(notice).toContain("折り目 #19");
    expect(notice).toContain("-178.3°");
    expect(notice).toContain("-162.5°");
    expect(notice).toContain("2本");
  });

  it("4本以上は上位3本とほかの本数にまとめる", () => {
    const requested = [1, 2, 3, 4, 5].map((hinge) => driver(hinge, 0));
    const actual = new Map([1, 2, 3, 4, 5].map((hinge) => [hinge, hinge * 10]));
    const notice = foldDeviationNotice(foldDeviations(requested, actual));
    expect(notice).toContain("5本あります");
    expect(notice).toContain("ほか2本");
  });

  it("利用者向けの文に内部用語を出さない", () => {
    const notice = foldDeviationNotice(
      foldDeviations([driver(19, -178.265)], new Map([[19, -162.505]])),
    );
    expect(notice).not.toBeNull();
    for (const word of [
      "hard",
      "preferred",
      "ソルバー",
      "solver",
      "RMS",
      "warm",
      "surface_rank",
      "ヤコビアン",
    ]) {
      expect(notice!.toLowerCase()).not.toContain(word.toLowerCase());
    }
  });
});
