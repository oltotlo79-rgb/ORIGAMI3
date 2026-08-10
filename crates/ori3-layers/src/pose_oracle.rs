//! Feature extraction and structured verification for non-flat [`Frame3D`] poses.
//!
//! Unlike the flat-step oracle, this module never assigns one global layer order
//! to a solid pose.  Depth is sampled along an explicit 3D ray and is therefore
//! local to a viewpoint.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use glam::{DVec2, DVec3};
use ori3_cp::Face;
use ori3_model::{CreasePattern, FaceId, Frame3D, VertexId};
use ori3_rigid::max_seam_gap;

const DEFAULT_GEOMETRY_TOLERANCE: f64 = 1.0e-8;

/// Numeric tolerances used while validating a solid pose.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PoseOracleTolerance {
    /// CP-space radius used to resolve a material-coordinate vertex reference.
    pub material_vertex: f64,
    /// Maximum distance between copies of one CP vertex on adjacent faces.
    pub shared_vertex: f64,
    /// Position and distance tolerance for landmark expectations.
    pub landmark: f64,
    /// Boundary and equal-depth tolerance used by ray casting.
    pub ray: f64,
}

impl Default for PoseOracleTolerance {
    fn default() -> Self {
        Self {
            material_vertex: 1.0e-9,
            shared_vertex: 1.0e-7,
            landmark: 1.0e-6,
            ray: DEFAULT_GEOMETRY_TOLERANCE,
        }
    }
}

/// A forward ray in the root-face coordinate system used by [`Frame3D`].
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Ray3 {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
}

/// One face hit by a [`Ray3`].  Hits are ordered from the ray origin outwards.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FaceHit {
    pub face: FaceId,
    pub distance: f64,
    pub position: [f64; 3],
    pub front_facing: bool,
}

/// Invalid input to [`raycast_faces`].
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum RaycastError {
    InvalidTolerance { tolerance: f64 },
    NonFiniteOrigin { origin: [f64; 3] },
    InvalidDirection { direction: [f64; 3] },
}

impl fmt::Display for RaycastError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTolerance { tolerance } => {
                write!(formatter, "ray tolerance is invalid: {tolerance}")
            }
            Self::NonFiniteOrigin { origin } => {
                write!(formatter, "ray origin is not finite: {origin:?}")
            }
            Self::InvalidDirection { direction } => {
                write!(
                    formatter,
                    "ray direction is not finite and non-zero: {direction:?}"
                )
            }
        }
    }
}

/// A material vertex condition, addressed by its stable CP coordinate rather
/// than by a regenerated [`VertexId`].
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PoseLandmarkExpectation {
    /// The material vertex must occupy this absolute root-frame position.
    Position {
        material: [f64; 2],
        expected: [f64; 3],
    },
    /// Euclidean separation of two material vertices in the solid pose.
    Distance {
        first_material: [f64; 2],
        second_material: [f64; 2],
        expected_distance: f64,
    },
}

/// Expected near-to-far faces along one 3D viewing ray.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PoseDepthExpectation {
    pub ray: Ray3,
    pub expected_near_to_far: Vec<FaceId>,
}

/// Data-driven constraints for one non-flat pose.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PoseExpectation {
    /// Exact expected face set.  Ordering is ignored, duplicates are reported.
    pub expected_faces: Vec<FaceId>,
    /// The measured normalized seam gap must be strictly below this value.
    pub max_seam_gap: f64,
    /// Minimum required z extent.  The rigid solver fixes the root face to z=0.
    pub min_z_span: f64,
    pub landmarks: Vec<PoseLandmarkExpectation>,
    pub depth_probes: Vec<PoseDepthExpectation>,
    pub tolerance: PoseOracleTolerance,
}

impl Default for PoseExpectation {
    fn default() -> Self {
        Self {
            expected_faces: Vec::new(),
            max_seam_gap: 1.0e-6,
            min_z_span: 1.0e-6,
            landmarks: Vec::new(),
            depth_probes: Vec::new(),
            tolerance: PoseOracleTolerance::default(),
        }
    }
}

