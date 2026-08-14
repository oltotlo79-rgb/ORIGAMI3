//! Static three-point support checks for folded frames.
//!
//! A folded sheet can be placed on a horizontal floor when three selected
//! vertices define a supporting plane, every vertex lies on one side of that
//! plane, and the projection of the sheet's area centroid lies inside the
//! support triangle.  The functions in this module are independent of world
//! orientation: the returned plane normal is oriented towards the sheet.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use glam::DVec3;
use ori3_cp::Face;
use ori3_model::{FaceId, Frame3D, VertexId};

/// Default absolute tolerance for normalized crease-pattern coordinates.
pub const DEFAULT_SUPPORT_TOLERANCE: f64 = 1e-8;

/// Oriented plane through the three support vertices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SupportPlane {
    /// First support point, used as a point on the plane.
    pub origin: [f64; 3],
    /// Unit normal oriented towards the folded sheet whenever it is one-sided.
    pub normal: [f64; 3],
}

/// Numerical result of a static three-point support check.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreePointSupport {
    pub support_vertices: [VertexId; 3],
    pub support_points: [[f64; 3]; 3],
    pub plane: SupportPlane,
    pub support_area: f64,
    pub surface_area: f64,
    pub surface_centroid: [f64; 3],
    pub projected_centroid: [f64; 3],
    /// Barycentric weights of `projected_centroid` in support-point order.
    pub centroid_barycentric: [f64; 3],
    /// Signed height of the area centroid above the oriented support plane.
    pub centroid_height: f64,
    /// Minimum signed vertex distance from the oriented support plane.
    pub min_signed_distance: f64,
    /// Maximum signed vertex distance from the oriented support plane.
    pub max_signed_distance: f64,
    /// Amount by which a vertex penetrates the support plane.
    pub sidedness_violation: f64,
    pub one_sided: bool,
    pub centroid_projection_inside: bool,
    /// True when the sheet is one-sided and its centroid projection is supported.
    pub stable: bool,
}

/// Invalid topology or geometry supplied to [`three_point_support`].
#[derive(Clone, Debug, PartialEq)]
pub enum SupportError {
    InvalidTolerance(f64),
    DuplicateSupportVertex(VertexId),
    DuplicateTopologyFace(FaceId),
    DuplicateFrameFace(FaceId),
    MissingTopologyFace(FaceId),
    MalformedFace {
        face: FaceId,
        topology_vertices: usize,
        polygon_points: usize,
    },
    NonFinitePoint {
        face: FaceId,
        index: usize,
    },
    InconsistentVertexPosition {
        vertex: VertexId,
        gap: f64,
    },
    MissingSupportVertex(VertexId),
    DegenerateFace(FaceId),
    DegenerateSurface,
    DegenerateSupportTriangle {
        twice_area: f64,
    },
}

impl fmt::Display for SupportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTolerance(value) => {
                write!(
                    f,
                    "support tolerance must be finite and non-negative: {value}"
                )
            }
            Self::DuplicateSupportVertex(vertex) => {
                write!(f, "support vertex {vertex} was specified more than once")
            }
            Self::DuplicateTopologyFace(face) => {
                write!(f, "topology contains duplicate face id {face}")
            }
            Self::DuplicateFrameFace(face) => {
                write!(f, "frame contains duplicate face id {face}")
            }
            Self::MissingTopologyFace(face) => {
                write!(f, "frame face {face} has no matching topology face")
            }
            Self::MalformedFace {
                face,
                topology_vertices,
                polygon_points,
            } => write!(
                f,
                "face {face} has {topology_vertices} topology vertices but {polygon_points} frame points"
            ),
            Self::NonFinitePoint { face, index } => {
                write!(f, "face {face} point {index} is not finite")
            }
            Self::InconsistentVertexPosition { vertex, gap } => write!(
                f,
                "copies of vertex {vertex} disagree by {gap} in the folded frame"
            ),
            Self::MissingSupportVertex(vertex) => {
                write!(f, "support vertex {vertex} is absent from the folded frame")
            }
            Self::DegenerateFace(face) => write!(f, "frame face {face} has zero area"),
            Self::DegenerateSurface => write!(f, "folded frame has no positive-area surface"),
            Self::DegenerateSupportTriangle { twice_area } => write!(
                f,
                "support vertices form a degenerate triangle (twice_area={twice_area})"
            ),
        }
    }
}

impl Error for SupportError {}

/// Evaluate static support using [`DEFAULT_SUPPORT_TOLERANCE`].
pub fn three_point_support(
    faces: &[Face],
    frame: &Frame3D,
    support_vertices: [VertexId; 3],
) -> Result<ThreePointSupport, SupportError> {
    three_point_support_with_tolerance(faces, frame, support_vertices, DEFAULT_SUPPORT_TOLERANCE)
}

