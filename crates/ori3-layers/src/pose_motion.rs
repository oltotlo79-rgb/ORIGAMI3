//! High-level construction and persistence of a rigid non-flat pose.
//!
//! Unlike [`crate::apply_pose_step`], this entry point accepts only the hinges
//! that the caller wants to drive.  The rigid solver derives every dependent
//! hinge angle needed to close face-adjacency loops, then the complete solution
//! is persisted as one Pose step so replay is deterministic.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use ori3_cp::{Face, extract_faces};
use ori3_model::{
    Document, Driver, DriverLine, EdgeId, EdgeKind, FaceId, Frame3D, StepId, VertexId,
};
use ori3_rigid::{max_seam_gap, solve};

use crate::{PoseStepInput, apply_pose_step, replay};

const MAX_POSE_SEAM_GAP: f64 = 1e-6;

/// Promotes or changes one exact crease-pattern edge before solving the pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoseEdgeActivation {
    pub edge_id: EdgeId,
    pub kind: EdgeKind,
}

/// A hard driver or a branch-selection hint for one exact hinge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseAngleTarget {
    pub edge_id: EdgeId,
    pub target_angle_deg: f64,
}

/// Inputs for one rigid pose motion.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseMotionInput {
    /// Exact Aux/Mountain/Valley edges to make Mountain or Valley creases.
    pub activations: Vec<PoseEdgeActivation>,
    /// Hard angle constraints. At least one is required.
    pub drivers: Vec<PoseAngleTarget>,
    /// Initial values used only to select a closed kinematic branch.
    pub branch_hints: Vec<PoseAngleTarget>,
    /// Human-readable instruction attached to the persistent Pose step.
    pub note: String,
}

/// Validated result of [`solve_and_apply_pose_step`].
#[derive(Clone, Debug)]
pub struct PoseMotionResult {
    pub step_id: StepId,
    pub frame: Frame3D,
    pub max_seam_gap: f64,
    /// Every two-face hinge angle derived by the rigid solver.
    pub hinge_angles: HashMap<EdgeId, f64>,
}

/// Solves a closed rigid pose and atomically persists all of its hinge angles.
///
/// The caller's document is replaced only after edge activation, replay of the
/// prior sequence, rigid solving, and replay of the appended Pose step all pass
/// their integrity checks.
pub fn solve_and_apply_pose_step(
    document: &mut Document,
    input: PoseMotionInput,
) -> Result<PoseMotionResult, String> {
    let mut candidate = document.clone();
    apply_activations(&mut candidate, &input.activations)?;

    let faces = extract_faces(&candidate.cp);
    if faces.is_empty() {
        return Err("pose motion cannot be applied to a document with no faces".to_string());
    }
    let owners = faces_by_edge(&faces);
    let hinges = hinge_ids(&owners);

    // Activating a crease can change the face graph. Replay the old sequence
    // against that exact candidate CP before choosing the next closed branch.
    let replayed = replay(&candidate, candidate.sequence.len(), 1.0);
    validate_replay(&candidate, &faces, &replayed)?;
    validate_angle_map("replayed start", &replayed.hinge_angles, &hinges)?;

    let drivers = validate_drivers(&candidate, &owners, &input.drivers)?;
    validate_branch_hints(&candidate, &owners, &input.branch_hints)?;

    let mut seed = replayed.hinge_angles;
    for hint in &input.branch_hints {
        seed.insert(hint.edge_id, hint.target_angle_deg);
    }

    let solved = solve(&candidate.cp, &faces, &drivers, Some(&seed));
    if !solved.converged {
        return Err("pose motion rigid solve did not converge".to_string());
    }
    if !solved.frame.warnings.is_empty() {
        return Err(format!(
            "pose motion rigid solve warnings: {:?}",
            solved.frame.warnings
        ));
    }
    validate_frame(&candidate, &faces, &solved.frame, "pose motion rigid solve")?;
    validate_angle_map("pose motion solution", &solved.angles, &hinges)?;

    let driver_updates = driver_lines_for_angles(&candidate, &solved.angles)?;
    let hinge_angles = solved.angles;
    let applied = apply_pose_step(
        &mut candidate,
        PoseStepInput {
            driver_updates,
            note: input.note,
        },
    )?;

    *document = candidate;
    Ok(PoseMotionResult {
        step_id: applied.step_id,
        frame: applied.frame,
        max_seam_gap: applied.max_seam_gap,
        hinge_angles,
    })
}

