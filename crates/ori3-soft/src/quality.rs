//! 表示用PBD候補を、入力の剛体フレームより悪化させないための品質検査。

use std::collections::{BTreeSet, HashMap};

use glam::DVec3;
use ori3_cp::Face;
use ori3_model::{FaceId, Frame3D, VertexId};

use crate::OVERLAP_RIGIDITY_TOLERANCE;

/// `ori3-rigid::self_intersection_pairs` と同じ正規化座標の許容誤差。
const INTERSECTION_TOLERANCE: f64 = 1e-6;

#[derive(Clone, Debug)]
pub(crate) struct FrameQuality {
    pub intersections: BTreeSet<(FaceId, FaceId)>,
    pub max_relative_edge_error: f64,
    pub max_face_planarity_error: f64,
    pub max_seam_gap: f64,
    pub finite: bool,
}

impl FrameQuality {
    /// 交差判定より安い剛性・finite検査だけを先に行う。バックトラック中の候補が
    /// 剛性閾値を超える場合、面ペアの全走査を省くために分離している。
    pub fn measure_geometry(faces: &[Face], before: &Frame3D, candidate: &Frame3D) -> Self {
        let finite = frame_is_finite(candidate);
        if !finite {
            return Self {
                intersections: BTreeSet::new(),
                max_relative_edge_error: f64::INFINITY,
                max_face_planarity_error: f64::INFINITY,
                max_seam_gap: f64::INFINITY,
                finite: false,
            };
        }

        let max_relative_edge_error = max_relative_edge_error(before, candidate);
        let max_face_planarity_error = max_face_planarity_error(candidate);
        let max_seam_gap = max_seam_gap(faces, candidate);
        let finite = max_relative_edge_error.is_finite()
            && max_face_planarity_error.is_finite()
            && max_seam_gap.is_finite();
        Self {
            intersections: BTreeSet::new(),
            max_relative_edge_error,
            max_face_planarity_error,
            max_seam_gap,
            finite,
        }
    }

    pub fn measure_intersections(&mut self, candidate: &Frame3D) {
        if self.finite {
            self.intersections = intersection_pairs(candidate);
        }
    }

    pub fn preserves_rigidity(&self) -> bool {
        self.finite
            && self.max_relative_edge_error <= OVERLAP_RIGIDITY_TOLERANCE
            && self.max_face_planarity_error <= OVERLAP_RIGIDITY_TOLERANCE
            && self.max_seam_gap <= OVERLAP_RIGIDITY_TOLERANCE
    }
}

pub(crate) fn intersection_pairs(frame: &Frame3D) -> BTreeSet<(FaceId, FaceId)> {
    // 製品判定と同じく、完全平坦な物理フレームでは同一平面上の重なりを調べない。
    if frame.faces.iter().all(|face| {
        face.polygon
            .iter()
            .all(|point| point[2].abs() < INTERSECTION_TOLERANCE)
    }) {
        return BTreeSet::new();
    }

    let parts: Vec<Part> = frame.faces.iter().map(Part::new).collect();
    let mut pairs = BTreeSet::new();
    for i in 0..parts.len() {
        for j in (i + 1)..parts.len() {
            let (a, b) = (&parts[i], &parts[j]);
            if !a.aabb_overlaps(b) || a.shares_edge(b) {
                continue;
            }
            if a.triangles.iter().any(|left| {
                b.triangles
                    .iter()
                    .any(|right| triangles_pierce(left, right))
            }) {
                let (a, b) = (frame.faces[i].face, frame.faces[j].face);
                pairs.insert((a.min(b), a.max(b)));
            }
        }
    }
    pairs
}

fn frame_is_finite(frame: &Frame3D) -> bool {
    frame
        .faces
        .iter()
        .flat_map(|face| &face.polygon)
        .flatten()
        .all(|coordinate| coordinate.is_finite())
}

