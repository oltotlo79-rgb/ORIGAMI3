//! ORIGAMI3作品ファイル(`.ori3`)のJSON書き出し。

use std::fs;
use std::path::Path;

use ori3_model::Document;

/// 作品を、読みやすく整形した正規のORIGAMI3 JSONへ変換する。
///
/// 戻り値はそのまま`.ori3`ファイルとして保存でき、
/// `serde_json::from_str::<Document>`で読み直せる。
pub fn document_json(document: &Document) -> Result<String, String> {
    serde_json::to_string_pretty(document)
        .map_err(|error| format!("作品をJSONへ変換できませんでした: {error}"))
}

/// 作品を、指定したパスへ整形済みのORIGAMI3 JSONとして保存する。
///
/// 親ディレクトリがまだ無い場合は作成する。ファイル名や拡張子は呼び出し側が
/// 決められるため、テストfixtureや一時的な検証出力にも同じAPIを使える。
pub fn save_document(document: &Document, path: impl AsRef<Path>) -> Result<(), String> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "作品の保存先「{}」を作れませんでした: {error}",
                parent.display()
            )
        })?;
    }
    let json = document_json(document)?;
    fs::write(path, json)
        .map_err(|error| format!("作品「{}」を書き出せませんでした: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use ori3_model::{Document, DriverLine, FoldStep, Paper, TechniqueKind};

    use super::{document_json, save_document};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn sample_document() -> Document {
        let mut document = Document::new(Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        });
        document.sequence.push(FoldStep {
            id: 0,
            kind: TechniqueKind::Twist,
            drivers: vec![DriverLine {
                a: [0.4, 0.5],
                b: [0.5, 0.6],
                target_angle_deg: -180.0,
            }],
            layer_order: Some(vec![[0.25, 0.25]]),
            alignment: None,
            curved_inside_reverse: None,
            finish_soft: None,
            note: "座布団花のチェックポイント".to_string(),
        });
        document.display.soft_enabled = true;
        document.display.soft_stiffness = 0.25;
        document
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ori3-export-{label}-{}-{serial}",
            std::process::id()
        ))
    }

    #[test]
    fn document_json_is_pretty_and_round_trips() {
        let document = sample_document();
        let json = document_json(&document).expect("作品をJSONへ変換できる");

        assert!(
            json.contains("\n  \"schema_version\": 1,"),
            "pretty JSONとして改行・字下げされる: {json}"
        );
        let restored: Document = serde_json::from_str(&json).expect("書き出したJSONを読める");
        assert_eq!(restored, document, "作品がJSONを往復しても変わらない");
    }

    #[test]
    fn save_document_writes_the_requested_path_and_creates_parents() {
        let document = sample_document();
        let root = unique_temp_dir("save");
        let nested = root.join("nested");
        let path = nested.join("sample.ori3");

        save_document(&document, &path).expect("存在しない親ディレクトリにも保存できる");
        let json = fs::read_to_string(&path).expect("指定したパスへ書かれている");
        let restored: Document = serde_json::from_str(&json).expect("保存した作品を読める");
        assert_eq!(restored, document);

        fs::remove_file(&path).expect("テスト作品を片付ける");
        fs::remove_dir(&nested).expect("空の子ディレクトリを片付ける");
        fs::remove_dir(&root).expect("空のテストディレクトリを片付ける");
    }

    #[test]
    fn save_document_reports_the_target_path_on_write_failure() {
        let document = sample_document();
        let target_directory = unique_temp_dir("error");
        fs::create_dir(&target_directory).expect("書き込み失敗用ディレクトリを作る");

        let error = save_document(&document, &target_directory)
            .expect_err("ディレクトリそのものへファイルを書けない");
        assert!(
            error.contains(&target_directory.display().to_string()),
            "失敗したパスを利用者へ示す: {error}"
        );
        assert!(
            error.contains("書き出せませんでした"),
            "原因の文脈を示す: {error}"
        );

        fs::remove_dir(&target_directory).expect("空のテストディレクトリを片付ける");
    }
}
