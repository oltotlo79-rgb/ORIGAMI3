
/// 179.999°の姿勢は面どうしが実際に離れているので、重なり順の正解は実測の隙間だけで
/// 一意に決まる(カメラも履歴も要らない)。その正解と、刻印した `surface_rank` が
/// 合っているかを数える。合否は付けず、数値を出すだけの測定である。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn surface_rank_against_the_measured_gap_at_179_999() {
    let diagrams = boundary_diagrams();
    let mut total_pairs = 0_usize;
    let mut total_mismatch = 0_usize;
    let mut total_mismatch_after = 0_usize;
    for diagram in &diagrams {
        for (hinge, _) in diagram.hinges.clone() {
            for sign in [1.0_f64, -1.0] {
                let (before, after) = endpoint_frames(diagram, hinge, sign);
                let ranks = |frame: &Frame3D| {
                    frame
                        .faces
                        .iter()
                        .map(|face| (face.face, face.surface_rank))
                        .collect::<BTreeMap<_, _>>()
                };
                let before_rank = ranks(&before.frame);
                let after_rank = ranks(&after.frame);
                let pairs = near_overlaps(&before.frame, 1e-3);
                let mut mismatch_before = Vec::new();
                let mut mismatch_after = Vec::new();
                for pair in &pairs {
                    // 隙間が丸めより十分大きい面対だけを正解の根拠にする。
                    if pair.gap.abs() < 1e-9 {
                        continue;
                    }
                    total_pairs += 1;
                    let truth_right_above = pair.gap > 0.0;
                    if (before_rank[&pair.right] > before_rank[&pair.left]) != truth_right_above {
                        mismatch_before.push((pair.left, pair.right, pair.gap));
                        total_mismatch += 1;
                    }
                    if (after_rank[&pair.right] > after_rank[&pair.left]) != truth_right_above {
                        mismatch_after.push((pair.left, pair.right, pair.gap));
                        total_mismatch_after += 1;
                    }
                }
                println!(
                    "GAPTRUTH diagram={} edge={hinge} sign={sign:+} near_pairs={} mismatch_at_179_999={} mismatch_at_180={} before_detail={:?} after_detail={:?}",
                    diagram.name,
                    pairs.len(),
                    mismatch_before.len(),
                    mismatch_after.len(),
                    mismatch_before,
                    mismatch_after,
                );
            }
        }
    }
    println!(
        "GAPTRUTH_TOTAL pairs={total_pairs} mismatch_at_179_999={total_mismatch} mismatch_at_180={total_mismatch_after}"
    );
}


/// 調査用。`surface_order_exact_endpoint_is_rank_stable_for_previous_19` が使う
/// 「warm start無しで解き直す」経路が、warm startで到達した姿勢と同じ形になるかを測る。
/// 形が違えば、重なり順が違うのは当然である(刻印する順は表示している形を説明する)。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_cold_solve_reaches_the_same_pose() {
    const PREVIOUS_RANK_CHANGES: [EdgeId; 19] = [
        125, 143, 181, 183, 297, 298, 309, 314, 352, 358, 362, 367, 380, 393, 394, 401, 402, 426,
        430,
    ];
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|item| item.name == "folded-sample.ori3")
        .expect("fixture");
    let mut worst = 0.0_f64;
    let mut differing = 0_usize;
    for hinge in PREVIOUS_RANK_CHANGES {
        for sign in [1.0_f64, -1.0] {
            let (_, after) = endpoint_frames(diagram, hinge, sign);
            let cold = solve_motion_with_contact_options(
                &diagram.cp,
                &diagram.faces,
                &[Driver {
                    hinge,
                    target_angle_deg: sign * 180.0,
                }],
                None,
                None,
                EXPLICIT_CONTACT_PREVENTION,
            );
            let delta = after
                .angles
                .iter()
                .map(|(edge, angle)| {
                    (angle - cold.result.angles.get(edge).copied().unwrap_or(f64::NAN)).abs()
                })
                .fold(0.0_f64, f64::max);
            let same_rank =
                surface_rank_order(&after.frame) == surface_rank_order(&cold.result.frame);
            if !same_rank {
                differing += 1;
            }
            worst = worst.max(delta);
            println!(
                "COLDPOSE edge={hinge} sign={sign:+} max_angle_difference_deg={delta:.6e} same_rank={same_rank}"
            );
        }
    }
    println!(
        "COLDPOSE_TOTAL cases={} worst_angle_difference_deg={worst:.6e} rank_differs={differing}",
        PREVIOUS_RANK_CHANGES.len() * 2
    );
}


