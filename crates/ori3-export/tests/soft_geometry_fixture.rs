mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ori3_model::{Document, Face3D, Frame3D, Paper};

use support::soft_geometry_fixture::{
    SoftGeometryCheckpoint, SoftGeometryFixture, fixture_from_json, fixture_json, load_fixture,
    save_fixture,
};

const STORED_FIXTURE: &str = include_str!("fixtures/minimal.frame3d-fixture");
const COORDINATE_TOLERANCE: f64 = 1e-12;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn sample_fixture() -> SoftGeometryFixture {
    SoftGeometryFixture::new(
        Document::new(Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        }),
        SoftGeometryCheckpoint {
            book_step: 3,
            instruction: Some("合成形状の丸みを検査する".to_string()),
            changes_shape: Some(true),
            frame: Frame3D {
                faces: vec![Face3D {
                    face: 0,
                    polygon: vec![[0.0, 0.0, 0.125], [1.0, 0.0, -0.25], [0.0, 1.0, 0.5]],
                    layer: 0,
                    surface_rank: 0,
                    mirrored: false,
                }],
                warnings: Vec::new(),
            },
        },
    )
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ori3-frame3d-fixture-{label}-{}-{serial}",
        std::process::id()
    ))
}

fn assert_coordinates_close(actual: &Frame3D, expected: &Frame3D) {
    assert_eq!(actual.faces.len(), expected.faces.len());
    for (actual_face, expected_face) in actual.faces.iter().zip(&expected.faces) {
        assert_eq!(actual_face.face, expected_face.face);
        assert_eq!(actual_face.polygon.len(), expected_face.polygon.len());
        for (actual_point, expected_point) in actual_face.polygon.iter().zip(&expected_face.polygon)
        {
            for (actual_coordinate, expected_coordinate) in actual_point.iter().zip(expected_point)
            {
                let difference = (actual_coordinate - expected_coordinate).abs();
                assert!(
                    difference <= COORDINATE_TOLERANCE,
                    "3D fixture座標差 {difference:e} が許容差 {COORDINATE_TOLERANCE:e} を超えた"
                );
            }
        }
    }
}

#[test]
fn dedicated_fixture_file_restores_the_validation_frame() {
    let fixture = fixture_from_json(STORED_FIXTURE).expect("専用fixtureを読める");
    let expected = sample_fixture();

    assert_eq!(fixture.document, expected.document);
    assert_eq!(fixture.checkpoint.book_step, 3);
    assert_eq!(
        fixture.checkpoint.instruction.as_deref(),
        Some("合成形状の丸みを検査する")
    );
    assert_eq!(fixture.checkpoint.changes_shape, Some(true));
    assert_coordinates_close(&fixture.checkpoint.frame, &expected.checkpoint.frame);
}

#[test]
fn fixture_save_and_load_round_trip_with_measured_tolerance() {
    let fixture = sample_fixture();
    let root = unique_temp_dir("roundtrip");
    let path = root.join("nested").join("sample.frame3d-fixture");

    save_fixture(&fixture, &path).expect("専用拡張子へ保存できる");
    let restored = load_fixture(&path).expect("保存した専用fixtureを読める");
    assert_eq!(restored.document, fixture.document);
    assert_coordinates_close(&restored.checkpoint.frame, &fixture.checkpoint.frame);

    fs::remove_file(&path).expect("一時fixtureを片付ける");
    fs::remove_dir(path.parent().expect("親がある")).expect("空の子ディレクトリを片付ける");
    fs::remove_dir(&root).expect("空の一時ディレクトリを片付ける");
}

#[test]
fn fixture_writer_rejects_ori3_before_creating_a_file() {
    let fixture = sample_fixture();
    let root = unique_temp_dir("extension");
    let path = root.join("must-not-exist.ori3");

    let error = save_fixture(&fixture, &path).expect_err(".ori3への3D頂点保存を拒否する");
    assert!(error.contains(".frame3d-fixture"), "error={error}");
    assert!(!path.exists(), "拒否した.ori3を作ってはいけない");
    assert!(!root.exists(), "拡張子検査は親ディレクトリ作成より先に行う");
}

#[test]
fn fixture_schema_version_is_independent_and_checked() {
    let mut value: serde_json::Value =
        serde_json::from_str(&fixture_json(&sample_fixture()).expect("fixture JSONを作れる"))
            .expect("JSON値へ読める");
    value["fixture_schema_version"] = serde_json::json!(2);

    let error = fixture_from_json(&value.to_string()).expect_err("未知のfixture版を拒否する");
    assert!(error.contains("版2"), "error={error}");
}

#[test]
fn ordinary_ori3_json_has_no_frame_or_vertex_snapshot() {
    let fixture = sample_fixture();
    let json = ori3_export::document_json(&fixture.document).expect("通常作品JSONを作れる");
    let value: serde_json::Value = serde_json::from_str(&json).expect("通常作品JSONを読める");

    assert!(value.get("frame").is_none());
    assert!(value.get("checkpoint").is_none());
    assert!(value.get("soft_geometry").is_none());
    assert!(value.get("fixture_schema_version").is_none());
}
