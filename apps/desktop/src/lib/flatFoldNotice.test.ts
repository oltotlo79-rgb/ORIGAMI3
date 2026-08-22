import { describe, expect, it } from "vitest";
import {
  flatFoldNotice,
  flatFoldViolationIds,
  statusBadgeText,
  warningCount,
} from "./flatFoldNotice";

const NOTICE_SUFFIX =
  "場所は展開図の橙色の丸で確認してください。折り目を足すか、使う折り目を減らすと畳めるようになることがあります。";

describe("平らに畳めない点の利用者向け通知", () => {
  it("1点は1か所と書き、0点では出さない", () => {
    expect(flatFoldNotice([])).toBeNull();
    expect(flatFoldNotice([9])).toBe(
      `この折り方では平らに畳めない点が1か所あります。${NOTICE_SUFFIX}`,
    );
  });

  it("4点をまとめて承認済みの1文にする", () => {
    expect(flatFoldNotice([9, 10, 11, 12])).toBe(
      `この折り方では平らに畳めない点が4か所あります。${NOTICE_SUFFIX}`,
    );
  });

  it("同じ点は重複して数えず、通常警告とは別に点ごとに数える", () => {
    expect(flatFoldViolationIds([12, 9, 12, 10, 11, 9])).toEqual([
      12, 9, 10, 11,
    ]);
    expect(
      warningCount(
        ["通常警告A", "通常警告B"],
        ["通常警告A"],
        [],
        [9, 10, 11, 12, 9],
      ),
    ).toBe(6);
  });

  it("形を正確に出せないときは警告印を重ねず、利用者の言葉で知らせる", () => {
    const text = statusBadgeText({
      hasError: false,
      followStatus: null,
      poseConverged: false,
      warningCount: 0,
      flatFoldViolationCount: 0,
    });
    expect(text).toBe("指定した角度に近い形です");
    expect(text).not.toMatch(/[⚠収束]/u);
    expect(text).not.toContain("追従計算");
  });
});