/// 段階1の測定(残った2本)。**規則をいっさい使わず**、裂けていない姿勢の幾何だけから
/// 上下を決める。折り目を 179.0 → 179.5 → 179.9 → 179.99 → 179.999 → 180.0 と送り、
/// 各姿勢で(a)裂けの量、(b)隣接2面の重心が基準面の平面からどちら側へ離れているか、
/// (c)ほぼ平行で実面積が重なる面対の隙間の符号、を並べて印字する。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_remaining_two_creases_gap_sign_ladder() {
    let diagrams = boundary_diagrams();
    for (name, hinge) in [
        ("diagonal-midline-square", 12_u32),
        ("folded-sample.ori3", 425_u32),
    ] {
        let diagram = diagrams
            .iter()
            .find(|diagram| diagram.name == name)
            .expect("diagram exists");
        // 折り目に接する2面(展開図の接続だけで決まる。上下の規則は使わない)
        let adjacent = diagram
            .faces
            .iter()
            .filter(|face| face.edges.contains(&hinge))
            .map(|face| face.id)
            .collect::<Vec<_>>();
        println!("LADDER_SETUP diagram={name} edge={hinge} adjacent_faces={adjacent:?}");
        for sign in [1.0_f64, -1.0] {
            let mut warm = None::<HashMap<EdgeId, f64>>;
            for absolute in WARMUP_ABS {
                let motion = solve_motion_with_contact_options(
                    &diagram.cp,
                    &diagram.faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: sign * absolute,
                    }],
                    None,
                    warm.as_ref(),
                    EXPLICIT_CONTACT_PREVENTION,
                );
                warm = Some(motion.result.angles);
            }
            for absolute in [179.0, 179.5, 179.9, 179.99, 179.999, 180.0] {
                let motion = solve_motion_with_contact_options(
                    &diagram.cp,
                    &diagram.faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: sign * absolute,
                    }],
                    None,
                    warm.as_ref(),
                    EXPLICIT_CONTACT_PREVENTION,
                );
                let frame = motion.result.frame.clone();
                let seam = ori3_rigid::max_seam_gap(&diagram.cp, &diagram.faces, &frame);
                let ranks = frame
                    .faces
                    .iter()
                    .map(|face| (face.face, face.surface_rank))
                    .collect::<BTreeMap<_, _>>();
                let polygons = frame_polygons(&frame);
                // (b) 隣接2面: 基準面の平面から相手の重心までの符号付き高さ。
                // 折り切る手前ではこの符号が「紙としてどちらが上か」そのものである。
                let mut adjacent_detail = Vec::new();
                if adjacent.len() == 2 {
                    let (left, right) = (adjacent[0], adjacent[1]);
                    let left_points = &polygons[&left];
                    let right_points = &polygons[&right];
                    if let (Some(left_normal), Some(right_normal)) =
                        (polygon_normal3(left_points), polygon_normal3(right_points))
                    {
                        let up = canonical3(left_normal);
                        let centroid = |points: &[V3]| {
                            points.iter().fold(V3::ZERO, |sum, &p| sum + p) / points.len() as f64
                        };
                        let height = (centroid(right_points) - centroid(left_points)).dot(up);
                        adjacent_detail.push((
                            left,
                            right,
                            height,
                            ranks[&right] > ranks[&left],
                            left_normal.dot(up),
                            right_normal.dot(up),
                        ));
                        println!(
                            "LADDER_NORMAL diagram={name} edge={hinge} sign={sign:+} target={absolute} face{left}_normal=({:.12},{:.12},{:.12}) face{right}_normal=({:.12},{:.12},{:.12}) up=({:.12},{:.12},{:.12})",
                            left_normal.x,
                            left_normal.y,
                            left_normal.z,
                            right_normal.x,
                            right_normal.y,
                            right_normal.z,
                            up.x,
                            up.y,
                            up.z,
                        );
                    }
                }
                // (c) ほぼ平行で実面積が重なる面対の隙間
                let pairs = near_overlaps(&frame, 1e-3);
                let detail = pairs
                    .iter()
                    .filter(|pair| pair.gap.abs() >= 1e-9)
                    .map(|pair| {
                        (
                            pair.left,
                            pair.right,
                            pair.gap,
                            (ranks[&pair.right] > ranks[&pair.left]) == (pair.gap > 0.0),
                        )
                    })
                    .collect::<Vec<_>>();
                let mut near_flat = motion
                    .result
                    .angles
                    .iter()
                    .filter(|(_, angle)| angle.abs() >= 179.9)
                    .map(|(&edge, &angle)| (edge, angle))
                    .collect::<Vec<_>>();
                near_flat.sort_by_key(|&(edge, _)| edge);
                let near_flat_faces = near_flat
                    .iter()
                    .map(|&(edge, angle)| {
                        let touching = diagram
                            .faces
                            .iter()
                            .filter(|face| face.edges.contains(&edge))
                            .map(|face| face.id)
                            .collect::<Vec<_>>();
                        (edge, angle, touching)
                    })
                    .collect::<Vec<_>>();
                println!(
                    "LADDER_FLAT_FACES diagram={name} edge={hinge} sign={sign:+} target={absolute} {near_flat_faces:?}"
                );
                println!(
                    "LADDER diagram={name} edge={hinge} sign={sign:+} target={absolute} driver={:.9} seam={seam:.3e} source={:?} adjacent={adjacent_detail:?} mismatched_pairs={} near_pairs={} ranks={:?} detail={detail:?} near_flat={near_flat:?}",
                    motion
                        .result
                        .angles
                        .get(&hinge)
                        .copied()
                        .unwrap_or(f64::NAN),
                    motion.surface_order,
                    detail.iter().filter(|item| !item.3).count(),
                    detail.len(),
                    if diagram.faces.len() <= 16 {
                        format!("{ranks:?}")
                    } else {
                        String::from("-")
                    },
                );
                if diagram.faces.len() <= 16 {
                    let mut all = motion
                        .result
                        .angles
                        .iter()
                        .map(|(&edge, &angle)| (edge, angle))
                        .collect::<Vec<_>>();
                    all.sort_by_key(|&(edge, _)| edge);
                    println!(
                        "LADDER_ANGLES diagram={name} edge={hinge} sign={sign:+} target={absolute} angles={all:?}"
                    );
                }
                warm = Some(motion.result.angles);
            }
        }
    }
}


