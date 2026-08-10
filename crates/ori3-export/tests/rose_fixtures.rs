use std::fs;
use std::path::PathBuf;

use ori3_cp::{extract_faces, validate};
use ori3_export::document_with_soft_geometry_from_json;
use ori3_layers::replay;
use ori3_model::Document;
use ori3_rigid::max_seam_gap;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ori3-layers/tests/fixtures")
        .join(name)
}

#[test]
fn rose_checkpoints_are_valid_ori3_documents() {
    for (name, expected_steps) in [
        ("rose-011.ori3", 1usize),
        ("rose-021.ori3", 4usize),
        ("rose-029.ori3", 5usize),
    ] {
        let json = fs::read_to_string(fixture(name)).expect("ローズfixtureを読める");
        let document: Document = serde_json::from_str(&json).expect("正規.ori3として読める");
        assert_eq!(document.sequence.len(), expected_steps, "{name}の操作数");
        assert!(validate(&document.cp).is_empty(), "{name}の型紙警告");
        let faces = extract_faces(&document.cp);
        let replayed = replay(&document, document.sequence.len(), 1.0);
        assert!(replayed.warnings.is_empty(), "{name}の再生警告");
        assert!(replayed.skipped.is_empty(), "{name}の再生スキップ");
        assert!(replayed.frame.warnings.is_empty(), "{name}の3D警告");
        assert_eq!(replayed.frame.faces.len(), faces.len(), "{name}の面数");
        let gap = max_seam_gap(&document.cp, &faces, &replayed.frame);
        assert!(gap < 1e-6, "{name}の裂け: {gap}");
        println!("{name}: max_seam_gap={gap:.3e}");
    }
}

#[test]
fn completed_rose_fixture_contains_the_seamless_soft_shape() {
    let json = fs::read_to_string(fixture("rose-029.ori3")).expect("完成ローズfixtureを読める");
    let (document, snapshot) =
        document_with_soft_geometry_from_json(&json).expect("完成曲面を復元できる");
    assert_eq!(snapshot.book_step, 29);
    assert!(snapshot.frame.warnings.is_empty());

    let faces = extract_faces(&document.cp);
    assert_eq!(
        snapshot.frame.faces.len(),
        faces.len(),
        "完成時にも全ての面がある"
    );
    assert!(
        snapshot
            .frame
            .faces
            .iter()
            .flat_map(|face| &face.polygon)
            .flatten()
            .all(|value| value.is_finite()),
        "完成曲面の全座標が有限"
    );
    let gap = max_seam_gap(&document.cp, &faces, &snapshot.frame);
    assert!(gap < 1e-6, "保存した完成曲面にも裂けがない: {gap}");
}
