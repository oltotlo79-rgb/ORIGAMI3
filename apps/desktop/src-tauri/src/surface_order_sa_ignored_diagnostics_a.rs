#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn live_frame_boundary_back_pixel_detail() {
    let cp = live_frame_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("live-frame-612", cp.clone(), 1.0, 1.0);
    let frame = live_frame_frame(&cp, &faces);
    for (azimuth_deg, elevation_deg, target_x, target_y) in
        [(20_i32, 20_i32, 730_usize, 255_usize), (140, -70, 163, 249)]
    {
        let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
        let mut rendered = render_faces(&diagram, &frame, VIEWPORT, view);
        rendered.sort_by_key(render_face_owner_key);
        let target = target_y * VIEWPORT + target_x;
        println!("--- az={azimuth_deg} el={elevation_deg} pixel=({target_x},{target_y}) ---");
        let image = visual_image(&diagram, &frame, VIEWPORT, view);
        for dy in -2_i32..=2 {
            let row = (-2_i32..=2)
                .map(|dx| {
                    let x = target_x as i32 + dx;
                    let y = target_y as i32 + dy;
                    match image.pixels[y as usize * VIEWPORT + x as usize] {
                        None => "----".to_string(),
                        Some(pixel) => format!(
                            "{}{}",
                            if pixel.back_facing { "B" } else { "F" },
                            pixel.face
                        ),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            println!("  neighbours dy={dy:+}: {row}");
        }
        for face in &rendered {
            for (index, triangle) in face.triangles.iter().enumerate() {
                let mut hit = None;
                rasterize(
                    |pixel, depth| {
                        if pixel == target {
                            hit = Some(depth);
                        }
                    },
                    triangle,
                    VIEWPORT,
                );
                if let Some(depth) = hit {
                    println!(
                        "  face={} rank={} side={} group={} back_facing={} tri={index} depth={depth:.9} key={:?}",
                        face.face,
                        face.surface_rank,
                        face.side,
                        face.coplanar_group,
                        triangle.back_facing,
                        render_face_owner_key(face),
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1b_edge36_pose_back_pixels_are_tie_or_geometry() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1b-edge36-pose", cp.clone(), 1.0, 1.0);
    let default_camera = camera(1.0, 1.0, 1.0);
    for (pose, final_angles) in [
        ("angles-now", zero_back_user_angles_now()),
        ("angles-old", zero_back_user_angles()),
    ] {
        let result = zero_back_warm_solve_to(
            &cp,
            &faces,
            &final_angles,
            ZeroBackWarmPath::Edge36Only,
            200,
        );
        let (front, back, covered) = covered_back_counts(&diagram, &result.frame, default_camera);
        let (strict_front, strict_back) =
            strict_nearest_counts(&diagram, &result.frame, default_camera);
        let angles = result
            .angles
            .iter()
            .map(|(&hinge, &angle)| (hinge, angle))
            .collect::<BTreeMap<_, _>>();
        println!(
            "STAGE1B pose={pose} rule_front={front} rule_back={back} rule_covered_back={covered} strict_front={strict_front} strict_back={strict_back} angles={:?}",
            angles
                .iter()
                .map(|(hinge, angle)| format!("{hinge}:{angle:.3}"))
                .collect::<Vec<_>>(),
        );
    }
}


/// `ORI3_STAGE1_IMAGE_DIR` を指定して実行したときだけ、CPU rasterの見た目をPPMで書き出す。
/// 既定では何も書かないので、追跡対象のファイルは変化しない。
#[test]
#[ignore = "調査用。ORI3_STAGE1_IMAGE_DIR を指定したときだけ画像を書く"]
fn stage1d_write_pose_images() {
    let Ok(directory) = std::env::var("ORI3_STAGE1_IMAGE_DIR") else {
        println!("STAGE1D skipped: ORI3_STAGE1_IMAGE_DIR is unset");
        return;
    };
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1d-images", cp.clone(), 1.0, 1.0);
    let edge36 = zero_back_warm_solve_to(
        &cp,
        &faces,
        &zero_back_user_angles(),
        ZeroBackWarmPath::Edge36Only,
        200,
    );
    let user = zero_back_user_frame(&cp, &faces);
    for (name, frame) in [("edge36-only-200", &edge36.frame), ("user-pose", &user)] {
        for (view_name, view) in [
            ("front", camera(1.0, 1.0, 1.0)),
            ("back", camera(1.0, 1.0, -1.0)),
        ] {
            let image = visual_image(&diagram, frame, VIEWPORT, view);
            let mut bytes = format!("P6\n{VIEWPORT} {VIEWPORT}\n255\n").into_bytes();
            for pixel in &image.pixels {
                let rgb = match pixel {
                    None => [207_u8, 203, 194],
                    Some(pixel) if pixel.back_facing => [255, 255, 255],
                    Some(_) => [237, 28, 36],
                };
                bytes.extend_from_slice(&rgb);
            }
            let path = format!("{directory}/stage1d-{name}-{view_name}.ppm");
            std::fs::write(&path, bytes).expect("write diagnostic image");
            println!("STAGE1D wrote {path}");
        }
    }
}

#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1c_edge36_pose_rule_versus_strict_over_612_directions() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1c-edge36-pose-612", cp.clone(), 1.0, 1.0);
    let result = zero_back_warm_solve_to(
        &cp,
        &faces,
        &zero_back_user_angles(),
        ZeroBackWarmPath::Edge36Only,
        200,
    );
    let mut directions = 0_usize;
    let mut rule_back_total = 0_u64;
    let mut strict_back_total = 0_u64;
    let mut worse_directions = 0_usize;
    let mut worst = (0_i64, 0_i32, 0_i32);
    for elevation_deg in (-80_i32..=80).step_by(10) {
        for azimuth_deg in (0_i32..=350).step_by(10) {
            let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
            let image = visual_image(&diagram, &result.frame, VIEWPORT, view);
            let (_, rule_back) = classified_fill_counts(&image);
            let (_, strict_back) = strict_nearest_counts(&diagram, &result.frame, view);
            directions += 1;
            rule_back_total += rule_back;
            strict_back_total += strict_back;
            let difference = rule_back as i64 - strict_back as i64;
            if difference > 0 {
                worse_directions += 1;
                if difference > worst.0 {
                    worst = (difference, azimuth_deg, elevation_deg);
                }
            }
        }
    }
    println!(
        "STAGE1C directions={directions} rule_back_total={rule_back_total} strict_back_total={strict_back_total} directions_rule_worse_than_strict={worse_directions} worst_excess={} at_az={} el={}",
        worst.0, worst.1, worst.2,
    );
}

#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1_surface_rank_direct_versus_warm_start() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1-rank-comparison", cp.clone(), 1.0, 1.0);
    let default_camera = camera(1.0, 1.0, 1.0);

    for (pose, final_angles) in [
        ("angles-now", zero_back_user_angles_now()),
        ("angles-old", zero_back_user_angles()),
    ] {
        let mut methods: Vec<(String, Frame3D, HashMap<EdgeId, f64>)> = Vec::new();

        // A0: 最終角をそのまま propagate して組み立てる基準形。
        let direct = zero_back_apply_overlap(&cp, &faces, {
            let folded = propagate(&cp, &faces, &final_angles);
            to_frame3d(&cp, &faces, &folded)
        });
        methods.push((
            "A0-propagate-direct".to_string(),
            direct,
            final_angles.clone(),
        ));

        // A1: 最終角をいきなり全部hardで1回solveする(warm無し)。
        let mut hard = final_angles
            .iter()
            .map(|(&hinge, &target_angle_deg)| Driver {
                hinge,
                target_angle_deg,
            })
            .collect::<Vec<_>>();
        hard.sort_unstable_by_key(|driver| driver.hinge);
        let cold = solve_motion(&cp, &faces, &hard, None, None, true);
        methods.push((
            "A1-solve-cold-one-shot".to_string(),
            zero_back_apply_overlap(&cp, &faces, cold.result.frame),
            cold.result.angles,
        ));

        // B: 0度から200段で少しずつ動かし、直前の解をwarm startにする。
        for path in [
            ZeroBackWarmPath::AllHardControl,
            ZeroBackWarmPath::Active36WithPreferred,
            ZeroBackWarmPath::Edge36Only,
        ] {
            let result = zero_back_warm_solve_to(&cp, &faces, &final_angles, path, 200);
            methods.push((
                format!("B-{}-200stages", path.label()),
                result.frame,
                result.angles,
            ));
        }

        let mut reference_ranks: Option<BTreeMap<FaceId, u32>> = None;
        for (label, frame, angles) in &methods {
            let ranks = frame
                .faces
                .iter()
                .map(|face| (face.face, face.surface_rank))
                .collect::<BTreeMap<_, _>>();
            let mismatched = reference_ranks.as_ref().map(|reference| {
                reference
                    .iter()
                    .filter(|(face, rank)| ranks.get(face) != Some(rank))
                    .map(|(&face, _)| face)
                    .collect::<Vec<_>>()
            });
            let (front, back, covered) = covered_back_counts(&diagram, frame, default_camera);
            let mut maximum_angle_delta = 0.0_f64;
            for (&hinge, &target) in &final_angles {
                let actual = angles.get(&hinge).copied().unwrap_or(0.0);
                let delta = (actual - target + 180.0).rem_euclid(360.0) - 180.0;
                maximum_angle_delta = maximum_angle_delta.max(delta.abs());
            }
            println!(
                "STAGE1 pose={pose} method={label} ranks={:?} mismatch_count={} mismatch_faces={:?} front={front} back={back} covered_back={covered} back_ratio={:.6}% max_angle_delta={maximum_angle_delta:.9}",
                ranks.values().copied().collect::<Vec<_>>(),
                mismatched.as_ref().map_or(0, Vec::len),
                mismatched.unwrap_or_default(),
                back as f64 / (front + back).max(1) as f64 * 100.0,
            );
            if reference_ranks.is_none() {
                reference_ranks = Some(ranks);
            }
        }
    }
}


/// 段階1: 多数の姿勢で「現行規則の裏」と「重なり順を一切使わない最前面判定の裏」を比べる。
/// 合否は付けない(§10.7.7)。数を出して段階2の判断材料にする。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage1_random_pose_sweep_rule_versus_strict() {
    const SEED: u64 = 0x0123_4567_89AB_CDEF;
    const POSES: usize = 240;
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage1-sweep", cp.clone(), 1.0, 1.0);
    let views = [
        ("default", camera(1.0, 1.0, 1.0)),
        ("back", camera(1.0, 1.0, -1.0)),
        ("az0el0", camera_from_orbit_angles(1.0, 1.0, 0, 0)),
        ("az90el30", camera_from_orbit_angles(1.0, 1.0, 90, 30)),
        ("az200el-40", camera_from_orbit_angles(1.0, 1.0, 200, -40)),
        ("az285el55", camera_from_orbit_angles(1.0, 1.0, 285, 55)),
    ];
    let faceid_order = {
        let mut order = faces.iter().map(|face| face.id).collect::<Vec<_>>();
        order.sort_unstable();
        order
    };
    let poses = sweep_poses(&cp, &faces, SEED, POSES);
    let mut worse_poses = 0_usize;
    let mut clean_poses = 0_usize;
    let mut clean_worse_poses = 0_usize;
    let mut faceid_order_poses = 0_usize;
    let mut current_depth_branch = 0_usize;
    let mut rule_grand_total = 0_u64;
    let mut strict_grand_total = 0_u64;
    for pose in &poses {
        let order = surface_rank_order(&pose.frame);
        let is_faceid_order = order == faceid_order;
        // 裂けが小さく自己交差の無い形だけを「実際に折れる形」として別に数える。
        let is_clean = pose.max_seam_gap < 1e-6 && !pose.self_intersects;
        if is_faceid_order {
            faceid_order_poses += 1;
        }
        if is_clean {
            clean_poses += 1;
        }
        if !pose.needs_canonical_path && !pose.has_exact {
            current_depth_branch += 1;
        }
        let mut worst = (0_i64, "");
        let mut rule_total = 0_u64;
        let mut strict_total = 0_u64;
        for (view_name, view) in &views {
            let (_, rule_back, _) = covered_back_counts(&diagram, &pose.frame, *view);
            let (_, strict_back) = strict_nearest_counts(&diagram, &pose.frame, *view);
            rule_total += rule_back;
            strict_total += strict_back;
            let excess = rule_back as i64 - strict_back as i64;
            if excess > worst.0 {
                worst = (excess, view_name);
            }
        }
        rule_grand_total += rule_total;
        strict_grand_total += strict_total;
        if worst.0 > 0 {
            worse_poses += 1;
            if is_clean {
                clean_worse_poses += 1;
            }
            println!(
                "STAGE1SWEEP worse {} clean={is_clean} excess={} view={} rule_back={rule_total} strict_back={strict_total} faceid_order={is_faceid_order} needs_canonical={} has_exact={} seam={:.3e} intersects={} ranks={:?}",
                pose.label,
                worst.0,
                worst.1,
                pose.needs_canonical_path,
                pose.has_exact,
                pose.max_seam_gap,
                pose.self_intersects,
                order,
            );
        }
    }
    println!(
        "STAGE1SWEEP seed={SEED:#x} poses={POSES} views={} clean_poses={clean_poses} rule_worse_than_strict_poses={worse_poses} clean_rule_worse_than_strict_poses={clean_worse_poses} faceid_order_poses={faceid_order_poses} current_depth_branch_poses={current_depth_branch} rule_back_total={rule_grand_total} strict_back_total={strict_grand_total}",
        views.len(),
    );
}

/// 段階2の測定。240通りの姿勢について
/// (1) 重なり順をどの経路で決めたかの件数
/// (2) 「上に来るべき面」を独立に求めた結果との食い違いの数
/// を出す。合否は付けない(§10.7.7)。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage2_random_pose_sweep_paths_and_top_faces() {
    const SEED: u64 = 0x0123_4567_89AB_CDEF;
    const POSES: usize = 240;
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage2-sweep", cp.clone(), 1.0, 1.0);
    let poses = sweep_poses(&cp, &faces, SEED, POSES);
    let mut sources = BTreeMap::<&'static str, usize>::new();
    let mut total = TopFaceAudit::default();
    let mut poses_with_mismatch = 0_usize;
    let mut poses_with_overlap = 0_usize;
    for pose in &poses {
        *sources.entry(pose.source_label).or_default() += 1;
        let mut lines = Vec::new();
        let audit = audit_top_faces(
            &cp,
            &faces,
            &diagram,
            &pose.frame,
            &pose.angles,
            |overlap, truth_left_above, rank_left_above, ladder| {
                // 梯子の高さの差と、その段の裂けの量を並べる。裂けのほうが大きければ
                // 「離れている」という判定は紙のちぎれに埋もれており、信用できない。
                let evidence = match ladder {
                    Some((height, seam)) => format!(
                        " ladder_height={height:.3e} ladder_seam={seam:.3e} height_over_seam={:.3}",
                        height.abs() / seam.max(f64::MIN_POSITIVE)
                    ),
                    None => " ladder=none".to_string(),
                };
                lines.push(format!(
                    "{}<->{} hinge={:?} truth_left_above={truth_left_above} rank_left_above={rank_left_above}{evidence}",
                    overlap.left, overlap.right, overlap.shared_hinge,
                ));
            },
        );
        total.overlaps += audit.overlaps;
        total.local_decided += audit.local_decided;
        total.ladder_decided += audit.ladder_decided;
        total.truth_disagreements += audit.truth_disagreements;
        total.rank_mismatches += audit.rank_mismatches;
        total.undecided += audit.undecided;
        if audit.overlaps > 0 {
            poses_with_overlap += 1;
        }
        if audit.rank_mismatches > 0 {
            poses_with_mismatch += 1;
            println!(
                "STAGE2MISMATCH {} source={} overlaps={} mismatches={} pairs=[{}] angles={:?}",
                pose.label,
                pose.source_label,
                audit.overlaps,
                audit.rank_mismatches,
                lines.join(" | "),
                pose.angles
                    .iter()
                    .map(|(&hinge, &angle)| (hinge, angle))
                    .collect::<BTreeMap<_, _>>()
                    .iter()
                    .map(|(hinge, angle)| format!("{hinge}:{angle:.6}"))
                    .collect::<Vec<_>>(),
            );
        }
    }
    for (label, count) in &sources {
        println!("STAGE2SOURCE {label}={count}");
    }
    println!(
        "STAGE2 seed={SEED:#x} poses={POSES} poses_with_overlap={poses_with_overlap} overlaps={} local_decided={} ladder_decided={} truth_disagreements={} undecided={} rank_mismatches={} poses_with_mismatch={poses_with_mismatch}",
        total.overlaps,
        total.local_decided,
        total.ladder_decided,
        total.truth_disagreements,
        total.undecided,
        total.rank_mismatches,
    );
}


/// 段階2で1件だけ残った食い違い(pose163 の面7と面13)を、
/// 独立判定と製品側の経路の両方から詳しく見る。合否は付けない。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn stage2_remaining_mismatch_detail() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("stage2-detail", cp.clone(), 1.0, 1.0);
    let poses = sweep_poses(&cp, &faces, 0x0123_4567_89AB_CDEF, 240);
    let wanted = std::env::var("ORI3_STAGE2_POSE").unwrap_or_else(|_| "pose163".to_string());
    let pose = poses
        .into_iter()
        .find(|pose| pose.label.starts_with(&wanted))
        .expect("the requested pose is in the fixed sweep");
    println!("DETAIL pose={} angles={:?}", pose.label, {
        let mut angles = pose
            .angles
            .iter()
            .map(|(&e, &a)| (e, a))
            .collect::<Vec<_>>();
        angles.sort_by_key(|pair| pair.0);
        angles
    });
    let exact = frame_polygons(&pose.frame);
    let overlaps = coincident_overlaps(&diagram, &pose.frame);
    let ranks = pose
        .frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank))
        .collect::<BTreeMap<_, _>>();
    println!(
        "DETAIL source={} overlaps={}",
        pose.source_label,
        overlaps.len()
    );
    for overlap in &overlaps {
        let local = local_hinge_truth(&cp, &faces, &pose.angles, &exact, overlap);
        println!(
            "DETAIL pair {}<->{} hinge={:?} rank_left={} rank_right={} local_truth={local:?}",
            overlap.left,
            overlap.right,
            overlap.shared_hinge,
            ranks[&overlap.left],
            ranks[&overlap.right],
        );
        // 全体を同じ割合で戻す梯子を、段ごとに裂けの量つきで出す。
        let mut warm = pose.angles.clone();
        for scale in [
            1.0 - 1e-9,
            1.0 - 1e-8,
            1.0 - 1e-7,
            1.0 - 1e-6,
            1.0 - 1e-5,
            1.0 - 1e-4,
            1.0 - 1e-3,
            0.99,
            0.95,
            0.9,
        ] {
            let mut hard = pose
                .angles
                .iter()
                .map(|(&hinge, &angle)| Driver {
                    hinge,
                    target_angle_deg: angle * scale,
                })
                .collect::<Vec<_>>();
            hard.sort_unstable_by_key(|driver| driver.hinge);
            let motion = solve_motion(&cp, &faces, &hard, None, Some(&warm), false);
            warm = motion.result.angles.clone();
            let seam = ori3_rigid::max_seam_gap(&cp, &faces, &motion.result.frame);
            let probe = frame_polygons(&motion.result.frame);
            let difference = probe_height_difference(&exact, &probe, overlap);
            println!(
                "DETAIL   ladder scale={scale:.10} seam={seam:.3e} height_difference={difference:?}",
            );
        }
        // 製品側 canonical_motion_surface_order と同じ「共通の角度で頭打ちにして
        // 各点を解き直す」経路。
        let mut solved_warm: Option<HashMap<EdgeId, f64>> = None;
        for checkpoint in [9.0_f64, 90.0, 171.0, 179.0, 179.5, 179.9, 179.99, 179.999] {
            let mut hard = pose
                .angles
                .iter()
                .map(|(&hinge, &angle)| Driver {
                    hinge,
                    target_angle_deg: angle.signum() * angle.abs().min(checkpoint),
                })
                .collect::<Vec<_>>();
            hard.sort_unstable_by_key(|driver| driver.hinge);
            let motion = solve_motion(&cp, &faces, &hard, None, solved_warm.as_ref(), false);
            solved_warm = Some(motion.result.angles.clone());
            let seam = ori3_rigid::max_seam_gap(&cp, &faces, &motion.result.frame);
            let probe = frame_polygons(&motion.result.frame);
            let difference = probe_height_difference(&exact, &probe, overlap);
            println!(
                "DETAIL   solved-clamp checkpoint={checkpoint} seam={seam:.3e} height_difference={difference:?}",
            );
        }
        // 製品側と同じ「共通の角度で頭打ちにする」経路を、伝播だけで再現する。
        for checkpoint in [90.0_f64, 171.0, 179.0, 179.9, 179.99, 179.999] {
            let clamped = pose
                .angles
                .iter()
                .map(|(&hinge, &angle)| (hinge, angle.signum() * angle.abs().min(checkpoint)))
                .collect::<HashMap<_, _>>();
            let frame = to_frame3d(&cp, &faces, &propagate(&cp, &faces, &clamped));
            let seam = ori3_rigid::max_seam_gap(&cp, &faces, &frame);
            let probe = frame_polygons(&frame);
            let difference = probe_height_difference(&exact, &probe, overlap);
            println!(
                "DETAIL   clamp checkpoint={checkpoint} seam={seam:.3e} height_difference={difference:?}",
            );
        }
    }
}


