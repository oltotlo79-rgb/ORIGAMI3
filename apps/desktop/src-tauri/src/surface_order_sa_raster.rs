use super::*;

pub(super) fn triangulations(
    cp: &CreasePattern,
    faces: &[Face],
) -> BTreeMap<FaceId, Vec<[usize; 3]>> {
    let positions = cp
        .vertices
        .iter()
        .map(|item| (item.id, item.pos))
        .collect::<HashMap<_, _>>();
    faces
        .iter()
        .map(|face| {
            let polygon = face
                .vertices
                .iter()
                .map(|vertex_id| positions[vertex_id])
                .collect::<Vec<_>>();
            (face.id, triangulate_polygon(&polygon))
        })
        .collect()
}

/// Viewer3Dと同じく、反時計回りの単純多角形を耳切り法で三角形へ分ける。
/// スリット等の退化で耳を切れなくなった場合も検査を止めず、残りを扇形に分ける。
pub(super) fn triangulate_polygon(points: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let mut triangles = Vec::with_capacity(points.len().saturating_sub(2));
    if points.len() < 3 {
        return triangles;
    }

    let cross = |a: usize, b: usize, c: usize| {
        (points[b][0] - points[a][0]) * (points[c][1] - points[a][1])
            - (points[b][1] - points[a][1]) * (points[c][0] - points[a][0])
    };
    let orient = |a: usize, b: usize, c: usize| {
        if cross(a, b, c) < 0.0 {
            [a, c, b]
        } else {
            [a, b, c]
        }
    };
    let mut remaining = (0..points.len()).collect::<Vec<_>>();
    while remaining.len() > 3 {
        let count = remaining.len();
        let mut ear = None;
        for index in 0..count {
            let a = remaining[(index + count - 1) % count];
            let b = remaining[index];
            let c = remaining[(index + 1) % count];
            if cross(a, b, c) <= 0.0 {
                continue;
            }
            let blocked = remaining.iter().any(|&point| {
                point != a
                    && point != b
                    && point != c
                    && cross(a, b, point) >= 0.0
                    && cross(b, c, point) >= 0.0
                    && cross(c, a, point) >= 0.0
            });
            if !blocked {
                ear = Some(index);
                break;
            }
        }
        let Some(index) = ear else {
            break;
        };
        let count = remaining.len();
        triangles.push(orient(
            remaining[(index + count - 1) % count],
            remaining[index],
            remaining[(index + 1) % count],
        ));
        remaining.remove(index);
    }
    for index in 1..remaining.len().saturating_sub(1) {
        triangles.push(orient(remaining[0], remaining[index], remaining[index + 1]));
    }
    triangles
}

pub(super) fn camera_from_direction(width: f64, height: f64, direction: V3) -> Camera {
    let center = V3::new(width * 0.5, height * 0.5, 0.0);
    let direction = direction.normalize();
    let tan_half_fov = (0.5 * CAMERA_FOV_DEG.to_radians()).tan();
    let distance = width.max(height) / (2.0 * tan_half_fov) * CAMERA_MARGIN;
    let position = center + direction * distance;
    let forward = (center - position).normalize();
    let right = forward.cross(V3::Y).normalize();
    let up = right.cross(forward).normalize();
    let view_row = |axis: V3| {
        [
            axis.x as f32,
            axis.y as f32,
            axis.z as f32,
            (-axis.dot(position)) as f32,
        ]
    };
    Camera {
        position,
        view_x: view_row(right),
        view_y: view_row(up),
        view_depth: view_row(forward),
        projection_scale: (1.0 / tan_half_fov) as f32,
        projection_depth_a: ((CAMERA_FAR + CAMERA_NEAR) / (CAMERA_FAR - CAMERA_NEAR)) as f32,
        projection_depth_b: (-2.0 * CAMERA_FAR * CAMERA_NEAR / (CAMERA_FAR - CAMERA_NEAR)) as f32,
    }
}

pub(super) fn camera(width: f64, height: f64, sign: f64) -> Camera {
    camera_from_direction(width, height, V3::new(0.35, -0.85, 0.95).normalize() * sign)
}

