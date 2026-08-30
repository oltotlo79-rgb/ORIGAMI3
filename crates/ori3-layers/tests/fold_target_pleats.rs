//! 「選んだ折り線の直下にあるひだ」を数える契約の先行検査。
//!
//! 段階1では製品APIの `Err(NotImplemented)` と `Ok(期待値)` の差として実行時の赤を確認した。
//! 段階2ではその期待値を変えず、製品APIの実装によって緑にする。

use ori3_layers::{
    COMPLETE_FOLD_ENDPOINT_EPS_DEG, FoldLineSection, FullFoldSign, HingeObservation, PleatAnalysis,
    PleatAnalysisError, PleatCountLimit, PleatPair, PleatSectionAnalysis, TopAction,
    analyze_pleats, analyze_single_section_from_top,
};
use ori3_model::FaceId;

fn section(faces_top_to_bottom: &[FaceId], hinges: &[(FaceId, FaceId, f64)]) -> FoldLineSection {
    FoldLineSection {
        faces_top_to_bottom: faces_top_to_bottom.to_vec(),
        hinges: hinges
            .iter()
            .map(|&(face_a, face_b, angle_deg)| HingeObservation {
                face_a,
                face_b,
                angle_deg,
            })
            .collect(),
    }
}

#[test]
fn conflicting_fragments_between_the_same_surfaces_are_ambiguous_in_any_input_order() {
    let cases = [
        vec![(10, 11, 180.0), (10, 11, 90.0)],
        vec![(10, 11, 90.0), (10, 11, 180.0)],
        vec![(10, 11, 180.0), (10, 11, -180.0)],
        vec![(10, 11, -180.0), (10, 11, 180.0)],
        vec![(10, 11, 0.0), (10, 11, 180.0)],
        vec![(10, 11, 180.0), (10, 11, 0.0)],
        vec![(10, 11, 0.0), (10, 11, -180.0)],
        vec![(10, 11, -180.0), (10, 11, 0.0)],
    ];

    for hinges in cases {
        assert_eq!(
            analyze_pleats(
                &[section(&[10, 11], &hinges)],
                COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            ),
            Err(PleatAnalysisError::AmbiguousRelation),
            "all fragments between one surface pair must agree on one signed complete fold",
        );
    }
}

#[test]
fn a_complete_fragment_inside_one_zero_degree_surface_is_ambiguous() {
    assert_eq!(
        analyze_pleats(
            &[section(
                &[10, 11, 12],
                &[(10, 11, 0.0), (11, 12, 0.0), (10, 12, 180.0)],
            )],
            COMPLETE_FOLD_ENDPOINT_EPS_DEG,
        ),
        Err(PleatAnalysisError::AmbiguousRelation),
        "a non-zero fragment must not disappear inside a surface joined by zero-degree fragments",
    );
}

#[test]
fn conflicting_fragments_between_split_surface_groups_are_ambiguous() {
    let relations = [
        [(10, 20, 180.0), (11, 21, -180.0)],
        [(11, 21, -180.0), (10, 20, 180.0)],
    ];
    for relation_order in relations {
        let hinges = [
            (10, 11, 0.0),
            (20, 21, 0.0),
            relation_order[0],
            relation_order[1],
        ];
        assert_eq!(
            analyze_pleats(
                &[section(&[10, 20], &hinges)],
                COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            ),
            Err(PleatAnalysisError::AmbiguousRelation),
            "different raw Face pairs between the same two surfaces must agree in either input order",
        );
    }
}

fn pair(
    hinge_faces: (FaceId, FaceId),
    upper_surface_faces: &[FaceId],
    lower_surface_faces: &[FaceId],
    sign: FullFoldSign,
) -> PleatPair {
    PleatPair {
        hinge_faces,
        upper_surface_faces: upper_surface_faces.to_vec(),
        lower_surface_faces: lower_surface_faces.to_vec(),
        sign,
    }
}

fn section_analysis(
    pairs_top_to_bottom: Vec<PleatPair>,
    boundary_signs_between_pairs: Vec<FullFoldSign>,
    top_action: Option<TopAction>,
    count_limit: Option<PleatCountLimit>,
) -> PleatSectionAnalysis {
    PleatSectionAnalysis {
        pairs_top_to_bottom,
        boundary_signs_between_pairs,
        top_action,
        count_limit,
    }
}

