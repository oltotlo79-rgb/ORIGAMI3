//! 前川淳「悪魔」の手順1〜16で得る厳密な予備折りから、手順144の
//! `Pose` に再利用できる対称な複数ヒンジ駆動を検証する。
//!
//! 予備折りの補助線は交点で細分されるため、永続化された `DriverLine`
//! 1本は複数の辺IDへ解決される。このテストでは左右対称な2本の論理線を
//! 谷折りへ昇格し、各論理線上の全断片を同じ角度で駆動する。

use std::collections::{BTreeSet, HashMap};

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces, insert_segment, local_violations, validate};
use ori3_model::{
    CreasePattern, Document, Driver, DriverLine, EPS, EdgeId, EdgeKind, Frame3D, Paper, VertexId,
};
use ori3_rigid::{max_seam_gap, solve, solve_near_with_reflection_symmetry, three_point_support};

const POSE_ANGLE_DEG: f64 = -60.0;
const GEOM_TOL: f64 = 1e-8;

fn devil_logical_lines() -> [([f64; 2], [f64; 2]); 22] {
    let sqrt2 = std::f64::consts::SQRT_2;
    let t = sqrt2 - 1.0;
    let q = 2.0 - sqrt2;
    let s = 2.0 * t;
    let e = 2.0 * q - 1.0;
    let a = sqrt2 / 4.0;
    let b = (2.0 + sqrt2) / 4.0;
    let k = 4.0 * t - 1.0;
    [
        ([0.0, 0.0], [1.0, 1.0]),
        ([1.0, 0.0], [0.0, 1.0]),
        ([1.0, 1.0], [0.0, q]),
        ([1.0, 1.0], [q, 0.0]),
        ([0.0, 1.0], [1.0, q]),
        ([1.0, 0.0], [q, 1.0]),
        ([0.0, q], [1.0, q]),
        ([q, 0.0], [q, 1.0]),
        ([0.0, t], [q, 1.0]),
        ([t, 0.0], [1.0, q]),
        ([0.0, t], [t, 0.0]),
        ([q, 1.0], [1.0, q]),
        ([s, 0.0], [t, 1.0]),
        ([0.0, s], [1.0, t]),
        ([q, 0.0], [e, 1.0]),
        ([0.0, q], [1.0, e]),
        ([0.0, s], [s, 0.0]),
        ([e, 1.0], [1.0, e]),
        ([0.0, a], [b, 0.0]),
        ([a, 0.0], [0.0, b]),
        ([0.0, k], [k, 0.0]),
        ([0.0, 0.5], [0.5, 0.0]),
    ]
}

fn point_on_segment(p: DVec2, a: DVec2, b: DVec2) -> bool {
    let ab = b - a;
    let len_squared = ab.length_squared();
    if len_squared < EPS * EPS {
        return (p - a).length() <= EPS;
    }
    let t = ((p - a).dot(ab) / len_squared).clamp(0.0, 1.0);
    (p - (a + ab * t)).length() <= EPS
}

fn edge_fragments_on_line(
    cp: &CreasePattern,
    line: &DriverLine,
    include_auxiliary: bool,
) -> Vec<EdgeId> {
    let a = DVec2::from(line.a);
    let b = DVec2::from(line.b);
    if (b - a).length() < EPS {
        return Vec::new();
    }
    let positions: HashMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect();
    cp.edges
        .iter()
        .filter(|edge| {
            include_auxiliary || matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley)
        })
        .filter(|edge| {
            let (Some(&p0), Some(&p1)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
                return false;
            };
            (p1 - p0).length() >= EPS && point_on_segment(p0, a, b) && point_on_segment(p1, a, b)
        })
        .map(|edge| edge.id)
        .collect()
}

