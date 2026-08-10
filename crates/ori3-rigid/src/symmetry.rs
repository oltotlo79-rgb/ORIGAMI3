//! Reflection-symmetric constrained rigid-fold solving.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use glam::DVec2;
use ori3_cp::Face;
use ori3_model::{CreasePattern, Driver, EPS, EdgeId, EdgeKind, VertexId};

use crate::solver::{SolveResult, solve_near};

/// Maximum mismatch allowed between the angles of mirrored hinges.
pub const DEFAULT_REFLECTION_ANGLE_TOLERANCE_DEG: f64 = 1e-9;
/// Maximum number of angle-projection/warm-resolve passes.
pub const DEFAULT_REFLECTION_PROJECTIONS: u32 = 12;

/// Result of [`solve_near_with_reflection_symmetry`].
#[derive(Clone, Debug)]
pub struct ReflectionSymmetrySolveResult {
    pub result: SolveResult,
    /// Vertex reflection map. The map is an involution and includes fixed vertices.
    pub mirrored_vertices: HashMap<VertexId, VertexId>,
    /// Edge reflection map. The map is an involution and includes fixed edges.
    pub mirrored_edges: HashMap<EdgeId, EdgeId>,
    /// Number of average-angle projection passes after the initial `solve_near`.
    pub projection_iterations: u32,
    pub max_mirrored_angle_error_deg: f64,
}

/// A crease pattern or driver set that is not reflection-symmetric.
#[derive(Clone, Debug, PartialEq)]
pub enum ReflectionSymmetryError {
    InvalidAxis {
        a: [f64; 2],
        b: [f64; 2],
    },
    MissingMirroredVertex {
        vertex: VertexId,
        reflected_position: [f64; 2],
    },
    AmbiguousMirroredVertex {
        vertex: VertexId,
        reflected_position: [f64; 2],
        candidates: Vec<VertexId>,
    },
    NonInvolutiveVertexMap {
        vertex: VertexId,
        mirror: VertexId,
        mirror_of_mirror: VertexId,
    },
    MissingMirroredEdge {
        edge: EdgeId,
        reflected_vertices: [VertexId; 2],
    },
    AmbiguousMirroredEdge {
        edge: EdgeId,
        reflected_vertices: [VertexId; 2],
        candidates: Vec<EdgeId>,
    },
    MirroredEdgeKindMismatch {
        edge: EdgeId,
        mirror: EdgeId,
        kind: EdgeKind,
        mirrored_kind: EdgeKind,
    },
    NonInvolutiveEdgeMap {
        edge: EdgeId,
        mirror: EdgeId,
        mirror_of_mirror: EdgeId,
    },
    UnknownHardDriver(EdgeId),
    NonFiniteHardDriver {
        edge: EdgeId,
        angle_deg: f64,
    },
    ConflictingHardDrivers {
        edge: EdgeId,
        first_angle_deg: f64,
        second_angle_deg: f64,
    },
    MissingMirroredHardDriver {
        edge: EdgeId,
        mirror: EdgeId,
    },
    MirroredHardDriverAngleMismatch {
        edge: EdgeId,
        mirror: EdgeId,
        angle_deg: f64,
        mirrored_angle_deg: f64,
    },
    UnknownTarget(EdgeId),
    NonFiniteTarget {
        edge: EdgeId,
        angle_deg: f64,
    },
    UnknownWarmStart(EdgeId),
    NonFiniteWarmStart {
        edge: EdgeId,
        angle_deg: f64,
    },
    MissingMirroredHinge {
        edge: EdgeId,
        mirror: EdgeId,
    },
    DidNotConverge {
        projection_iterations: u32,
        solver_converged: bool,
        max_mirrored_angle_error_deg: f64,
    },
}

