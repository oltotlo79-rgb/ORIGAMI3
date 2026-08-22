use std::collections::HashMap;

use glam::DVec3;
use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeId, EdgeKind, FaceId, Vertex};
use ori3_rigid::{
    propagate, solve_motion, solve_motion_once, surface_order_from_angles,
    surface_order_from_angles_flat_path, to_frame3d,
};

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

/// 2本の独立した折り目で同じ三層へ畳める帯。どちらを現在のhard driverにしても
/// 最終角は同じなので、surface rankも同じでなければならない。
fn three_panel_strip() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0 / 3.0, 0.0),
            vertex(2, 2.0 / 3.0, 0.0),
            vertex(3, 1.0, 0.0),
            vertex(4, 1.0, 1.0),
            vertex(5, 2.0 / 3.0, 1.0),
            vertex(6, 1.0 / 3.0, 1.0),
            vertex(7, 0.0, 1.0),
        ],
        edges: vec![
            edge(0, 0, 1, EdgeKind::Border),
            edge(1, 1, 2, EdgeKind::Border),
            edge(2, 2, 3, EdgeKind::Border),
            edge(3, 3, 4, EdgeKind::Border),
            edge(4, 4, 5, EdgeKind::Border),
            edge(5, 5, 6, EdgeKind::Border),
            edge(6, 6, 7, EdgeKind::Border),
            edge(7, 7, 0, EdgeKind::Border),
            edge(8, 1, 6, EdgeKind::Mountain),
            edge(9, 2, 5, EdgeKind::Mountain),
        ],
        next_vertex_id: 8,
        next_edge_id: 10,
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

/// 利用者の画面で紙の裏が 66.88% 見えていた、実機の展開図と姿勢。
///
/// 動作中の `desktop.exe` から読み取った `doc.cp` をそのまま写した(頂点13・辺28)。
/// `.gitignore` 対象の `verification/` を検査から読まないため、値を検査コードへ
/// 埋め込んでいる(CLAUDE.md §10.1)。
fn live_frame_square() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0, 0.0),
            vertex(2, 1.0, 1.0),
            vertex(3, 0.0, 1.0),
            vertex(4, 1.0, 0.5),
            vertex(5, 0.0, 0.5),
            vertex(6, 0.5, 1.0),
            vertex(7, 0.5, 0.0),
            vertex(8, 0.5, 0.5),
            vertex(9, 0.5, 0.792_893_218_813_452_5),
            vertex(10, 0.792_893_218_813_452_5, 0.5),
            vertex(11, 0.5, 0.207_106_781_186_547_52),
            vertex(12, 0.207_106_781_186_547_52, 0.5),
        ],
        edges: vec![
            edge(4, 1, 4, EdgeKind::Border),
            edge(5, 4, 2, EdgeKind::Border),
            edge(6, 3, 5, EdgeKind::Border),
            edge(7, 5, 0, EdgeKind::Border),
            edge(9, 2, 6, EdgeKind::Border),
            edge(10, 6, 3, EdgeKind::Border),
            edge(11, 0, 7, EdgeKind::Border),
            edge(12, 7, 1, EdgeKind::Border),
            edge(17, 0, 8, EdgeKind::Valley),
            edge(18, 8, 2, EdgeKind::Valley),
            edge(19, 6, 9, EdgeKind::Mountain),
            edge(20, 9, 8, EdgeKind::Mountain),
            edge(21, 2, 9, EdgeKind::Mountain),
            edge(22, 4, 10, EdgeKind::Mountain),
            edge(23, 10, 8, EdgeKind::Mountain),
            edge(24, 2, 10, EdgeKind::Mountain),
            edge(25, 10, 1, EdgeKind::Mountain),
            edge(26, 8, 11, EdgeKind::Mountain),
            edge(27, 11, 7, EdgeKind::Mountain),
            edge(28, 0, 11, EdgeKind::Mountain),
            edge(29, 11, 1, EdgeKind::Mountain),
            edge(30, 8, 12, EdgeKind::Mountain),
            edge(31, 12, 5, EdgeKind::Mountain),
            edge(32, 0, 12, EdgeKind::Mountain),
            edge(33, 12, 3, EdgeKind::Mountain),
            edge(34, 3, 9, EdgeKind::Mountain),
            edge(35, 12, 9, EdgeKind::Valley),
            edge(36, 10, 11, EdgeKind::Valley),
        ],
        next_vertex_id: 13,
        next_edge_id: 37,
    }
}