/// 調査用。梯子の実測で上下が決まった面対が何組あり、そのうち刻印された
/// `surface_rank` と食い違う組が両端点でいくつあるかを数える。合否は付けない。
#[test]
#[ignore = "調査用の測定。合否ではなく数値の出力が目的"]
fn diag_determined_stacks_versus_surface_rank() {
    let diagrams = boundary_diagrams();
    let mut total_pairs = 0_usize;
    let mut total_determined = 0_usize;
    let mut before_bad = 0_usize;
    let mut after_bad = 0_usize;
    for diagram in &diagrams {
        for &(hinge, _) in &diagram.hinges {
            for sign in [1.0_f64, -1.0] {
                let ladder = boundary_ladder(diagram, hinge, sign);
                let determined = determined_stacks(&diagram.cp, &diagram.faces, &ladder);
                let before = &ladder[ladder.len() - 2].1.frame;
                let after = &ladder[ladder.len() - 1].1.frame;
                let pairs = near_overlaps(before, f64::INFINITY).len();
                let before_disagreements = stack_disagreements(before, &determined)
                    .into_iter()
                    .map(|(left, right, _)| (left, right))
                    .collect::<BTreeSet<_>>();
                let after_disagreements = stack_disagreements(after, &determined)
                    .into_iter()
                    .map(|(left, right, _)| (left, right))
                    .collect::<BTreeSet<_>>();
                total_pairs += pairs;
                total_determined += determined.len();
                // 両端点で食い違いが同じ面対は、端点の差ではなく最初から
                // 実測と合っていない面対である。端点の差はその対称差で数える。
                let only_before = before_disagreements
                    .difference(&after_disagreements)
                    .copied()
                    .collect::<Vec<_>>();
                let only_after = after_disagreements
                    .difference(&before_disagreements)
                    .copied()
                    .collect::<Vec<_>>();
                before_bad += only_before.len();
                after_bad += only_after.len();
                if !only_before.is_empty() || !only_after.is_empty() {
                    println!(
                        "DETSTACK diagram={} edge={hinge} sign={sign:+} pairs={pairs} determined={} shared_bad={} only_before={only_before:?} only_after={only_after:?}",
                        diagram.name,
                        determined.len(),
                        before_disagreements
                            .intersection(&after_disagreements)
                            .count(),
                    );
                }
                if diagram.name == "folded-sample.ori3" && (hinge == 306 || hinge == 425) {
                    println!(
                        "DETSTACK_FOCUS edge={hinge} sign={sign:+} determined={} focus={:?}",
                        determined.len(),
                        determined
                            .iter()
                            .filter(|stack| {
                                [(31, 34), (30, 35), (6, 8), (7, 9)]
                                    .contains(&(stack.left, stack.right))
                            })
                            .map(|stack| (
                                stack.left,
                                stack.right,
                                stack.samples,
                                stack.smallest_gap,
                                stack.largest_gap
                            ))
                            .collect::<Vec<_>>(),
                    );
                }
            }
        }
    }
    println!(
        "DETSTACK_TOTAL near_pairs={total_pairs} determined={total_determined} before_disagreements={before_bad} after_disagreements={after_bad}"
    );
}