fn expected(
    scalar_count: Option<usize>,
    sections: Vec<PleatSectionAnalysis>,
    reason: Option<&str>,
) -> PleatAnalysis {
    PleatAnalysis {
        scalar_count,
        sections,
        reason: reason.map(str::to_owned),
    }
}

fn one_section(
    pairs_top_to_bottom: Vec<PleatPair>,
    boundary_signs_between_pairs: Vec<FullFoldSign>,
    top_action: Option<TopAction>,
    count_limit: Option<PleatCountLimit>,
) -> PleatAnalysis {
    let count = pairs_top_to_bottom.len();
    expected(
        Some(count),
        vec![section_analysis(
            pairs_top_to_bottom,
            boundary_signs_between_pairs,
            top_action,
            count_limit,
        )],
        None,
    )
}

fn crease_only(surface_faces: &[FaceId]) -> PleatAnalysis {
    one_section(
        vec![],
        vec![],
        Some(TopAction::CreaseOnlyTop {
            surface_faces: surface_faces.to_vec(),
        }),
        None,
    )
}

#[test]
fn half_fold_with_two_faces_is_one_pleat() {
    let actual = analyze_pleats(
        &[section(&[10, 11], &[(10, 11, 180.0)])],
        COMPLETE_FOLD_ENDPOINT_EPS_DEG,
    );

    assert_eq!(
        actual,
        Ok(one_section(
            vec![pair((10, 11), &[10], &[11], FullFoldSign::Positive180,)],
            vec![],
            None,
            None,
        )),
        "半折りの表裏2面は1ひだ"
    );
}

#[test]
fn one_surface_has_no_pleat_action_because_there_is_no_paper_below_it() {
    assert_eq!(
        analyze_pleats(&[section(&[10], &[])], COMPLETE_FOLD_ENDPOINT_EPS_DEG,),
        Ok(one_section(vec![], vec![], None, None)),
        "a lone sheet remains available to the pre-existing all/top fold path",
    );
}

#[test]
fn hinges_outside_the_local_fold_line_stack_do_not_invalidate_the_section() {
    let actual = analyze_pleats(
        &[section(&[10, 11], &[(10, 11, 180.0), (20, 21, 180.0)])],
        COMPLETE_FOLD_ENDPOINT_EPS_DEG,
    );

    assert_eq!(
        actual,
        Ok(one_section(
            vec![pair((10, 11), &[10], &[11], FullFoldSign::Positive180,)],
            vec![],
            None,
            None,
        )),
        "a section receives document-wide hinge observations but counts only its local stack",
    );
}

#[test]
fn zero_degree_face_split_does_not_increase_the_pleat_count() {
    // Face 20と21は0°で連続する同じ表面。局所点には20だけが現れるが、
    // 21と22を結ぶ+180°をsurface group間の1組として数えなければならない。
    let actual = analyze_pleats(
        &[section(&[20, 22], &[(20, 21, 0.0), (21, 22, 180.0)])],
        COMPLETE_FOLD_ENDPOINT_EPS_DEG,
    );

    assert_eq!(
        actual,
        Ok(one_section(
            vec![pair((21, 22), &[20, 21], &[22], FullFoldSign::Positive180,)],
            vec![],
            None,
            None,
        )),
        "0°分割で3 Faceになっても、surface group [20, 21] と [22] の1ひだ"
    );
}

#[test]
fn full_hinge_chain_uses_top_down_greedy_non_overlapping_pairs() {
    // 利用者決定: ABを1組として消費した後は、BをBCへ再利用しない。BCはpair間の
    // 境目として同じ許容差で確認し、その先のCDを2組目として数える。
    let boundary_inside_tolerance = -180.0 + 0.5 * COMPLETE_FOLD_ENDPOINT_EPS_DEG;
    let actual = analyze_pleats(
        &[section(
            &[30, 31, 32, 33],
            &[
                (30, 31, 180.0),
                (31, 32, boundary_inside_tolerance),
                (32, 33, 180.0),
            ],
        )],
        COMPLETE_FOLD_ENDPOINT_EPS_DEG,
    );

    assert_eq!(
        actual,
        Ok(one_section(
            vec![
                pair((30, 31), &[30], &[31], FullFoldSign::Positive180,),
                pair((32, 33), &[32], &[33], FullFoldSign::Positive180,),
            ],
            vec![FullFoldSign::Negative180],
            None,
            None,
        )),
        "ABとCDをこの順で2ひだ、許容差の半分だけ内側の負符号BCを境目として保つ"
    );
}

