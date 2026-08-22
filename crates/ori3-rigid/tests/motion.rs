//! 角度操作の継続法と、停止しない接触診断の検査。
//!
//! 速さの上限はここでは判定しない。以前は往復スイープの中で
//! `solve_motion` と自己交差の走査の実時間を330ms・500msと比べていたが、
//! この検査は最適化なしのビルドでも走るため、計算機の混み具合が
//! そのまま合否に出てしまう。上限値は緩めずに
//! `crates/ori3-rigid/tests/perf_contact.rs` へ移し、最適化ありのビルドの
//! ときだけ判定するようにした(経緯は同ファイルの冒頭)。

use std::collections::HashMap;

use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeKind, Vertex};
use ori3_rigid::{
    MotionContactOptions, PENETRATION_WARNING, contact_metrics, contact_witnesses, max_seam_gap,
    self_intersection_pairs, self_intersects, solve, solve_motion,
    solve_motion_with_contact_options,
};

fn v(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn e(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

fn d(hinge: u32, deg: f64) -> Driver {
    Driver {
        hinge,
        target_angle_deg: deg,
    }
}

/// 正方形を縦3短冊へ分けた木構造。外側の面どうしは非隣接なので、巻き込むと
/// self_intersectsが面0と面2の貫通を検出する。
fn three_strips() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            v(0, 0.0, 0.0),
            v(1, 1.0 / 3.0, 0.0),
            v(2, 2.0 / 3.0, 0.0),
            v(3, 1.0, 0.0),
            v(4, 1.0, 1.0),
            v(5, 2.0 / 3.0, 1.0),
            v(6, 1.0 / 3.0, 1.0),
            v(7, 0.0, 1.0),
        ],
        edges: vec![
            e(0, 0, 1, EdgeKind::Border),
            e(1, 1, 2, EdgeKind::Border),
            e(2, 2, 3, EdgeKind::Border),
            e(3, 3, 4, EdgeKind::Border),
            e(4, 4, 5, EdgeKind::Border),
            e(5, 5, 6, EdgeKind::Border),
            e(6, 6, 7, EdgeKind::Border),
            e(7, 7, 0, EdgeKind::Border),
            e(8, 1, 6, EdgeKind::Mountain),
            e(9, 2, 5, EdgeKind::Mountain),
        ],
        next_vertex_id: 8,
        next_edge_id: 10,
    }
}

fn degree4_cp() -> CreasePattern {
    let p1x = 0.5 + 0.5 * 50f64.to_radians().cos() / 50f64.to_radians().sin();
    let p2x = 0.5 + 0.5 * 110f64.to_radians().cos() / 110f64.to_radians().sin();
    let p3x = 0.5 + 0.5 / 240f64.to_radians().sin().abs() * 240f64.to_radians().cos();
    CreasePattern {
        vertices: vec![
            v(0, 0.0, 0.0),
            v(1, p3x, 0.0),
            v(2, 1.0, 0.0),
            v(3, 1.0, 0.5),
            v(4, 1.0, 1.0),
            v(5, p1x, 1.0),
            v(6, p2x, 1.0),
            v(7, 0.0, 1.0),
            v(8, 0.5, 0.5),
        ],
        edges: vec![
            e(0, 0, 1, EdgeKind::Border),
            e(1, 1, 2, EdgeKind::Border),
            e(2, 2, 3, EdgeKind::Border),
            e(3, 3, 4, EdgeKind::Border),
            e(4, 4, 5, EdgeKind::Border),
            e(5, 5, 6, EdgeKind::Border),
            e(6, 6, 7, EdgeKind::Border),
            e(7, 7, 0, EdgeKind::Border),
            e(8, 8, 3, EdgeKind::Mountain),
            e(9, 8, 5, EdgeKind::Valley),
            e(10, 8, 6, EdgeKind::Mountain),
            e(11, 8, 1, EdgeKind::Mountain),
        ],
        next_vertex_id: 9,
        next_edge_id: 12,
    }
}

fn max_frame_delta(a: &ori3_model::Frame3D, b: &ori3_model::Frame3D) -> f64 {
    a.faces
        .iter()
        .zip(&b.faces)
        .flat_map(|(left, right)| left.polygon.iter().zip(&right.polygon))
        .flat_map(|(left, right)| left.iter().zip(right))
        .map(|(left, right)| (left - right).abs())
        .fold(0.0, f64::max)
}

