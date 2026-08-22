//! 3D頂点を使う検証fixture専用形式。
//!
//! 製品の`.ori3`形式と公開APIへ3D状態を混ぜないため、integration testの
//! support内だけに置く。保存先も専用拡張子以外は書込み前に拒否する。

use std::fs;
use std::path::Path;

use ori3_model::{Document, Frame3D};

pub const FIXTURE_EXTENSION: &str = "frame3d-fixture";
const FIXTURE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SoftGeometryCheckpoint {
    pub book_step: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes_shape: Option<bool>,
    pub frame: Frame3D,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SoftGeometryFixture {
    pub fixture_schema_version: u32,
    pub document: Document,
    pub checkpoint: SoftGeometryCheckpoint,
}

impl SoftGeometryFixture {
    pub fn new(document: Document, checkpoint: SoftGeometryCheckpoint) -> Self {
        Self {
            fixture_schema_version: FIXTURE_SCHEMA_VERSION,
            document,
            checkpoint,
        }
    }
}

pub fn fixture_json(fixture: &SoftGeometryFixture) -> Result<String, String> {
    serde_json::to_string_pretty(fixture)
        .map_err(|error| format!("3D検証fixtureをJSONへ変換できませんでした: {error}"))
}

pub fn fixture_from_json(json: &str) -> Result<SoftGeometryFixture, String> {
    let fixture: SoftGeometryFixture = serde_json::from_str(json)
        .map_err(|error| format!("3D検証fixtureを読めませんでした: {error}"))?;
    if fixture.fixture_schema_version != FIXTURE_SCHEMA_VERSION {
        return Err(format!(
            "3D検証fixtureの版{}には対応していません(対応版: {})",
            fixture.fixture_schema_version, FIXTURE_SCHEMA_VERSION
        ));
    }
    Ok(fixture)
}

pub fn save_fixture(fixture: &SoftGeometryFixture, path: impl AsRef<Path>) -> Result<(), String> {
    let path = path.as_ref();
    require_fixture_extension(path)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "3D検証fixtureの保存先「{}」を作れませんでした: {error}",
                parent.display()
            )
        })?;
    }
    let json = fixture_json(fixture)?;
    fs::write(path, json).map_err(|error| {
        format!(
            "3D検証fixture「{}」を書き出せませんでした: {error}",
            path.display()
        )
    })
}

pub fn load_fixture(path: impl AsRef<Path>) -> Result<SoftGeometryFixture, String> {
    let path = path.as_ref();
    require_fixture_extension(path)?;
    let json = fs::read_to_string(path).map_err(|error| {
        format!(
            "3D検証fixture「{}」を読めませんでした: {error}",
            path.display()
        )
    })?;
    fixture_from_json(&json)
}

fn require_fixture_extension(path: &Path) -> Result<(), String> {
    if path.extension().and_then(|extension| extension.to_str()) == Some(FIXTURE_EXTENSION) {
        return Ok(());
    }
    Err(format!(
        "3D頂点を含む検証データは .{FIXTURE_EXTENSION} だけに保存できます: {}",
        path.display()
    ))
}
