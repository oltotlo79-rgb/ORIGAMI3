//! High-level construction and persistence of a rigid non-flat pose.
//!
//! Unlike [`crate::apply_pose_step`], this entry point accepts only the hinges
//! that the caller wants to drive.  The rigid solver derives every dependent
//! hinge angle needed to close face-adjacency loops, then the complete solution
//! is persisted as one Pose step so replay is deterministic.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces};
use ori3_model::{
    Document, Driver, DriverLine, EdgeId, EdgeKind, FaceId, FoldStep, Frame3D, StepId,
    TechniqueKind, VertexId,
};
use ori3_rigid::{
    FoldedFrame, max_seam_gap, propagate, solve, solve_near, solve_near_exact, to_frame3d,
};

use crate::{FlatState, PoseStepInput, apply_pose_step, flat_state_at, replay};

const MAX_POSE_SEAM_GAP: f64 = 1e-6;
const FLAT_APPROACH_DEGREES: f64 = 1.0;
const FLAT_ANGLE_TOLERANCE_DEGREES: f64 = 2.0;
const MAX_FLAT_CONTINUATION_DELTA_DEGREES: f64 = 1.0;
const FLAT_TARGET_EPS: f64 = 1e-9;
const DEPTH_ORDER_EPS: f64 = MAX_POSE_SEAM_GAP;
const OVERLAP_AREA_EPS: f64 = 1e-14;

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

struct ReplayPoseStart {
    angles: HashMap<EdgeId, f64>,
    preferred: HashMap<EdgeId, f64>,
}

/// Inputs for a rigid motion whose exact endpoint is flat.
///
/// A flat endpoint is singular for the loop-closure solver.  The hard drivers
/// therefore name exact `0`/`+180`/`-180` degree endpoints while the
/// implementation approaches the same branch from one degree away, snaps the
/// complete closed solution to exact flat angles, and validates the snapped
/// frame before persisting it.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatPoseMotionInput {
    /// Exact Aux/Mountain/Valley edges to make Mountain or Valley creases.
    pub activations: Vec<PoseEdgeActivation>,
    /// Exact flat hard-driver targets.  Every angle must be 0 or +/-180 degrees.
    pub drivers: Vec<PoseAngleTarget>,
    /// Optional hints used only to choose the approaching kinematic branch.
    pub branch_hints: Vec<PoseAngleTarget>,
    /// Human-readable instruction attached to the persistent flat step.
    pub note: String,
}

/// Validated result of [`solve_and_apply_flat_pose_step`].
#[derive(Clone, Debug)]
pub struct FlatPoseMotionResult {
    pub step_id: StepId,
    pub frame: Frame3D,
    pub max_seam_gap: f64,
    /// Every two-face hinge angle, snapped to exactly 0 or +/-180 degrees.
    pub hinge_angles: HashMap<EdgeId, f64>,
    pub state: FlatState,
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
    let ReplayPoseStart {
        angles: start_angles,
        preferred: previous_targets,
    } = replay_pose_start(document, &candidate, &faces, &hinges)?;

    let drivers = validate_drivers(&candidate, &owners, &input.drivers)?;
    validate_branch_hints(&candidate, &owners, &input.branch_hints)?;

    let mut seed = start_angles.clone();
    for hint in &input.branch_hints {
        seed.insert(hint.edge_id, hint.target_angle_deg);
    }

