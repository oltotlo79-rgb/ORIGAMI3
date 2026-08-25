use std::panic::{AssertUnwindSafe, catch_unwind};

use ori3_export::fold::{FoldAssignment, FoldParseError, FoldParseErrorKind, parse_fold_1_2};
use serde_json::{Value, json};

const MINIMAL_SUPPORTED: &str = include_str!("fixtures/fold/minimal-supported.fold");
const MALFORMED_SYNTAX: &str = include_str!("fixtures/fold/malformed-syntax.fold");
const MISSING_FILE_SPEC: &str = include_str!("fixtures/fold/missing-file-spec.fold");
const MALFORMED_FIELD_TYPE: &str = include_str!("fixtures/fold/malformed-field-type.fold");
const MALFORMED_NESTED_CASES: &str = include_str!("fixtures/fold/malformed-nested-cases.json");
const MULTIPLE_MALFORMED_FIELDS: &str =
    include_str!("fixtures/fold/multiple-malformed-fields.fold");
const UNSUPPORTED_EXTENSIONS: &str = include_str!("fixtures/fold/unsupported-extensions.fold");
const UNSUPPORTED_3D_BRANCH: &str = include_str!("fixtures/fold/unsupported-3d-branch.fold");
const FU_ASSIGNMENTS: &str = include_str!("fixtures/fold/fu-assignments.fold");
const UNKNOWN_FIELD_OWNERS: &str = include_str!("fixtures/fold/unknown-field-owners.fold");
const OTHER_ASSIGNMENTS: &str = include_str!("fixtures/fold/other-assignments.fold");

fn parse_error_without_panic(json: &str) -> FoldParseError {
    let outcome = catch_unwind(AssertUnwindSafe(|| parse_fold_1_2(json)));
    assert!(outcome.is_ok(), "壊れたFOLDでもpanicしてはいけない");
    outcome
        .expect("panicしないことを直前で確認済み")
        .expect_err("malformed caseは成功扱いにしない")
}

#[test]
fn minimal_supported_fixture_is_parsed_into_typed_fields() {
    let file = parse_fold_1_2(MINIMAL_SUPPORTED).expect("最小のFOLD 1.2を読める");
    let vertices = file
        .root
        .vertices_coords
        .as_ref()
        .expect("頂点座標を保持する");
    let edges = file
        .root
        .edges_vertices
        .as_ref()
        .expect("edge topologyを保持する");
    let assignments = file
        .root
        .edges_assignment
        .as_ref()
        .expect("assignmentを保持する");
    let angles = file
        .root
        .edges_fold_angle
        .as_ref()
        .expect("折り角を保持する");

    assert_eq!(vertices.len(), 4);
    assert!(vertices.iter().all(|vertex| vertex.len() == 2));
    assert_eq!(edges.len(), 5);
    assert_eq!(assignments.len(), 5);
    assert_eq!(assignments[4], FoldAssignment::Mountain);
    assert_eq!(angles.len(), 5);

    // 手書きfixtureの-90度からのJSON変換誤差は実測0だが、計算小数のexact比較を
    // 避ける。1e-12は受入境界1e-9度より3桁小さく、別の角度と区別できる。
    let mountain_angle = angles[4].expect("M edgeに角度がある");
    assert!((mountain_angle - (-90.0)).abs() <= 1e-12);
}

#[test]
fn malformed_fixtures_return_the_expected_kind_and_path_without_panicking() {
    let cases = [
        (
            "syntax",
            MALFORMED_SYNTAX,
            FoldParseErrorKind::InvalidJson,
            "$",
        ),
        (
            "missing file_spec",
            MISSING_FILE_SPEC,
            FoldParseErrorKind::MissingField,
            "$.file_spec",
        ),
        (
            "known field type",
            MALFORMED_FIELD_TYPE,
            FoldParseErrorKind::InvalidType,
            "$.vertices_coords",
        ),
    ];

    for (name, json, expected_kind, expected_path) in cases {
        let error = parse_error_without_panic(json);
        assert_eq!(error.kind, expected_kind, "case={name}: {error}");
        assert_eq!(error.path, expected_path, "case={name}: {error}");
    }
}

