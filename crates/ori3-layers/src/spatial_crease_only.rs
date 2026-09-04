//! Add a material crease to only the top surface of a non-flat pose.
//!
//! This module deliberately accepts material (crease-pattern) coordinates.
//! A screen-space line or a live 3D frame is not a stable document input: the
//! canonical pose is derived separately from the document and signed drivers.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_model::{
    CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, Face3D, FaceId, FoldDirection, FoldStep,
    Frame3D, TechniqueKind, VertexId,
};

use crate::fold_through::{flat_fold_kind, resolve_driver_edges};
use crate::plane_pullback::clip_material_line_to_face;
use crate::{point_in_face, representative_point};

/// A material-space crease requested for the top surface only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialCreaseOnlyInput {
    /// The new crease in crease-pattern coordinates.
    pub material_line: [[f64; 2]; 2],
    /// A strict interior material point on the side retained by the operation.
    pub material_keep_side_point: [f64; 2],
    /// Mountain/valley sense recorded for the new material crease.
    pub direction: FoldDirection,
}

/// A stable material vertex and its position in a document-derived 3D pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialVertex3D {
    pub vertex: VertexId,
    pub position: [f64; 3],
}

/// The rigid material-to-world isometry reconstructed for one source face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceRigidTransform3 {
    pub face: FaceId,
    pub material_origin: [f64; 2],
    pub world_origin: [f64; 3],
    pub world_x_axis: [f64; 3],
    pub world_y_axis: [f64; 3],
}

/// A non-flat pose reconstructed only from a document prefix and signed input.
#[derive(Clone, Debug)]
pub struct CanonicalNonflatPose {
    pub frame: Frame3D,
    pub material_vertices: Vec<MaterialVertex3D>,
    pub face_transforms: Vec<FaceRigidTransform3>,
    /// Signed declarations are retained verbatim; opposite signs are not
    /// periodicized or replaced with a solved endpoint.
    pub signed_hinge_angles: Vec<(EdgeId, f64)>,
}

/// The observed relation between one material surface and the next one down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfaceRelationFromTop {
    /// A finite signed angle other than zero or either complete-fold endpoint.
    Incomplete {
        signed_angle_deg: f64,
    },
    CompletePositive180,
    CompleteNegative180,
    Zero,
    Missing,
    Ambiguous,
}

/// The top material surface and its first relation to the paper below it.
#[derive(Clone, Debug, PartialEq)]
pub struct TopSurfaceObservation {
    /// Internal, document-derived face IDs. These are never accepted over IPC.
    pub surface_faces: Vec<FaceId>,
    pub relation_to_next: SurfaceRelationFromTop,
}

/// Supplies surface relations from the reconstructed document pose.
///
/// A crease-only operation may request depth zero exactly once. Once that
/// relation is incomplete, the user decision requires it not to inspect any
/// deeper surface.
pub trait TopSurfaceProvider {
    fn observe_from_top(
        &mut self,
        depth: usize,
    ) -> Result<TopSurfaceObservation, SpatialCreaseOnlyError>;
}

/// One vertex added by the crease and the source material face containing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewMaterialVertex {
    pub vertex: VertexId,
    pub parent_face: FaceId,
}

/// A successful top-surface crease without any rigid motion.
#[derive(Clone, Debug)]
pub struct SpatialCreaseOnlyResult {
    pub cp: CreasePattern,
    pub faces: Vec<Face>,
    pub frame: Frame3D,
    /// Includes every pre-existing material vertex at bit-identical positions,
    /// followed by any vertices introduced on the crease.
    pub material_vertices: Vec<MaterialVertex3D>,
    /// Every source-face transform is retained bit-for-bit. Children created by
    /// splitting may add entries, but may not replace these source entries.
    pub source_face_transforms: Vec<FaceRigidTransform3>,
    pub new_vertices: Vec<NewMaterialVertex>,
    pub added_edges: Vec<EdgeId>,
    /// All drivers in this step finish at explicit zero degrees.
    pub step: FoldStep,
}

/// A fail-closed reason for refusing to guess a material surface or line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpatialCreaseOnlyError {
    NotImplemented,
    DegenerateMaterialLine,
    NonFiniteInput,
    MaterialKeepSidePointOnBoundary,
    MaterialKeepSidePointOutsidePaper,
    AmbiguousTopSurface,
    MaterialLineMismatchAcrossSurfaceFaces,
    MissingTopRelation,
    ZeroTopRelation,
    CompleteTopRelation,
    InvalidTopRelation,
    PartialInsertion,
}

