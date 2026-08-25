use std::collections::BTreeSet;

use ori3_export::fold::{
    FoldIssueCode, FoldIssueSeverity, parse_fold_1_2, unsupported_fields, validate_fold_1_2,
};
use serde_json::{Value, json};

const MINIMAL_SUPPORTED: &str = include_str!("fixtures/fold/minimal-supported.fold");
const FU_ASSIGNMENTS: &str = include_str!("fixtures/fold/fu-assignments.fold");
const LINEAR_STEPS: &str = include_str!("fixtures/fold/linear-steps.fold");
const UNSUPPORTED_3D_BRANCH: &str = include_str!("fixtures/fold/unsupported-3d-branch.fold");
const UNSUPPORTED_EXTENSIONS: &str = include_str!("fixtures/fold/unsupported-extensions.fold");
const VALIDATION_CASES: &str = include_str!("fixtures/fold/validation-cases.json");

fn validation_case(name: &str) -> ori3_export::fold::FoldFile {
    let manifest: Value =
        serde_json::from_str(VALIDATION_CASES).expect("validation fixture一覧を読める");
    let input = manifest
        .get(name)
        .expect("指定したvalidation fixtureがある");
    let json = serde_json::to_string(input).expect("validation fixtureをJSON化できる");
    parse_fold_1_2(&json).expect("validation fixtureをtyped値へ読める")
}

#[test]
fn direct_fold_file_still_requires_finite_exact_file_spec_1_2() {
    let mut file = parse_fold_1_2(MINIMAL_SUPPORTED).expect("基準fixtureを読む");
    for invalid in [1.1, f64::NAN] {
        file.file_spec = invalid;
        let validation = validate_fold_1_2(&file);
        assert!(validation.errors.iter().any(|issue| {
            issue.code == FoldIssueCode::InvalidValue && issue.path == "$.file_spec"
        }));
    }
}

#[test]
fn every_flat_and_unassigned_edge_keeps_its_value_and_exact_warning_path() {
    let file = parse_fold_1_2(FU_ASSIGNMENTS).expect("F/U fixtureをtyped値へ読む");
    let validation = validate_fold_1_2(&file);
    assert!(
        validation.errors.is_empty(),
        "F/Uは警告付きで限定取込できる: {:?}",
        validation.errors
    );
    let downgraded = validation
        .warnings
        .iter()
        .filter(|issue| issue.code == FoldIssueCode::AssignmentDowngradedToAux)
        .collect::<Vec<_>>();

    assert_eq!(downgraded.len(), 2, "入力F/U 2件に警告2件が必要");
    assert_eq!(downgraded[0].path, "$.edges_assignment[4]");
    assert_eq!(downgraded[0].original_value, Some(json!("F")));
    assert_eq!(downgraded[1].path, "$.edges_assignment[5]");
    assert_eq!(downgraded[1].original_value, Some(json!("U")));
    assert!(
        downgraded.iter().all(|issue| issue.message.contains("Aux")),
        "縮退先を利用者へ明示する"
    );

    let assignments = file
        .root
        .edges_assignment
        .as_ref()
        .expect("fixtureにassignmentがある");
    let fu_count = assignments
        .iter()
        .filter(|assignment| matches!(assignment.code(), "F" | "U"))
        .count();
    assert_eq!(downgraded.len(), fu_count, "F/Uのsilent dropは0件");
}

#[test]
fn all_twenty_unknown_extensions_are_visible_and_deterministic() {
    let file = parse_fold_1_2(UNSUPPORTED_EXTENSIONS).expect("未知fieldを保持して読む");
    let direct = unsupported_fields(&file);

    assert_eq!(direct.len(), 20, "未知extension 20/20を報告する");
    assert!(direct.iter().all(|issue| {
        issue.severity == FoldIssueSeverity::Warning
            && issue.code == FoldIssueCode::UnsupportedField
            && issue.original_value == Some(json!(true))
    }));
    for number in 1..=20 {
        let path = format!("$.x_extension_{number:02}");
        assert!(
            direct.iter().any(|issue| issue.path == path),
            "missing path: {path}"
        );
    }

    let baseline = validate_fold_1_2(&file);
    assert!(
        baseline.errors.is_empty(),
        "metadata extensionは限定取込可能"
    );
    assert_eq!(baseline.warnings.len(), 20);
    for repetition in 0..10 {
        assert_eq!(
            validate_fold_1_2(&file),
            baseline,
            "repeat={repetition}: warning分類と順序が変わらない"
        );
    }
}

#[test]
fn arbitrary_extension_keys_use_unambiguous_json_paths() {
    let file = validation_case("arbitrary_extension");
    let issues = unsupported_fields(&file);

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "$[\"vendor:root\"]");
    assert_eq!(issues[0].original_value, Some(json!({"kept": true})));
}

