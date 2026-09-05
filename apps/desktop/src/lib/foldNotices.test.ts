import { describe, expect, it } from "vitest";
import noticeContract from "../../../../crates/ori3-export/tests/fixtures/fold/fold-issue-notices.json";
import {
  FOLD_EXPORT_CHOICE_HINT,
  FOLD_FILE_DISPLAY_NAME,
  FOLD_FILE_SCOPE_SUMMARY,
  FOLD_ISSUE_CODES,
  FOLD_OPEN_FILE_GUIDANCE,
  FOLD_UNSUPPORTED_CONTENT_ITEMS,
  FOLD_UNSUPPORTED_CONTENT_TITLE,
  foldIssueNotice,
  type FoldNoticeDirection,
} from "./foldNotices";

const NOTICE_CASES = [
  ["assignment_downgraded_to_aux", "warning", "import", "元のファイルにある折り目の種類の一部は区別して保持できないため、補助線として読み込みました。"],
  ["assignment_downgraded_to_aux", "warning", "export", "補助線の一部は、元の種類を区別できない形で書き出しました。"],
  ["unsupported_field", "warning", "import", "このファイルに含まれる付加情報の一部は読み込まれませんでした。"],
  ["unsupported_field", "warning", "export", "作品固有の表示や説明の一部は書き出されませんでした。"],
  ["unsupported_field", "error", "import", "このファイルには、ORIGAMI3で扱えない追加情報または手順が含まれています。"],
  ["unsupported_field", "error", "export", "この作品には、書き出し先で扱えない追加情報が含まれています。"],
  ["unsupported_geometry", "warning", "import", "紙の位置・向き・大きさをORIGAMI3に合わせて読み込みました。"],
  ["unsupported_geometry", "warning", "export", "紙の位置・向き・大きさを調整して書き出しました。"],
  ["unsupported_geometry", "error", "import", "このファイルの紙の形や折った状態は、ORIGAMI3でそのまま扱えません。"],
  ["unsupported_geometry", "error", "export", "この作品の紙の形や折った状態は、ほかの折り紙ソフトで使える形に書き出せません。"],
  ["non_linear_frames", "error", "import", "このファイルの折る手順は、1つずつ順番に並んだ形ではありません。"],
  ["non_linear_frames", "error", "export", "この作品の折る手順を、1つずつ順番に並べて書き出せません。"],
  ["unrepresentable_face_orders", "error", "import", "このファイルの紙の重なり順を、意味を変えずに読み込めません。"],
  ["unrepresentable_face_orders", "error", "export", "この作品の紙の重なり順を、意味を変えずに書き出せません。"],
  ["invalid_topology", "error", "import", "このファイルでは、点・線・面のつながりに矛盾があります。"],
  ["invalid_topology", "error", "export", "この作品では、点・線・面のつながりに矛盾があるため書き出せません。"],
  ["missing_required_field", "error", "import", "このファイルには、読み込みに必要な情報がありません。"],
  ["missing_required_field", "error", "export", "書き出しに必要な情報を作品から作れませんでした。"],
  ["invalid_value", "error", "import", "このファイルに、読み込めない値があります。"],
  ["invalid_value", "error", "export", "この作品に、書き出せない値があります。"],
] as const;

