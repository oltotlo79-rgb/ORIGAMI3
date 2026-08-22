import { describe, expect, it } from "vitest";
import {
  CLOSURE_TEAR_LIMIT,
  DEVIATION_NOTICE_EPS_DEG,
  foldAngleDelta,
  foldDeviationNotice,
  foldDeviations,
  isSettledFold,
  keptFoldsFailed,
  MAX_PIN_RELEASES_ON_SETTLE,
  MAX_PIN_SOLVES_WHILE_MOVING,
  PIN_CONFLICT_EPS_DEG,
  PIN_RELEASE_NOTICE_EPS_DEG,
  PIN_RELEASE_ORDER,
  pinReleaseCandidates,
  pinReleaseNotice,
  releasedPins,
  splitKeptFolds,
  splitSettledFolds,
  withFoldDeviationNotice,
} from "./settledFolds";
import { RELAX_NOTICE_EPS_DEG } from "../store/appStore";
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

describe("splitKeptFolds(固定した折り目)", () => {
  it("利用者が固定した折り目は、途中の角度でも動かさない側へ入れる", () => {
    const preferred = [driver(5, 180), driver(6, 45), driver(7, -12.5)];
    const { kept, rest } = splitKeptFolds(preferred, new Set([6]), new Set());
    // 折り切った5と、固定した6が動かさない側。7は従来どおり希望のまま。
    expect(kept.map((d) => d.hinge)).toEqual([5, 6]);
    expect(rest.map((d) => d.hinge)).toEqual([7]);
  });

  it("外した折り目は、固定していても守る側へ戻す", () => {
    const preferred = [driver(5, 180), driver(6, 45)];
    const { kept, rest } = splitKeptFolds(
      preferred,
      new Set([6]),
      new Set([6]),
    );
    expect(kept.map((d) => d.hinge)).toEqual([5]);
    expect(rest.map((d) => d.hinge)).toEqual([6]);
  });

  it("外した折り目が折り切りでも、守る側へ戻す", () => {
    const preferred = [driver(5, 180), driver(6, 45)];
    const { kept, rest } = splitKeptFolds(preferred, new Set(), new Set([5]));
    expect(kept).toEqual([]);
    expect(rest.map((d) => d.hinge)).toEqual([5, 6]);
  });

  it("固定が無ければ、これまでの折り切りの分け方と同じになる", () => {
    const preferred = [driver(5, 180), driver(6, 45), driver(7, 0)];
    const { kept, rest } = splitKeptFolds(preferred, new Set(), new Set());
    const old = splitSettledFolds(preferred);
    expect(kept).toEqual(old.settled);
    expect(rest).toEqual(old.rest);
  });
});

describe("foldAngleDelta", () => {
  it("そのままの差を返す", () => {
    expect(foldAngleDelta(-145, -178.265)).toBeCloseTo(33.265, 10);
    expect(foldAngleDelta(45, 45)).toBe(0);
  });

  it("±180度をまたぐときは近いほうの回り方で測る", () => {
    // 山折り180度と谷折り-180度は同じ折り切りなので、359.9度ではなく0.1度差。
    expect(Math.abs(foldAngleDelta(-179.95, 180))).toBeCloseTo(0.05, 10);
    expect(Math.abs(foldAngleDelta(179.95, -180))).toBeCloseTo(0.05, 10);
  });
});