fn assert_finite_motion_result(result: &ori3_rigid::SolveResult, expected_faces: usize) {
    assert!(result.closure_rms.is_finite());
    assert_eq!(result.frame.faces.len(), expected_faces);
    assert!(result.angles.values().all(|angle| angle.is_finite()));
    assert!(result.frame.faces.iter().all(|face| {
        face.polygon
            .iter()
            .flatten()
            .all(|coordinate| coordinate.is_finite())
    }));
}

fn assert_penetration_warning(result: &ori3_rigid::SolveResult) {
    assert!(
        result
            .frame
            .warnings
            .iter()
            .any(|warning| warning == PENETRATION_WARNING),
        "warnings={:?}",
        result.frame.warnings
    );
}

fn assert_single_penetration(result: &ori3_rigid::SolveResult, expected_faces: (u32, u32)) -> f64 {
    let witnesses = contact_witnesses(&result.frame);
    assert_eq!(witnesses.len(), 1);
    assert_eq!(witnesses[0].faces, expected_faces);
    assert!(witnesses[0].penetration_depth.is_finite());
    assert!(witnesses[0].penetration_depth > 0.0);

    let metrics = contact_metrics(&result.frame);
    assert_eq!(metrics.pair_count, 1);
    assert_eq!(metrics.max_penetration, witnesses[0].penetration_depth);
    assert_eq!(metrics.total_penetration, witnesses[0].penetration_depth);
    witnesses[0].penetration_depth
}

fn assert_same_shape(left: &ori3_rigid::MotionSolveResult, right: &ori3_rigid::MotionSolveResult) {
    assert_eq!(left.result.angles, right.result.angles);
    assert_eq!(
        left.result.frame.faces.len(),
        right.result.frame.faces.len()
    );
    for (left_face, right_face) in left
        .result
        .frame
        .faces
        .iter()
        .zip(&right.result.frame.faces)
    {
        assert_eq!(left_face.face, right_face.face);
        assert_eq!(left_face.polygon, right_face.polygon);
    }
}

fn assert_motion_bits_eq(
    left: &ori3_rigid::MotionSolveResult,
    right: &ori3_rigid::MotionSolveResult,
) {
    let sorted_angles = |result: &ori3_rigid::SolveResult| {
        let mut values: Vec<_> = result
            .angles
            .iter()
            .map(|(&hinge, &angle)| (hinge, angle.to_bits()))
            .collect();
        values.sort_unstable_by_key(|item| item.0);
        values
    };
    assert_eq!(left.contact_detected, right.contact_detected);
    assert_eq!(left.contact_stopped, right.contact_stopped);
    assert_eq!(sorted_angles(&left.result), sorted_angles(&right.result));
    assert_eq!(left.result.converged, right.result.converged);
    assert_eq!(left.result.best_effort, right.result.best_effort);
    assert_eq!(
        left.result.closure_rms.to_bits(),
        right.result.closure_rms.to_bits()
    );
    assert_eq!(left.result.iterations, right.result.iterations);
    assert_eq!(left.result.relaxations, right.result.relaxations);
    assert_eq!(left.result.frame.warnings, right.result.frame.warnings);
    assert_eq!(
        left.result.frame.faces.len(),
        right.result.frame.faces.len()
    );
    for (left_face, right_face) in left
        .result
        .frame
        .faces
        .iter()
        .zip(&right.result.frame.faces)
    {
        assert_eq!(
            (left_face.face, left_face.layer),
            (right_face.face, right_face.layer)
        );
        assert_eq!(left_face.polygon.len(), right_face.polygon.len());
        for (left_point, right_point) in left_face.polygon.iter().zip(&right_face.polygon) {
            assert_eq!(left_point.map(f64::to_bits), right_point.map(f64::to_bits));
        }
    }
}

#[test]
fn three_strips_contact_warning_keeps_requested_angles() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let start = solve(&cp, &faces, &[d(8, 150.0), d(9, 104.0)], None);
    assert!(start.converged);
    assert!(!self_intersects(&start.frame));

    let targets = HashMap::from([(8, 150.0)]);
    let motion = solve_motion(
        &cp,
        &faces,
        &[d(9, 110.0)],
        Some(&targets),
        Some(&start.angles),
        true,
    );
    assert!(motion.contact_detected);
    assert!(!motion.contact_stopped);
    assert!(motion.result.converged);
    assert_finite_motion_result(&motion.result, faces.len());
    assert!((motion.result.angles[&9] - 110.0).abs() < 1e-9);
    assert!(
        (motion.result.angles[&8] - 150.0).abs() < 1e-9,
        "警告だけなら過去に指定した #8 も書き換えない: {:?}",
        motion.result.angles
    );
    assert!(
        max_seam_gap(&cp, &faces, &motion.result.frame) < 1e-6,
        "交差しても紙の接続を保つ"
    );
    assert_eq!(self_intersection_pairs(&motion.result.frame), vec![(0, 2)]);
    let depth = assert_single_penetration(&motion.result, (0, 2));
    println!("tree-hinge warning-only faces=0/2 depth={depth:.15e}");
    assert_penetration_warning(&motion.result);
}