/// 段階1(その2): `folded-sample.ori3` の辺425で食い違う面対が、経路のどの段で
/// どちらへ決まったのかを追う。製品コードは読むだけで、経路は公開APIで再現する。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_edge425_canonical_path_decision() {
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "folded-sample.ori3")
        .expect("fixture");
    let hinge = 425_u32;
    for sign in [-1.0_f64, 1.0] {
        for target in [179.999_f64, 180.0] {
            let mut warm = None::<HashMap<EdgeId, f64>>;
            for absolute in WARMUP_ABS.iter().copied().chain(
                BOUNDARY_ABS
                    .iter()
                    .copied()
                    .take_while(|&value| value <= target),
            ) {
                let motion = solve_motion_with_contact_options(
                    &diagram.cp,
                    &diagram.faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: sign * absolute,
                    }],
                    None,
                    warm.as_ref(),
                    EXPLICIT_CONTACT_PREVENTION,
                );
                warm = Some(motion.result.angles);
            }
            let displayed_angles = warm.expect("ladder ran");
            let (path, exact) = canonical_path_frames(diagram, &displayed_angles);
            let exact_polygons = frame_polygons(&exact);
            let path_polygons = path.iter().map(frame_polygons).collect::<Vec<_>>();
            for (left, right) in [(6_u32, 8_u32), (7_u32, 9_u32)] {
                let left_points = &exact_polygons[&left];
                let right_points = &exact_polygons[&right];
                let (Some(plane), Some(right_normal)) = (
                    overlap_plane(left_points),
                    polygon_normal3(right_points).map(canonical3),
                ) else {
                    println!(
                        "PATHDEC sign={sign:+} target={target} pair=({left},{right}) no_plane"
                    );
                    continue;
                };
                let parallel = plane.normal.dot(right_normal);
                let coplanar_error = left_points
                    .iter()
                    .chain(right_points)
                    .map(|point| plane.normal.dot(*point - plane.origin).abs())
                    .fold(0.0_f64, f64::max);
                let left_2d = project2(left_points, &plane);
                let right_2d = project2(right_points, &plane);
                let left_triangles = triangulate_polygon(&left_2d);
                let right_triangles = triangulate_polygon(&right_2d);
                let Some((witness, area)) =
                    overlap_witness(&left_2d, &left_triangles, &right_2d, &right_triangles)
                else {
                    println!(
                        "PATHDEC sign={sign:+} target={target} pair=({left},{right}) no_overlap"
                    );
                    continue;
                };
                let overlap = CoincidentOverlap {
                    left,
                    right,
                    normal: plane.normal,
                    witness: plane.origin + plane.u * witness[0] + plane.v * witness[1],
                    shared_hinge: None,
                };
                let heights = path_polygons
                    .iter()
                    .enumerate()
                    .map(|(index, probe)| {
                        (
                            CANONICAL_CHECKPOINT_DEG[index],
                            probe_height_difference(&exact_polygons, probe, &overlap),
                        )
                    })
                    .collect::<Vec<_>>();
                let displayed = frame_polygons(
                    &solve_motion_with_contact_options(
                        &diagram.cp,
                        &diagram.faces,
                        &[Driver {
                            hinge,
                            target_angle_deg: sign * target,
                        }],
                        None,
                        Some(&displayed_angles),
                        EXPLICIT_CONTACT_PREVENTION,
                    )
                    .result
                    .frame,
                );
                let displayed_difference =
                    probe_height_difference(&exact_polygons, &displayed, &overlap);
                println!(
                    "PATHDEC sign={sign:+} target={target} pair=({left},{right}) parallel={parallel:.12} coplanar_error={coplanar_error:.3e} area={area:.3e} exact_normal=({:.12},{:.12},{:.12}) displayed_height_difference={displayed_difference:?} path={heights:?}",
                    plane.normal.x, plane.normal.y, plane.normal.z,
                );
            }
        }
    }
}


