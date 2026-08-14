use std::collections::HashMap;

use glam::DVec3;
use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeKind, FaceId, Vertex};
use ori3_rigid::{propagate, solve_motion, to_frame3d};

fn vertex(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

fn split_square() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 0.5, 0.0),
            vertex(2, 1.0, 0.0),
            vertex(3, 1.0, 1.0),
            vertex(4, 0.5, 1.0),
            vertex(5, 0.0, 1.0),
        ],
        edges: vec![
            edge(0, 0, 1, EdgeKind::Border),
            edge(1, 1, 2, EdgeKind::Border),
            edge(2, 2, 3, EdgeKind::Border),
            edge(3, 3, 4, EdgeKind::Border),
            edge(4, 4, 5, EdgeKind::Border),
            edge(5, 5, 0, EdgeKind::Border),
            edge(6, 1, 4, EdgeKind::Mountain),
        ],
        next_vertex_id: 6,
        next_edge_id: 7,
    }
}

/// 利用者の不具合を再現した、対角線と中線を持つ正方形。
/// `tree.rs` の再発防止fixtureと同じ頂点・辺で、操作対象は中央のedge 43。
fn diagonal_midline_square() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0, 0.0),
            vertex(2, 1.0, 1.0),
            vertex(3, 0.0, 1.0),
            vertex(4, 0.0, 0.5),
            vertex(5, 1.0, 0.5),
            vertex(6, 0.5, 1.0),
            vertex(7, 0.5, 0.0),
            vertex(8, 0.5, 0.5),
            vertex(9, 0.792_893_218_813_452_5, 0.5),
            vertex(10, 0.5, 0.207_106_781_186_547_52),
            vertex(11, 0.25, 0.5),
            vertex(12, 0.5, 0.792_893_218_813_452_5),
            vertex(13, 0.207_106_781_186_547_52, 0.5),
        ],
        edges: vec![
            edge(4, 3, 4, EdgeKind::Border),
            edge(5, 4, 0, EdgeKind::Border),
            edge(6, 1, 5, EdgeKind::Border),
            edge(7, 5, 2, EdgeKind::Border),
            edge(9, 2, 6, EdgeKind::Border),
            edge(10, 6, 3, EdgeKind::Border),
            edge(11, 0, 7, EdgeKind::Border),
            edge(12, 7, 1, EdgeKind::Border),
            edge(17, 0, 8, EdgeKind::Valley),
            edge(18, 8, 2, EdgeKind::Valley),
            edge(19, 8, 9, EdgeKind::Mountain),
            edge(20, 9, 5, EdgeKind::Mountain),
            edge(21, 2, 9, EdgeKind::Mountain),
            edge(22, 9, 1, EdgeKind::Mountain),
            edge(23, 8, 10, EdgeKind::Mountain),
            edge(24, 10, 7, EdgeKind::Mountain),
            edge(25, 0, 10, EdgeKind::Mountain),
            edge(26, 10, 1, EdgeKind::Mountain),
            edge(28, 11, 8, EdgeKind::Mountain),
            edge(31, 6, 12, EdgeKind::Mountain),
            edge(32, 12, 8, EdgeKind::Mountain),
            edge(33, 2, 12, EdgeKind::Mountain),
            edge(34, 12, 3, EdgeKind::Mountain),
            edge(37, 4, 13, EdgeKind::Mountain),
            edge(38, 13, 11, EdgeKind::Mountain),
            edge(39, 0, 13, EdgeKind::Mountain),
            edge(40, 13, 3, EdgeKind::Mountain),
            edge(42, 13, 12, EdgeKind::Valley),
            edge(43, 10, 9, EdgeKind::Valley),
        ],
        next_vertex_id: 14,
        next_edge_id: 44,
    }
}

fn diagonal_midline_drivers(angle: f64) -> Vec<Driver> {
    let mut drivers = [21, 22, 25, 26, 33, 34, 39, 40]
        .into_iter()
        .map(|hinge| Driver {
            hinge,
            target_angle_deg: 180.0,
        })
        .collect::<Vec<_>>();
    drivers.push(Driver {
        hinge: 43,
        target_angle_deg: angle,
    });
    drivers
}

