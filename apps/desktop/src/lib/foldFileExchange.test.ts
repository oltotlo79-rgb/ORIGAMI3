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
    // 旧「…開きます」→新「…開きます。読み込めない内容…」。8-Dで失敗理由の案内を足した照合で、緩和ではない。
    expect(OPEN_FILE_TOOLTIP).toBe(
      "保存した作品または、ほかの折り紙ソフトのファイルを開きます。読み込めない内容があるときは、理由をお知らせします",
    );

    const displayed = [
      ...OPEN_FILE_FILTERS.map((filter) => filter.name),
      OPEN_FILE_TOOLTIP,
    ].join(" ");
    for (const internalTerm of [
      "FOLD 1.1",
      "FOLD 1.2",
      "parser",
      "schema",
      "validator",
      "パーサ",
      "スキーマ",
      "バリデータ",
      "faceOrders",
      "frame",
      "Aux",
      "JSON path",
      "$.",
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
    // 旧「…開きます」→新「…開きます。読み込めない内容…」。true分岐も製品案内と同じにする意図変更である。
    expect(openFileTooltipForReadiness(true)).toBe(
      "保存した作品または、ほかの折り紙ソフトのファイルを開きます。読み込めない内容があるときは、理由をお知らせします",
    );
    expect(SAVE_FILE_FILTERS).toEqual([
      { name: "ORIGAMI3作品", extensions: ["ori3"] },
    ]);
  });
});