impl std::fmt::Display for SpatialCreaseOnlyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for SpatialCreaseOnlyError {}

/// Insert a zero-degree crease into only the incomplete top material surface.
///
pub fn crease_only_top_from_material_line(
    cp: &CreasePattern,
    faces: &[Face],
    pose: &CanonicalNonflatPose,
    input: &SpatialCreaseOnlyInput,
    surface_provider: &mut dyn TopSurfaceProvider,
) -> Result<SpatialCreaseOnlyResult, SpatialCreaseOnlyError> {
    validate_input(input)?;

    // User decision: inspect the top relation exactly once. If it is
    // incomplete, do not ask the provider about anything below it.
    let observation = surface_provider.observe_from_top(0)?;
    validate_top_relation(observation.relation_to_next)?;
    let selected = validate_surface_group(cp, faces, pose, &observation.surface_faces)?;
    validate_keep_side_point(cp, faces, &selected, input.material_keep_side_point)?;

    let face_map = faces
        .iter()
        .map(|face| (face.id, face))
        .collect::<HashMap<_, _>>();
    let mut material_segments = Vec::new();
    for face_id in &observation.surface_faces {
        let face = face_map
            .get(face_id)
            .copied()
            .ok_or(SpatialCreaseOnlyError::AmbiguousTopSurface)?;
        let segments = clip_material_line_to_face(cp, face, input.material_line)
            .map_err(|_| SpatialCreaseOnlyError::NonFiniteInput)?;
        for segment in segments {
            material_segments.push(MaterialFaceSegment {
                parent_face: *face_id,
                segment,
            });
        }
    }
    if material_segments.is_empty() {
        return Err(SpatialCreaseOnlyError::PartialInsertion);
    }
    sort_material_segments(&mut material_segments, input.material_line);
    validate_world_line(pose, faces, &material_segments)?;

    let mirrored = unique_surface_mirroring(pose, &observation.surface_faces)?;
    let crease_kind = flat_fold_kind(Some(input.direction), mirrored);
    let before_vertex_ids = cp
        .vertices
        .iter()
        .map(|vertex| vertex.id)
        .collect::<BTreeSet<_>>();
    let before_edge_ids = cp.edges.iter().map(|edge| edge.id).collect::<BTreeSet<_>>();
    let mut work = cp.clone();
    let mut new_vertex_parents = BTreeMap::<VertexId, FaceId>::new();

    for material_segment in &material_segments {
        let vertices_before_segment = work
            .vertices
            .iter()
            .map(|vertex| vertex.id)
            .collect::<BTreeSet<_>>();
        insert_segment(
            &mut work,
            material_segment.segment[0],
            material_segment.segment[1],
            crease_kind,
        );
        for vertex in &work.vertices {
            if !vertices_before_segment.contains(&vertex.id) {
                new_vertex_parents
                    .entry(vertex.id)
                    .or_insert(material_segment.parent_face);
            }
        }
    }

    promote_aux_segments(&mut work, &material_segments, crease_kind);
    let drivers = material_segments
        .iter()
        .map(|material_segment| DriverLine {
            a: material_segment.segment[0],
            b: material_segment.segment[1],
            target_angle_deg: 0.0,
        })
        .collect::<Vec<_>>();
    if drivers
        .iter()
        .any(|driver| !driver_is_fully_covered(&work, driver))
    {
        return Err(SpatialCreaseOnlyError::PartialInsertion);
    }

    let new_vertices = validate_new_vertices(
        cp,
        &work,
        &face_map,
        &selected,
        &before_vertex_ids,
        new_vertex_parents,
    )?;
    let material_vertices = unchanged_and_new_world_vertices(pose, &work, &new_vertices)?;
    let new_faces = extract_faces(&work);
    let frame =
        unchanged_frame_for_split_faces(cp, faces, pose, &work, &new_faces, &material_vertices)?;
    let added_edges = work
        .edges
        .iter()
        .filter(|edge| !before_edge_ids.contains(&edge.id))
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .filter(|edge| edge_on_any_segment(&work, edge.v0, edge.v1, &material_segments))
        .map(|edge| edge.id)
        .collect();
    let step = FoldStep {
        id: 0,
        kind: TechniqueKind::Simple,
        drivers,
        layer_order: None,
        alignment: None,
        finish_soft: None,
        note: String::new(),
        technique_classification: None,
    };

    Ok(SpatialCreaseOnlyResult {
        cp: work,
        faces: new_faces,
        frame,
        material_vertices,
        source_face_transforms: pose.face_transforms.clone(),
        new_vertices,
        added_edges,
        step,
    })
}

