//! Reversal of an already folded local crease network.

use std::collections::{HashMap, HashSet};

use glam::DVec2;
use ori3_cp::Face;
use ori3_model::{CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, FaceId, TechniqueKind};

use crate::flat_motion::{
    EvidenceWanted, FlatMotionInput, LayerTurn, MotionPart, MotionTransform, run_motion,
};
use crate::flat_state::FlatState;
use crate::fold_through::{FoldThroughResult, angle_of, push_driver_line};

const STATE_EPS: f64 = 1e-7;

/// Input for [`reverse_fold_network`].
#[derive(Clone, Debug)]
pub struct ReverseFoldNetworkInput {
    /// Local layers forming the folded packet, in their current bottom-to-top order.
    pub target_layers: Vec<FaceId>,
    /// Existing network crease lines in current folded-plane coordinates.
    pub creases: Vec<[[f64; 2]; 2]>,
}

/// Turn a folded local network over without changing its outline.
///
/// The packet's layer order is reversed in place.  Every selected existing hinge on `creases`
/// keeps its absolute 180-degree angle and changes its sign; open precreases on the same material
/// network change mountain/valley assignment while retaining a zero-degree driver.  Reordering a
/// local packet can also exchange two globally ordered faces across a hinge outside the named
/// network; those incidental changes are restored explicitly so the operation remains local.
pub fn reverse_fold_network(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &ReverseFoldNetworkInput,
) -> Result<FoldThroughResult, String> {
    validate_input(faces, state, input)?;
    let positions = vertex_positions(cp);
    let network = resolve_network_edges(cp, faces, state, input, &positions)?;
    if network.is_empty() {
        return Err(
            "none of the selected layers has an existing crease on the network".to_string(),
        );
    }

    let before_kinds = cp
        .edges
        .iter()
        .map(|edge| (edge.id, edge.kind))
        .collect::<HashMap<_, _>>();
    let out = run_motion(
        cp,
        faces,
        state,
        &FlatMotionInput {
            parts: vec![MotionPart {
                layers: input.target_layers.clone(),
                region: Vec::new(),
                transform: MotionTransform::Stay,
                turn: LayerTurn::Keep,
                reverse_layers: Some(true),
            }],
            kind: TechniqueKind::Simple,
        },
        EvidenceWanted::No,
    )?;
    if !out.result.warnings.is_empty() {
        return Err(format!(
            "fold-network reversal produced warnings: {:?}",
            out.result.warnings
        ));
    }
    for face in faces {
        if !out.result.state.placements[&face.id].approx_eq(&state.placements[&face.id], STATE_EPS)
        {
            return Err(format!(
                "fold-network reversal moved face {} instead of preserving the outline",
                face.id
            ));
        }
    }

    let changed_by_motion = out
        .cp
        .edges
        .iter()
        .filter(|edge| before_kinds.get(&edge.id) != Some(&edge.kind))
        .map(|edge| edge.id)
        .collect::<HashSet<_>>();
    let mut work = out.cp;
    let mut result = out.result;
    // `FlatState` has one deterministic global order even for faces that do not overlap.  Reversing
    // a local packet in its existing slots can therefore make the generic settling pass flip a
    // boundary hinge that was not named by this operation.  Preserve every such hinge, including
    // its replay angle, instead of leaking the local reversal into the rest of the model.
    for edge_id in changed_by_motion.difference(&network) {
        let edge = work
            .edges
            .iter_mut()
            .find(|edge| edge.id == *edge_id)
            .ok_or_else(|| format!("crease edge {edge_id} disappeared"))?;
        let before = before_kinds[edge_id];
        edge.kind = before;
        let (p0, p1) = (positions[&edge.v0], positions[&edge.v1]);
        result
            .step
            .drivers
            .retain(|driver| !same_segment(DVec2::from(driver.a), DVec2::from(driver.b), p0, p1));
        push_driver_line(&mut result.step.drivers, p0, p1, angle_of(before));
    }
    // A folded hinge is normally flipped by flat_motion's settling pass.  An open precrease has
    // no relative placement change, so flip its material assignment explicitly and retain a 0°
    // driver.  In both cases every named network edge changes sense exactly once.
    for edge_id in &network {
        if changed_by_motion.contains(edge_id) {
            continue;
        }
        let edge = work
            .edges
            .iter_mut()
            .find(|edge| edge.id == *edge_id)
            .ok_or_else(|| format!("network edge {edge_id} disappeared"))?;
        edge.kind = opposite(edge.kind);
        let (p0, p1) = (positions[&edge.v0], positions[&edge.v1]);
        result.step.drivers.push(DriverLine {
            a: [p0.x, p0.y],
            b: [p1.x, p1.y],
            target_angle_deg: 0.0,
        });
    }
    for edge_id in &network {
        let before = before_kinds[edge_id];
        let after = work
            .edges
            .iter()
            .find(|edge| edge.id == *edge_id)
            .map(|edge| edge.kind)
            .ok_or_else(|| format!("network edge {edge_id} disappeared"))?;
        if after != opposite(before) {
            return Err(format!(
                "network edge {edge_id} did not reverse its crease sense"
            ));
        }
    }
    result.step.layer_order = Some(result.state.to_layer_points(&work, faces));
    result.added_edges = network.iter().copied().collect();
    result.added_edges.sort_unstable();
    *cp = work;
    Ok(result)
}