impl fmt::Display for ReflectionSymmetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAxis { a, b } => write!(f, "invalid reflection axis {a:?} -> {b:?}"),
            Self::MissingMirroredVertex {
                vertex,
                reflected_position,
            } => write!(f, "vertex {vertex} has no mirror at {reflected_position:?}"),
            Self::AmbiguousMirroredVertex {
                vertex,
                reflected_position,
                candidates,
            } => write!(
                f,
                "vertex {vertex} has ambiguous mirrors at {reflected_position:?}: {candidates:?}"
            ),
            Self::NonInvolutiveVertexMap {
                vertex,
                mirror,
                mirror_of_mirror,
            } => write!(
                f,
                "vertex reflection is not involutive: {vertex} -> {mirror} -> {mirror_of_mirror}"
            ),
            Self::MissingMirroredEdge {
                edge,
                reflected_vertices,
            } => write!(
                f,
                "edge {edge} has no mirror between vertices {reflected_vertices:?}"
            ),
            Self::AmbiguousMirroredEdge {
                edge,
                reflected_vertices,
                candidates,
            } => write!(
                f,
                "edge {edge} has ambiguous mirrors between {reflected_vertices:?}: {candidates:?}"
            ),
            Self::MirroredEdgeKindMismatch {
                edge,
                mirror,
                kind,
                mirrored_kind,
            } => write!(
                f,
                "mirrored edges {edge}/{mirror} have different kinds: {kind:?}/{mirrored_kind:?}"
            ),
            Self::NonInvolutiveEdgeMap {
                edge,
                mirror,
                mirror_of_mirror,
            } => write!(
                f,
                "edge reflection is not involutive: {edge} -> {mirror} -> {mirror_of_mirror}"
            ),
            Self::UnknownHardDriver(edge) => write!(f, "hard driver edge {edge} is not in the CP"),
            Self::NonFiniteHardDriver { edge, angle_deg } => {
                write!(
                    f,
                    "hard driver edge {edge} has non-finite angle {angle_deg}"
                )
            }
            Self::ConflictingHardDrivers {
                edge,
                first_angle_deg,
                second_angle_deg,
            } => write!(
                f,
                "hard driver edge {edge} has conflicting angles {first_angle_deg}/{second_angle_deg}"
            ),
            Self::MissingMirroredHardDriver { edge, mirror } => {
                write!(
                    f,
                    "hard driver edge {edge} is missing mirrored driver {mirror}"
                )
            }
            Self::MirroredHardDriverAngleMismatch {
                edge,
                mirror,
                angle_deg,
                mirrored_angle_deg,
            } => write!(
                f,
                "mirrored hard drivers {edge}/{mirror} have different angles {angle_deg}/{mirrored_angle_deg}"
            ),
            Self::UnknownTarget(edge) => write!(f, "target edge {edge} is not in the CP"),
            Self::NonFiniteTarget { edge, angle_deg } => {
                write!(f, "target edge {edge} has non-finite angle {angle_deg}")
            }
            Self::UnknownWarmStart(edge) => write!(f, "warm-start edge {edge} is not in the CP"),
            Self::NonFiniteWarmStart { edge, angle_deg } => {
                write!(f, "warm-start edge {edge} has non-finite angle {angle_deg}")
            }
            Self::MissingMirroredHinge { edge, mirror } => write!(
                f,
                "solved hinge {edge} has mirrored CP edge {mirror}, but that edge is not a hinge"
            ),
            Self::DidNotConverge {
                projection_iterations,
                solver_converged,
                max_mirrored_angle_error_deg,
            } => write!(
                f,
                "reflection-symmetric solve did not converge after {projection_iterations} projections (solver_converged={solver_converged}, angle_error={max_mirrored_angle_error_deg})"
            ),
        }
    }
}

impl Error for ReflectionSymmetryError {}