impl PoseExpectation {
    /// Start an expectation with every supplied topology face required exactly once.
    #[must_use]
    pub fn from_faces(faces: &[Face]) -> Self {
        let mut expected_faces = faces.iter().map(|face| face.id).collect::<Vec<_>>();
        expected_faces.sort_unstable();
        Self {
            expected_faces,
            ..Self::default()
        }
    }
}

/// Global measurements extracted from one pose.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PoseFeatures {
    /// Sorted frame face IDs.  Duplicates remain visible in this list.
    pub face_ids: Vec<FaceId>,
    pub max_seam_gap: f64,
    pub z_span: f64,
}

/// Actual value measured for a landmark expectation.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PoseLandmarkSample {
    Position {
        material: [f64; 2],
        actual: Option<[f64; 3]>,
    },
    Distance {
        first_material: [f64; 2],
        second_material: [f64; 2],
        actual_distance: Option<f64>,
    },
}

/// Actual ray hits collected for a depth probe.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PoseDepthSample {
    pub ray: Ray3,
    pub hits: Vec<FaceHit>,
}

/// A structured mismatch suitable for test diagnostics and candidate scoring.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "feature", rename_all = "snake_case")]
pub enum PoseDifference {
    DuplicateExpectedFace {
        face: FaceId,
    },
    DuplicateTopologyFace {
        face: FaceId,
    },
    DuplicateFrameFace {
        face: FaceId,
    },
    ExpectedFaceMissingFromTopology {
        face: FaceId,
    },
    MissingFace {
        face: FaceId,
    },
    UnexpectedFace {
        face: FaceId,
    },
    FrameFaceMissingFromTopology {
        face: FaceId,
    },
    PolygonVertexCount {
        face: FaceId,
        expected: usize,
        actual: usize,
    },
    NonFiniteCoordinate {
        face: FaceId,
        vertex_index: usize,
        axis: usize,
    },
    SharedVertexMismatch {
        vertex: VertexId,
        first_face: FaceId,
        second_face: FaceId,
        distance: f64,
        tolerance: f64,
    },
    NonFiniteSeamGap,
    SeamGap {
        maximum: f64,
        actual: f64,
    },
    NonFlatness {
        minimum_z_span: f64,
        actual_z_span: f64,
    },
    MaterialVertexMissing {
        landmark: usize,
        material: [f64; 2],
    },
    MaterialVertexAmbiguous {
        landmark: usize,
        material: [f64; 2],
        candidates: Vec<VertexId>,
    },
    LandmarkVertexUnavailable {
        landmark: usize,
        material: [f64; 2],
        vertex: VertexId,
    },
    LandmarkPosition {
        landmark: usize,
        material: [f64; 2],
        expected: [f64; 3],
        actual: [f64; 3],
        distance: f64,
    },
    LandmarkDistance {
        landmark: usize,
        first_material: [f64; 2],
        second_material: [f64; 2],
        expected: f64,
        actual: f64,
        delta: f64,
    },
    InvalidDepthRay {
        probe: usize,
        message: String,
    },
    DepthOrder {
        probe: usize,
        expected_near_to_far: Vec<FaceId>,
        actual_near_to_far: Vec<FaceId>,
    },
}

