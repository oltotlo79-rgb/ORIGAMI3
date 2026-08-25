#[test]
fn surface_order_exact_user_zero_step_pose_reproduction() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    assert_eq!(cp.vertices.len(), 13);
    assert_eq!(cp.edges.len(), 28);
    assert_eq!(faces.len(), 16);
    assert_eq!(zero_back_user_angles().len(), 20);
    let diagram = diagram("exact-user-zero-step-pose", cp.clone(), 1.0, 1.0);
    let frame = zero_back_user_frame(&cp, &faces);
    let default_camera = camera(1.0, 1.0, 1.0);
    let image = visual_image(&diagram, &frame, VIEWPORT, default_camera);

    let mut calculated_faces = render_faces(&diagram, &frame, VIEWPORT, default_camera);
    calculated_faces.sort_by_key(render_face_owner_key);
    let calculation_faces = calculated_faces
        .iter()
        .enumerate()
        .map(|(draw_order, face)| {
            serde_json::json!({
                "face": face.face,
                "draw_order": draw_order,
                "surface_rank": face.surface_rank,
                "side": face.side,
                "material_orientation": face.material_orientation,
                "back_facing": face.triangles.iter().all(|triangle| triangle.back_facing),
                "triangle_back_facing": face
                    .triangles
                    .iter()
                    .map(|triangle| triangle.back_facing)
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let fixture = serde_json::json!({
        "document": {
            "schema_version": 1,
            "paper": { "width_mm": 150.0, "height_mm": 150.0 },
            "cp": cp,
            "sequence": [],
            "display": {
                "front_color": [237, 28, 36],
                "back_color": [255, 255, 255],
                "grid_divisions": 8,
                "soft_enabled": false,
                "soft_stiffness": 0.5,
                "soft_pressure": 0.0,
                "overlap_prevention_enabled": true,
                "penetration_prevention_enabled": true,
            },
        },
        "faces": faces
            .iter()
            .map(|face| serde_json::json!({
                "id": face.id,
                "vertices": face.vertices,
                "edges": face.edges,
            }))
            .collect::<Vec<_>>(),
        "frame": frame,
        "calculation_faces": calculation_faces,
    });
    println!(
        "ZERO_BACK_PIPELINE_FIXTURE {}",
        serde_json::to_string(&fixture).expect("pipeline fixture serializes")
    );

    // 利用者指定の相互排他的RGB判定を、既定色を持つCPU rasterへそのまま適用する。
    // 背景と黒線はfill-only rasterのNoneであり、紙画素の分母へ含めない。
    let mut front = 0_u64;
    let mut back = 0_u64;
    for pixel in image.pixels.iter().flatten() {
        let [r, g, b] = if pixel.back_facing {
            [255_i16, 255_i16, 255_i16]
        } else {
            [237_i16, 28_i16, 36_i16]
        };
        let is_front = r > 140 && r - g > 40 && r - b > 40;
        let is_background = (r - 205).abs() <= 12 && (g - 200).abs() <= 12 && (b - 193).abs() <= 12;
        let is_black = r < 90 && g < 90 && b < 90;
        let is_back = !is_front
            && !is_background
            && !is_black
            && r > 150
            && (r - g).abs() < 30
            && (g - b).abs() < 30;
        front += u64::from(is_front);
        back += u64::from(is_back);
    }
    assert_eq!(front, image.red_pixels);
    assert_eq!(back, image.light_pixels);
    let total = front + back;
    assert!(total > 0);
    let back_ratio = back as f64 / total as f64;
    println!(
        "ZERO_BACK_REPRO steps=0 vertices={} edges={} creases={} front={} back={} back_ratio={:.6}% visible={} back_faces={}",
        cp.vertices.len(),
        cp.edges.len(),
        zero_back_user_angles().len(),
        front,
        back,
        back_ratio * 100.0,
        ids(&image.visible_faces),
        ids(&image.visible_back_faces),
    );

    // bad-full.pngの3D canvasは925×536 CSS px。800角の測定bufferでも水平投影へ
    // 同じaspectを入れれば、色比を保ったまま画面と同じclipping範囲を測れる。
    let screen_aspect = 925.0 / 536.0;
    let screen_image = visual_image(
        &diagram,
        &frame,
        VIEWPORT,
        camera_with_aspect(1.0, 1.0, 1.0, screen_aspect),
    );
    let screen_total = screen_image.red_pixels + screen_image.light_pixels;
    let screen_back_ratio = screen_image.light_pixels as f64 / screen_total as f64;
    println!(
        "ZERO_BACK_SCREEN_ASPECT aspect={screen_aspect:.9} front={} back={} back_ratio={:.6}% visible={} back_faces={}",
        screen_image.red_pixels,
        screen_image.light_pixels,
        screen_back_ratio * 100.0,
        ids(&screen_image.visible_faces),
        ids(&screen_image.visible_back_faces),
    );
}

#[test]
fn surface_order_exact_user_warm_start_paths() {
    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("exact-user-warm-start-paths", cp.clone(), 1.0, 1.0);
    let final_angles = zero_back_user_angles();
    let direct_frame = zero_back_user_frame(&cp, &faces);
    let default_camera = camera(1.0, 1.0, 1.0);
    let screen_camera = camera_with_aspect(1.0, 1.0, 1.0, 925.0 / 536.0);

    let face_pixels = |image: &VisualImage| {
        let mut counts = BTreeMap::<FaceId, [u64; 2]>::new();
        for pixel in image.pixels.iter().flatten() {
            let count = counts.entry(pixel.face).or_default();
            count[usize::from(pixel.back_facing)] += 1;
        }
        counts
    };
    let direct_image = visual_image(&diagram, &direct_frame, VIEWPORT, default_camera);
    let direct_total = direct_image.red_pixels + direct_image.light_pixels;
    println!(
        "WARMSTART_BASELINE {}",
        serde_json::to_string(&serde_json::json!({
            "method": "direct-final-angle-propagation",
            "stages": 1,
            "front": direct_image.red_pixels,
            "back": direct_image.light_pixels,
            "back_ratio_percent": direct_image.light_pixels as f64 / direct_total as f64 * 100.0,
            "face_pixels_front_back": face_pixels(&direct_image),
        }))
        .expect("warm-start baseline serializes")
    );

    let mut measured = 0_usize;
    for path in [
        ZeroBackWarmPath::Active36WithPreferred,
        ZeroBackWarmPath::Edge36Only,
        ZeroBackWarmPath::AllHardControl,
    ] {
        for stages in [1_usize, 10, 50, 200] {
            let result = zero_back_warm_solve(&cp, &faces, path, stages);
            let square = visual_image(&diagram, &result.frame, VIEWPORT, default_camera);
            let screen = visual_image(&diagram, &result.frame, VIEWPORT, screen_camera);
            let square_total = square.red_pixels + square.light_pixels;
            let screen_total = screen.red_pixels + screen.light_pixels;
            assert!(square_total > 0 && screen_total > 0);

            let mut maximum_angle_delta = 0.0_f64;
            let mut angle_comparison = final_angles
                .iter()
                .map(|(&hinge, &target)| {
                    let actual = result.angles.get(&hinge).copied().unwrap_or(0.0);
                    let delta = (actual - target + 180.0).rem_euclid(360.0) - 180.0;
                    maximum_angle_delta = maximum_angle_delta.max(delta.abs());
                    serde_json::json!({
                        "hinge": hinge,
                        "target": target,
                        "actual": actual,
                        "canonical_delta": delta,
                    })
                })
                .collect::<Vec<_>>();
            angle_comparison.sort_by_key(|entry| entry["hinge"].as_u64().unwrap_or_default());

            let square_face_pixels = face_pixels(&square);
            let mut maximum_vertex_delta = 0.0_f64;
            let mut face_comparison = Vec::with_capacity(direct_frame.faces.len());
            for reference in &direct_frame.faces {
                let candidate = result
                    .frame
                    .faces
                    .iter()
                    .find(|face| face.face == reference.face)
                    .expect("warm-start frame contains every reference face");
                assert_eq!(reference.polygon.len(), candidate.polygon.len());
                let face_vertex_delta = reference
                    .polygon
                    .iter()
                    .zip(&candidate.polygon)
                    .map(|(left, right)| {
                        left.iter()
                            .zip(right)
                            .map(|(a, b)| (a - b).powi(2))
                            .sum::<f64>()
                            .sqrt()
                    })
                    .fold(0.0_f64, f64::max);
                maximum_vertex_delta = maximum_vertex_delta.max(face_vertex_delta);
                face_comparison.push(serde_json::json!({
                    "face": reference.face,
                    "direct_surface_rank": reference.surface_rank,
                    "warm_surface_rank": candidate.surface_rank,
                    "direct_mirrored": reference.mirrored,
                    "warm_mirrored": candidate.mirrored,
                    "max_vertex_delta": face_vertex_delta,
                    "front_pixels": square_face_pixels
                        .get(&reference.face)
                        .map_or(0, |counts| counts[0]),
                    "back_pixels": square_face_pixels
                        .get(&reference.face)
                        .map_or(0, |counts| counts[1]),
                }));
            }

            println!(
                "WARMSTART_SUMMARY method={} stages={} square_front={} square_back={} square_back_ratio={:.9}% screen_front={} screen_back={} screen_back_ratio={:.9}% converged={} closure_rms={:.3e} seam={:.3e} intersects={} contact={} max_angle_delta={:.9} max_vertex_delta={:.9}",
                path.label(),
                stages,
                square.red_pixels,
                square.light_pixels,
                square.light_pixels as f64 / square_total as f64 * 100.0,
                screen.red_pixels,
                screen.light_pixels,
                screen.light_pixels as f64 / screen_total as f64 * 100.0,
                result.converged,
                result.closure_rms,
                result.max_seam_gap,
                result.self_intersects,
                result.contact_detected,
                maximum_angle_delta,
                maximum_vertex_delta,
            );
            println!(
                "WARMSTART_CASE {}",
                serde_json::to_string(&serde_json::json!({
                    "method": path.label(),
                    "stages": stages,
                    "square": {
                        "front": square.red_pixels,
                        "back": square.light_pixels,
                        "back_ratio_percent": square.light_pixels as f64 / square_total as f64 * 100.0,
                        "visible_faces": square.visible_faces,
                        "visible_back_faces": square.visible_back_faces,
                    },
                    "screen_aspect": {
                        "front": screen.red_pixels,
                        "back": screen.light_pixels,
                        "back_ratio_percent": screen.light_pixels as f64 / screen_total as f64 * 100.0,
                        "visible_faces": screen.visible_faces,
                        "visible_back_faces": screen.visible_back_faces,
                    },
                    "solve": {
                        "converged": result.converged,
                        "closure_rms": result.closure_rms,
                        "iterations": result.iterations,
                        "contact_detected": result.contact_detected,
                        "self_intersects": result.self_intersects,
                        "max_seam_gap": result.max_seam_gap,
                        "maximum_final_angle_delta_deg": maximum_angle_delta,
                        "maximum_direct_frame_vertex_delta": maximum_vertex_delta,
                    },
                    "angles": angle_comparison,
                    "faces": face_comparison,
                }))
                .expect("warm-start measurement serializes")
            );
            measured += 1;
        }
    }
    assert_eq!(measured, 12, "three paths times four stage counts");
}

/// 利用者が実際に表示する20角度の形を、固定の36方位角 x 17仰角で測る常設検査。
/// 旧検査はedge36だけをwarm solveしたほぼ展開状態を測り、裏53,762,620画素のうち
/// 46,271,965画素に表の覆いが無かったため、612方向で裏0という条件の対象として誤っていた。
/// この検査は `zero_back_user_frame` へ対象を直し、表triangleが同じpixelを覆う裏だけを
/// 所有者判定の失敗とする。表の覆いが無い2画素は幾何的露出として座標ごと固定する。
#[test]
fn surface_order_user_frame_has_only_expected_geometric_exposure_from_all_612_directions() {
    type Exposure = (i32, i32, usize, usize, FaceId);
    // (azimuth, elevation, x, y, owner face)。CPU rasterの座標原点は左上。
    const EXPECTED_GEOMETRIC_EXPOSURES: [Exposure; 2] =
        [(40, 60, 7, 370, 13), (250, -20, 673, 206, 2)];

    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("user-frame-zero-612", cp.clone(), 1.0, 1.0);
    let frame = zero_back_user_frame(&cp, &faces);
    assert_eq!(cp.vertices.len(), 13);
    assert_eq!(cp.edges.len(), 28);
    assert_eq!(faces.len(), 16);
    assert_eq!(frame.faces.len(), 16);

    let mut measured_directions = 0_usize;
    let mut total_front_pixels = 0_u64;
    let mut raw_directions_with_back = 0_usize;
    let mut raw_back_pixels = 0_u64;
    let mut directions_with_geometric_exposure = 0_usize;
    // 合否に使う `directions_with_back` は、表の覆いがあるのに裏ownerとなった方向数。
    let mut directions_with_back = 0_usize;
    let mut geometric_exposures = BTreeSet::<Exposure>::new();
    let mut unexpected_back_pixels = BTreeSet::<Exposure>::new();

    for elevation_deg in (-80_i32..=80).step_by(10) {
        for azimuth_deg in (0_i32..=350).step_by(10) {
            let view = camera_from_orbit_angles(1.0, 1.0, azimuth_deg, elevation_deg);
            let image = visual_image(&diagram, &frame, VIEWPORT, view);
            let (front, back) = classified_fill_counts(&image);
            assert_eq!(front, image.red_pixels);
            assert_eq!(back, image.light_pixels);
            measured_directions += 1;
            total_front_pixels += front;
            if back == 0 {
                continue;
            }

            raw_directions_with_back += 1;
            raw_back_pixels += back;
            let mut front_coverage = vec![false; VIEWPORT * VIEWPORT];
            for face in render_faces(&diagram, &frame, VIEWPORT, view) {
                for triangle in face
                    .triangles
                    .iter()
                    .filter(|triangle| !triangle.back_facing)
                {
                    rasterize(
                        |pixel, _depth| front_coverage[pixel] = true,
                        triangle,
                        VIEWPORT,
                    );
                }
            }

            let mut has_geometric_exposure = false;
            let mut has_unexpected_back = false;
            for (pixel_index, pixel) in image.pixels.iter().enumerate() {
                let Some(owner) = pixel.filter(|pixel| pixel.back_facing) else {
                    continue;
                };
                let exposure = (
                    azimuth_deg,
                    elevation_deg,
                    pixel_index % VIEWPORT,
                    pixel_index / VIEWPORT,
                    owner.face,
                );
                if front_coverage[pixel_index] {
                    unexpected_back_pixels.insert(exposure);
                    has_unexpected_back = true;
                } else {
                    geometric_exposures.insert(exposure);
                    has_geometric_exposure = true;
                }
            }
            directions_with_geometric_exposure += usize::from(has_geometric_exposure);
            directions_with_back += usize::from(has_unexpected_back);
        }
    }

    assert_eq!(measured_directions, 36 * 17);
    println!(
        "ZERO612_SUMMARY directions={} total_front_pixels={} raw_directions_with_back={} raw_back_pixels={} directions_with_geometric_exposure={} geometric_exposure_pixels={} directions_with_back={} unexpected_back_pixels={} geometric_exposures={geometric_exposures:?} unexpected={unexpected_back_pixels:?}",
        measured_directions,
        total_front_pixels,
        raw_directions_with_back,
        raw_back_pixels,
        directions_with_geometric_exposure,
        geometric_exposures.len(),
        directions_with_back,
        unexpected_back_pixels.len(),
    );
    assert_eq!(
        directions_with_back, 0,
        "all covered back pixels must be eliminated: {unexpected_back_pixels:?}"
    );
    assert!(unexpected_back_pixels.is_empty());
    assert_eq!(raw_back_pixels as usize, geometric_exposures.len());
    assert_eq!(geometric_exposures.len(), 2);
    assert_eq!(
        geometric_exposures,
        BTreeSet::from(EXPECTED_GEOMETRIC_EXPOSURES),
        "geometric exposure pixels changed"
    );
}

