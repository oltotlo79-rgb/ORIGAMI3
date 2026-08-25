import { describe, expect, it } from "vitest";
import {
  FOLD_FILE_EXCHANGE_READY,
  OPEN_FILE_FILTERS,
  SAVE_FILE_FILTERS,
  openFileFiltersForReadiness,
} from "./foldFileExchange";

describe("ほかの折り紙ソフトのファイルを既存導線へ混ぜる準備", () => {
  it("状態保持と注意表示が未接続の間は、利用者が開く対象へ出さない", () => {
    expect(FOLD_FILE_EXCHANGE_READY).toBe(false);
    expect(OPEN_FILE_FILTERS).toEqual([
      { name: "ORIGAMI3作品", extensions: ["ori3"] },
    ]);
  });

  it("接続後の開く対象にはfoldを足すが、通常保存はori3のままにする", () => {
    expect(openFileFiltersForReadiness(true)).toEqual([
      { name: "ORIGAMI3作品", extensions: ["ori3"] },
      { name: "ほかの折り紙ソフトのファイル", extensions: ["fold"] },
    ]);
    expect(SAVE_FILE_FILTERS).toEqual([
      { name: "ORIGAMI3作品", extensions: ["ori3"] },
    ]);
  });
});