impl fmt::Display for PoseDifference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateExpectedFace { face } => {
                write!(formatter, "期待face集合で面{face}が重複しています")
            }
            Self::DuplicateTopologyFace { face } => {
                write!(formatter, "topologyで面{face}が重複しています")
            }
            Self::DuplicateFrameFace { face } => {
                write!(formatter, "Frame3Dで面{face}が重複しています")
            }
            Self::ExpectedFaceMissingFromTopology { face } => {
                write!(formatter, "期待面{face}がtopologyにありません")
            }
            Self::MissingFace { face } => {
                write!(formatter, "Frame3Dから期待面{face}が失われました")
            }
            Self::UnexpectedFace { face } => write!(formatter, "Frame3Dに余剰面{face}があります"),
            Self::FrameFaceMissingFromTopology { face } => {
                write!(formatter, "Frame3Dの面{face}に対応するtopologyがありません")
            }
            Self::PolygonVertexCount {
                face,
                expected,
                actual,
            } => write!(
                formatter,
                "面{face}のpolygon頂点数が期待{expected}に対し実際{actual}です"
            ),
            Self::NonFiniteCoordinate {
                face,
                vertex_index,
                axis,
            } => write!(
                formatter,
                "面{face}のpolygon頂点{vertex_index}の座標軸{axis}が有限値ではありません"
            ),
            Self::SharedVertexMismatch {
                vertex,
                first_face,
                second_face,
                distance,
                tolerance,
            } => write!(
                formatter,
                "共有頂点{vertex}の面{first_face}/面{second_face}間距離{distance:.3e}が許容{tolerance:.3e}を超えます"
            ),
            Self::NonFiniteSeamGap => write!(formatter, "max_seam_gapが有限値ではありません"),
            Self::SeamGap { maximum, actual } => write!(
                formatter,
                "max_seam_gap={actual:.3e}で、必要な上限{maximum:.3e}未満ではありません"
            ),
            Self::NonFlatness {
                minimum_z_span,
                actual_z_span,
            } => write!(
                formatter,
                "z_span={actual_z_span:.3e}で、必要な非平坦幅{minimum_z_span:.3e}に達しません"
            ),
            Self::MaterialVertexMissing { landmark, material } => write!(
                formatter,
                "landmark {landmark}のmaterial頂点{material:?}がCPにありません"
            ),
            Self::MaterialVertexAmbiguous {
                landmark,
                material,
                candidates,
            } => write!(
                formatter,
                "landmark {landmark}のmaterial頂点{material:?}が複数候補{candidates:?}へ解決されます"
            ),
            Self::LandmarkVertexUnavailable {
                landmark,
                material,
                vertex,
            } => write!(
                formatter,
                "landmark {landmark}のmaterial頂点{material:?}(vertex {vertex})をFrame3Dから取得できません"
            ),
            Self::LandmarkPosition {
                landmark,
                expected,
                actual,
                distance,
                ..
            } => write!(
                formatter,
                "landmark {landmark}の3D位置が期待{expected:?}に対し実際{actual:?}、距離{distance:.3e}です"
            ),
            Self::LandmarkDistance {
                landmark,
                expected,
                actual,
                delta,
                ..
            } => write!(
                formatter,
                "landmark {landmark}の頂点間距離が期待{expected:.9}に対し実際{actual:.9}、差{delta:.3e}です"
            ),
            Self::InvalidDepthRay { probe, message } => {
                write!(formatter, "depth probe {probe}のrayが不正です: {message}")
            }
            Self::DepthOrder {
                probe,
                expected_near_to_far,
                actual_near_to_far,
            } => write!(
                formatter,
                "depth probe {probe}の前後順が期待{expected_near_to_far:?}に対し実際{actual_near_to_far:?}です"
            ),
        }
    }
}

/// Complete pose-oracle output.  `differences` is empty exactly on a match.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PoseOracleReport {
    pub actual: PoseFeatures,
    pub landmark_samples: Vec<PoseLandmarkSample>,
    pub depth_samples: Vec<PoseDepthSample>,
    pub differences: Vec<PoseDifference>,
}

impl PoseOracleReport {
    #[must_use]
    pub fn is_match(&self) -> bool {
        self.differences.is_empty()
    }

    #[must_use]
    pub fn explanations(&self) -> Vec<String> {
        self.differences.iter().map(ToString::to_string).collect()
    }
}