fn apply_activations(
    document: &mut Document,
    activations: &[PoseEdgeActivation],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for (index, activation) in activations.iter().enumerate() {
        if !seen.insert(activation.edge_id) {
            return Err(format!(
                "pose edge activation {} duplicates edge {}",
                index + 1,
                activation.edge_id
            ));
        }
        if !matches!(activation.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            return Err(format!(
                "pose edge activation {} must target Mountain or Valley",
                index + 1
            ));
        }

        let edge = document
            .cp
            .edges
            .iter_mut()
            .find(|edge| edge.id == activation.edge_id)
            .ok_or_else(|| {
                format!(
                    "pose edge activation {} references missing edge {}",
                    index + 1,
                    activation.edge_id
                )
            })?;
        match edge.kind {
            EdgeKind::Aux | EdgeKind::Mountain | EdgeKind::Valley => {
                edge.kind = activation.kind;
            }
            EdgeKind::Border => {
                return Err(format!(
                    "pose edge activation {} cannot change border edge {}",
                    index + 1,
                    activation.edge_id
                ));
            }
        }
    }
    Ok(())
}

fn validate_drivers(
    document: &Document,
    owners: &BTreeMap<EdgeId, BTreeSet<FaceId>>,
    targets: &[PoseAngleTarget],
) -> Result<Vec<Driver>, String> {
    if targets.is_empty() {
        return Err("pose motion needs at least one hard driver".to_string());
    }

    let mut seen = HashSet::new();
    let mut drivers = Vec::with_capacity(targets.len());
    for (index, target) in targets.iter().enumerate() {
        validate_target(document, owners, target, "driver", index)?;
        if !(-180.0..=180.0).contains(&target.target_angle_deg) {
            return Err(format!(
                "pose driver {} angle {} is outside [-180, 180] degrees",
                index + 1,
                target.target_angle_deg
            ));
        }
        if !seen.insert(target.edge_id) {
            return Err(format!(
                "pose driver {} duplicates edge {}",
                index + 1,
                target.edge_id
            ));
        }
        drivers.push(Driver {
            hinge: target.edge_id,
            target_angle_deg: target.target_angle_deg,
        });
    }
    Ok(drivers)
}

fn validate_branch_hints(
    document: &Document,
    owners: &BTreeMap<EdgeId, BTreeSet<FaceId>>,
    targets: &[PoseAngleTarget],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for (index, target) in targets.iter().enumerate() {
        validate_target(document, owners, target, "branch hint", index)?;
        if !seen.insert(target.edge_id) {
            return Err(format!(
                "pose branch hint {} duplicates edge {}",
                index + 1,
                target.edge_id
            ));
        }
    }
    Ok(())
}

fn validate_target(
    document: &Document,
    owners: &BTreeMap<EdgeId, BTreeSet<FaceId>>,
    target: &PoseAngleTarget,
    label: &str,
    index: usize,
) -> Result<(), String> {
    if !target.target_angle_deg.is_finite() {
        return Err(format!("pose {label} {} has a non-finite angle", index + 1));
    }
    let edge = document
        .cp
        .edges
        .iter()
        .find(|edge| edge.id == target.edge_id)
        .ok_or_else(|| {
            format!(
                "pose {label} {} references missing edge {}",
                index + 1,
                target.edge_id
            )
        })?;
    if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
        return Err(format!(
            "pose {label} {} edge {} is not an active crease",
            index + 1,
            target.edge_id
        ));
    }
    if owners
        .get(&target.edge_id)
        .is_none_or(|adjacent| adjacent.len() != 2)
    {
        return Err(format!(
            "pose {label} {} edge {} is not a two-face hinge",
            index + 1,
            target.edge_id
        ));
    }
    Ok(())
}