#[derive(Clone, Copy)]
struct MaterialFaceSegment {
    parent_face: FaceId,
    segment: [[f64; 2]; 2],
}

fn validate_input(input: &SpatialCreaseOnlyInput) -> Result<(), SpatialCreaseOnlyError> {
    let start = DVec2::from(input.material_line[0]);
    let end = DVec2::from(input.material_line[1]);
    let keep = DVec2::from(input.material_keep_side_point);
    if !finite2(start) || !finite2(end) || !finite2(keep) {
        return Err(SpatialCreaseOnlyError::NonFiniteInput);
    }
    if (end - start).length() <= EPS {
        return Err(SpatialCreaseOnlyError::DegenerateMaterialLine);
    }
    Ok(())
}

fn validate_top_relation(relation: SurfaceRelationFromTop) -> Result<(), SpatialCreaseOnlyError> {
    match relation {
        SurfaceRelationFromTop::Incomplete { signed_angle_deg } => {
            if !signed_angle_deg.is_finite() {
                return Err(SpatialCreaseOnlyError::InvalidTopRelation);
            }
            if signed_angle_deg == 0.0 || signed_endpoint(signed_angle_deg).is_some() {
                return Err(SpatialCreaseOnlyError::InvalidTopRelation);
            }
            Ok(())
        }
        SurfaceRelationFromTop::Missing => Err(SpatialCreaseOnlyError::MissingTopRelation),
        SurfaceRelationFromTop::Zero => Err(SpatialCreaseOnlyError::ZeroTopRelation),
        SurfaceRelationFromTop::CompletePositive180
        | SurfaceRelationFromTop::CompleteNegative180 => {
            Err(SpatialCreaseOnlyError::CompleteTopRelation)
        }
        SurfaceRelationFromTop::Ambiguous => Err(SpatialCreaseOnlyError::InvalidTopRelation),
    }
}

fn signed_endpoint(angle_deg: f64) -> Option<bool> {
    let positive_delta = angle_deg - 180.0;
    if (-crate::COMPLETE_FOLD_ENDPOINT_EPS_DEG..=crate::COMPLETE_FOLD_ENDPOINT_EPS_DEG)
        .contains(&positive_delta)
    {
        return Some(true);
    }
    let negative_delta = angle_deg + 180.0;
    if (-crate::COMPLETE_FOLD_ENDPOINT_EPS_DEG..=crate::COMPLETE_FOLD_ENDPOINT_EPS_DEG)
        .contains(&negative_delta)
    {
        return Some(false);
    }
    None
}

fn validate_surface_group(
    cp: &CreasePattern,
    faces: &[Face],
    pose: &CanonicalNonflatPose,
    surface_faces: &[FaceId],
) -> Result<HashSet<FaceId>, SpatialCreaseOnlyError> {
    let selected = surface_faces.iter().copied().collect::<HashSet<_>>();
    if selected.is_empty() || selected.len() != surface_faces.len() {
        return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
    }
    if selected
        .iter()
        .any(|face_id| !faces.iter().any(|face| face.id == *face_id))
    {
        return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
    }
    for face_id in &selected {
        if unique_frame_face(&pose.frame, *face_id).is_err()
            || unique_transform(pose, *face_id).is_err()
        {
            return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
        }
    }
    validate_pose_face_isometries(cp, faces, pose, &selected)?;
    if selected.len() == 1 {
        return Ok(selected);
    }

    let angle_by_edge = signed_angle_map(&pose.signed_hinge_angles)?;
    let mut adjacency = HashMap::<FaceId, Vec<FaceId>>::new();
    for (index, left) in faces.iter().enumerate() {
        if !selected.contains(&left.id) {
            continue;
        }
        for right in faces.iter().skip(index + 1) {
            if !selected.contains(&right.id) {
                continue;
            }
            let connected_at_zero = left.edges.iter().any(|edge| {
                right.edges.contains(edge)
                    && angle_by_edge
                        .get(edge)
                        .copied()
                        .is_some_and(|angle| angle == 0.0)
            });
            if connected_at_zero {
                adjacency.entry(left.id).or_default().push(right.id);
                adjacency.entry(right.id).or_default().push(left.id);
            }
        }
    }
    let start = surface_faces[0];
    let mut reached = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(face) = queue.pop_front() {
        for neighbor in adjacency.get(&face).into_iter().flatten() {
            if reached.insert(*neighbor) {
                queue.push_back(*neighbor);
            }
        }
    }
    if reached != selected {
        return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
    }
    Ok(selected)
}

