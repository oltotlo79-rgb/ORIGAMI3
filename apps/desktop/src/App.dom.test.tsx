// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import { relaxationStatus } from "./App";
import { statusBadgeText, warningCount } from "./lib/flatFoldNotice";

describe("3D右上の自然追従表示(SIM-018)", () => {
  it("0.1度以上では本数と最大偏差を出し、0.099度は表示しない", () => {
    expect(
      relaxationStatus(
        [{ hinge: 5, target_angle_deg: 90, actual_angle_deg: 89.901, delta_deg: -0.099 }],
        false,
      ),
    ).toBeNull();

    expect(
      relaxationStatus(
        [
          { hinge: 5, target_angle_deg: 90, actual_angle_deg: 89.9, delta_deg: -0.1 },
          { hinge: 9, target_angle_deg: 90, actual_angle_deg: 72, delta_deg: -18 },
        ],
        false,
      ),
    ).toBe("前の折り目2本が追従（最大18.0°）");

    // 10進の90.0°と89.9°を実際に引くと、二進浮動小数ではわずかに
    // 0.1°を下回る。この丸め誤差だけで通知を落とさない。
    expect(
      relaxationStatus(
        [
          {
            hinge: 11,
            target_angle_deg: 90,
            actual_angle_deg: 89.9,
            delta_deg: 89.9 - 90,
          },
        ],
        false,
      ),
    ).toBe("前の折り目1本が追従（最大0.1°）");
  });

  it("最良候補では指定を優先して追従中と知らせる", () => {
    expect(relaxationStatus([], true)).toBe("指定を優先し、いちばん近い形で追従中");
  });
});

describe("3D右上の平らに畳めない点の件数", () => {
  it("通常警告2件と4点を警告6件として数える", () => {
    expect(
      warningCount(
        ["通常警告A", "通常警告B"],
        ["通常警告A"],
        [],
        [9, 10, 11, 12],
      ),
    ).toBe(6);
  });

  it("平らに畳めない点は自然追従の表示より優先する", () => {
    expect(
      statusBadgeText({
        hasError: false,
        followStatus: "前の折り目6本が追従（最大89.4°）",
        poseConverged: true,
        warningCount: 4,
        flatFoldViolationCount: 4,
      }),
    ).toBe("警告 4");
  });
});
