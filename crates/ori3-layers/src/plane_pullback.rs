//! 立体姿勢にある各面と3D平面の交線を、展開図座標へ引き戻す。
//!
//! 剛体折りでは、面ごとに展開図座標から3D座標への等長変換が存在する。
//! [`Frame3D`] の面多角形は [`Face::vertices`] と同じ順序なので、対応する
//! 3点からその変換を復元し、3D平面の式を面の展開図座標へ合成できる。
//! 全体が同じ平面に畳まれている必要はない。

use std::collections::{BTreeMap, BTreeSet, HashMap};

use glam::{DVec2, DVec3};
use ori3_cp::Face;
use ori3_model::{CreasePattern, EPS, Face3D, FaceId, Frame3D, VertexId};

use crate::point_in_face;

/// `Frame3D` の面が剛体等長変換から外れていると警告する許容差。
const ISOMETRY_WARNING_EPS: f64 = EPS;
/// f64 の加減算・内積に伴う絶対誤差を安全側に見積もる係数。
const ROUNDING_ERROR_FACTOR: f64 = 32.0;

struct FaceIsometry3 {
    origin2d: DVec2,
    origin3d: DVec3,
    x_axis: DVec3,
    y_axis: DVec3,
    warning: Option<String>,
}

struct FacePullback {
    segments: Vec<[[f64; 2]; 2]>,
    warnings: Vec<String>,
}

/// 無限に広がる3Dの折り平面。
///
/// `origin` は平面上の任意の1点、`normal` は0でない法線。法線は呼び出し側で
/// 正規化していなくてもよく、このモジュール内で正規化してから許容差を適用する。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoldPlane3D {
    pub origin: [f64; 3],
    pub normal: [f64; 3],
}

/// 1面に引き戻された折り目線分。
///
/// 凹面では同じ直線が面内の複数区間を通るため、面ごとに複数線分を持つ。
#[derive(Clone, Debug, PartialEq)]
pub struct FaceCreaseSegments {
    pub face: FaceId,
    pub segments: Vec<[[f64; 2]; 2]>,
}

/// 3D平面を全ての面へ引き戻した結果。
///
/// `faces` は入力された全ての面を同じ順で含む。姿勢を復元できない面や、平面との
/// 交線が一意に定まらない面も削除せず、空の `segments` と理由の警告を返す。
#[derive(Clone, Debug, PartialEq)]
pub struct PlanePullbackResult {
    pub faces: Vec<FaceCreaseSegments>,
    pub warnings: Vec<String>,
}

/// 3Dの折り平面と各剛体面の交線を展開図座標へ引き戻す。
///
/// `frame.faces[*].polygon` と `faces[*].vertices` は、同じ面IDについて同じ頂点順で
/// なければならない（[`ori3_rigid::to_frame3d`] が作るフレームはこの条件を満たす）。
/// 入力の一部が壊れていても関数全体を失敗させず、該当面を空にして警告を返す。
#[must_use]
pub fn pull_back_plane_to_faces(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
    plane: FoldPlane3D,
) -> PlanePullbackResult {
    let mut result = PlanePullbackResult {
        faces: faces
            .iter()
            .map(|face| FaceCreaseSegments {
                face: face.id,
                segments: Vec::new(),
            })
            .collect(),
        warnings: Vec::new(),
    };

    let origin = DVec3::from(plane.origin);
    let raw_normal = DVec3::from(plane.normal);
    let Some(normal) = normalize_plane_normal(raw_normal) else {
        result.warnings.push(
            "3Dの折り平面の法線が0または有限でないため、全ての面の折り目を空にしました".to_string(),
        );
        return result;
    };
    if !finite3(origin) {
        result
            .warnings
            .push("3Dの折り平面の原点が有限でないため、全ての面の折り目を空にしました".to_string());
        return result;
    }

    let vertex_positions: HashMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect();

    for (face, output) in faces.iter().zip(&mut result.faces) {
        let mut matches = frame
            .faces
            .iter()
            .filter(|candidate| candidate.face == face.id);
        let Some(face3d) = matches.next() else {
            result.warnings.push(format!(
                "面 {} の3D姿勢がフレームに無いため、この面の折り目を空にしました",
                face.id
            ));
            continue;
        };
        if matches.next().is_some() {
            result.warnings.push(format!(
                "面 {} の3D姿勢がフレームに複数あるため、この面の折り目を空にしました",
                face.id
            ));
            continue;
        }

        match pull_back_face(cp, face, face3d, &vertex_positions, origin, normal) {
            Ok(face_pullback) => {
                output.segments = face_pullback.segments;
                result.warnings.extend(
                    face_pullback
                        .warnings
                        .into_iter()
                        .map(|warning| format!("面 {}: {warning}", face.id)),
                );
            }
            Err(reason) => result.warnings.push(format!(
                "面 {} の折り目を求められませんでした: {reason}",
                face.id
            )),
        }
    }

    let seam_gap = ori3_rigid::max_seam_gap(cp, faces, frame);
    if seam_gap.is_finite() && seam_gap >= EPS {
        result.warnings.push(format!(
            "3Dフレームの共有辺または共有頂点に {seam_gap:.3e} のずれがあります。共有辺上で対応づけられる折り目端点はそろえて返します"
        ));
    }
    reconcile_shared_edge_endpoints(cp, faces, &mut result);

    result
}

