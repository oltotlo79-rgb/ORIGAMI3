//! Transactional composition of several validated flat-fold operations into one step.
//!
//! A compound step is not an escape hatch for storing arbitrary final face poses.  Every
//! intermediate operation is first executed on a private [`Document`], and both its flat state
//! and its recorded [`FoldStep`] are replayed and checked.  Only then are the resulting face
//! isometries composed into the single [`TechniqueKind::Simple`] step returned to the caller.

use std::collections::{HashMap, HashSet};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_model::{CreasePattern, Document, EdgeId, EdgeKind, FaceId, FoldStep, TechniqueKind};

use crate::crease_only::{
    CreaseOnlyInput, ReverseOpenCreaseInput, crease_only, reverse_open_crease_sense,
};
use crate::flat_motion::{FlatMotionInput, LayerTurn, MotionPart, MotionTransform, flat_motion};
use crate::flat_state::{FlatState, point_in_face, representative_point};
use crate::fold_network::{ReverseFoldNetworkInput, reverse_fold_network};
use crate::fold_through::{
    FoldDirection, FoldThroughInput, FoldThroughResult, fold_through,
    fold_through_with_additional_crease,
};
use crate::rabbit_ear::{RabbitEarInput, rabbit_ear};
use crate::replay::{flat_state_at, replay};
use crate::techniques::{
    TechniqueInput, inside_reverse, open_sink, outside_reverse, petal, pleat, squash, swivel, twist,
};

const STATE_EPS: f64 = 1e-7;
const AREA_EPS: f64 = 1e-8;

/// A named, validated macro operation that can participate in a compound step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompoundTechnique {
    Pleat,
    InsideReverse,
    OutsideReverse,
    Petal,
    Squash,
    OpenSink,
    Swivel,
    Twist,
}

/// Private working document used while constructing a compound flat-fold step.
///
/// Instances are supplied by [`compose_flat_motion_step`].  Each `apply_*` method is
/// transactional: the operation, its recorded step, replay, warning set, and face coverage are
/// checked before the session advances.
pub struct CompoundMotionSession {
    document: Document,
    faces: Vec<Face>,
    state: FlatState,
    applied_steps: usize,
}

impl CompoundMotionSession {
    fn new(document: &Document) -> Result<Self, String> {
        let faces = extract_faces(&document.cp);
        let state = validate_document(document, &faces, "compound step start")?;
        Ok(Self {
            document: document.clone(),
            faces,
            state,
            applied_steps: 0,
        })
    }

    /// Current temporary document, including all operations accepted so far.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Current crease pattern in the temporary document.
    #[must_use]
    pub fn crease_pattern(&self) -> &CreasePattern {
        &self.document.cp
    }

    /// Faces extracted from [`Self::crease_pattern`].
    #[must_use]
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    /// Validated flat state after all operations accepted so far.
    #[must_use]
    pub fn state(&self) -> &FlatState {
        &self.state
    }

    /// Number of elementary operations accepted by this session.
    #[must_use]
    pub fn applied_steps(&self) -> usize {
        self.applied_steps
    }

    /// Apply one general flat motion to the temporary document.
    pub fn apply_flat_motion(&mut self, input: &FlatMotionInput) -> Result<&FlatState, String> {
        self.apply_generated(|cp, faces, state| flat_motion(cp, faces, state, input))
    }

    /// Crease selected local layers and unfold them again inside the compound operation.
    pub fn apply_crease_only(&mut self, input: &CreaseOnlyInput) -> Result<&FlatState, String> {
        self.apply_generated(|cp, faces, state| crease_only(cp, faces, state, input))
    }

    /// Reverse selected currently-open material crease segments without moving the paper.
    pub fn apply_reverse_open_crease_sense(
        &mut self,
        input: &ReverseOpenCreaseInput,
    ) -> Result<&FlatState, String> {
        self.apply_generated(|cp, faces, state| reverse_open_crease_sense(cp, faces, state, input))
    }

    /// Reverse a validated local folded network without changing its outline.
    pub fn apply_reverse_fold_network(
        &mut self,
        input: &ReverseFoldNetworkInput,
    ) -> Result<&FlatState, String> {
        self.apply_generated(|cp, faces, state| reverse_fold_network(cp, faces, state, input))
    }

    /// Apply one three-crease rabbit-ear collapse to the temporary document.
    pub fn apply_rabbit_ear(&mut self, input: &RabbitEarInput) -> Result<&FlatState, String> {
        self.apply_generated(|cp, faces, state| rabbit_ear(cp, faces, state, input))
    }

    /// Apply one ordinary fold-through operation to the temporary document.
    pub fn apply_fold_through(&mut self, input: &FoldThroughInput) -> Result<&FlatState, String> {
        self.apply_generated(|cp, faces, state| fold_through(cp, faces, state, input))
    }