describe("ほかの折り紙ソフトのファイルに関する注意文", () => {
  it("対応外8項目を平易な固定文で順序どおり保持し、内部用語を含めない", () => {
    expect(FOLD_UNSUPPORTED_CONTENT_ITEMS).toEqual([
      "立体になったときの点の位置",
      "途中から複数の流れに分かれる折る手順",
      "動画として記録された動き",
      "名前の付いた折り方が何を意味するか",
      "作品につけたメモや説明",
      "仕上げにつけた丸み",
      "元のファイルで「平らな折り目」と「種類が指定されていない折り目」を区別すること",
      // 2026-09-05追加。平らな形で終わる手順は書き出せるようになったので、残る範囲だけを示す。
      "まだ平らになっていない途中の形で終わる手順のうち、紙を曲げないと作れないもの",
    ]);
    expect(FOLD_UNSUPPORTED_CONTENT_ITEMS).toHaveLength(8);
    expect(new Set(FOLD_UNSUPPORTED_CONTENT_ITEMS).size).toBe(8);

    const displayed = [
      FOLD_FILE_DISPLAY_NAME,
      FOLD_OPEN_FILE_GUIDANCE,
      FOLD_FILE_SCOPE_SUMMARY,
      FOLD_EXPORT_CHOICE_HINT,
      FOLD_UNSUPPORTED_CONTENT_TITLE,
      ...FOLD_UNSUPPORTED_CONTENT_ITEMS,
    ].join("\n");
    expect(displayed).toContain("ほかの折り紙ソフトのファイル");
    for (const forbidden of [
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
      expect(displayed).not.toContain(forbidden);
    }
  });

  it("Rust側の追跡対象契約と8種類・20文・予備2文が一致する", () => {
    expect(noticeContract.schema).toBe(1);
    expect(noticeContract.notices.map(({ code }) => code)).toEqual(
      FOLD_ISSUE_CODES,
    );

    let translatedCount = 0;
    for (const entry of noticeContract.notices) {
      for (const severity of ["warning", "error"] as const) {
        const translations = entry[severity];
        if (translations === null) continue;
        for (const direction of ["import", "export"] as const) {
          expect(
            foldIssueNotice({ code: entry.code, severity }, direction),
          ).toBe(translations[direction]);
          translatedCount += 1;
        }
      }
    }

    expect(translatedCount).toBe(20);
    expect(
      foldIssueNotice({ code: "future_code", severity: "error" }, "import"),
    ).toBe(noticeContract.unknown.import);
    expect(
      foldIssueNotice({ code: "future_code", severity: "error" }, "export"),
    ).toBe(noticeContract.unknown.export);
  });

  it("Rustが返す8種類のcodeを、固定した20通りの日本語へ漏れなく変換する", () => {
    expect(FOLD_ISSUE_CODES).toEqual([
      "assignment_downgraded_to_aux",
      "unsupported_field",
      "unsupported_geometry",
      "non_linear_frames",
      "unrepresentable_face_orders",
      "invalid_topology",
      "missing_required_field",
      "invalid_value",
    ]);
    expect(NOTICE_CASES).toHaveLength(20);

    for (const [code, severity, direction, expected] of NOTICE_CASES) {
      expect(foldIssueNotice({ code, severity }, direction)).toBe(expected);
    }
  });

  it.each([
    ["import", "このファイルには、そのまま読み込めない内容があります。"],
    ["export", "この作品には、そのまま書き出せない内容があります。"],
  ] as const)("未知のcodeも%s用の安全な予備文へ変換する", (direction, expected) => {
    expect(
      foldIssueNotice(
        { code: "future_code", severity: "future_severity" },
        direction satisfies FoldNoticeDirection,
      ),
    ).toBe(expected);
  });

  it("rawのmessage・path・original_valueと内部用語を利用者向け文へ混ぜない", () => {
    const notice = foldIssueNotice(
      {
        code: "unsupported_field",
        severity: "warning",
        message:
          "faceOrders frame parser validator schema FOLD 1.1 FOLD 1.2 パーサ スキーマ バリデータ Aux",
        path: "$.file_frames[0].frame_parent",
        original_value: { secret: "raw-value" },
      },
      "import",
    );

    for (const forbidden of [
      "faceOrders",
      "frame",
      "parser",
      "validator",
      "schema",
      "FOLD 1.1",
      "FOLD 1.2",
      "パーサ",
      "スキーマ",
      "バリデータ",
      "Aux",
      "file_frames",
      "raw-value",
    ]) {
      expect(notice).not.toContain(forbidden);
    }
  });
});