/// Evaluate whether three vertices can support a folded sheet.
///
/// `faces` supplies the vertex IDs corresponding to each polygon point in
/// `frame`. The normal of the returned support plane is automatically oriented
/// to the side containing the sheet. A successful return describes both stable
/// and unstable configurations; [`ThreePointSupport::stable`] is the combined
/// static-support decision. Structural input errors are returned as
/// [`SupportError`].
pub fn three_point_support_with_tolerance(
    faces: &[Face],
    frame: &Frame3D,
    support_vertices: [VertexId; 3],
    tolerance: f64,
) -> Result<ThreePointSupport, SupportError> {
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err(SupportError::InvalidTolerance(tolerance));
    }
    for i in 0..support_vertices.len() {
        if support_vertices[..i].contains(&support_vertices[i]) {
            return Err(SupportError::DuplicateSupportVertex(support_vertices[i]));
        }
    }

    let mut topology = HashMap::<FaceId, &Face>::new();
    for face in faces {
        if topology.insert(face.id, face).is_some() {
            return Err(SupportError::DuplicateTopologyFace(face.id));
        }
    }

    let mut seen_frame_faces = HashSet::<FaceId>::new();
    let mut positions = HashMap::<VertexId, DVec3>::new();
    let mut surface_area = 0.0;
    let mut surface_moment = DVec3::ZERO;
    for frame_face in &frame.faces {
        if !seen_frame_faces.insert(frame_face.face) {
            return Err(SupportError::DuplicateFrameFace(frame_face.face));
        }
        let Some(face) = topology.get(&frame_face.face) else {
            return Err(SupportError::MissingTopologyFace(frame_face.face));
        };
        if face.vertices.len() != frame_face.polygon.len() {
            return Err(SupportError::MalformedFace {
                face: frame_face.face,
                topology_vertices: face.vertices.len(),
                polygon_points: frame_face.polygon.len(),
            });
        }

        let mut polygon = Vec::with_capacity(frame_face.polygon.len());
        for (index, (&vertex, &point)) in face.vertices.iter().zip(&frame_face.polygon).enumerate()
        {
            let point = DVec3::from(point);
            if !point.is_finite() {
                return Err(SupportError::NonFinitePoint {
                    face: frame_face.face,
                    index,
                });
            }
            if let Some(previous) = positions.insert(vertex, point) {
                let gap = (point - previous).length();
                if gap > tolerance {
                    return Err(SupportError::InconsistentVertexPosition { vertex, gap });
                }
            }
            polygon.push(point);
        }

        let (area, centroid) = polygon_area_centroid(&polygon, tolerance)
            .ok_or(SupportError::DegenerateFace(frame_face.face))?;
        surface_area += area;
        surface_moment += centroid * area;
    }
    if !surface_area.is_finite() || surface_area <= tolerance * tolerance {
        return Err(SupportError::DegenerateSurface);
    }
    let surface_centroid = surface_moment / surface_area;

    let mut support_points = [DVec3::ZERO; 3];
    for (point, vertex) in support_points.iter_mut().zip(support_vertices) {
        *point = positions
            .get(&vertex)
            .copied()
            .ok_or(SupportError::MissingSupportVertex(vertex))?;
    }
    let [a, b, c] = support_points;
    let cross = (b - a).cross(c - a);
    let twice_area = cross.length();
    if !twice_area.is_finite() || twice_area <= tolerance * tolerance {
        return Err(SupportError::DegenerateSupportTriangle { twice_area });
    }
    let mut normal = cross / twice_area;

    let raw_range = signed_distance_range(positions.values().copied(), a, normal);
    if raw_range.0 >= -tolerance {
        // The input winding already points towards the sheet.
    } else if raw_range.1 <= tolerance || normal.dot(surface_centroid - a) < 0.0 {
        normal = -normal;
    }
    let (min_signed_distance, max_signed_distance) =
        signed_distance_range(positions.values().copied(), a, normal);
    let one_sided = min_signed_distance >= -tolerance;

    let centroid_height = normal.dot(surface_centroid - a);
    let projected_centroid = surface_centroid - normal * centroid_height;
    let centroid_barycentric = barycentric(projected_centroid, a, b, c);
    let longest_edge = (b - a).length().max((c - b).length()).max((a - c).length());
    let barycentric_tolerance = if longest_edge > 0.0 {
        tolerance / longest_edge
    } else {
        tolerance
    };
    let centroid_projection_inside = centroid_barycentric
        .iter()
        .all(|&weight| weight >= -barycentric_tolerance);
    let stable = one_sided && centroid_height >= -tolerance && centroid_projection_inside;

    Ok(ThreePointSupport {
        support_vertices,
        support_points: support_points.map(|point| point.to_array()),
        plane: SupportPlane {
            origin: a.to_array(),
            normal: normal.to_array(),
        },
        support_area: 0.5 * twice_area,
        surface_area,
        surface_centroid: surface_centroid.to_array(),
        projected_centroid: projected_centroid.to_array(),
        centroid_barycentric,
        centroid_height,
        min_signed_distance,
        max_signed_distance,
        sidedness_violation: (-min_signed_distance).max(0.0),
        one_sided,
        centroid_projection_inside,
        stable,
    })
}

