use std::panic::{AssertUnwindSafe, catch_unwind};

use ori3_cp::extract_faces;
use ori3_export::fold::{FoldIssueCode, FoldIssueSeverity, fold_to_document, parse_fold_1_2};
use ori3_layers::{FlatState, replay};
use ori3_model::{EdgeKind, SCHEMA_VERSION, TechniqueKind};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

const MINIMAL: &str = include_str!("fixtures/fold/minimal-supported.fold");
const FU: &str = include_str!("fixtures/fold/fu-assignments.fold");
const LINEAR: &str = include_str!("fixtures/fold/linear-steps.fold");
const ROTATED: &str = include_str!("fixtures/fold/rotated-scaled.fold");
const FLAT_ORDERS: &str = include_str!("fixtures/fold/flat-face-orders.fold");
const REDUNDANT_ORDERS: &str = include_str!("fixtures/fold/redundant-face-orders.fold");
const EXTENSIONS: &str = include_str!("fixtures/fold/unsupported-extensions.fold");
const UNSUPPORTED: &str = include_str!("fixtures/fold/unsupported-3d-branch.fold");
const CHANGED_STEP_COORDS: &str = include_str!("fixtures/fold/changed-step-coordinates.fold");
const INHERITED_ORDER_SOURCE: &str = include_str!("fixtures/fold/inherited-face-order-source.fold");
const INHERITED_NULL_ANGLE_SOURCE: &str =
    include_str!("fixtures/fold/inherited-null-angle-source.fold");

#[test]
fn fold_assignments_and_angles_map_through_one_conversion_table() {
    let mut file = parse_fold_1_2(FU).expect("F/U fixtureをparseできる");
    // This test isolates assignment conversion. The fixture's four FOLD faces are
    // intentionally not one-to-one after F/U become non-face-splitting Aux edges.
    file.root.face_orders = None;

    let imported = fold_to_document(&file).expect("faceOrdersを除けば限定取込できる");
    assert_eq!(
        imported
            .document
            .cp
            .edges
            .iter()
            .map(|edge| edge.kind)
            .collect::<Vec<_>>(),
        vec![
            EdgeKind::Border,
            EdgeKind::Border,
            EdgeKind::Border,
            EdgeKind::Border,
            EdgeKind::Aux,
            EdgeKind::Aux,
            EdgeKind::Mountain,
            EdgeKind::Valley,
        ]
    );

    let downgrade = imported
        .warnings
        .iter()
        .filter(|issue| issue.code == FoldIssueCode::AssignmentDowngradedToAux)
        .collect::<Vec<_>>();
    assert_eq!(downgrade.len(), 2);
    assert_eq!(downgrade[0].path, "$.edges_assignment[4]");
    assert_eq!(
        downgrade[0]
            .original_value
            .as_ref()
            .and_then(|v| v.as_str()),
        Some("F")
    );
    assert_eq!(downgrade[1].path, "$.edges_assignment[5]");
    assert_eq!(
        downgrade[1]
            .original_value
            .as_ref()
            .and_then(|v| v.as_str()),
        Some("U")
    );

    let step = &imported.document.sequence[0];
    assert_eq!(step.kind, TechniqueKind::Pose);
    assert_close(step.drivers[0].target_angle_deg, 45.0);
    assert_close(step.drivers[1].target_angle_deg, -45.0);
}

#[test]
fn root_and_each_linear_child_preserve_step_count_order_and_absolute_endpoint_angles() {
    let mut file = parse_fold_1_2(LINEAR).expect("linear fixtureをparseできる");
    // Partial angles and layer_order cannot coexist in the model. Removing only
    // faceOrders lets this fixture exercise its three partial-angle Pose endpoints.
    file.root.face_orders = None;
    for frame in &mut file.file_frames {
        frame.face_orders = None;
    }

    let imported = fold_to_document(&file).expect("3 endpointを取込める");
    assert_eq!(imported.document.sequence.len(), 3);
    let targets = imported
        .document
        .sequence
        .iter()
        .map(|step| {
            assert_eq!(step.kind, TechniqueKind::Pose);
            assert_eq!(step.drivers.len(), 1);
            step.drivers[0].target_angle_deg
        })
        .collect::<Vec<_>>();
    assert_close(targets[0], -30.0);
    assert_close(targets[1], -60.0);
    assert_close(targets[2], -90.0);
}

