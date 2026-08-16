//! 同一深度の紙面を、完全重なりへ入る直前の幾何から下→上へ並べる。

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{DMat3, DVec2, DVec3};
use ori3_cp::Face;
use ori3_model::{EPS, FaceId, Frame3D};

const COPLANAR_EPS: f64 = 1e-8;
const DEPTH_ORDER_EPS: f64 = 1e-12;
const OVERLAP_AREA_EPS: f64 = 1e-14;
pub(crate) const EXACT_FLAT_EPS_RAD: f64 = 1e-8;
/// 平坦な展開図から最終角へ向かう決定的な重なり順経路。
///
/// 各値は全ヒンジに共通の角度checkpointである。各ヒンジは符号を保ったまま
/// `min(|最終角|, checkpoint)` まで動き、部分折りは最終角へ達した後固定される。
/// 終点だけで分離しない面対も、終点に最も近い分離点まで順に戻って物理的な上下を決める。
pub(crate) const SURFACE_PATH_CHECKPOINT_DEG: [f64; 22] = [
    9.0, 19.0, 29.0, 39.0, 49.0, 59.0, 69.0, 79.0, 90.0, 101.0, 111.0, 121.0, 131.0, 141.0, 151.0,
    161.0, 171.0, 179.0, 179.5, 179.9, 179.99, 179.999,
];

type Transforms = HashMap<FaceId, (DMat3, DVec3)>;

/// 重なり順の導出結果と、幾何からは決められなかった箇所の数。
///
/// 呼出し元は `unresolved_overlaps` を見て「幾何が答えを持っていない」ことを
/// 知る。面の番号順で当てずっぽうに埋めるのではなく、平らな状態からの経路など
/// 別の幾何へ切り替えるための判断材料である。
#[derive(Debug, Clone)]
pub(crate) struct SurfaceOrder {
    /// 下→上の面順。
    pub(crate) order: Vec<FaceId>,
    /// 実面積で重なっていて、上下を幾何から決められた面対の数。
    pub(crate) resolved_overlaps: usize,
    /// 実面積で重なっているのに上下を決められなかった面対の数。
    pub(crate) unresolved_overlaps: usize,
    /// 多角形が退化していて比較そのものができなかった面対の数。
    pub(crate) skipped_pairs: usize,
    /// 上下の制約が輪になっていたため落とした制約の数。
    pub(crate) broken_constraints: usize,
    /// 折り目の向きが決める厳密な上下と食い違ったため捨てた、深度由来の制約の数。
    pub(crate) dropped_depth_constraints: usize,
}

/// `path` の実深度を制約、`previous_order` を同値時の順として、下→上を返す。
///
/// `exact_frame` で同一平面かつ実面積が重なる面対だけを比較する。exact上の同一点を
/// それぞれの材質座標へ戻してから経路上の各姿勢へ写すため、祖先面の共通剛体運動は
/// 相殺される。終点に近い点から調べ、まだ同じ深度なら前の点へ戻る。画面履歴や
/// カメラを入力にせず、同じ4入力には常に同じ順を返す。
pub(crate) fn derive_surface_order(
    faces: &[Face],
    path: &[Transforms],
    exact: &Transforms,
    exact_frame: &Frame3D,
    previous_order: &[FaceId],
    exact_constraints: &[(FaceId, FaceId)],
) -> Result<SurfaceOrder, String> {
    validate_order(faces, exact_frame, previous_order)?;
    if path.is_empty() {
        return Ok(SurfaceOrder {
            order: previous_order.to_vec(),
            resolved_overlaps: 0,
            unresolved_overlaps: 0,
            skipped_pairs: 0,
            broken_constraints: 0,
            dropped_depth_constraints: 0,
        });
    }
    derive_surface_order_with(
        faces,
        exact_frame,
        previous_order,
        exact_constraints,
        true,
        path.len(),
        |sample, face, point, normal| approached_height(face, point, normal, &path[sample], exact),
    )
}