/// Viewer3Dの横長canvasと同じ水平画角にする。既存のCPU rasterはaspect=1固定なので、
/// view行のx側だけを割り、clipping範囲を画面と同じにして測定する。
pub(super) fn camera_with_aspect(width: f64, height: f64, sign: f64, aspect: f64) -> Camera {
    assert!(aspect.is_finite() && aspect > 0.0);
    let mut result = camera(width, height, sign);
    for value in &mut result.view_x {
        *value /= aspect as f32;
    }
    result
}

/// Viewer3DのY-up軌道カメラと同じ球面方向を、方位角と仰角から作る。
/// 方位角0度は紙の表法線(+Z)、90度は+X、仰角はY方向を正とする。
pub(super) fn camera_from_orbit_angles(
    width: f64,
    height: f64,
    azimuth_deg: i32,
    elevation_deg: i32,
) -> Camera {
    let azimuth = f64::from(azimuth_deg).to_radians();
    let elevation = f64::from(elevation_deg).to_radians();
    let horizontal = elevation.cos();
    camera_from_direction(
        width,
        height,
        V3::new(
            horizontal * azimuth.sin(),
            elevation.sin(),
            horizontal * azimuth.cos(),
        ),
    )
}

pub(super) fn dot4(row: [f32; 4], point: [f32; 4]) -> f32 {
    row[0] * point[0] + row[1] * point[1] + row[2] * point[2] + row[3] * point[3]
}

pub(super) fn project(camera: Camera, point: V3, viewport: usize) -> Option<Projected> {
    let point = [point.x as f32, point.y as f32, point.z as f32, 1.0];
    let view_depth = dot4(camera.view_depth, point);
    if !view_depth.is_finite() || view_depth <= CAMERA_NEAR as f32 {
        return None;
    }
    let ndc_x = dot4(camera.view_x, point) * camera.projection_scale / view_depth;
    let ndc_y = dot4(camera.view_y, point) * camera.projection_scale / view_depth;
    let clip_z = camera.projection_depth_a * view_depth + camera.projection_depth_b;
    let depth = (clip_z / view_depth) * 0.5 + 0.5;
    Some(Projected {
        x: (ndc_x + 1.0) * 0.5 * viewport as f32,
        y: (1.0 - ndc_y) * 0.5 * viewport as f32,
        depth,
    })
}

pub(super) fn canonicalize(normal: &mut V3) -> bool {
    let ax = normal.x.abs();
    let ay = normal.y.abs();
    let az = normal.z.abs();
    let component = if ax >= ay && ax >= az {
        normal.x
    } else if ay >= az {
        normal.y
    } else {
        normal.z
    };
    if component < 0.0 {
        *normal = -*normal;
        true
    } else {
        false
    }
}

pub(super) fn camera_plane_depth(camera: Camera, normal: V3, distance: f64) -> Option<[f32; 3]> {
    let row_vector =
        |row: [f32; 4]| V3::new(f64::from(row[0]), f64::from(row[1]), f64::from(row[2]));
    let view_x = row_vector(camera.view_x);
    let view_y = row_vector(camera.view_y);
    let view_depth = row_vector(camera.view_depth);
    let determinant = view_x.dot(view_y.cross(view_depth));
    if !determinant.is_finite() || determinant.abs() <= SURFACE_OWNER_NORMAL_EPSILON {
        return None;
    }
    // Columns of the inverse view linear transform.  Unlike assuming an
    // orthonormal basis, this also covers camera_with_aspect's scaled x row.
    let view_x_dual = view_y.cross(view_depth) / determinant;
    let view_y_dual = view_depth.cross(view_x) / determinant;
    let view_depth_dual = view_x.cross(view_y) / determinant;
    let projection_scale = f64::from(camera.projection_scale);
    let camera_plane_distance = normal.dot(camera.position) - distance;
    if !projection_scale.is_finite()
        || projection_scale.abs() <= f64::EPSILON
        || camera_plane_distance.abs() <= SURFACE_OWNER_COPLANAR_EPSILON
    {
        return None;
    }

    // CPU equivalent of surfaceOwnerCanonicalDepth in surfaceOwnerShader.ts.
    let depth_scale = -0.5 * f64::from(camera.projection_depth_b) / camera_plane_distance;
    let x = depth_scale * normal.dot(view_x_dual) / projection_scale;
    let y = depth_scale * normal.dot(view_y_dual) / projection_scale;
    let constant = 0.5 * (f64::from(camera.projection_depth_a) + 1.0)
        + depth_scale * normal.dot(view_depth_dual);
    let coefficients = [x as f32, y as f32, constant as f32];
    coefficients
        .iter()
        .all(|value| value.is_finite())
        .then_some(coefficients)
}