#[test]
fn flat_face_orders_become_bottom_to_top_representative_points() {
    let file = parse_fold_1_2(FLAT_ORDERS).expect("flat order fixtureをparseできる");
    let imported = fold_to_document(&file).expect("平坦なtotal orderを取込める");
    assert_eq!(imported.document.sequence.len(), 1);
    let step = &imported.document.sequence[0];
    assert_eq!(step.kind, TechniqueKind::Simple);
    assert_close(step.drivers[0].target_angle_deg, 180.0);

    let points = step
        .layer_order
        .as_ref()
        .expect("平坦stepはlayer_orderを持つ");
    let faces = extract_faces(&imported.document.cp);
    let (resolved, warnings) = FlatState::resolve_order(&imported.document.cp, &faces, points);
    assert!(warnings.is_empty());
    // FOLD [0,1,+1] means face 0 is above face 1; model order is bottom→top.
    assert_eq!(resolved, vec![1, 0]);

    let endpoint = replay(&imported.document, imported.document.sequence.len(), 1.0);
    assert!(endpoint.skipped.is_empty());
    assert!(
        endpoint
            .frame
            .faces
            .iter()
            .all(|face| { face.polygon.iter().flatten().copied().all(f64::is_finite) })
    );
    assert!(
        max_seam_gap(&imported.document.cp, &faces, &endpoint.frame) <= 1e-6,
        "§12.6のflat endpoint seam境界"
    );
    assert_eq!(self_intersection_pairs(&endpoint.frame).len(), 0);
}

#[test]
fn non_flat_face_orders_are_rejected_instead_of_attached_to_pose() {
    for input in [MINIMAL, LINEAR] {
        let file = parse_fold_1_2(input).expect("fixtureをparseできる");
        let error = fold_to_document(&file).expect_err("非平坦faceOrdersを成功扱いしない");
        assert!(error.errors.iter().any(|issue| {
            issue.code == FoldIssueCode::UnrepresentableFaceOrders
                && issue.path.ends_with("faceOrders")
                && issue.message.contains("平坦終点")
        }));
    }
}

#[test]
fn face_orders_must_be_exactly_the_canonical_adjacent_chain() {
    let file = parse_fold_1_2(REDUNDANT_ORDERS).expect("redundant order fixtureをparseできる");
    let error = fold_to_document(&file).expect_err("推移的な冗長制約を黙って落とさない");
    let relevant = error
        .errors
        .iter()
        .filter(|issue| {
            issue.code == FoldIssueCode::UnrepresentableFaceOrders
                && issue.path == "$.file_frames[0].faceOrders"
                && issue.message.contains("隣接chain")
        })
        .count();
    assert_eq!(relevant, 1);
}

#[test]
fn aux_downgrade_that_changes_face_identity_rejects_orders_but_keeps_fu_warnings() {
    let file = parse_fold_1_2(FU).expect("F/U fixtureをparseできる");
    let error = fold_to_document(&file).expect_err("Aux化後に同型でないfaceOrdersを拒否する");
    assert_eq!(
        error
            .warnings
            .iter()
            .filter(|issue| issue.code == FoldIssueCode::AssignmentDowngradedToAux)
            .count(),
        2
    );
    assert!(error.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnrepresentableFaceOrders
            && issue.path.starts_with("$.faces_vertices[")
    }));
}

#[test]
fn similarity_normalization_is_finite_ratio_preserving_and_explicitly_warned() {
    let file = parse_fold_1_2(ROTATED).expect("rotated fixtureをparseできる");
    let imported = fold_to_document(&file).expect("回転・scale長方形を正規化できる");
    assert_close(imported.document.paper.width_mm, 1.0);
    assert_close(imported.document.paper.height_mm, 0.5);
    let expected = [[0.0, 0.0], [1.0, 0.0], [1.0, 0.5], [0.0, 0.5]];
    for (actual, expected) in imported.document.cp.vertices.iter().zip(expected) {
        assert_close(actual.pos[0], expected[0]);
        assert_close(actual.pos[1], expected[1]);
        assert!(actual.pos.into_iter().all(f64::is_finite));
    }
    let normalization = imported
        .warnings
        .iter()
        .filter(|issue| issue.path == "$.vertices_coords" && issue.message.contains("similarity"))
        .collect::<Vec<_>>();
    assert_eq!(normalization.len(), 1);
    assert!(normalization[0].original_value.is_some());
}