/// Compare a solid frame with topology and externally supplied expectations.
#[must_use]
pub fn evaluate_pose(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
    expectation: &PoseExpectation,
) -> PoseOracleReport {
    let mut differences = Vec::new();
    let mut topology = BTreeMap::<FaceId, &Face>::new();
    for face in faces {
        if topology.insert(face.id, face).is_some() {
            differences.push(PoseDifference::DuplicateTopologyFace { face: face.id });
        }
    }

    let mut expected = BTreeSet::new();
    for &face in &expectation.expected_faces {
        if !expected.insert(face) {
            differences.push(PoseDifference::DuplicateExpectedFace { face });
        }
        if !topology.contains_key(&face) {
            differences.push(PoseDifference::ExpectedFaceMissingFromTopology { face });
        }
    }

    let mut actual_face_ids = frame.faces.iter().map(|face| face.face).collect::<Vec<_>>();
    actual_face_ids.sort_unstable();
    let mut frame_by_face = BTreeMap::new();
    for face in &frame.faces {
        if frame_by_face.insert(face.face, face).is_some() {
            differences.push(PoseDifference::DuplicateFrameFace { face: face.face });
        }
    }
    let actual_set = frame_by_face.keys().copied().collect::<BTreeSet<_>>();
    for &face in expected.difference(&actual_set) {
        differences.push(PoseDifference::MissingFace { face });
    }
    for &face in actual_set.difference(&expected) {
        differences.push(PoseDifference::UnexpectedFace { face });
    }

    let mut vertex_copies = BTreeMap::<VertexId, Vec<(FaceId, DVec3)>>::new();
    let mut min_z = f64::INFINITY;
    let mut max_z = f64::NEG_INFINITY;
    for (&face_id, output) in &frame_by_face {
        let Some(face) = topology.get(&face_id) else {
            differences.push(PoseDifference::FrameFaceMissingFromTopology { face: face_id });
            continue;
        };
        if face.vertices.len() != output.polygon.len() {
            differences.push(PoseDifference::PolygonVertexCount {
                face: face_id,
                expected: face.vertices.len(),
                actual: output.polygon.len(),
            });
        }
        for (index, point) in output.polygon.iter().enumerate() {
            let mut finite = true;
            for (axis, coordinate) in point.iter().enumerate() {
                if !coordinate.is_finite() {
                    finite = false;
                    differences.push(PoseDifference::NonFiniteCoordinate {
                        face: face_id,
                        vertex_index: index,
                        axis,
                    });
                }
            }
            if finite {
                min_z = min_z.min(point[2]);
                max_z = max_z.max(point[2]);
                if let Some(&vertex) = face.vertices.get(index) {
                    vertex_copies
                        .entry(vertex)
                        .or_default()
                        .push((face_id, DVec3::from(*point)));
                }
            }
        }
    }

    let shared_tolerance = expectation.tolerance.shared_vertex;
    for (&vertex, copies) in &vertex_copies {
        let Some(&(first_face, first)) = copies.first() else {
            continue;
        };
        if let Some((second_face, _, distance)) = copies
            .iter()
            .skip(1)
            .map(|&(face, point)| (face, point, point.distance(first)))
            .max_by(|left, right| left.2.total_cmp(&right.2))
            && distance > shared_tolerance
        {
            differences.push(PoseDifference::SharedVertexMismatch {
                vertex,
                first_face,
                second_face,
                distance,
                tolerance: shared_tolerance,
            });
        }
    }

    let z_span = if min_z.is_finite() && max_z.is_finite() {
        max_z - min_z
    } else {
        0.0
    };
    if z_span < expectation.min_z_span {
        differences.push(PoseDifference::NonFlatness {
            minimum_z_span: expectation.min_z_span,
            actual_z_span: z_span,
        });
    }

    let seam_gap = max_seam_gap(cp, faces, frame);
    if !seam_gap.is_finite() {
        differences.push(PoseDifference::NonFiniteSeamGap);
    } else if seam_gap >= expectation.max_seam_gap {
        differences.push(PoseDifference::SeamGap {
            maximum: expectation.max_seam_gap,
            actual: seam_gap,
        });
    }

    let vertex_positions = vertex_copies
        .iter()
        .filter_map(|(&vertex, copies)| copies.first().map(|&(_, point)| (vertex, point)))
        .collect::<BTreeMap<_, _>>();
    let landmark_samples = evaluate_landmarks(cp, &vertex_positions, expectation, &mut differences);

    let mut depth_samples = Vec::with_capacity(expectation.depth_probes.len());
    for (probe, expected_depth) in expectation.depth_probes.iter().enumerate() {
        match raycast_faces(frame, expected_depth.ray, expectation.tolerance.ray) {
            Ok(hits) => {
                let actual_near_to_far = hits.iter().map(|hit| hit.face).collect::<Vec<_>>();
                if actual_near_to_far != expected_depth.expected_near_to_far {
                    differences.push(PoseDifference::DepthOrder {
                        probe,
                        expected_near_to_far: expected_depth.expected_near_to_far.clone(),
                        actual_near_to_far,
                    });
                }
                depth_samples.push(PoseDepthSample {
                    ray: expected_depth.ray,
                    hits,
                });
            }
            Err(error) => {
                differences.push(PoseDifference::InvalidDepthRay {
                    probe,
                    message: error.to_string(),
                });
                depth_samples.push(PoseDepthSample {
                    ray: expected_depth.ray,
                    hits: Vec::new(),
                });
            }
        }
    }

    PoseOracleReport {
        actual: PoseFeatures {
            face_ids: actual_face_ids,
            max_seam_gap: seam_gap,
            z_span,
        },
        landmark_samples,
        depth_samples,
        differences,
    }
}

