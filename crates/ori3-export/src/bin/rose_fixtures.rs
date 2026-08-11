//! 川崎「1分ローズ」の受入試験と同じ折りから、区切りfixtureを再生成する。
//!
//! 実行:
//! cargo run -p ori3-export --bin rose_fixtures

#[cfg(not(test))]
use std::{fs, path::PathBuf};

#[cfg(not(test))]
use ori3_export::{
    SoftGeometrySnapshot, document_json, document_with_soft_geometry_json,
    save_document_with_soft_geometry,
};

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
    const PRESERVED: [u32; 3] = [11, 21, 29];
    let preserved = PRESERVED
        .map(|book_step| {
            let path = fixture_path(&format!("rose-{book_step:03}.ori3"));
            let bytes = fs::read(&path).map_err(|error| {
                format!("既存fixture「{}」を読めません: {error}", path.display())
            })?;
            Ok((book_step, path, bytes))
        })
        .into_iter()
        .collect::<Result<Vec<_>, String>>()?;

    let artifacts = acceptance_rose::rose_step_artifacts();
    if artifacts.len() != 29
        || !artifacts
            .iter()
            .enumerate()
            .all(|(index, artifact)| artifact.book_step as usize == index + 1)
    {
        return Err("ローズ工程が1〜29の連番になっていません".to_string());
    }

    for artifact in &artifacts {
        if PRESERVED.contains(&artifact.book_step) {
            let (_, path, bytes) = preserved
                .iter()
                .find(|(book_step, _, _)| *book_step == artifact.book_step)
                .expect("保持対象は3件");
            let expected = if artifact.book_step == 29 {
                document_with_soft_geometry_json(
                    &artifact.document,
                    &SoftGeometrySnapshot {
                        book_step: 29,
                        instruction: None,
                        changes_shape: None,
                        frame: artifact.frame.clone(),
                    },
                )?
            } else {
                document_json(&artifact.document)?
            };
            if bytes.as_slice() != expected.as_bytes() {
                return Err(format!(
                    "既存fixture「{}」と再計算した手順{}がバイト一致しません",
                    path.display(),
                    artifact.book_step
                ));
            }
            println!("保持（書込みなし）: {}", path.display());
            continue;
        }

        let path = fixture_path(&format!("rose-{:03}.ori3", artifact.book_step));
        save_document_with_soft_geometry(
            &artifact.document,
            &SoftGeometrySnapshot {
                book_step: artifact.book_step,
                instruction: Some(artifact.instruction.to_string()),
                changes_shape: Some(artifact.changes_shape),
                frame: artifact.frame.clone(),
            },
            &path,
        )?;
        println!("保存: {}", path.display());
    }

    for (_, path, before) in preserved {
        let after = fs::read(&path).map_err(|error| {
            format!(
                "既存fixture「{}」を再読込できません: {error}",
                path.display()
            )
        })?;
        if before != after {
            return Err(format!("既存fixture「{}」が変更されました", path.display()));
        }
    }
    Ok(())
}

#[cfg(test)]
fn main() {}
