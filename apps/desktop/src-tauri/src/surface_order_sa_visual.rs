/// 利用者の画面で紙の裏が 66.88% 見えていた `live-frame` の姿勢を、同じ
/// 36方位角 x 17仰角 = 612方向で測る常設検査。
///
/// この姿勢では16面すべてがほぼ同じ平面に重なるため、どの面が見えるかは
/// `surface_rank` だけが決める。修正前は180°の折り目15本のうち7本が折り目の
/// 向きに反しており、612方向**すべて**で裏が出ていた(実測 裏40.38%)。
///
/// 修正後に残る裏の画素は、612方向 x 800x800 の合計 22,442,018 表画素に対して
/// **2画素だけ**で、いずれも面の輪郭上に孤立した1画素である(上下左右の隣接画素は
/// すべて表)。面11・面10の投影多角形の縁がその1画素にだけ掛かった、CPU rasterの
/// 境界画素であり、まとまった面が裏を向いている状態ではない。
/// 隣接画素まで含めて固定するので、面が1枚でも裏返れば必ず落ちる。
#[test]
fn surface_order_live_frame_has_no_back_pixels_from_all_612_directions() {
    type Exposure = (i32, i32, usize, usize, FaceId);
    // (方位角, 仰角, x, y, 面番号)。CPU rasterの座標原点は左上。
    const EXPECTED_BOUNDARY_PIXELS: [Exposure; 2] =
        [(20, 20, 730, 255, 11), (140, -70, 163, 249, 10)];

    let cp = live_frame_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("live-frame-612", cp.clone(), 1.0, 1.0);
    let frame = live_frame_frame(&cp, &faces);
    assert_eq!(cp.vertices.len(), 13);
    assert_eq!(cp.edges.len(), 28);
    assert_eq!(faces.len(), 16);
    assert_eq!(frame.faces.len(), 16);

    let mut measured_directions = 0_usize;
    let mut total_front_pixels = 0_u64;
    let mut raw_back_pixels = 0_u64;
    let mut raw_directions_with_back = 0_usize;
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
            directions_with_back += 1;
            for (pixel_index, pixel) in image.pixels.iter().enumerate() {
                let Some(owner) = pixel.filter(|pixel| pixel.back_facing) else {
                    continue;
                };
                let x = pixel_index % VIEWPORT;
                let y = pixel_index / VIEWPORT;
                let exposure = (azimuth_deg, elevation_deg, x, y, owner.face);
                // 上下左右の隣接画素に裏が1つでもあれば、輪郭の1画素ではなく
                // 面が裏返って見えている領域である。
                let isolated =
                    [(-1_i32, 0_i32), (1, 0), (0, -1), (0, 1)]
                        .into_iter()
                        .all(|(dx, dy)| {
                            let (Some(nx), Some(ny)) = (
                                x.checked_add_signed(dx as isize),
                                y.checked_add_signed(dy as isize),
                            ) else {
                                return true;
                            };
                            if nx >= VIEWPORT || ny >= VIEWPORT {
                                return true;
                            }
                            image.pixels[ny * VIEWPORT + nx]
                                .is_none_or(|neighbour| !neighbour.back_facing)
                        });
                if isolated {
                    geometric_exposures.insert(exposure);
                } else {
                    unexpected_back_pixels.insert(exposure);
                }
            }
        }
    }

    assert_eq!(measured_directions, 36 * 17);
    println!(
        "LIVE612_SUMMARY directions={measured_directions} total_front_pixels={total_front_pixels} raw_directions_with_back={raw_directions_with_back} raw_back_pixels={raw_back_pixels} isolated_boundary_pixels={} directions_with_back={directions_with_back} clustered_back_pixels={} boundary={geometric_exposures:?} clustered={unexpected_back_pixels:?}",
        geometric_exposures.len(),
        unexpected_back_pixels.len(),
    );
    assert!(total_front_pixels > 0, "紙が1画素も描かれていない");
    assert!(
        unexpected_back_pixels.is_empty(),
        "隣接画素まで裏になっている。面がまとまって裏返っている: {unexpected_back_pixels:?}"
    );
    assert_eq!(raw_back_pixels as usize, geometric_exposures.len());
    assert_eq!(
        geometric_exposures,
        BTreeSet::from(EXPECTED_BOUNDARY_PIXELS),
        "輪郭の裏画素が変わった"
    );
    // 修正前は612方向すべてで裏が出ていた。裏の割合の上限を実測値で固定する。
    assert!(
        raw_back_pixels * 1_000_000 < total_front_pixels,
        "裏 {raw_back_pixels} / 表 {total_front_pixels}"
    );
}

/// 上の612方向検査で裏になった2画素を、その画素を覆う全triangleと一緒に並べる。