fn frame(angle: f64) -> ori3_model::Frame3D {
    let cp = split_square();
    let faces = extract_faces(&cp);
    let folded = propagate(&cp, &faces, &HashMap::from([(6, angle)]));
    to_frame3d(&cp, &faces, &folded)
}

fn rank_order(frame: &ori3_model::Frame3D) -> Vec<FaceId> {
    let mut ranked = frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect::<Vec<_>>();
    ranked.sort_unstable();
    ranked.into_iter().map(|(_, face)| face).collect()
}

fn depth_order(frame: &ori3_model::Frame3D) -> Vec<FaceId> {
    let mut depths = frame
        .faces
        .iter()
        .map(|face| {
            let mean =
                face.polygon.iter().map(|point| point[2]).sum::<f64>() / face.polygon.len() as f64;
            (mean, face.face)
        })
        .collect::<Vec<_>>();
    depths.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.cmp(&right.1)));
    depths.into_iter().map(|(_, face)| face).collect()
}

#[test]
fn exact_180_surface_order_matches_the_explicit_179_999_depth_limit() {
    for sign in [1.0, -1.0] {
        let approached = frame(sign * 179.999);
        let exact = frame(sign * 180.0);
        assert_eq!(rank_order(&exact), depth_order(&approached));
    }
}