#[test]
fn three_dimensional_vertices_and_branching_frames_are_rejected_with_paths() {
    let file = parse_fold_1_2(UNSUPPORTED_3D_BRANCH).expect("profile判定前までは読む");
    let validation = validate_fold_1_2(&file);

    assert!(validation.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnsupportedGeometry && issue.path == "$.vertices_coords[0]"
    }));
    assert!(validation.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnsupportedGeometry && issue.path == "$.frame_attributes[0]"
    }));
    let nonlinear_paths = validation
        .errors
        .iter()
        .filter(|issue| issue.code == FoldIssueCode::NonLinearFrames)
        .map(|issue| issue.path.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!nonlinear_paths.contains("$.file_frames[1].frame_parent"));
    assert!(nonlinear_paths.contains("$.file_frames[2].frame_parent"));
}

#[test]
fn frame_parent_uses_root_zero_and_the_immediately_previous_frame() {
    let recorded_fixture = parse_fold_1_2(LINEAR_STEPS).expect("既存linear fixtureを読む");
    let recorded_result = validate_fold_1_2(&recorded_fixture);
    assert!(
        recorded_result.errors.is_empty(),
        "parents 0,1 の既存fixtureは直列かつprofile内: {:?}",
        recorded_result.errors
    );

    let mut missing_first_parent = recorded_fixture;
    missing_first_parent.file_frames[0].frame_parent = None;
    let missing_parent_result = validate_fold_1_2(&missing_first_parent);
    assert!(missing_parent_result.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::NonLinearFrames
            && issue.path == "$.file_frames[0].frame_parent"
    }));
}

#[test]
fn fold_angle_uses_fold_sign_and_fu_may_only_be_zero_or_null() {
    let file = validation_case("angle_valid");
    let valid = validate_fold_1_2(&file);
    assert!(
        valid
            .errors
            .iter()
            .all(|issue| issue.code != FoldIssueCode::InvalidValue),
        "FOLDではMが負、Vが正"
    );

    let invalid = validation_case("angle_invalid");
    let invalid_result = validate_fold_1_2(&invalid);
    let paths = invalid_result
        .errors
        .iter()
        .filter(|issue| issue.code == FoldIssueCode::InvalidValue)
        .map(|issue| issue.path.as_str())
        .collect::<BTreeSet<_>>();
    assert!(paths.contains("$.edges_foldAngle[2]"));
    assert!(paths.contains("$.edges_foldAngle[3]"));
}

#[test]
fn unique_total_face_orders_are_supported_but_cycles_and_ambiguity_are_not() {
    let minimal = parse_fold_1_2(MINIMAL_SUPPORTED).expect("既存faceOrders fixtureを読む");
    let minimal_result = validate_fold_1_2(&minimal);
    assert!(
        minimal_result.errors.is_empty(),
        "一意な2面のfaceOrdersはprofile内: {:?}",
        minimal_result.errors
    );

    let mut cyclic = parse_fold_1_2(FU_ASSIGNMENTS).expect("4面fixtureを読む");
    cyclic.root.face_orders = Some(vec![vec![0, 1, 1], vec![1, 2, 1], vec![2, 0, 1]]);
    let cyclic_result = validate_fold_1_2(&cyclic);
    assert!(cyclic_result.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnrepresentableFaceOrders
            && issue.path == "$.faceOrders"
            && issue.message.contains("循環")
    }));

    let mut ambiguous = cyclic.clone();
    ambiguous.root.face_orders = Some(vec![vec![0, 1, 1]]);
    let ambiguous_result = validate_fold_1_2(&ambiguous);
    assert!(ambiguous_result.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnrepresentableFaceOrders
            && issue.path == "$.faceOrders"
            && issue.message.contains("一意でない")
    }));
}

#[test]
fn rotated_rectangle_is_supported_and_trapezoid_is_rejected() {
    let rectangle = validation_case("rotated_rectangle");
    let rectangle_result = validate_fold_1_2(&rectangle);
    assert!(
        rectangle_result.errors.is_empty(),
        "axis alignedに限定しない: {:?}",
        rectangle_result.errors
    );

    let trapezoid = validation_case("trapezoid");
    let trapezoid_result = validate_fold_1_2(&trapezoid);
    assert!(trapezoid_result.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnsupportedGeometry && issue.path == "$.edges_assignment"
    }));
}

#[test]
fn invalid_topology_reports_the_failing_index_without_panicking() {
    let file = validation_case("invalid_topology");
    let validation = validate_fold_1_2(&file);

    assert!(validation.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::InvalidTopology
            && issue.path == "$.edges_vertices[1][1]"
            && issue.original_value == Some(json!(4))
    }));
}