    // Explicit branch hints describe the outgoing branch at a flat singularity.  Follow that
    // warm-started hard solve first; pulling every old flat angle toward its saved target can
    // select a different closed mechanism.  Without branch hints, the priority solve remains the
    // stable default and preserves the previously displayed pose.
    let mut solved = if input.branch_hints.is_empty() {
        solve_near(
            &candidate.cp,
            &faces,
            &drivers,
            &previous_targets,
            Some(&seed),
        )
    } else {
        solve(&candidate.cp, &faces, &drivers, Some(&seed))
    };
    if solved.converged && !input.branch_hints.is_empty() {
        let refined = solve_near_exact(
            &candidate.cp,
            &faces,
            &drivers,
            &previous_targets,
            Some(&solved.angles),
        );
        if refined.converged {
            solved = refined;
        }
    } else if !solved.converged && !input.branch_hints.is_empty() {
        solved = solve_near_exact(
            &candidate.cp,
            &faces,
            &drivers,
            &previous_targets,
            Some(&seed),
        );
    }
    if !solved.converged {
        return Err(format!(
            "pose motion rigid solve did not converge: closure_rms={:.3e}, iterations={}, warnings={:?}",
            solved.closure_rms, solved.iterations, solved.frame.warnings
        ));
    }
    if !solved.frame.warnings.is_empty() {
        return Err(format!(
            "pose motion rigid solve warnings: {:?}",
            solved.frame.warnings
        ));
    }
    validate_frame(&candidate, &faces, &solved.frame, "pose motion rigid solve")?;
    validate_angle_map("pose motion solution", &solved.angles, &hinges)?;