fn validate_replay(
    document: &Document,
    faces: &[Face],
    replayed: &crate::ReplayResult,
) -> Result<(), String> {
    if !replayed.skipped.is_empty() {
        return Err(format!(
            "pose motion start replay skipped steps: {:?}",
            replayed.skipped
        ));
    }
    if !replayed.warnings.is_empty() {
        return Err(format!(
            "pose motion start replay warnings: {:?}",
            replayed.warnings
        ));
    }
    if !replayed.frame.warnings.is_empty() {
        return Err(format!(
            "pose motion start frame warnings: {:?}",
            replayed.frame.warnings
        ));
    }
    validate_frame(document, faces, &replayed.frame, "pose motion start replay")?;
    Ok(())
}

fn validate_frame(
    document: &Document,
    faces: &[Face],
    frame: &Frame3D,
    label: &str,
) -> Result<f64, String> {
    let mut expected = faces
        .iter()
        .map(|face| (face.id, face.vertices.len()))
        .collect::<BTreeMap<_, _>>();
    for face in &frame.faces {
        let Some(vertex_count) = expected.remove(&face.face) else {
            return Err(format!(
                "{label} contains duplicate or unexpected face {}",
                face.face
            ));
        };
        if face.polygon.len() != vertex_count {
            return Err(format!(
                "{label} face {} is incomplete: expected {} vertices, got {}",
                face.face,
                vertex_count,
                face.polygon.len()
            ));
        }
        if face
            .polygon
            .iter()
            .flatten()
            .any(|coordinate| !coordinate.is_finite())
        {
            return Err(format!("{label} contains a non-finite frame coordinate"));
        }
    }
    if !expected.is_empty() {
        return Err(format!(
            "{label} lost faces: expected {}, got {}",
            faces.len(),
            frame.faces.len()
        ));
    }

    let gap = max_seam_gap(&document.cp, faces, frame);
    if !gap.is_finite() || gap >= MAX_POSE_SEAM_GAP {
        return Err(format!(
            "{label} max_seam_gap={gap:.3e}, expected < {MAX_POSE_SEAM_GAP:.1e}"
        ));
    }
    Ok(gap)
}

fn validate_angle_map(
    label: &str,
    angles: &HashMap<EdgeId, f64>,
    hinges: &BTreeSet<EdgeId>,
) -> Result<(), String> {
    let actual = angles.keys().copied().collect::<BTreeSet<_>>();
    if actual != *hinges {
        return Err(format!(
            "{label} has incomplete hinge angles: expected {:?}, got {:?}",
            hinges, actual
        ));
    }
    if angles.values().any(|angle| !angle.is_finite()) {
        return Err(format!("{label} contains a non-finite hinge angle"));
    }
    Ok(())
}

fn driver_lines_for_angles(
    document: &Document,
    angles: &HashMap<EdgeId, f64>,
) -> Result<Vec<DriverLine>, String> {
    let vertices = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<HashMap<VertexId, [f64; 2]>>();
    let edges = document
        .cp
        .edges
        .iter()
        .map(|edge| (edge.id, edge))
        .collect::<HashMap<_, _>>();

    let mut sorted = angles
        .iter()
        .map(|(&id, &angle)| (id, angle))
        .collect::<Vec<_>>();
    sorted.sort_by_key(|&(id, _)| id);
    sorted
        .into_iter()
        .map(|(edge_id, target_angle_deg)| {
            let edge = edges
                .get(&edge_id)
                .ok_or_else(|| format!("pose solution references missing edge {edge_id}"))?;
            let a = vertices.get(&edge.v0).copied().ok_or_else(|| {
                format!(
                    "pose solution edge {edge_id} references missing vertex {}",
                    edge.v0
                )
            })?;
            let b = vertices.get(&edge.v1).copied().ok_or_else(|| {
                format!(
                    "pose solution edge {edge_id} references missing vertex {}",
                    edge.v1
                )
            })?;
            Ok(DriverLine {
                a,
                b,
                target_angle_deg,
            })
        })
        .collect()
}

fn faces_by_edge(faces: &[Face]) -> BTreeMap<EdgeId, BTreeSet<FaceId>> {
    let mut owners = BTreeMap::<EdgeId, BTreeSet<FaceId>>::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().insert(face.id);
        }
    }
    owners
}

fn hinge_ids(owners: &BTreeMap<EdgeId, BTreeSet<FaceId>>) -> BTreeSet<EdgeId> {
    owners
        .iter()
        .filter_map(|(&edge, faces)| (faces.len() == 2).then_some(edge))
        .collect()
}
