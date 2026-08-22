//! 立体姿勢の紙を、画面で指定した3D平面に沿って折る。
//!
//! 平坦な層順序は使わず、つかんだ面から折り平面の同じ側へ共有辺で到達できる面を
//! 動かす。折り平面と各面の交線は [`crate::pull_back_plane_to_faces`] で展開図へ戻し、
//! 反射後の面法線から全ヒンジの目標角を記録する。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_geometry::point_on_segment;
use ori3_model::{
    CreasePattern, Document, DriverLine, EPS, EdgeId, EdgeKind, Face3D, FaceId, FoldDirection,
    FoldStep, Frame3D, TechniqueKind, VertexId,
};

use crate::fold_through::flat_fold_kind;
use crate::{
    FoldPlane3D, point_in_face, pull_back_plane_to_faces, replay_with_faces, representative_point,
};

/// 立体姿勢からの折り入力。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialFoldInput {
    pub plane: FoldPlane3D,
    pub grab_point: [f64; 3],
    pub grab_face: FaceId,
    pub direction: FoldDirection,
}

/// 立体姿勢から展開図へ戻した折りの結果。
#[derive(Clone, Debug)]
pub struct SpatialFoldResult {
    pub cp: CreasePattern,
    pub faces: Vec<Face>,
    /// 折り目を求められない場合も操作を拒否せず、`None` と警告を返す。
    pub step: Option<FoldStep>,
    pub added_edges: Vec<EdgeId>,
    pub moving_faces: Vec<FaceId>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
struct SideOfPlane {
    origin: DVec3,
    normal: DVec3,
    sign: f64,
}

struct NewFaceSelection<'a> {
    parent_of: &'a HashMap<FaceId, FaceId>,
    selected_parents: &'a HashSet<FaceId>,
    grabbed_parent: FaceId,
    grab_point: DVec3,
    grab_material: Option<[f64; 2]>,
    side: SideOfPlane,
}

struct TargetHingeAngles {
    angles: HashMap<EdgeId, f64>,
    missing: usize,
}