    preserve_flat_boundary_signs(&mut solved.angles, &start_angles);

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

/// Keep the persisted sign of an unchanged flat hinge.
///
/// `+180°` and `-180°` describe the same rigid geometry, but replay interpolates stored angles.
/// Letting a solver arbitrarily exchange those two representations would make replay unfold that
/// hinge through zero even though it did not move during this pose.
fn preserve_flat_boundary_signs(solved: &mut HashMap<EdgeId, f64>, start: &HashMap<EdgeId, f64>) {
    for (&edge_id, angle) in solved {
        let Some(&start_angle) = start.get(&edge_id) else {
            continue;
        };
        if (angle.abs() - 180.0).abs() <= FLAT_TARGET_EPS
            && (start_angle.abs() - 180.0).abs() <= FLAT_TARGET_EPS
        {
            *angle = start_angle;
        }
    }
}

/// Continues a closed non-flat pose to an exact flat endpoint and records its
/// new layer order atomically.
///
/// The near-flat frame determines front/back constraints before all z values
/// collapse to zero.  Unrelated or exactly coincident faces retain their prior
/// relative order.  A cyclic set of depth constraints is rejected because no
/// single flat layer order could represent that endpoint.
pub fn solve_and_apply_flat_pose_step(
    document: &mut Document,
    input: FlatPoseMotionInput,
) -> Result<FlatPoseMotionResult, String> {
    let mut candidate = document.clone();
    apply_activations(&mut candidate, &input.activations)?;

    let faces = extract_faces(&candidate.cp);
    if faces.is_empty() {
        return Err("flat pose motion cannot be applied to a document with no faces".to_string());
    }
    let owners = faces_by_edge(&faces);
    let hinges = hinge_ids(&owners);

    let replayed = replay(&candidate, candidate.sequence.len(), 1.0);
    validate_replay(&candidate, &faces, &replayed)?;
    // `replay` is allowed to relax old preferred angles for display.  A physical continuation,
    // however, must start from the complete angle set persisted by the preceding Pose step.  Pose
    // persistence records every hinge, while a newly activated hinge starts at exactly zero.
    let start_angles = hinges
        .iter()
        .map(|&hinge| {
            let angle = replayed
                .sequence_targets
                .iter()
                .rev()
                .find(|driver| driver.hinge == hinge)
                .map_or(0.0, |driver| driver.target_angle_deg);
            (hinge, angle)
        })
        .collect::<HashMap<_, _>>();
    validate_angle_map("flat pose motion persisted start", &start_angles, &hinges)?;
    let start_folded = propagate(&candidate.cp, &faces, &start_angles);
    let start_frame = to_frame3d(&candidate.cp, &faces, &start_folded);
    validate_frame(
        &candidate,
        &faces,
        &start_frame,
        "flat pose motion persisted start",
    )?;

    let exact_drivers = validate_drivers(&candidate, &owners, &input.drivers)?;
    validate_branch_hints(&candidate, &owners, &input.branch_hints)?;
    let previous_targets = replayed
        .sequence_targets
        .iter()
        .map(|driver| (driver.hinge, driver.target_angle_deg))
        .collect::<HashMap<_, _>>();
    let sampled_drivers = exact_drivers
        .iter()
        .map(|driver| {
            let exact = canonical_flat_angle(driver.target_angle_deg).ok_or_else(|| {
                format!(
                    "flat pose driver edge {} angle {} is not 0 or +/-180 degrees",
                    driver.hinge, driver.target_angle_deg
                )
            })?;
            let start = start_angles.get(&driver.hinge).copied().unwrap_or(0.0);
            Ok(Driver {
                hinge: driver.hinge,
                target_angle_deg: approach_flat_angle(exact, start),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut seed = start_angles.clone();
    let replay_seed = seed.clone();
    let mut branch_seed = seed.clone();
    for hint in &input.branch_hints {
        branch_seed.insert(hint.edge_id, hint.target_angle_deg);
    }
    let continuation_steps = sampled_drivers
        .iter()
        .map(|driver| {
            let start = start_angles[&driver.hinge];
            ((driver.target_angle_deg - start).abs() / MAX_FLAT_CONTINUATION_DELTA_DEGREES).ceil()
                as usize
        })
        .max()
        .unwrap_or(1)
        .max(1);
    let mut approached = if input.branch_hints.is_empty() {
        None
    } else {
        let mut direct = solve(&candidate.cp, &faces, &sampled_drivers, Some(&branch_seed));
        let refined = solve_near_exact(
            &candidate.cp,
            &faces,
            &sampled_drivers,
            &previous_targets,
            Some(&direct.angles),
        );
        if refined.converged {
            direct = refined;
        }
        direct.converged.then_some(direct)
    };
    if approached.is_none() {
        for step in 1..=continuation_steps {
            let progress = step as f64 / continuation_steps as f64;
            let drivers = sampled_drivers
                .iter()
                .map(|driver| {
                    let start = start_angles[&driver.hinge];
                    Driver {
                        hinge: driver.hinge,
                        target_angle_deg: start + (driver.target_angle_deg - start) * progress,
                    }
                })
                .collect::<Vec<_>>();
            let mut solved = solve(&candidate.cp, &faces, &drivers, Some(&seed));
            if !solved.converged {
                // A very accurate previous solution can sit on a rank-deficient tangent near a
                // flat singularity. Retry from the original pose before rejecting the path.
                solved = solve(&candidate.cp, &faces, &drivers, Some(&replay_seed));
            }
            if !solved.converged {
                solved = solve_near_exact(
                    &candidate.cp,
                    &faces,
                    &drivers,
                    &previous_targets,
                    Some(&seed),
                );
            } else {
                // The hard continuation solve selects the outgoing branch.  Refine only after
                // that choice so saved sequence angles remain medium preferences rather than
                // being silently demoted to a warm start.
                let refined = solve_near_exact(
                    &candidate.cp,
                    &faces,
                    &drivers,
                    &previous_targets,
                    Some(&solved.angles),
                );
                if refined.converged {
                    solved = refined;
                }
            }
            if !solved.converged {
                return Err(format!(
                    "flat pose motion approach solve did not converge at continuation step {step}/{continuation_steps}"
                ));
            }
            if !solved.frame.warnings.is_empty() {
                return Err(format!(
                    "flat pose motion approach step {step}/{continuation_steps} warnings: {:?}",
                    solved.frame.warnings
                ));
            }
            validate_frame(
                &candidate,
                &faces,
                &solved.frame,
                "flat pose motion continuation",
            )?;
            seed = solved.angles.clone();
            approached = Some(solved);
        }
    }
    let approached = approached.expect("flat continuation always has at least one step");
    if !approached.frame.warnings.is_empty() {
        return Err(format!(
            "flat pose motion approach warnings: {:?}",
            approached.frame.warnings
        ));
    }
    validate_frame(
        &candidate,
        &faces,
        &approached.frame,
        "flat pose motion approach",
    )?;
    validate_angle_map("flat pose motion approach", &approached.angles, &hinges)?;

    let mut exact_angles = snap_complete_flat_angles(&approached.angles)?;
    // A dependent hinge can reach the same flat geometry as either +180 or -180 degrees.  Keep
    // the side from which the preceding pose approached that boundary; otherwise the next motion
    // starts from the opposite branch of the flat singularity.  Explicit drivers remain
    // authoritative because callers use their sign to request a deliberate branch change.
    let driven = exact_drivers
        .iter()
        .map(|driver| driver.hinge)
        .collect::<HashSet<_>>();
    for (&hinge, angle) in &mut exact_angles {
        if driven.contains(&hinge) || angle.abs() != 180.0 {
            continue;
        }
        let start = start_angles.get(&hinge).copied().unwrap_or(0.0);
        if start.abs() > FLAT_TARGET_EPS {
            *angle = start.signum() * 180.0;
        }
    }
    for driver in &exact_drivers {
        let expected = canonical_flat_angle(driver.target_angle_deg)
            .expect("flat targets were validated while constructing the approach");
        if exact_angles.get(&driver.hinge).copied() != Some(expected) {
            return Err(format!(
                "flat pose driver edge {} approached the wrong flat branch",
                driver.hinge
            ));
        }
    }

    let exact_folded = propagate(&candidate.cp, &faces, &exact_angles);
    let exact_frame = to_frame3d(&candidate.cp, &faces, &exact_folded);
    if !exact_frame.warnings.is_empty() {
        return Err(format!(
            "flat pose motion snapped frame warnings: {:?}",
            exact_frame.warnings
        ));
    }
    validate_frame(
        &candidate,
        &faces,
        &exact_frame,
        "flat pose motion snapped frame",
    )?;
    validate_flat_frame(&exact_frame)?;

    let approached_folded = propagate(&candidate.cp, &faces, &approached.angles);
    let order = derive_flat_layer_order(
        &faces,
        &approached_folded,
        &exact_folded,
        &exact_frame,
        &replayed.layer_transition.end,
    )?;
    let layer_points = FlatState {
        placements: HashMap::new(),
        order: order.clone(),
    }
    .to_layer_points(&candidate.cp, &faces);
    let driver_updates = driver_lines_for_angles(&candidate, &exact_angles)?;
    let step_id = u32::try_from(candidate.sequence.len())
        .map_err(|_| "flat pose motion step count exceeds u32".to_string())?;
    if candidate.sequence.iter().any(|step| step.id == step_id) {
        return Err(format!(
            "flat pose motion step ID {step_id} is already in use"
        ));
    }
    candidate.sequence.push(FoldStep {
        id: step_id,
        kind: TechniqueKind::Simple,
        drivers: driver_updates,
        layer_order: Some(layer_points),
        alignment: None,
        finish_soft: None,
        note: input.note,
    });

    let completed = replay(&candidate, candidate.sequence.len(), 1.0);
    validate_replay(&candidate, &faces, &completed)?;
    validate_angle_map("flat pose motion replay", &completed.hinge_angles, &hinges)?;
    for (&edge, &expected) in &exact_angles {
        if (completed.hinge_angles[&edge] - expected).abs() > FLAT_TARGET_EPS {
            return Err(format!(
                "flat pose motion replay changed edge {edge} from {expected} to {}",
                completed.hinge_angles[&edge]
            ));
        }
    }
    let (state, warnings) = flat_state_at(&candidate, &faces, candidate.sequence.len())?;
    if !warnings.is_empty() {
        return Err(format!("flat pose motion state warnings: {warnings:?}"));
    }
    if state.order != order {
        return Err("flat pose motion replay changed the derived layer order".to_string());
    }
    validate_flat_state_placements(&candidate, &faces, &state, &completed.frame)?;
    let gap = validate_frame(
        &candidate,
        &faces,
        &completed.frame,
        "flat pose motion replay",
    )?;

    *document = candidate;
    Ok(FlatPoseMotionResult {
        step_id,
        frame: completed.frame,
        max_seam_gap: gap,
        hinge_angles: exact_angles,
        state,
    })
}

fn canonical_flat_angle(angle: f64) -> Option<f64> {
    [-180.0, 0.0, 180.0]
        .into_iter()
        .min_by(|left, right| (angle - left).abs().total_cmp(&(angle - right).abs()))
        .filter(|target| (angle - target).abs() <= FLAT_TARGET_EPS)
}

fn approach_flat_angle(target: f64, start: f64) -> f64 {
    if target == 180.0 {
        180.0 - FLAT_APPROACH_DEGREES
    } else if target == -180.0 {
        -180.0 + FLAT_APPROACH_DEGREES
    } else if start > FLAT_APPROACH_DEGREES {
        FLAT_APPROACH_DEGREES
    } else if start < -FLAT_APPROACH_DEGREES {
        -FLAT_APPROACH_DEGREES
    } else {
        0.0
    }
}

fn snap_complete_flat_angles(
    angles: &HashMap<EdgeId, f64>,
) -> Result<HashMap<EdgeId, f64>, String> {
    angles
        .iter()
        .map(|(&edge, &angle)| {
            let target = [-180.0, 0.0, 180.0]
                .into_iter()
                .min_by(|left, right| (angle - left).abs().total_cmp(&(angle - right).abs()))
                .expect("three flat targets");
            if (angle - target).abs() > FLAT_ANGLE_TOLERANCE_DEGREES {
                return Err(format!(
                    "flat pose motion edge {edge} stopped at {angle} degrees, not near a flat angle"
                ));
            }
            Ok((edge, target))
        })
        .collect()
}

fn validate_flat_frame(frame: &Frame3D) -> Result<(), String> {
    let maximum_height = frame
        .faces
        .iter()
        .flat_map(|face| face.polygon.iter())
        .map(|point| point[2].abs())
        .fold(0.0, f64::max);
    if maximum_height >= MAX_POSE_SEAM_GAP {
        return Err(format!(
            "flat pose motion snapped frame has max |z|={maximum_height:.3e}"
        ));
    }
    Ok(())
}

fn derive_flat_layer_order(
    faces: &[Face],
    approached_folded: &FoldedFrame,
    exact_folded: &FoldedFrame,
    exact_frame: &Frame3D,
    previous_order: &[FaceId],
) -> Result<Vec<FaceId>, String> {
    let face_ids = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    let previous_ids = previous_order.iter().copied().collect::<BTreeSet<_>>();
    if previous_order.len() != faces.len() || previous_ids != face_ids {
        return Err("flat pose motion prior layer order does not match its faces".to_string());
    }
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
            let left_polygon = frame_faces
                .get(&left)
                .ok_or_else(|| format!("flat pose motion exact frame lost face {left}"))?
                .polygon
                .iter()
                .map(|point| DVec2::new(point[0], point[1]))
                .collect::<Vec<_>>();
            let right_polygon = frame_faces
                .get(&right)
                .ok_or_else(|| format!("flat pose motion exact frame lost face {right}"))?
                .polygon
                .iter()
                .map(|point| DVec2::new(point[0], point[1]))
                .collect::<Vec<_>>();
            let witnesses = overlap_witnesses(&left_polygon, &right_polygon)?;
            let mut left_above = false;
            let mut right_above = false;
            for witness in witnesses {
                let left_height = approached_height_at_flat_point(
                    left,
                    witness,
                    approached_folded,
                    exact_folded,
                )?;
                let right_height = approached_height_at_flat_point(
                    right,
                    witness,
                    approached_folded,
                    exact_folded,
                )?;
                let difference = left_height - right_height;
                left_above |= difference > DEPTH_ORDER_EPS;
                right_above |= difference < -DEPTH_ORDER_EPS;
            }
            if left_above && right_above {
                return Err(format!(
                    "flat pose motion faces {left} and {right} exchange depth across their overlap"
                ));
            }
            if left_above {
                constraints.insert((right, left));
            } else if right_above {
                constraints.insert((left, right));
            }
        }
    }

    stable_topological_order(previous_order, &constraints)
}

fn approached_height_at_flat_point(
    face: FaceId,
    point: DVec2,
    approached: &FoldedFrame,
    exact: &FoldedFrame,
) -> Result<f64, String> {
    let (exact_rotation, exact_translation) = exact
        .transforms
        .get(&face)
        .ok_or_else(|| format!("flat pose motion exact transform lost face {face}"))?;
    let (approached_rotation, approached_translation) = approached
        .transforms
        .get(&face)
        .ok_or_else(|| format!("flat pose motion approach transform lost face {face}"))?;
    let material =
        exact_rotation.transpose() * (DVec3::new(point.x, point.y, 0.0) - *exact_translation);
    let approached_point = *approached_rotation * material + *approached_translation;
    if !approached_point.is_finite() {
        return Err(format!(
            "flat pose motion face {face} produced a non-finite depth sample"
        ));
    }
    Ok(approached_point.z)
}

fn overlap_witnesses(left: &[DVec2], right: &[DVec2]) -> Result<Vec<DVec2>, String> {
    let left_triangles = triangulate_polygon(left)?;
    let right_triangles = triangulate_polygon(right)?;
    let mut witnesses = Vec::new();
    for left_triangle in &left_triangles {
        for right_triangle in &right_triangles {
            let intersection = intersect_convex_polygons(left_triangle, right_triangle);
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
        return Err("flat pose motion encountered a degenerate face polygon".to_string());
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
            (b - a).perp_dot(c - b) > ori3_model::EPS * ori3_model::EPS
                && !polygon.iter().enumerate().any(|(other, &point)| {
                    other != index
                        && other != (index + count - 1) % count
                        && other != (index + 1) % count
                        && point_in_triangle(point, a, b, c)
                })
        }) else {
            return Err("flat pose motion could not triangulate a face polygon".to_string());
        };
        let triangle = vec![
            polygon[(ear + count - 1) % count],
            polygon[ear],
            polygon[(ear + 1) % count],
        ];
        triangles.push(triangle);
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
            .is_none_or(|previous: &DVec2| (*previous - point).length() > ori3_model::EPS)
        {
            polygon.push(point);
        }
    }
    while polygon.len() > 1 && (polygon[0] - polygon[polygon.len() - 1]).length() <= ori3_model::EPS
    {
        polygon.pop();
    }
    loop {
        let count = polygon.len();
        if count < 3 {
            break;
        }
        let Some(tip) = (0..count).find(|&index| {
            (polygon[(index + count - 1) % count] - polygon[(index + 1) % count]).length()
                <= ori3_model::EPS
        }) else {
            break;
        };
        let duplicate = (tip + 1) % count;
        polygon = polygon
            .into_iter()
            .enumerate()
            .filter_map(|(index, point)| (index != tip && index != duplicate).then_some(point))
            .collect();
    }
    polygon
}

fn point_in_triangle(point: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    (b - a).perp_dot(point - a) >= -ori3_model::EPS
        && (c - b).perp_dot(point - b) >= -ori3_model::EPS
        && (a - c).perp_dot(point - c) >= -ori3_model::EPS
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
            let previous_inside = previous_side >= -ori3_model::EPS;
            let current_inside = current_side >= -ori3_model::EPS;
            if previous_inside != current_inside {
                let denominator = previous_side - current_side;
                if denominator.abs() > ori3_model::EPS * ori3_model::EPS {
                    let parameter = previous_side / denominator;
                    output.push(previous + (current - previous) * parameter);
                }
            }
            if current_inside {
                output.push(current);
            }
            previous = current;
            previous_side = current_side;
        }
    }
    let mut deduplicated = Vec::with_capacity(output.len());
    for point in output {
        if deduplicated
            .last()
            .is_none_or(|previous: &DVec2| (*previous - point).length() > ori3_model::EPS)
        {
            deduplicated.push(point);
        }
    }
    if deduplicated.len() > 1
        && (deduplicated[0] - deduplicated[deduplicated.len() - 1]).length() <= ori3_model::EPS
    {
        deduplicated.pop();
    }
    deduplicated
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
                "flat pose motion constraint references missing face {below}"
            ));
        };
        if !indegree.contains_key(&above) {
            return Err(format!(
                "flat pose motion constraint references missing face {above}"
            ));
        }
        if neighbors.insert(above) {
            indegree.entry(above).and_modify(|degree| *degree += 1);
        }
    }

    let previous_rank = previous_order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let mut emitted = BTreeSet::new();
    let mut order = Vec::with_capacity(previous_order.len());
    while order.len() < previous_order.len() {
        let next = previous_order
            .iter()
            .copied()
            .filter(|face| !emitted.contains(face) && indegree[face] == 0)
            .min_by_key(|face| previous_rank[face])
            .ok_or_else(|| "flat pose motion depth constraints contain a cycle".to_string())?;
        emitted.insert(next);
        order.push(next);
        for above in outgoing[&next].iter().copied() {
            indegree.entry(above).and_modify(|degree| *degree -= 1);
        }
    }
    Ok(order)
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