/// Clip one material-space infinite line to a source face.
///
/// The non-flat crease-only path already receives its stable CP line from the
/// visible material surface, so it must not manufacture a live 3D plane merely
/// to reuse the public pullback entry point. Keeping the clipping here makes it
/// share the same concave-face and endpoint rules as 3D plane pullback.
pub(crate) fn clip_material_line_to_face(
    cp: &CreasePattern,
    face: &Face,
    line: [[f64; 2]; 2],
) -> Result<Vec<[[f64; 2]; 2]>, String> {
    let start = DVec2::from(line[0]);
    let end = DVec2::from(line[1]);
    if !finite2(start) || !finite2(end) {
        return Err("material line is not finite".to_string());
    }
    let direction = end - start;
    if direction.length() <= EPS {
        return Err("material line is degenerate".to_string());
    }
    let normal = DVec2::new(-direction.y, direction.x).normalize();
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let mut points = Vec::with_capacity(face.vertices.len());
    for vertex_id in &face.vertices {
        let position = positions
            .get(vertex_id)
            .copied()
            .ok_or_else(|| format!("material vertex {vertex_id} is missing"))?;
        if !finite2(position) {
            return Err(format!("material vertex {vertex_id} is not finite"));
        }
        points.push((position, DVec3::ZERO));
    }
    Ok(clip_line_to_face(
        cp,
        face,
        &points,
        normal.x,
        normal.y,
        -normal.dot(start),
    ))
}