fn evaluate_landmarks(
    cp: &CreasePattern,
    vertex_positions: &BTreeMap<VertexId, DVec3>,
    expectation: &PoseExpectation,
    differences: &mut Vec<PoseDifference>,
) -> Vec<PoseLandmarkSample> {
    expectation
        .landmarks
        .iter()
        .enumerate()
        .map(|(landmark, expected)| match expected {
            PoseLandmarkExpectation::Position { material, expected } => {
                let actual = resolve_landmark_position(
                    cp,
                    vertex_positions,
                    landmark,
                    *material,
                    expectation.tolerance.material_vertex,
                    differences,
                );
                if let Some(actual) = actual {
                    let distance = actual.distance(DVec3::from(*expected));
                    if distance > expectation.tolerance.landmark {
                        differences.push(PoseDifference::LandmarkPosition {
                            landmark,
                            material: *material,
                            expected: *expected,
                            actual: actual.to_array(),
                            distance,
                        });
                    }
                }
                PoseLandmarkSample::Position {
                    material: *material,
                    actual: actual.map(|point| point.to_array()),
                }
            }
            PoseLandmarkExpectation::Distance {
                first_material,
                second_material,
                expected_distance,
            } => {
                let first = resolve_landmark_position(
                    cp,
                    vertex_positions,
                    landmark,
                    *first_material,
                    expectation.tolerance.material_vertex,
                    differences,
                );
                let second = resolve_landmark_position(
                    cp,
                    vertex_positions,
                    landmark,
                    *second_material,
                    expectation.tolerance.material_vertex,
                    differences,
                );
                let actual_distance = first.zip(second).map(|(a, b)| a.distance(b));
                if let Some(actual) = actual_distance {
                    let delta = (actual - expected_distance).abs();
                    if delta > expectation.tolerance.landmark {
                        differences.push(PoseDifference::LandmarkDistance {
                            landmark,
                            first_material: *first_material,
                            second_material: *second_material,
                            expected: *expected_distance,
                            actual,
                            delta,
                        });
                    }
                }
                PoseLandmarkSample::Distance {
                    first_material: *first_material,
                    second_material: *second_material,
                    actual_distance,
                }
            }
        })
        .collect()
}

fn resolve_landmark_position(
    cp: &CreasePattern,
    vertex_positions: &BTreeMap<VertexId, DVec3>,
    landmark: usize,
    material: [f64; 2],
    tolerance: f64,
    differences: &mut Vec<PoseDifference>,
) -> Option<DVec3> {
    let target = DVec2::from(material);
    let mut candidates = cp
        .vertices
        .iter()
        .filter(|vertex| DVec2::from(vertex.pos).distance(target) <= tolerance)
        .map(|vertex| vertex.id)
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    let vertex = match candidates.as_slice() {
        [] => {
            differences.push(PoseDifference::MaterialVertexMissing { landmark, material });
            return None;
        }
        &[vertex] => vertex,
        _ => {
            differences.push(PoseDifference::MaterialVertexAmbiguous {
                landmark,
                material,
                candidates,
            });
            return None;
        }
    };
    match vertex_positions.get(&vertex).copied() {
        Some(point) => Some(point),
        None => {
            differences.push(PoseDifference::LandmarkVertexUnavailable {
                landmark,
                material,
                vertex,
            });
            None
        }
    }
}