/// 全22線を `devil_precrease.rs` と同じ線種・厳密値で挿入し、対角線 `x=y`
/// に関して鏡像になる論理線2, 3だけを追加で折り目へ昇格する。
fn symmetric_pose_cp() -> (CreasePattern, [DriverLine; 2]) {
    let lines = devil_logical_lines();
    let mut document = Document::new(Paper {
        width_mm: 250.0,
        height_mm: 250.0,
    });
    for (index, (a, b)) in lines.into_iter().enumerate() {
        let kind = if index == 0 {
            EdgeKind::Valley
        } else {
            EdgeKind::Aux
        };
        insert_segment(&mut document.cp, a, b, kind);
    }

    let driver_lines = [2, 3].map(|index| DriverLine {
        a: lines[index].0,
        b: lines[index].1,
        target_angle_deg: POSE_ANGLE_DEG,
    });
    let promoted: BTreeSet<EdgeId> = driver_lines
        .iter()
        .flat_map(|line| edge_fragments_on_line(&document.cp, line, true))
        .collect();
    for edge in &mut document.cp.edges {
        if promoted.contains(&edge.id) && edge.kind == EdgeKind::Aux {
            edge.kind = EdgeKind::Valley;
        }
    }
    (document.cp, driver_lines)
}

fn all_valley_pose_cp() -> (CreasePattern, [DriverLine; 2]) {
    let lines = devil_logical_lines();
    let mut document = Document::new(Paper {
        width_mm: 250.0,
        height_mm: 250.0,
    });
    for (a, b) in lines {
        insert_segment(&mut document.cp, a, b, EdgeKind::Valley);
    }
    let driver_lines = [2, 3].map(|index| DriverLine {
        a: lines[index].0,
        b: lines[index].1,
        target_angle_deg: POSE_ANGLE_DEG,
    });
    (document.cp, driver_lines)
}

