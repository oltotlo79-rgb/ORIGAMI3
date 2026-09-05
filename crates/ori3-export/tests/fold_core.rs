use ori3_export::fold::{
    FOLD_1_2_PROFILE_NAME, FOLD_1_2_UNSUPPORTED_FEATURES, FoldComparisonOptions, FoldIssueCode,
    compare_fold_1_2, parse_fold_1_2, validate_fold_1_2, write_fold_1_2,
};

const MINIMAL: &str = include_str!("fixtures/fold/minimal-supported.fold");
const FU_ASSIGNMENTS: &str = include_str!("fixtures/fold/fu-assignments.fold");
const UNSUPPORTED_3D_BRANCH: &str = include_str!("fixtures/fold/unsupported-3d-branch.fold");

#[test]
fn public_contract_uses_the_approved_profile_name_and_all_eight_limitations() {
    assert_eq!(FOLD_1_2_PROFILE_NAME, "FOLD 1.2 限定");
    assert_eq!(
        FOLD_1_2_UNSUPPORTED_FEATURES,
        [
            "3D座標",
            "枝分かれした手順",
            "動画",
            "名前付き技法の意味",
            "注記",
            "仕上げの丸み",
            "FOLDの「平ら(F)」「未指定(U)」の区別",
            // 2026-09-05追加。終点が平坦な手順は宣言角の平坦再生で確かめるよう直したので、
            // 「紙を曲げずに到達できること」の要求はここに書いた範囲だけに残る。
            "平らでない途中の形で終わる手順のうち、紙を曲げずには作れないもの",
        ]
    );
}

#[test]
fn supported_core_parse_validate_write_parse_round_trip_is_equivalent() {
    let parsed = parse_fold_1_2(MINIMAL).expect("限定profileをparseできる");
    let validation = validate_fold_1_2(&parsed);
    assert!(
        validation.errors.is_empty(),
        "errors={:?}",
        validation.errors
    );

    let json = write_fold_1_2(&parsed).expect("限定profileを書き出せる");
    let restored = parse_fold_1_2(&json).expect("writerのJSONをparseできる");
    let comparison = compare_fold_1_2(&parsed, &restored, FoldComparisonOptions::default())
        .expect("roundtripを比較できる");
    assert!(
        comparison.is_equivalent(),
        "differences={:?}",
        comparison.differences
    );
}

#[test]
fn every_fu_assignment_keeps_its_value_and_path_in_an_aux_warning() {
    let parsed = parse_fold_1_2(FU_ASSIGNMENTS).expect("F/U fixtureをparseできる");
    let validation = validate_fold_1_2(&parsed);
    assert!(
        validation.errors.is_empty(),
        "errors={:?}",
        validation.errors
    );
    assert_eq!(validation.warnings.len(), 2, "F/Uの2 edgeを2/2警告する");

    let paths = validation
        .warnings
        .iter()
        .filter(|issue| issue.code == FoldIssueCode::AssignmentDowngradedToAux)
        .map(|issue| {
            assert!(issue.message.contains("Aux"), "message={}", issue.message);
            (issue.path.as_str(), issue.original_value.as_ref())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            ("$.edges_assignment[4]", Some(&serde_json::json!("F"))),
            ("$.edges_assignment[5]", Some(&serde_json::json!("U"))),
        ]
    );
}

#[test]
fn unsupported_structure_is_rejected_with_paths_before_writing() {
    let parsed = parse_fold_1_2(UNSUPPORTED_3D_BRANCH).expect("対応外構造もtyped層では保持する");
    let validation = validate_fold_1_2(&parsed);
    assert!(!validation.errors.is_empty());
    assert!(
        validation
            .errors
            .iter()
            .any(|issue| issue.path.starts_with("$.vertices_coords[")),
        "3D座標pathを示す: {:?}",
        validation.errors
    );
    assert!(
        validation
            .errors
            .iter()
            .any(|issue| issue.path == "$.file_frames[2].frame_parent"),
        "branch pathを示す: {:?}",
        validation.errors
    );

    let error = write_fold_1_2(&parsed).expect_err("対応外構造を成功扱いで書き出さない");
    assert!(!error.issues.is_empty());
}