fn pull_back_face(
    cp: &CreasePattern,
    face: &Face,
    face3d: &Face3D,
    vertex_positions: &HashMap<VertexId, DVec2>,
    plane_origin: DVec3,
    plane_normal: DVec3,
) -> Result<FacePullback, String> {
    if face.vertices.len() != face3d.polygon.len() {
        return Err(format!(
            "展開図の頂点数{}と3D多角形の頂点数{}が一致しません",
            face.vertices.len(),
            face3d.polygon.len()
        ));
    }
    if face.vertices.len() < 3 {
        return Err("頂点が3個未満です".to_string());
    }

    let mut points = Vec::with_capacity(face.vertices.len());
    for (&vertex_id, &position3d) in face.vertices.iter().zip(&face3d.polygon) {
        let Some(&position2d) = vertex_positions.get(&vertex_id) else {
            return Err(format!("展開図の頂点 {vertex_id} が見つかりません"));
        };
        let position3d = DVec3::from(position3d);
        if !finite2(position2d) || !finite3(position3d) {
            return Err(format!("頂点 {vertex_id} の座標が有限ではありません"));
        }
        points.push((position2d, position3d));
    }

    let isometry = face_isometry(&points)?;
    let mut warnings = Vec::new();
    if let Some(warning) = isometry.warning {
        warnings.push(warning);
    }
    let translation = isometry.origin3d
        - isometry.x_axis * isometry.origin2d.x
        - isometry.y_axis * isometry.origin2d.y;
    let mut a = plane_normal.dot(isometry.x_axis);
    let mut b = plane_normal.dot(isometry.y_axis);
    let mut c = plane_normal.dot(translation - plane_origin);
    if !a.is_finite() || !b.is_finite() || !c.is_finite() {
        return Err(format!(
            "面へ合成した平面式が有限になりません（a={a:.3e}, b={b:.3e}, c={c:.3e}）"
        ));
    }
    let in_plane_gradient = a.hypot(b);

    if !in_plane_gradient.is_finite() {
        return Err(format!(
            "面へ合成した平面式の面内勾配が有限になりません（面内勾配={in_plane_gradient:.3e}）"
        ));
    }

    if in_plane_gradient <= EPS {
        let distance = points
            .iter()
            .map(|&(_, point3d)| plane_normal.dot(point3d - plane_origin).abs())
            .fold(0.0_f64, f64::max);
        return if distance <= EPS {
            Err(format!(
                "面と3D平面が許容差内で同一平面にあり、交線を1本に決められません（面内勾配={in_plane_gradient:.3e}、最大距離={distance:.3e}）"
            ))
        } else {
            Err(format!(
                "面と3D平面が平行または近平行で交線がありません（面内勾配={in_plane_gradient:.3e}、距離={distance:.3e}）"
            ))
        };
    }

    // 面と平面が浅い角度で交わると、3D 距離の丸め誤差が展開図上で
    // `1 / in_plane_gradient` 倍される。1e-9 の端点精度を保証できない条件では、
    // 誤った線分を返さず、この面だけを空にして数値付きの警告へ送る。
    // 平面内方向へ origin を移しても同じ平面なので、その接線成分を誤差尺度へ
    // 混ぜない。実際の内積に寄与する絶対項和だけから丸め誤差を見積もる。
    let coefficient_scale = abs_dot_sum(plane_normal, isometry.x_axis)
        .max(abs_dot_sum(plane_normal, isometry.y_axis))
        .max(
            (abs_dot_sum(plane_normal, translation) + abs_dot_sum(plane_normal, plane_origin))
                .max(1.0),
        );
    let estimated_cp_error =
        ROUNDING_ERROR_FACTOR * f64::EPSILON * coefficient_scale / in_plane_gradient;
    if !estimated_cp_error.is_finite() || estimated_cp_error >= EPS {
        return Err(format!(
            "面と3D平面が数値的に近平行で、展開図上の端点精度を保証できません（面内勾配={in_plane_gradient:.3e}、推定CP誤差={estimated_cp_error:.3e}）"
        ));
    }

    // 以降のEPSは展開図座標での距離として適用する。未正規化のままだと、3D平面が
    // 面と近平行なほど実効許容差が EPS / in_plane_gradient へ膨らむ。
    a /= in_plane_gradient;
    b /= in_plane_gradient;
    c /= in_plane_gradient;
    if !a.is_finite() || !b.is_finite() || !c.is_finite() {
        return Err(format!(
            "展開図距離へ正規化した平面式が有限になりません（a={a:.3e}, b={b:.3e}, c={c:.3e}）"
        ));
    }

    let segments = clip_line_to_face(cp, face, &points, a, b, c);
    if segments.is_empty()
        && points
            .iter()
            .any(|&(point2d, _)| (a * point2d.x + b * point2d.y + c).abs() <= EPS)
    {
        return Err("3D平面が面の頂点に接するだけで、長さのある交線になりません".to_string());
    }
    Ok(FacePullback { segments, warnings })
}

/// 対応する2D/3D頂点から `T(p) = x_axis*p.x + y_axis*p.y + translation` を復元する。
fn face_isometry(points: &[(DVec2, DVec3)]) -> Result<FaceIsometry3, String> {
    let mut best_pair = (0_usize, 0_usize);
    let mut best_distance2 = 0.0_f64;
    for first in 0..points.len() {
        for second in (first + 1)..points.len() {
            let distance2 = (points[second].0 - points[first].0).length_squared();
            if distance2 > best_distance2 {
                best_distance2 = distance2;
                best_pair = (first, second);
            }
        }
    }
    if best_distance2 <= EPS * EPS {
        return Err(format!(
            "展開図上で独立した2点を選べません（最大距離²={best_distance2:.3e}）"
        ));
    }

    let (first, second) = best_pair;
    let base = points[second].0 - points[first].0;
    let mut third = first;
    let mut best_area2 = 0.0_f64;
    for candidate in 0..points.len() {
        let area2 = base.perp_dot(points[candidate].0 - points[first].0).abs();
        if area2 > best_area2 {
            best_area2 = area2;
            third = candidate;
        }
    }
    if best_area2 <= EPS * best_distance2 {
        return Err(format!(
            "展開図上で独立した3点を選べません（最大面積係数={best_area2:.3e}）"
        ));
    }

    let p0 = points[first].0;
    let dp1 = points[second].0 - p0;
    let dp2 = points[third].0 - p0;
    let q0 = points[first].1;
    let dq1 = points[second].1 - q0;
    let dq2 = points[third].1 - q0;
    let determinant = dp1.perp_dot(dp2);
    let x_axis = (dq1 * dp2.y - dq2 * dp1.y) / determinant;
    let y_axis = (dq2 * dp1.x - dq1 * dp2.x) / determinant;
    if !finite3(x_axis) || !finite3(y_axis) {
        return Err("面の3D等長変換が有限値になりません".to_string());
    }

    let translation = q0 - x_axis * p0.x - y_axis * p0.y;
    let rigid_error = (x_axis.length() - 1.0)
        .abs()
        .max((y_axis.length() - 1.0).abs())
        .max(x_axis.dot(y_axis).abs());
    let mapping_error = points
        .iter()
        .map(|&(point2d, point3d)| {
            (x_axis * point2d.x + y_axis * point2d.y + translation - point3d).length()
        })
        .fold(0.0_f64, f64::max);
    let warning = (rigid_error > ISOMETRY_WARNING_EPS
        || mapping_error > ISOMETRY_WARNING_EPS)
        .then(|| {
            format!(
                "3D多角形が面の剛体等長変換から外れています（等長誤差={rigid_error:.3e}、頂点誤差={mapping_error:.3e}）。復元できた有限な変換で計算を続けます"
            )
        });

    Ok(FaceIsometry3 {
        origin2d: p0,
        origin3d: q0,
        x_axis,
        y_axis,
        warning,
    })
}

