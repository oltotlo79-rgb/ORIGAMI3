use std::collections::BTreeSet;
use std::path::PathBuf;

use ori3_export::fold::FoldIssueCode;
use serde::Deserialize;

const CONTRACT_PATH: &str = "tests/fixtures/fold/fold-issue-notices.json";
const FORBIDDEN_USER_TERMS: &[&str] = &[
    "fold 1.2",
    "fold対応",
    "fold完全対応",
    "schema",
    "parser",
    "validator",
    "faceorders",
    "frame",
    "aux",
    "json",
    "path",
    "assignment",
    "field",
    "topology",
    "document",
    "edge",
    "vertex",
    "driver",
    "endpoint",
    "layer_order",
    "profile",
];

#[derive(Debug, Deserialize)]
struct NoticeContract {
    schema: u32,
    unknown: NoticePair,
    notices: Vec<NoticeEntry>,
}

#[derive(Debug, Deserialize)]
struct NoticeEntry {
    code: String,
    warning: Option<NoticePair>,
    error: Option<NoticePair>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct NoticePair {
    import: String,
    export: String,
}

type ExpectedPair = Option<(&'static str, &'static str)>;

macro_rules! fold_issue_notice_contract {
    ($(
        $variant:ident => {
            wire: $wire:literal,
            warning: $warning:expr,
            error: $error:expr
        }
    ),+ $(,)?) => {
        const ALL_CODES: &[FoldIssueCode] = &[$(FoldIssueCode::$variant),+];

        fn expected_notice(
            code: FoldIssueCode,
        ) -> (&'static str, ExpectedPair, ExpectedPair) {
            match code {
                $(FoldIssueCode::$variant => ($wire, $warning, $error)),+
            }
        }
    };
}

fold_issue_notice_contract! {
    AssignmentDowngradedToAux => {
        wire: "assignment_downgraded_to_aux",
        warning: Some((
            "元のファイルにある折り目の種類の一部は区別して保持できないため、補助線として読み込みました。",
            "補助線の一部は、元の種類を区別できない形で書き出しました。",
        )),
        error: None
    },
    UnsupportedField => {
        wire: "unsupported_field",
        warning: Some((
            "このファイルに含まれる付加情報の一部は読み込まれませんでした。",
            "作品固有の表示や説明の一部は書き出されませんでした。",
        )),
        error: Some((
            "このファイルには、ORIGAMI3で扱えない追加情報または手順が含まれています。",
            "この作品には、書き出し先で扱えない追加情報が含まれています。",
        ))
    },
    UnsupportedGeometry => {
        wire: "unsupported_geometry",
        warning: Some((
            "紙の位置・向き・大きさをORIGAMI3に合わせて読み込みました。",
            "紙の位置・向き・大きさを調整して書き出しました。",
        )),
        error: Some((
            "このファイルの紙の形や折った状態は、ORIGAMI3でそのまま扱えません。",
            "この作品の紙の形や折った状態は、ほかの折り紙ソフトで使える形に書き出せません。",
        ))
    },
    NonLinearFrames => {
        wire: "non_linear_frames",
        warning: None,
        error: Some((
            "このファイルの折る手順は、1つずつ順番に並んだ形ではありません。",
            "この作品の折る手順を、1つずつ順番に並べて書き出せません。",
        ))
    },
    UnrepresentableFaceOrders => {
        wire: "unrepresentable_face_orders",
        warning: None,
        error: Some((
            "このファイルの紙の重なり順を、意味を変えずに読み込めません。",
            "この作品の紙の重なり順を、意味を変えずに書き出せません。",
        ))
    },
    InvalidTopology => {
        wire: "invalid_topology",
        warning: None,
        error: Some((
            "このファイルでは、点・線・面のつながりに矛盾があります。",
            "この作品では、点・線・面のつながりに矛盾があるため書き出せません。",
        ))
    },
    MissingRequiredField => {
        wire: "missing_required_field",
        warning: None,
        error: Some((
            "このファイルには、読み込みに必要な情報がありません。",
            "書き出しに必要な情報を作品から作れませんでした。",
        ))
    },
    InvalidValue => {
        wire: "invalid_value",
        warning: None,
        error: Some((
            "このファイルに、読み込めない値があります。",
            "この作品に、書き出せない値があります。",
        ))
    },
}

fn read_contract() -> NoticeContract {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CONTRACT_PATH);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "利用者向け注意文の契約を読めません: {}: {error}",
            path.display()
        )
    });
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("利用者向け注意文の契約がJSONとして不正です: {error}"))
}