#[test]
fn surface_order_user_pose_az320_el20_reports_owner_candidates() {
    #[derive(Clone, Copy)]
    struct Cover {
        draw_order: usize,
        triangle: usize,
        depth: f32,
        depth_code: u32,
        back_facing: bool,
    }

    let cp = zero_back_user_cp();
    let faces = extract_faces(&cp);
    let diagram = diagram("user-pose-owner-diagnosis", cp.clone(), 1.0, 1.0);
    let frame = zero_back_user_frame(&cp, &faces);
    println!(
        "ZERO_OWNER_FRAME_LAYERS {:?}",
        frame
            .faces
            .iter()
            .map(|face| (face.face, face.layer, face.surface_rank))
            .collect::<Vec<_>>()
    );
    let view = camera_from_orbit_angles(1.0, 1.0, 320, 20);
    let image = visual_image(&diagram, &frame, VIEWPORT, view);
    let mut rendered = render_faces(&diagram, &frame, VIEWPORT, view);
    rendered.sort_by_key(render_face_owner_key);

    let max_depth_code = (1_u64 << DEPTH_BITS) - 1;
    let back_mask = image
        .pixels
        .iter()
        .map(|pixel| pixel.is_some_and(|pixel| pixel.back_facing))
        .collect::<Vec<_>>();
    let mut covers = (0..VIEWPORT * VIEWPORT)
        .map(|_| Vec::<Cover>::new())
        .collect::<Vec<_>>();
    for (draw_order, face) in rendered.iter().enumerate() {
        for (triangle_index, triangle) in face.triangles.iter().enumerate() {
            rasterize(
                |pixel, depth| {
                    if !back_mask[pixel] {
                        return;
                    }
                    covers[pixel].push(Cover {
                        draw_order,
                        triangle: triangle_index,
                        depth,
                        depth_code: (depth.clamp(0.0, 1.0) * max_depth_code as f32).round() as u32,
                        back_facing: triangle.back_facing,
                    });
                },
                triangle,
                VIEWPORT,
            );
        }
    }

    let mut diagnosed_back_pixels = 0_u64;
    let mut no_front_at_pixel = 0_u64;
    let mut front_exists_but_farther = 0_u64;
    let mut lost_to_eligible_front = 0_u64;
    let mut same_side = 0_u64;
    let mut split_side = 0_u64;
    let mut adjacent_front_covering_current = 0_u64;
    let mut adjacent_front_not_covering_current = 0_u64;
    let mut pixels_with_adjacent_front_covering_current = 0_u64;
    let mut pixels_with_only_adjacent_front_not_covering_current = 0_u64;
    let mut winner_front_pairs = BTreeMap::<(FaceId, FaceId, u32, u32, i64, i64), u64>::new();
    let mut nearest_front_pairs = BTreeMap::<(FaceId, FaceId, u32), u64>::new();
    let mut adjacent_cover_pairs = BTreeMap::<(FaceId, FaceId, u32), u64>::new();

    for (pixel_index, pixel) in image.pixels.iter().enumerate() {
        let Some(owner) = pixel.filter(|pixel| pixel.back_facing) else {
            continue;
        };
        diagnosed_back_pixels += 1;
        let pixel_covers = &covers[pixel_index];
        assert!(
            !pixel_covers.is_empty(),
            "owner pixel must have a raster cover"
        );
        let minimum_depth_code = pixel_covers
            .iter()
            .map(|cover| cover.depth_code)
            .min()
            .expect("a back pixel has at least one cover");
        let minimum_depth = minimum_depth_code as f32 / max_depth_code as f32;
        let tolerance = DEPTH_TIE_CODES as f32 / max_depth_code as f32;
        // productionのvisual_imageと同じく、量子化されたnearestに対して
        // fragmentのraw f32 depthを比較する。丸め後code差だけでは境界がずれる。
        let eligible = |cover: &Cover| cover.depth - minimum_depth <= tolerance;
        let expected_owner = pixel_covers
            .iter()
            .filter(|cover| eligible(cover))
            .max_by_key(|cover| cover.draw_order)
            .expect("the nearest cover is always eligible");
        assert_eq!(rendered[expected_owner.draw_order].face, owner.face);
        assert_eq!(expected_owner.back_facing, owner.back_facing);
        let best_front_any = pixel_covers
            .iter()
            .filter(|cover| !cover.back_facing)
            .max_by_key(|cover| cover.draw_order);
        let nearest_front = pixel_covers
            .iter()
            .filter(|cover| !cover.back_facing)
            .min_by(|left, right| {
                left.depth
                    .total_cmp(&right.depth)
                    .then(right.draw_order.cmp(&left.draw_order))
            });
        let best_front_eligible = pixel_covers
            .iter()
            .filter(|cover| !cover.back_facing && eligible(cover))
            .max_by_key(|cover| cover.draw_order);
        if let Some(front) = nearest_front {
            *nearest_front_pairs
                .entry((
                    owner.face,
                    rendered[front.draw_order].face,
                    front.depth_code.saturating_sub(minimum_depth_code),
                ))
                .or_default() += 1;
        }

        match (best_front_any, best_front_eligible) {
            (None, _) => no_front_at_pixel += 1,
            (Some(_), None) => front_exists_but_farther += 1,
            (Some(_), Some(front)) => {
                lost_to_eligible_front += 1;
                let winner = rendered
                    .iter()
                    .find(|face| face.face == owner.face)
                    .expect("the owner face remains in the rendered list");
                let front_face = &rendered[front.draw_order];
                if winner.side == front_face.side {
                    same_side += 1;
                } else {
                    split_side += 1;
                }
                *winner_front_pairs
                    .entry((
                        winner.face,
                        front_face.face,
                        winner.surface_rank,
                        front_face.surface_rank,
                        winner.side,
                        front_face.side,
                    ))
                    .or_default() += 1;
            }
        }

        let x = pixel_index % VIEWPORT;
        let y = pixel_index / VIEWPORT;
        let mut adjacent = Vec::new();
        let mut has_adjacent_front_covering_current = false;
        let mut has_adjacent_front_not_covering_current = false;
        for (dx, dy) in [(-1_isize, 0_isize), (1, 0), (0, -1), (0, 1)] {
            let nx = x.checked_add_signed(dx);
            let ny = y.checked_add_signed(dy);
            let Some((nx, ny)) = nx
                .zip(ny)
                .filter(|(nx, ny)| *nx < VIEWPORT && *ny < VIEWPORT)
            else {
                continue;
            };
            let neighbor_index = ny * VIEWPORT + nx;
            let neighbor = image.pixels[neighbor_index];
            let covers_current = neighbor.is_some_and(|neighbor| {
                pixel_covers
                    .iter()
                    .any(|cover| rendered[cover.draw_order].face == neighbor.face)
            });
            if let Some(neighbor) = neighbor.filter(|neighbor| !neighbor.back_facing) {
                if covers_current {
                    adjacent_front_covering_current += 1;
                    has_adjacent_front_covering_current = true;
                    if let Some(cover) = pixel_covers
                        .iter()
                        .filter(|cover| rendered[cover.draw_order].face == neighbor.face)
                        .min_by(|left, right| left.depth.total_cmp(&right.depth))
                    {
                        *adjacent_cover_pairs
                            .entry((
                                owner.face,
                                neighbor.face,
                                cover.depth_code.saturating_sub(minimum_depth_code),
                            ))
                            .or_default() += 1;
                    }
                } else {
                    adjacent_front_not_covering_current += 1;
                    has_adjacent_front_not_covering_current = true;
                }
                adjacent.push(serde_json::json!({
                    "dx": dx,
                    "dy": dy,
                    "face": neighbor.face,
                    "back_facing": neighbor.back_facing,
                    "covers_current_pixel": covers_current,
                }));
            }
        }
        if has_adjacent_front_covering_current {
            pixels_with_adjacent_front_covering_current += 1;
        } else if has_adjacent_front_not_covering_current {
            pixels_with_only_adjacent_front_not_covering_current += 1;
        }

        let candidate_json = pixel_covers
            .iter()
            .map(|cover| {
                let face = &rendered[cover.draw_order];
                serde_json::json!({
                    "face": face.face,
                    "surface_rank": face.surface_rank,
                    "side": face.side,
                    "side_times_surface_rank": face.side * i64::from(face.surface_rank),
                    "material_orientation": face.material_orientation,
                    "triangle": cover.triangle,
                    "back_facing": cover.back_facing,
                    "depth_code": cover.depth_code,
                    "minimum_depth_delta_codes": cover.depth_code.saturating_sub(minimum_depth_code),
                    "raw_minimum_depth_delta_codes":
                        (cover.depth - minimum_depth) * max_depth_code as f32,
                    "eligible": eligible(cover),
                    "draw_order": cover.draw_order,
                })
            })
            .collect::<Vec<_>>();
        let front_json = |cover: &Cover| {
            let face = &rendered[cover.draw_order];
            serde_json::json!({
                "face": face.face,
                "surface_rank": face.surface_rank,
                "side": face.side,
                "side_times_surface_rank": face.side * i64::from(face.surface_rank),
                "material_orientation": face.material_orientation,
                "depth_code": cover.depth_code,
                "minimum_depth_delta_codes": cover.depth_code.saturating_sub(minimum_depth_code),
                "raw_minimum_depth_delta_codes":
                    (cover.depth - minimum_depth) * max_depth_code as f32,
                "draw_order": cover.draw_order,
            })
        };
        if std::env::var_os("ZERO_OWNER_VERBOSE").is_some() {
            println!(
                "ZERO_OWNER_PIXEL {}",
                serde_json::to_string(&serde_json::json!({
                    "x": x,
                    "y": y,
                    "winner": {
                        "face": owner.face,
                        "back_facing": owner.back_facing,
                    },
                    "minimum_depth_code": minimum_depth_code,
                    "best_front_any": best_front_any.map(front_json),
                    "best_front_eligible": best_front_eligible.map(front_json),
                    "candidates": candidate_json,
                    "adjacent_front_winners": adjacent,
                }))
                .expect("owner pixel diagnosis serializes")
            );
        }
    }

    assert_eq!(diagnosed_back_pixels, image.light_pixels);
    assert_eq!(
        no_front_at_pixel + front_exists_but_farther + lost_to_eligible_front,
        diagnosed_back_pixels,
    );
    println!(
        "ZERO_OWNER_SUMMARY azimuth_deg=320 elevation_deg=20 viewport={} front_pixels={} back_pixels={} no_front_at_pixel={} front_exists_but_farther={} lost_to_eligible_front={} same_side={} split_side={} adjacent_front_covering_current={} adjacent_front_not_covering_current={} pixels_with_adjacent_front_covering_current={} pixels_with_only_adjacent_front_not_covering_current={} winner_front_pairs={winner_front_pairs:?} nearest_front_pairs={nearest_front_pairs:?} adjacent_cover_pairs={adjacent_cover_pairs:?}",
        VIEWPORT,
        image.red_pixels,
        image.light_pixels,
        no_front_at_pixel,
        front_exists_but_farther,
        lost_to_eligible_front,
        same_side,
        split_side,
        adjacent_front_covering_current,
        adjacent_front_not_covering_current,
        pixels_with_adjacent_front_covering_current,
        pixels_with_only_adjacent_front_not_covering_current,
    );
}

