use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use glam::DVec3;
use ori3_cp::extract_faces;
use ori3_export::fold::{
    FoldAssignment, FoldIssueCode, document_to_fold, fold_to_document, parse_fold_1_2,
    write_fold_1_2,
};
use ori3_layers::{FlatState, replay};
use ori3_model::{Document, EdgeKind, FinishSoftSettings, TechniqueKind};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

const LINEAR: &str = include_str!("fixtures/fold/linear-steps.fold");
const FLAT_ORDERS: &str = include_str!("fixtures/fold/flat-face-orders.fold");
const FU: &str = include_str!("fixtures/fold/fu-assignments.fold");

#[test]
fn document_fold_document_preserves_topology_kinds_angles_steps_and_endpoints() {
    let original = import_linear_without_non_flat_orders();
    let exported = document_to_fold(&original).expect("linear Documentを書き出せる");
    assert!(exported.warnings.is_empty(), "{:?}", exported.warnings);
    assert_eq!(exported.file.file_frames.len(), original.sequence.len());
    for (index, frame) in exported.file.file_frames.iter().enumerate() {
        assert_eq!(frame.frame_parent, Some(index));
        assert_eq!(frame.frame_inherit, Some(true));
    }

    let json = write_fold_1_2(&exported.file).expect("FOLD JSONを書ける");
    let reparsed = parse_fold_1_2(&json).expect("書いたFOLD JSONを読める");
    let roundtrip = fold_to_document(&reparsed)
        .expect("書いたFOLDをDocumentへ戻せる")
        .document;

    assert_document_cp_equivalent(&original, &roundtrip);
    assert_eq!(roundtrip.sequence.len(), original.sequence.len());
    for (before, after) in original.sequence.iter().zip(&roundtrip.sequence) {
        assert_eq!(before.drivers.len(), after.drivers.len());
        for (before_driver, after_driver) in before.drivers.iter().zip(&after.drivers) {
            assert_point_close(before_driver.a, after_driver.a, 1e-9);
            assert_point_close(before_driver.b, after_driver.b, 1e-9);
            assert_close(
                before_driver.target_angle_deg,
                after_driver.target_angle_deg,
                1e-9,
            );
        }
    }

    assert_endpoint_geometry_equivalent(&original, &roundtrip);
}

#[test]
fn face_order_adjacent_triples_and_bottom_to_top_constraints_round_trip_exactly() {
    let file = parse_fold_1_2(FLAT_ORDERS).expect("flat faceOrders fixtureを読める");
    let first = fold_to_document(&file).expect("flat faceOrdersを取込める");
    let exported = document_to_fold(&first.document).expect("layer_orderを書き出せる");
    assert_eq!(
        exported.file.file_frames[0].face_orders.as_deref(),
        Some(&[vec![0, 1, 1]][..]),
        "FOLD +1はfirstがsecondより上、modelは下→上"
    );

    let json = write_fold_1_2(&exported.file).expect("faceOrders JSONを書ける");
    let reparsed = parse_fold_1_2(&json).expect("faceOrders JSONを読める");
    let second = fold_to_document(&reparsed).expect("faceOrdersを再取込できる");
    let before = first.document.sequence[0]
        .layer_order
        .as_ref()
        .expect("最初のlayer_order");
    let after = second.document.sequence[0]
        .layer_order
        .as_ref()
        .expect("往復後のlayer_order");
    assert_eq!(before.len(), after.len());
    for (&before, &after) in before.iter().zip(after) {
        assert_point_close(before, after, 1e-9);
    }

    let faces = extract_faces(&second.document.cp);
    let (resolved, warnings) = FlatState::resolve_order(&second.document.cp, &faces, after);
    assert!(warnings.is_empty());
    assert_eq!(resolved, vec![1, 0]);
}