fn assert_pair(actual: Option<&NoticePair>, expected: ExpectedPair, label: &str) {
    match (actual, expected) {
        (Some(actual), Some((expected_import, expected_export))) => {
            assert_eq!(actual.import, expected_import, "{label}の読込文が違います");
            assert_eq!(
                actual.export, expected_export,
                "{label}の書出し文が違います"
            );
        }
        (None, None) => {}
        (Some(_), None) => panic!("{label}には到達しないseverityの文があります"),
        (None, Some(_)) => panic!("{label}の文がありません"),
    }
}

fn assert_safe_japanese_notice(text: &str, label: &str) {
    assert!(!text.trim().is_empty(), "{label}が空です");
    assert!(
        text.chars().any(|character| {
            matches!(
                character,
                '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}'
            )
        }),
        "{label}に日本語がありません: {text}"
    );

    let lowercase = text.to_lowercase();
    for forbidden in FORBIDDEN_USER_TERMS {
        assert!(
            !lowercase.contains(forbidden),
            "{label}に内部用語「{forbidden}」があります: {text}"
        );
    }
    for raw_marker in ["$.", "{", "}", "`"] {
        assert!(
            !text.contains(raw_marker),
            "{label}に内部値の目印「{raw_marker}」があります: {text}"
        );
    }
}

#[test]
fn every_fold_issue_code_has_an_exact_safe_notice_contract() {
    let contract = read_contract();
    assert_eq!(contract.schema, 1, "未知の注意文契約schemaです");
    assert_eq!(
        contract.notices.len(),
        ALL_CODES.len(),
        "FoldIssueCode 8種類と注意文を1対1にしてください"
    );

    let mut seen = BTreeSet::new();
    for entry in &contract.notices {
        assert!(
            seen.insert(entry.code.as_str()),
            "codeが重複しています: {}",
            entry.code
        );
    }

    for code in ALL_CODES {
        let (wire, expected_warning, expected_error) = expected_notice(*code);
        assert_eq!(
            serde_json::to_value(code).expect("FoldIssueCodeをwire値へ変換できる"),
            serde_json::Value::String(wire.to_string()),
            "serde wire値が注意文契約と違います"
        );
        let entry = contract
            .notices
            .iter()
            .find(|entry| entry.code == wire)
            .unwrap_or_else(|| panic!("{wire}の注意文がありません"));
        assert_pair(
            entry.warning.as_ref(),
            expected_warning,
            &format!("{wire}/warning"),
        );
        assert_pair(
            entry.error.as_ref(),
            expected_error,
            &format!("{wire}/error"),
        );
    }

    for entry in &contract.notices {
        for (severity, pair) in [
            ("warning", entry.warning.as_ref()),
            ("error", entry.error.as_ref()),
        ] {
            if let Some(pair) = pair {
                assert_safe_japanese_notice(
                    &pair.import,
                    &format!("{}/{severity}/import", entry.code),
                );
                assert_safe_japanese_notice(
                    &pair.export,
                    &format!("{}/{severity}/export", entry.code),
                );
            }
        }
    }
}

#[test]
fn an_unknown_future_code_has_safe_import_and_export_fallbacks() {
    let contract = read_contract();
    assert_eq!(
        contract.unknown,
        NoticePair {
            import: "このファイルには、そのまま読み込めない内容があります。".to_string(),
            export: "この作品には、そのまま書き出せない内容があります。".to_string(),
        }
    );
    assert_safe_japanese_notice(&contract.unknown.import, "unknown/import");
    assert_safe_japanese_notice(&contract.unknown.export, "unknown/export");
}
