//! Jun Maekawa's Devil, book figures 65--103.
//!
//! The drawings in this interval repeatedly expose another packet and reuse
//! the same material precreases.  Consequently none of the selectors below
//! use a screen coordinate or a face id.  A guide is first identified in the
//! material crease pattern, then mapped through the current face placement.
//! Local layer counts (the explicit one/two/four-sheet instructions) are
//! resolved at a point inside the relevant current face.
//!
//! Several figures are compound operations.  Their elementary motions are
//! executed and replay-validated on a clone, then compacted to one persisted
//! book step.  An ambiguous exposed guide is an error: choosing an arbitrary
//! candidate would silently fold a different appendage.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::DVec2;
use ori3_cp::{Face, extract_faces, validate};
use ori3_geometry::align::{
    distance_to_line, existing_line, line_through_points, perpendicular_bisector,
    perpendicular_through_point,
};
use ori3_layers::{
    CreaseOnlyInput, FlatMotionInput, FlatState, FoldDirection, FoldThroughResult, HalfPlane,
    LayerTurn, MotionPart, MotionTransform, RabbitEarInput, TechniqueInput, crease_only,
    flat_motion, flat_state_at, inside_reverse, layers_at_point, layers_from_top_at_point,
    open_sink, point_in_face, rabbit_ear, replay, representative_point, squash,
};
use ori3_model::{CreasePattern, Document, EdgeKind, FaceId, FoldStep, TechniqueKind, VertexId};
use ori3_rigid::max_seam_gap;

const GEOM_EPS: f64 = 1e-8;
const SIDE_EPS: f64 = 1e-9;
const STATE_EPS: f64 = 1e-7;