/// 裏画素を「表の覆いがある(所有者判定の失敗)」と「表の覆いが無い(幾何的露出)」へ分ける。

#[test]
fn surface_order_minus_94_four_view_conditions() {
    let cp = angle_surface_cp();
    let faces = extract_faces(&cp);
    assert_eq!(cp.vertices.len(), 14);
    assert_eq!(cp.edges.len(), 29);
    assert_eq!(faces.len(), 16);
    let diagram = diagram("diagonal-midline-user-cp", cp.clone(), 1.0, 1.0);
    let mut valley_front = None;
    for (fold, angle) in [("mountain", 94.0), ("valley", -94.0)] {
        let folded = propagate(&cp, &faces, &angle_surface_angles(angle));
        let frame = to_frame3d(&cp, &faces, &folded);
        for (view_name, sign) in [("front", 1.0), ("back", -1.0)] {
            let image = visual_image(&diagram, &frame, VIEWPORT, camera(1.0, 1.0, sign));
            let (baseline_red, baseline_light) = match (fold, view_name) {
                ("mountain", "front") => (45_015_u64, 80_u64),
                ("mountain", "back") => (50_438, 6_244),
                ("valley", "front") => (45_777, 4_580),
                ("valley", "back") => (56_603, 79),
                _ => unreachable!("the four acceptance conditions are exhaustive"),
            };
            let baseline_ratio = baseline_red as f64 / (baseline_red + baseline_light) as f64;
            println!(
                "SURFACE_94 fold={fold} angle={angle:+.0} view={view_name} red={} light={} red_ratio={:.6}% baseline_red={} baseline_light={} baseline_red_ratio={:.6}% visible={} back_faces={}",
                image.red_pixels,
                image.light_pixels,
                image.red_ratio() * 100.0,
                baseline_red,
                baseline_light,
                baseline_ratio * 100.0,
                ids(&image.visible_faces),
                ids(&image.visible_back_faces),
            );
            assert!(image.red_pixels + image.light_pixels > 0);
            assert!(
                image.red_ratio() >= baseline_ratio,
                "{fold} {angle:+.0} degrees from the {view_name} must not regress below the measured baseline: red={} light={} ratio={:.9}% baseline={:.9}%",
                image.red_pixels,
                image.light_pixels,
                image.red_ratio() * 100.0,
                baseline_ratio * 100.0,
            );
            if fold == "valley" && view_name == "front" {
                valley_front = Some(image);
            }
        }
    }
    let valley_front = valley_front.expect("the four conditions include valley/front");
    assert!(
        valley_front.red_ratio() >= 0.909,
        "valley -94 degrees from the front must be at least 90.9% red: red={} light={}",
        valley_front.red_pixels,
        valley_front.light_pixels,
    );
}