/// 段階1(その3): 米印の辺12で、上下の向きを決める「正準法線」がどの姿勢で
/// どちらを向くかを並べる。折り目の向きの規則は
/// **snapした表示姿勢を伝播した形**の面法線を、深度の規則は
/// **経路の終点(exact frame)**の面法線を使うため、両方を出す。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_kome_edge12_canonical_axis() {
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "diagonal-midline-square")
        .expect("fixture");
    let hinge = 12_u32;
    for sign in [1.0_f64, -1.0] {
        for target in [179.999_f64, 180.0] {
            let mut warm = None::<HashMap<EdgeId, f64>>;
            for absolute in WARMUP_ABS.iter().copied().chain(
                BOUNDARY_ABS
                    .iter()
                    .copied()
                    .take_while(|&value| value <= target),
            ) {
                let motion = solve_motion_with_contact_options(
                    &diagram.cp,
                    &diagram.faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: sign * absolute,
                    }],
                    None,
                    warm.as_ref(),
                    EXPLICIT_CONTACT_PREVENTION,
                );
                warm = Some(motion.result.angles);
            }
            let displayed_angles = warm.expect("ladder ran");
            let snapped = displayed_angles
                .iter()
                .map(|(&edge, &angle)| {
                    (
                        edge,
                        if angle.abs() >= CANONICAL_STACK_FLAT_DEG {
                            angle.signum() * 180.0
                        } else {
                            angle
                        },
                    )
                })
                .collect::<HashMap<_, _>>();
            let propagated = to_frame3d(
                &diagram.cp,
                &diagram.faces,
                &propagate(&diagram.cp, &diagram.faces, &snapped),
            );
            let (_, exact) = canonical_path_frames(diagram, &displayed_angles);
            for (label, frame) in [
                ("snapped_propagated", &propagated),
                ("canonical_exact", &exact),
            ] {
                let polygons = frame_polygons(frame);
                for face in [3_u32, 4_u32] {
                    let normal = polygon_normal3(&polygons[&face]).expect("normal");
                    let up = canonical3(normal);
                    println!(
                        "AXIS sign={sign:+} target={target} {label} face={face} normal=({:.12},{:.12},{:.12}) up_is_normal={} abs_x_minus_abs_y={:.6e}",
                        normal.x,
                        normal.y,
                        normal.z,
                        normal.dot(up) > 0.0,
                        normal.x.abs() - normal.y.abs(),
                    );
                }
            }
        }
    }
}