#[test]
fn fu_become_aux_with_original_paths_and_export_never_invents_flat() {
    let mut file = parse_fold_1_2(FU).expect("F/U fixtureを読める");
    file.root.face_orders = None;
    let first = fold_to_document(&file).expect("F/Uを限定取込できる");
    let imported_downgrades = first
        .warnings
        .iter()
        .filter(|issue| issue.code == FoldIssueCode::AssignmentDowngradedToAux)
        .collect::<Vec<_>>();
    assert_eq!(imported_downgrades.len(), 2);
    assert_eq!(imported_downgrades[0].path, "$.edges_assignment[4]");
    assert_eq!(imported_downgrades[1].path, "$.edges_assignment[5]");
    assert_eq!(
        first.document.cp.edges[4].kind,
        EdgeKind::Aux,
        "FはAuxへ縮退する"
    );
    assert_eq!(
        first.document.cp.edges[5].kind,
        EdgeKind::Aux,
        "UもAuxへ縮退する"
    );

    let exported = document_to_fold(&first.document).expect("Auxを警告付きで書き出せる");
    let aux_warnings = exported
        .warnings
        .iter()
        .filter(|issue| issue.code == FoldIssueCode::AssignmentDowngradedToAux)
        .collect::<Vec<_>>();
    assert_eq!(aux_warnings.len(), 2);
    assert_eq!(aux_warnings[0].path, "$.cp.edges[4].kind");
    assert_eq!(aux_warnings[1].path, "$.cp.edges[5].kind");
    let assignments = exported
        .file
        .root
        .edges_assignment
        .as_ref()
        .expect("assignment array");
    assert_eq!(assignments[4], FoldAssignment::Unassigned);
    assert_eq!(assignments[5], FoldAssignment::Unassigned);
    assert!(!assignments.contains(&FoldAssignment::Flat));

    let json = write_fold_1_2(&exported.file).expect("Aux/U JSONを書ける");
    let second_file = parse_fold_1_2(&json).expect("Aux/U JSONを読める");
    let second = fold_to_document(&second_file).expect("Uを再取込できる");
    assert_eq!(second.document.cp.edges[4].kind, EdgeKind::Aux);
    assert_eq!(second.document.cp.edges[5].kind, EdgeKind::Aux);
    assert_eq!(
        second
            .warnings
            .iter()
            .filter(|issue| issue.code == FoldIssueCode::AssignmentDowngradedToAux)
            .count(),
        2,
        "往復後もUを黙って取込まない"
    );
}

#[test]
fn unsupported_document_step_meanings_are_returned_as_path_warnings() {
    let mut document = import_linear_without_non_flat_orders();
    document.sequence[0].kind = TechniqueKind::Pleat;
    document.sequence[0].note = "注記".to_string();
    document.sequence[0].finish_soft = Some(FinishSoftSettings::default());
    document.display.soft_enabled = true;
    document.display.front_color = [1, 2, 3];
    document.display.back_color = [4, 5, 6];
    document.display.grid_divisions += 1;
    document.display.overlap_prevention_enabled = !document.display.overlap_prevention_enabled;
    document.display.penetration_prevention_enabled =
        !document.display.penetration_prevention_enabled;

    let exported = document_to_fold(&document).expect("限定書出し自体はできる");
    let paths = exported
        .warnings
        .iter()
        .map(|issue| issue.path.as_str())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"$.sequence[0].kind"));
    assert!(paths.contains(&"$.sequence[0].note"));
    assert!(paths.contains(&"$.sequence[0].finish_soft"));
    assert!(paths.contains(&"$.display"));
    assert!(paths.contains(&"$.display.front_color"));
    assert!(paths.contains(&"$.display.back_color"));
    assert!(paths.contains(&"$.display.grid_divisions"));
    assert!(paths.contains(&"$.display.overlap_prevention_enabled"));
    assert!(paths.contains(&"$.display.penetration_prevention_enabled"));
    assert!(
        exported
            .warnings
            .iter()
            .all(|issue| issue.original_value.is_some())
    );
}

#[test]
fn physical_paper_size_loss_is_returned_with_path_and_original_value() {
    let mut document = import_linear_without_non_flat_orders();
    document.paper.width_mm = 150.0;
    document.paper.height_mm = 100.0;

    let exported = document_to_fold(&document).expect("物理寸法以外は限定書出しできる");
    let warning = exported
        .warnings
        .iter()
        .find(|issue| issue.path == "$.paper")
        .expect("物理的な紙寸法の非保持を警告する");
    assert_eq!(warning.code, FoldIssueCode::UnsupportedField);
    assert_eq!(
        warning.original_value,
        Some(serde_json::json!({ "width_mm": 150.0, "height_mm": 100.0 }))
    );
}

