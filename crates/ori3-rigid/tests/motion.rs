//! 角度操作の継続法と、停止しない接触診断の検査。

use std::collections::HashMap;

use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeKind, Vertex};
use ori3_rigid::{contact_metrics, max_seam_gap, self_intersects, solve, solve_motion};

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
fn three_strips_tree_hinge_yields_to_avoid_contact() {
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
    assert!((motion.result.angles[&9] - 110.0).abs() < 1e-9);
    assert!(
        (motion.result.angles[&8] - 150.0).abs() >= 0.1,
        "閉路外のmedium #8が接触を避けるために譲る: {:?}",
        motion.result.angles
    );
    assert!(
        max_seam_gap(&cp, &faces, &motion.result.frame) < 1e-6,
        "接触回避後も紙の接続を保つ"
    );
    assert!(
        !self_intersects(&motion.result.frame),
        "止めずに最終要求へ進み、他の折り目を譲らせて交差を避ける"
    );
}

#[test]
fn unavoidable_contact_returns_minimum_finite_best_effort() {
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
    assert!(first.result.best_effort);
    assert_finite_motion_result(&first.result, faces.len());
    assert!((first.result.angles[&8] - 150.0).abs() < 1e-9);
    assert!((first.result.angles[&9] - 110.0).abs() < 1e-9);
    assert!(max_seam_gap(&cp, &faces, &first.result.frame) < 1e-6);
    assert!(self_intersects(&first.result.frame));
    assert!(
        first
            .result
            .frame
            .warnings
            .iter()
            .any(|warning| warning.contains("貫通が最も少ない有限形")),
        "避けられない場合は最小侵入の有限形であることを警告する: {:?}",
        first.result.frame.warnings
    );

    // 両方をhardにしているため可変角はなく、完全固定の形が侵入量の唯一の候補。
    let forced = solve(&cp, &faces, &hard, Some(&start.angles));
    assert_eq!(
        contact_metrics(&first.result.frame),
        contact_metrics(&forced.frame),
        "hardを守る有限形の中で最小の侵入量を返す"
    );

    let second = solve_motion(&cp, &faces, &hard, None, Some(&start.angles), true);
    assert_motion_bits_eq(&first, &second);
}

#[test]
fn coalesced_angle_jump_still_yields_without_contact() {
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
    assert!((motion.result.angles[&9] - 120.0).abs() < 1e-9);
    assert!((motion.result.angles[&8] - 150.0).abs() >= 0.1);
    assert!(max_seam_gap(&cp, &faces, &motion.result.frame) < 1e-6);
    assert!(!self_intersects(&motion.result.frame));
}

#[test]
fn three_strips_bidirectional_sweep_stays_clear() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let targets = HashMap::from([(8, 150.0)]);
    let mut warm = solve(&cp, &faces, &[d(8, 150.0), d(9, 0.0)], None).angles;
    let reported = [0, 104, 106, 110, 120, 150, 179, 180];

    for (label, angles) in [
        ("up", (0..=180).collect::<Vec<_>>()),
        ("down", (0..=180).rev().collect::<Vec<_>>()),
    ] {
        for angle in angles {
            let started = std::time::Instant::now();
            let motion = solve_motion(
                &cp,
                &faces,
                &[d(9, f64::from(angle))],
                Some(&targets),
                Some(&warm),
                true,
            );
            let solve_time = started.elapsed();
            let contact_started = std::time::Instant::now();
            let pairs = ori3_rigid::self_intersection_pairs(&motion.result.frame);
            let contact_time = contact_started.elapsed();
            assert!(
                solve_time < std::time::Duration::from_millis(330),
                "{label} {angle}° solve={solve_time:?}"
            );
            assert!(
                contact_time < std::time::Duration::from_millis(500),
                "{label} {angle}° contact={contact_time:?}"
            );
            assert!(!motion.contact_stopped);
            assert!(motion.result.converged, "{label} {angle}°");
            assert!((motion.result.angles[&9] - f64::from(angle)).abs() < 1e-9);
            assert!(
                max_seam_gap(&cp, &faces, &motion.result.frame) < 1e-6,
                "{label} {angle}°"
            );
            assert!(pairs.is_empty(), "{label} {angle}° pairs={pairs:?}");
            if reported.contains(&angle) {
                println!(
                    "three-strips {label} {angle}°: pairs={} yielded={:.6}° solve={solve_time:?} contact={contact_time:?}",
                    pairs.len(),
                    (motion.result.angles[&8] - 150.0).abs()
                );
            }
            warm = motion.result.angles;
        }
    }
}

#[test]
fn three_strips_bidirectional_sixteen_degree_jumps_stay_clear() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let targets = HashMap::from([(8, 150.0)]);
    let mut warm = solve(&cp, &faces, &[d(8, 150.0), d(9, 0.0)], None).angles;
    let mut upward: Vec<u32> = (0..=180).step_by(16).collect();
    if upward.last() != Some(&180) {
        upward.push(180);
    }
    let downward: Vec<u32> = upward.iter().copied().rev().collect();

    for (direction, angles) in [("up", upward), ("down", downward)] {
        for angle in angles {
            let started = std::time::Instant::now();
            let motion = solve_motion(
                &cp,
                &faces,
                &[d(9, f64::from(angle))],
                Some(&targets),
                Some(&warm),
                true,
            );
            let solve_time = started.elapsed();
            let contact_started = std::time::Instant::now();
            let pairs = ori3_rigid::self_intersection_pairs(&motion.result.frame);
            let contact_time = contact_started.elapsed();

            assert!(
                solve_time < std::time::Duration::from_millis(330),
                "{direction} {angle}° solve={solve_time:?}"
            );
            assert!(
                contact_time < std::time::Duration::from_millis(500),
                "{direction} {angle}° contact={contact_time:?}"
            );
            assert!(!motion.contact_stopped, "{direction} {angle}°");
            assert!(motion.result.converged, "{direction} {angle}°");
            assert!((motion.result.angles[&9] - f64::from(angle)).abs() < 1e-9);
            assert!(
                max_seam_gap(&cp, &faces, &motion.result.frame) < 1e-6,
                "{direction} {angle}°"
            );
            assert!(pairs.is_empty(), "{direction} {angle}° pairs={pairs:?}");
            warm = motion.result.angles;
        }
    }
}

#[test]
fn disabled_contact_prevention_keeps_the_intersecting_legacy_result() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let start = solve(&cp, &faces, &[d(8, 150.0), d(9, 104.0)], None);
    let motion = solve_motion(
        &cp,
        &faces,
        &[d(9, 110.0)],
        None,
        Some(&start.angles),
        false,
    );
    assert!(!motion.contact_detected);
    assert!(motion.result.converged);
    assert_eq!(motion.result.angles[&9], 110.0);
    assert!(self_intersects(&motion.result.frame));
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