#[test]
fn incomplete_top_stops_before_all_faces_below_and_requests_crease_only() {
    let mut observed_calls = Vec::new();
    let actual = analyze_single_section_from_top(
        &[40, 41, 42, 43],
        COMPLETE_FOLD_ENDPOINT_EPS_DEG,
        |face_a, face_b| {
            observed_calls.push((face_a, face_b));
            match (face_a, face_b) {
                (40, 41) => Some(90.0),
                (41, 42) | (42, 43) => {
                    panic!("最上紙が90°と分かった後に、B以降の関係を探索してはいけない")
                }
                other => panic!("想定していない表裏pairへ問い合わせました: {other:?}"),
            }
        },
    );

    assert_eq!(
        (actual, observed_calls),
        (Ok(crease_only(&[40])), vec![(40, 41)]),
        "最上紙が90°ならFace 40へ折り目だけを付け、B以降を一切読まない"
    );
}

#[test]
fn incomplete_boundary_angles_after_one_pair_stop_before_the_next_pair() {
    let cases = [
        (90.0, "90°"),
        (179.0, "+179°"),
        (-179.0, "-179°"),
        (
            180.0 - 2.0 * COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            "+180°終端から2e-9°外",
        ),
        (
            -180.0 + 2.0 * COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            "-180°終端から2e-9°外",
        ),
    ];

    let actual_cases = cases.map(|(boundary_angle, label)| {
        let mut observed_calls = Vec::new();
        let actual = analyze_single_section_from_top(
            &[100, 101, 102, 103],
            COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            |face_a, face_b| {
                observed_calls.push((face_a, face_b));
                match (face_a, face_b) {
                    (100, 101) => Some(180.0),
                    (101, 102) => Some(boundary_angle),
                    (102, 103) => {
                        panic!("BC境界が{label}と分かった後に、下のCD pairを探索してはいけない")
                    }
                    other => panic!("想定していない表裏pairへ問い合わせました: {other:?}"),
                }
            },
        );
        (label, actual, observed_calls)
    });

    let expected_analysis = Ok(one_section(
        vec![pair((100, 101), &[100], &[101], FullFoldSign::Positive180)],
        vec![],
        None,
        Some(PleatCountLimit::IncompleteBoundaryAfter { count: 1 }),
    ));
    let expected_cases = cases.map(|(_, label)| {
        (
            label,
            expected_analysis.clone(),
            vec![(100, 101), (101, 102)],
        )
    });

    assert_eq!(
        actual_cases, expected_cases,
        "ABの1ひだ後、5種類の未完なBC境界で打ち切り、CDを読まない"
    );
}

#[test]
fn six_surfaces_stop_after_two_pairs_at_the_first_incomplete_boundary() {
    // 別検査の負符号connectorと対にし、許容差の半分だけ内側の正符号も固定する。
    let mut observed_calls = Vec::new();
    let actual = analyze_single_section_from_top(
        &[110, 111, 112, 113, 114, 115],
        COMPLETE_FOLD_ENDPOINT_EPS_DEG,
        |face_a, face_b| {
            observed_calls.push((face_a, face_b));
            match (face_a, face_b) {
                (110, 111) => Some(180.0),
                (111, 112) => Some(180.0 - 0.5 * COMPLETE_FOLD_ENDPOINT_EPS_DEG),
                (112, 113) => Some(180.0),
                (113, 114) => Some(90.0),
                (114, 115) => {
                    panic!("DE境界が90°と分かった後に、下のEF pairを探索してはいけない")
                }
                other => panic!("想定していない表裏pairへ問い合わせました: {other:?}"),
            }
        },
    );

    assert_eq!(
        (actual, observed_calls),
        (
            Ok(one_section(
                vec![
                    pair((110, 111), &[110], &[111], FullFoldSign::Positive180,),
                    pair((112, 113), &[112], &[113], FullFoldSign::Positive180,),
                ],
                vec![FullFoldSign::Positive180],
                None,
                Some(PleatCountLimit::IncompleteBoundaryAfter { count: 2 }),
            )),
            vec![(110, 111), (111, 112), (112, 113), (113, 114)],
        ),
        "AB・許容差の半分だけ内側の正符号BC・CDを確認して2ひだ、未完DEで止まりEFを読まない"
    );
}