fn clip_line_to_face(
    cp: &CreasePattern,
    face: &Face,
    points: &[(DVec2, DVec3)],
    a: f64,
    b: f64,
    c: f64,
) -> Vec<[[f64; 2]; 2]> {
    let line_direction = DVec2::new(-b, a).normalize();
    let signed = |point: DVec2| a * point.x + b * point.y + c;
    let mut intersections = Vec::new();
    for index in 0..points.len() {
        let p0 = points[index].0;
        let p1 = points[(index + 1) % points.len()].0;
        let s0 = signed(p0);
        let s1 = signed(p1);
        if s0.abs() <= EPS && s1.abs() <= EPS {
            intersections.push(p0);
            intersections.push(p1);
        } else if s0.abs() <= EPS {
            intersections.push(p0);
        } else if s1.abs() <= EPS {
            intersections.push(p1);
        } else if s0.is_sign_positive() != s1.is_sign_positive() {
            intersections.push(p0 + (p1 - p0) * (s0 / (s0 - s1)));
        }
    }

    intersections.sort_by(|left, right| {
        left.dot(line_direction)
            .total_cmp(&right.dot(line_direction))
            .then_with(|| left.x.total_cmp(&right.x))
            .then_with(|| left.y.total_cmp(&right.y))
    });
    intersections.dedup_by(|left, right| (*left - *right).length() <= EPS);

    let mut segments = Vec::new();
    for pair in intersections.windows(2) {
        let first = pair[0];
        let second = pair[1];
        if (second - first).length() <= EPS {
            continue;
        }
        let midpoint = (first + second) * 0.5;
        if !point_in_face(cp, face, midpoint.to_array()) {
            continue;
        }
        let (first, second) = canonical_endpoints(first, second);
        segments.push([first.to_array(), second.to_array()]);
    }
    segments.sort_by(compare_segments);
    segments.dedup_by(|left, right| {
        (DVec2::from(left[0]) - DVec2::from(right[0])).length() <= EPS
            && (DVec2::from(left[1]) - DVec2::from(right[1])).length() <= EPS
    });
    segments
}

