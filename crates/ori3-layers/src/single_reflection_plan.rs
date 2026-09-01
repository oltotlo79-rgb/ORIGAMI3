//! Plans the smallest SIM-011 move: one selected packet reflected in one axis.
//!
//! This module only produces `FlatMotionInput`; it never mutates the caller's
//! crease pattern and does not grant permission to apply a move.

use std::collections::HashSet;

use glam::DVec2;
use ori3_cp::Face;
use ori3_geometry::Isometry2;
use ori3_model::{CreasePattern, EPS, FaceId, FoldDirection, TechniqueKind};

use crate::flat_motion::{FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform};
use crate::flat_state::{FlatState, point_in_face};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectedPoint {
    pub point: [f64; 2],
    pub direction: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SingleReflectionRequest {
    /// An explicit packet. The planner must never broaden this to all layers.
    pub layers: Vec<FaceId>,
    pub source: DirectedPoint,
    pub target: DirectedPoint,
    pub direction: FoldDirection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingleReflectionPlanError {
    EmptySelection,
    DuplicateLayer(FaceId),
    UnknownLayer(FaceId),
    MissingPlacement(FaceId),
    SourceOutsideSelection,
    NonFiniteInput,
    ZeroDirection,
    StationaryTarget,
    NotSingleReflection,
}

/// Returns a one-reflection `Simple` motion only when it exactly maps the
/// source frame to the target frame. `cp` is read-only by contract.
pub fn plan_single_reflection_motion(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    request: &SingleReflectionRequest,
) -> Result<FlatMotionInput, SingleReflectionPlanError> {
    if request.layers.is_empty() {
        return Err(SingleReflectionPlanError::EmptySelection);
    }
    let known: HashSet<FaceId> = faces.iter().map(|face| face.id).collect();
    let mut selected = HashSet::with_capacity(request.layers.len());
    for &id in &request.layers {
        if !selected.insert(id) {
            return Err(SingleReflectionPlanError::DuplicateLayer(id));
        }
        if !known.contains(&id) {
            return Err(SingleReflectionPlanError::UnknownLayer(id));
        }
        if !state.placements.contains_key(&id) {
            return Err(SingleReflectionPlanError::MissingPlacement(id));
        }
    }

    let source = point(request.source.point)?;
    let target = point(request.target.point)?;
    let source_direction = unit(request.source.direction)?;
    let target_direction = unit(request.target.direction)?;
    let displacement = target - source;
    if displacement.length() <= EPS {
        return Err(SingleReflectionPlanError::NotSingleReflection);
    }
    let source_is_on_selection = faces.iter().any(|face| {
        selected.contains(&face.id) && {
            let local = state.placements[&face.id].inverse().apply(source);
            point_in_face(cp, face, local.to_array())
        }
    });
    if !source_is_on_selection {
        return Err(SingleReflectionPlanError::SourceOutsideSelection);
    }

    let sum = source_direction + target_direction;
    let axis = if sum.length() > EPS {
        sum.normalize()
    } else {
        DVec2::new(-source_direction.y, source_direction.x)
    };
    if displacement.dot(axis).abs() > EPS * displacement.length().max(1.0) {
        return Err(SingleReflectionPlanError::NotSingleReflection);
    }
    let midpoint = (source + target) * 0.5;
    let line = [midpoint.to_array(), (midpoint + axis).to_array()];
    let reflection = Isometry2::reflection(midpoint, midpoint + axis);
    let mapped_point = reflection.apply(source);
    let mapped_direction = reflection.apply(source + source_direction) - mapped_point;
    if (mapped_point - target).length() > EPS
        || (mapped_direction.normalize_or_zero() - target_direction).length() > EPS
    {
        return Err(SingleReflectionPlanError::NotSingleReflection);
    }

    Ok(FlatMotionInput {
        parts: vec![MotionPart {
            layers: request.layers.clone(),
            region: vec![HalfPlane {
                line,
                inside_point: request.source.point,
            }],
            transform: MotionTransform::Reflect(vec![line]),
            turn: LayerTurn::Outside(request.direction),
            reverse_layers: None,
        }],
        kind: TechniqueKind::Simple,
    })
}

fn point(value: [f64; 2]) -> Result<DVec2, SingleReflectionPlanError> {
    if value.iter().all(|component| component.is_finite()) {
        Ok(DVec2::from(value))
    } else {
        Err(SingleReflectionPlanError::NonFiniteInput)
    }
}

fn unit(value: [f64; 2]) -> Result<DVec2, SingleReflectionPlanError> {
    let vector = point(value)?;
    if vector.length() <= EPS {
        Err(SingleReflectionPlanError::ZeroDirection)
    } else {
        Ok(vector.normalize())
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use ori3_cp::extract_faces;
    use ori3_geometry::Isometry2;
    use ori3_model::{Document, Paper, TechniqueKind};

    use super::{
        DirectedPoint, SingleReflectionPlanError, SingleReflectionRequest,
        plan_single_reflection_motion,
    };
    use crate::flat_motion::{MotionTransform, flat_motion};
    use crate::flat_state::{FlatState, point_in_face, representative_point};
    use crate::fold_through::{FoldDirection, FoldThroughInput, fold_through};

    fn square() -> Document {
        Document::new(Paper { width_mm: 100.0, height_mm: 100.0 })
    }

    fn two_layer_packet() -> (
        ori3_model::CreasePattern,
        Vec<ori3_cp::Face>,
        FlatState,
        ori3_model::FaceId,
        ori3_model::FaceId,
    ) {
        let mut cp = square().cp;
        let faces = extract_faces(&cp);
        let initial = FlatState::initial(&cp, &faces);
        let folded = fold_through(&mut cp, &faces, &initial, &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            keep_side_point: [0.25, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        }).expect("build two-layer packet");
        let faces = extract_faces(&cp);
        let selected = *folded.state.order.last().expect("top layer");
        let unselected = *folded.state.order.first().expect("bottom layer");
        (cp, faces, folded.state, selected, unselected)
    }

    fn request(
        cp: &ori3_model::CreasePattern,
        faces: &[ori3_cp::Face],
        state: &FlatState,
        layer: ori3_model::FaceId,
    ) -> SingleReflectionRequest {
        let face = faces.iter().find(|face| face.id == layer).unwrap();
        let local = DVec2::from(representative_point(cp, face));
        let source = state.placements[&layer].apply(local);
        let source_direction = (state.placements[&layer].apply(local + DVec2::X) - source).normalize();
        SingleReflectionRequest {
            layers: vec![layer],
            source: DirectedPoint { point: source.to_array(), direction: source_direction.to_array() },
            target: DirectedPoint {
                point: [1.0 - source.x, source.y],
                direction: [-source_direction.x, source_direction.y],
            },
            direction: FoldDirection::Up,
        }
    }

    #[test]
    fn plans_the_axis_that_maps_the_selected_layer_to_its_target_frame() {
        let (cp, faces, state, selected, _) = two_layer_packet();
        let request = request(&cp, &faces, &state, selected);
        let plan = plan_single_reflection_motion(&cp, &faces, &state, &request).unwrap();
        assert_eq!(plan.kind, TechniqueKind::Simple);
        let MotionTransform::Reflect(lines) = &plan.parts[0].transform else { panic!("not a reflection") };
        assert_eq!(lines.len(), 1);
        assert!((lines[0][0][0] - 0.5).abs() <= 1e-9);
        assert!((lines[0][1][0] - 0.5).abs() <= 1e-9);
        let reflection = Isometry2::reflection(DVec2::from(lines[0][0]), DVec2::from(lines[0][1]));
        let source = DVec2::from(request.source.point);
        let direction = DVec2::from(request.source.direction);
        let mapped = reflection.apply(source);
        let mapped_direction = reflection.apply(source + direction) - mapped;
        assert!((mapped - DVec2::from(request.target.point)).length() <= 1e-9);
        assert!((mapped_direction.normalize() - DVec2::from(request.target.direction)).length() <= 1e-9);
    }

    #[test]
    fn rejects_a_target_frame_that_requires_more_than_one_reflection() {
        let (cp, faces, state, selected, _) = two_layer_packet();
        let mut impossible = request(&cp, &faces, &state, selected);
        impossible.target.direction = [0.0, 1.0];
        assert!(matches!(
            plan_single_reflection_motion(&cp, &faces, &state, &impossible),
            Err(SingleReflectionPlanError::NotSingleReflection)
        ));
    }

    #[test]
    fn leaves_the_unselected_layer_stationary_and_never_expands_to_it() {
        let (cp, faces, state, selected, unselected) = two_layer_packet();
        let plan = plan_single_reflection_motion(&cp, &faces, &state, &request(&cp, &faces, &state, selected)).unwrap();
        assert_eq!(plan.parts.len(), 1);
        assert_eq!(plan.parts[0].layers, vec![selected]);
        assert!(!plan.parts[0].layers.contains(&unselected));
        let unselected_face = faces.iter().find(|face| face.id == unselected).unwrap();
        let probe = representative_point(&cp, unselected_face);
        let before = state.placements[&unselected].apply(DVec2::from(probe));
        let mut after_cp = cp.clone();
        let result = flat_motion(&mut after_cp, &faces, &state, &plan).unwrap();
        let after_face = extract_faces(&after_cp).into_iter()
            .find(|face| point_in_face(&after_cp, face, probe)).unwrap();
        let after = result.state.placements[&after_face.id].apply(DVec2::from(probe));
        assert!((after - before).length() <= 1e-9);
    }

    #[test]
    fn planning_does_not_change_the_crease_pattern() {
        let (cp, faces, state, selected, _) = two_layer_packet();
        let before = cp.clone();
        let _ = plan_single_reflection_motion(&cp, &faces, &state, &request(&cp, &faces, &state, selected)).unwrap();
        assert_eq!(cp, before);
    }

    #[test]
    fn existing_fold_through_still_records_a_simple_reflection_step() {
        let mut cp = square().cp;
        let faces = extract_faces(&cp);
        let state = FlatState::initial(&cp, &faces);
        let result = fold_through(&mut cp, &faces, &state, &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]], keep_side_point: [0.25, 0.5],
            target_layers: None, direction: FoldDirection::Up,
        }).unwrap();
        assert_eq!(result.step.kind, TechniqueKind::Simple);
        assert!(result.state.placements.values().any(|placement| placement.mirrored));
    }
}