fn same_segment(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> bool {
    ((a0 - b0).length() <= STATE_EPS && (a1 - b1).length() <= STATE_EPS)
        || ((a0 - b1).length() <= STATE_EPS && (a1 - b0).length() <= STATE_EPS)
}

fn validate_input(
    faces: &[Face],
    state: &FlatState,
    input: &ReverseFoldNetworkInput,
) -> Result<(), String> {
    if input.target_layers.len() < 2 {
        return Err("fold-network reversal needs at least two local layers".to_string());
    }
    if input.creases.is_empty() {
        return Err("fold-network reversal needs at least one crease line".to_string());
    }
    let selected = input.target_layers.iter().copied().collect::<HashSet<_>>();
    if selected.len() != input.target_layers.len() {
        return Err("fold-network target layers contain duplicates".to_string());
    }
    for id in &input.target_layers {
        if !faces.iter().any(|face| face.id == *id) || !state.placements.contains_key(id) {
            return Err(format!("fold-network target layer {id} does not exist"));
        }
    }
    let rank = state
        .order
        .iter()
        .enumerate()
        .map(|(rank, &id)| (id, rank))
        .collect::<HashMap<_, _>>();
    let supplied = input
        .target_layers
        .iter()
        .map(|id| rank[id])
        .collect::<Vec<_>>();
    if supplied.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("fold-network target layers must be supplied bottom-to-top".to_string());
    }
    for (index, line) in input.creases.iter().enumerate() {
        if (DVec2::from(line[1]) - DVec2::from(line[0])).length() <= EPS {
            return Err(format!("fold-network crease {} is degenerate", index + 1));
        }
    }
    Ok(())
}

fn resolve_network_edges(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &ReverseFoldNetworkInput,
    positions: &HashMap<u32, DVec2>,
) -> Result<HashSet<EdgeId>, String> {
    let selected = input.target_layers.iter().copied().collect::<HashSet<_>>();
    let mut network = HashSet::new();
    let mut hit = vec![false; input.creases.len()];
    for face in faces.iter().filter(|face| selected.contains(&face.id)) {
        let placement = state.placements[&face.id];
        for &edge_id in &face.edges {
            let Some(edge) = cp.edges.iter().find(|edge| edge.id == edge_id) else {
                continue;
            };
            if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                continue;
            }
            let q0 = placement.apply(positions[&edge.v0]);
            let q1 = placement.apply(positions[&edge.v1]);
            for (index, line) in input.creases.iter().enumerate() {
                if segment_on_line(q0, q1, *line) {
                    hit[index] = true;
                    network.insert(edge_id);
                }
            }
        }
    }
    if let Some(index) = hit.iter().position(|hit| !hit) {
        return Err(format!(
            "fold-network crease {} has no selected existing segment",
            index + 1
        ));
    }
    Ok(network)
}

fn segment_on_line(a: DVec2, b: DVec2, line: [[f64; 2]; 2]) -> bool {
    let l0 = DVec2::from(line[0]);
    let direction = (DVec2::from(line[1]) - l0).normalize();
    direction.perp_dot(a - l0).abs() <= STATE_EPS && direction.perp_dot(b - l0).abs() <= STATE_EPS
}

fn vertex_positions(cp: &CreasePattern) -> HashMap<u32, DVec2> {
    cp.vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect()
}

fn opposite(kind: EdgeKind) -> EdgeKind {
    match kind {
        EdgeKind::Mountain => EdgeKind::Valley,
        EdgeKind::Valley => EdgeKind::Mountain,
        other => other,
    }
}
