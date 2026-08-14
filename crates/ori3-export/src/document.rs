//! ORIGAMI3作品ファイル(`.ori3`)のJSON書き出し。

use std::fs;
use std::path::Path;

use ori3_model::{Document, Frame3D};

/// 折り目のない丸み・カールや複合技法の途中を含む、工程到達時の3D形状。
///
/// 通常のDocumentは手順と表示パラメータを保存し、頂点位置を持たない。
/// 局所的な手整形まで行った作品を検証用fixtureへ残す場合だけ、この拡張を
/// .ori3のトップレベルへ加える。通常のDocument読込では未知フィールドとして
/// 安全に無視され、このモジュールの読込APIでは完全な形を復元できる。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SoftGeometrySnapshot {
    pub book_step: u32,
    /// 原資料の工程に対応する説明。既存fixtureには無いため任意。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    /// この工程で紙の形が変わるか。向き変更・保持・完成図はfalse。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes_shape: Option<bool>,
    pub frame: Frame3D,
}

#[derive(serde::Serialize)]
struct DocumentWithSoftGeometry<'a> {
    #[serde(flatten)]
    document: &'a Document,
    soft_geometry: &'a SoftGeometrySnapshot,
}

#[derive(serde::Deserialize)]
struct OwnedDocumentWithSoftGeometry {
    #[serde(flatten)]
    document: Document,
    soft_geometry: SoftGeometrySnapshot,
}

/// 作品を、読みやすく整形した正規のORIGAMI3 JSONへ変換する。
///
/// 戻り値はそのまま`.ori3`ファイルとして保存でき、
/// `serde_json::from_str::<Document>`で読み直せる。
pub fn document_json(document: &Document) -> Result<String, String> {
    serde_json::to_string_pretty(document)
        .map_err(|error| format!("作品をJSONへ変換できませんでした: {error}"))
}

/// 作品手順と、折り目のない局所変形後の実3D形状を同じ.ori3 JSONへ変換する。
pub fn document_with_soft_geometry_json(
    document: &Document,
    soft_geometry: &SoftGeometrySnapshot,
) -> Result<String, String> {
    serde_json::to_string_pretty(&DocumentWithSoftGeometry {
        document,
        soft_geometry,
    })
    .map_err(|error| format!("軟体形状を含む作品をJSONへ変換できませんでした: {error}"))
}

/// 軟体形状付きJSONを、作品手順と実3D形状へ読み戻す。
pub fn document_with_soft_geometry_from_json(
    json: &str,
) -> Result<(Document, SoftGeometrySnapshot), String> {
    let owned: OwnedDocumentWithSoftGeometry = serde_json::from_str(json)
        .map_err(|error| format!("軟体形状を含む作品JSONを読めませんでした: {error}"))?;
    Ok((owned.document, owned.soft_geometry))
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

/// 作品手順と折り目のない実3D形状を、同じ.ori3ファイルへ保存する。
pub fn save_document_with_soft_geometry(
    document: &Document,
    soft_geometry: &SoftGeometrySnapshot,
    path: impl AsRef<Path>,
) -> Result<(), String> {
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
    let json = document_with_soft_geometry_json(document, soft_geometry)?;
    fs::write(path, json)
        .map_err(|error| format!("作品「{}」を書き出せませんでした: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use ori3_model::{Document, DriverLine, Face3D, FoldStep, Frame3D, Paper, TechniqueKind};

    use super::{
        SoftGeometrySnapshot, document_json, document_with_soft_geometry_from_json,
        document_with_soft_geometry_json, save_document,
    };

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
            note: "川崎ローズのチェックポイント".to_string(),
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

    #[test]
    fn soft_geometry_extension_round_trips_and_remains_document_compatible() {
        let document = sample_document();
        let snapshot = SoftGeometrySnapshot {
            book_step: 29,
            instruction: Some("完成形を確認する".to_string()),
            changes_shape: Some(false),
            frame: Frame3D {
                faces: vec![Face3D {
                    face: 0,
                    polygon: vec![[0.0, 0.0, 0.2], [1.0, 0.0, -0.1], [0.0, 1.0, 0.0]],
                    layer: 0,
                    mirrored: false,
                }],
                warnings: Vec::new(),
            },
        };
        let json = document_with_soft_geometry_json(&document, &snapshot)
            .expect("軟体形状付き作品を書ける");
        assert!(json.contains("\"soft_geometry\""));

        let ordinary: Document =
            serde_json::from_str(&json).expect("通常の作品読込とも後方互換である");
        assert_eq!(ordinary, document);

        let (restored, restored_snapshot) =
            document_with_soft_geometry_from_json(&json).expect("軟体形状も読み戻せる");
        assert_eq!(restored, document);
        assert_eq!(restored_snapshot.book_step, 29);
        assert_eq!(
            restored_snapshot.instruction.as_deref(),
            Some("完成形を確認する")
        );
        assert_eq!(restored_snapshot.changes_shape, Some(false));
        assert_eq!(restored_snapshot.frame.faces.len(), 1);
        assert_eq!(
            restored_snapshot.frame.faces[0].polygon,
            snapshot.frame.faces[0].polygon
        );
        assert!(restored_snapshot.frame.warnings.is_empty());
    }
}