#[test]
fn fully_specified_intersection_returns_warning_without_stopping() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let start = solve(&cp, &faces, &[d(8, 150.0), d(9, 104.0)], None);
    assert!(start.converged);
    assert!(!self_intersects(&start.frame));
    let hard = [d(8, 150.0), d(9, 110.0)];

    let first = solve_motion(&cp, &faces, &hard, None, Some(&start.angles), true);
    assert!(first.contact_detected);
    assert!(!first.contact_stopped);
    assert!(first.result.converged);
    assert_finite_motion_result(&first.result, faces.len());
    assert!((first.result.angles[&8] - 150.0).abs() < 1e-9);
    assert!((first.result.angles[&9] - 110.0).abs() < 1e-9);
    assert!(max_seam_gap(&cp, &faces, &first.result.frame) < 1e-6);
    assert_eq!(self_intersection_pairs(&first.result.frame), vec![(0, 2)]);
    let depth = assert_single_penetration(&first.result, (0, 2));
    println!("fully-specified warning-only faces=0/2 depth={depth:.15e}");
    assert_penetration_warning(&first.result);

    // 両方をhardにしているため可変角はなく、完全固定の形が侵入量の唯一の候補。
    let forced = solve(&cp, &faces, &hard, Some(&start.angles));
    assert_eq!(
        contact_metrics(&first.result.frame),
        contact_metrics(&forced.frame),
        "指定を守った物理形をそのまま返す"
    );

    let second = solve_motion(&cp, &faces, &hard, None, Some(&start.angles), true);
    assert_motion_bits_eq(&first, &second);
}

#[test]
fn coalesced_angle_jump_keeps_angles_and_warns() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let start = solve(&cp, &faces, &[d(8, 150.0), d(9, 104.0)], None);
    let targets = HashMap::from([(8, 150.0)]);

    // 入力の間引きで中間角が届かず、104°から120°へ一度に進む場合を再現する。
    let motion = solve_motion(
        &cp,
        &faces,
        &[d(9, 120.0)],
        Some(&targets),
        Some(&start.angles),
        true,
    );
    assert!(motion.contact_detected);
    assert!(!motion.contact_stopped);
    assert!(motion.result.converged);
    assert_finite_motion_result(&motion.result, faces.len());
    assert!((motion.result.angles[&9] - 120.0).abs() < 1e-9);
    assert!((motion.result.angles[&8] - 150.0).abs() < 1e-9);
    assert!(max_seam_gap(&cp, &faces, &motion.result.frame) < 1e-6);
    assert_eq!(self_intersection_pairs(&motion.result.frame), vec![(0, 2)]);
    let depth = assert_single_penetration(&motion.result, (0, 2));
    println!("coalesced-jump warning-only faces=0/2 depth={depth:.15e}");
    assert_penetration_warning(&motion.result);
}

#[test]
fn three_strips_bidirectional_sweep_reports_the_geometric_crossing() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let targets = HashMap::from([(8, 150.0)]);
    let mut warm = solve(&cp, &faces, &[d(8, 150.0), d(9, 0.0)], None).angles;
    let reported = [0, 104, 106, 110, 120, 150, 179, 180];

    let mut intersecting_results = 0;
    let mut representative_depth = None;
    for (label, angles) in [
        ("up", (0..=180).collect::<Vec<_>>()),
        ("down", (0..=180).rev().collect::<Vec<_>>()),
    ] {
        for angle in angles {
            let motion = solve_motion(
                &cp,
                &faces,
                &[d(9, f64::from(angle))],
                Some(&targets),
                Some(&warm),
                true,
            );
            let pairs = ori3_rigid::self_intersection_pairs(&motion.result.frame);
            assert!(!motion.contact_stopped);
            assert!(motion.result.converged, "{label} {angle}°");
            assert_finite_motion_result(&motion.result, faces.len());
            assert!((motion.result.angles[&9] - f64::from(angle)).abs() < 1e-9);
            assert!((motion.result.angles[&8] - 150.0).abs() < 1e-9);
            assert!(
                max_seam_gap(&cp, &faces, &motion.result.frame) < 1e-6,
                "{label} {angle}°"
            );
            let should_intersect = (106..180).contains(&angle);
            assert_eq!(
                pairs,
                if should_intersect {
                    vec![(0, 2)]
                } else {
                    vec![]
                }
            );
            if should_intersect {
                intersecting_results += 1;
                assert!(motion.contact_detected, "{label} {angle}°");
                assert_penetration_warning(&motion.result);
                if label == "up" && angle == 106 {
                    representative_depth = Some(assert_single_penetration(&motion.result, (0, 2)));
                }
            }
            if reported.contains(&angle) {
                println!(
                    "three-strips {label} {angle}°: pairs={} yielded={:.6}°",
                    pairs.len(),
                    (motion.result.angles[&8] - 150.0).abs()
                );
            }
            warm = motion.result.angles;
        }
    }
    assert_eq!(intersecting_results, 148);
    println!(
        "one-degree sweep up 106° faces=0/2 depth={:.15e}",
        representative_depth.expect("上昇106°の交差を数値化する")
    );
}