/// 段階1(その4): 経路の終点(exact frame)で「法線が平行で投影が重なる」面対の
/// 平面からのずれが、どの桁に分布しているかを数える。
/// `surface_order.rs::COPLANAR_EPS`(1e-8)がこの分布のどこにあるかを見るための測定。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_exact_frame_coplanarity_histogram() {
    let diagrams = boundary_diagrams();
    let mut histogram = BTreeMap::<i32, usize>::new();
    let mut worst_same_stack = 0.0_f64;
    let mut worst_seam = 0.0_f64;
    for diagram in &diagrams {
        for &(hinge, _) in &diagram.hinges {
            for sign in [1.0_f64, -1.0] {
                let mut warm = None::<HashMap<EdgeId, f64>>;
                for absolute in WARMUP_ABS.iter().copied().chain(BOUNDARY_ABS) {
                    let motion = solve_motion_with_contact_options(
                        &diagram.cp,
                        &diagram.faces,
                        &[Driver {
                            hinge,
                            target_angle_deg: sign * absolute,
                        }],
                        None,
                        warm.as_ref(),
                        EXPLICIT_CONTACT_PREVENTION,
                    );
                    warm = Some(motion.result.angles);
                }
                let displayed_angles = warm.expect("ladder ran");
                let (_, exact) = canonical_path_frames(diagram, &displayed_angles);
                worst_seam = worst_seam.max(ori3_rigid::max_seam_gap(
                    &diagram.cp,
                    &diagram.faces,
                    &exact,
                ));
                let polygons = frame_polygons(&exact);
                let mut face_ids = polygons.keys().copied().collect::<Vec<_>>();
                face_ids.sort_unstable();
                for (left_index, &left) in face_ids.iter().enumerate() {
                    for &right in &face_ids[left_index + 1..] {
                        let left_points = &polygons[&left];
                        let right_points = &polygons[&right];
                        let (Some(plane), Some(right_normal)) = (
                            overlap_plane(left_points),
                            polygon_normal3(right_points).map(canonical3),
                        ) else {
                            continue;
                        };
                        if plane.normal.dot(right_normal).abs() < 1.0 - 1e-8 {
                            continue;
                        }
                        let left_2d = project2(left_points, &plane);
                        let right_2d = project2(right_points, &plane);
                        let left_triangles = triangulate_polygon(&left_2d);
                        let right_triangles = triangulate_polygon(&right_2d);
                        if overlap_witness(&left_2d, &left_triangles, &right_2d, &right_triangles)
                            .is_none()
                        {
                            continue;
                        }
                        let error = left_points
                            .iter()
                            .chain(right_points)
                            .map(|point| plane.normal.dot(*point - plane.origin).abs())
                            .fold(0.0_f64, f64::max);
                        let decade = if error <= 0.0 {
                            -99
                        } else {
                            error.log10().floor() as i32
                        };
                        *histogram.entry(decade).or_default() += 1;
                        if error < 1e-4 {
                            worst_same_stack = worst_same_stack.max(error);
                        }
                    }
                }
            }
        }
    }
    println!(
        "COPLANARITY_HISTOGRAM worst_exact_seam={worst_seam:.3e} worst_error_below_1e-4={worst_same_stack:.3e} decades={histogram:?}"
    );
}