#[test]
fn canonical_axis_aligned_coordinates_do_not_claim_a_normalization_loss() {
    let mut file = parse_fold_1_2(MINIMAL).expect("minimal fixtureをparseできる");
    file.root.face_orders = None;
    let imported = fold_to_document(&file).expect("canonical座標を取込める");
    assert!(!imported.warnings.iter().any(|issue| {
        issue.path == "$.vertices_coords" && issue.message.contains("similarity")
    }));
    assert_eq!(imported.document.schema_version, SCHEMA_VERSION);
}

#[test]
fn all_unknown_extension_paths_flow_through_a_successful_conversion() {
    let file = parse_fold_1_2(EXTENSIONS).expect("extension fixtureをparseできる");
    let imported = fold_to_document(&file).expect("警告付き限定取込をできる");
    let extension_warnings = imported
        .warnings
        .iter()
        .filter(|issue| issue.path.starts_with("$.x_extension_"))
        .collect::<Vec<_>>();
    assert_eq!(extension_warnings.len(), 20);
    assert!(extension_warnings.iter().all(|issue| {
        issue.severity == FoldIssueSeverity::Warning && issue.original_value.is_some()
    }));
}

#[test]
fn validator_errors_stop_conversion_and_return_all_issues() {
    let file = parse_fold_1_2(UNSUPPORTED).expect("unsupported fixtureもtyped parseはできる");
    let error = fold_to_document(&file).expect_err("validator errorがあれば変換しない");
    assert!(!error.errors.is_empty());
    assert!(
        error
            .errors
            .iter()
            .any(|issue| issue.path.contains("vertices_coords"))
    );
    assert!(
        error
            .errors
            .iter()
            .any(|issue| issue.path.contains("frame_parent"))
    );
}

#[test]
fn endpoint_missing_mv_angle_is_an_error_not_an_assumed_zero() {
    let mut file = parse_fold_1_2(FLAT_ORDERS).expect("flat fixtureをparseできる");
    file.file_frames[0]
        .edges_fold_angle
        .as_mut()
        .expect("angle array")[4] = None;
    let error = fold_to_document(&file).expect_err("null M angleを0度と推測しない");
    assert!(error.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::MissingRequiredField
            && issue.path == "$.file_frames[0].edges_foldAngle[4]"
    }));
}

#[test]
fn changed_step_coordinates_are_rejected_instead_of_silently_dropped() {
    let file =
        parse_fold_1_2(CHANGED_STEP_COORDS).expect("changed-coordinate fixtureをparseできる");
    let error = fold_to_document(&file).expect_err("frame固有座標を黙って捨てない");
    assert!(error.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnsupportedGeometry
            && issue.path == "$.file_frames[0].vertices_coords[2]"
    }));
}

#[test]
fn inherited_face_order_error_uses_the_last_declared_source_path() {
    let file =
        parse_fold_1_2(INHERITED_ORDER_SOURCE).expect("inherited-order fixtureをparseできる");
    let error = fold_to_document(&file).expect_err("非平坦になった継承orderを拒否する");
    assert!(error.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnrepresentableFaceOrders
            && issue.path == "$.file_frames[0].faceOrders"
    }));
    assert!(
        !error
            .errors
            .iter()
            .any(|issue| issue.path == "$.file_frames[1].faceOrders")
    );
}

#[test]
fn inherited_null_angle_error_uses_the_root_source_path() {
    let file =
        parse_fold_1_2(INHERITED_NULL_ANGLE_SOURCE).expect("inherited-null fixtureをparseできる");
    let error = fold_to_document(&file).expect_err("継承null角を0度と推測しない");
    assert!(error.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::MissingRequiredField && issue.path == "$.edges_foldAngle[4]"
    }));
    assert!(
        !error
            .errors
            .iter()
            .any(|issue| issue.path == "$.file_frames[0].edges_foldAngle[4]")
    );
}

#[test]
fn corrupted_typed_values_return_errors_without_panicking() {
    let mut file = parse_fold_1_2(MINIMAL).expect("minimal fixtureをparseできる");
    file.root.edges_vertices.as_mut().expect("edge array")[4].clear();
    let caught = catch_unwind(AssertUnwindSafe(|| fold_to_document(&file)));
    assert!(caught.is_ok(), "壊れたtyped値でもpanicしない");
    let result = caught.expect("panic 0");
    assert!(result.is_err());
}

fn assert_close(actual: f64, expected: f64) {
    // The approved coordinate/angle roundtrip boundary is 1e-9. Test values are
    // exact/simple projections, so 1e-12 catches a real mapping error while leaving
    // three orders of magnitude before the product acceptance threshold.
    assert!(
        (actual - expected).abs() <= 1e-12,
        "actual={actual}, expected={expected}"
    );
}
