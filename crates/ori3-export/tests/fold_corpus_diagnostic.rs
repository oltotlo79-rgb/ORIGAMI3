//! 外部corpusの受け入れ診断。**製品を1つも変えずに、読むだけ**で結果を出す。
//!
//! `docs/improvement-roadmap-2026-08-24.md` §12.6-1・2 の「対応範囲内/外」を
//! 記録へ書く前に、実際の製品（`parse_fold_1_2` → `validate_fold_1_2` →
//! `fold_to_document`）が各ファイルをどう扱うかを実測するための道具である。
//!
//! # なぜ通常のテストにしないか
//!
//! §10.7.6「通常テストは記録を読んで照合するだけにする」に従い、この診断は
//! `#[ignore]` にして通常のゲートから外す。出力は標準出力だけで、追跡対象の
//! ファイルを1バイトも書き換えない。環境変数 `ORI3_FOLD_DIAGNOSTIC_DIR` が
//! 無ければ即座に失敗し、既定の対象を勝手に決めない。
//!
//! # 使い方
//!
//! ```powershell
//! $env:ORI3_FOLD_DIAGNOSTIC_DIR = "C:\...\corpus\external"
//! cargo test -p ori3-export --test fold_corpus_diagnostic -- --ignored --nocapture
//! ```
//!
//! 1ファイルにつき次を出す。`observed` 欄へそのまま写せる形にしてある。
//!
//! - `bytes` / `sha256`: ディスク上の生バイトの実測値
//! - `file_spec`: 宣言された版（無ければ `-`）
//! - `stage`: どこで止まったか（`parse` / `convert` / `ok`）
//! - `error` / `warning`: `code @ JSON path` の組（散文は出さない）

#[path = "support/fold_sha256.rs"]
mod fold_sha256;

