use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_geometry::Isometry2;
use ori3_layers::{
    COMPLETE_FOLD_ENDPOINT_EPS_DEG, FlatState, FoldDirection, FoldLineSection, FoldTargetAnalysis,
    FoldThroughInput, FullFoldSign, HingeObservation, PleatAnalysis, PleatAnalysisError, PleatPair,
    PleatSectionAnalysis, analyze_fold_target_at_state, analyze_pleats,
    flat_state_with_declared_angles_at, fold_through, representative_point,
    target_faces_for_pleat_count,
};
use ori3_model::{CreasePattern, Document, Edge, EdgeId, EdgeKind, FaceId, Paper, Vertex};

fn half_fold_fixture(
    angle_deg: f64,
) -> (
    ori3_model::CreasePattern,
    Vec<Face>,
    FlatState,
    HashMap<EdgeId, f64>,
    FaceId,
    FaceId,
) {
    let mut cp = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
    .cp;
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Valley);
    let faces = extract_faces(&cp);
    let left = faces
        .iter()
        .find(|face| representative_point(&cp, face)[0] < 0.5)
        .expect("left face")
        .id;
    let right = faces
        .iter()
        .find(|face| representative_point(&cp, face)[0] > 0.5)
        .expect("right face")
        .id;
    let mirror = Isometry2::reflection(DVec2::new(0.5, 0.0), DVec2::new(0.5, 1.0));
    let state = FlatState {
        placements: HashMap::from([(left, Isometry2::identity()), (right, mirror)]),
        // FlatState order is bottom-to-top. The bridge must reverse this for FoldLineSection.
        order: vec![left, right],
    };
    let hinge = cp
        .edges
        .iter()
        .find(|edge| edge.kind == EdgeKind::Valley)
        .expect("inserted hinge")
        .id;
    (
        cp,
        faces,
        state,
        HashMap::from([(hinge, angle_deg)]),
        left,
        right,
    )
}

#[test]
fn bridge_reverses_bottom_to_top_order_and_one_pleat_selects_both_surfaces() {
    let (cp, faces, state, angles, left, right) = half_fold_fixture(180.0);
    let actual = analyze_fold_target_at_state(
        &cp,
        &faces,
        &state,
        &angles,
        [[0.0, 0.5], [0.5, 0.5]],
        [0.25, 0.75],
    )
    .expect("the explicit canonical flat fixture is analyzable");

    assert_eq!(actual.pleats.scalar_count, Some(1));
    assert_eq!(
        actual.section_surfaces_top_to_bottom,
        vec![vec![vec![right], vec![left]]],
        "FoldedQuery/FlatState is bottom-to-top, but pleat analysis is top-to-bottom",
    );
    assert_eq!(
        actual.pleats.sections[0].pairs_top_to_bottom[0],
        PleatPair {
            hinge_faces: (right, left),
            upper_surface_faces: vec![right],
            lower_surface_faces: vec![left],
            sign: FullFoldSign::Positive180,
        },
    );
    let mut selected = target_faces_for_pleat_count(&actual, 1).expect("select one pleat");
    selected.sort_unstable();
    let mut expected = vec![left, right];
    expected.sort_unstable();
    assert_eq!(selected, expected, "one pleat is not inferred as one Face");
}

#[test]
fn a_zero_degree_surface_repeated_in_one_local_stack_is_unavailable() {
    let (cp, faces, mut state, angles, left, right) = half_fold_fixture(0.0);
    state.placements.insert(
        right,
        Isometry2 {
            rotation: 0.0,
            translation: DVec2::new(-0.5, 0.0),
            mirrored: false,
        },
    );
    state.order = vec![left, right];

    assert_eq!(
        analyze_fold_target_at_state(
            &cp,
            &faces,
            &state,
            &angles,
            [[0.0, 0.5], [0.5, 0.5]],
            [0.25, 0.75],
        ),
        Err(PleatAnalysisError::RepeatedSurfaceInSection),
        "A...A in one local stack must not be silently deduplicated",
    );
}

