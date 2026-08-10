//! 川崎「1分ローズ」の受入試験と同じ折りから、区切りfixtureを再生成する。
//!
//! 実行:
//! cargo run -p ori3-export --bin rose_fixtures

#[cfg(not(test))]
use std::path::PathBuf;

#[cfg(not(test))]
use ori3_export::{SoftGeometrySnapshot, save_document, save_document_with_soft_geometry};

#[cfg(not(test))]
#[path = "../../../ori3-layers/tests/acceptance_rose.rs"]
mod acceptance_rose;

#[cfg(not(test))]
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ori3-layers/tests/fixtures")
        .join(name)
}

#[cfg(not(test))]
fn main() -> Result<(), String> {
    let (step11, step21, step29, frame29) = acceptance_rose::rose_checkpoint_artifacts();
    let path11 = fixture_path("rose-011.ori3");
    let path21 = fixture_path("rose-021.ori3");
    let path29 = fixture_path("rose-029.ori3");

    save_document(&step11, &path11)?;
    println!("保存: {}", path11.display());
    save_document(&step21, &path21)?;
    println!("保存: {}", path21.display());
    save_document_with_soft_geometry(
        &step29,
        &SoftGeometrySnapshot {
            book_step: 29,
            frame: frame29,
        },
        &path29,
    )?;
    println!("保存: {}", path29.display());
    Ok(())
}

#[cfg(test)]
fn main() {}