pub(super) fn render_face_owner_key(face: &RenderFace) -> (i64, u32, i64, i64, i64) {
    (
        face.side * i64::from(face.surface_rank),
        face.surface_rank,
        face.material_orientation,
        face.side * i64::from(face.face),
        face.side * face.owner_code,
    )
}

pub(super) fn render_faces(
    diagram: &Diagram,
    frame: &Frame3D,
    viewport: usize,
    view: Camera,
) -> Vec<RenderFace> {
    let mut rendered = frame
        .faces
        .iter()
        .map(|face| {
            let polygon = face
                .polygon
                .iter()
                .map(|point| {
                    V3::new(
                        point[0] as f32 as f64,
                        point[1] as f32 as f64,
                        point[2] as f32 as f64,
                    )
                })
                .collect::<Vec<_>>();
            let mut order_normal = V3::ZERO;
            let mut order_center = V3::ZERO;
            let mut order_points = 0_usize;
            for indices in &diagram.triangles[&face.face] {
                let a = polygon[indices[0]];
                let b = polygon[indices[1]];
                let c = polygon[indices[2]];
                order_normal += (b - a).cross(c - a);
                order_center += a + b + c;
                order_points += 3;
            }
            if order_points > 0 {
                order_center /= order_points as f64;
            }
            let normal_valid = order_normal.length_squared()
                > SURFACE_OWNER_NORMAL_EPSILON * SURFACE_OWNER_NORMAL_EPSILON;
            if !normal_valid {
                order_normal = V3::Z;
            } else {
                order_normal = order_normal.normalize();
            }
            // Match updateBatchViewOrder: material orientation uses the raw
            // normalized winding, before canonicalizing the plane normal.
            let material_orientation = if order_normal.z < 0.0 { -1 } else { 1 };
            canonicalize(&mut order_normal);
            let side = if order_normal.dot(view.position - order_center) >= 0.0 {
                1
            } else {
                -1
            };
            let plane_normal = order_normal;
            let plane_distance = plane_normal.dot(order_center);
            let planar = normal_valid
                && polygon.iter().all(|point| {
                    (plane_normal.dot(*point) - plane_distance).abs()
                        <= SURFACE_OWNER_PLANARITY_EPSILON
                });
            let triangles = diagram.triangles[&face.face]
                .iter()
                .filter_map(|indices| {
                    let points = [
                        polygon[indices[0]],
                        polygon[indices[1]],
                        polygon[indices[2]],
                    ];
                    let normal = (points[1] - points[0]).cross(points[2] - points[0]);
                    let center = (points[0] + points[1] + points[2]) / 3.0;
                    Some(RenderTriangle {
                        projected: [
                            project(view, points[0], viewport)?,
                            project(view, points[1], viewport)?,
                            project(view, points[2], viewport)?,
                        ],
                        depth_plane: None,
                        back_facing: normal.dot(view.position - center) < 0.0,
                    })
                })
                .collect();
            RenderFace {
                face: face.face,
                owner_code: diagram.owner_codes[&face.face],
                surface_rank: face.surface_rank,
                side,
                material_orientation,
                plane_normal,
                plane_distance,
                planar,
                coplanar_group: 0,
                points: polygon,
                triangles,
            }
        })
        .collect::<Vec<_>>();

    // Match Viewer3D's common supporting-plane rule. Stable face-id traversal
    // plus a fixed group anchor prevents tolerance chaining from merging
    // distinct physical layers.
    let mut face_indices = (0..rendered.len()).collect::<Vec<_>>();
    face_indices.sort_unstable_by_key(|&index| rendered[index].face);
    let mut strict_groups = Vec::<Vec<usize>>::new();
    for face_index in face_indices {
        if !rendered[face_index].planar {
            continue;
        }
        let matching_group = strict_groups.iter_mut().find(|members| {
            let anchor = &rendered[members[0]];
            let candidate = &rendered[face_index];
            let alignment = if anchor.plane_normal.dot(candidate.plane_normal) < 0.0 {
                -1.0
            } else {
                1.0
            };
            let candidate_normal = candidate.plane_normal * alignment;
            let candidate_distance = candidate.plane_distance * alignment;
            (candidate_normal - anchor.plane_normal)
                .length_squared()
                .sqrt()
                <= SURFACE_OWNER_NORMAL_EPSILON
                && (candidate_distance - anchor.plane_distance).abs()
                    <= SURFACE_OWNER_COPLANAR_EPSILON
                && candidate.points.iter().all(|point| {
                    (anchor.plane_normal.dot(*point) - anchor.plane_distance).abs()
                        <= SURFACE_OWNER_COPLANAR_EPSILON
                })
        });
        if let Some(members) = matching_group {
            members.push(face_index);
        } else {
            strict_groups.push(vec![face_index]);
        }
    }

    // Viewer3Dと同じく、支持平面ごとに組符号を配る。共平面の面が補間の丸めで
    // tie窓から外れても、重なり順で所有者を決められるようにするため。
    let mut coplanar_group = 0_u32;
    for members in &strict_groups {
        coplanar_group += 1;
        for &index in members {
            rendered[index].coplanar_group = coplanar_group;
        }
    }

    let maximum_depth_code = ((1_u64 << DEPTH_BITS) - 1) as f32;
    let residual_tolerance = DEPTH_TIE_CODES as f32 / maximum_depth_code;
    for members in strict_groups {
        let triangle_count = members
            .iter()
            .map(|&index| rendered[index].triangles.len())
            .sum::<usize>();
        if triangle_count <= 1 {
            continue;
        }
        let anchor = &rendered[members[0]];
        let Some(depth_plane) =
            camera_plane_depth(view, anchor.plane_normal, anchor.plane_distance)
        else {
            continue;
        };
        let residuals_valid = members.iter().all(|&index| {
            rendered[index].triangles.iter().all(|triangle| {
                triangle.projected.iter().all(|point| {
                    let ndc_x = point.x / viewport as f32 * 2.0 - 1.0;
                    let ndc_y = 1.0 - point.y / viewport as f32 * 2.0;
                    let shared_depth =
                        depth_plane[0] * ndc_x + depth_plane[1] * ndc_y + depth_plane[2];
                    (shared_depth - point.depth).abs() <= residual_tolerance
                })
            })
        });
        if !residuals_valid {
            continue;
        }
        for index in members {
            for triangle in &mut rendered[index].triangles {
                triangle.depth_plane = Some(depth_plane);
            }
        }
    }
    rendered
}

