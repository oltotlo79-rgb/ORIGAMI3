// 左右対称に線を引く計算のテスト(CPE-010)。

import { describe, expect, it } from "vitest";
import {
  isOnMirrorAxis,
  isSameSegment,
  mirrorAxisX,
  mirrorPoint,
  mirrorSegment,
  mirrorSegments,
  type Segment,
} from "./mirror";

describe("左右対称の計算", () => {
  it("正方形の紙の中心線は0.5、横長の紙でも紙の真ん中を通る", () => {
    expect(mirrorAxisX({ width_mm: 150, height_mm: 150 })).toBeCloseTo(0.5);
    expect(mirrorAxisX({ width_mm: 100, height_mm: 200 })).toBeCloseTo(0.25);
    expect(mirrorAxisX({ width_mm: 200, height_mm: 100 })).toBeCloseTo(0.5);
  });

  it("点と線分は中心線をはさんで反対側へ移る(高さは変わらない)", () => {
    expect(mirrorPoint([0.2, 0.7], 0.5)).toEqual([0.8, 0.7]);
    const seg: Segment = [
      [0, 0],
      [0.25, 1],
    ];
    expect(mirrorSegment(seg, 0.5)).toEqual([
      [1, 0],
      [0.75, 1],
    ]);
  });

  it("中心線の上にある線が分かる", () => {
    expect(
      isOnMirrorAxis(
        [
          [0.5, 0],
          [0.5, 1],
        ],
        0.5,
      ),
    ).toBe(true);
    expect(
      isOnMirrorAxis(
        [
          [0.5, 0],
          [0.2, 1],
        ],
        0.5,
      ),
    ).toBe(false);
  });

  it("同じ線分は向きが逆でも同じと分かる", () => {
    const a: Segment = [
      [0.1, 0.2],
      [0.9, 0.8],
    ];
    const b: Segment = [
      [0.9, 0.8],
      [0.1, 0.2],
    ];
    expect(isSameSegment(a, b)).toBe(true);
    expect(
      isSameSegment(a, [
        [0.1, 0.2],
        [0.8, 0.8],
      ]),
    ).toBe(false);
  });

  it("ふつうの線は2本、中心線の上・もともと対称な線は1本だけになる", () => {
    expect(
      mirrorSegments(
        [
          [0.1, 0],
          [0.4, 1],
        ],
        0.5,
      ),
    ).toEqual([
      [
        [0.1, 0],
        [0.4, 1],
      ],
      [
        [0.9, 0],
        [0.6, 1],
      ],
    ]);
    // 中心線に重なる線(縦の折り目)は1本だけ
    expect(
      mirrorSegments(
        [
          [0.5, 0],
          [0.5, 1],
        ],
        0.5,
      ),
    ).toHaveLength(1);
    // 中心線をまたいで左右対称な線も1本だけ
    expect(
      mirrorSegments(
        [
          [0.2, 0.3],
          [0.8, 0.3],
        ],
        0.5,
      ),
    ).toHaveLength(1);
  });
});