/// Order nearly parallel overlapping faces by their current physical separation.
pub(crate) fn derive_surface_order_from_current_depths(
    faces: &[Face],
    frame: &Frame3D,
    previous_order: &[FaceId],
    exact_constraints: &[(FaceId, FaceId)],
) -> Result<SurfaceOrder, String> {
    validate_order(faces, frame, previous_order)?;
    let frame_faces = frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    derive_surface_order_with(
        faces,
        frame,
        previous_order,
        exact_constraints,
        false,
        1,
        |_sample, face, point, normal| {
            approached_frame_height(face, point, normal, &frame_faces, &frame_faces)
        },
    )
}

/// `Frame3D` で再生した全ヒンジ経路を、完全折りendpointの同じ材質点で比較する。
/// solverの各checkpointに含まれる従属ヒンジも、そのまま上下制約へ参加する。
pub(crate) fn derive_surface_order_from_frame_path(
    faces: &[Face],
    path: &[Frame3D],
    exact_frame: &Frame3D,
    previous_order: &[FaceId],
    exact_constraints: &[(FaceId, FaceId)],
) -> Result<SurfaceOrder, String> {
    validate_order(faces, exact_frame, previous_order)?;
    for frame in path {
        validate_order(faces, frame, previous_order)?;
    }
    let exact_faces = exact_frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    let path_faces = path
        .iter()
        .map(|frame| {
            frame
                .faces
                .iter()
                .map(|face| (face.face, face))
                .collect::<HashMap<_, _>>()
        })
        .collect::<Vec<_>>();
    derive_surface_order_with(
        faces,
        exact_frame,
        previous_order,
        exact_constraints,
        true,
        path.len(),
        |sample, face, point, normal| {
            approached_frame_height(face, point, normal, &path_faces[sample], &exact_faces)
        },
    )
}

fn derive_surface_order_with(
    faces: &[Face],
    exact_frame: &Frame3D,
    previous_order: &[FaceId],
    exact_constraints: &[(FaceId, FaceId)],
    require_coplanar: bool,
    sample_count: usize,
    mut height: impl FnMut(usize, FaceId, DVec3, DVec3) -> Result<f64, String>,
) -> Result<SurfaceOrder, String> {
    let frame_faces = exact_frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    // Each face participates in up to F-1 comparisons. Its normal, local plane,
    // and projection onto that plane do not depend on the other face.
    let geometries = faces
        .iter()
        .map(|face| {
            let polygon = frame_faces[&face.id]
                .polygon
                .iter()
                .copied()
                .map(DVec3::from)
                .collect::<Vec<_>>();
            let plane = face_plane(&polygon);
            let projected = plane.map(|plane| project_polygon(&polygon, plane));
            FaceGeometry {
                id: face.id,
                plane,
                projected,
                normal: polygon_normal(&polygon),
                polygon,
            }
        })
        .collect::<Vec<_>>();
    // 折り目の向きが決める厳密な上下を先に入れる。深度の差は丸めで壊れ得るが、
    // 折り目の向きは壊れない。あとから来る深度由来の制約がこれと食い違ったら、
    // 深度側を捨てる(下の `dropped_depth_constraints`)。
    let known_faces = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    let exact_pairs = exact_constraints
        .iter()
        .copied()
        .filter(|(below, above)| {
            below != above && known_faces.contains(below) && known_faces.contains(above)
        })
        .collect::<BTreeSet<(FaceId, FaceId)>>();
    let mut constraints = exact_pairs.clone();
    let mut dropped_depth_constraints = 0_usize;
    let mut resolved_overlaps = 0_usize;
    let mut unresolved_overlaps = 0_usize;
    let mut skipped_pairs = 0_usize;

    for left_index in 0..faces.len() {
        for right_index in left_index + 1..faces.len() {
            let left_geometry = &geometries[left_index];
            let right_geometry = &geometries[right_index];
            let left = left_geometry.id;
            let right = right_geometry.id;
            let Some(plane) = common_plane(left_geometry, right_geometry, require_coplanar) else {
                continue;
            };
            let left_2d = left_geometry
                .projected
                .as_deref()
                .expect("a face plane always has a projection");
            let right_2d = project_polygon(&right_geometry.polygon, plane);
            // Polygon clipping is only needed when the projected axis intervals
            // can meet. Expanding by EPS keeps boundary-tolerance cases on the
            // exact path while rejecting spatially separated faces cheaply.
            if !projected_bounds_overlap(left_2d, &right_2d) {
                continue;
            }
            // 1対の多角形が退化していても、他の面対から得た上下は捨てない。
            // 以前はここで全体をErrにしており、呼出し元が全16面を面の番号順へ
            // 落としていた。1対の失敗で紙全体の重なり順を失わせない。
            let Ok(witnesses) = overlap_witnesses(left_2d, &right_2d) else {
                skipped_pairs += 1;
                continue;
            };
            if witnesses.is_empty() {
                continue;
            }
            let mut resolved = false;
            let mut sampling_failed = false;
            for sample in (0..sample_count).rev() {
                let mut left_above = false;
                let mut right_above = false;
                for &witness in &witnesses {
                    let point = plane.origin + plane.u * witness.x + plane.v * witness.y;
                    let (Ok(left_height), Ok(right_height)) = (
                        height(sample, left, point, plane.normal),
                        height(sample, right, point, plane.normal),
                    ) else {
                        sampling_failed = true;
                        break;
                    };
                    let difference = left_height - right_height;
                    left_above |= difference > DEPTH_ORDER_EPS;
                    right_above |= difference < -DEPTH_ORDER_EPS;
                }
                if sampling_failed {
                    break;
                }
                // 同じ深度なら、経路上の1つ前の姿勢まで戻る。1つの面対が重なり領域内で
                // 交差する点では面単位rankを決めず、さらに前の非交差姿勢を探す。
                if left_above == right_above {
                    continue;
                }
                let depth_constraint = if left_above {
                    (right, left)
                } else {
                    (left, right)
                };
                if exact_pairs.contains(&(depth_constraint.1, depth_constraint.0)) {
                    // 180°の折り目が決める上下と逆。折り目の向きは丸めでは壊れないが、
                    // 経路上の深度差は丸めや、経路が作れなかった中間形で壊れる。
                    // 折り目側を残し、深度由来のこの1件だけを捨てる。
                    dropped_depth_constraints += 1;
                } else {
                    constraints.insert(depth_constraint);
                }
                resolved = true;
                break;
            }
            if sampling_failed {
                skipped_pairs += 1;
            } else if resolved {
                resolved_overlaps += 1;
            } else {
                // 実面積で重なっているのに上下が決まらなかった。呼出し元へ
                // 「この幾何は答えを持っていない」ことを伝える。
                unresolved_overlaps += 1;
            }
        }
    }

    let (order, broken_constraints) = stable_topological_order(previous_order, &constraints);
    Ok(SurfaceOrder {
        order,
        resolved_overlaps,
        unresolved_overlaps,
        skipped_pairs,
        broken_constraints,
        dropped_depth_constraints,
    })
}