#[test]
fn later_driver_for_the_same_edge_is_the_endpoint_authority() {
    let mut document = import_linear_without_non_flat_orders();
    let mut later = document.sequence[0].drivers[0].clone();
    later.target_angle_deg = -45.0;
    document.sequence[0].drivers.push(later);

    let exported = document_to_fold(&document).expect("replayと同じ後勝ちで書き出せる");
    let angles = exported.file.file_frames[0]
        .edges_fold_angle
        .as_ref()
        .expect("endpoint angle snapshot");
    assert_close(angles[4].expect("V angle"), 45.0, 1e-9);
}

#[test]
fn noncanonical_document_coordinates_are_rejected_instead_of_breaking_roundtrip_epsilon() {
    let mut document = import_linear_without_non_flat_orders();
    for vertex in &mut document.cp.vertices {
        vertex.pos = [10.0 - vertex.pos[1], 20.0 + vertex.pos[0]];
    }
    for step in &mut document.sequence {
        for driver in &mut step.drivers {
            driver.a = [10.0 - driver.a[1], 20.0 + driver.a[0]];
            driver.b = [10.0 - driver.b[1], 20.0 + driver.b[0]];
        }
    }

    let error = document_to_fold(&document)
        .expect_err("非canonical座標を黙って正規化して往復成功扱いにしない");
    assert!(error.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnsupportedGeometry
            && issue.path.starts_with("$.cp.vertices[")
            && issue.message.contains("canonical紙座標")
    }));
}

#[test]
fn export_warning_paths_follow_source_vectors_even_when_ids_are_unsorted() {
    let mut file = parse_fold_1_2(FU).expect("F/U fixtureを読める");
    file.root.face_orders = None;
    let mut document = fold_to_document(&file)
        .expect("F/UをDocumentへ変換できる")
        .document;
    document.cp.edges.swap(0, 4);

    let exported = document_to_fold(&document).expect("unsorted IDもcanonicalに書き出せる");
    let aux = exported
        .warnings
        .iter()
        .filter(|issue| issue.code == FoldIssueCode::AssignmentDowngradedToAux)
        .collect::<Vec<_>>();
    assert_eq!(aux.len(), 2);
    assert_eq!(aux[0].path, "$.cp.edges[0].kind");
    assert_eq!(aux[1].path, "$.cp.edges[5].kind");
}

#[test]
fn face_orders_that_conflict_with_flat_mountain_valley_are_not_accepted() {
    let mut file = parse_fold_1_2(FLAT_ORDERS).expect("flat faceOrders fixtureを読める");
    file.file_frames[0].face_orders = Some(vec![vec![1, 0, 1]]);
    let error = fold_to_document(&file).expect_err("山谷と逆の上下制約を成功扱いにしない");
    assert!(error.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnrepresentableFaceOrders
            && issue.path == "$.file_frames[0].faceOrders"
            && issue.message.contains("山谷と矛盾")
    }));
}

#[test]
fn non_flat_layer_order_is_rejected_with_count_and_reason_without_panicking() {
    let mut document = import_linear_without_non_flat_orders();
    let faces = extract_faces(&document.cp);
    document.sequence[0].layer_order = Some(
        faces
            .iter()
            .map(|face| ori3_layers::representative_point(&document.cp, face))
            .collect(),
    );

    let caught = catch_unwind(AssertUnwindSafe(|| document_to_fold(&document)));
    assert!(caught.is_ok(), "表現不能Documentでもpanic 0");
    let error = caught
        .expect("panicしない")
        .expect_err("非平坦layer_orderを近似しない");
    assert!(!error.errors.is_empty());
    assert!(error.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::UnrepresentableFaceOrders
            && issue.path == "$.sequence[0].layer_order"
            && issue.message.contains("非平坦")
    }));
}