#[test]
fn endpoint_errors_are_complete_and_positive_negative_signs_stay_distinct() {
    // 実測最大誤差2例に加え、許容差の半分だけ内側の正負2例を一括で固定する。
    // これにより、実装が許容差を誤って1e-12等へ狭めても通らない。
    let cases = [
        (
            50,
            51,
            180.0 - 1.0342061893570844e-13,
            FullFoldSign::Positive180,
            "+180°側の実機最大誤差",
        ),
        (
            60,
            61,
            -180.0 + 4.1702290600902744e-13,
            FullFoldSign::Negative180,
            "-180°側のDocument-only最大誤差",
        ),
        (
            52,
            53,
            180.0 - 0.5 * COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            FullFoldSign::Positive180,
            "+180°から許容差の半分だけ内側",
        ),
        (
            62,
            63,
            -180.0 + 0.5 * COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            FullFoldSign::Negative180,
            "-180°から許容差の半分だけ内側",
        ),
    ];

    let actual_cases = cases.map(|(upper, lower, angle_deg, sign, label)| {
        (
            label,
            analyze_pleats(
                &[section(&[upper, lower], &[(upper, lower, angle_deg)])],
                COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            ),
            sign,
        )
    });
    let expected_cases = cases.map(|(upper, lower, _, sign, label)| {
        (
            label,
            Ok(one_section(
                vec![pair((upper, lower), &[upper], &[lower], sign)],
                vec![],
                None,
                None,
            )),
            sign,
        )
    });

    assert_eq!(
        actual_cases, expected_cases,
        "実測誤差内と許容差の半分内の正負4例は、すべて1ひだで符号を保つ"
    );
}

#[test]
fn unfinished_angles_and_values_outside_endpoint_tolerance_are_crease_only() {
    let cases = [
        (70, 71, 179.0, "+179°"),
        (72, 73, -179.0, "-179°"),
        (
            74,
            75,
            180.0 - 2.0 * COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            "+180°終端から2e-9°外",
        ),
        (
            76,
            77,
            -180.0 + 2.0 * COMPLETE_FOLD_ENDPOINT_EPS_DEG,
            "-180°終端から2e-9°外",
        ),
    ];

    for (upper, lower, angle_deg, label) in cases {
        let actual = analyze_pleats(
            &[section(&[upper, lower], &[(upper, lower, angle_deg)])],
            COMPLETE_FOLD_ENDPOINT_EPS_DEG,
        );

        assert_eq!(
            actual,
            Ok(crease_only(&[upper])),
            "{label}は0ひだで、最上Face {upper}へ折り目だけ"
        );
    }
}

#[test]
fn differing_sections_have_no_scalar_count_and_return_the_exact_reason() {
    let boundary_document_only = -180.0 + 4.1702290600902744e-13;
    let actual = analyze_pleats(
        &[
            section(&[80, 81], &[(80, 81, 180.0)]),
            section(
                &[90, 91, 92, 93],
                &[
                    (90, 91, 180.0),
                    (91, 92, boundary_document_only),
                    (92, 93, -180.0),
                ],
            ),
        ],
        COMPLETE_FOLD_ENDPOINT_EPS_DEG,
    );

    assert_eq!(
        actual,
        Ok(expected(
            None,
            vec![
                section_analysis(
                    vec![pair((80, 81), &[80], &[81], FullFoldSign::Positive180,)],
                    vec![],
                    None,
                    None,
                ),
                section_analysis(
                    vec![
                        pair((90, 91), &[90], &[91], FullFoldSign::Positive180,),
                        pair((92, 93), &[92], &[93], FullFoldSign::Negative180,),
                    ],
                    vec![FullFoldSign::Negative180],
                    None,
                    None,
                ),
            ],
            Some("折り線の場所によって、同時に折れるひだの枚数が異なります"),
        )),
        "区間を実際に1ひだ・2ひだと数え、1つの数へ丸めず理由を返す"
    );
}