/// 立体姿勢に3D折り平面を適用する。
#[must_use]
pub fn fold_from_plane_3d(
    doc: &Document,
    faces: &[Face],
    up_to: usize,
    input: &SpatialFoldInput,
) -> SpatialFoldResult {
    let mut output = unchanged_result(doc, faces);
    let origin = DVec3::from(input.plane.origin);
    let Some(normal) = normalized(DVec3::from(input.plane.normal)) else {
        output
            .warnings
            .push("折る向きを決められなかったため、紙の形を保ちました".to_string());
        return output;
    };
    if !finite3(origin) || !finite3(DVec3::from(input.grab_point)) {
        output
            .warnings
            .push("つかんだ位置を確かめられなかったため、紙の形を保ちました".to_string());
        return output;
    }

    let current = replay_with_faces(doc, faces, up_to, 1.0);
    let Some(grabbed) = faces.iter().find(|face| face.id == input.grab_face) else {
        output
            .warnings
            .push("つかんだ紙を見つけられなかったため、紙の形を保ちました".to_string());
        return output;
    };
    let Some(grabbed_3d) = frame_face(&current.frame, input.grab_face) else {
        output
            .warnings
            .push("つかんだ紙の位置を確かめられなかったため、紙の形を保ちました".to_string());
        return output;
    };
    let grab_material =
        pull_point_to_face(&doc.cp, grabbed, grabbed_3d, DVec3::from(input.grab_point));
    if grab_material.is_none() {
        output.warnings.push(
            "つかんだ紙の位置を正確に戻せなかったため、最も近い部分を動かしました".to_string(),
        );
    }

    let grab_signed = normal.dot(DVec3::from(input.grab_point) - origin);
    let moving_sign = if grab_signed.abs() > EPS {
        grab_signed.signum()
    } else {
        let farthest = grabbed_3d
            .polygon
            .iter()
            .map(|point| normal.dot(DVec3::from(*point) - origin))
            .max_by(|left, right| left.abs().total_cmp(&right.abs()))
            .unwrap_or(0.0);
        if farthest.abs() <= EPS {
            output
                .warnings
                .push("つかんだ場所が折り目に近いため、紙の形を保ちました".to_string());
            return output;
        }
        output
            .warnings
            .push("つかんだ場所が折り目に近いため、面の広い側を動かしました".to_string());
        farthest.signum()
    };

    let selected = connected_old_faces(
        &doc.cp,
        faces,
        &current.frame,
        grabbed,
        SideOfPlane {
            origin,
            normal,
            sign: moving_sign,
        },
    );
    if selected.is_empty() {
        output
            .warnings
            .push("折る側の紙を決められなかったため、紙の形を保ちました".to_string());
        return output;
    }
    let selected_faces: Vec<_> = faces
        .iter()
        .filter(|face| selected.contains(&face.id))
        .cloned()
        .collect();
    let pullback = pull_back_plane_to_faces(&doc.cp, &selected_faces, &current.frame, input.plane);
    if !pullback.warnings.is_empty() {
        output.warnings.push(
            "一部の紙では折り目の位置を正確に決められなかったため、決められた範囲で折りました"
                .to_string(),
        );
    }
    let segments: Vec<_> = pullback
        .faces
        .into_iter()
        .flat_map(|face| face.segments)
        .collect();
    if segments.is_empty() {
        output
            .warnings
            .push("この位置では折り目を決められなかったため、紙の形を保ちました".to_string());
        return output;
    }

    let provisional_kind = flat_fold_kind(Some(input.direction), grabbed_3d.mirrored);
    let mut work = doc.cp.clone();
    for segment in &segments {
        insert_segment(&mut work, segment[0], segment[1], provisional_kind);
    }
    let mut positions = vertex_positions2(&work);
    let mut promoted = 0_usize;
    for edge in &mut work.edges {
        if edge.kind == EdgeKind::Aux && edge_on_segments(edge, &positions, &segments) {
            edge.kind = provisional_kind;
            promoted += 1;
        }
    }
    if promoted > 0 {
        output.warnings.push(format!(
            "折り目と重なっていた補助線{promoted}本を折り線に変更しました"
        ));
    }

    let new_faces = extract_faces(&work);
    let parent_of = parent_faces(&doc.cp, faces, &work, &new_faces);
    let mut expanded = doc.clone();
    expanded.cp = work.clone();
    let expanded_current = replay_with_faces(&expanded, &new_faces, up_to, 1.0);
    if !expanded_current.converged {
        output
            .warnings
            .push("現在の形を求める計算が安定しなかったため、最も近い形で続けました".to_string());
    }
    let moving = connected_new_faces(
        &work,
        &new_faces,
        &expanded_current.frame,
        NewFaceSelection {
            parent_of: &parent_of,
            selected_parents: &selected,
            grabbed_parent: input.grab_face,
            grab_point: DVec3::from(input.grab_point),
            grab_material,
            side: SideOfPlane {
                origin,
                normal,
                sign: moving_sign,
            },
        },
    );
    if moving.is_empty() {
        output
            .warnings
            .push("折る側の紙を分けられなかったため、紙の形を保ちました".to_string());
        return output;
    }

    let target = reflected_frame(&expanded_current.frame, &moving, origin, normal);
    let predicted_gap = ori3_rigid::max_seam_gap(&work, &new_faces, &target);
    if !predicted_gap.is_finite() || predicted_gap >= EPS {
        push_warning_once(
            &mut output.warnings,
            "一部のつながりを正確に保てない可能性があるため、最も近い形で続けました",
        );
    }

    let target_angles = target_hinge_angles(
        &new_faces,
        &expanded_current.frame,
        &target,
        &expanded_current.hinge_angles,
        &moving,
        input.direction,
    );
    if target_angles.missing > 0 {
        output.warnings.push(format!(
            "一部の折り目（{}本）の角度を正確に決められなかったため、求められた角度で続けました",
            target_angles.missing
        ));
    }
    positions = vertex_positions2(&work);
    let mut drivers = Vec::new();
    for edge in &mut work.edges {
        let Some(&angle) = target_angles.angles.get(&edge.id) else {
            continue;
        };
        if angle.abs() > EPS {
            edge.kind = if angle.is_sign_positive() {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
        }
        let (Some(&a), Some(&b)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
            continue;
        };
        drivers.push(DriverLine {
            a: a.to_array(),
            b: b.to_array(),
            target_angle_deg: angle,
        });
    }
    if drivers.is_empty() {
        output
            .warnings
            .push("折った後の形を決められなかったため、紙の形を保ちました".to_string());
        return output;
    }

    let final_positions = vertex_positions2(&work);
    let mut added_edges: Vec<_> = work
        .edges
        .iter()
        .filter(|edge| {
            edge.kind != EdgeKind::Border && edge_on_segments(edge, &final_positions, &segments)
        })
        .map(|edge| edge.id)
        .collect();
    added_edges.sort_unstable();
    added_edges.dedup();
    let mut moving_faces: Vec<_> = moving.into_iter().collect();
    moving_faces.sort_unstable();

    let step = FoldStep {
        id: 0,
        kind: TechniqueKind::Simple,
        drivers,
        layer_order: None,
        alignment: None,
        finish_soft: None,
        note: String::new(),
    };
    if up_to <= doc.sequence.len() {
        let mut candidate = doc.clone();
        candidate.cp = work.clone();
        candidate.sequence.insert(up_to, step.clone());
        let saved = replay_with_faces(&candidate, &new_faces, up_to + 1, 1.0);
        if !saved.converged {
            push_warning_once(
                &mut output.warnings,
                "保存後の形を最後まで確かめられなかったため、求められた形で続けました",
            );
        }
        let saved_gap = ori3_rigid::max_seam_gap(&work, &new_faces, &saved.frame);
        if !saved_gap.is_finite() || saved_gap >= EPS {
            push_warning_once(
                &mut output.warnings,
                "一部のつながりを正確に保てない可能性があるため、最も近い形で続けました",
            );
        }
    } else {
        push_warning_once(
            &mut output.warnings,
            "保存後の形を最後まで確かめられなかったため、求められた形で続けました",
        );
    }

    output.cp = work;
    output.faces = new_faces;
    output.step = Some(step);
    output.added_edges = added_edges;
    output.moving_faces = moving_faces;
    output
}

fn unchanged_result(doc: &Document, faces: &[Face]) -> SpatialFoldResult {
    SpatialFoldResult {
        cp: doc.cp.clone(),
        faces: faces.to_vec(),
        step: None,
        added_edges: Vec::new(),
        moving_faces: Vec::new(),
        warnings: Vec::new(),
    }
}

fn push_warning_once(warnings: &mut Vec<String>, warning: &str) {
    if !warnings.iter().any(|existing| existing == warning) {
        warnings.push(warning.to_string());
    }
}

fn connected_old_faces(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
    grabbed: &Face,
    side: SideOfPlane,
) -> HashSet<FaceId> {
    if !face_reaches_side(frame_face(frame, grabbed.id), side) {
        return HashSet::new();
    }
    let owners = edge_owners(faces);
    let by_id: HashMap<_, _> = faces.iter().map(|face| (face.id, face)).collect();
    let mut selected = HashSet::from([grabbed.id]);
    let mut queue = VecDeque::from([grabbed.id]);
    while let Some(face_id) = queue.pop_front() {
        let Some(face) = by_id.get(&face_id) else {
            continue;
        };
        for &edge_id in &face.edges {
            if !edge_reaches_side(cp, face, frame_face(frame, face_id), edge_id, side) {
                continue;
            }
            for &neighbor in owners.get(&edge_id).into_iter().flatten() {
                if selected.contains(&neighbor)
                    || !face_reaches_side(frame_face(frame, neighbor), side)
                {
                    continue;
                }
                selected.insert(neighbor);
                queue.push_back(neighbor);
            }
        }
    }
    selected
}

fn connected_new_faces(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
    selection: NewFaceSelection<'_>,
) -> HashSet<FaceId> {
    let candidates: HashSet<_> = faces
        .iter()
        .filter(|face| {
            selection
                .parent_of
                .get(&face.id)
                .is_some_and(|id| selection.selected_parents.contains(id))
        })
        .filter(|face| face_reaches_side(frame_face(frame, face.id), selection.side))
        .map(|face| face.id)
        .collect();
    let mut starts: Vec<_> = candidates
        .iter()
        .filter(|face_id| selection.parent_of.get(face_id) == Some(&selection.grabbed_parent))
        .copied()
        .collect();
    starts.sort_unstable();
    if let Some(grab_material) = selection.grab_material {
        let containing: Vec<_> = starts
            .iter()
            .copied()
            .filter(|face_id| {
                faces
                    .iter()
                    .find(|face| face.id == *face_id)
                    .is_some_and(|face| point_in_face(cp, face, grab_material))
            })
            .collect();
        if !containing.is_empty() {
            starts = containing;
        }
    }
    let Some(start) = starts.into_iter().min_by(|left, right| {
        face_centroid(frame_face(frame, *left))
            .distance_squared(selection.grab_point)
            .total_cmp(
                &face_centroid(frame_face(frame, *right)).distance_squared(selection.grab_point),
            )
            .then_with(|| left.cmp(right))
    }) else {
        return HashSet::new();
    };
    let owners = edge_owners(faces);
    let by_id: HashMap<_, _> = faces.iter().map(|face| (face.id, face)).collect();
    let mut moving = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(face_id) = queue.pop_front() {
        let Some(face) = by_id.get(&face_id) else {
            continue;
        };
        for &edge_id in &face.edges {
            if !edge_reaches_side(
                cp,
                face,
                frame_face(frame, face_id),
                edge_id,
                selection.side,
            ) {
                continue;
            }
            for &neighbor in owners.get(&edge_id).into_iter().flatten() {
                if candidates.contains(&neighbor) && moving.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
    }
    moving
}

fn target_hinge_angles(
    faces: &[Face],
    current: &Frame3D,
    target: &Frame3D,
    current_angles: &HashMap<EdgeId, f64>,
    moving: &HashSet<FaceId>,
    direction: FoldDirection,
) -> TargetHingeAngles {
    let mut occurrences: BTreeMap<EdgeId, Vec<(usize, usize)>> = BTreeMap::new();
    for (face_index, face) in faces.iter().enumerate() {
        for (edge_index, &edge_id) in face.edges.iter().enumerate() {
            occurrences
                .entry(edge_id)
                .or_default()
                .push((face_index, edge_index));
        }
    }
    let mut angles = HashMap::new();
    let mut missing = 0_usize;
    for (edge_id, occurrence) in occurrences {
        if occurrence.len() != 2 {
            continue;
        }
        if occurrence[0].0 == occurrence[1].0 {
            missing += 1;
            continue;
        }
        let (left_index, edge_index) = occurrence[0];
        let right_index = occurrence[1].0;
        let left = &faces[left_index];
        let right = &faces[right_index];
        let (Some(left_target), Some(right_target)) =
            (frame_face(target, left.id), frame_face(target, right.id))
        else {
            missing += 1;
            continue;
        };
        if left_target.polygon.len() != left.vertices.len()
            || right_target.polygon.len() != right.vertices.len()
            || left_target.polygon.is_empty()
        {
            missing += 1;
            continue;
        }
        let a = DVec3::from(left_target.polygon[edge_index]);
        let b = DVec3::from(left_target.polygon[(edge_index + 1) % left.vertices.len()]);
        let Some(axis) = normalized(b - a) else {
            missing += 1;
            continue;
        };
        let (Some(left_normal), Some(right_normal)) =
            (polygon_normal(left_target), polygon_normal(right_target))
        else {
            missing += 1;
            continue;
        };
        let sine = axis.dot(left_normal.cross(right_normal));
        let cosine = left_normal.dot(right_normal).clamp(-1.0, 1.0);
        let mut angle = sine.atan2(cosine).to_degrees();
        if (angle.abs() - 180.0).abs() <= 1e-7 {
            let left_moves = moving.contains(&left.id);
            let right_moves = moving.contains(&right.id);
            angle = if left_moves != right_moves {
                let moving_face = if left_moves { left.id } else { right.id };
                let Some(moving_current) = frame_face(current, moving_face) else {
                    missing += 1;
                    continue;
                };
                let mirrored = moving_current.mirrored;
                match flat_fold_kind(Some(direction), mirrored) {
                    EdgeKind::Mountain => 180.0,
                    _ => -180.0,
                }
            } else {
                let before = current_angles.get(&edge_id).copied().unwrap_or(angle);
                if left_moves { -before } else { before }
            };
        }
        if angle.is_finite() {
            angles.insert(edge_id, angle.clamp(-180.0, 180.0));
        } else {
            missing += 1;
        }
    }
    TargetHingeAngles { angles, missing }
}

fn reflected_frame(
    current: &Frame3D,
    moving: &HashSet<FaceId>,
    origin: DVec3,
    normal: DVec3,
) -> Frame3D {
    let mut target = current.clone();
    for face in &mut target.faces {
        if !moving.contains(&face.face) {
            continue;
        }
        for point in &mut face.polygon {
            let p = DVec3::from(*point);
            *point = (p - 2.0 * normal * normal.dot(p - origin)).to_array();
        }
        face.mirrored = !face.mirrored;
    }
    target
}

fn parent_faces(
    old_cp: &CreasePattern,
    old_faces: &[Face],
    new_cp: &CreasePattern,
    new_faces: &[Face],
) -> HashMap<FaceId, FaceId> {
    new_faces
        .iter()
        .filter_map(|face| {
            let point = representative_point(new_cp, face);
            old_faces
                .iter()
                .find(|parent| point_in_face(old_cp, parent, point))
                .map(|parent| (face.id, parent.id))
        })
        .collect()
}

fn pull_point_to_face(
    cp: &CreasePattern,
    face: &Face,
    face3d: &Face3D,
    point: DVec3,
) -> Option<[f64; 2]> {
    if face.vertices.len() != face3d.polygon.len() || face.vertices.len() < 3 {
        return None;
    }
    let positions = vertex_positions2(cp);
    let points: Vec<_> = face
        .vertices
        .iter()
        .zip(&face3d.polygon)
        .map(|(vertex_id, point3d)| {
            positions
                .get(vertex_id)
                .copied()
                .map(|point2d| (point2d, DVec3::from(*point3d)))
        })
        .collect::<Option<_>>()?;
    let mut pair = (0_usize, 0_usize);
    let mut distance2 = 0.0_f64;
    for first in 0..points.len() {
        for second in (first + 1)..points.len() {
            let candidate = (points[second].0 - points[first].0).length_squared();
            if candidate > distance2 {
                distance2 = candidate;
                pair = (first, second);
            }
        }
    }
    if distance2 <= EPS * EPS {
        return None;
    }
    let base = points[pair.1].0 - points[pair.0].0;
    let mut third = pair.0;
    let mut area = 0.0_f64;
    for candidate in 0..points.len() {
        let candidate_area = base.perp_dot(points[candidate].0 - points[pair.0].0).abs();
        if candidate_area > area {
            area = candidate_area;
            third = candidate;
        }
    }
    if area <= EPS * distance2 {
        return None;
    }
    let p0 = points[pair.0].0;
    let dp1 = points[pair.1].0 - p0;
    let dp2 = points[third].0 - p0;
    let q0 = points[pair.0].1;
    let dq1 = points[pair.1].1 - q0;
    let dq2 = points[third].1 - q0;
    let determinant = dp1.perp_dot(dp2);
    let x_axis = (dq1 * dp2.y - dq2 * dp1.y) / determinant;
    let y_axis = (dq2 * dp1.x - dq1 * dp2.x) / determinant;
    if !finite3(x_axis) || !finite3(y_axis) {
        return None;
    }
    let gram_xx = x_axis.dot(x_axis);
    let gram_xy = x_axis.dot(y_axis);
    let gram_yy = y_axis.dot(y_axis);
    let gram_det = gram_xx * gram_yy - gram_xy * gram_xy;
    if !gram_det.is_finite() || gram_det.abs() <= EPS * EPS {
        return None;
    }
    let delta = point - q0;
    let rhs_x = delta.dot(x_axis);
    let rhs_y = delta.dot(y_axis);
    let material = p0
        + DVec2::new(
            (rhs_x * gram_yy - rhs_y * gram_xy) / gram_det,
            (rhs_y * gram_xx - rhs_x * gram_xy) / gram_det,
        );
    (material.x.is_finite() && material.y.is_finite()).then_some(material.to_array())
}

fn edge_owners(faces: &[Face]) -> BTreeMap<EdgeId, Vec<FaceId>> {
    let mut owners: BTreeMap<EdgeId, Vec<FaceId>> = BTreeMap::new();
    for face in faces {
        for &edge_id in &face.edges {
            owners.entry(edge_id).or_default().push(face.id);
        }
    }
    for values in owners.values_mut() {
        values.sort_unstable();
        values.dedup();
    }
    owners
}

fn edge_reaches_side(
    cp: &CreasePattern,
    face: &Face,
    face3d: Option<&Face3D>,
    edge_id: EdgeId,
    side: SideOfPlane,
) -> bool {
    let Some(face3d) = face3d else {
        return false;
    };
    let Some(edge) = cp.edges.iter().find(|edge| edge.id == edge_id) else {
        return false;
    };
    [edge.v0, edge.v1].into_iter().any(|vertex_id| {
        face.vertices
            .iter()
            .position(|candidate| *candidate == vertex_id)
            .and_then(|index| face3d.polygon.get(index))
            .is_some_and(|point| {
                side.sign * side.normal.dot(DVec3::from(*point) - side.origin) > EPS
            })
    })
}

fn face_reaches_side(face: Option<&Face3D>, side: SideOfPlane) -> bool {
    face.is_some_and(|face| {
        face.polygon
            .iter()
            .any(|point| side.sign * side.normal.dot(DVec3::from(*point) - side.origin) > EPS)
    })
}

fn edge_on_segments(
    edge: &ori3_model::Edge,
    positions: &HashMap<VertexId, DVec2>,
    segments: &[[[f64; 2]; 2]],
) -> bool {
    let (Some(&a), Some(&b)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
        return false;
    };
    segments.iter().any(|segment| {
        let first = DVec2::from(segment[0]);
        let second = DVec2::from(segment[1]);
        point_on_segment(a, first, second) && point_on_segment(b, first, second)
    })
}

fn vertex_positions2(cp: &CreasePattern) -> HashMap<VertexId, DVec2> {
    cp.vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect()
}

fn frame_face(frame: &Frame3D, face: FaceId) -> Option<&Face3D> {
    frame.faces.iter().find(|candidate| candidate.face == face)
}

fn face_centroid(face: Option<&Face3D>) -> DVec3 {
    let Some(face) = face else {
        return DVec3::ZERO;
    };
    if face.polygon.is_empty() {
        return DVec3::ZERO;
    }
    face.polygon
        .iter()
        .map(|point| DVec3::from(*point))
        .sum::<DVec3>()
        / face.polygon.len() as f64
}

fn polygon_normal(face: &Face3D) -> Option<DVec3> {
    if face.polygon.len() < 3 {
        return None;
    }
    let mut normal = DVec3::ZERO;
    for index in 0..face.polygon.len() {
        let a = DVec3::from(face.polygon[index]);
        let b = DVec3::from(face.polygon[(index + 1) % face.polygon.len()]);
        normal += a.cross(b);
    }
    normalized(normal)
}

fn normalized(value: DVec3) -> Option<DVec3> {
    if !finite3(value) {
        return None;
    }
    let scale = value.x.abs().max(value.y.abs()).max(value.z.abs());
    if scale == 0.0 {
        return None;
    }
    let scaled = value / scale;
    let length = scaled.length();
    (length.is_finite() && length > 0.0).then_some(scaled / length)
}

fn finite3(value: DVec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ori3_model::Paper;

    #[test]
    fn inverse_target_angles_match_all_four_flat_mountain_valley_cases() {
        let mut document = Document::new(Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        });
        insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
        let faces = extract_faces(&document.cp);
        let shared_edge = document
            .cp
            .edges
            .iter()
            .find(|edge| edge.kind != EdgeKind::Border)
            .expect("二面を結ぶ折り目")
            .id;
        let base = replay_with_faces(&document, &faces, 0, 1.0);
        let moving_face = faces
            .iter()
            .find(|face| representative_point(&document.cp, face)[0] < 0.5)
            .expect("左側の面")
            .id;
        let moving = HashSet::from([moving_face]);
        let cases = [
            (FoldDirection::Up, false, EdgeKind::Valley, -180.0),
            (FoldDirection::Up, true, EdgeKind::Mountain, 180.0),
            (FoldDirection::Down, false, EdgeKind::Mountain, 180.0),
            (FoldDirection::Down, true, EdgeKind::Valley, -180.0),
        ];
        for (direction, mirrored, expected_kind, expected_angle) in cases {
            let mut current = base.frame.clone();
            frame_face_mut(&mut current, moving_face)
                .expect("動かす面")
                .mirrored = mirrored;
            let target = reflected_frame(&current, &moving, DVec3::new(0.5, 0.0, 0.0), DVec3::X);
            let inverse = target_hinge_angles(
                &faces,
                &current,
                &target,
                &HashMap::new(),
                &moving,
                direction,
            );
            assert_eq!(inverse.missing, 0);
            let angle = inverse.angles[&shared_edge];
            let actual_kind = if angle.is_sign_positive() {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            assert_eq!(actual_kind, expected_kind);
            assert_eq!(angle, expected_angle);
            assert_eq!(actual_kind, flat_fold_kind(Some(direction), mirrored));
        }
    }

    #[test]
    fn ninety_degree_pose_moves_only_the_connected_grab_side_and_replays_joined() {
        let mut document = Document::new(Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        });
        insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
        document.sequence.push(FoldStep {
            id: 0,
            kind: TechniqueKind::Pose,
            drivers: vec![DriverLine {
                a: [0.5, 0.0],
                b: [0.5, 1.0],
                target_angle_deg: 90.0,
            }],
            layer_order: None,
            alignment: None,
            finish_soft: None,
            note: String::new(),
        });
        let faces = extract_faces(&document.cp);
        let before = replay_with_faces(&document, &faces, 1, 1.0);
        let grabbed = before
            .frame
            .faces
            .iter()
            .find(|face| {
                let (low, high) = face
                    .polygon
                    .iter()
                    .map(|point| point[2])
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), z| {
                        (low.min(z), high.max(z))
                    });
                high - low > 0.4
            })
            .expect("90度で立った面");
        let center_x = grabbed.polygon.iter().map(|point| point[0]).sum::<f64>()
            / grabbed.polygon.len() as f64;
        let center_z = grabbed.polygon.iter().map(|point| point[2]).sum::<f64>()
            / grabbed.polygon.len() as f64;
        let moved_vertex = document
            .cp
            .vertices
            .iter()
            .min_by(|left, right| {
                left.pos[1]
                    .total_cmp(&right.pos[1])
                    .then_with(|| left.pos[0].total_cmp(&right.pos[0]))
            })
            .expect("動かす側の既存頂点")
            .id;
        let fixed_vertex = document
            .cp
            .vertices
            .iter()
            .max_by(|left, right| {
                left.pos[1]
                    .total_cmp(&right.pos[1])
                    .then_with(|| left.pos[0].total_cmp(&right.pos[0]))
            })
            .expect("動かさない側の既存頂点")
            .id;
        let before_vertex = frame_vertex_position(&faces, &before.frame, moved_vertex)
            .expect("折る前の既存頂点位置");
        let before_fixed = frame_vertex_position(&faces, &before.frame, fixed_vertex)
            .expect("折る前の固定頂点位置");

        let result = fold_from_plane_3d(
            &document,
            &faces,
            1,
            &SpatialFoldInput {
                plane: FoldPlane3D {
                    origin: [center_x, 0.375, center_z],
                    normal: [0.0, 1.0, 0.0],
                },
                grab_point: [center_x, 0.25, center_z],
                grab_face: grabbed.face,
                direction: FoldDirection::Up,
            },
        );

        assert!(result.step.is_some(), "warnings={:?}", result.warnings);
        assert!(!result.added_edges.is_empty());
        assert_eq!(result.moving_faces.len(), 2);
        let moving: HashSet<_> = result.moving_faces.iter().copied().collect();
        for face in &result.faces {
            let y = representative_point(&result.cp, face)[1];
            assert_eq!(
                moving.contains(&face.id),
                y < 0.375 - EPS,
                "つかんだ側へ共有辺でつながる下半分だけを動かす: face={}, y={y}",
                face.id
            );
        }

        let step = result.step.expect("折り手順");
        assert_eq!(step.drivers.len(), 4, "分割後の4ヒンジ全てを保存する");
        document.cp = result.cp;
        document.sequence.push(step);
        let final_faces = extract_faces(&document.cp);
        let after = replay_with_faces(&document, &final_faces, 2, 1.0);
        let after_vertex = frame_vertex_position(&final_faces, &after.frame, moved_vertex)
            .expect("折った後の既存頂点位置");
        let after_fixed = frame_vertex_position(&final_faces, &after.frame, fixed_vertex)
            .expect("折った後の固定頂点位置");
        assert!(
            after_vertex.distance(before_vertex) > 1e-9,
            "既存頂点が実際に動く"
        );
        assert!(
            (after_vertex.distance(after_fixed) - before_vertex.distance(before_fixed)).abs()
                > 1e-9,
            "全体の置き直しではなく、動く頂点と固定頂点の相対形が変わる"
        );
        let seam = ori3_rigid::max_seam_gap(&document.cp, &final_faces, &after.frame);
        assert!(seam < 1e-9, "共有辺の隙間={seam:.17e}");
    }

    fn frame_face_mut(frame: &mut Frame3D, face: FaceId) -> Option<&mut Face3D> {
        frame
            .faces
            .iter_mut()
            .find(|candidate| candidate.face == face)
    }

    fn frame_vertex_position(faces: &[Face], frame: &Frame3D, vertex: VertexId) -> Option<DVec3> {
        faces.iter().find_map(|face| {
            let index = face
                .vertices
                .iter()
                .position(|candidate| *candidate == vertex)?;
            frame_face(frame, face.id)
                .and_then(|face3d| face3d.polygon.get(index))
                .map(|point| DVec3::from(*point))
        })
    }
}
