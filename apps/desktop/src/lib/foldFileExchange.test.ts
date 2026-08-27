import { describe, expect, it } from "vitest";
import {
  FOLD_FILE_EXCHANGE_READY,
  OPEN_FILE_FILTERS,
  OPEN_FILE_TOOLTIP,
  SAVE_FILE_FILTERS,
  openFileFiltersForReadiness,
  openFileTooltipForReadiness,
} from "./foldFileExchange";

describe("ほかの折り紙ソフトのファイルを既存導線へ混ぜる", () => {
  it("接続完了後は開く対象と安全な案内へ加える", () => {
    expect(FOLD_FILE_EXCHANGE_READY).toBe(true);
    expect(OPEN_FILE_FILTERS).toEqual([
      { name: "ORIGAMI3作品", extensions: ["ori3"] },
      { name: "ほかの折り紙ソフトのファイル", extensions: ["fold"] },
    ]);
    expect(OPEN_FILE_TOOLTIP).toBe(
      "保存した作品または、ほかの折り紙ソフトのファイルを開きます",
    );

    const displayed = [
      ...OPEN_FILE_FILTERS.map((filter) => filter.name),
      OPEN_FILE_TOOLTIP,
    ].join(" ");
    for (const internalTerm of [
      "FOLD 1.1",
      "FOLD 1.2",
      "パーサ",
      "スキーマ",
      "バリデータ",
    ]) {
      expect(displayed).not.toContain(internalTerm);
    }
  });

  it("readiness helperは両状態を保ち、通常保存はori3のままにする", () => {
    expect(openFileFiltersForReadiness(false)).toEqual([
      { name: "ORIGAMI3作品", extensions: ["ori3"] },
    ]);
    expect(openFileFiltersForReadiness(true)).toEqual([
      { name: "ORIGAMI3作品", extensions: ["ori3"] },
      { name: "ほかの折り紙ソフトのファイル", extensions: ["fold"] },
    ]);
    expect(openFileTooltipForReadiness(false)).toBe(
      "保存した作品(.ori3)を開きます",
    );
    expect(openFileTooltipForReadiness(true)).toBe(
      "保存した作品または、ほかの折り紙ソフトのファイルを開きます",
    );
    expect(SAVE_FILE_FILTERS).toEqual([
      { name: "ORIGAMI3作品", extensions: ["ori3"] },
    ]);
  });
});
