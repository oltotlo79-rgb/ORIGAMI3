import {
  FOLD_ISSUE_CODES,
  type FoldIssueCode,
  type FoldIssueSeverity,
} from "./types";

export { FOLD_ISSUE_CODES } from "./types";

export type FoldNoticeDirection = "import" | "export";

export const FOLD_FILE_DISPLAY_NAME = "ほかの折り紙ソフトのファイル";

export const FOLD_OPEN_FILE_GUIDANCE =
  `保存した作品または、${FOLD_FILE_DISPLAY_NAME}を開きます。` +
  "読み込めない内容があるときは、理由をお知らせします";

export const FOLD_FILE_SCOPE_SUMMARY =
  "紙の形、折り目、折る角度、1つずつ順番に並んだ折る手順、" +
  "意味を変えずに扱える紙の重なりを読み込み・書き出しできます。" +
  "扱えない内容があるときは、理由をお知らせします。";

export const FOLD_EXPORT_CHOICE_HINT =
  "紙の形、折り目、折る角度、1つずつ順番に並んだ折る手順、" +
  "意味を変えずに扱える紙の重なりを、対応しているほかの折り紙ソフトで使える形にします。" +
  "書き出せない内容があるときは、理由をお知らせします。";

export const FOLD_UNSUPPORTED_CONTENT_TITLE =
  `${FOLD_FILE_DISPLAY_NAME}でそのまま扱えない内容（7項目）`;

export const FOLD_UNSUPPORTED_CONTENT_ITEMS = [
  "立体になったときの点の位置",
  "途中から複数の流れに分かれる折る手順",
  "動画として記録された動き",
  "名前の付いた折り方が何を意味するか",
  "作品につけたメモや説明",
  "仕上げにつけた丸み",
  "元のファイルで「平らな折り目」と「種類が指定されていない折り目」を区別すること",
] as const;

export type FoldIssueNoticeInput = {
  code: string;
  severity: string;
  message?: string;
  path?: string;
  original_value?: unknown;
};

type NoticeByDirection = Record<FoldNoticeDirection, string>;
type NoticeBySeverity = Partial<Record<FoldIssueSeverity, NoticeByDirection>>;

const NOTICE_BY_CODE = {
  assignment_downgraded_to_aux: {
    warning: {
      import:
        "元のファイルにある折り目の種類の一部は区別して保持できないため、補助線として読み込みました。",
      export: "補助線の一部は、元の種類を区別できない形で書き出しました。",
    },
  },
  unsupported_field: {
    warning: {
      import: "このファイルに含まれる付加情報の一部は読み込まれませんでした。",
      export: "作品固有の表示や説明の一部は書き出されませんでした。",
    },
    error: {
      import:
        "このファイルには、ORIGAMI3で扱えない追加情報または手順が含まれています。",
      export: "この作品には、書き出し先で扱えない追加情報が含まれています。",
    },
  },
  unsupported_geometry: {
    warning: {
      import: "紙の位置・向き・大きさをORIGAMI3に合わせて読み込みました。",
      export: "紙の位置・向き・大きさを調整して書き出しました。",
    },
    error: {
      import:
        "このファイルの紙の形や折った状態は、ORIGAMI3でそのまま扱えません。",
      export:
        "この作品の紙の形や折った状態は、ほかの折り紙ソフトで使える形に書き出せません。",
    },
  },
  non_linear_frames: {
    error: {
      import: "このファイルの折る手順は、1つずつ順番に並んだ形ではありません。",
      export: "この作品の折る手順を、1つずつ順番に並べて書き出せません。",
    },
  },
  unrepresentable_face_orders: {
    error: {
      import: "このファイルの紙の重なり順を、意味を変えずに読み込めません。",
      export: "この作品の紙の重なり順を、意味を変えずに書き出せません。",
    },
  },
  invalid_topology: {
    error: {
      import: "このファイルでは、点・線・面のつながりに矛盾があります。",
      export: "この作品では、点・線・面のつながりに矛盾があるため書き出せません。",
    },
  },
  missing_required_field: {
    error: {
      import: "このファイルには、読み込みに必要な情報がありません。",
      export: "書き出しに必要な情報を作品から作れませんでした。",
    },
  },
  invalid_value: {
    error: {
      import: "このファイルに、読み込めない値があります。",
      export: "この作品に、書き出せない値があります。",
    },
  },
} satisfies Record<FoldIssueCode, NoticeBySeverity>;

const UNKNOWN_NOTICE: NoticeByDirection = {
  import: "このファイルには、そのまま読み込めない内容があります。",
  export: "この作品には、そのまま書き出せない内容があります。",
};

function isFoldIssueCode(code: string): code is FoldIssueCode {
  return (FOLD_ISSUE_CODES as readonly string[]).includes(code);
}

/**
 * rawのmessage/path/valueは参照せず、閉じたcode表だけから利用者向け文を返す。
 * 未知codeや、現在Rustが返さないcode/severityの組合せも内部語の無い予備文へ落とす。
 */
export function foldIssueNotice(
  issue: FoldIssueNoticeInput,
  direction: FoldNoticeDirection,
): string {
  if (!isFoldIssueCode(issue.code)) return UNKNOWN_NOTICE[direction];
  if (issue.severity !== "warning" && issue.severity !== "error") {
    return UNKNOWN_NOTICE[direction];
  }

  const notices: NoticeBySeverity = NOTICE_BY_CODE[issue.code];
  return notices[issue.severity]?.[direction] ?? UNKNOWN_NOTICE[direction];
}