use fold_sha256::sha256_hex;
use ori3_export::fold::{
    FoldIssue, FoldIssueSeverity, fold_to_document, parse_fold_1_2, unsupported_fields,
    validate_fold_1_2,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const DIRECTORY_VARIABLE: &str = "ORI3_FOLD_DIAGNOSTIC_DIR";

#[test]
#[ignore = "受け入れ診断の道具。対象は ORI3_FOLD_DIAGNOSTIC_DIR で明示する"]
fn report_product_outcome_for_every_fold_file() {
    let directory = std::env::var(DIRECTORY_VARIABLE).unwrap_or_else(|_| {
        panic!("{DIRECTORY_VARIABLE} に診断したいdirectoryの絶対pathを設定してください")
    });
    let root = PathBuf::from(&directory);
    assert!(root.is_dir(), "{DIRECTORY_VARIABLE} がdirectoryではありません: {directory}");

    let mut files = Vec::new();
    collect_fold_files(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "{directory} に .fold がありません");

    println!();
    println!("== 外部corpus 受け入れ診断: {directory} ==");
    println!("対象 {} 件", files.len());

    let mut supported = 0_usize;
    let mut rejected = 0_usize;
    let mut machine_readable = Vec::new();
    for path in &files {
        let name = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let raw = fs::read(path).unwrap_or_else(|error| panic!("{name} を読めません: {error}"));
        let text = String::from_utf8_lossy(&raw).into_owned();

        println!();
        println!("--- {name}");
        println!("    bytes  {}", raw.len());
        println!("    sha256 {}", sha256_hex(&raw));
        println!("    file_spec {}", declared_file_spec(&text));

        let mut record = serde_json::Map::new();
        record.insert("name".to_string(), Value::String(name.clone()));
        record.insert("bytes".to_string(), Value::from(raw.len()));
        record.insert("sha256".to_string(), Value::String(sha256_hex(&raw)));
        record.insert(
            "file_spec".to_string(),
            Value::String(declared_file_spec(&text)),
        );

        match parse_fold_1_2(&text) {
            Err(error) => {
                rejected += 1;
                println!("    stage  parse で拒否");
                println!("    error  {:?} @ {}", error.kind, error.path);
                println!("    message {}", error.message);
                record.insert("stage".to_string(), Value::String("parse".to_string()));
                record.insert("result".to_string(), Value::String("unsupported".to_string()));
                record.insert(
                    "errors".to_string(),
                    Value::Array(vec![issue_value(&format!("{:?}", error.kind), &error.path)]),
                );
                record.insert("warnings".to_string(), Value::Array(Vec::new()));
            }
            Ok(file) => {
                let validation = validate_fold_1_2(&file);
                println!(
                    "    stage  parse 成功 / validate 警告{} 拒否{}",
                    validation.warnings.len(),
                    validation.errors.len()
                );
                print_issues("validate", &validation.warnings, &validation.errors);
                println!("    未対応field {} 件", unsupported_fields(&file).len());

                match fold_to_document(&file) {
                    Ok(import) => {
                        supported += 1;
                        println!(
                            "    stage  convert 成功（頂点{} 折り目{} 手順{}）",
                            import.document.cp.vertices.len(),
                            import.document.cp.edges.len(),
                            import.document.sequence.len()
                        );
                        print_issues("convert", &import.warnings, &[]);
                        record.insert("stage".to_string(), Value::String("convert".to_string()));
                        record
                            .insert("result".to_string(), Value::String("supported".to_string()));
                        record.insert("errors".to_string(), Value::Array(Vec::new()));
                        record.insert("warnings".to_string(), issue_values(&import.warnings));
                    }
                    Err(error) => {
                        rejected += 1;
                        println!("    stage  convert で拒否");
                        print_issues("convert", &error.warnings, &error.errors);
                        record.insert("stage".to_string(), Value::String("convert".to_string()));
                        record.insert(
                            "result".to_string(),
                            Value::String("unsupported".to_string()),
                        );
                        record.insert("errors".to_string(), issue_values(&error.errors));
                        record.insert("warnings".to_string(), issue_values(&error.warnings));
                    }
                }
            }
        }
        machine_readable.push(Value::Object(record));
    }

    println!();
    println!("== 集計 ==");
    println!("    取込成功 {supported} 件 / 拒否 {rejected} 件 / 合計 {} 件", files.len());
    println!("    panic 0 件（この診断が最後まで進んだこと自体が証拠）");

    // 記録へ書き写すときに人が打ち直して取り違えないよう、同じ実測を
    // 1行のJSONでも出す。ファイルは書かない（§10.7.6）。
    println!();
    println!("== 機械可読 ==");
    println!(
        "ORI3_FOLD_DIAGNOSTIC_JSON {}",
        serde_json::to_string(&machine_readable).expect("診断結果をJSONにできる")
    );
}

fn collect_fold_files(directory: &Path, output: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{} を読めません: {error}", directory.display()));
    for entry in entries {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_fold_files(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "fold") {
            output.push(path);
        }
    }
}

fn declared_file_spec(text: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return "(JSONとして読めない)".to_string();
    };
    match value.get("file_spec") {
        None => "(なし)".to_string(),
        Some(value) => value.to_string(),
    }
}

fn print_issues(stage: &str, warnings: &[FoldIssue], errors: &[FoldIssue]) {
    for issue in errors {
        assert_eq!(issue.severity, FoldIssueSeverity::Error, "{stage}: errorの重さ");
        println!("    error  {} @ {}", code_text(issue), issue.path);
    }
    for issue in warnings {
        assert_eq!(
            issue.severity,
            FoldIssueSeverity::Warning,
            "{stage}: warningの重さ"
        );
        println!("    warn   {} @ {}", code_text(issue), issue.path);
    }
}

fn code_text(issue: &FoldIssue) -> String {
    serde_json::to_value(issue.code)
        .expect("issue codeをserialize")
        .as_str()
        .expect("serialized issue codeはstring")
        .to_string()
}

fn issue_values(issues: &[FoldIssue]) -> Value {
    Value::Array(
        issues
            .iter()
            .map(|issue| issue_value(&code_text(issue), &issue.path))
            .collect(),
    )
}

fn issue_value(code: &str, path: &str) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("code".to_string(), Value::String(code.to_string()));
    object.insert("path".to_string(), Value::String(path.to_string()));
    Value::Object(object)
}