#[test]
fn section_specific_top_pairs_that_cannot_be_one_whole_face_set_fail_closed() {
    let pair = |upper, lower| PleatPair {
        hinge_faces: (upper, lower),
        upper_surface_faces: vec![upper],
        lower_surface_faces: vec![lower],
        sign: FullFoldSign::Positive180,
    };
    let analysis = FoldTargetAnalysis {
        pleats: PleatAnalysis {
            scalar_count: Some(1),
            sections: vec![
                PleatSectionAnalysis {
                    pairs_top_to_bottom: vec![pair(1, 2)],
                    ..PleatSectionAnalysis::default()
                },
                PleatSectionAnalysis {
                    pairs_top_to_bottom: vec![pair(3, 4)],
                    ..PleatSectionAnalysis::default()
                },
            ],
            reason: None,
        },
        // Face 1 is selected in section 1 but lies below the selected pair in section 2.
        section_surfaces_top_to_bottom: vec![
            vec![vec![1], vec![2], vec![3], vec![4]],
            vec![vec![3], vec![4], vec![1], vec![2]],
        ],
    };

    assert_eq!(
        target_faces_for_pleat_count(&analysis, 1),
        Err(PleatAnalysisError::UnsafeWholeFaceSelection),
        "whole-Face targeting must fail closed when one union would select deeper paper elsewhere",
    );
}

#[test]
fn every_section_must_have_the_same_surface_pair_identity_sign_and_order() {
    let pair = |upper, lower, sign| PleatPair {
        hinge_faces: (upper, lower),
        upper_surface_faces: vec![upper],
        lower_surface_faces: vec![lower],
        sign,
    };
    let analysis = |sections: Vec<PleatSectionAnalysis>, surfaces| FoldTargetAnalysis {
        pleats: PleatAnalysis {
            scalar_count: sections
                .first()
                .map(|section| section.pairs_top_to_bottom.len()),
            sections,
            reason: None,
        },
        section_surfaces_top_to_bottom: surfaces,
    };
    let cases = [
        (
            analysis(
                vec![
                    PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![pair(1, 2, FullFoldSign::Positive180)],
                        ..PleatSectionAnalysis::default()
                    },
                    PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![pair(3, 4, FullFoldSign::Positive180)],
                        ..PleatSectionAnalysis::default()
                    },
                ],
                vec![vec![vec![1], vec![2]], vec![vec![3], vec![4]]],
            ),
            1,
            "different surface identities",
        ),
        (
            analysis(
                vec![
                    PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![pair(1, 2, FullFoldSign::Positive180)],
                        ..PleatSectionAnalysis::default()
                    },
                    PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![pair(1, 2, FullFoldSign::Negative180)],
                        ..PleatSectionAnalysis::default()
                    },
                ],
                vec![vec![vec![1], vec![2]], vec![vec![1], vec![2]]],
            ),
            1,
            "different pair signs",
        ),
        (
            analysis(
                vec![
                    PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![
                            pair(1, 2, FullFoldSign::Positive180),
                            pair(3, 4, FullFoldSign::Positive180),
                        ],
                        boundary_signs_between_pairs: vec![FullFoldSign::Positive180],
                        ..PleatSectionAnalysis::default()
                    },
                    PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![
                            pair(3, 4, FullFoldSign::Positive180),
                            pair(1, 2, FullFoldSign::Positive180),
                        ],
                        boundary_signs_between_pairs: vec![FullFoldSign::Positive180],
                        ..PleatSectionAnalysis::default()
                    },
                ],
                vec![
                    vec![vec![1], vec![2], vec![3], vec![4]],
                    vec![vec![3], vec![4], vec![1], vec![2]],
                ],
            ),
            2,
            "different pair order",
        ),
        (
            analysis(
                vec![
                    PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![
                            pair(1, 2, FullFoldSign::Positive180),
                            pair(3, 4, FullFoldSign::Positive180),
                        ],
                        boundary_signs_between_pairs: vec![FullFoldSign::Positive180],
                        ..PleatSectionAnalysis::default()
                    },
                    PleatSectionAnalysis {
                        pairs_top_to_bottom: vec![
                            pair(1, 2, FullFoldSign::Positive180),
                            pair(3, 4, FullFoldSign::Positive180),
                        ],
                        boundary_signs_between_pairs: vec![FullFoldSign::Negative180],
                        ..PleatSectionAnalysis::default()
                    },
                ],
                vec![
                    vec![vec![1], vec![2], vec![3], vec![4]],
                    vec![vec![1], vec![2], vec![3], vec![4]],
                ],
            ),
            2,
            "different connector signs",
        ),
    ];

    for (analysis, count, label) in cases {
        assert_eq!(
            target_faces_for_pleat_count(&analysis, count),
            Err(PleatAnalysisError::UnsafeWholeFaceSelection),
            "{label} must not be reduced to one scalar K",
        );
    }
}