#[test]
fn surface_order_does_not_follow_the_mirrored_flip_at_90_degrees() {
    for sign in [1.0, -1.0] {
        let before = frame(sign * 89.0);
        let after = frame(sign * 91.0);
        assert_eq!(rank_order(&before), rank_order(&after));
        assert_ne!(
            before
                .faces
                .iter()
                .map(|face| face.mirrored)
                .collect::<Vec<_>>(),
            after
                .faces
                .iter()
                .map(|face| face.mirrored)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn surface_order_is_identical_for_ten_runs() {
    let expected = rank_order(&frame(-180.0));
    for _ in 0..10 {
        assert_eq!(rank_order(&frame(-180.0)), expected);
    }
}

#[derive(Clone, Copy, Debug)]
struct OwnerSample {
    angle_deg: f64,
    depth_delta: f64,
    depth_owner: Option<FaceId>,
    display_owner: FaceId,
}

/// 深度がこの値以下のときだけrankを使う。近平坦solverの閉包誤差より十分大きく、
/// 1°刻みで実際に離れた面の深度差より十分小さい固定値。
const DEPTH_TIE_EPS: f64 = 1e-8;
const FIRST_FACE: FaceId = 12;
const SECOND_FACE: FaceId = 13;

fn camera_direction() -> DVec3 {
    DVec3::new(0.35, -0.85, 0.95).normalize()
}

fn canonical_face_normal(frame: &ori3_model::Frame3D, face_id: FaceId) -> DVec3 {
    let polygon = &frame
        .faces
        .iter()
        .find(|face| face.face == face_id)
        .expect("表示frameに対象面がある")
        .polygon;
    let mut normal = polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| DVec3::from(*a).cross(DVec3::from(*b)))
        .sum::<DVec3>()
        .normalize();
    let absolute = normal.abs();
    let component = if absolute.x >= absolute.y && absolute.x >= absolute.z {
        normal.x
    } else if absolute.y >= absolute.z {
        normal.y
    } else {
        normal.z
    };
    if component < 0.0 {
        normal = -normal;
    }
    normal
}

fn face_centroid(cp: &CreasePattern, face: &ori3_cp::Face) -> DVec3 {
    let point = face
        .vertices
        .iter()
        .map(|&vertex| cp.vertices[vertex as usize].pos)
        .fold([0.0, 0.0], |sum, point| {
            [sum[0] + point[0], sum[1] + point[1]]
        });
    let count = face.vertices.len() as f64;
    DVec3::new(point[0] / count, point[1] / count, 0.0)
}

fn reflect_in_material_plane(point: DVec3, a: DVec3, b: DVec3) -> DVec3 {
    let axis = (b - a).normalize();
    let projection = a + axis * (point - a).dot(axis);
    projection * 2.0 - point
}

fn owner_samples(sign: f64) -> Vec<OwnerSample> {
    let cp = diagonal_midline_square();
    let faces = extract_faces(&cp);
    let first = faces
        .iter()
        .find(|face| face.id == FIRST_FACE)
        .expect("edge 43の第1隣接面がある");
    let second = faces
        .iter()
        .find(|face| face.id == SECOND_FACE)
        .expect("edge 43の第2隣接面がある");
    assert!(first.edges.contains(&43) && second.edges.contains(&43));

    // 面12の内部点と、edge 43について鏡映した面13の物質点。同じ式をsolver結果から
    // 逆算せずCP座標だけで決めるため、期待する交換角を計算結果へ結び付けない。
    let first_material = face_centroid(&cp, first);
    let hinge = cp.edges.iter().find(|edge| edge.id == 43).expect("edge 43");
    let [hinge_ax, hinge_ay] = cp.vertices[hinge.v0 as usize].pos;
    let [hinge_bx, hinge_by] = cp.vertices[hinge.v1 as usize].pos;
    let hinge_a = DVec3::new(hinge_ax, hinge_ay, 0.0);
    let hinge_b = DVec3::new(hinge_bx, hinge_by, 0.0);
    let second_material = reflect_in_material_plane(first_material, hinge_a, hinge_b);

    let mut warm = None;
    let magnitudes = (0..=179).map(f64::from).chain([179.999, 180.0]);
    let mut samples = Vec::with_capacity(182);
    for magnitude in magnitudes {
        let angle = sign * magnitude;
        let solved = solve_motion(
            &cp,
            &faces,
            &diagonal_midline_drivers(angle),
            None,
            warm.as_ref(),
            false,
        )
        .result;
        assert!(
            solved.converged,
            "edge 43 angle={angle}: closure_rms={:.3e}, warnings={:?}",
            solved.closure_rms, solved.frame.warnings
        );

        // 真の前後関係は固定した2つの物質点をFoldedFrameで変換し、固定cameraへの
        // dotで直接測る。世界zは斜め視点での実表示深度とは一致しないため使わない。
        // 表示ownerは別に、Frame3Dが返したrankを同深度時だけ使って求める。
        let folded = propagate(&cp, &faces, &solved.angles);
        let (first_rotation, first_translation) = folded.transforms[&FIRST_FACE];
        let (second_rotation, second_translation) = folded.transforms[&SECOND_FACE];
        let first_world = first_rotation * first_material + first_translation;
        let second_world = second_rotation * second_material + second_translation;
        let camera = camera_direction();
        let depth_delta = (first_world - second_world).dot(camera);
        let depth_owner = if depth_delta > DEPTH_TIE_EPS {
            Some(FIRST_FACE)
        } else if depth_delta < -DEPTH_TIE_EPS {
            Some(SECOND_FACE)
        } else {
            None
        };
        let display_owner = depth_owner.unwrap_or_else(|| {
            let rank = |face_id| {
                solved
                    .frame
                    .faces
                    .iter()
                    .find(|face| face.face == face_id)
                    .expect("表示frameに対象面がある")
                    .surface_rank
            };
            // `surfaceOwner` と同じくcanonical面法線のcamera側を正として、
            // `side * surface_rank` が大きい面をLEQUALの後描きownerにする。
            let normal = canonical_face_normal(&solved.frame, FIRST_FACE);
            let side = if normal.dot(camera) >= 0.0 { 1i64 } else { -1 };
            let first_key = side * i64::from(rank(FIRST_FACE));
            let second_key = side * i64::from(rank(SECOND_FACE));
            if first_key > second_key {
                FIRST_FACE
            } else {
                SECOND_FACE
            }
        });
        samples.push(OwnerSample {
            angle_deg: angle,
            depth_delta,
            depth_owner,
            display_owner,
        });
        warm = Some(solved.angles);
    }
    samples
}

/// 深度が同じ整数角の区間をまたいだ場合も、1つの物理的交換区間として返す。
fn depth_exchange_windows(samples: &[OwnerSample]) -> Vec<(f64, f64)> {
    let mut previous = None;
    let mut exchanges = Vec::new();
    for sample in samples {
        let Some(owner) = sample.depth_owner else {
            continue;
        };
        if let Some((angle, previous_owner)) = previous
            && owner != previous_owner
        {
            exchanges.push((angle, sample.angle_deg));
        }
        previous = Some((sample.angle_deg, owner));
    }
    exchanges
}

fn display_exchange_windows(samples: &[OwnerSample]) -> Vec<(f64, f64)> {
    samples
        .windows(2)
        .filter(|pair| pair[0].display_owner != pair[1].display_owner)
        .map(|pair| (pair[0].angle_deg, pair[1].angle_deg))
        .collect()
}

fn exchange_diagnostic(
    sign: f64,
    samples: &[OwnerSample],
    depth: &[(f64, f64)],
    display: &[(f64, f64)],
) -> String {
    let interesting = samples
        .iter()
        .filter(|sample| {
            sample.depth_owner.is_none()
                || [89.0, 91.0, 179.999, 180.0]
                    .iter()
                    .any(|target| (sample.angle_deg.abs() - target).abs() <= 1e-9)
                || depth.iter().chain(display).any(|&(from, to)| {
                    sample.angle_deg >= from - 1.0 && sample.angle_deg <= to + 1.0
                })
        })
        .map(|sample| {
            format!(
                "{:.3}°:dCamera={:.3e},depth={:?},display={}",
                sample.angle_deg, sample.depth_delta, sample.depth_owner, sample.display_owner
            )
        })
        .collect::<Vec<_>>();
    format!(
        "edge43 sign={sign:+}: depth exchanges={depth:?}; display exchanges={display:?}; samples=[{}]",
        interesting.join(", ")
    )
}

#[test]
fn one_degree_sweep_only_changes_owner_at_real_depth_exchanges() {
    let mut failures = Vec::new();
    for sign in [1.0, -1.0] {
        let samples = owner_samples(sign);
        let depth = depth_exchange_windows(&samples);
        let display = display_exchange_windows(&samples);
        let diagnostic = exchange_diagnostic(sign, &samples, &depth, &display);
        eprintln!("{diagnostic}");

        let checkpoint = |magnitude: f64| {
            samples
                .iter()
                .find(|sample| (sample.angle_deg - sign * magnitude).abs() <= 1e-9)
                .expect("明示した掃引角のsampleがある")
        };
        for (left, right, label) in [(89.0, 91.0, "±89°/±91°"), (179.999, 180.0, "179.999°/180°")]
        {
            let left_sample = checkpoint(left);
            let right_sample = checkpoint(right);
            if left_sample.display_owner != right_sample.display_owner {
                failures.push(format!(
                    "{label}でownerが{}→{}へ飛んだ: {diagnostic}",
                    left_sample.display_owner, right_sample.display_owner
                ));
            }
        }

        // 0°と±180°の同深度端点は、それぞれ隣接する非同深度角のownerを選ぶ。
        // ここが違うと、物理的な前後交換が無いのに1°または180°で表示だけが飛ぶ。
        let first_non_tie = samples
            .iter()
            .find_map(|sample| sample.depth_owner)
            .expect("掃引中に実深度差が生じる");
        for sample in samples
            .iter()
            .take_while(|sample| sample.depth_owner.is_none())
        {
            if sample.display_owner != first_non_tie {
                failures.push(format!(
                    "開始端{}°は隣接深度owner {}でなく{}: {diagnostic}",
                    sample.angle_deg, first_non_tie, sample.display_owner
                ));
            }
        }
        let last_non_tie = samples
            .iter()
            .rev()
            .find_map(|sample| sample.depth_owner)
            .expect("掃引中に実深度差が生じる");
        for sample in samples
            .iter()
            .rev()
            .take_while(|sample| sample.depth_owner.is_none())
        {
            if sample.display_owner != last_non_tie {
                failures.push(format!(
                    "完了端{}°は隣接深度owner {}でなく{}: {diagnostic}",
                    sample.angle_deg, last_non_tie, sample.display_owner
                ));
            }
        }

        if display.len() != depth.len() {
            failures.push(format!(
                "実深度交換{}件に対して表示owner交換{}件: {diagnostic}",
                depth.len(),
                display.len()
            ));
        }
        for ((display_from, display_to), (depth_from, depth_to)) in display.iter().zip(&depth) {
            let depth_min = (*depth_from).min(*depth_to);
            let depth_max = (*depth_from).max(*depth_to);
            if !(display_from >= &depth_min
                && display_from <= &depth_max
                && display_to >= &depth_min
                && display_to <= &depth_max)
            {
                failures.push(format!(
                    "表示交換({display_from},{display_to})が実深度交換({depth_from},{depth_to})外: {diagnostic}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn mixed_exact_angles() -> HashMap<u32, f64> {
    HashMap::from([
        (17, -180.0),
        (18, -180.0),
        (20, -95.0),
        (21, 180.0),
        (22, 180.0),
        (23, 180.0),
        (24, -95.0),
        (25, 180.0),
        (31, -123.0),
        (32, 180.0),
        (34, 180.0),
        (37, -123.0),
        (39, 180.0),
        (40, 180.0),
        (42, -57.0),
        (43, -85.0),
    ])
}

fn explicit_approach_angles(
    angles: &HashMap<u32, f64>,
    move_only_exact: bool,
) -> HashMap<u32, f64> {
    angles
        .iter()
        .map(|(&edge, &angle)| {
            let exact = (angle.abs() - 180.0).abs() <= 1e-9;
            let should_move = exact || (!move_only_exact && angle != 0.0);
            let approached = if should_move {
                angle - angle.signum() * 0.001
            } else {
                angle
            };
            (edge, approached)
        })
        .collect()
}

#[test]
fn mixed_exact_stack_uses_only_exact_angles_for_its_approach() {
    let cp = diagonal_midline_square();
    let faces = extract_faces(&cp);
    let exact_angles = mixed_exact_angles();
    let exact_only_angles = explicit_approach_angles(&exact_angles, true);
    let legacy_angles = explicit_approach_angles(&exact_angles, false);
    assert!(
        exact_angles
            .values()
            .filter(|angle| (angle.abs() - 180.0).abs() <= 1e-9)
            .count()
            >= 2
    );
    assert!(
        exact_angles
            .values()
            .any(|angle| { angle.abs() > 1e-9 && (angle.abs() - 180.0).abs() > 1e-9 })
    );
    for (&hinge, &angle) in &exact_angles {
        if (angle.abs() - 180.0).abs() > 1e-9 {
            assert_eq!(exact_only_angles[&hinge], angle, "non-exact hinge {hinge}");
        }
    }
    let exact = propagate(&cp, &faces, &exact_angles);
    let exact_frame = to_frame3d(&cp, &faces, &exact);
    let expected = propagate(&cp, &faces, &exact_only_angles);
    let legacy = propagate(&cp, &faces, &legacy_angles);

    let rank = |face_id| {
        exact_frame
            .faces
            .iter()
            .find(|face| face.face == face_id)
            .unwrap()
            .surface_rank
    };
    let world_centroid = |face_id| {
        let polygon = &exact_frame
            .faces
            .iter()
            .find(|face| face.face == face_id)
            .unwrap()
            .polygon;
        polygon.iter().copied().map(DVec3::from).sum::<DVec3>() / polygon.len() as f64
    };
    let approach_height =
        |folded: &ori3_rigid::FoldedFrame, face_id: FaceId, point: DVec3, normal: DVec3| {
            let (exact_rotation, exact_translation) = exact.transforms[&face_id];
            let material = exact_rotation.transpose() * (point - exact_translation);
            let (rotation, translation) = folded.transforms[&face_id];
            (rotation * material + translation).dot(normal)
        };
    // Face 4 is an area-overlapping sub-triangle of face 10 in this explicit
    // exact pose. Its centroid is therefore a fixed material witness shared by
    // both faces, independent of any solver result.
    let witness = world_centroid(4);
    let normal = canonical_face_normal(&exact_frame, 4);
    let expected_delta = approach_height(&expected, 4, witness, normal)
        - approach_height(&expected, 10, witness, normal);
    let legacy_delta = approach_height(&legacy, 4, witness, normal)
        - approach_height(&legacy, 10, witness, normal);

    assert!(
        expected_delta > 1e-9,
        "the exact-only approach must put face 4 above face 10: {expected_delta:.3e}"
    );
    assert!(
        legacy_delta < -1e-9,
        "moving non-exact angles would not exercise the old reversed order: {legacy_delta:.3e}"
    );
    assert!(
        rank(4) > rank(10),
        "exact rank {}/{} must follow the explicit exact-only approach",
        rank(4),
        rank(10)
    );
}

#[test]
fn mixed_exact_stack_rank_is_independent_of_warm_start() {
    let cp = diagonal_midline_square();
    let faces = extract_faces(&cp);
    let explicit = mixed_exact_angles();
    let mut drivers = cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|edge| Driver {
            hinge: edge.id,
            target_angle_deg: explicit.get(&edge.id).copied().unwrap_or(0.0),
        })
        .collect::<Vec<_>>();
    drivers.sort_unstable_by_key(|driver| driver.hinge);

    // Keep every non-exact final angle in the seed, but make every exact angle
    // different. The hard drivers still define exactly the same final state.
    // A history-dependent implementation would probe a different moving set.
    let selective_warm = drivers
        .iter()
        .map(|driver| {
            let angle = driver.target_angle_deg;
            let warm_angle = if (angle.abs() - 180.0).abs() <= 1e-9 {
                0.0
            } else {
                angle
            };
            (driver.hinge, warm_angle)
        })
        .collect::<HashMap<_, _>>();

    let cold = solve_motion(&cp, &faces, &drivers, None, None, false).result;
    let warm = solve_motion(&cp, &faces, &drivers, None, Some(&selective_warm), false).result;

    assert_eq!(cold.angles.len(), warm.angles.len());
    for driver in &drivers {
        let cold_angle = cold.angles[&driver.hinge];
        let warm_angle = warm.angles[&driver.hinge];
        assert!(
            (cold_angle - driver.target_angle_deg).abs() <= 1e-12,
            "cold hinge {}: {cold_angle} != {}",
            driver.hinge,
            driver.target_angle_deg
        );
        assert!(
            (warm_angle - driver.target_angle_deg).abs() <= 1e-12,
            "warm hinge {}: {warm_angle} != {}",
            driver.hinge,
            driver.target_angle_deg
        );
    }
    for cold_face in &cold.frame.faces {
        let warm_face = warm
            .frame
            .faces
            .iter()
            .find(|face| face.face == cold_face.face)
            .expect("warm frame contains every cold face");
        assert_eq!(cold_face.polygon.len(), warm_face.polygon.len());
        for (cold_point, warm_point) in cold_face.polygon.iter().zip(&warm_face.polygon) {
            for axis in 0..3 {
                assert!(
                    (cold_point[axis] - warm_point[axis]).abs() <= 1e-12,
                    "face {} axis {axis}: {} != {}",
                    cold_face.face,
                    cold_point[axis],
                    warm_point[axis]
                );
            }
        }
    }

    assert_eq!(rank_order(&cold.frame), rank_order(&warm.frame));
    let expected = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &explicit));
    assert_eq!(rank_order(&cold.frame), rank_order(&expected));
}