/// 各面内の全頂点対距離を比較する。境界辺だけでなく対角距離も含めることで、
/// 平面内のせん断を辺長保存と誤認しない。
fn max_relative_edge_error(before: &Frame3D, candidate: &Frame3D) -> f64 {
    let candidate_by_id: HashMap<FaceId, _> = candidate
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect();
    let mut maximum = 0.0f64;
    for original in &before.faces {
        let Some(corrected) = candidate_by_id.get(&original.face) else {
            return f64::INFINITY;
        };
        if original.polygon.len() != corrected.polygon.len() {
            return f64::INFINITY;
        }
        for i in 0..original.polygon.len() {
            for j in (i + 1)..original.polygon.len() {
                let original_length =
                    (DVec3::from(original.polygon[j]) - DVec3::from(original.polygon[i])).length();
                let corrected_length = (DVec3::from(corrected.polygon[j])
                    - DVec3::from(corrected.polygon[i]))
                .length();
                let scale = original_length.max(INTERSECTION_TOLERANCE);
                maximum = maximum.max((corrected_length - original_length).abs() / scale);
            }
        }
    }
    if candidate.faces.len() == before.faces.len() {
        maximum
    } else {
        f64::INFINITY
    }
}

fn max_face_planarity_error(frame: &Frame3D) -> f64 {
    frame
        .faces
        .iter()
        .map(|face| face_planarity_error(&face.polygon))
        .fold(0.0, f64::max)
}

fn face_planarity_error(polygon: &[[f64; 3]]) -> f64 {
    if polygon.len() <= 3 {
        return 0.0;
    }
    let points: Vec<DVec3> = polygon.iter().copied().map(DVec3::from).collect();
    let centroid = points.iter().copied().sum::<DVec3>() / points.len() as f64;
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    let mut normal = DVec3::ZERO;
    for i in 0..points.len() {
        let current = points[i];
        let next = points[(i + 1) % points.len()];
        lo = lo.min(current);
        hi = hi.max(current);
        // Newell法。頂点順が保たれた凹多角形でも決定的な代表法線になる。
        normal.x += (current.y - next.y) * (current.z + next.z);
        normal.y += (current.z - next.z) * (current.x + next.x);
        normal.z += (current.x - next.x) * (current.y + next.y);
    }
    let scale = (hi - lo).length();
    if scale <= INTERSECTION_TOLERANCE {
        return 0.0;
    }
    let Some(normal) = normal.try_normalize() else {
        return f64::INFINITY;
    };
    points
        .iter()
        .map(|point| (*point - centroid).dot(normal).abs() / scale)
        .fold(0.0, f64::max)
}

fn max_seam_gap(faces: &[Face], frame: &Frame3D) -> f64 {
    let frame_by_id: HashMap<FaceId, _> =
        frame.faces.iter().map(|face| (face.face, face)).collect();
    let mut positions: HashMap<VertexId, Vec<DVec3>> = HashMap::new();
    for face in faces {
        let Some(output) = frame_by_id.get(&face.id) else {
            return f64::INFINITY;
        };
        if output.polygon.len() != face.vertices.len() {
            return f64::INFINITY;
        }
        for (&vertex, &point) in face.vertices.iter().zip(&output.polygon) {
            positions
                .entry(vertex)
                .or_default()
                .push(DVec3::from(point));
        }
    }
    positions
        .values()
        .flat_map(|points| {
            (0..points.len()).flat_map(move |i| {
                ((i + 1)..points.len()).map(move |j| (points[j] - points[i]).length())
            })
        })
        .fold(0.0, f64::max)
}

/// 製品の自己交差警告と同じ、扇分割済みの1面。
struct Part {
    triangles: Vec<[DVec3; 3]>,
    lo: DVec3,
    hi: DVec3,
    points: Vec<DVec3>,
}

impl Part {
    fn new(face: &ori3_model::Face3D) -> Self {
        let points: Vec<DVec3> = face.polygon.iter().copied().map(DVec3::from).collect();
        let triangles = (1..points.len().saturating_sub(1))
            .map(|index| [points[0], points[index], points[index + 1]])
            .collect();
        let lo = points
            .iter()
            .copied()
            .fold(DVec3::splat(f64::MAX), DVec3::min);
        let hi = points
            .iter()
            .copied()
            .fold(DVec3::splat(f64::MIN), DVec3::max);
        Self {
            triangles,
            lo,
            hi,
            points,
        }
    }