/// 実機が表示していた20本の折り角。`poseAngles` をそのまま写した。
/// このうち ±180°(誤差 `1e-6` 以内)は **15本** で、修正前は7本が
/// `surface_rank` と食い違っていた。
fn live_frame_angles() -> HashMap<EdgeId, f64> {
    HashMap::from([
        (17, -180.0),
        (18, -180.0),
        (19, -178.265_130_385_534_97),
        (20, 180.0),
        (21, 180.0),
        (22, -3.062_204_584_590_538_5e-15),
        (23, 180.0),
        (24, 180.0),
        (25, 180.0),
        (26, 180.0),
        (27, -5.233_885_113_024_099e-15),
        (28, 180.0),
        (29, 180.0),
        (30, 179.999_999_999_999_97),
        (31, -178.265_130_385_534_97),
        (32, 180.0),
        (33, 180.0),
        (34, 180.0),
        (35, -1.734_869_614_465_027),
        (36, -180.0),
    ])
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

fn path_checkpoint_angles(angles: &HashMap<u32, f64>, checkpoint_deg: f64) -> HashMap<u32, f64> {
    angles
        .iter()
        .map(|(&edge, &angle)| (edge, angle.signum() * angle.abs().min(checkpoint_deg)))
        .collect()
}

#[test]
fn mixed_exact_stack_uses_every_folded_hinge_on_its_path() {
    let cp = diagonal_midline_square();
    let faces = extract_faces(&cp);
    let exact_angles = mixed_exact_angles();
    let path_angles = path_checkpoint_angles(&exact_angles, 179.999);
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
        if (angle.abs() - 180.0).abs() <= 1e-9 {
            assert_eq!(path_angles[&hinge], angle.signum() * 179.999);
        } else {
            assert_eq!(
                path_angles[&hinge], angle,
                "partial hinge {hinge} must stay at its final angle once reached"
            );
        }
    }
    let exact = propagate(&cp, &faces, &exact_angles);
    let exact_frame = to_frame3d(&cp, &faces, &exact);
    let path = propagate(&cp, &faces, &path_angles);

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
    let path_delta =
        approach_height(&path, 4, witness, normal) - approach_height(&path, 10, witness, normal);

    assert!(
        path_delta.abs() > 1e-9,
        "the all-fold path must separate faces 4 and 10: {path_delta:.3e}"
    );
    assert_eq!(
        rank(4) > rank(10),
        path_delta > 0.0,
        "exact rank {}/{} must follow the all-fold path",
        rank(4),
        rank(10)
    );
}

#[test]
fn same_final_angles_have_the_same_rank_for_propagate_and_each_selected_driver() {
    let cp = three_panel_strip();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 3);
    let final_angles = HashMap::from([(8, 180.0), (9, 180.0)]);
    let direct = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &final_angles));

    let selected_eight = solve_motion(
        &cp,
        &faces,
        &[Driver {
            hinge: 8,
            target_angle_deg: 180.0,
        }],
        Some(&HashMap::from([(9, 180.0)])),
        None,
        false,
    )
    .result;
    let selected_nine = solve_motion(
        &cp,
        &faces,
        &[Driver {
            hinge: 9,
            target_angle_deg: 180.0,
        }],
        Some(&HashMap::from([(8, 180.0)])),
        None,
        false,
    )
    .result;

    for solved in [&selected_eight, &selected_nine] {
        assert_eq!(solved.angles, final_angles);
        assert_eq!(rank_order(&solved.frame), rank_order(&direct));
    }
    assert_eq!(
        rank_order(&selected_eight.frame),
        rank_order(&selected_nine.frame),
        "surface rank must not depend on which exact hinge is the selected hard driver"
    );
}

#[test]
fn exact_constraint_chain_is_authoritative_without_a_synthetic_history() {
    let mut cp = three_panel_strip();
    cp.edges
        .iter_mut()
        .find(|edge| edge.id == 9)
        .expect("second hinge")
        .kind = EdgeKind::Valley;
    let faces = extract_faces(&cp);
    let final_angles = HashMap::from([(8, 180.0), (9, -180.0)]);
    let direct = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &final_angles));

    let (order, _provenance) = surface_order_from_angles(&cp, &faces, &final_angles, &[], None)
        .expect("厳密折り目の推移閉包だけで三層すべての上下が決まる");

    assert_eq!(order, rank_order(&direct));
}