/// 段階1(その5): 画面側 (`surfaceOwner.ts::updateBatchViewOrder`) は
/// **Float32のposition bufferから読んだ頂点**で法線を組み立て、その絶対値を
/// 比べて正準法線の向きを決める。対称に折り切った形で \|x\| と \|y\| が
/// どれだけ違うかを、Float32へ丸めた頂点で測る。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_kome_edge12_float32_axis_choice() {
    let diagrams = boundary_diagrams();
    let diagram = diagrams
        .iter()
        .find(|diagram| diagram.name == "diagonal-midline-square")
        .expect("fixture");
    let hinge = 12_u32;
    for sign in [1.0_f64, -1.0] {
        for target in [179.999_f64, 180.0] {
            let mut warm = None::<HashMap<EdgeId, f64>>;
            for absolute in WARMUP_ABS.iter().copied().chain(
                BOUNDARY_ABS
                    .iter()
                    .copied()
                    .take_while(|&value| value <= target),
            ) {
                let motion = solve_motion_with_contact_options(
                    &diagram.cp,
                    &diagram.faces,
                    &[Driver {
                        hinge,
                        target_angle_deg: sign * absolute,
                    }],
                    None,
                    warm.as_ref(),
                    EXPLICIT_CONTACT_PREVENTION,
                );
                warm = Some(motion.result.angles);
            }
            let displayed = solve_motion_with_contact_options(
                &diagram.cp,
                &diagram.faces,
                &[Driver {
                    hinge,
                    target_angle_deg: sign * target,
                }],
                None,
                warm.as_ref(),
                EXPLICIT_CONTACT_PREVENTION,
            )
            .result
            .frame;
            let polygons = frame_polygons(&displayed);
            for face in [3_u32, 4_u32] {
                let exact_points = &polygons[&face];
                // 画面側と同じ: 頂点はFloat32、そこから先の掛け算・足し算はf64。
                let float32_points = exact_points
                    .iter()
                    .map(|point| {
                        V3::new(
                            f64::from(point.x as f32),
                            f64::from(point.y as f32),
                            f64::from(point.z as f32),
                        )
                    })
                    .collect::<Vec<_>>();
                let triangle_normal = |points: &[V3]| {
                    let mut normal = V3::ZERO;
                    for indices in &diagram.triangles[&face] {
                        let a = points[indices[0]];
                        let b = points[indices[1]];
                        let c = points[indices[2]];
                        normal += (b - a).cross(c - a);
                    }
                    normal.normalize()
                };
                let exact_normal = triangle_normal(exact_points);
                let float32_normal = triangle_normal(&float32_points);
                println!(
                    "F32AXIS sign={sign:+} target={target} face={face} f64=({:.12},{:.12},{:.12}) f64_abs_x_minus_abs_y={:.6e} f32=({:.12},{:.12},{:.12}) f32_abs_x_minus_abs_y={:.6e} f64_axis={} f32_axis={}",
                    exact_normal.x,
                    exact_normal.y,
                    exact_normal.z,
                    exact_normal.x.abs() - exact_normal.y.abs(),
                    float32_normal.x,
                    float32_normal.y,
                    float32_normal.z,
                    float32_normal.x.abs() - float32_normal.y.abs(),
                    if exact_normal.x.abs() >= exact_normal.y.abs() {
                        "x"
                    } else {
                        "y"
                    },
                    if float32_normal.x.abs() >= float32_normal.y.abs() {
                        "x"
                    } else {
                        "y"
                    },
                );
            }
        }
    }
}