fn signed_angle_map(
    signed_angles: &[(EdgeId, f64)],
) -> Result<HashMap<EdgeId, f64>, SpatialCreaseOnlyError> {
    let mut result = HashMap::new();
    for &(edge, angle) in signed_angles {
        if !angle.is_finite() {
            return Err(SpatialCreaseOnlyError::InvalidTopRelation);
        }
        if let Some(previous) = result.insert(edge, angle)
            && previous.to_bits() != angle.to_bits()
        {
            return Err(SpatialCreaseOnlyError::InvalidTopRelation);
        }
    }
    Ok(result)
}

fn validate_pose_face_isometries(
    cp: &CreasePattern,
    faces: &[Face],
    pose: &CanonicalNonflatPose,
    selected: &HashSet<FaceId>,
) -> Result<(), SpatialCreaseOnlyError> {
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<HashMap<_, _>>();
    for face in faces.iter().filter(|face| selected.contains(&face.id)) {
        let frame_face = unique_frame_face(&pose.frame, face.id)?;
        let transform = unique_transform(pose, face.id)?;
        if face.vertices.len() != frame_face.polygon.len() || !transform_is_finite_rigid(transform)
        {
            return Err(SpatialCreaseOnlyError::MaterialLineMismatchAcrossSurfaceFaces);
        }
        for (vertex_id, actual) in face.vertices.iter().zip(&frame_face.polygon) {
            let material = positions
                .get(vertex_id)
                .copied()
                .ok_or(SpatialCreaseOnlyError::MaterialLineMismatchAcrossSurfaceFaces)?;
            let expected = apply_transform(transform, material);
            if !finite3(DVec3::from(*actual)) || (expected - DVec3::from(*actual)).length() > EPS {
                return Err(SpatialCreaseOnlyError::MaterialLineMismatchAcrossSurfaceFaces);
            }
        }
    }
    Ok(())
}

fn validate_keep_side_point(
    cp: &CreasePattern,
    faces: &[Face],
    selected: &HashSet<FaceId>,
    point: [f64; 2],
) -> Result<(), SpatialCreaseOnlyError> {
    let point = DVec2::from(point);
    let any_paper = faces
        .iter()
        .any(|face| point_location(cp, face, point) != PointLocation::Outside);
    if !any_paper {
        return Err(SpatialCreaseOnlyError::MaterialKeepSidePointOutsidePaper);
    }

    let selected_faces = faces
        .iter()
        .filter(|face| selected.contains(&face.id))
        .collect::<Vec<_>>();
    if !selected_faces
        .iter()
        .any(|face| point_location(cp, face, point) != PointLocation::Outside)
    {
        return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
    }
    let edge_owners = selected_edge_owner_counts(&selected_faces);
    for face in &selected_faces {
        for (index, edge_id) in face.edges.iter().enumerate() {
            if edge_owners.get(edge_id).copied() != Some(1) {
                continue;
            }
            let first = cp_vertex(cp, face.vertices[index])?;
            let second = cp_vertex(cp, face.vertices[(index + 1) % face.vertices.len()])?;
            if point_segment_distance(point, first, second) <= EPS {
                return Err(SpatialCreaseOnlyError::MaterialKeepSidePointOnBoundary);
            }
        }
    }
    Ok(())
}