#[test]
fn malformed_document_numbers_return_errors_without_panicking() {
    let mut document = import_linear_without_non_flat_orders();
    document.cp.vertices[0].pos[0] = f64::NAN;

    let caught = catch_unwind(AssertUnwindSafe(|| document_to_fold(&document)));
    assert!(caught.is_ok(), "非有限Documentでもpanic 0");
    let error = caught
        .expect("panicしない")
        .expect_err("非有限座標をFOLDへ書かない");
    assert!(error.errors.iter().any(|issue| {
        issue.code == FoldIssueCode::InvalidValue
            && issue.path == "$.cp.vertices[0].pos"
            && issue.original_value.is_some()
    }));
}

#[test]
fn repeated_export_is_byte_deterministic_and_does_not_mutate_the_document() {
    let document = import_linear_without_non_flat_orders();
    let before = document.clone();
    let mut outputs = BTreeMap::new();
    for iteration in 0..10 {
        let exported = document_to_fold(&document).expect("deterministic export");
        let json = write_fold_1_2(&exported.file).expect("deterministic JSON");
        outputs.insert(json, iteration);
    }
    assert_eq!(outputs.len(), 1);
    assert_eq!(document, before);
}

fn import_linear_without_non_flat_orders() -> Document {
    let mut file = parse_fold_1_2(LINEAR).expect("linear fixtureを読める");
    file.root.face_orders = None;
    for frame in &mut file.file_frames {
        frame.face_orders = None;
    }
    fold_to_document(&file)
        .expect("partial-angle linear framesを取込める")
        .document
}

fn assert_document_cp_equivalent(before: &Document, after: &Document) {
    assert_eq!(before.cp.vertices.len(), after.cp.vertices.len());
    for (before, after) in before.cp.vertices.iter().zip(&after.cp.vertices) {
        assert_eq!(before.id, after.id);
        assert_point_close(before.pos, after.pos, 1e-9);
    }
    assert_eq!(before.cp.edges, after.cp.edges);
}

fn assert_endpoint_geometry_equivalent(before: &Document, after: &Document) {
    let before_faces = extract_faces(&before.cp);
    let after_faces = extract_faces(&after.cp);
    assert_eq!(before_faces, after_faces);
    for up_to in 1..=before.sequence.len() {
        let left = replay(before, up_to, 1.0);
        let right = replay(after, up_to, 1.0);
        assert!(
            left.skipped.is_empty(),
            "before step {up_to}: {:?}",
            left.skipped
        );
        assert!(
            right.skipped.is_empty(),
            "after step {up_to}: {:?}",
            right.skipped
        );
        assert_eq!(left.frame.faces.len(), right.frame.faces.len());
        for (left_face, right_face) in left.frame.faces.iter().zip(&right.frame.faces) {
            assert_eq!(left_face.face, right_face.face);
            assert_eq!(left_face.polygon.len(), right_face.polygon.len());
            for (&left_point, &right_point) in left_face.polygon.iter().zip(&right_face.polygon) {
                assert!(left_point.into_iter().all(f64::is_finite));
                assert!(right_point.into_iter().all(f64::is_finite));
                let distance = (DVec3::from(left_point) - DVec3::from(right_point)).length();
                // replayのflat/endpoint数値境界と§12.6はいずれも1e-6。
                assert!(distance <= 1e-6, "step {up_to}: distance={distance:e}");
            }
        }
        assert!(max_seam_gap(&before.cp, &before_faces, &left.frame) <= 1e-6);
        assert!(max_seam_gap(&after.cp, &after_faces, &right.frame) <= 1e-6);
        assert!(self_intersection_pairs(&left.frame).is_empty());
        assert!(self_intersection_pairs(&right.frame).is_empty());
    }
}

fn assert_point_close(actual: [f64; 2], expected: [f64; 2], epsilon: f64) {
    assert_close(actual[0], expected[0], epsilon);
    assert_close(actual[1], expected[1], epsilon);
}

fn assert_close(actual: f64, expected: f64, epsilon: f64) {
    // Coordinates and angles use the model/roadmap 1e-9 boundary, never exact f64 equality.
    assert!(
        (actual - expected).abs() <= epsilon,
        "actual={actual}, expected={expected}, epsilon={epsilon:e}"
    );
}
