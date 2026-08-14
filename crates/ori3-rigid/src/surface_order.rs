//! 同一深度の紙面を、完全重なりへ入る直前の幾何から下→上へ並べる。

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{DMat3, DVec2, DVec3};
use ori3_cp::Face;
use ori3_model::{EPS, FaceId, Frame3D};

const COPLANAR_EPS: f64 = 1e-8;
const DEPTH_ORDER_EPS: f64 = 1e-12;
const OVERLAP_AREA_EPS: f64 = 1e-14;
pub(crate) const EXACT_FLAT_EPS_RAD: f64 = 1e-8;
pub(crate) const SURFACE_APPROACH_DEG: f64 = 0.001;

type Transforms = HashMap<FaceId, (DMat3, DVec3)>;

/// `approached` の実深度を制約、`previous_order` を同値時の順として、下→上を返す。
///
/// `exact_frame` で同一平面かつ実面積が重なる面対だけを比較する。exact上の同一点を
/// それぞれの材質座標へ戻してからapproach姿勢へ写すため、祖先面の共通剛体運動は
/// 相殺される。画面履歴やカメラを入力にせず、同じ4入力には常に同じ順を返す。
pub(crate) fn derive_surface_order(
    faces: &[Face],
    approached: &Transforms,
    exact: &Transforms,
    exact_frame: &Frame3D,
    previous_order: &[FaceId],
) -> Result<Vec<FaceId>, String> {
    validate_order(faces, exact_frame, previous_order)?;
    derive_surface_order_with(
        faces,
        exact_frame,
        previous_order,
        true,
        |face, point, normal| approached_height(face, point, normal, approached, exact),
    )
}

/// Order nearly parallel overlapping faces by their current physical separation.
pub(crate) fn derive_surface_order_from_current_depths(
    faces: &[Face],
    frame: &Frame3D,
    previous_order: &[FaceId],
) -> Result<Vec<FaceId>, String> {
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
        false,
        |face, point, normal| {
            approached_frame_height(face, point, normal, &frame_faces, &frame_faces)
        },
    )
}

fn derive_surface_order_with(
    faces: &[Face],
    exact_frame: &Frame3D,
    previous_order: &[FaceId],
    require_coplanar: bool,
    mut height: impl FnMut(FaceId, DVec3, DVec3) -> Result<f64, String>,
) -> Result<Vec<FaceId>, String> {
    let frame_faces = exact_frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    let mut constraints = BTreeSet::<(FaceId, FaceId)>::new();

    for left_index in 0..faces.len() {
        for right_index in left_index + 1..faces.len() {
            let left = faces[left_index].id;
            let right = faces[right_index].id;
            let left_polygon = frame_faces[&left]
                .polygon
                .iter()
                .copied()
                .map(DVec3::from)
                .collect::<Vec<_>>();
            let right_polygon = frame_faces[&right]
                .polygon
                .iter()
                .copied()
                .map(DVec3::from)
                .collect::<Vec<_>>();
            let Some(plane) = common_plane(&left_polygon, &right_polygon, require_coplanar) else {
                continue;
            };
            let left_2d = project_polygon(&left_polygon, plane);
            let right_2d = project_polygon(&right_polygon, plane);
            let witnesses = overlap_witnesses(&left_2d, &right_2d)?;
            let mut left_above = false;
            let mut right_above = false;
            for witness in witnesses {
                let point = plane.origin + plane.u * witness.x + plane.v * witness.y;
                let left_height = height(left, point, plane.normal)?;
                let right_height = height(right, point, plane.normal)?;
                let difference = left_height - right_height;
                left_above |= difference > DEPTH_ORDER_EPS;
                right_above |= difference < -DEPTH_ORDER_EPS;
            }
            // 1つの面対が重なり領域内で交差する場合、面単位rankでは表現できない。
            // その対だけ従来順を保ち、表現できる上下制約まで失わない。
            if left_above == right_above {
                continue;
            }
            constraints.insert(if left_above {
                (right, left)
            } else {
                (left, right)
            });
        }
    }

    stable_topological_order(previous_order, &constraints)
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

fn common_plane(left: &[DVec3], right: &[DVec3], require_coplanar: bool) -> Option<Plane> {
    let origin = *left.first()?;
    let raw_normal = polygon_normal(left)?;
    let normal = canonical(raw_normal);
    let u = left
        .iter()
        .zip(left.iter().cycle().skip(1))
        .take(left.len())
        .map(|(&a, &b)| b - a)
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?
        .normalize();
    let v = normal.cross(u).normalize();
    let right_normal = polygon_normal(right)?;
    if normal.dot(right_normal).abs() < 1.0 - COPLANAR_EPS
        || (require_coplanar
            && left
                .iter()
                .chain(right)
                .any(|point| normal.dot(*point - origin).abs() > COPLANAR_EPS))
    {
        return None;
    }
    Some(Plane {
        origin,
        normal,
        u,
        v,
    })
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

fn canonical(mut normal: DVec3) -> DVec3 {
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

fn stable_topological_order(
    previous_order: &[FaceId],
    constraints: &BTreeSet<(FaceId, FaceId)>,
) -> Result<Vec<FaceId>, String> {
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
        let Some(neighbors) = outgoing.get_mut(&below) else {
            return Err(format!(
                "surface constraint references missing face {below}"
            ));
        };
        if !indegree.contains_key(&above) {
            return Err(format!(
                "surface constraint references missing face {above}"
            ));
        }
        if neighbors.insert(above) {
            *indegree.get_mut(&above).expect("checked above face") += 1;
        }
    }
    let mut emitted = BTreeSet::new();
    let mut order = Vec::with_capacity(previous_order.len());
    while order.len() < previous_order.len() {
        let next = previous_order
            .iter()
            .copied()
            .find(|face| !emitted.contains(face) && indegree[face] == 0)
            .ok_or_else(|| "surface depth constraints contain a cycle".to_string())?;
        emitted.insert(next);
        order.push(next);
        for &above in &outgoing[&next] {
            *indegree.get_mut(&above).expect("known above face") -= 1;
        }
    }
    Ok(order)
}