/// 調査用。入力座標を1 ULPだけ動かしたときに、179.999°の姿勢で測った面対の
/// 隙間がどれだけ動くか(＝丸めの雑音の大きさ)を測る。合否は付けない。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_gap_noise_from_one_ulp_of_input() {
    let diagrams = boundary_diagrams();
    let base = diagrams
        .iter()
        .find(|item| item.name == "folded-sample.ori3")
        .expect("the folded acceptance fixture exists");
    // 入力の展開図の頂点座標を全て1 ULPだけ大きい側へ動かした複製。
    let mut nudged_cp = base.cp.clone();
    for vertex in &mut nudged_cp.vertices {
        for coordinate in &mut vertex.pos {
            *coordinate = f64::from_bits(coordinate.to_bits() + 1);
        }
    }
    let nudged = diagram(
        "folded-sample.ori3",
        nudged_cp,
        base.paper_width,
        base.paper_height,
    );

    let mut noise = Vec::<f64>::new();
    let mut sign_flips = 0_usize;
    let mut compared = 0_usize;
    for hinge in [306_u32, 425, 125, 12, 181, 297] {
        for sign in [1.0_f64, -1.0] {
            let (base_before, base_after) = endpoint_frames(base, hinge, sign);
            let (nudged_before, _) = endpoint_frames(&nudged, hinge, sign);
            let base_gaps = near_overlaps(&base_before.frame, f64::INFINITY)
                .into_iter()
                .map(|pair| ((pair.left, pair.right), pair.gap))
                .collect::<BTreeMap<_, _>>();
            let nudged_gaps = near_overlaps(&nudged_before.frame, f64::INFINITY)
                .into_iter()
                .map(|pair| ((pair.left, pair.right), pair.gap))
                .collect::<BTreeMap<_, _>>();
            for (key, base_gap) in &base_gaps {
                let Some(nudged_gap) = nudged_gaps.get(key) else {
                    continue;
                };
                compared += 1;
                noise.push((base_gap - nudged_gap).abs());
                if (*base_gap > 0.0) != (*nudged_gap > 0.0) {
                    sign_flips += 1;
                    println!(
                        "ULPFLIP edge={hinge} sign={sign:+} pair={key:?} base={base_gap:.6e} nudged={nudged_gap:.6e}"
                    );
                }
            }
            println!(
                "ULPNOISE edge={hinge} sign={sign:+} base_pairs={} nudged_pairs={} seam_before={:.3e} seam_after={:.3e}",
                base_gaps.len(),
                nudged_gaps.len(),
                ori3_rigid::max_seam_gap(&base.cp, &base.faces, &base_before.frame),
                ori3_rigid::max_seam_gap(&base.cp, &base.faces, &base_after.frame),
            );
        }
    }
    noise.sort_by(f64::total_cmp);
    let quantile = |ratio: f64| {
        noise
            .get(((noise.len() as f64 - 1.0) * ratio) as usize)
            .copied()
            .unwrap_or(f64::NAN)
    };
    println!(
        "ULPNOISE_TOTAL compared={compared} sign_flips={sign_flips} median={:.6e} p90={:.6e} p99={:.6e} max={:.6e}",
        quantile(0.5),
        quantile(0.9),
        quantile(0.99),
        noise.last().copied().unwrap_or(f64::NAN),
    );
}