pub type DevilFoldLine = [[f64; 2]; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevilSide {
    /// Material points with x < y.
    Left,
    /// Material points with x > y.
    Right,
}

impl DevilSide {
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    fn contains(self, point: DVec2) -> bool {
        match self {
            Self::Left => point.x < point.y - SIDE_EPS,
            Self::Right => point.x > point.y + SIDE_EPS,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DevilStepReport {
    pub book_step: u32,
    pub sequence_len: usize,
    pub faces: usize,
    pub max_seam_gap: f64,
}

#[derive(Clone)]
struct Snapshot {
    faces: Vec<Face>,
    state: FlatState,
    positions: HashMap<VertexId, DVec2>,
    ranks: HashMap<FaceId, usize>,
}

#[derive(Clone, Debug)]
struct MappedGuide {
    logical_index: usize,
    face: FaceId,
    edge: u32,
    line: DevilFoldLine,
    sample: [f64; 2],
}

#[derive(Clone)]
struct ProposedMotion {
    cp: CreasePattern,
    result: FoldThroughResult,
    description: String,
}

/// The exact material construction from figures 1--16.
#[must_use]
pub fn devil_material_lines() -> [DevilFoldLine; 22] {
    let sqrt2 = std::f64::consts::SQRT_2;
    let t = sqrt2 - 1.0;
    let q = 2.0 - sqrt2;
    let s = 2.0 * t;
    let e = 2.0 * q - 1.0;
    let a = sqrt2 / 4.0;
    let b = (2.0 + sqrt2) / 4.0;
    let k = 4.0 * t - 1.0;
    [
        [[0.0, 0.0], [1.0, 1.0]],
        [[1.0, 0.0], [0.0, 1.0]],
        [[1.0, 1.0], [0.0, q]],
        [[1.0, 1.0], [q, 0.0]],
        [[0.0, 1.0], [1.0, q]],
        [[1.0, 0.0], [q, 1.0]],
        [[0.0, q], [1.0, q]],
        [[q, 0.0], [q, 1.0]],
        [[0.0, t], [q, 1.0]],
        [[t, 0.0], [1.0, q]],
        [[0.0, t], [t, 0.0]],
        [[q, 1.0], [1.0, q]],
        [[s, 0.0], [t, 1.0]],
        [[0.0, s], [1.0, t]],
        [[q, 0.0], [e, 1.0]],
        [[0.0, q], [1.0, e]],
        [[0.0, s], [s, 0.0]],
        [[e, 1.0], [1.0, e]],
        [[0.0, a], [b, 0.0]],
        [[a, 0.0], [0.0, b]],
        [[0.0, k], [k, 0.0]],
        [[0.0, 0.5], [0.5, 0.0]],
    ]
}

/// Reconstruct the current flat state, rejecting skipped or non-flat history.
pub fn current_flat_state(document: &Document) -> Result<(Vec<Face>, FlatState), String> {
    let faces = extract_faces(&document.cp);
    if faces.is_empty() {
        return Err("Devil state has no faces".to_string());
    }
    let (state, warnings) = flat_state_at(document, &faces, document.sequence.len())?;
    if !warnings.is_empty() {
        return Err(format!("Devil flat-state warnings: {warnings:?}"));
    }
    Ok((faces, state))
}

fn snapshot(document: &Document) -> Result<Snapshot, String> {
    let (faces, state) = current_flat_state(document)?;
    let positions = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let ranks = state
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect();
    Ok(Snapshot {
        faces,
        state,
        positions,
        ranks,
    })
}

fn face_by_id(snapshot: &Snapshot, id: FaceId) -> Result<&Face, String> {
    snapshot
        .faces
        .iter()
        .find(|face| face.id == id)
        .ok_or_else(|| format!("missing face {id}"))
}

fn mapped_polygon(snapshot: &Snapshot, face: &Face) -> Result<Vec<DVec2>, String> {
    let placement = snapshot
        .state
        .placements
        .get(&face.id)
        .ok_or_else(|| format!("face {} has no placement", face.id))?;
    face.vertices
        .iter()
        .map(|vertex| {
            snapshot
                .positions
                .get(vertex)
                .copied()
                .map(|point| placement.apply(point))
                .ok_or_else(|| format!("face {} refers to missing vertex {vertex}", face.id))
        })
        .collect()
}

fn material_side_of_face(snapshot: &Snapshot, cp: &CreasePattern, face: &Face) -> Option<DevilSide> {
    let point = DVec2::from(representative_point(cp, face));
    if DevilSide::Left.contains(point) {
        Some(DevilSide::Left)
    } else if DevilSide::Right.contains(point) {
        Some(DevilSide::Right)
    } else {
        let _ = snapshot;
        None
    }
}

fn edge_on_material_line(
    cp: &CreasePattern,
    positions: &HashMap<VertexId, DVec2>,
    edge_id: u32,
    line: DevilFoldLine,
) -> bool {
    let Some(edge) = cp.edges.iter().find(|edge| edge.id == edge_id) else {
        return false;
    };
    let Some(&a) = positions.get(&edge.v0) else {
        return false;
    };
    let Some(&b) = positions.get(&edge.v1) else {
        return false;
    };
    distance_to_line(line, [a.x, a.y]) <= GEOM_EPS
        && distance_to_line(line, [b.x, b.y]) <= GEOM_EPS
}

fn farthest_pair(points: &[DVec2]) -> Option<(DVec2, DVec2)> {
    let mut best = None;
    for (index, &a) in points.iter().enumerate() {
        for &b in &points[index + 1..] {
            let length = (b - a).length_squared();
            if best
                .as_ref()
                .is_none_or(|(_, _, best_length)| length > *best_length)
            {
                best = Some((a, b, length));
            }
        }
    }
    best.map(|(a, b, _)| (a, b))
}

/// Map one exact material logical line through a specified current face.
pub fn mapped_material_line(
    document: &Document,
    logical_index: usize,
    face_id: FaceId,
) -> Result<DevilFoldLine, String> {
    let snapshot = snapshot(document)?;
    let face = face_by_id(&snapshot, face_id)?;
    let material = *devil_material_lines()
        .get(logical_index)
        .ok_or_else(|| format!("invalid Devil logical line index {logical_index}"))?;
    let placement = snapshot.state.placements[&face_id];
    let mut points = Vec::new();
    for &edge_id in &face.edges {
        if !edge_on_material_line(&document.cp, &snapshot.positions, edge_id, material) {
            continue;
        }
        let edge = document
            .cp
            .edges
            .iter()
            .find(|edge| edge.id == edge_id)
            .ok_or_else(|| format!("missing edge {edge_id}"))?;
        points.push(placement.apply(snapshot.positions[&edge.v0]));
        points.push(placement.apply(snapshot.positions[&edge.v1]));
    }
    let (a, b) = farthest_pair(&points).ok_or_else(|| {
        format!("face {face_id} has no segment on Devil logical line {logical_index}")
    })?;
    existing_line([[a.x, a.y], [b.x, b.y]])
        .ok_or_else(|| format!("mapped logical line {logical_index} is degenerate"))
}

/// Current image of the material symmetry spine x=y (logical line zero).
pub fn devil_center_line(document: &Document) -> Result<DevilFoldLine, String> {
    let snapshot = snapshot(document)?;
    let material = devil_material_lines()[0];
    let mut points = Vec::new();
    for face in &snapshot.faces {
        let placement = snapshot.state.placements[&face.id];
        for &edge_id in &face.edges {
            if !edge_on_material_line(&document.cp, &snapshot.positions, edge_id, material) {
                continue;
            }
            let edge = document
                .cp
                .edges
                .iter()
                .find(|edge| edge.id == edge_id)
                .ok_or_else(|| format!("missing center edge {edge_id}"))?;
            points.push(placement.apply(snapshot.positions[&edge.v0]));
            points.push(placement.apply(snapshot.positions[&edge.v1]));
        }
    }
    let (a, b) = farthest_pair(&points)
        .ok_or_else(|| "the current Devil state has no mapped center spine".to_string())?;
    let line = [[a.x, a.y], [b.x, b.y]];
    for point in &points {
        if distance_to_line(line, [point.x, point.y]) > STATE_EPS {
            return Err("material center-spine fragments do not share one folded line".to_string());
        }
    }
    existing_line(line).ok_or_else(|| "mapped center spine is degenerate".to_string())
}

/// Exact local top-layer selection used by the one/two/four-sheet figures.
pub fn local_top_layers(
    document: &Document,
    point: [f64; 2],
    skip: usize,
    count: usize,
) -> Result<Vec<FaceId>, String> {
    let (faces, state) = current_flat_state(document)?;
    let all = layers_at_point(&document.cp, &faces, &state, point);
    if all.len() < skip + count {
        return Err(format!(
            "point {point:?} has {} local layers; need skip {skip} plus {count}",
            all.len()
        ));
    }
    Ok(layers_from_top_at_point(
        &document.cp,
        &faces,
        &state,
        point,
        skip,
        count,
    ))
}

fn append_generated(
    document: &mut Document,
    mut cp: CreasePattern,
    mut result: FoldThroughResult,
    book_step: u32,
    note: &str,
) -> Result<FlatState, String> {
    if !result.warnings.is_empty() {
        return Err(format!(
            "step {book_step} produced warnings: {:?}",
            result.warnings
        ));
    }
    if !validate(&cp).is_empty() {
        return Err(format!(
            "step {book_step} produced invalid CP: {:?}",
            validate(&cp)
        ));
    }
    result.step.id = u32::try_from(document.sequence.len())
        .map_err(|_| "Devil sequence length does not fit u32".to_string())?;
    result.step.note = format!("手順{book_step}: {note}");
    let state = result.state.clone();
    document.cp = std::mem::take(&mut cp);
    document.sequence.push(result.step);
    Ok(state)
}

fn apply_operation<F>(
    document: &mut Document,
    book_step: u32,
    note: &str,
    operation: F,
) -> Result<FlatState, String>
where
    F: FnOnce(&mut CreasePattern, &[Face], &FlatState) -> Result<FoldThroughResult, String>,
{
    let (faces, state) = current_flat_state(document)?;
    let mut cp = document.cp.clone();
    let result = operation(&mut cp, &faces, &state)?;
    append_generated(document, cp, result, book_step, note)
}

fn verify_step(document: &Document, book_step: u32) -> Result<DevilStepReport, String> {
    let violations = validate(&document.cp);
    if !violations.is_empty() {
        return Err(format!("step {book_step} CP violations: {violations:?}"));
    }
    let faces = extract_faces(&document.cp);
    let replayed = replay(document, document.sequence.len(), 1.0);
    if !replayed.warnings.is_empty()
        || !replayed.skipped.is_empty()
        || !replayed.frame.warnings.is_empty()
    {
        return Err(format!(
            "step {book_step} replay failed: warnings={:?}, skipped={:?}, frame={:?}",
            replayed.warnings, replayed.skipped, replayed.frame.warnings
        ));
    }
    let gap = max_seam_gap(&document.cp, &faces, &replayed.frame);
    if gap >= 1e-6 {
        return Err(format!("step {book_step} max_seam_gap={gap:.9e}"));
    }
    Ok(DevilStepReport {
        book_step,
        sequence_len: document.sequence.len(),
        faces: faces.len(),
        max_seam_gap: gap,
    })
}