/// 全面を一度ずつ含む下→上順を `surface_rank` へ刻印する。
pub fn stamp_surface_order(frame: &mut Frame3D, order: &[FaceId]) -> Result<(), String> {
    let frame_ids = frame
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<BTreeSet<_>>();
    let order_ids = order.iter().copied().collect::<BTreeSet<_>>();
    if frame.faces.len() != order.len()
        || frame_ids.len() != frame.faces.len()
        || order_ids.len() != order.len()
        || frame_ids != order_ids
    {
        return Err("surface order does not contain every frame face exactly once".to_string());
    }
    let ranks = order
        .iter()
        .enumerate()
        .map(|(rank, &face)| {
            Ok((
                face,
                u32::try_from(rank).map_err(|_| "surface rank exceeds u32".to_string())?,
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    for face in &mut frame.faces {
        face.surface_rank = ranks[&face.face];
    }
    Ok(())
}

fn validate_order(faces: &[Face], frame: &Frame3D, order: &[FaceId]) -> Result<(), String> {
    let face_ids = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    let frame_ids = frame
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<BTreeSet<_>>();
    let order_ids = order.iter().copied().collect::<BTreeSet<_>>();
    if face_ids.len() != faces.len()
        || frame_ids.len() != frame.faces.len()
        || order_ids.len() != order.len()
        || face_ids != frame_ids
        || face_ids != order_ids
    {
        return Err("surface order inputs do not contain the same unique faces".to_string());
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Plane {
    origin: DVec3,
    normal: DVec3,
    u: DVec3,
    v: DVec3,
}

struct FaceGeometry {
    id: FaceId,
    polygon: Vec<DVec3>,
    normal: Option<DVec3>,
    plane: Option<Plane>,
    projected: Option<Vec<DVec2>>,
}

fn face_plane(polygon: &[DVec3]) -> Option<Plane> {
    let origin = *polygon.first()?;
    let raw_normal = polygon_normal(polygon)?;
    let normal = canonical(raw_normal);
    let u = polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(&a, &b)| b - a)
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?
        .normalize();
    let v = normal.cross(u).normalize();
    Some(Plane {
        origin,
        normal,
        u,
        v,
    })
}

fn common_plane(
    left: &FaceGeometry,
    right: &FaceGeometry,
    require_coplanar: bool,
) -> Option<Plane> {
    let plane = left.plane?;
    let right_normal = right.normal?;
    if plane.normal.dot(right_normal).abs() < 1.0 - COPLANAR_EPS
        || (require_coplanar
            && left
                .polygon
                .iter()
                .chain(&right.polygon)
                .any(|point| plane.normal.dot(*point - plane.origin).abs() > COPLANAR_EPS))
    {
        return None;
    }
    Some(plane)
}

fn polygon_normal(points: &[DVec3]) -> Option<DVec3> {
    if points.len() < 3 {
        return None;
    }
    let normal = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(&a, &b)| a.cross(b))
        .sum::<DVec3>();
    (normal.length_squared() > EPS * EPS).then(|| normal.normalize())
}

/// 平面の2つの法線のうち、絶対値が最大の成分を正にした側を返す。`n` と `-n` に
/// 同じ向きを返すので、面の並び順によらず同じ「上」の向きを与える。
pub(crate) fn canonical(mut normal: DVec3) -> DVec3 {
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

fn project_polygon(points: &[DVec3], plane: Plane) -> Vec<DVec2> {
    points
        .iter()
        .map(|point| {
            let relative = *point - plane.origin;
            DVec2::new(relative.dot(plane.u), relative.dot(plane.v))
        })
        .collect()
}

fn projected_bounds_overlap(left: &[DVec2], right: &[DVec2]) -> bool {
    let bounds = |polygon: &[DVec2]| {
        polygon.iter().copied().fold(
            (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
            |(minimum, maximum), point| (minimum.min(point), maximum.max(point)),
        )
    };
    let (left_minimum, left_maximum) = bounds(left);
    let (right_minimum, right_maximum) = bounds(right);
    left_maximum.x + EPS >= right_minimum.x
        && right_maximum.x + EPS >= left_minimum.x
        && left_maximum.y + EPS >= right_minimum.y
        && right_maximum.y + EPS >= left_minimum.y
}

fn approached_height(
    face: FaceId,
    point: DVec3,
    normal: DVec3,
    approached: &Transforms,
    exact: &Transforms,
) -> Result<f64, String> {
    let (exact_rotation, exact_translation) = exact
        .get(&face)
        .ok_or_else(|| format!("exact transform lost face {face}"))?;
    let (approached_rotation, approached_translation) = approached
        .get(&face)
        .ok_or_else(|| format!("approach transform lost face {face}"))?;
    let material = exact_rotation.transpose() * (point - *exact_translation);
    let approached_point = *approached_rotation * material + *approached_translation;
    if !approached_point.is_finite() {
        return Err(format!("face {face} produced a non-finite depth sample"));
    }
    Ok(approached_point.dot(normal))
}

fn approached_frame_height(
    face: FaceId,
    point: DVec3,
    normal: DVec3,
    approached_faces: &HashMap<FaceId, &ori3_model::Face3D>,
    exact_faces: &HashMap<FaceId, &ori3_model::Face3D>,
) -> Result<f64, String> {
    let approached = approached_faces
        .get(&face)
        .ok_or_else(|| format!("approach frame lost face {face}"))?;
    let exact = exact_faces
        .get(&face)
        .ok_or_else(|| format!("exact frame lost face {face}"))?;
    if approached.polygon.len() != exact.polygon.len() || exact.polygon.len() < 3 {
        return Err(format!("face {face} changed its polygon topology"));
    }

    let exact_points = exact
        .polygon
        .iter()
        .copied()
        .map(DVec3::from)
        .collect::<Vec<_>>();
    let approached_points = approached
        .polygon
        .iter()
        .copied()
        .map(DVec3::from)
        .collect::<Vec<_>>();
    let exact_origin = exact_points[0];
    let Some((first, second)) = (1..exact_points.len()).find_map(|first| {
        (first + 1..exact_points.len())
            .find(|&second| {
                (exact_points[first] - exact_origin)
                    .cross(exact_points[second] - exact_origin)
                    .length_squared()
                    > EPS * EPS
            })
            .map(|second| (first, second))
    }) else {
        return Err(format!("face {face} has no non-collinear material basis"));
    };

    let exact_first = exact_points[first] - exact_origin;
    let exact_second = exact_points[second] - exact_origin;
    let relative = point - exact_origin;
    let first_squared = exact_first.length_squared();
    let cross = exact_first.dot(exact_second);
    let second_squared = exact_second.length_squared();
    let determinant = first_squared * second_squared - cross * cross;
    if determinant.abs() <= EPS * EPS {
        return Err(format!("face {face} has a singular material basis"));
    }
    let relative_first = relative.dot(exact_first);
    let relative_second = relative.dot(exact_second);
    let first_weight = (relative_first * second_squared - relative_second * cross) / determinant;
    let second_weight = (relative_second * first_squared - relative_first * cross) / determinant;

    let approached_origin = approached_points[0];
    let approached_point = approached_origin
        + (approached_points[first] - approached_origin) * first_weight
        + (approached_points[second] - approached_origin) * second_weight;
    if !approached_point.is_finite() {
        return Err(format!(
            "face {face} produced a non-finite frame depth sample"
        ));
    }
    Ok(approached_point.dot(normal))
}

fn overlap_witnesses(left: &[DVec2], right: &[DVec2]) -> Result<Vec<DVec2>, String> {
    let mut witnesses = Vec::new();
    for left_triangle in triangulate_polygon(left)? {
        for right_triangle in triangulate_polygon(right)? {
            let intersection = intersect_convex_polygons(&left_triangle, &right_triangle);
            if polygon_area(&intersection).abs() <= OVERLAP_AREA_EPS {
                continue;
            }
            let center = intersection.iter().copied().sum::<DVec2>() / intersection.len() as f64;
            witnesses.push(center);
            witnesses.extend(
                intersection
                    .iter()
                    .copied()
                    .map(|point| (point + center) * 0.5),
            );
        }
    }
    Ok(witnesses)
}

fn triangulate_polygon(boundary: &[DVec2]) -> Result<Vec<Vec<DVec2>>, String> {
    let mut polygon = simple_polygon(boundary);
    if polygon.len() < 3 || polygon_area(&polygon).abs() <= OVERLAP_AREA_EPS {
        return Err("surface order encountered a degenerate face polygon".to_string());
    }
    if polygon_area(&polygon) < 0.0 {
        polygon.reverse();
    }
    let mut triangles = Vec::with_capacity(polygon.len().saturating_sub(2));
    while polygon.len() > 3 {
        let count = polygon.len();
        let Some(ear) = (0..count).find(|&index| {
            let a = polygon[(index + count - 1) % count];
            let b = polygon[index];
            let c = polygon[(index + 1) % count];
            (b - a).perp_dot(c - b) > EPS * EPS
                && !polygon.iter().enumerate().any(|(other, &point)| {
                    other != index
                        && other != (index + count - 1) % count
                        && other != (index + 1) % count
                        && point_in_triangle(point, a, b, c)
                })
        }) else {
            return Err("surface order could not triangulate a face polygon".to_string());
        };
        triangles.push(vec![
            polygon[(ear + count - 1) % count],
            polygon[ear],
            polygon[(ear + 1) % count],
        ]);
        polygon.remove(ear);
    }
    triangles.push(polygon);
    Ok(triangles)
}

fn simple_polygon(boundary: &[DVec2]) -> Vec<DVec2> {
    let mut polygon = Vec::with_capacity(boundary.len());
    for &point in boundary {
        if polygon
            .last()
            .is_none_or(|previous: &DVec2| (*previous - point).length() > EPS)
        {
            polygon.push(point);
        }
    }
    while polygon.len() > 1 && (polygon[0] - polygon[polygon.len() - 1]).length() <= EPS {
        polygon.pop();
    }
    polygon
}

fn point_in_triangle(point: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    (b - a).perp_dot(point - a) >= -EPS
        && (c - b).perp_dot(point - b) >= -EPS
        && (a - c).perp_dot(point - c) >= -EPS
}

fn intersect_convex_polygons(subject: &[DVec2], clip: &[DVec2]) -> Vec<DVec2> {
    let mut output = subject.to_vec();
    for index in 0..clip.len() {
        let clip_start = clip[index];
        let clip_end = clip[(index + 1) % clip.len()];
        let input = std::mem::take(&mut output);
        let Some(mut previous) = input.last().copied() else {
            break;
        };
        let mut previous_side = (clip_end - clip_start).perp_dot(previous - clip_start);
        for current in input {
            let current_side = (clip_end - clip_start).perp_dot(current - clip_start);
            let previous_inside = previous_side >= -EPS;
            let current_inside = current_side >= -EPS;
            if previous_inside != current_inside {
                let denominator = previous_side - current_side;
                if denominator.abs() > EPS * EPS {
                    output.push(previous + (current - previous) * (previous_side / denominator));
                }
            }
            if current_inside {
                output.push(current);
            }
            previous = current;
            previous_side = current_side;
        }
    }
    deduplicate_polygon(output)
}

fn deduplicate_polygon(points: Vec<DVec2>) -> Vec<DVec2> {
    let mut output = Vec::with_capacity(points.len());
    for point in points {
        if output
            .last()
            .is_none_or(|previous: &DVec2| (*previous - point).length() > EPS)
        {
            output.push(point);
        }
    }
    if output.len() > 1 && (output[0] - output[output.len() - 1]).length() <= EPS {
        output.pop();
    }
    output
}

fn polygon_area(polygon: &[DVec2]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }
    0.5 * (0..polygon.len())
        .map(|index| polygon[index].perp_dot(polygon[(index + 1) % polygon.len()]))
        .sum::<f64>()
}

/// 下→上の制約を満たす順を返す。制約が輪になっていても止まらず、落とした制約の
/// 数を返す。輪は「紙がすり抜けている」形でだけ起きるので、そこで全体を面の番号順へ
/// 捨てるより、残りの制約を全て活かした順を返すほうが実際の重なりに近い。
fn stable_topological_order(
    previous_order: &[FaceId],
    constraints: &BTreeSet<(FaceId, FaceId)>,
) -> (Vec<FaceId>, usize) {
    if constraints.is_empty() {
        return (previous_order.to_vec(), 0);
    }
    let mut outgoing = previous_order
        .iter()
        .copied()
        .map(|face| (face, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut indegree = previous_order
        .iter()
        .copied()
        .map(|face| (face, 0usize))
        .collect::<BTreeMap<_, _>>();
    for &(below, above) in constraints {
        let (Some(neighbors), true) = (outgoing.get_mut(&below), indegree.contains_key(&above))
        else {
            // 呼出し元の面集合に無い面を指す制約は作れない(`validate_order` 済み)。
            continue;
        };
        if neighbors.insert(above) {
            *indegree.get_mut(&above).expect("checked above face") += 1;
        }
    }
    let mut emitted = BTreeSet::new();
    let mut order = Vec::with_capacity(previous_order.len());
    let mut broken = 0_usize;
    while order.len() < previous_order.len() {
        let ready = previous_order
            .iter()
            .copied()
            .find(|face| !emitted.contains(face) && indegree[face] == 0);
        let next = match ready {
            Some(face) => face,
            // 輪になった。まだ出していない面のうち、下から押さえる制約が最も少ない
            // 面を出す。同数なら `previous_order` の並びで決めるので結果は決定的。
            None => {
                let Some(face) = previous_order
                    .iter()
                    .copied()
                    .filter(|face| !emitted.contains(face))
                    .min_by_key(|face| indegree[face])
                else {
                    break;
                };
                broken += indegree[&face];
                face
            }
        };
        emitted.insert(next);
        order.push(next);
        for &above in &outgoing[&next] {
            if !emitted.contains(&above) {
                let degree = indegree.get_mut(&above).expect("known above face");
                *degree = degree.saturating_sub(1);
            }
        }
    }
    (order, broken)
}