/// Intersect a ray with every polygon and return hits in increasing distance.
///
/// Polygon containment is evaluated after projection to its dominant plane, so
/// concave faces do not acquire the false triangles produced by fan splitting.
pub fn raycast_faces(
    frame: &Frame3D,
    ray: Ray3,
    tolerance: f64,
) -> Result<Vec<FaceHit>, RaycastError> {
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err(RaycastError::InvalidTolerance { tolerance });
    }
    let origin = DVec3::from(ray.origin);
    if !origin.is_finite() {
        return Err(RaycastError::NonFiniteOrigin { origin: ray.origin });
    }
    let direction = DVec3::from(ray.direction);
    if !direction.is_finite() || direction.length() <= tolerance.max(f64::EPSILON) {
        return Err(RaycastError::InvalidDirection {
            direction: ray.direction,
        });
    }
    let direction = direction.normalize();
    let epsilon = tolerance.max(f64::EPSILON);
    let mut hits = Vec::new();
    for face in &frame.faces {
        let polygon = face
            .polygon
            .iter()
            .copied()
            .map(DVec3::from)
            .collect::<Vec<_>>();
        if polygon.len() < 3 || polygon.iter().any(|point| !point.is_finite()) {
            continue;
        }
        let Some(normal) = polygon_normal(&polygon, epsilon) else {
            continue;
        };
        let denominator = normal.dot(direction);
        if denominator.abs() <= epsilon {
            continue;
        }
        let distance = normal.dot(polygon[0] - origin) / denominator;
        if distance < -epsilon {
            continue;
        }
        let distance = distance.max(0.0);
        let position = origin + direction * distance;
        if point_in_planar_polygon(position, &polygon, normal, epsilon) {
            hits.push(FaceHit {
                face: face.face,
                distance,
                position: position.to_array(),
                front_facing: denominator < 0.0,
            });
        }
    }
    hits.sort_by(|left, right| {
        left.distance
            .total_cmp(&right.distance)
            .then(left.face.cmp(&right.face))
    });
    Ok(hits)
}

fn polygon_normal(polygon: &[DVec3], tolerance: f64) -> Option<DVec3> {
    let origin = polygon[0];
    for first in 1..polygon.len().saturating_sub(1) {
        for second in first + 1..polygon.len() {
            let cross = (polygon[first] - origin).cross(polygon[second] - origin);
            if cross.length() > tolerance * tolerance {
                return Some(cross.normalize());
            }
        }
    }
    None
}

fn point_in_planar_polygon(point: DVec3, polygon: &[DVec3], normal: DVec3, tolerance: f64) -> bool {
    let axis = if normal.x.abs() >= normal.y.abs() && normal.x.abs() >= normal.z.abs() {
        0
    } else if normal.y.abs() >= normal.z.abs() {
        1
    } else {
        2
    };
    let project = |point: DVec3| match axis {
        0 => DVec2::new(point.y, point.z),
        1 => DVec2::new(point.x, point.z),
        _ => DVec2::new(point.x, point.y),
    };
    let point = project(point);
    let polygon = polygon.iter().copied().map(project).collect::<Vec<_>>();
    if (0..polygon.len()).any(|index| {
        distance_to_segment(point, polygon[index], polygon[(index + 1) % polygon.len()])
            <= tolerance
    }) {
        return true;
    }
    let mut inside = false;
    for index in 0..polygon.len() {
        let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
        if (a.y > point.y) != (b.y > point.y) {
            let parameter = (point.y - a.y) / (b.y - a.y);
            if point.x < a.x + parameter * (b.x - a.x) {
                inside = !inside;
            }
        }
    }
    inside
}

fn distance_to_segment(point: DVec2, a: DVec2, b: DVec2) -> f64 {
    let direction = b - a;
    let length_squared = direction.length_squared();
    if length_squared <= f64::EPSILON {
        return point.distance(a);
    }
    let parameter = ((point - a).dot(direction) / length_squared).clamp(0.0, 1.0);
    point.distance(a + direction * parameter)
}