/// 調査用。`surface_order_179_999_to_180_all_110_creases` が落ちる折り目について、
/// 順位が入れ替わった面対の隙間の実測値を出す。合否は付けない。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_rank_flip_pairs_measured_gap() {
    let diagrams = boundary_diagrams();
    for (name, hinge, sign) in [
        ("folded-sample.ori3", 306_u32, 1.0_f64),
        ("folded-sample.ori3", 306, -1.0),
        ("diagonal-midline-square", 12, 1.0),
        ("diagonal-midline-square", 12, -1.0),
    ] {
        let diagram = diagrams
            .iter()
            .find(|diagram| diagram.name == name)
            .expect("diagram exists");
        let (before, after) = endpoint_frames(diagram, hinge, sign);
        let rank_of = |frame: &Frame3D| {
            frame
                .faces
                .iter()
                .map(|face| (face.face, face.surface_rank))
                .collect::<BTreeMap<_, _>>()
        };
        let before_rank = rank_of(&before.frame);
        let after_rank = rank_of(&after.frame);
        let before_gaps = near_overlaps(&before.frame, f64::INFINITY)
            .into_iter()
            .map(|pair| ((pair.left, pair.right), pair.gap))
            .collect::<BTreeMap<_, _>>();
        let after_gaps = near_overlaps(&after.frame, f64::INFINITY)
            .into_iter()
            .map(|pair| ((pair.left, pair.right), pair.gap))
            .collect::<BTreeMap<_, _>>();
        println!(
            "FLIPSETUP diagram={name} edge={hinge} sign={sign:+} seam_before={:.3e} seam_after={:.3e} before_pairs={} after_pairs={}",
            ori3_rigid::max_seam_gap(&diagram.cp, &diagram.faces, &before.frame),
            ori3_rigid::max_seam_gap(&diagram.cp, &diagram.faces, &after.frame),
            before_gaps.len(),
            after_gaps.len(),
        );
        let mut faces = before_rank.keys().copied().collect::<Vec<_>>();
        faces.sort_unstable();
        let exact_error = |state: &EndpointState, left: FaceId, right: FaceId| {
            let (_, exact) = canonical_path_frames(diagram, &state.angles);
            let polygons = frame_polygons(&exact);
            let left_points = polygons.get(&left)?.clone();
            let right_points = polygons.get(&right)?.clone();
            let plane = overlap_plane(&left_points)?;
            let right_normal = polygon_normal3(&right_points).map(canonical3)?;
            let error = left_points
                .iter()
                .chain(&right_points)
                .map(|point| plane.normal.dot(*point - plane.origin).abs())
                .fold(0.0_f64, f64::max);
            Some((plane.normal.dot(right_normal), error))
        };
        for (left_index, &left) in faces.iter().enumerate() {
            for &right in &faces[left_index + 1..] {
                if (before_rank[&right] > before_rank[&left])
                    == (after_rank[&right] > after_rank[&left])
                {
                    continue;
                }
                println!(
                    "FLIPPAIR diagram={name} edge={hinge} sign={sign:+} pair=({left},{right}) before_gap={:?} after_gap={:?} vertex_move={:.3e} before_exact={:?} after_exact={:?}",
                    before_gaps
                        .get(&(left, right))
                        .map(|gap| format!("{gap:.6e}")),
                    after_gaps
                        .get(&(left, right))
                        .map(|gap| format!("{gap:.6e}")),
                    max_vertex_distance(&before.frame, &after.frame),
                    exact_error(&before, left, right).map(|(parallel, error)| format!(
                        "parallel={parallel:.9} coplanar_error={error:.6e}"
                    )),
                    exact_error(&after, left, right).map(|(parallel, error)| format!(
                        "parallel={parallel:.9} coplanar_error={error:.6e}"
                    )),
                );
            }
        }
    }
}