#[test]
fn loop_closure_never_treats_a_propagated_flat_path_as_physical_authority() {
    let cp = diagonal_midline_square();
    let faces = extract_faces(&cp);
    let error = surface_order_from_angles_flat_path(&cp, &faces, &mixed_exact_angles())
        .expect_err("非木辺の閉包を解かない伝播経路は重なり順の証明にならない");

    assert!(
        error.contains("loop surface order"),
        "閉路専用のauthority gateより前で失敗した: {error}"
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

#[test]
fn single_exact_stack_preserves_non_exact_driver_context() {
    let cp = diagonal_midline_square();
    let faces = extract_faces(&cp);
    let exact_hinge = 17;
    let contextual = mixed_exact_angles()
        .into_iter()
        .map(|(hinge, angle)| {
            let retained = if hinge != exact_hinge && (angle.abs() - 180.0).abs() <= 1e-9 {
                angle.signum() * 170.0
            } else {
                angle
            };
            (hinge, retained)
        })
        .collect::<HashMap<_, _>>();
    assert_eq!(
        contextual
            .values()
            .filter(|angle| (angle.abs() - 180.0).abs() <= 1e-9)
            .count(),
        1
    );

    let drivers = |exact_angle| {
        let mut values = cp
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .map(|edge| Driver {
                hinge: edge.id,
                target_angle_deg: if edge.id == exact_hinge {
                    exact_angle
                } else {
                    contextual.get(&edge.id).copied().unwrap_or(0.0)
                },
            })
            .collect::<Vec<_>>();
        values.sort_unstable_by_key(|driver| driver.hinge);
        values
    };

    // この検査は追従経路ではなく、同じ指定を1回で解いた面順位の契約を比較する。
    // 接触検出のON/OFFへ単発solveの意味を抱き合わせない。
    let before_hard = solve_motion_once(&cp, &faces, &drivers(-179.999), None, None);
    let exact_hard = solve_motion_once(&cp, &faces, &drivers(-180.0), None, None);
    assert_eq!(
        rank_order(&exact_hard.frame),
        rank_order(&before_hard.frame)
    );
    for driver in drivers(-180.0) {
        if driver.hinge != exact_hinge {
            assert!(
                (exact_hard.angles[&driver.hinge] - driver.target_angle_deg).abs() <= 1e-12,
                "non-exact hard hinge {} changed",
                driver.hinge
            );
        }
    }

    let preferred = contextual
        .iter()
        .filter(|(hinge, _)| **hinge != exact_hinge)
        .map(|(&hinge, &angle)| (hinge, angle))
        .collect::<HashMap<_, _>>();
    let before_preferred = solve_motion_once(
        &cp,
        &faces,
        &[Driver {
            hinge: exact_hinge,
            target_angle_deg: -179.999,
        }],
        Some(&preferred),
        None,
    );
    let exact_preferred = solve_motion_once(
        &cp,
        &faces,
        &[Driver {
            hinge: exact_hinge,
            target_angle_deg: -180.0,
        }],
        Some(&preferred),
        None,
    );
    assert_eq!(
        rank_order(&exact_preferred.frame),
        rank_order(&before_preferred.frame),
        "before angles={:?}; exact angles={:?}",
        before_preferred.angles,
        exact_preferred.angles
    );
}

/// 180°に折り切った折り目がつなぐ2面について、`surface_rank` が折り目の向きと
/// 一致しているかを数える。カメラも画素も使わず、角度と重なり順の整合だけを見る。
///
/// 判定は製品側とは別に、面の世界法線から直に立てている。谷折り(角度が負)なら
/// 相手は基準面の紙の表側(法線の+側)へ、山折り(正)なら反対側へ来る。
fn exact_fold_rank_violations(
    cp: &ori3_model::CreasePattern,
    faces: &[ori3_cp::Face],
    angles: &HashMap<EdgeId, f64>,
    frame: &ori3_model::Frame3D,
) -> Vec<(EdgeId, FaceId, FaceId, f64)> {
    let polygons = frame
        .faces
        .iter()
        .map(|face| {
            (
                face.face,
                face.polygon
                    .iter()
                    .map(|point| glam::DVec3::from(*point))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let ranks = frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<std::collections::BTreeMap<_, _>>();
    let normal_of = |points: &Vec<glam::DVec3>| {
        let mut normal = glam::DVec3::ZERO;
        for index in 0..points.len() {
            normal += points[index].cross(points[(index + 1) % points.len()]);
        }
        normal.normalize()
    };
    let mut owners = std::collections::BTreeMap::<EdgeId, Vec<FaceId>>::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().push(face.id);
        }
    }
    let mut violations = Vec::new();
    for edge in &cp.edges {
        let Some(&angle) = angles.get(&edge.id) else {
            continue;
        };
        if (angle.abs() - 180.0).abs() > 1e-6 {
            continue;
        }
        let Some(pair) = owners.get(&edge.id).filter(|owners| owners.len() == 2) else {
            continue;
        };
        let (left, right) = (pair[0], pair[1]);
        let (Some(left_points), Some(right_points)) = (polygons.get(&left), polygons.get(&right))
        else {
            continue;
        };
        // 面の頂点は展開図で反時計回りなので、この法線が紙の表側を向く。
        let left_normal = normal_of(left_points);
        let right_normal = normal_of(right_points);
        // 完全に折り切っているなら、2面の紙の表は必ず逆を向く。
        assert!(
            left_normal.dot(right_normal) < -0.999_999,
            "折り目 {} は {angle}° なのに面 {left} と面 {right} の表が逆を向いていない",
            edge.id,
        );
        // 「上」は `surface_rank` が深度を測るのと同じ軸、つまり2面が乗る平面の
        // canonical法線(絶対値が最大の成分を正にした向き)で決める。世界のzで
        // 代用すると、束が立った平面(法線のzが0に近い姿勢)で判定が反転する。
        let up = {
            let absolute = left_normal.abs();
            let component = if absolute.x >= absolute.y && absolute.x >= absolute.z {
                left_normal.x
            } else if absolute.y >= absolute.z {
                left_normal.y
            } else {
                left_normal.z
            };
            if component < 0.0 {
                -left_normal
            } else {
                left_normal
            }
        };
        let left_front_is_up = left_normal.dot(up) > 0.0;
        let right_should_be_above = (angle < 0.0) == left_front_is_up;
        let right_is_above = ranks[&right] > ranks[&left];
        if right_is_above != right_should_be_above {
            violations.push((edge.id, left, right, angle));
        }
    }
    violations
}

/// 完全に折り切った折り目の上下が、`surface_rank` と必ず一致することを検査する。
///
/// 深度の差は丸めで壊れ得るが、折り目の向きは壊れない。実機で紙の裏が 66.88%
/// 見えていた `live-frame` の姿勢では、180°の折り目15本のうち **7本** がこの関係に
/// 反しており、それが「見えないはずの裏が見える」不具合の実体だった。
///
/// カメラも画素も使わず、明示的に与えた角度と、そこから伝播して得た `surface_rank`
/// という2つの値の整合だけを見る。solveの収束結果に期待値を結び付けていないので、
/// 計算機が変わっても結果は変わらない(CLAUDE.md §10.7.7)。
#[test]
fn exact_folds_agree_with_the_surface_rank() {
    let cases: [(&str, ori3_model::CreasePattern, HashMap<EdgeId, f64>, usize); 4] = [
        (
            "split-square-mountain",
            split_square(),
            HashMap::from([(6, 180.0)]),
            1,
        ),
        (
            "split-square-valley",
            split_square(),
            HashMap::from([(6, -180.0)]),
            1,
        ),
        (
            "diagonal-midline-mixed",
            diagonal_midline_square(),
            mixed_exact_angles(),
            10,
        ),
        ("live-frame", live_frame_square(), live_frame_angles(), 15),
    ];
    let mut failures = Vec::new();
    for (label, cp, angles, expected_exact_folds) in cases {
        let faces = extract_faces(&cp);
        let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &angles));
        let exact_folds = angles
            .values()
            .filter(|angle| (angle.abs() - 180.0).abs() <= 1e-6)
            .count();
        assert_eq!(
            exact_folds, expected_exact_folds,
            "{label}: 180°の折り目の本数が変わっている"
        );
        let violations = exact_fold_rank_violations(&cp, &faces, &angles, &frame);
        println!(
            "EXACT_FOLD_RANK case={label} exact_folds={exact_folds} violations={}",
            violations.len()
        );
        if !violations.is_empty() {
            failures.push(format!(
                "{label}: 180°の折り目{exact_folds}本中{}本で、surface_rankが折り目の向きに反している detail(edge,下,上,角度)={violations:?}",
                violations.len(),
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
