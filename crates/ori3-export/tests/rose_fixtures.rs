use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use ori3_cp::{extract_faces, validate};
use ori3_export::document_with_soft_geometry_from_json;
use ori3_layers::replay;
use ori3_model::{Document, Frame3D};
use ori3_rigid::max_seam_gap;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ori3-layers/tests/fixtures")
        .join(name)
}

fn fixture_name(book_step: u32) -> String {
    format!("rose-{book_step:03}.ori3")
}

fn expected_sequence_len(book_step: u32) -> usize {
    match book_step {
        1..=3 => 0,
        4..=11 => 1,
        12..=17 => 2,
        18..=19 => 3,
        20..=21 => 4,
        22..=29 => 5,
        _ => unreachable!("工程番号は1〜29"),
    }
}

fn changes_shape(book_step: u32) -> bool {
    !matches!(book_step, 2 | 16 | 17 | 19 | 21 | 25 | 29)
}

fn load_arrival_frame(book_step: u32) -> (Document, Frame3D) {
    let name = fixture_name(book_step);
    let json = fs::read_to_string(fixture(&name)).expect("ローズfixtureを読める");
    let document: Document = serde_json::from_str(&json).expect("正規.ori3として読める");

    if matches!(book_step, 11 | 21) {
        let replayed = replay(&document, document.sequence.len(), 1.0);
        assert!(replayed.warnings.is_empty(), "{name}の再生警告");
        assert!(replayed.skipped.is_empty(), "{name}の再生スキップ");
        assert!(replayed.frame.warnings.is_empty(), "{name}の3D警告");
        return (document, replayed.frame);
    }

    let (extended_document, snapshot) =
        document_with_soft_geometry_from_json(&json).expect("工程到達形状を復元できる");
    assert_eq!(extended_document, document, "{name}の通常読込との一致");
    assert_eq!(snapshot.book_step, book_step, "{name}の工程番号");
    if book_step != 29 {
        assert!(
            snapshot
                .instruction
                .as_deref()
                .is_some_and(|text| !text.is_empty()),
            "{name}に原図対応の説明がある"
        );
        assert_eq!(
            snapshot.changes_shape,
            Some(changes_shape(book_step)),
            "{name}の形状変化判定"
        );
    }
    assert!(snapshot.frame.warnings.is_empty(), "{name}の保存3D警告");
    (document, snapshot.frame)
}

#[test]
fn all_29_rose_step_fixture_names_exist_exactly_once() {
    let directory = fixture("rose-001.ori3")
        .parent()
        .expect("fixtureディレクトリ")
        .to_path_buf();
    let actual = fs::read_dir(directory)
        .expect("fixture一覧を読める")
        .map(|entry| entry.expect("fixtureエントリ").file_name())
        .filter_map(|name| name.to_str().map(str::to_owned))
        .filter(|name| name.starts_with("rose-") && name.ends_with(".ori3"))
        .collect::<BTreeSet<_>>();
    let expected = (1..=29).map(fixture_name).collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "ローズfixtureは001〜029の29件だけ");
}

#[test]
fn every_rose_step_is_complete_warning_free_and_seamless() {
    let mut largest_gap = 0.0_f64;
    let mut step29_gap = None;
    for book_step in 1..=29 {
        let name = fixture_name(book_step);
        let (document, frame) = load_arrival_frame(book_step);
        assert_eq!(
            document.sequence.len(),
            expected_sequence_len(book_step),
            "{name}の操作数"
        );
        assert!(validate(&document.cp).is_empty(), "{name}の型紙警告");

        let replayed = replay(&document, document.sequence.len(), 1.0);
        assert!(replayed.warnings.is_empty(), "{name}の通常再生警告");
        assert!(replayed.skipped.is_empty(), "{name}の通常再生スキップ");
        assert!(replayed.frame.warnings.is_empty(), "{name}の通常3D警告");

        let faces = extract_faces(&document.cp);
        let expected = faces
            .iter()
            .map(|face| (face.id, face.vertices.len()))
            .collect::<HashMap<_, _>>();
        let actual = frame
            .faces
            .iter()
            .map(|face| (face.face, face.polygon.len()))
            .collect::<HashMap<_, _>>();
        assert_eq!(frame.faces.len(), faces.len(), "{name}で面数が変わった");
        assert_eq!(actual.len(), frame.faces.len(), "{name}で面IDが重複した");
        assert_eq!(actual, expected, "{name}で面または面頂点が失われた");

        let cp_vertices = document
            .cp
            .vertices
            .iter()
            .map(|vertex| vertex.id)
            .collect::<HashSet<_>>();
        let face_vertices = faces
            .iter()
            .flat_map(|face| face.vertices.iter().copied())
            .collect::<HashSet<_>>();
        assert_eq!(face_vertices, cp_vertices, "{name}でCP頂点が面から失われた");
        assert!(
            frame
                .faces
                .iter()
                .flat_map(|face| &face.polygon)
                .flatten()
                .all(|value| value.is_finite()),
            "{name}の保存座標が有限"
        );

        let gap = max_seam_gap(&document.cp, &faces, &frame);
        assert!(gap < 1e-6, "{name}の裂け: {gap:.9e}");
        largest_gap = largest_gap.max(gap);
        if book_step == 29 {
            step29_gap = Some(gap);
        }
    }
    assert_eq!(step29_gap, Some(0.0), "完成形の裂けは従来どおり0");
    println!("rose-001〜029: 最大max_seam_gap={largest_gap:.3e}");
}

#[test]
fn shape_invariant_book_steps_share_the_same_frame() {
    for (first, second) in [
        (1, 2),
        (15, 16),
        (16, 17),
        (18, 19),
        (20, 21),
        (24, 25),
        (28, 29),
    ] {
        let (_, first_frame) = load_arrival_frame(first);
        let (_, second_frame) = load_arrival_frame(second);
        assert_eq!(first_frame.warnings, second_frame.warnings);
        assert_eq!(first_frame.faces.len(), second_frame.faces.len());
        let mut largest_difference = 0.0_f64;
        for (a, b) in first_frame.faces.iter().zip(&second_frame.faces) {
            assert_eq!(a.face, b.face, "形状不変の工程{first}→{second}: 面ID");
            assert_eq!(a.layer, b.layer, "形状不変の工程{first}→{second}: 層番号");
            assert_eq!(
                a.polygon.len(),
                b.polygon.len(),
                "形状不変の工程{first}→{second}: 面頂点数"
            );
            for (pa, pb) in a.polygon.iter().zip(&b.polygon) {
                for (va, vb) in pa.iter().zip(pb) {
                    largest_difference = largest_difference.max((va - vb).abs());
                }
            }
        }
        assert!(
            largest_difference < 1e-12,
            "形状不変の工程{first}→{second}: 座標差={largest_difference:.3e}"
        );
    }
}

#[test]
fn legacy_checkpoint_documents_keep_their_original_history_lengths() {
    for (name, expected_steps) in [
        ("rose-011.ori3", 1usize),
        ("rose-021.ori3", 4usize),
        ("rose-029.ori3", 5usize),
    ] {
        let json = fs::read_to_string(fixture(name)).expect("既存fixtureを読める");
        let document: Document = serde_json::from_str(&json).expect("既存.ori3を読める");
        assert_eq!(
            document.sequence.len(),
            expected_steps,
            "{name}の履歴を保持"
        );
    }
}