describe("pinReleaseCandidates(外す順番)", () => {
  const kept = [
    driver(1, 45), // 固定・途中の角度
    driver(2, 90), // 固定・途中の角度
    driver(3, 180), // 折り切り・固定していない(自動)
    driver(4, 0), // 折り切り・利用者が固定
  ];
  const pinned = new Set([1, 2, 4]);

  it("途中の角度の固定 → 自動の折り切り → 固定した折り切り の順に外す", () => {
    const actual = new Map([
      [1, 55], // ずれ10
      [2, 110], // ずれ20
      [3, 150], // ずれ30
      [4, 40], // ずれ40
    ]);
    const out = pinReleaseCandidates(kept, pinned, actual);
    expect(out.map((c) => c.hinge)).toEqual([2, 1, 3, 4]);
    expect(out.map((c) => c.order)).toEqual([
      PIN_RELEASE_ORDER.pinned,
      PIN_RELEASE_ORDER.pinned,
      PIN_RELEASE_ORDER.autoSettled,
      PIN_RELEASE_ORDER.pinnedSettled,
    ]);
  });

  it("同じ群ではずれの大きい順、同じずれなら辺IDの小さい順(毎回同じ順になる)", () => {
    const actual = new Map([
      [1, 55],
      [2, 100],
      [3, 180],
      [4, 0],
    ]);
    const first = pinReleaseCandidates(kept, pinned, actual);
    expect(first.map((c) => c.hinge)).toEqual([1, 2]);
    // 同じ入力を10回:並びが1度も変わらない
    for (let i = 0; i < 10; i++) {
      expect(pinReleaseCandidates(kept, pinned, actual).map((c) => c.hinge)).toEqual(
        first.map((c) => c.hinge),
      );
    }
    const tie = new Map([
      [2, 100],
      [1, 55],
    ]);
    expect(
      pinReleaseCandidates([driver(2, 90), driver(1, 45)], pinned, tie).map(
        (c) => c.hinge,
      ),
    ).toEqual([1, 2]);
  });

  it("動いていない折り目は候補にしない(外しても解けるようにならないため)", () => {
    const actual = new Map([
      [1, 45],
      [2, 90],
      [3, 180],
      [4, 0],
    ]);
    expect(pinReleaseCandidates(kept, pinned, actual)).toEqual([]);
  });

  it("しきい値まわりのずれを取り違えない", () => {
    // しきい値ちょうどの値は、45+1e-6 のような足し算では丸めで下側にも上側にも
    // 転ぶ。境目そのものではなく、10分の1と10倍で確かめる。
    const below = new Map([[1, 45 + PIN_CONFLICT_EPS_DEG / 10]]);
    expect(pinReleaseCandidates([driver(1, 45)], pinned, below)).toEqual([]);
    const above = new Map([[1, 45 + PIN_CONFLICT_EPS_DEG * 10]]);
    expect(
      pinReleaseCandidates([driver(1, 45)], pinned, above).map((c) => c.hinge),
    ).toEqual([1]);
  });

  it("角度が返ってこない折り目・有限でない角度は候補にしない", () => {
    expect(pinReleaseCandidates(kept, pinned, new Map())).toEqual([]);
    expect(
      pinReleaseCandidates(kept, pinned, new Map([[1, Number.NaN]])),
    ).toEqual([]);
  });
});