#[test]
fn three_strips_bidirectional_sixteen_degree_jumps_report_crossing() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let targets = HashMap::from([(8, 150.0)]);
    let mut warm = solve(&cp, &faces, &[d(8, 150.0), d(9, 0.0)], None).angles;
    let mut upward: Vec<u32> = (0..=180).step_by(16).collect();
    if upward.last() != Some(&180) {
        upward.push(180);
    }
    let downward: Vec<u32> = upward.iter().copied().rev().collect();

    let mut intersecting_results = 0;
    let mut representative_depth = None;
    for (direction, angles) in [("up", upward), ("down", downward)] {
        for angle in angles {
            let motion = solve_motion(
                &cp,
                &faces,
                &[d(9, f64::from(angle))],
                Some(&targets),
                Some(&warm),
                true,
            );
            let pairs = ori3_rigid::self_intersection_pairs(&motion.result.frame);

            assert!(!motion.contact_stopped, "{direction} {angle}°");
            assert!(motion.result.converged, "{direction} {angle}°");
            assert_finite_motion_result(&motion.result, faces.len());
            assert!((motion.result.angles[&9] - f64::from(angle)).abs() < 1e-9);
            assert!((motion.result.angles[&8] - 150.0).abs() < 1e-9);
            assert!(
                max_seam_gap(&cp, &faces, &motion.result.frame) < 1e-6,
                "{direction} {angle}°"
            );
            let should_intersect = (106..180).contains(&angle);
            assert_eq!(
                pairs,
                if should_intersect {
                    vec![(0, 2)]
                } else {
                    vec![]
                },
                "{direction} {angle}°"
            );
            if should_intersect {
                intersecting_results += 1;
                assert!(motion.contact_detected, "{direction} {angle}°");
                assert_penetration_warning(&motion.result);
                if direction == "up" && angle == 112 {
                    representative_depth = Some(assert_single_penetration(&motion.result, (0, 2)));
                }
            }
            warm = motion.result.angles;
        }
    }
    assert_eq!(intersecting_results, 10);
    println!(
        "sixteen-degree sweep up 112° faces=0/2 depth={:.15e}",
        representative_depth.expect("上昇112°の交差を数値化する")
    );
}

#[test]
fn enabling_contact_detection_does_not_change_the_shape() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let start = solve(&cp, &faces, &[d(8, 150.0), d(9, 104.0)], None);
    let detection_off = solve_motion(
        &cp,
        &faces,
        &[d(9, 110.0)],
        None,
        Some(&start.angles),
        false,
    );
    let detection_on = solve_motion(&cp, &faces, &[d(9, 110.0)], None, Some(&start.angles), true);
    assert!(!detection_off.contact_detected);
    assert!(detection_on.contact_detected);
    assert_same_shape(&detection_off, &detection_on);
    assert!(detection_on.result.converged);
    assert_eq!(detection_on.result.angles[&9], 110.0);
    assert!(self_intersects(&detection_on.result.frame));
    assert_penetration_warning(&detection_on.result);
}

