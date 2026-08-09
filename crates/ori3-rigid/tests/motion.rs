//! 角度操作の食い込み防止（接触直前停止）の検査。

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

#[test]
fn contact_prevention_returns_the_last_safe_pose_near_impact() {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let start = solve(&cp, &faces, &[d(8, 150.0), d(9, 104.0)], None);
    assert!(start.converged);
    assert!(!self_intersects(&start.frame));

    let motion = solve_motion(&cp, &faces, &[d(9, 110.0)], None, Some(&start.angles), true);
    assert!(motion.contact_stopped);
    assert!(motion.result.converged);
    assert!(!self_intersects(&motion.result.frame));
    assert_eq!(motion.result.angles[&8], 150.0);
    assert!(
        (104.0..105.0).contains(&motion.result.angles[&9]),
        "angles={:?}",
        motion.result.angles
    );
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
    assert!(!motion.contact_stopped);
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
    assert!(!motion.contact_stopped, "angles={:?}", motion.result.angles);
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
