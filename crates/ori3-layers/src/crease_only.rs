//! Crease a flap and immediately unfold it.
//!
//! The operation changes the crease pattern, but its completed state has the
//! same face placements and layer order as the input state.  A zero-degree
//! driver is recorded so replay also finishes in that flat state.

use ori3_cp::Face;
use std::collections::{HashMap, HashSet};

use glam::DVec2;
use ori3_model::{
    CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, FaceId, FoldStep, TechniqueKind,
};

use crate::flat_motion::{EvidenceWanted, FlatMotionInput, MotionPart, run_motion};
use crate::flat_state::FlatState;
use crate::fold_through::{FoldDirection, FoldThroughResult};

/// Input for [`crease_only`].
#[derive(Clone, Debug)]
pub struct CreaseOnlyInput {
    /// Crease line in the current folded-plane coordinates.
    pub line: [[f64; 2]; 2],
    /// A point on the side that is folded temporarily.
    pub movable_side_point: [f64; 2],
    /// Current face IDs to crease. `None` selects every intersected layer.
    pub target_layers: Option<Vec<FaceId>>,
    /// Sense of the temporary fold. The final hinge angle is always zero.
    pub direction: FoldDirection,
}

/// Input for [`reverse_open_crease_sense`].
#[derive(Clone, Debug)]
pub struct ReverseOpenCreaseInput {
    /// Existing crease line in current folded-plane coordinates.
    pub line: [[f64; 2]; 2],
    /// Local face IDs on the side whose material crease segments are reversed.
    /// `None` selects all faces incident to the visible line.
    pub target_layers: Option<Vec<FaceId>>,
}

/// Fold the selected flap on `line`, unfold it, and retain only the crease.
///
/// This is one persistent operation: it adds the crease and records a
/// zero-degree driver, while preserving the completed placement and stack
/// order. The crease pattern is updated only after the operation succeeds.
pub fn crease_only(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &CreaseOnlyInput,
) -> Result<FoldThroughResult, String> {
    if input.target_layers.as_ref().is_some_and(Vec::is_empty) {
        return Err("crease-only target layer list is empty".to_string());
    }

    let motion = FlatMotionInput {
        parts: vec![MotionPart::crease_only(
            input.target_layers.clone().unwrap_or_default(),
            input.line,
            input.movable_side_point,
            input.direction,
        )],
        kind: TechniqueKind::Simple,
    };
    let out = run_motion(cp, faces, state, &motion, EvidenceWanted::No)?;
    if !out.crossed_any {
        return Err("crease line does not cross any selected layer".to_string());
    }

    *cp = out.cp;
    Ok(out.result)
}

/// Reverse mountain/valley assignment on selected, currently open crease segments.
///
/// Book instructions occasionally ask to reverse only the front sheet's precreases before a
/// collapse.  A mountain/valley kind belongs to the material CP segment, so coincident back-sheet
/// segments are left untouched.  Folded hinges are rejected: changing their sign without moving
/// the paper would change the represented state.  The returned step drives every changed segment
/// to zero degrees and therefore replays to the unchanged flat state.
pub fn reverse_open_crease_sense(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &ReverseOpenCreaseInput,
) -> Result<FoldThroughResult, String> {
    let a = DVec2::from(input.line[0]);
    let b = DVec2::from(input.line[1]);
    let direction = b - a;
    if direction.length() <= EPS {
        return Err("reverse-crease line is degenerate".to_string());
    }
    let unit = direction.normalize();
    let selected = input
        .target_layers
        .clone()
        .unwrap_or_else(|| faces.iter().map(|face| face.id).collect());
    if selected.is_empty() {
        return Err("reverse-crease target layer list is empty".to_string());
    }
    let selected_set = selected.iter().copied().collect::<HashSet<_>>();
    if selected_set.len() != selected.len() {
        return Err("reverse-crease target layers contain duplicates".to_string());
    }
    if let Some(missing) = selected_set
        .iter()
        .find(|id| !faces.iter().any(|face| face.id == **id))
    {
        return Err(format!(
            "reverse-crease target layer {missing} does not exist"
        ));
    }

    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let mut owners: HashMap<EdgeId, Vec<FaceId>> = HashMap::new();
    for face in faces {
        for edge in &face.edges {
            owners.entry(*edge).or_default().push(face.id);
        }
    }

    let mut changed = HashSet::new();
    for face in faces.iter().filter(|face| selected_set.contains(&face.id)) {
        let placement = state
            .placements
            .get(&face.id)
            .ok_or_else(|| format!("reverse-crease target layer {} has no placement", face.id))?;
        for &edge_id in &face.edges {
            let Some(edge) = cp.edges.iter().find(|edge| edge.id == edge_id) else {
                continue;
            };
            if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                continue;
            }
            let (Some(&p0), Some(&p1)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
                continue;
            };
            let q0 = placement.apply(p0);
            let q1 = placement.apply(p1);
            if unit.perp_dot(q0 - a).abs() > EPS || unit.perp_dot(q1 - a).abs() > EPS {
                continue;
            }
            if let Some(adjacent) = owners.get(&edge_id).filter(|owners| owners.len() == 2) {
                let left = state
                    .placements
                    .get(&adjacent[0])
                    .ok_or_else(|| format!("crease owner {} has no placement", adjacent[0]))?;
                let right = state
                    .placements
                    .get(&adjacent[1])
                    .ok_or_else(|| format!("crease owner {} has no placement", adjacent[1]))?;
                if left.mirrored != right.mirrored {
                    return Err(format!(
                        "crease edge {edge_id} is folded; open it before reversing its sense"
                    ));
                }
            }
            changed.insert(edge_id);
        }
    }
    if changed.is_empty() {
        return Err(
            "selected layers contain no open mountain/valley segment on the line".to_string(),
        );
    }

    let mut work = cp.clone();
    for edge in &mut work.edges {
        if changed.contains(&edge.id) {
            edge.kind = match edge.kind {
                EdgeKind::Mountain => EdgeKind::Valley,
                EdgeKind::Valley => EdgeKind::Mountain,
                other => other,
            };
        }
    }
    let mut changed = changed.into_iter().collect::<Vec<_>>();
    changed.sort_unstable();
    let drivers = changed
        .iter()
        .filter_map(|id| {
            let edge = work.edges.iter().find(|edge| edge.id == *id)?;
            let p0 = positions.get(&edge.v0)?;
            let p1 = positions.get(&edge.v1)?;
            Some(DriverLine {
                a: [p0.x, p0.y],
                b: [p1.x, p1.y],
                target_angle_deg: 0.0,
            })
        })
        .collect();
    let step = FoldStep {
        id: 0,
        kind: TechniqueKind::Simple,
        drivers,
        layer_order: Some(state.to_layer_points(&work, faces)),
        alignment: None,
        finish_soft: None,
        note: String::new(),
        technique_classification: None,
    };
    *cp = work;
    Ok(FoldThroughResult {
        state: state.clone(),
        added_edges: changed,
        step,
        warnings: Vec::new(),
        source_face_of: faces.iter().map(|face| (face.id, face.id)).collect(),
    })
}