#[test]
fn one_pleat_with_a_split_surface_selects_three_faces_instead_of_two_times_k() {
    let pleats = analyze_pleats(
        &[FoldLineSection {
            faces_top_to_bottom: vec![20, 22],
            hinges: vec![
                HingeObservation {
                    face_a: 20,
                    face_b: 21,
                    angle_deg: 0.0,
                },
                HingeObservation {
                    face_a: 21,
                    face_b: 22,
                    angle_deg: 180.0,
                },
            ],
        }],
        COMPLETE_FOLD_ENDPOINT_EPS_DEG,
    )
    .expect("the zero-degree split is one surface");
    let analysis = FoldTargetAnalysis {
        pleats,
        section_surfaces_top_to_bottom: vec![vec![vec![20, 21], vec![22]]],
    };

    assert_eq!(
        target_faces_for_pleat_count(&analysis, 1),
        Ok(vec![20, 21, 22]),
        "one pleat is a pair of surfaces, not two Face IDs",
    );
}

#[test]
fn nonzero_document_without_saved_layer_order_does_not_use_face_id_order_for_top_k() {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let faces_before = extract_faces(&document.cp);
    let state_before = FlatState::initial(&document.cp, &faces_before);
    let result = fold_through(
        &mut document.cp,
        &faces_before,
        &state_before,
        &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            keep_side_point: [0.25, 0.5],
            target_layers: None,
            direction: FoldDirection::Down,
        },
    )
    .expect("build a valid fold whose canonical order is not FaceId order");
    let canonical_order = result.state.order.clone();
    let mut face_id_order = canonical_order.clone();
    face_id_order.sort_unstable();
    assert_ne!(
        canonical_order, face_id_order,
        "fixture must distinguish the two orders"
    );
    let mut step = result.step;
    step.layer_order = None;
    document.sequence.push(step);
    let faces_after = extract_faces(&document.cp);

    let (actual, _, _) = flat_state_with_declared_angles_at(&document, &faces_after, 1)
        .expect("document-only canonical surface rank supplies the missing order");
    assert_eq!(
        actual.order, canonical_order,
        "fold-target replay must use canonical surface rank instead of FaceId order",
    );
}

#[test]
fn concave_face_keeps_two_disconnected_fold_line_sections() {
    let cp = concave_u_pattern();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1);
    let face = faces[0].id;
    let state = FlatState::initial(&cp, &faces);

    let actual = analyze_fold_target_at_state(
        &cp,
        &faces,
        &state,
        &HashMap::new(),
        [[0.0, 2.0], [3.0, 2.0]],
        [1.5, 0.0],
    )
    .expect("the two U arms are separate intervals on the moving side");

    assert_eq!(
        actual.section_surfaces_top_to_bottom,
        vec![vec![vec![face]], vec![vec![face]]],
        "the empty slit between the U arms must not be bridged or deduplicated",
    );
}

fn concave_u_pattern() -> CreasePattern {
    let positions = [
        [0.0, 0.0],
        [3.0, 0.0],
        [3.0, 3.0],
        [2.0, 3.0],
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 3.0],
        [0.0, 3.0],
    ];
    let vertices = positions
        .into_iter()
        .enumerate()
        .map(|(index, pos)| Vertex {
            id: u32::try_from(index).expect("vertex id"),
            pos,
        })
        .collect::<Vec<_>>();
    let edges = (0..vertices.len())
        .map(|index| Edge {
            id: u32::try_from(index).expect("edge id"),
            v0: u32::try_from(index).expect("edge start"),
            v1: u32::try_from((index + 1) % vertices.len()).expect("edge end"),
            kind: EdgeKind::Border,
        })
        .collect::<Vec<_>>();
    CreasePattern {
        next_vertex_id: u32::try_from(vertices.len()).expect("next vertex id"),
        next_edge_id: u32::try_from(edges.len()).expect("next edge id"),
        vertices,
        edges,
    }
}
