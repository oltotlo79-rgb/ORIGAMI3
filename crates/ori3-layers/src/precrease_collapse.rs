//! Simultaneous flat collapse of an existing auxiliary crease network.
//!
//! Several precreases meeting at one vertex cannot be represented faithfully as a sequence of
//! unrelated book folds. This module activates named material lines, solves all face placements
//! from exact 180-degree hinge reflections, and records the collapse atomically.

use std::collections::{HashMap, HashSet, VecDeque};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::Isometry2;
use ori3_model::{CreasePattern, EPS, EdgeId, EdgeKind, FaceId, TechniqueKind, VertexId};

use crate::flat_motion::{FlatMotionInput, LayerTurn, MotionPart, MotionTransform, run_motion};
use crate::flat_state::{FlatState, point_in_face, representative_point};
use crate::fold_through::{FoldDirection, FoldThroughResult};

const PLACEMENT_EPS: f64 = 1e-7;

/// Material-coordinate precrease lines to close in one simultaneous collapse.
#[derive(Clone, Debug)]
pub struct PrecreaseCollapseInput {
    pub lines: Vec<[[f64; 2]; 2]>,
    /// Optional old-face packet. Named hinges must be internal to this packet; auxiliary
    /// segments are activated only inside one of these faces.
    pub target_layers: Option<Vec<FaceId>>,
}

/// Close an intersecting network of existing auxiliary lines to 180 degrees in one flat step.
pub fn collapse_precrease_network(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &PrecreaseCollapseInput,
) -> Result<FoldThroughResult, String> {
    validate_input(input)?;
    let positions = vertex_positions(cp);
    let selected = input
        .target_layers
        .as_ref()
        .map(|layers| layers.iter().copied().collect::<HashSet<_>>());
    if let Some(layers) = &selected {
        if layers.is_empty() {
            return Err("precrease collapse target layer packet is empty".to_string());
        }
        if layers
            .iter()
            .any(|id| !faces.iter().any(|face| face.id == *id))
        {
            return Err("precrease collapse target layer does not exist".to_string());
        }
    }
    let old_owners = edge_owners(faces);
    let mut work = cp.clone();
    let mut activated = Vec::<EdgeId>::new();
    let mut network = HashSet::<EdgeId>::new();
    let mut hit = vec![false; input.lines.len()];
    for edge in &mut work.edges {
        if edge.kind == EdgeKind::Border {
            continue;
        }
        let (Some(a), Some(b)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
            continue;
        };
        let selected_here = selected.as_ref().is_none_or(|selected| {
            if edge.kind == EdgeKind::Aux {
                let midpoint = (*a + *b) * 0.5;
                faces.iter().any(|face| {
                    selected.contains(&face.id) && point_in_face(cp, face, [midpoint.x, midpoint.y])
                })
            } else {
                old_owners.get(&edge.id).is_some_and(|owners| {
                    owners.len() == 2 && owners.iter().all(|owner| selected.contains(owner))
                })
            }
        });
        if !selected_here {
            continue;
        }
        for (index, line) in input.lines.iter().enumerate() {
            if segment_on_line(*a, *b, *line) {
                network.insert(edge.id);
                if edge.kind == EdgeKind::Aux {
                    edge.kind = EdgeKind::Valley;
                    activated.push(edge.id);
                }
                hit[index] = true;
                break;
            }
        }
    }
    if let Some(index) = hit.iter().position(|hit| !hit) {
        return Err(format!(
            "precrease collapse line {} has no auxiliary material segment",
            index + 1
        ));
    }
    if network.is_empty() {
        return Err("precrease collapse did not resolve any material segment".to_string());
    }

    let split_faces = extract_faces(&work);
    let (current, parent_of) = expanded_state(cp, faces, state, &work, &split_faces)?;
    let target_placements =
        reflected_placements(&work, &split_faces, &current, &parent_of, &network, state)?;
    let target_order = current.order.clone();
    settle_kinds_from_order(&mut work, &split_faces, &target_placements, &target_order)?;

    let parts = target_order
        .iter()
        .map(|face| MotionPart {
            layers: vec![*face],
            region: Vec::new(),
            transform: MotionTransform::Isometry(
                target_placements[face].compose(&current.placements[face].inverse()),
            ),
            turn: LayerTurn::Outside(FoldDirection::Up),
            reverse_layers: Some(false),
        })
        .collect();
    let outcome = run_motion(
        &work,
        &split_faces,
        &current,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Twist,
        },
    )?;
    if !outcome.result.warnings.is_empty() {
        return Err(format!(
            "precrease collapse produced warnings: {:?}",
            outcome.result.warnings
        ));
    }
    for face in &split_faces {
        if !outcome.result.state.placements[&face.id]
            .approx_eq(&target_placements[&face.id], PLACEMENT_EPS)
        {
            return Err(format!(
                "precrease collapse did not reach solved placement for face {}",
                face.id
            ));
        }
    }
    if outcome.result.state.order != target_order {
        return Err("precrease collapse did not preserve its solved layer order".to_string());
    }
    let mut result = outcome.result;
    result.added_edges = activated;
    result.added_edges.sort_unstable();
    result.added_edges.dedup();
    *cp = outcome.cp;
    Ok(result)
}

