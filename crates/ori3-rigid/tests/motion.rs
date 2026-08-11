//! 角度操作の継続法と、停止しない接触診断の検査。

use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeKind, Vertex};
use ori3_rigid::{self_intersects, solve, solve_motion};

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

#[test]
fn motion_contact_warns_without_stopping() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let start = solve(&cp, &faces, &[d(8, 150.0), d(9, 104.0)], None);
    assert!(start.converged);
    assert!(!self_intersects(&start.frame));

    let motion = solve_motion(&cp, &faces, &[d(9, 110.0)], None, Some(&start.angles), true);
    assert!(motion.contact_detected);
    assert!(motion.result.converged);
    assert!(self_intersects(&motion.result.frame));
    assert_eq!(motion.result.angles[&8], 150.0);
    assert!((motion.result.angles[&9] - 110.0).abs() < 1e-9);
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