#[test]
fn overlap_prevention_changes_the_shape_only_when_explicitly_enabled() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let start = solve(&cp, &faces, &[d(8, 150.0), d(9, 104.0)], None);
    let targets = HashMap::from([(8, 150.0)]);
    let warning_only = solve_motion(
        &cp,
        &faces,
        &[d(9, 110.0)],
        Some(&targets),
        Some(&start.angles),
        true,
    );
    let prevented = solve_motion_with_contact_options(
        &cp,
        &faces,
        &[d(9, 110.0)],
        Some(&targets),
        Some(&start.angles),
        MotionContactOptions {
            detect: true,
            prevent: true,
        },
    );
    let prevented_without_detection = solve_motion_with_contact_options(
        &cp,
        &faces,
        &[d(9, 110.0)],
        Some(&targets),
        Some(&start.angles),
        MotionContactOptions {
            detect: false,
            prevent: true,
        },
    );

    assert_eq!(
        self_intersection_pairs(&warning_only.result.frame),
        vec![(0, 2)]
    );
    assert!((warning_only.result.angles[&8] - 150.0).abs() < 1e-9);
    assert!(!self_intersects(&prevented.result.frame));
    assert!((prevented.result.angles[&8] - 139.453_125).abs() < 1e-9);
    assert!((prevented.result.angles[&9] - 110.0).abs() < 1e-9);
    assert_same_shape(&prevented, &prevented_without_detection);
    assert!(!prevented_without_detection.contact_detected);
    assert!(
        prevented_without_detection
            .result
            .frame
            .warnings
            .iter()
            .all(|warning| warning != PENETRATION_WARNING),
        "検出OFFなら、明示補正は形だけを変えて食い込み警告を足さない"
    );
}

#[test]
fn warning_only_keeps_five_intersecting_requested_shapes() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let start = solve(&cp, &faces, &[d(8, 150.0), d(9, 104.0)], None);
    let targets = HashMap::from([(8, 150.0)]);

    for angle in [106.0, 110.0, 112.0, 120.0, 150.0] {
        let motion = solve_motion(
            &cp,
            &faces,
            &[d(9, angle)],
            Some(&targets),
            Some(&start.angles),
            true,
        );
        assert!(!motion.contact_stopped, "{angle}°");
        assert!(motion.contact_detected, "{angle}°");
        assert_finite_motion_result(&motion.result, faces.len());
        assert!((motion.result.angles[&8] - 150.0).abs() < 1e-9, "{angle}°");
        assert!((motion.result.angles[&9] - angle).abs() < 1e-9, "{angle}°");
        assert_eq!(self_intersection_pairs(&motion.result.frame), vec![(0, 2)]);
        assert_penetration_warning(&motion.result);
    }
}

#[test]
fn coplanar_flat_fold_reaches_180_without_being_stopped() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let motion = solve_motion(&cp, &faces, &[d(8, 180.0), d(9, -180.0)], None, None, true);
    assert!(motion.result.converged, "angles={:?}", motion.result.angles);
    assert!(
        !motion.contact_detected,
        "angles={:?}",
        motion.result.angles
    );
    assert_eq!(motion.result.angles[&8], 180.0);
    assert_eq!(motion.result.angles[&9], -180.0);
    assert!(!self_intersects(&motion.result.frame));
    assert!(
        motion
            .result
            .frame
            .faces
            .iter()
            .all(|face| { face.polygon.iter().all(|point| point[2].abs() < 1e-6) })
    );
}

#[test]
fn nonconverged_best_effort_keeps_active_and_moves() {
    let cp = degree4_cp();
    let faces = extract_faces(&cp);
    let flat = solve(&cp, &faces, &[], None);

    let first = solve_motion(
        &cp,
        &faces,
        &[d(8, 180.0), d(9, -90.0)],
        None,
        Some(&flat.angles),
        true,
    )
    .result;
    assert!(!first.converged);
    assert!(first.best_effort);
    assert!(first.closure_rms.is_finite());
    assert_eq!(first.frame.faces.len(), faces.len());
    assert!(first.angles.values().all(|angle| angle.is_finite()));
    assert!(first.frame.faces.iter().all(|face| {
        face.polygon
            .iter()
            .flatten()
            .all(|coordinate| coordinate.is_finite())
    }));
    assert!((first.angles[&8] - 180.0).abs() < 1e-9);
    assert!((first.angles[&9] + 90.0).abs() < 1e-9);
    assert!(max_frame_delta(&flat.frame, &first.frame) > 1e-6);

    let second = solve_motion(
        &cp,
        &faces,
        &[d(8, 170.0), d(9, -80.0)],
        None,
        Some(&first.angles),
        true,
    )
    .result;
    assert!(!second.converged);
    assert!(second.best_effort);
    assert!(second.closure_rms.is_finite());
    assert!((second.angles[&8] - 170.0).abs() < 1e-9);
    assert!((second.angles[&9] + 80.0).abs() < 1e-9);
    assert!(max_frame_delta(&first.frame, &second.frame) > 1e-6);
}