fn validate_input(input: &PrecreaseCollapseInput) -> Result<(), String> {
    if input.lines.is_empty() {
        return Err("precrease collapse needs at least one material line".to_string());
    }
    for (index, line) in input.lines.iter().enumerate() {
        let a = DVec2::from(line[0]);
        let b = DVec2::from(line[1]);
        if !a.is_finite() || !b.is_finite() || (b - a).length() <= EPS {
            return Err(format!(
                "precrease collapse line {} is degenerate",
                index + 1
            ));
        }
    }
    Ok(())
}

fn expanded_state(
    old_cp: &CreasePattern,
    old_faces: &[Face],
    old_state: &FlatState,
    work: &CreasePattern,
    split_faces: &[Face],
) -> Result<(FlatState, HashMap<FaceId, FaceId>), String> {
    let old_rank = old_state
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let mut parent_of = HashMap::new();
    let mut placements = HashMap::new();
    for face in split_faces {
        let point = representative_point(work, face);
        let parent = old_faces
            .iter()
            .find(|candidate| point_in_face(old_cp, candidate, point))
            .ok_or_else(|| format!("new collapse face {} has no parent face", face.id))?;
        let placement = old_state
            .placements
            .get(&parent.id)
            .copied()
            .ok_or_else(|| format!("parent face {} has no flat placement", parent.id))?;
        parent_of.insert(face.id, parent.id);
        placements.insert(face.id, placement);
    }
    let mut order = split_faces.iter().map(|face| face.id).collect::<Vec<_>>();
    order.sort_by_key(|face| (old_rank[&parent_of[face]], *face));
    Ok((FlatState { placements, order }, parent_of))
}