describe("releasedPins / pinReleaseNotice", () => {
  it("外しても動かなかった折り目は知らせない", () => {
    const out = releasedPins([driver(1, 45)], new Map([[1, 45.05]]));
    expect(out).toEqual([]);
  });

  it("動いた折り目だけを大きい順に返す", () => {
    const out = releasedPins(
      [driver(19, -178.265), driver(31, -178.265), driver(20, 180)],
      new Map([
        [19, -145],
        [31, -160],
        [20, 180],
      ]),
    );
    expect(out.map((p) => p.hinge)).toEqual([19, 31]);
    expect(out[0].deviation).toBeCloseTo(33.265, 3);
  });

  it("知らせる下限は、画面が既に使っている0.1度と同じにする", () => {
    expect(PIN_RELEASE_NOTICE_EPS_DEG).toBe(RELAX_NOTICE_EPS_DEG);
    expect(releasedPins([driver(1, 45)], new Map([[1, 45.1]]))).toHaveLength(1);
    expect(releasedPins([driver(1, 45)], new Map([[1, 45.09]]))).toEqual([]);
  });

  it("該当が無ければ何も出さない", () => {
    expect(pinReleaseNotice([])).toBeNull();
  });

  it("どの折り目がどれだけ動いたかを出す", () => {
    const notice = pinReleaseNotice(
      releasedPins(
        [driver(19, -178.265), driver(31, -178.265)],
        new Map([
          [19, -145],
          [31, -145],
        ]),
      ),
    );
    expect(notice).not.toBeNull();
    expect(notice).toContain("固定した折り目2本");
    expect(notice).toContain("折り目 #19");
    expect(notice).toContain("固定 -178.3°");
    expect(notice).toContain("いま -145.0°");
    expect(notice).toContain("差 33.3°");
    expect(notice).toContain("これらの固定を外すか");
  });

  it("1本のときは言い回しを合わせる", () => {
    const notice = pinReleaseNotice(
      releasedPins([driver(19, 45)], new Map([[19, 80]])),
    );
    expect(notice).toContain("固定した折り目1本");
    expect(notice).toContain("この折り目の固定を外すか");
  });

  it("4本以上は上位3本とほかの本数にまとめる", () => {
    const requested = [1, 2, 3, 4, 5].map((hinge) => driver(hinge, 0));
    const actual = new Map([1, 2, 3, 4, 5].map((hinge) => [hinge, hinge * 10]));
    const notice = pinReleaseNotice(releasedPins(requested, actual));
    expect(notice).toContain("固定した折り目5本");
    expect(notice).toContain("ほか2本");
  });

  it("利用者向けの文に内部用語を出さない", () => {
    const notice = pinReleaseNotice(
      releasedPins([driver(19, 45)], new Map([[19, 80]])),
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
      "拘束",
      "収束",
      "残差",
    ]) {
      expect(notice!.toLowerCase()).not.toContain(word.toLowerCase());
    }
  });
});

describe("keptFoldsFailed(固定したまま折れたかの判定)", () => {
  it("収束していなければ失敗", () => {
    expect(keptFoldsFailed({ converged: false })).toBe(true);
    expect(keptFoldsFailed({})).toBe(true);
  });

  it("収束していて紙が閉じていれば成功", () => {
    // 実測: 固定したまま解けた形の閉じ残りは 3.58e-16〜1.0e-15。
    expect(keptFoldsFailed({ converged: true, closure_rms: 3.58e-16 })).toBe(false);
    expect(keptFoldsFailed({ converged: true, closure_rms: 1.0e-15 })).toBe(false);
    expect(keptFoldsFailed({ converged: true })).toBe(false);
  });

  it("収束したと報告されても、紙が裂けていれば失敗として扱う", () => {
    // 実測: 固定を欲張って成り立たなくなった形の閉じ残りは 2.86e-1。
    expect(keptFoldsFailed({ converged: true, closure_rms: 2.86e-1 })).toBe(true);
    expect(
      keptFoldsFailed({ converged: true, closure_rms: Number.NaN }),
    ).toBe(true);
  });

  it("しきい値ちょうどは失敗にしない", () => {
    expect(
      keptFoldsFailed({ converged: true, closure_rms: CLOSURE_TEAR_LIMIT }),
    ).toBe(false);
  });
});

