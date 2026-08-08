use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use ori3_export::manual_pdf_with_stats;

fn usage(program: &OsString) -> String {
    format!(
        "使い方: {} <入力.json> <出力.pdf> [画面写真assetsディレクトリ]",
        Path::new(program).display()
    )
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os();
    let program = arguments
        .next()
        .unwrap_or_else(|| OsString::from("manual_pdf"));
    let input = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    let assets = arguments.next().map(PathBuf::from).unwrap_or_else(|| {
        output
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("assets")
    });
    if arguments.next().is_some() {
        return Err(usage(&program));
    }

    let json = fs::read_to_string(&input)
        .map_err(|error| format!("入力JSON「{}」を読めませんでした: {error}", input.display()))?;
    let (pdf, stats) = manual_pdf_with_stats(&json, &assets)?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "出力ディレクトリ「{}」を作れませんでした: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(&output, pdf)
        .map_err(|error| format!("PDF「{}」を書き出せませんでした: {error}", output.display()))?;
    println!(
        "取扱説明書を生成しました: {} ({}ページ / 目次{}項目)",
        output.display(),
        stats.page_count,
        stats.toc_item_count
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("エラー: {error}");
        std::process::exit(1);
    }
}