    fn aabb_overlaps(&self, other: &Self) -> bool {
        (0..3).all(|axis| {
            self.lo[axis] <= other.hi[axis] + INTERSECTION_TOLERANCE
                && other.lo[axis] <= self.hi[axis] + INTERSECTION_TOLERANCE
        })
    }

    fn shares_edge(&self, other: &Self) -> bool {
        self.points
            .iter()
            .filter(|point| {
                other
                    .points
                    .iter()
                    .any(|other_point| (**point - *other_point).length() <= INTERSECTION_TOLERANCE)
            })
            .count()
            >= 2
    }
}

fn triangles_pierce(left: &[DVec3; 3], right: &[DVec3; 3]) -> bool {
    (0..3).any(|index| segment_pierces(left[index], left[(index + 1) % 3], right))
        || (0..3).any(|index| segment_pierces(right[index], right[(index + 1) % 3], left))
}

fn segment_pierces(start: DVec3, end: DVec3, triangle: &[DVec3; 3]) -> bool {
    let direction = end - start;
    let (edge1, edge2) = (triangle[1] - triangle[0], triangle[2] - triangle[0]);
    let h = direction.cross(edge2);
    let determinant = edge1.dot(h);
    if determinant.abs()
        < INTERSECTION_TOLERANCE * direction.length() * edge1.length() * edge2.length()
    {
        return false;
    }
    let relative = start - triangle[0];
    let u = relative.dot(h) / determinant;
    let q = relative.cross(edge1);
    let v = direction.dot(q) / determinant;
    if u <= INTERSECTION_TOLERANCE
        || v <= INTERSECTION_TOLERANCE
        || u + v >= 1.0 - INTERSECTION_TOLERANCE
    {
        return false;
    }
    let t = edge2.dot(q) / determinant;
    if !(t > INTERSECTION_TOLERANCE && t < 1.0 - INTERSECTION_TOLERANCE) {
        return false;
    }

    // 製品判定は交点だけでなく、警告へ渡す法線・侵入深さもfiniteな場合だけ
    // 貫通として採用する。品質ゲートの組数を製品の組数と完全に揃える。
    let raw_normal = edge1.cross(edge2);
    let normal_length = raw_normal.length();
    if !normal_length.is_finite() || normal_length == 0.0 {
        return false;
    }
    let normal = raw_normal / normal_length;
    let point = start + direction * t;
    let endpoint_depth = (start - triangle[0])
        .dot(normal)
        .abs()
        .min((end - triangle[0]).dot(normal).abs());
    let penetration_depth = (endpoint_depth - INTERSECTION_TOLERANCE).max(0.0);
    point.is_finite() && normal.is_finite() && penetration_depth.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ori3_model::Face3D;

    fn rigid_product_pairs(frame: &Frame3D) -> BTreeSet<(FaceId, FaceId)> {
        ori3_rigid::self_intersection_pairs(frame)
            .into_iter()
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect()
    }

    #[test]
    fn product_equivalent_intersection_detects_a_proper_piercing() {
        let frame = Frame3D {
            faces: vec![
                Face3D {
                    face: 7,
                    polygon: vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]],
                    layer: 0,
                },
                Face3D {
                    face: 3,
                    polygon: vec![[0.0, -0.5, -1.0], [0.0, 0.5, 1.0], [0.5, 0.0, 1.0]],
                    layer: 1,
                },
            ],
            warnings: Vec::new(),
        };
        assert_eq!(intersection_pairs(&frame), BTreeSet::from([(3, 7)]));
        assert_eq!(intersection_pairs(&frame), rigid_product_pairs(&frame));
    }

    #[test]
    fn coplanar_overlap_is_not_a_product_penetration() {
        let frame = Frame3D {
            faces: vec![
                Face3D {
                    face: 0,
                    polygon: vec![[0.0, 0.0, 0.1], [1.0, 0.0, 0.1], [0.0, 1.0, 0.1]],
                    layer: 0,
                },
                Face3D {
                    face: 1,
                    polygon: vec![[0.1, 0.1, 0.1], [1.1, 0.1, 0.1], [0.1, 1.1, 0.1]],
                    layer: 1,
                },
            ],
            warnings: Vec::new(),
        };
        assert!(intersection_pairs(&frame).is_empty());
        assert_eq!(intersection_pairs(&frame), rigid_product_pairs(&frame));
    }
}