describe("解き直しの打ち切り", () => {
  it("回数で打ち切る(時間では打ち切らない)", () => {
    // 時間で打ち切ると、計算機の速さで外れる本数が変わり、同じ操作でも
    // 同じ形にならなくなる。上限は必ず整数の回数で持つ。
    expect(Number.isInteger(MAX_PIN_SOLVES_WHILE_MOVING)).toBe(true);
    expect(Number.isInteger(MAX_PIN_RELEASES_ON_SETTLE)).toBe(true);
    expect(MAX_PIN_SOLVES_WHILE_MOVING).toBeGreaterThanOrEqual(2);
    expect(MAX_PIN_RELEASES_ON_SETTLE).toBeGreaterThanOrEqual(1);
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

  it("+180度と-180度をまたぐ近い角度を大きなずれと誤判定しない", () => {
    const actual = new Map([[9, -179.95]]);
    expect(foldDeviations([driver(9, 180)], actual)).toEqual([]);
  });

  it("+180度と-180度をまたぐずれは近い回り方の差を返す", () => {
    const deviations = foldDeviations(
      [driver(9, 179)],
      new Map([[9, -178]]),
    );
    expect(deviations).toHaveLength(1);
    expect(deviations[0].deviation).toBeCloseTo(3, 12);
    expect(foldDeviationNotice(deviations)).toContain("差 3.0°");
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

describe("withFoldDeviationNotice", () => {
  it("既存の警告を保ち、折り目ごとの指定角度と実際の角度を知らせる", () => {
    const warnings = ["既存の知らせ"];
    const result = withFoldDeviationNotice(
      warnings,
      [driver(19, -178.265), driver(31, 90)],
      new Map([
        [19, -162.505],
        [31, 120],
      ]),
    );

    expect(result).not.toBe(warnings);
    expect(result[0]).toBe("既存の知らせ");
    expect(result[1]).toBe(
      "指定した角度にならなかった折り目が2本あります: " +
        "折り目 #31(指定 90.0° → いま 120.0°、差 30.0°)、" +
        "折り目 #19(指定 -178.3° → いま -162.5°、差 15.8°)。" +
        "ほかの折り目と同時にはその角度にできない形なので、" +
        "紙が裂けないいちばん近い形を表示しています。" +
        "動かしたい折り目以外の指定を減らすと、指定どおりに折れることがあります。",
    );
  });

  it("4本以上はずれの大きい上位3本と残りの本数を知らせる", () => {
    const result = withFoldDeviationNotice(
      [],
      [1, 2, 3, 4, 5].map((hinge) => driver(hinge, hinge)),
      new Map([
        [1, 11],
        [2, 22],
        [3, 33],
        [4, 44],
        [5, 55],
      ]),
    );

    expect(result).toHaveLength(1);
    expect(result[0]).toContain("5本あります");
    expect(result[0]).toContain(
      "折り目 #5(指定 5.0° → いま 55.0°、差 50.0°)",
    );
    expect(result[0]).toContain(
      "折り目 #4(指定 4.0° → いま 44.0°、差 40.0°)",
    );
    expect(result[0]).toContain(
      "折り目 #3(指定 3.0° → いま 33.0°、差 30.0°)",
    );
    expect(result[0]).toContain("ほか2本");
    expect(result[0]).not.toContain("折り目 #2(");
    expect(result[0]).not.toContain("折り目 #1(");
  });

  it("有限でない指定角度と実際の角度は知らせに混ぜない", () => {
    const warnings = ["既存の知らせ"];
    const result = withFoldDeviationNotice(
      warnings,
      [
        driver(1, Number.NaN),
        driver(2, Number.POSITIVE_INFINITY),
        driver(3, 45),
        driver(4, -45),
      ],
      new Map([
        [1, 0],
        [2, 0],
        [3, Number.NaN],
        [4, Number.NEGATIVE_INFINITY],
      ]),
    );

    expect(result).toBe(warnings);
  });

  it("ずれが無ければ警告を増やさず元の配列を返す", () => {
    const warnings = ["既存の知らせ"];
    const result = withFoldDeviationNotice(
      warnings,
      [driver(7, 45)],
      new Map([[7, 45]]),
    );

    expect(result).toBe(warnings);
  });

  it("同じ知らせが既にあれば重複させず元の配列を返す", () => {
    const requested = [driver(7, 45)];
    const actual = new Map([[7, 60]]);
    const notice = foldDeviationNotice(foldDeviations(requested, actual));
    expect(notice).not.toBeNull();
    const warnings = ["既存の知らせ", notice!];

    const result = withFoldDeviationNotice(warnings, requested, actual);

    expect(result).toBe(warnings);
    expect(result.filter((warning) => warning === notice)).toHaveLength(1);
  });
});