/// Solve near target angles while enforcing reflection-symmetric hinge angles.
///
/// The entire CP must be invariant under reflection about `axis`. Every hard
/// driver must have a mirrored hard driver with the same angle. After the first
/// [`solve_near`] call, each mirrored hinge pair is averaged into a new target,
/// and that target is solved from the previous result as a warm start. This is
/// repeated until the exact loop-closure solve and the reflection constraint
/// both converge.
pub fn solve_near_with_reflection_symmetry(
    cp: &CreasePattern,
    faces: &[Face],
    hard_drivers: &[Driver],
    targets: &HashMap<EdgeId, f64>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    axis: [[f64; 2]; 2],
) -> Result<ReflectionSymmetrySolveResult, ReflectionSymmetryError> {
    let mirrored_vertices = build_vertex_reflection(cp, axis)?;
    let mirrored_edges = build_edge_reflection(cp, &mirrored_vertices)?;
    validate_angle_map(cp, targets, false)?;
    if let Some(warm_start) = warm_start {
        validate_angle_map(cp, warm_start, true)?;
    }
    validate_hard_drivers(hard_drivers, &mirrored_edges)?;

    let mut result = solve_near(cp, faces, hard_drivers, targets, warm_start);
    let mut angle_error = mirrored_angle_error(&result.angles, &mirrored_edges)?;
    if result.converged && angle_error <= DEFAULT_REFLECTION_ANGLE_TOLERANCE_DEG {
        return Ok(ReflectionSymmetrySolveResult {
            result,
            mirrored_vertices,
            mirrored_edges,
            projection_iterations: 0,
            max_mirrored_angle_error_deg: angle_error,
        });
    }

    for projection_iterations in 1..=DEFAULT_REFLECTION_PROJECTIONS {
        let projected_targets = average_mirrored_angles(&result.angles, &mirrored_edges)?;
        result = solve_near(
            cp,
            faces,
            hard_drivers,
            &projected_targets,
            Some(&result.angles),
        );
        angle_error = mirrored_angle_error(&result.angles, &mirrored_edges)?;
        if result.converged && angle_error <= DEFAULT_REFLECTION_ANGLE_TOLERANCE_DEG {
            return Ok(ReflectionSymmetrySolveResult {
                result,
                mirrored_vertices,
                mirrored_edges,
                projection_iterations,
                max_mirrored_angle_error_deg: angle_error,
            });
        }
    }

    Err(ReflectionSymmetryError::DidNotConverge {
        projection_iterations: DEFAULT_REFLECTION_PROJECTIONS,
        solver_converged: result.converged,
        max_mirrored_angle_error_deg: angle_error,
    })
}

fn reflect_point(point: DVec2, a: DVec2, b: DVec2) -> DVec2 {
    let direction = b - a;
    let projection = a + direction * ((point - a).dot(direction) / direction.length_squared());
    projection * 2.0 - point
}

fn build_vertex_reflection(
    cp: &CreasePattern,
    axis: [[f64; 2]; 2],
) -> Result<HashMap<VertexId, VertexId>, ReflectionSymmetryError> {
    let a = DVec2::from(axis[0]);
    let b = DVec2::from(axis[1]);
    if !a.is_finite() || !b.is_finite() || (b - a).length() < EPS {
        return Err(ReflectionSymmetryError::InvalidAxis {
            a: axis[0],
            b: axis[1],
        });
    }

    let mut mirrored = HashMap::with_capacity(cp.vertices.len());
    for vertex in &cp.vertices {
        let reflected = reflect_point(DVec2::from(vertex.pos), a, b);
        let mut candidates: Vec<VertexId> = cp
            .vertices
            .iter()
            .filter(|candidate| (DVec2::from(candidate.pos) - reflected).length() <= EPS)
            .map(|candidate| candidate.id)
            .collect();
        candidates.sort_unstable();
        match candidates.as_slice() {
            [] => {
                return Err(ReflectionSymmetryError::MissingMirroredVertex {
                    vertex: vertex.id,
                    reflected_position: reflected.to_array(),
                });
            }
            &[mirror] => {
                mirrored.insert(vertex.id, mirror);
            }
            _ => {
                return Err(ReflectionSymmetryError::AmbiguousMirroredVertex {
                    vertex: vertex.id,
                    reflected_position: reflected.to_array(),
                    candidates,
                });
            }
        }
    }
    for (&vertex, &mirror) in &mirrored {
        let mirror_of_mirror = mirrored[&mirror];
        if mirror_of_mirror != vertex {
            return Err(ReflectionSymmetryError::NonInvolutiveVertexMap {
                vertex,
                mirror,
                mirror_of_mirror,
            });
        }
    }
    Ok(mirrored)
}