/// Reconstruct the closed pose immediately before activating a new crease network.
///
/// In the usual case replaying on the activated candidate succeeds directly.  At an exact flat
/// singularity a priority replay may not select a closed branch after the face graph is split.
/// The old document is still an authoritative closed state: replay it before activation, add the
/// newly-created hinges at 0 degrees, and require an all-hard rigid solve on the candidate graph.
fn replay_pose_start(
    document: &Document,
    candidate: &Document,
    faces: &[Face],
    hinges: &BTreeSet<EdgeId>,
) -> Result<ReplayPoseStart, String> {
    let replayed = replay(candidate, candidate.sequence.len(), 1.0);
    let resolved_targets = replayed
        .sequence_targets
        .iter()
        .map(|driver| (driver.hinge, driver.target_angle_deg))
        .collect::<HashMap<_, _>>();
    let direct = validate_replay(candidate, faces, &replayed)
        .and_then(|()| validate_angle_map("replayed start", &replayed.hinge_angles, hinges));
    let saved_is_exactly_flat = hinges.iter().all(|hinge| {
        canonical_flat_angle(resolved_targets.get(hinge).copied().unwrap_or(0.0)).is_some()
    });
    if direct.is_ok() && !saved_is_exactly_flat {
        return Ok(ReplayPoseStart {
            angles: replayed.hinge_angles,
            preferred: resolved_targets,
        });
    }
    let direct_error = match direct {
        Ok(()) => "priority replay of an activated exact-flat state was replaced by its saved exact angles"
            .to_string(),
        Err(error) => error,
    };

    let prior_faces = extract_faces(&document.cp);
    let prior = replay(document, document.sequence.len(), 1.0);
    validate_replay(document, &prior_faces, &prior).map_err(|fallback_error| {
        format!("{direct_error}; replay before crease activation also failed: {fallback_error}")
    })?;

    let exact_angles = hinges
        .iter()
        .map(|&hinge| (hinge, resolved_targets.get(&hinge).copied().unwrap_or(0.0)))
        .collect::<HashMap<_, _>>();
    let exact_folded = propagate(&candidate.cp, faces, &exact_angles);
    let exact_frame = to_frame3d(&candidate.cp, faces, &exact_folded);
    validate_frame(
        candidate,
        faces,
        &exact_frame,
        "pose motion exact start after crease activation",
    )
    .map_err(|fallback_error| format!("{direct_error}; {fallback_error}"))?;
    validate_angle_map("exact start after crease activation", &exact_angles, hinges)?;
    Ok(ReplayPoseStart {
        angles: exact_angles,
        preferred: resolved_targets,
    })
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

fn validate_flat_state_placements(
    document: &Document,
    faces: &[Face],
    state: &FlatState,
    frame: &Frame3D,
) -> Result<(), String> {
    let expected = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    let actual = state.placements.keys().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!(
            "flat pose motion state placements have wrong faces: expected {expected:?}, got {actual:?}"
        ));
    }
    let vertices = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let frame_faces = frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    for face in faces {
        let placement = state
            .placements
            .get(&face.id)
            .ok_or_else(|| format!("flat pose motion state lost face {}", face.id))?;
        let output = frame_faces
            .get(&face.id)
            .ok_or_else(|| format!("flat pose motion replay frame lost face {}", face.id))?;
        for (&vertex, point) in face.vertices.iter().zip(&output.polygon) {
            let material = vertices.get(&vertex).copied().ok_or_else(|| {
                format!(
                    "flat pose motion face {} references missing vertex {vertex}",
                    face.id
                )
            })?;
            let folded = placement.apply(material);
            if !folded.is_finite()
                || (folded - DVec2::new(point[0], point[1])).length() >= MAX_POSE_SEAM_GAP
            {
                return Err(format!(
                    "flat pose motion state placement for face {} disagrees with replay",
                    face.id
                ));
            }
        }
    }
    Ok(())
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