pub(super) fn raster_bounds(
    triangle: &[Projected; 3],
    viewport: usize,
) -> (usize, usize, usize, usize) {
    let min_x = triangle
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as usize;
    let max_x = triangle
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(viewport as f32 - 1.0)
        .max(0.0) as usize;
    let min_y = triangle
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as usize;
    let max_y = triangle
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(viewport as f32 - 1.0)
        .max(0.0) as usize;
    (min_x, max_x, min_y, max_y)
}

pub(super) fn rasterize(
    mut visit: impl FnMut(usize, f32),
    triangle: &RenderTriangle,
    viewport: usize,
) {
    // Preserve the original vertex order for legacy coverage. Only strict
    // coplanar groups replace barycentric depth with their common NDC plane.
    let [a, b, c] = triangle.projected;
    let denominator = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    if denominator.abs() <= RASTER_EPS {
        return;
    }
    let (min_x, max_x, min_y, max_y) = raster_bounds(&triangle.projected, viewport);
    if min_x > max_x || min_y > max_y {
        return;
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let wa = ((b.y - c.y) * (px - c.x) + (c.x - b.x) * (py - c.y)) / denominator;
            let wb = ((c.y - a.y) * (px - c.x) + (a.x - c.x) * (py - c.y)) / denominator;
            let wc = 1.0 - wa - wb;
            if wa >= -RASTER_EPS && wb >= -RASTER_EPS && wc >= -RASTER_EPS {
                let depth =
                    if let Some([x_coefficient, y_coefficient, constant]) = triangle.depth_plane {
                        let ndc_x = px / viewport as f32 * 2.0 - 1.0;
                        let ndc_y = 1.0 - py / viewport as f32 * 2.0;
                        x_coefficient * ndc_x + y_coefficient * ndc_y + constant
                    } else {
                        wa * a.depth + wb * b.depth + wc * c.depth
                    };
                if (0.0..=1.0).contains(&depth) {
                    visit(y * viewport + x, depth);
                }
            }
        }
    }
}