fn reflected_placements(
    cp: &CreasePattern,
    faces: &[Face],
    current: &FlatState,
    parent_of: &HashMap<FaceId, FaceId>,
    network: &HashSet<EdgeId>,
    old_state: &FlatState,
) -> Result<HashMap<FaceId, Isometry2>, String> {
    let positions = vertex_positions(cp);
    let owners = edge_owners(faces);
    let mut adjacency = HashMap::<FaceId, Vec<(FaceId, EdgeId)>>::new();
    for (edge, incident) in owners {
        if incident.len() != 2 {
            continue;
        }
        let crease = cp
            .edges
            .iter()
            .find(|candidate| candidate.id == edge)
            .ok_or_else(|| format!("crease edge {edge} disappeared"))?;
        if !matches!(crease.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        adjacency
            .entry(incident[0])
            .or_default()
            .push((incident[1], edge));
        adjacency
            .entry(incident[1])
            .or_default()
            .push((incident[0], edge));
    }

    let root = faces
        .iter()
        .map(|face| face.id)
        .min()
        .ok_or("precrease collapse has no faces")?;
    let mut placements = HashMap::from([(root, current.placements[&root])]);
    let mut queue = VecDeque::from([root]);
    while let Some(face) = queue.pop_front() {
        let placement = placements[&face];
        for &(neighbor, edge_id) in adjacency.get(&face).map(Vec::as_slice).unwrap_or(&[]) {
            let edge = cp
                .edges
                .iter()
                .find(|candidate| candidate.id == edge_id)
                .ok_or_else(|| format!("crease edge {edge_id} disappeared"))?;
            let old_a = parent_of[&face];
            let old_b = parent_of[&neighbor];
            let closes = network.contains(&edge_id)
                || old_state.placements[&old_a].mirrored != old_state.placements[&old_b].mirrored;
            let candidate = if closes {
                let reflection = Isometry2::reflection(positions[&edge.v0], positions[&edge.v1]);
                placement.compose(&reflection)
            } else {
                placement
            };
            if let Some(existing) = placements.get(&neighbor) {
                if !existing.approx_eq(&candidate, PLACEMENT_EPS) {
                    return Err(format!(
                        "precrease network is inconsistent around edge {edge_id}"
                    ));
                }
            } else {
                placements.insert(neighbor, candidate);
                queue.push_back(neighbor);
            }
        }
    }
    if placements.len() != faces.len() {
        let missing = faces
            .iter()
            .filter(|face| !placements.contains_key(&face.id))
            .map(|face| face.id)
            .collect::<Vec<_>>();
        return Err(format!(
            "precrease collapse face graph is disconnected; missing {missing:?}"
        ));
    }
    Ok(placements)
}

fn settle_kinds_from_order(
    cp: &mut CreasePattern,
    faces: &[Face],
    placements: &HashMap<FaceId, Isometry2>,
    order: &[FaceId],
) -> Result<(), String> {
    let rank = order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    for (edge_id, owners) in edge_owners(faces) {
        if owners.len() != 2 {
            continue;
        }
        let edge = cp
            .edges
            .iter_mut()
            .find(|candidate| candidate.id == edge_id)
            .ok_or_else(|| format!("crease edge {edge_id} disappeared"))?;
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let (a, b) = (owners[0], owners[1]);
        edge.kind = if (rank[&b] > rank[&a]) == placements[&a].mirrored {
            EdgeKind::Mountain
        } else {
            EdgeKind::Valley
        };
    }
    Ok(())
}

fn edge_owners(faces: &[Face]) -> HashMap<EdgeId, Vec<FaceId>> {
    let mut owners = HashMap::<EdgeId, Vec<FaceId>>::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().push(face.id);
        }
    }
    owners
}

fn vertex_positions(cp: &CreasePattern) -> HashMap<VertexId, DVec2> {
    cp.vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect()
}

fn segment_on_line(a: DVec2, b: DVec2, line: [[f64; 2]; 2]) -> bool {
    let l0 = DVec2::from(line[0]);
    let direction = (DVec2::from(line[1]) - l0).normalize();
    direction.perp_dot(a - l0).abs() <= PLACEMENT_EPS
        && direction.perp_dot(b - l0).abs() <= PLACEMENT_EPS
}

#[cfg(test)]
mod tests {
    use super::*;
    use ori3_cp::insert_segment;
    use ori3_model::{Document, Paper};

    #[test]
    fn collapses_crossing_precreases_without_sampling_an_angle() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Aux);
        insert_segment(&mut document.cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Aux);
        let faces = extract_faces(&document.cp);
        let state = FlatState::initial(&document.cp, &faces);
        let result = collapse_precrease_network(
            &mut document.cp,
            &faces,
            &state,
            &PrecreaseCollapseInput {
                lines: vec![[[0.5, 0.0], [0.5, 1.0]], [[0.0, 0.5], [1.0, 0.5]]],
                target_layers: None,
            },
        )
        .unwrap();
        assert!(result.warnings.is_empty());
        assert_eq!(extract_faces(&document.cp).len(), 4);
        assert_eq!(result.state.placements.len(), 4);
        assert!(
            document
                .cp
                .edges
                .iter()
                .all(|edge| edge.kind != EdgeKind::Aux)
        );
    }
}