#[test]
fn malformed_nested_values_report_the_exact_path_without_panicking() {
    let manifest: Value =
        serde_json::from_str(MALFORMED_NESTED_CASES).expect("malformed fixture一覧を読める");
    let cases = manifest.as_array().expect("malformed fixture一覧はarray");
    assert_eq!(cases.len(), 9);

    for case in cases {
        let name = case["name"].as_str().expect("case名がある");
        let json = serde_json::to_string(&case["input"]).expect("fixture入力をJSON化できる");
        let expected_kind = match case["kind"].as_str().expect("error kindがある") {
            "invalid_type" => FoldParseErrorKind::InvalidType,
            "invalid_value" => FoldParseErrorKind::InvalidValue,
            "unsupported_version" => FoldParseErrorKind::UnsupportedVersion,
            "root_not_object" => FoldParseErrorKind::RootNotObject,
            other => panic!("未知のfixture error kind: {other}"),
        };
        let expected_path = case["path"].as_str().expect("expected pathがある");
        let error = parse_error_without_panic(&json);
        assert_eq!(error.kind, expected_kind, "case={name}: {error}");
        assert_eq!(error.path, expected_path, "case={name}: {error}");
    }
}

#[test]
fn first_error_is_deterministic_for_ten_repeated_parses() {
    for repetition in 0..10 {
        let error = parse_error_without_panic(MULTIPLE_MALFORMED_FIELDS);
        assert_eq!(
            error.kind,
            FoldParseErrorKind::InvalidType,
            "repeat={repetition}: {error}"
        );
        assert_eq!(
            error.path, "$.vertices_coords",
            "repeat={repetition}: {error}"
        );
    }
}

#[test]
fn all_twenty_unknown_extension_fields_are_preserved() {
    let file = parse_fold_1_2(UNSUPPORTED_EXTENSIONS).expect("未知fieldもtyped層では保持する");

    assert!(file.extra_fields.is_empty());
    assert_eq!(file.root.extra_fields.len(), 20);
    for number in 1..=20 {
        let key = format!("x_extension_{number:02}");
        assert_eq!(
            file.root.extra_fields.get(&key),
            Some(&Value::Bool(true)),
            "未知field {key}を捨てない"
        );
    }
}

#[test]
fn unknown_fields_stay_with_the_file_root_or_child_frame_that_owned_them() {
    let file = parse_fold_1_2(UNKNOWN_FIELD_OWNERS).expect("未知fieldはparseを妨げない");

    assert_eq!(
        file.extra_fields.get("file_vendor"),
        Some(&json!({"name": "fixture"}))
    );
    assert_eq!(
        file.root.extra_fields.get("vendor:root"),
        Some(&json!([1, 2]))
    );
    assert_eq!(file.file_frames.len(), 1);
    assert_eq!(
        file.file_frames[0].extra_fields.get("vendor:child"),
        Some(&json!({"kept": true}))
    );
    let retained_count = file.extra_fields.len()
        + file.root.extra_fields.len()
        + file.file_frames[0].extra_fields.len();
    assert_eq!(retained_count, 3, "入力した未知key 3/3を保持する");
}

#[test]
fn profile_semantics_are_preserved_for_the_validator_instead_of_rejected_by_parser() {
    let file =
        parse_fold_1_2(UNSUPPORTED_3D_BRANCH).expect("3D/branchの対応外判定はvalidatorへ渡す");
    let vertices = file
        .root
        .vertices_coords
        .as_ref()
        .expect("3D座標を警告用に保持する");

    assert!(vertices.iter().all(|vertex| vertex.len() == 3));
    assert_eq!(file.file_frames.len(), 3);
    assert_eq!(file.file_frames[0].frame_parent, Some(0));
    assert_eq!(file.file_frames[1].frame_parent, Some(1));
    assert_eq!(file.file_frames[2].frame_parent, Some(0));

    let other_assignment =
        parse_fold_1_2(OTHER_ASSIGNMENTS).expect("未知assignmentもvalidator用に保持する");
    assert_eq!(
        other_assignment.root.edges_assignment,
        Some(vec![
            FoldAssignment::Other("C".to_string()),
            FoldAssignment::Other("future-code".to_string())
        ])
    );
}

#[test]
fn flat_unassigned_and_null_fold_angle_are_not_silently_discarded() {
    let file = parse_fold_1_2(FU_ASSIGNMENTS).expect("F/Uを元値のままtyped層へ読む");
    let assignments = file
        .root
        .edges_assignment
        .as_ref()
        .expect("assignmentを保持する");
    let angles = file
        .root
        .edges_fold_angle
        .as_ref()
        .expect("fold angleを保持する");

    assert_eq!(assignments[4], FoldAssignment::Flat);
    assert_eq!(assignments[5], FoldAssignment::Unassigned);
    assert_eq!(angles[5], None, "nullを0度へ変えずに保持する");
}