fn signed_distance_range(
    points: impl Iterator<Item = DVec3>,
    origin: DVec3,
    normal: DVec3,
) -> (f64, f64) {
    points.fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(minimum, maximum), point| {
            let distance = normal.dot(point - origin);
            (minimum.min(distance), maximum.max(distance))
        },
    )
}

fn polygon_area_centroid(points: &[DVec3], tolerance: f64) -> Option<(f64, DVec3)> {
    if points.len() < 3 {
        return None;
    }
    let area_vector = points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
        .fold(DVec3::ZERO, |sum, (a, b)| sum + a.cross(b));
    let twice_area = area_vector.length();
    if !twice_area.is_finite() || twice_area <= tolerance * tolerance {
        return None;
    }
    let normal = area_vector / twice_area;
    let origin = points[0];
    let mut signed_twice_area = 0.0;
    let mut signed_moment = DVec3::ZERO;
    for triangle in points[1..].windows(2) {
        let b = triangle[0];
        let c = triangle[1];
        let weight = normal.dot((b - origin).cross(c - origin));
        signed_twice_area += weight;
        signed_moment += (origin + b + c) / 3.0 * weight;
    }
    if !signed_twice_area.is_finite() || signed_twice_area <= tolerance * tolerance {
        return None;
    }
    Some((0.5 * signed_twice_area, signed_moment / signed_twice_area))
}

fn barycentric(point: DVec3, a: DVec3, b: DVec3, c: DVec3) -> [f64; 3] {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = point - a;
    let d00 = v0.dot(v0);
    let d01 = v0.dot(v1);
    let d11 = v1.dot(v1);
    let d20 = v2.dot(v0);
    let d21 = v2.dot(v1);
    let denominator = d00 * d11 - d01 * d01;
    let b_weight = (d11 * d20 - d01 * d21) / denominator;
    let c_weight = (d00 * d21 - d01 * d20) / denominator;
    [1.0 - b_weight - c_weight, b_weight, c_weight]
}

#[cfg(test)]
mod tests {
    use ori3_cp::Face;
    use ori3_model::{Face3D, Frame3D};

    use super::{SupportError, three_point_support, three_point_support_with_tolerance};

    fn face(id: u32, vertices: &[u32]) -> Face {
        Face {
            id,
            vertices: vertices.to_vec(),
            edges: vec![0; vertices.len()],
        }
    }

    fn frame(id: u32, polygon: &[[f64; 3]]) -> Frame3D {
        Frame3D {
            faces: vec![Face3D {
                face: id,
                polygon: polygon.to_vec(),
                layer: 0,
                surface_rank: 0,
                mirrored: false,
            }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn planar_triangle_is_statically_supported() {
        let faces = [face(0, &[10, 11, 12])];
        let frame = frame(0, &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let support = three_point_support(&faces, &frame, [10, 11, 12]).unwrap();
        assert!(support.stable);
        assert!(support.one_sided);
        assert!(support.centroid_projection_inside);
        assert!((support.support_area - 1.0).abs() < 1e-12);
        assert!((support.surface_area - 1.0).abs() < 1e-12);
        assert!(support.centroid_height.abs() < 1e-12);
        assert_eq!(support.plane.origin, [0.0, 0.0, 0.0]);
        assert_eq!(support.plane.normal, [0.0, 0.0, 1.0]);
        for (actual, expected) in support
            .surface_centroid
            .iter()
            .zip([2.0 / 3.0, 1.0 / 3.0, 0.0])
        {
            assert!((actual - expected).abs() < 1e-12);
        }
        for weight in support.centroid_barycentric {
            assert!((weight - 1.0 / 3.0).abs() < 1e-12);
        }
    }

    #[test]
    fn invalid_inputs_are_reported() {
        let faces = [face(0, &[10, 11, 12])];
        let frame = frame(0, &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        assert!(matches!(
            three_point_support_with_tolerance(&faces, &frame, [10, 11, 12], f64::NAN),
            Err(SupportError::InvalidTolerance(value)) if value.is_nan()
        ));
        assert_eq!(
            three_point_support(&faces, &frame, [10, 10, 12]),
            Err(SupportError::DuplicateSupportVertex(10))
        );
        assert_eq!(
            three_point_support(&faces, &frame, [10, 11, 99]),
            Err(SupportError::MissingSupportVertex(99))
        );
    }

    #[test]
    fn collinear_support_triangle_is_rejected() {
        let faces = [face(0, &[10, 11, 12, 13, 14])];
        let frame = frame(
            0,
            &[
                [0.0, 0.0, 0.0],
                [0.5, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        );
        assert_eq!(
            three_point_support(&faces, &frame, [10, 11, 12]),
            Err(SupportError::DegenerateSupportTriangle { twice_area: 0.0 })
        );
    }
}