fn drivers_for_lines(cp: &CreasePattern, lines: &[DriverLine]) -> Vec<Driver> {
    let mut by_edge = HashMap::<EdgeId, f64>::new();
    for line in lines {
        let fragments = edge_fragments_on_line(cp, line, false);
        assert!(
            fragments.len() > 1,
            "論理線 {:?}->{:?} が複数ヒンジへ解決されない: {fragments:?}",
            line.a,
            line.b
        );
        for hinge in fragments {
            if let Some(previous) = by_edge.insert(hinge, line.target_angle_deg) {
                assert_eq!(previous, line.target_angle_deg);
            }
        }
    }
    let mut drivers: Vec<Driver> = by_edge
        .into_iter()
        .map(|(hinge, target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect();
    drivers.sort_unstable_by_key(|driver| driver.hinge);
    drivers
}

fn record_pose_lines(cp: &CreasePattern, angles: &HashMap<EdgeId, f64>) -> Vec<DriverLine> {
    let positions: HashMap<VertexId, [f64; 2]> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect();
    let mut edges: Vec<_> = cp
        .edges
        .iter()
        .filter(|edge| angles.contains_key(&edge.id))
        .collect();
    edges.sort_unstable_by_key(|edge| edge.id);
    edges
        .into_iter()
        .map(|edge| DriverLine {
            a: positions[&edge.v0],
            b: positions[&edge.v1],
            target_angle_deg: angles[&edge.id],
        })
        .collect()
}

fn resolve_pose_lines(cp: &CreasePattern, lines: &[DriverLine]) -> Vec<Driver> {
    let mut by_edge = HashMap::<EdgeId, f64>::new();
    for line in lines {
        let fragments = edge_fragments_on_line(cp, line, false);
        assert!(
            !fragments.is_empty(),
            "Pose DriverLineが辺へ解決されない: {line:?}"
        );
        for hinge in fragments {
            if let Some(previous) = by_edge.insert(hinge, line.target_angle_deg) {
                assert!(
                    (previous - line.target_angle_deg).abs() < 1e-9,
                    "hinge {hinge}へ矛盾するPose角: {previous} / {}",
                    line.target_angle_deg
                );
            }
        }
    }
    let mut drivers: Vec<_> = by_edge
        .into_iter()
        .map(|(hinge, target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect();
    drivers.sort_unstable_by_key(|driver| driver.hinge);
    drivers
}

fn folded_positions(faces: &[Face], frame: &Frame3D) -> HashMap<VertexId, DVec3> {
    let mut positions = HashMap::<VertexId, DVec3>::new();
    for face3d in &frame.faces {
        let face = &faces[face3d.face as usize];
        assert_eq!(face.vertices.len(), face3d.polygon.len());
        for (&vertex, &point) in face.vertices.iter().zip(&face3d.polygon) {
            let point = DVec3::from(point);
            if let Some(previous) = positions.insert(vertex, point) {
                assert!(
                    (point - previous).length() < 1e-7,
                    "共有頂点{vertex}が裂けた: {previous} vs {point}"
                );
            }
        }
    }
    positions
}

fn z_span(frame: &Frame3D) -> f64 {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for z in frame
        .faces
        .iter()
        .flat_map(|face| face.polygon.iter().map(|point| point[2]))
    {
        min = min.min(z);
        max = max.max(z);
    }
    max - min
}

fn vertex_at(cp: &CreasePattern, target: DVec2) -> VertexId {
    cp.vertices
        .iter()
        .find(|vertex| (DVec2::from(vertex.pos) - target).length() < GEOM_TOL)
        .unwrap_or_else(|| panic!("頂点{target}がCPにない"))
        .id
}

/// CP座標の `(x,y) <-> (y,x)` が、3Dでは1枚の鏡映面になることを検証する。
/// ソルバーの根面は固定されるため、その鏡映面は座標軸とは一般に一致しない。
fn assert_diagonal_reflection_symmetry(
    cp: &CreasePattern,
    positions: &HashMap<VertexId, DVec3>,
) -> f64 {
    let worst = diagonal_reflection_error(cp, positions);
    assert!(worst < 1e-7, "左右鏡映対称誤差={worst:.3e}");
    worst
}

fn diagonal_reflection_error(cp: &CreasePattern, positions: &HashMap<VertexId, DVec3>) -> f64 {
    let pairs: Vec<(DVec3, DVec3)> = cp
        .vertices
        .iter()
        .filter_map(|vertex| {
            let p = *positions.get(&vertex.id)?;
            let mirror_id = vertex_at(cp, DVec2::new(vertex.pos[1], vertex.pos[0]));
            let q = *positions.get(&mirror_id)?;
            Some((p, q))
        })
        .collect();
    assert!(
        pairs.len() >= 8,
        "対称性検証点が少なすぎる: {}",
        pairs.len()
    );

    let &(p0, q0) = pairs
        .iter()
        .max_by(|(p1, q1), (p2, q2)| {
            (*p1 - *q1)
                .length_squared()
                .total_cmp(&(*p2 - *q2).length_squared())
        })
        .expect("対称点対");
    let normal = (p0 - q0).normalize();
    let plane_offset = normal.dot((p0 + q0) * 0.5);
    let mut worst = 0.0_f64;
    for (p, q) in pairs {
        let delta = p - q;
        if delta.length() > GEOM_TOL {
            worst = worst.max(delta.cross(normal).length());
        }
        worst = worst.max((normal.dot((p + q) * 0.5) - plane_offset).abs());
    }
    worst
}

fn boundary_vertex_ids(cp: &CreasePattern) -> BTreeSet<VertexId> {
    cp.edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Border)
        .flat_map(|edge| [edge.v0, edge.v1])
        .collect()
}

fn boundary_corner_count(cp: &CreasePattern) -> usize {
    let positions: HashMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect();
    boundary_vertex_ids(cp)
        .into_iter()
        .filter(|&vertex| {
            let mut directions: Vec<DVec2> = cp
                .edges
                .iter()
                .filter(|edge| edge.kind == EdgeKind::Border)
                .filter_map(|edge| {
                    let other = if edge.v0 == vertex {
                        edge.v1
                    } else if edge.v1 == vertex {
                        edge.v0
                    } else {
                        return None;
                    };
                    Some((positions[&other] - positions[&vertex]).normalize())
                })
                .collect();
            directions.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
            directions.len() == 2 && directions[0].perp_dot(directions[1]).abs() > 0.5
        })
        .count()
}

fn driven_boundary_tip_count(cp: &CreasePattern, driven: &[Driver]) -> usize {
    let boundary = boundary_vertex_ids(cp);
    let driven: BTreeSet<EdgeId> = driven.iter().map(|driver| driver.hinge).collect();
    cp.edges
        .iter()
        .filter(|edge| driven.contains(&edge.id))
        .flat_map(|edge| [edge.v0, edge.v1])
        .filter(|vertex| boundary.contains(vertex))
        .collect::<BTreeSet<_>>()
        .len()
}

#[test]
fn exact_devil_precrease_drives_a_watertight_symmetric_standing_pose() {
    let (cp, driver_lines) = symmetric_pose_cp();
    assert_eq!(cp.vertices.len(), 92);
    assert_eq!(cp.edges.len(), 201);
    assert!(validate(&cp).is_empty(), "warnings={:?}", validate(&cp));
    assert!(local_violations(&cp).is_empty());

    let faces = extract_faces(&cp);
    assert_eq!(
        faces.len(),
        4,
        "既存の対角谷線と対称な2本の折り目は紙を4面に分ける"
    );
    let fragment_counts: Vec<usize> = driver_lines
        .iter()
        .map(|line| edge_fragments_on_line(&cp, line, false).len())
        .collect();
    assert_eq!(fragment_counts, [10, 10]);
    let drivers = drivers_for_lines(&cp, &driver_lines);
    assert_eq!(drivers.len(), 20, "細分ヒンジ数={}", drivers.len());
    assert_eq!(driven_boundary_tip_count(&cp, &drivers), 3);
    assert_eq!(boundary_corner_count(&cp), 4);

    // 予備折りに既に存在する対角谷線はこのPoseでは開いた0°のままにする。
    // 細分された同一直線の自由ヒンジへソルバーの初期バイアスを入れないため、
    // 全ヒンジの平坦状態を明示的なwarm startとして渡す。
    let flat_start: HashMap<EdgeId, f64> = cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|edge| (edge.id, 0.0))
        .collect();
    let result = solve(&cp, &faces, &drivers, Some(&flat_start));
    assert!(
        result.converged,
        "iterations={} warnings={:?} angles={:?}",
        result.iterations, result.frame.warnings, result.angles
    );
    assert!(
        result.frame.warnings.is_empty(),
        "warnings={:?}",
        result.frame.warnings
    );
    assert_eq!(result.frame.faces.len(), faces.len(), "面が失われた");
    for driver in &drivers {
        let angle = result.angles[&driver.hinge];
        assert!(
            (angle - POSE_ANGLE_DEG).abs() < 1e-9,
            "hinge {}: angle={angle}",
            driver.hinge
        );
    }
    let existing_diagonal = DriverLine {
        a: devil_logical_lines()[0].0,
        b: devil_logical_lines()[0].1,
        target_angle_deg: 0.0,
    };
    for hinge in edge_fragments_on_line(&cp, &existing_diagonal, false) {
        assert!(
            result.angles[&hinge].abs() < 1e-9,
            "既存の対角谷線hinge {hinge}が意図せず動いた"
        );
    }

    let seam_gap = max_seam_gap(&cp, &faces, &result.frame);
    assert!(seam_gap < 1e-6, "max_seam_gap={seam_gap:.3e}");
    let span = z_span(&result.frame);
    assert!(span > 0.1, "平面のまま: z_span={span:.6}");
    let positions = folded_positions(&faces, &result.frame);
    let symmetry_error = assert_diagonal_reflection_symmetry(&cp, &positions);
    let support_vertices = [1, 2, 3];
    let support = three_point_support(&faces, &result.frame, support_vertices)
        .expect("境界上の3点を支持点として評価できない");
    assert!(support.one_sided, "metrics={support:?}");
    assert!(support.centroid_projection_inside, "metrics={support:?}");
    assert!(support.stable, "metrics={support:?}");
    assert!(support.centroid_height > 0.1, "metrics={support:?}");

    println!(
        "devil pose: faces={}, hinges={}, boundary_tips=3, corners=4, support={:?}, support_area={:.12}, centroid_height={:.12}, z_span={span:.12}, symmetry_error={symmetry_error:.3e}, max_seam_gap={seam_gap:.3e}, iterations={}",
        faces.len(),
        drivers.len(),
        support.support_vertices,
        support.support_area,
        support.centroid_height,
        result.iterations
    );
}

#[test]
fn full_devil_precrease_pose_round_trips_through_driver_lines() {
    let (cp, seed_lines) = all_valley_pose_cp();
    assert_eq!(cp.vertices.len(), 92);
    assert_eq!(cp.edges.len(), 201);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 110);

    let seed_drivers = drivers_for_lines(&cp, &seed_lines);
    assert_eq!(seed_drivers.len(), 20);
    let cold = solve(&cp, &faces, &seed_drivers, None);
    let uniform_warm: HashMap<EdgeId, f64> = cold
        .angles
        .keys()
        .copied()
        .map(|hinge| (hinge, -30.0))
        .collect();
    let symmetric = solve_near_with_reflection_symmetry(
        &cp,
        &faces,
        &seed_drivers,
        &uniform_warm,
        Some(&uniform_warm),
        [[0.0, 0.0], [1.0, 1.0]],
    )
    .expect("全Valley CPの左右対称拘束solve");
    for (name, candidate) in [("cold", &cold), ("symmetric", &symmetric.result)] {
        let candidate_gap = max_seam_gap(&cp, &faces, &candidate.frame);
        let candidate_symmetry = if candidate.converged && candidate_gap < 1e-6 {
            diagonal_reflection_error(&cp, &folded_positions(&faces, &candidate.frame))
        } else {
            f64::INFINITY
        };
        println!(
            "full CP candidate {name}: converged={} iterations={} gap={candidate_gap:.3e} symmetry={candidate_symmetry:.3e}",
            candidate.converged, candidate.iterations
        );
    }
    assert!(symmetric.max_mirrored_angle_error_deg < 1e-9);
    let solved = &symmetric.result;
    assert!(solved.converged, "warnings={:?}", solved.frame.warnings);
    assert!(solved.frame.warnings.is_empty());
    assert_eq!(solved.angles.len(), 177);
    let gap = max_seam_gap(&cp, &faces, &solved.frame);
    assert!(gap < 1e-6, "max_seam_gap={gap:.3e}");
    let span = z_span(&solved.frame);
    assert!(span > 0.1, "z_span={span}");
    let positions = folded_positions(&faces, &solved.frame);
    let symmetry_error = assert_diagonal_reflection_symmetry(&cp, &positions);
    assert!(symmetry_error < 1e-9, "symmetry_error={symmetry_error:.3e}");

    let support = three_point_support(&faces, &solved.frame, [1, 2, 3])
        .expect("full CPの支持点を評価できない");
    println!("full CP initial support metrics={support:?}");
    assert!(support.stable, "metrics={support:?}");
    assert!(support.centroid_height > 0.1, "metrics={support:?}");

    let pose_lines = record_pose_lines(&cp, &solved.angles);
    assert_eq!(pose_lines.len(), 177);
    let replay_drivers = resolve_pose_lines(&cp, &pose_lines);
    assert_eq!(replay_drivers.len(), 177);
    let replayed = solve(&cp, &faces, &replay_drivers, None);
    assert!(replayed.converged, "warnings={:?}", replayed.frame.warnings);
    assert!(replayed.frame.warnings.is_empty());
    assert_eq!(replayed.iterations, 0);
    for driver in &replay_drivers {
        assert!(
            (replayed.angles[&driver.hinge] - solved.angles[&driver.hinge]).abs() < 1e-9,
            "hinge {} replay角が不一致",
            driver.hinge
        );
    }
    let replay_gap = max_seam_gap(&cp, &faces, &replayed.frame);
    assert!(replay_gap < 1e-6, "replay max_seam_gap={replay_gap:.3e}");
    for (before, after) in solved.frame.faces.iter().zip(&replayed.frame.faces) {
        assert_eq!(before.face, after.face);
        for (&p, &q) in before.polygon.iter().zip(&after.polygon) {
            assert!(
                (DVec3::from(p) - DVec3::from(q)).length() < 1e-9,
                "face {}のreplay座標が不一致: {p:?} / {q:?}",
                before.face
            );
        }
    }

    println!(
        "full devil pose: faces={}, hinges={}, symmetry_projections={}, mirrored_angle_error={:.3e}, replay_iterations={}, support_area={:.12}, centroid_height={:.12}, z_span={span:.12}, symmetry_error={symmetry_error:.3e}, gap={gap:.3e}, replay_gap={replay_gap:.3e}",
        faces.len(),
        solved.angles.len(),
        symmetric.projection_iterations,
        symmetric.max_mirrored_angle_error_deg,
        replayed.iterations,
        support.support_area,
        support.centroid_height,
    );
}