fn build_edge_reflection(
    cp: &CreasePattern,
    mirrored_vertices: &HashMap<VertexId, VertexId>,
) -> Result<HashMap<EdgeId, EdgeId>, ReflectionSymmetryError> {
    let mut by_vertices = HashMap::<(VertexId, VertexId), Vec<EdgeId>>::new();
    for edge in &cp.edges {
        let key = ordered_pair(edge.v0, edge.v1);
        by_vertices.entry(key).or_default().push(edge.id);
    }
    let kinds: HashMap<EdgeId, EdgeKind> =
        cp.edges.iter().map(|edge| (edge.id, edge.kind)).collect();
    let mut mirrored = HashMap::with_capacity(cp.edges.len());
    for edge in &cp.edges {
        let reflected_vertices = [mirrored_vertices[&edge.v0], mirrored_vertices[&edge.v1]];
        let key = ordered_pair(reflected_vertices[0], reflected_vertices[1]);
        let candidates = by_vertices.get(&key).cloned().unwrap_or_default();
        let mirror = match candidates.as_slice() {
            [] => {
                return Err(ReflectionSymmetryError::MissingMirroredEdge {
                    edge: edge.id,
                    reflected_vertices,
                });
            }
            &[mirror] => mirror,
            _ => {
                return Err(ReflectionSymmetryError::AmbiguousMirroredEdge {
                    edge: edge.id,
                    reflected_vertices,
                    candidates,
                });
            }
        };
        if kinds[&mirror] != edge.kind {
            return Err(ReflectionSymmetryError::MirroredEdgeKindMismatch {
                edge: edge.id,
                mirror,
                kind: edge.kind,
                mirrored_kind: kinds[&mirror],
            });
        }
        mirrored.insert(edge.id, mirror);
    }
    for (&edge, &mirror) in &mirrored {
        let mirror_of_mirror = mirrored[&mirror];
        if mirror_of_mirror != edge {
            return Err(ReflectionSymmetryError::NonInvolutiveEdgeMap {
                edge,
                mirror,
                mirror_of_mirror,
            });
        }
    }
    Ok(mirrored)
}

