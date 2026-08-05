// 手順再生の進み具合の計算(純関数)のテスト。
// 進行・手順の切り替わり・最終手順での停止・長い中断の扱い・再開位置を確かめる。

import { describe, expect, it } from "vitest";
import { advancePlayback, startPlayback, STEP_DURATION_MS } from "./playback";

const TOTAL = 3;

describe("advancePlayback", () => {
  it("再生していないときは何も変えない", () => {
    const state = { step: 1, t: 0.5, playing: false };
    expect(advancePlayback(state, 100, TOTAL)).toBe(state);
  });

  it("1手順ぶんの時間で0から1まで進む", () => {
    const half = advancePlayback(
      { step: 1, t: 0, playing: true },
      STEP_DURATION_MS / 2,
      TOTAL,
    );
    expect(half).toEqual({ step: 1, t: 0.5, playing: true });
  });

  it("手順を折り終えたら次の手順の頭から続ける", () => {
    const next = advancePlayback(
      { step: 1, t: 0.9, playing: true },
      STEP_DURATION_MS * 0.2,
      TOTAL,
    );
    expect(next.step).toBe(2);
    expect(next.t).toBeCloseTo(0.1, 10);
    expect(next.playing).toBe(true);
  });

  it("最終手順を折り終えたら止まる", () => {
    const end = advancePlayback(
      { step: TOTAL, t: 0.9, playing: true },
      STEP_DURATION_MS,
      TOTAL,
    );
    expect(end).toEqual({ step: TOTAL, t: 1, playing: false });
  });

  it("最終手順を折り終えた状態からは進まない", () => {
    const end = advancePlayback({ step: TOTAL, t: 1, playing: true }, 16, TOTAL);
    expect(end).toEqual({ step: TOTAL, t: 1, playing: false });
  });

  it("長く止まっていても一度に進むのは1手順ぶんまで", () => {
    const next = advancePlayback(
      { step: 0, t: 1, playing: true },
      10_000,
      TOTAL,
    );
    expect(next).toEqual({ step: 1, t: 1, playing: true });
  });

  it("最終手順で長く止まっていても、飛び越さずに最後で終わる", () => {
    const next = advancePlayback(
      { step: TOTAL - 1, t: 1, playing: true },
      10_000,
      TOTAL,
    );
    expect(next).toEqual({ step: TOTAL, t: 1, playing: false });
  });

  it("経過時間が負でも巻き戻らない", () => {
    const next = advancePlayback({ step: 2, t: 0.4, playing: true }, -50, TOTAL);
    expect(next).toEqual({ step: 2, t: 0.4, playing: true });
  });

  it("経過時間が数として壊れていても進み具合を壊さない", () => {
    const state = { step: 2, t: 0.4, playing: true };
    expect(advancePlayback(state, Number.NaN, TOTAL)).toEqual(state);
    expect(advancePlayback(state, Number.POSITIVE_INFINITY, TOTAL)).toEqual(
      state,
    );
  });

  it("手順が1つも無ければ止まる", () => {
    const next = advancePlayback({ step: 0, t: 0, playing: true }, 16, 0);
    expect(next).toEqual({ step: 0, t: 1, playing: false });
  });
});

describe("startPlayback", () => {
  it("最新表示からは折る前に戻して最初から再生する", () => {
    expect(startPlayback(null, 1, TOTAL)).toEqual({
      step: 0,
      t: 1,
      playing: true,
    });
  });

  it("最後まで見終わった状態からも最初から再生する", () => {
    expect(startPlayback(TOTAL, 1, TOTAL)).toEqual({
      step: 0,
      t: 1,
      playing: true,
    });
  });

  it("途中で一時停止していたら、その続きから再開する", () => {
    expect(startPlayback(2, 0.25, TOTAL)).toEqual({
      step: 2,
      t: 0.25,
      playing: true,
    });
  });

  it("手順が1つも無ければ再生しない", () => {
    expect(startPlayback(null, 1, 0).playing).toBe(false);
  });
});