pub(super) fn visual_image(
    diagram: &Diagram,
    frame: &Frame3D,
    viewport: usize,
    view: Camera,
) -> VisualImage {
    let mut faces = render_faces(diagram, frame, viewport, view);
    let max_depth_code = (1_u64 << DEPTH_BITS) - 1;
    // 第1passの描画順もGPUと同じ所有者順にそろえる。同じ最前深度を書いた面のうち
    // 最後に描いた面の組符号が残る、GLのLEQUAL + colorWriteと同じ規則にするため。
    faces.sort_by_key(render_face_owner_key);
    let mut nearest = vec![u32::MAX; viewport * viewport];
    let mut nearest_group = vec![0_u32; viewport * viewport];
    for face in &faces {
        for triangle in &face.triangles {
            rasterize(
                |pixel, depth| {
                    let code = (depth.clamp(0.0, 1.0) * max_depth_code as f32).round() as u32;
                    if code <= nearest[pixel] {
                        nearest[pixel] = code;
                        nearest_group[pixel] = face.coplanar_group;
                    }
                },
                triangle,
                viewport,
            );
        }
    }

    let tolerance = DEPTH_TIE_CODES as f32 / max_depth_code as f32;
    let mut owners = vec![None::<(usize, bool)>; viewport * viewport];
    for (face_index, face) in faces.iter().enumerate() {
        for triangle in &face.triangles {
            rasterize(
                |pixel, depth| {
                    let nearest_code = nearest[pixel];
                    if nearest_code == u32::MAX {
                        return;
                    }
                    // 最前面と同じ支持平面の面は、補間の丸めで深度がずれていても候補に残す。
                    let same_group =
                        face.coplanar_group != 0 && face.coplanar_group == nearest_group[pixel];
                    let nearest_depth = nearest_code as f32 / max_depth_code as f32;
                    if same_group || depth - nearest_depth <= tolerance {
                        owners[pixel] = Some((face_index, triangle.back_facing));
                    }
                },
                triangle,
                viewport,
            );
        }
    }

    let mut visible_faces = BTreeSet::new();
    let mut visible_back_faces = BTreeSet::new();
    let mut red_pixels = 0;
    let mut light_pixels = 0;
    let pixels = owners
        .into_iter()
        .map(|owner| {
            owner.map(|(owner, back_facing)| {
                let face = &faces[owner];
                visible_faces.insert(face.face);
                if back_facing {
                    visible_back_faces.insert(face.face);
                    light_pixels += 1;
                } else {
                    red_pixels += 1;
                }
                VisualPixel {
                    face: face.face,
                    back_facing,
                }
            })
        })
        .collect();
    VisualImage {
        visible_faces,
        visible_back_faces,
        red_pixels,
        light_pixels,
        pixels,
    }
}

/// 利用者指定の相互排他的RGB条件で、fill-only rasterの表・裏画素を数える。
/// 背景・黒線・UI・輪郭AAはこのrasterへ入らない。
pub(super) fn classified_fill_counts(image: &VisualImage) -> (u64, u64) {
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
    (front, back)
}