fn selected_edge_owner_counts(faces: &[&Face]) -> HashMap<EdgeId, usize> {
    let mut owners = HashMap::new();
    for face in faces {
        for edge in &face.edges {
            *owners.entry(*edge).or_default() += 1;
        }
    }
    owners
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PointLocation {
    Outside,
    Boundary,
    Inside,
}

fn point_location(cp: &CreasePattern, face: &Face, point: DVec2) -> PointLocation {
    for index in 0..face.vertices.len() {
        let Ok(first) = cp_vertex(cp, face.vertices[index]) else {
            return PointLocation::Outside;
        };
        let Ok(second) = cp_vertex(cp, face.vertices[(index + 1) % face.vertices.len()]) else {
            return PointLocation::Outside;
        };
        if point_segment_distance(point, first, second) <= EPS {
            return PointLocation::Boundary;
        }
    }
    if point_in_face(cp, face, point.to_array()) {
        PointLocation::Inside
    } else {
        PointLocation::Outside
    }
}

fn validate_world_line(
    pose: &CanonicalNonflatPose,
    _faces: &[Face],
    segments: &[MaterialFaceSegment],
) -> Result<(), SpatialCreaseOnlyError> {
    let first = segments
        .first()
        .ok_or(SpatialCreaseOnlyError::PartialInsertion)?;
    let first_transform = unique_transform(pose, first.parent_face)?;
    let origin = apply_transform(first_transform, first.segment[0]);
    let first_end = apply_transform(first_transform, first.segment[1]);
    let direction = normalized3(first_end - origin)
        .ok_or(SpatialCreaseOnlyError::MaterialLineMismatchAcrossSurfaceFaces)?;
    for segment in segments {
        let transform = unique_transform(pose, segment.parent_face)?;
        let start = apply_transform(transform, segment.segment[0]);
        let end = apply_transform(transform, segment.segment[1]);
        let candidate_direction = normalized3(end - start)
            .ok_or(SpatialCreaseOnlyError::MaterialLineMismatchAcrossSurfaceFaces)?;
        if direction.cross(candidate_direction).length() > EPS
            || (start - origin).cross(direction).length() > EPS
            || (end - origin).cross(direction).length() > EPS
        {
            return Err(SpatialCreaseOnlyError::MaterialLineMismatchAcrossSurfaceFaces);
        }
    }
    Ok(())
}

fn unique_surface_mirroring(
    pose: &CanonicalNonflatPose,
    surface_faces: &[FaceId],
) -> Result<bool, SpatialCreaseOnlyError> {
    let mut values = surface_faces
        .iter()
        .map(|face| unique_frame_face(&pose.frame, *face).map(|frame_face| frame_face.mirrored));
    let first = values
        .next()
        .ok_or(SpatialCreaseOnlyError::AmbiguousTopSurface)??;
    for value in values {
        if value? != first {
            return Err(SpatialCreaseOnlyError::MaterialLineMismatchAcrossSurfaceFaces);
        }
    }
    Ok(first)
}

fn unique_frame_face(frame: &Frame3D, face: FaceId) -> Result<&Face3D, SpatialCreaseOnlyError> {
    let mut matches = frame
        .faces
        .iter()
        .filter(|candidate| candidate.face == face);
    let result = matches
        .next()
        .ok_or(SpatialCreaseOnlyError::AmbiguousTopSurface)?;
    if matches.next().is_some() {
        return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
    }
    Ok(result)
}

fn unique_transform(
    pose: &CanonicalNonflatPose,
    face: FaceId,
) -> Result<&FaceRigidTransform3, SpatialCreaseOnlyError> {
    let mut matches = pose
        .face_transforms
        .iter()
        .filter(|candidate| candidate.face == face);
    let result = matches
        .next()
        .ok_or(SpatialCreaseOnlyError::AmbiguousTopSurface)?;
    if matches.next().is_some() {
        return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
    }
    Ok(result)
}

fn transform_is_finite_rigid(transform: &FaceRigidTransform3) -> bool {
    let origin = DVec3::from(transform.world_origin);
    let x_axis = DVec3::from(transform.world_x_axis);
    let y_axis = DVec3::from(transform.world_y_axis);
    finite2(DVec2::from(transform.material_origin))
        && finite3(origin)
        && finite3(x_axis)
        && finite3(y_axis)
        && (x_axis.length() - 1.0).abs() <= EPS
        && (y_axis.length() - 1.0).abs() <= EPS
        && x_axis.dot(y_axis).abs() <= EPS
}

fn apply_transform(transform: &FaceRigidTransform3, material: [f64; 2]) -> DVec3 {
    let delta = DVec2::from(material) - DVec2::from(transform.material_origin);
    DVec3::from(transform.world_origin)
        + DVec3::from(transform.world_x_axis) * delta.x
        + DVec3::from(transform.world_y_axis) * delta.y
}

fn sort_material_segments(segments: &mut [MaterialFaceSegment], line: [[f64; 2]; 2]) {
    let start = DVec2::from(line[0]);
    let direction = (DVec2::from(line[1]) - start).normalize();
    segments.sort_by(|left, right| {
        segment_parameter_range(left.segment, start, direction)
            .0
            .total_cmp(&segment_parameter_range(right.segment, start, direction).0)
            .then_with(|| {
                segment_parameter_range(left.segment, start, direction)
                    .1
                    .total_cmp(&segment_parameter_range(right.segment, start, direction).1)
            })
    });
}

fn segment_parameter_range(segment: [[f64; 2]; 2], origin: DVec2, direction: DVec2) -> (f64, f64) {
    let first = (DVec2::from(segment[0]) - origin).dot(direction);
    let second = (DVec2::from(segment[1]) - origin).dot(direction);
    (first.min(second), first.max(second))
}

fn promote_aux_segments(
    cp: &mut CreasePattern,
    segments: &[MaterialFaceSegment],
    crease_kind: EdgeKind,
) {
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    for edge in &mut cp.edges {
        if edge.kind != EdgeKind::Aux {
            continue;
        }
        let (Some(&first), Some(&second)) = (positions.get(&edge.v0), positions.get(&edge.v1))
        else {
            continue;
        };
        if segments.iter().any(|segment| {
            point_on_material_segment(first, segment.segment)
                && point_on_material_segment(second, segment.segment)
        }) {
            edge.kind = crease_kind;
        }
    }
}

fn driver_is_fully_covered(cp: &CreasePattern, driver: &DriverLine) -> bool {
    let start = DVec2::from(driver.a);
    let end = DVec2::from(driver.b);
    let length = (end - start).length();
    if length <= EPS {
        return false;
    }
    let direction = (end - start) / length;
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let resolved = resolve_driver_edges(cp, driver)
        .into_iter()
        .collect::<HashSet<_>>();
    let mut intervals = cp
        .edges
        .iter()
        .filter(|edge| resolved.contains(&edge.id))
        .filter_map(|edge| {
            let first = positions.get(&edge.v0)?;
            let second = positions.get(&edge.v1)?;
            let first_t = (*first - start).dot(direction).clamp(0.0, length);
            let second_t = (*second - start).dot(direction).clamp(0.0, length);
            Some((first_t.min(second_t), first_t.max(second_t)))
        })
        .collect::<Vec<_>>();
    intervals.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut covered_until = 0.0;
    for (interval_start, interval_end) in intervals {
        if interval_start > covered_until + EPS {
            return false;
        }
        covered_until = covered_until.max(interval_end);
    }
    covered_until >= length - EPS
}

fn validate_new_vertices(
    original: &CreasePattern,
    work: &CreasePattern,
    face_map: &HashMap<FaceId, &Face>,
    selected: &HashSet<FaceId>,
    before_vertex_ids: &BTreeSet<VertexId>,
    parents: BTreeMap<VertexId, FaceId>,
) -> Result<Vec<NewMaterialVertex>, SpatialCreaseOnlyError> {
    let actual_new = work
        .vertices
        .iter()
        .filter(|vertex| !before_vertex_ids.contains(&vertex.id))
        .map(|vertex| vertex.id)
        .collect::<BTreeSet<_>>();
    // A valid crease may connect two pre-existing material vertices and add no
    // vertices at all. When vertices are added, the sets must still match
    // exactly so a partial split cannot be reported as success.
    if actual_new != parents.keys().copied().collect() {
        return Err(SpatialCreaseOnlyError::PartialInsertion);
    }
    let mut result = Vec::with_capacity(actual_new.len());
    for vertex_id in actual_new {
        let parent_face = parents[&vertex_id];
        if !selected.contains(&parent_face) {
            return Err(SpatialCreaseOnlyError::PartialInsertion);
        }
        let parent = face_map
            .get(&parent_face)
            .copied()
            .ok_or(SpatialCreaseOnlyError::PartialInsertion)?;
        let position = work
            .vertices
            .iter()
            .find(|vertex| vertex.id == vertex_id)
            .ok_or(SpatialCreaseOnlyError::PartialInsertion)?
            .pos;
        if !point_in_face(original, parent, position) {
            return Err(SpatialCreaseOnlyError::PartialInsertion);
        }
        result.push(NewMaterialVertex {
            vertex: vertex_id,
            parent_face,
        });
    }
    Ok(result)
}

fn unchanged_and_new_world_vertices(
    pose: &CanonicalNonflatPose,
    cp: &CreasePattern,
    new_vertices: &[NewMaterialVertex],
) -> Result<Vec<MaterialVertex3D>, SpatialCreaseOnlyError> {
    let mut result = pose.material_vertices.clone();
    let old_ids = result
        .iter()
        .map(|vertex| vertex.vertex)
        .collect::<HashSet<_>>();
    if old_ids.len() != result.len() {
        return Err(SpatialCreaseOnlyError::InvalidTopRelation);
    }
    for new_vertex in new_vertices {
        let material = cp
            .vertices
            .iter()
            .find(|vertex| vertex.id == new_vertex.vertex)
            .ok_or(SpatialCreaseOnlyError::PartialInsertion)?
            .pos;
        let transform = unique_transform(pose, new_vertex.parent_face)?;
        result.push(MaterialVertex3D {
            vertex: new_vertex.vertex,
            position: apply_transform(transform, material).to_array(),
        });
    }
    Ok(result)
}

fn unchanged_frame_for_split_faces(
    original_cp: &CreasePattern,
    original_faces: &[Face],
    pose: &CanonicalNonflatPose,
    work: &CreasePattern,
    new_faces: &[Face],
    material_vertices: &[MaterialVertex3D],
) -> Result<Frame3D, SpatialCreaseOnlyError> {
    let world = material_vertices
        .iter()
        .map(|vertex| (vertex.vertex, vertex.position))
        .collect::<HashMap<_, _>>();
    let mut frame_faces = Vec::with_capacity(new_faces.len());
    for face in new_faces {
        let point = representative_point(work, face);
        let mut parents = original_faces
            .iter()
            .filter(|candidate| point_in_face(original_cp, candidate, point));
        let parent = parents
            .next()
            .ok_or(SpatialCreaseOnlyError::PartialInsertion)?;
        if parents.next().is_some() {
            return Err(SpatialCreaseOnlyError::AmbiguousTopSurface);
        }
        let source = unique_frame_face(&pose.frame, parent.id)?;
        let polygon = face
            .vertices
            .iter()
            .map(|vertex| {
                world
                    .get(vertex)
                    .copied()
                    .ok_or(SpatialCreaseOnlyError::PartialInsertion)
            })
            .collect::<Result<Vec<_>, _>>()?;
        frame_faces.push(Face3D {
            face: face.id,
            polygon,
            layer: source.layer,
            surface_rank: source.surface_rank,
            mirrored: source.mirrored,
        });
    }
    Ok(Frame3D {
        faces: frame_faces,
        warnings: pose.frame.warnings.clone(),
    })
}

fn edge_on_any_segment(
    cp: &CreasePattern,
    first_vertex: VertexId,
    second_vertex: VertexId,
    segments: &[MaterialFaceSegment],
) -> bool {
    let (Ok(first), Ok(second)) = (cp_vertex(cp, first_vertex), cp_vertex(cp, second_vertex))
    else {
        return false;
    };
    segments.iter().any(|segment| {
        point_on_material_segment(first, segment.segment)
            && point_on_material_segment(second, segment.segment)
    })
}

fn point_on_material_segment(point: DVec2, segment: [[f64; 2]; 2]) -> bool {
    let start = DVec2::from(segment[0]);
    let end = DVec2::from(segment[1]);
    point_segment_distance(point, start, end) <= EPS
        && (point - start).dot(point - end) <= EPS * EPS
}

fn cp_vertex(cp: &CreasePattern, vertex: VertexId) -> Result<DVec2, SpatialCreaseOnlyError> {
    cp.vertices
        .iter()
        .find(|candidate| candidate.id == vertex)
        .map(|candidate| DVec2::from(candidate.pos))
        .filter(|position| finite2(*position))
        .ok_or(SpatialCreaseOnlyError::InvalidTopRelation)
}

fn point_segment_distance(point: DVec2, start: DVec2, end: DVec2) -> f64 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= EPS * EPS {
        return (point - start).length();
    }
    let parameter = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    (point - (start + segment * parameter)).length()
}

fn normalized3(value: DVec3) -> Option<DVec3> {
    if !finite3(value) {
        return None;
    }
    let length = value.length();
    (length.is_finite() && length > EPS).then_some(value / length)
}

fn finite2(value: DVec2) -> bool {
    value.x.is_finite() && value.y.is_finite()
}

fn finite3(value: DVec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}