    /// Apply one fold-through operation, optionally accepting its additional-crease proposal.
    pub fn apply_fold_through_with_additional_crease(
        &mut self,
        input: &FoldThroughInput,
        accept_additional_crease: bool,
    ) -> Result<&FlatState, String> {
        self.apply_generated(|cp, faces, state| {
            fold_through_with_additional_crease(cp, faces, state, input, accept_additional_crease)
        })
    }

    /// Apply one of the built-in technique macros to the temporary document.
    pub fn apply_technique(
        &mut self,
        technique: CompoundTechnique,
        input: &TechniqueInput,
    ) -> Result<&FlatState, String> {
        self.apply_generated(|cp, faces, state| match technique {
            CompoundTechnique::Pleat => pleat(cp, faces, state, input),
            CompoundTechnique::InsideReverse => inside_reverse(cp, faces, state, input),
            CompoundTechnique::OutsideReverse => outside_reverse(cp, faces, state, input),
            CompoundTechnique::Petal => petal(cp, faces, state, input),
            CompoundTechnique::Squash => squash(cp, faces, state, input),
            CompoundTechnique::OpenSink => open_sink(cp, faces, state, input),
            CompoundTechnique::Swivel => swivel(cp, faces, state, input),
            CompoundTechnique::Twist => twist(cp, faces, state, input),
        })
    }

    /// Append and validate a recorded step that uses creases already present in the current CP.
    ///
    /// This is useful when a caller already has a persistent [`FoldStep`] rather than an operation
    /// input.  It cannot add or remove CP geometry; driver resolution and full replay must succeed.
    pub fn apply_fold_step(&mut self, mut step: FoldStep) -> Result<&FlatState, String> {
        step.id = next_step_id(&self.document)?;
        let mut candidate = self.document.clone();
        candidate.sequence.push(step);
        let state = validate_document(&candidate, &self.faces, "recorded compound operation")?;
        validate_face_coverage(
            &self.document.cp,
            &self.faces,
            &candidate.cp,
            &self.faces,
            "recorded compound operation",
        )?;
        self.document = candidate;
        self.state = state;
        self.applied_steps += 1;
        Ok(&self.state)
    }

    fn apply_generated<F>(&mut self, operation: F) -> Result<&FlatState, String>
    where
        F: FnOnce(&mut CreasePattern, &[Face], &FlatState) -> Result<FoldThroughResult, String>,
    {
        let mut cp = self.document.cp.clone();
        let mut result = operation(&mut cp, &self.faces, &self.state)?;
        if !result.warnings.is_empty() {
            return Err(format!(
                "compound operation produced warnings: {:?}",
                result.warnings
            ));
        }

        let next_faces = extract_faces(&cp);
        validate_face_coverage(
            &self.document.cp,
            &self.faces,
            &cp,
            &next_faces,
            "compound operation",
        )?;
        validate_state_faces(&result.state, &next_faces, "compound operation result")?;

        result.step.id = next_step_id(&self.document)?;
        let mut candidate = self.document.clone();
        candidate.cp = cp;
        candidate.sequence.push(result.step);

        let replayed = validate_document(&candidate, &next_faces, "compound operation replay")?;
        validate_state_matches(
            &replayed,
            &result.state,
            &next_faces,
            "compound operation replay",
        )?;

        self.document = candidate;
        self.faces = next_faces;
        self.state = replayed;
        self.applied_steps += 1;
        Ok(&self.state)
    }
}

