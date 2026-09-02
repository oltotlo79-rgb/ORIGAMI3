//! Transactional persistence for non-flat pose updates.

use std::collections::{HashMap, HashSet};

use ori3_cp::extract_faces;
use ori3_model::{Document, DriverLine, EPS, FoldStep, Frame3D, StepId, TechniqueKind};
use ori3_rigid::max_seam_gap;

use crate::fold_through::{faces_by_edge, resolve_driver_edges};
use crate::replay::replay;

const MAX_POSE_SEAM_GAP: f64 = 1e-6;
const ANGLE_EPS: f64 = 1e-9;

/// Updates persisted by one [`TechniqueKind::Pose`] step.
#[derive(Clone, Debug)]
pub struct PoseStepInput {
    /// Only these logical crease lines are updated. Angles assigned by all
    /// earlier flat and Pose steps remain active during replay.
    pub driver_updates: Vec<DriverLine>,
    /// Human-readable instruction attached to the persistent step.
    pub note: String,
}

/// Validated result of [`apply_pose_step`].
#[derive(Clone, Debug)]
pub struct PoseStepResult {
    /// ID assigned to the appended Pose step.
    pub step_id: StepId,
    /// Fully replayed completed frame, including every paper face.
    pub frame: Frame3D,
    /// Largest normalized separation across any shared crease.
    pub max_seam_gap: f64,
}

/// Atomically append and validate one persistent non-flat Pose step.
///
/// Driver lines are resolved before mutation. The candidate document is then
/// fully replayed; a missing/skipped command, warning, lost face, non-finite
/// coordinate, or seam gap at least `1e-6` rejects the update and leaves the
/// caller's document unchanged.
pub fn apply_pose_step(
    document: &mut Document,
    input: PoseStepInput,
) -> Result<PoseStepResult, String> {
    let faces = extract_faces(&document.cp);
    if faces.is_empty() {
        return Err("pose step cannot be applied to a document with no faces".to_string());
    }
    validate_driver_updates(document, &faces, &input.driver_updates)?;

    let step_id = u32::try_from(document.sequence.len())
        .map_err(|_| "fold sequence length does not fit in a step ID".to_string())?;
    if document.sequence.iter().any(|step| step.id == step_id) {
        return Err(format!("pose step ID {step_id} is already in use"));
    }

    let mut candidate = document.clone();
    candidate.sequence.push(FoldStep {
        id: step_id,
        kind: TechniqueKind::Pose,
        drivers: input.driver_updates,
        layer_order: None,
        alignment: None,
        curved_inside_reverse: None,
        finish_soft: None,
        note: input.note,
    });

    let replayed = replay(&candidate, candidate.sequence.len(), 1.0);
    if !replayed.skipped.is_empty() {
        return Err(format!(
            "pose candidate skipped steps: {:?}",
            replayed.skipped
        ));
    }
    if !replayed.warnings.is_empty() {
        return Err(format!(
            "pose candidate replay warnings: {:?}",
            replayed.warnings
        ));
    }
    if !replayed.frame.warnings.is_empty() {
        return Err(format!(
            "pose candidate frame warnings: {:?}",
            replayed.frame.warnings
        ));
    }

    let expected = faces.iter().map(|face| face.id).collect::<HashSet<_>>();
    let actual = replayed
        .frame
        .faces
        .iter()
        .map(|face| face.face)
        .collect::<HashSet<_>>();
    if replayed.frame.faces.len() != faces.len() || actual != expected {
        return Err(format!(
            "pose candidate lost faces: expected {}, got {}",
            faces.len(),
            replayed.frame.faces.len()
        ));
    }
    if replayed.frame.faces.iter().any(|face| {
        face.polygon
            .iter()
            .flatten()
            .any(|coordinate| !coordinate.is_finite())
    }) {
        return Err("pose candidate contains a non-finite frame coordinate".to_string());
    }

    let gap = max_seam_gap(&candidate.cp, &faces, &replayed.frame);
    if !gap.is_finite() || gap >= MAX_POSE_SEAM_GAP {
        return Err(format!(
            "pose candidate max_seam_gap={gap:.3e}, expected < {MAX_POSE_SEAM_GAP:.1e}"
        ));
    }

    let frame = replayed.frame;
    *document = candidate;
    Ok(PoseStepResult {
        step_id,
        frame,
        max_seam_gap: gap,
    })
}

fn validate_driver_updates(
    document: &Document,
    faces: &[ori3_cp::Face],
    updates: &[DriverLine],
) -> Result<(), String> {
    if updates.is_empty() {
        return Err("pose step needs at least one driver update".to_string());
    }

    let owners = faces_by_edge(faces);
    let mut claimed: HashMap<u32, (usize, f64)> = HashMap::new();
    for (index, update) in updates.iter().enumerate() {
        let values = [
            update.a[0],
            update.a[1],
            update.b[0],
            update.b[1],
            update.target_angle_deg,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "pose driver update {} contains a non-finite value",
                index + 1
            ));
        }
        let dx = update.b[0] - update.a[0];
        let dy = update.b[1] - update.a[1];
        if dx.hypot(dy) <= EPS {
            return Err(format!(
                "pose driver update {} has coincident endpoints",
                index + 1
            ));
        }

        let edges = resolve_driver_edges(&document.cp, update);
        if edges.is_empty() {
            return Err(format!(
                "pose driver update {} does not resolve to a crease",
                index + 1
            ));
        }
        for edge in edges {
            if owners.get(&edge).is_none_or(|adjacent| adjacent.len() != 2) {
                return Err(format!(
                    "pose driver update {} resolves edge {edge}, which is not a two-face hinge",
                    index + 1
                ));
            }
            if let Some(&(previous, angle)) = claimed.get(&edge) {
                if (angle - update.target_angle_deg).abs() > ANGLE_EPS {
                    return Err(format!(
                        "pose driver updates {} and {} assign conflicting angles to hinge {edge}",
                        previous + 1,
                        index + 1
                    ));
                }
                return Err(format!(
                    "pose driver updates {} and {} duplicate hinge {edge}",
                    previous + 1,
                    index + 1
                ));
            }
            claimed.insert(edge, (index, update.target_angle_deg));
        }
    }
    Ok(())
}