fn canonical_endpoints(first: DVec2, second: DVec2) -> (DVec2, DVec2) {
    if compare_points(first, second).is_gt() {
        (second, first)
    } else {
        (first, second)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct EndpointKey {
    face_index: usize,
    segment_index: usize,
    endpoint_index: usize,
}

#[derive(Clone, Copy)]
struct EdgeEndpoint {
    key: EndpointKey,
    parameter: f64,
    point: DVec2,
}

#[derive(Clone, Copy)]
struct EndpointConstraint {
    left: EndpointKey,
    right: EndpointKey,
    edge_id: u32,
    canonical_on_edge: DVec2,
}

/// 同じ材質辺を共有する面どうしで、独立計算した端点を1つのCP座標へそろえる。
///
/// 閉包前のbest-effortフレームでは、各面が単独では剛体でも共有辺の3Dコピーが
/// わずかに離れることがある。その場合も展開図へ裂けた2点を残さず、両面の交点
/// パラメータの平均を共通値として返し、元の差を警告する。共有頂点の近くでは
/// 複数辺の対応を先に連結し、端点群を一度だけ共通頂点へそろえる。
fn reconcile_shared_edge_endpoints(
    cp: &CreasePattern,
    faces: &[Face],
    result: &mut PlanePullbackResult,
) {
    let edges: HashMap<_, _> = cp.edges.iter().map(|edge| (edge.id, edge)).collect();
    // EdgeId 順で処理し、頂点で複数の共有辺が接する場合も結果を決定的にする。
    let mut owners: BTreeMap<_, Vec<usize>> = BTreeMap::new();
    for (face_index, face) in faces.iter().enumerate() {
        let mut edge_ids = face.edges.clone();
        edge_ids.sort_unstable();
        edge_ids.dedup();
        for edge_id in edge_ids {
            owners.entry(edge_id).or_default().push(face_index);
        }
    }

    let mut constraints = Vec::new();
    for (edge_id, face_indices) in owners {
        if face_indices.len() != 2 || face_indices[0] == face_indices[1] {
            continue;
        }
        let Some(edge) = edges.get(&edge_id) else {
            continue;
        };
        let (Some(first), Some(second)) = (
            cp.vertices.iter().find(|vertex| vertex.id == edge.v0),
            cp.vertices.iter().find(|vertex| vertex.id == edge.v1),
        ) else {
            continue;
        };
        let edge_start = DVec2::from(first.pos);
        let edge_end = DVec2::from(second.pos);
        if (edge_end - edge_start).length() <= EPS {
            continue;
        }

        let mut endpoints: [Vec<EdgeEndpoint>; 2] = [Vec::new(), Vec::new()];
        for (side, &face_index) in face_indices.iter().enumerate() {
            for (segment_index, segment) in result.faces[face_index].segments.iter().enumerate() {
                for (endpoint_index, &point) in segment.iter().enumerate() {
                    let point = DVec2::from(point);
                    if let Some(parameter) = parameter_on_edge(point, edge_start, edge_end) {
                        endpoints[side].push(EdgeEndpoint {
                            key: EndpointKey {
                                face_index,
                                segment_index,
                                endpoint_index,
                            },
                            parameter,
                            point,
                        });
                    }
                }
            }
            endpoints[side].sort_by(|left, right| left.parameter.total_cmp(&right.parameter));
        }
        if endpoints[0].len() != endpoints[1].len() {
            if !endpoints[0].is_empty() || !endpoints[1].is_empty() {
                result.warnings.push(format!(
                    "共有辺 {edge_id} 上の折り目端点数が両面で一致しません（{} 対 {}）。この辺の端点はそろえられませんでした",
                    endpoints[0].len(),
                    endpoints[1].len()
                ));
            }
            continue;
        }
        if endpoints[0].is_empty() {
            continue;
        }

        for (&left, &right) in endpoints[0].iter().zip(&endpoints[1]) {
            let original_gap = (left.point - right.point).length();
            let parameter = ((left.parameter + right.parameter) * 0.5).clamp(0.0, 1.0);
            constraints.push(EndpointConstraint {
                left: left.key,
                right: right.key,
                edge_id,
                canonical_on_edge: edge_start + (edge_end - edge_start) * parameter,
            });
            if original_gap >= EPS {
                result.warnings.push(format!(
                    "共有辺 {edge_id} 上の面別折り目端点に {original_gap:.3e} の差があったため、同じ展開図座標へそろえました"
                ));
            }
        }
    }

    let mut adjacency: BTreeMap<EndpointKey, BTreeSet<EndpointKey>> = BTreeMap::new();
    for constraint in &constraints {
        adjacency
            .entry(constraint.left)
            .or_default()
            .insert(constraint.right);
        adjacency
            .entry(constraint.right)
            .or_default()
            .insert(constraint.left);
    }
    let mut visited = BTreeSet::new();
    let starts: Vec<_> = adjacency.keys().copied().collect();
    for start in starts {
        if !visited.insert(start) {
            continue;
        }
        let mut component = BTreeSet::from([start]);
        let mut stack = vec![start];
        while let Some(current) = stack.pop() {
            if let Some(neighbors) = adjacency.get(&current) {
                for &neighbor in neighbors {
                    if visited.insert(neighbor) {
                        component.insert(neighbor);
                        stack.push(neighbor);
                    }
                }
            }
        }

        let component_constraints: Vec<_> = constraints
            .iter()
            .filter(|constraint| component.contains(&constraint.left))
            .collect();
        let edge_ids: BTreeSet<_> = component_constraints
            .iter()
            .map(|constraint| constraint.edge_id)
            .collect();
        let canonical = if edge_ids.len() > 1 {
            let mut common_vertices: Option<BTreeSet<VertexId>> = None;
            for edge_id in &edge_ids {
                let Some(edge) = edges.get(edge_id) else {
                    continue;
                };
                let vertices = BTreeSet::from([edge.v0, edge.v1]);
                common_vertices = Some(match common_vertices {
                    None => vertices,
                    Some(current) => current.intersection(&vertices).copied().collect(),
                });
            }
            common_vertices
                .and_then(|vertices| vertices.into_iter().next())
                .and_then(|vertex_id| {
                    cp.vertices
                        .iter()
                        .find(|vertex| vertex.id == vertex_id)
                        .map(|vertex| DVec2::from(vertex.pos))
                })
                .unwrap_or_else(|| {
                    result.warnings.push(format!(
                        "共有辺群 {edge_ids:?} の端点が同じ共有頂点へ対応しないため、近接端点の平均へそろえました"
                    ));
                    average_constraint_points(&component_constraints)
                })
        } else {
            average_constraint_points(&component_constraints)
        };
        let canonical = canonical.to_array();
        for endpoint in component {
            result.faces[endpoint.face_index].segments[endpoint.segment_index]
                [endpoint.endpoint_index] = canonical;
        }
    }

    let mut collapsed_warnings = Vec::new();
    for face in &mut result.faces {
        for segment in &mut face.segments {
            let (first, second) =
                canonical_endpoints(DVec2::from(segment[0]), DVec2::from(segment[1]));
            *segment = [first.to_array(), second.to_array()];
        }
        let before = face.segments.len();
        face.segments
            .retain(|segment| (DVec2::from(segment[1]) - DVec2::from(segment[0])).length() > EPS);
        let removed = before - face.segments.len();
        if removed > 0 {
            collapsed_warnings.push(format!(
                "面 {} の折り目線分 {removed} 本が共有端点の補正後に許容差以下へ縮んだため空として扱いました",
                face.face
            ));
        }
        face.segments.sort_by(compare_segments);
        face.segments.dedup_by(|left, right| {
            (DVec2::from(left[0]) - DVec2::from(right[0])).length() <= EPS
                && (DVec2::from(left[1]) - DVec2::from(right[1])).length() <= EPS
        });
    }
    result.warnings.extend(collapsed_warnings);
}

fn average_constraint_points(constraints: &[&EndpointConstraint]) -> DVec2 {
    let sum = constraints
        .iter()
        .map(|constraint| constraint.canonical_on_edge)
        .fold(DVec2::ZERO, |sum, point| sum + point);
    sum / constraints.len() as f64
}

fn parameter_on_edge(point: DVec2, edge_start: DVec2, edge_end: DVec2) -> Option<f64> {
    let edge = edge_end - edge_start;
    let length_squared = edge.length_squared();
    let parameter = (point - edge_start).dot(edge) / length_squared;
    if !(-EPS..=1.0 + EPS).contains(&parameter) {
        return None;
    }
    let projected = edge_start + edge * parameter.clamp(0.0, 1.0);
    ((point - projected).length() <= EPS).then_some(parameter.clamp(0.0, 1.0))
}

fn compare_segments(left: &[[f64; 2]; 2], right: &[[f64; 2]; 2]) -> std::cmp::Ordering {
    compare_points(DVec2::from(left[0]), DVec2::from(right[0]))
        .then_with(|| compare_points(DVec2::from(left[1]), DVec2::from(right[1])))
}

fn compare_points(left: DVec2, right: DVec2) -> std::cmp::Ordering {
    left.x
        .total_cmp(&right.x)
        .then_with(|| left.y.total_cmp(&right.y))
}

fn finite2(value: DVec2) -> bool {
    value.x.is_finite() && value.y.is_finite()
}

fn finite3(value: DVec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

fn abs_dot_sum(left: DVec3, right: DVec3) -> f64 {
    (left.x * right.x).abs() + (left.y * right.y).abs() + (left.z * right.z).abs()
}

fn normalize_plane_normal(normal: DVec3) -> Option<DVec3> {
    if !finite3(normal) {
        return None;
    }
    let scale = normal.x.abs().max(normal.y.abs()).max(normal.z.abs());
    if scale == 0.0 {
        return None;
    }
    let scaled = normal / scale;
    let length = scaled.length();
    (length.is_finite() && length > 0.0).then_some(scaled / length)
}