/// Compose a sequence of validated flat-fold operations into one persistent simple step.
///
/// `build` runs against a private document through [`CompoundMotionSession`].  The final
/// isometry for every resulting face is derived from that validated sequence, never supplied by
/// the caller.  CP subdivisions and crease-kind changes produced by the sequence are retained.
/// The returned [`FoldThroughResult::step`] always has [`TechniqueKind::Simple`].
///
/// As with [`flat_motion`], this function commits only the CP.  The caller assigns the returned
/// step's ID and appends it to `document.sequence`.  Any error leaves `document` unchanged.
pub fn compose_flat_motion_step<F>(
    document: &mut Document,
    build: F,
) -> Result<FoldThroughResult, String>
where
    F: FnOnce(&mut CompoundMotionSession) -> Result<(), String>,
{
    let original = document.clone();
    let original_faces = extract_faces(&original.cp);
    let mut session = CompoundMotionSession::new(&original)?;
    build(&mut session)?;
    if session.applied_steps == 0 {
        return Err("a compound step must contain at least one validated operation".to_string());
    }

    let target_cp = session.document.cp.clone();
    let target_faces = session.faces.clone();
    let target = session.state.clone();
    validate_face_coverage(
        &original.cp,
        &original_faces,
        &target_cp,
        &target_faces,
        "complete compound sequence",
    )?;

    // Re-evaluate the starting sequence on the final CP subdivision.  This gives both ends of
    // every final face's exact composed isometry in the same face-ID coordinate system.
    let mut start_on_final_cp = original.clone();
    start_on_final_cp.cp = target_cp.clone();
    let current = validate_document(
        &start_on_final_cp,
        &target_faces,
        "compound start on final CP",
    )?;

    let parts = target
        .order
        .iter()
        .map(|face| {
            let from = current
                .placements
                .get(face)
                .ok_or_else(|| format!("compound start is missing face {face}"))?;
            let to = target
                .placements
                .get(face)
                .ok_or_else(|| format!("compound target is missing face {face}"))?;
            Ok(MotionPart {
                layers: vec![*face],
                region: Vec::new(),
                transform: MotionTransform::Isometry(to.compose(&from.inverse())),
                turn: LayerTurn::Outside(FoldDirection::Up),
                reverse_layers: Some(false),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    // If a folded crease stayed folded while its incident layers exchanged order an odd number
    // of times, seed the inverse kind.  flat_motion's normal settling pass then records the exact
    // final mountain/valley kind produced by the validated elementary operations.
    let mut compact_cp = target_cp.clone();
    seed_exchanged_fold_kinds(&mut compact_cp, &target_faces, &current, &target)?;
    let mut compact = flat_motion(
        &mut compact_cp,
        &target_faces,
        &current,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Simple,
        },
    )?;
    if !compact.warnings.is_empty() {
        return Err(format!(
            "compacted motion produced warnings: {:?}",
            compact.warnings
        ));
    }
    if compact_cp != target_cp {
        return Err(
            "compacted motion did not reproduce the validated final crease pattern".to_string(),
        );
    }
    validate_state_matches(
        &compact.state,
        &target,
        &target_faces,
        "compacted motion result",
    )?;
    compact.step.kind = TechniqueKind::Simple;

    let initial_edges = original
        .cp
        .edges
        .iter()
        .map(|edge| (edge.id, edge.kind))
        .collect::<HashMap<_, _>>();
    compact.added_edges = compact_cp
        .edges
        .iter()
        .filter(|edge| initial_edges.get(&edge.id) != Some(&edge.kind))
        .map(|edge| edge.id)
        .collect();
    compact.added_edges.sort_unstable();
    compact.added_edges.dedup();

    // Validate the actual one-step artifact, not only the direct flat-motion result.
    let mut one_step = original.clone();
    one_step.cp = compact_cp.clone();
    compact.step.id = next_step_id(&one_step)?;
    one_step.sequence.push(compact.step.clone());
    let replayed = validate_document(&one_step, &target_faces, "compacted one-step replay")?;
    validate_state_matches(
        &replayed,
        &target,
        &target_faces,
        "compacted one-step replay",
    )?;

    document.cp = compact_cp;
    Ok(compact)
}

fn next_step_id(document: &Document) -> Result<u32, String> {
    u32::try_from(document.sequence.len())
        .map_err(|_| "fold sequence length does not fit in a step ID".to_string())
}

fn validate_document(
    document: &Document,
    faces: &[Face],
    label: &str,
) -> Result<FlatState, String> {
    let (state, flat_warnings) = flat_state_at(document, faces, document.sequence.len())?;
    if !flat_warnings.is_empty() {
        return Err(format!("{label}: flat-state warnings: {flat_warnings:?}"));
    }
    validate_state_faces(&state, faces, label)?;

    let replayed = replay(document, document.sequence.len(), 1.0);
    if !replayed.skipped.is_empty() {
        return Err(format!(
            "{label}: replay skipped steps: {:?}",
            replayed.skipped
        ));
    }
    if !replayed.warnings.is_empty() {
        return Err(format!("{label}: replay warnings: {:?}", replayed.warnings));
    }
    if !replayed.frame.warnings.is_empty() {
        return Err(format!(
            "{label}: frame warnings: {:?}",
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
            "{label}: replay did not retain every face (expected {}, got {})",
            faces.len(),
            replayed.frame.faces.len()
        ));
    }
    Ok(state)
}

fn validate_state_faces(state: &FlatState, faces: &[Face], label: &str) -> Result<(), String> {
    let expected = faces.iter().map(|face| face.id).collect::<HashSet<_>>();
    let placements = state.placements.keys().copied().collect::<HashSet<_>>();
    let order = state.order.iter().copied().collect::<HashSet<_>>();
    if state.placements.len() != faces.len() || placements != expected {
        return Err(format!(
            "{label}: placement map does not contain every face exactly once"
        ));
    }
    if state.order.len() != faces.len() || order.len() != state.order.len() || order != expected {
        return Err(format!(
            "{label}: layer order does not contain every face exactly once"
        ));
    }
    Ok(())
}

fn validate_state_matches(
    actual: &FlatState,
    expected: &FlatState,
    faces: &[Face],
    label: &str,
) -> Result<(), String> {
    validate_state_faces(actual, faces, label)?;
    validate_state_faces(expected, faces, label)?;
    if actual.order != expected.order {
        return Err(format!(
            "{label}: layer order differs from the validated sequence"
        ));
    }
    for face in faces {
        if !actual.placements[&face.id].approx_eq(&expected.placements[&face.id], STATE_EPS) {
            return Err(format!(
                "{label}: face {} placement differs from the validated sequence",
                face.id
            ));
        }
    }
    Ok(())
}

fn validate_face_coverage(
    before_cp: &CreasePattern,
    before: &[Face],
    after_cp: &CreasePattern,
    after: &[Face],
    label: &str,
) -> Result<(), String> {
    if before.is_empty() || after.is_empty() {
        return Err(format!("{label}: face extraction returned no paper faces"));
    }

    let mut child_area = before
        .iter()
        .map(|face| (face.id, 0.0))
        .collect::<HashMap<_, _>>();
    let mut child_count = before
        .iter()
        .map(|face| (face.id, 0usize))
        .collect::<HashMap<_, _>>();
    for child in after {
        let point = representative_point(after_cp, child);
        let parents = before
            .iter()
            .filter(|parent| point_in_face(before_cp, parent, point))
            .collect::<Vec<_>>();
        if parents.len() != 1 {
            return Err(format!(
                "{label}: final face {} has {} parent faces",
                child.id,
                parents.len()
            ));
        }
        let parent = parents[0].id;
        *child_count
            .get_mut(&parent)
            .expect("parent was initialized") += 1;
        *child_area.get_mut(&parent).expect("parent was initialized") +=
            face_area(after_cp, child)?;
    }

    for parent in before {
        let count = child_count[&parent.id];
        if count == 0 {
            return Err(format!("{label}: original face {} was lost", parent.id));
        }
        let expected = face_area(before_cp, parent)?;
        let actual = child_area[&parent.id];
        let tolerance = AREA_EPS.max(expected * AREA_EPS);
        if (actual - expected).abs() > tolerance {
            return Err(format!(
                "{label}: descendants of face {} cover area {actual:.12}, expected {expected:.12}",
                parent.id
            ));
        }
    }
    Ok(())
}

fn face_area(cp: &CreasePattern, face: &Face) -> Result<f64, String> {
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let points = face
        .vertices
        .iter()
        .map(|vertex| {
            positions
                .get(vertex)
                .copied()
                .ok_or_else(|| format!("face {} refers to missing vertex {vertex}", face.id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f64>();
    Ok(twice_area.abs() * 0.5)
}

fn seed_exchanged_fold_kinds(
    cp: &mut CreasePattern,
    faces: &[Face],
    current: &FlatState,
    target: &FlatState,
) -> Result<(), String> {
    let mut owners: HashMap<EdgeId, Vec<FaceId>> = HashMap::new();
    for face in faces {
        for edge in &face.edges {
            owners.entry(*edge).or_default().push(face.id);
        }
    }
    let current_rank = current
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let target_rank = target
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();

    for edge in &mut cp.edges {
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let Some(adjacent) = owners.get(&edge.id).filter(|faces| faces.len() == 2) else {
            continue;
        };
        let (a, b) = (adjacent[0], adjacent[1]);
        let ca = current
            .placements
            .get(&a)
            .ok_or_else(|| format!("compound start is missing face {a}"))?;
        let cb = current
            .placements
            .get(&b)
            .ok_or_else(|| format!("compound start is missing face {b}"))?;
        let ta = target
            .placements
            .get(&a)
            .ok_or_else(|| format!("compound target is missing face {a}"))?;
        let tb = target
            .placements
            .get(&b)
            .ok_or_else(|| format!("compound target is missing face {b}"))?;
        if ca.mirrored == cb.mirrored || ta.mirrored == tb.mirrored {
            continue;
        }
        let current_kind = expected_kind(current_rank[&a], current_rank[&b], ca.mirrored);
        let target_kind = expected_kind(target_rank[&a], target_rank[&b], ta.mirrored);
        if current_kind != target_kind {
            edge.kind = opposite_kind(edge.kind);
        }
    }
    Ok(())
}

fn expected_kind(rank_a: usize, rank_b: usize, a_mirrored: bool) -> EdgeKind {
    if (rank_b > rank_a) == a_mirrored {
        EdgeKind::Mountain
    } else {
        EdgeKind::Valley
    }
}

fn opposite_kind(kind: EdgeKind) -> EdgeKind {
    match kind {
        EdgeKind::Mountain => EdgeKind::Valley,
        EdgeKind::Valley => EdgeKind::Mountain,
        other => other,
    }
}