fn ordered_pair(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn validate_angle_map(
    cp: &CreasePattern,
    values: &HashMap<EdgeId, f64>,
    warm_start: bool,
) -> Result<(), ReflectionSymmetryError> {
    for (&edge, &angle_deg) in values {
        if !cp.edges.iter().any(|candidate| candidate.id == edge) {
            return Err(if warm_start {
                ReflectionSymmetryError::UnknownWarmStart(edge)
            } else {
                ReflectionSymmetryError::UnknownTarget(edge)
            });
        }
        if !angle_deg.is_finite() {
            return Err(if warm_start {
                ReflectionSymmetryError::NonFiniteWarmStart { edge, angle_deg }
            } else {
                ReflectionSymmetryError::NonFiniteTarget { edge, angle_deg }
            });
        }
    }
    Ok(())
}

fn validate_hard_drivers(
    hard_drivers: &[Driver],
    mirrored_edges: &HashMap<EdgeId, EdgeId>,
) -> Result<(), ReflectionSymmetryError> {
    let mut angles = HashMap::<EdgeId, f64>::new();
    for driver in hard_drivers {
        if !mirrored_edges.contains_key(&driver.hinge) {
            return Err(ReflectionSymmetryError::UnknownHardDriver(driver.hinge));
        }
        if !driver.target_angle_deg.is_finite() {
            return Err(ReflectionSymmetryError::NonFiniteHardDriver {
                edge: driver.hinge,
                angle_deg: driver.target_angle_deg,
            });
        }
        if let Some(first_angle_deg) = angles.insert(driver.hinge, driver.target_angle_deg)
            && (first_angle_deg - driver.target_angle_deg).abs()
                > DEFAULT_REFLECTION_ANGLE_TOLERANCE_DEG
        {
            return Err(ReflectionSymmetryError::ConflictingHardDrivers {
                edge: driver.hinge,
                first_angle_deg,
                second_angle_deg: driver.target_angle_deg,
            });
        }
    }
    for (&edge, &angle_deg) in &angles {
        let mirror = mirrored_edges[&edge];
        let Some(&mirrored_angle_deg) = angles.get(&mirror) else {
            return Err(ReflectionSymmetryError::MissingMirroredHardDriver { edge, mirror });
        };
        if (angle_deg - mirrored_angle_deg).abs() > DEFAULT_REFLECTION_ANGLE_TOLERANCE_DEG {
            return Err(ReflectionSymmetryError::MirroredHardDriverAngleMismatch {
                edge,
                mirror,
                angle_deg,
                mirrored_angle_deg,
            });
        }
    }
    Ok(())
}

fn average_mirrored_angles(
    angles: &HashMap<EdgeId, f64>,
    mirrored_edges: &HashMap<EdgeId, EdgeId>,
) -> Result<HashMap<EdgeId, f64>, ReflectionSymmetryError> {
    angles
        .iter()
        .map(|(&edge, &angle)| {
            let mirror = mirrored_edges[&edge];
            let mirrored_angle = angles
                .get(&mirror)
                .copied()
                .ok_or(ReflectionSymmetryError::MissingMirroredHinge { edge, mirror })?;
            Ok((edge, 0.5 * (angle + mirrored_angle)))
        })
        .collect()
}

fn mirrored_angle_error(
    angles: &HashMap<EdgeId, f64>,
    mirrored_edges: &HashMap<EdgeId, EdgeId>,
) -> Result<f64, ReflectionSymmetryError> {
    let mut worst = 0.0_f64;
    for (&edge, &angle) in angles {
        let mirror = mirrored_edges[&edge];
        let mirrored_angle = angles
            .get(&mirror)
            .copied()
            .ok_or(ReflectionSymmetryError::MissingMirroredHinge { edge, mirror })?;
        worst = worst.max((angle - mirrored_angle).abs());
    }
    Ok(worst)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ori3_cp::{extract_faces, insert_segment};
    use ori3_model::{Document, Driver, EdgeKind, Paper};

    use super::{ReflectionSymmetryError, solve_near_with_reflection_symmetry};

    fn symmetric_parallel_cp() -> (ori3_model::CreasePattern, [u32; 2]) {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        insert_segment(&mut document.cp, [0.25, 0.0], [0.25, 1.0], EdgeKind::Valley);
        insert_segment(&mut document.cp, [0.75, 0.0], [0.75, 1.0], EdgeKind::Valley);
        let mut hinges: Vec<_> = document
            .cp
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Valley)
            .map(|edge| edge.id)
            .collect();
        hinges.sort_unstable();
        (document.cp, [hinges[0], hinges[1]])
    }

    #[test]
    fn symmetric_hard_drivers_solve_without_projection() {
        let (cp, hinges) = symmetric_parallel_cp();
        let faces = extract_faces(&cp);
        let drivers: Vec<_> = hinges
            .into_iter()
            .map(|hinge| Driver {
                hinge,
                target_angle_deg: -60.0,
            })
            .collect();
        let solved = solve_near_with_reflection_symmetry(
            &cp,
            &faces,
            &drivers,
            &HashMap::new(),
            None,
            [[0.5, 0.0], [0.5, 1.0]],
        )
        .unwrap();
        assert!(solved.result.converged);
        assert_eq!(solved.projection_iterations, 0);
        assert!(solved.max_mirrored_angle_error_deg < 1e-12);
    }

    #[test]
    fn missing_or_different_mirrored_hard_driver_is_rejected() {
        let (cp, hinges) = symmetric_parallel_cp();
        let faces = extract_faces(&cp);
        let missing = solve_near_with_reflection_symmetry(
            &cp,
            &faces,
            &[Driver {
                hinge: hinges[0],
                target_angle_deg: -60.0,
            }],
            &HashMap::new(),
            None,
            [[0.5, 0.0], [0.5, 1.0]],
        );
        assert!(matches!(
            missing,
            Err(ReflectionSymmetryError::MissingMirroredHardDriver { .. })
        ));

        let different = solve_near_with_reflection_symmetry(
            &cp,
            &faces,
            &[
                Driver {
                    hinge: hinges[0],
                    target_angle_deg: -60.0,
                },
                Driver {
                    hinge: hinges[1],
                    target_angle_deg: -45.0,
                },
            ],
            &HashMap::new(),
            None,
            [[0.5, 0.0], [0.5, 1.0]],
        );
        assert!(matches!(
            different,
            Err(ReflectionSymmetryError::MirroredHardDriverAngleMismatch { .. })
        ));
    }

    #[test]
    fn invalid_axis_and_asymmetric_cp_are_rejected() {
        let (mut cp, _) = symmetric_parallel_cp();
        let faces = extract_faces(&cp);
        let invalid_axis = solve_near_with_reflection_symmetry(
            &cp,
            &faces,
            &[],
            &HashMap::new(),
            None,
            [[0.5, 0.5], [0.5, 0.5]],
        );
        assert!(matches!(
            invalid_axis,
            Err(ReflectionSymmetryError::InvalidAxis { .. })
        ));

        cp.vertices[0].pos[0] += 0.01;
        let asymmetric = solve_near_with_reflection_symmetry(
            &cp,
            &faces,
            &[],
            &HashMap::new(),
            None,
            [[0.5, 0.0], [0.5, 1.0]],
        );
        assert!(matches!(
            asymmetric,
            Err(ReflectionSymmetryError::MissingMirroredVertex { .. })
        ));
    }
}
