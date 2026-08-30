//! Executable contract for adding a crease to an incomplete top surface.
//!
//! The fixture is a hand-authored rigid pose. It never calls a solver and all
//! invariants compare the result to its own input rather than to golden solve
//! coordinates or a face-ID ordering.

use std::collections::{BTreeMap, BTreeSet};

use ori3_cp::Face;
use ori3_layers::{
    CanonicalNonflatPose, FaceRigidTransform3, MaterialVertex3D, SpatialCreaseOnlyError,
    SpatialCreaseOnlyInput, SurfaceRelationFromTop, TopSurfaceObservation, TopSurfaceProvider,
    crease_only_top_from_material_line, point_in_face,
};
use ori3_model::{CreasePattern, Edge, EdgeKind, Face3D, FoldDirection, Frame3D, Vertex};

const TOP_LEFT: u32 = 10;
const TOP_RIGHT: u32 = 11;
const LOWER: u32 = 20;

struct Fixture {
    cp: CreasePattern,
    faces: Vec<Face>,
    pose: CanonicalNonflatPose,
    input: SpatialCreaseOnlyInput,
}

#[derive(Default)]
struct IncompleteTopProvider {
    calls: usize,
}

impl TopSurfaceProvider for IncompleteTopProvider {
    fn observe_from_top(
        &mut self,
        depth: usize,
    ) -> Result<TopSurfaceObservation, SpatialCreaseOnlyError> {
        if depth != 0 {
            panic!(
                "the top is already known to be incomplete; surface depth {depth} must not be read"
            );
        }
        self.calls += 1;
        Ok(TopSurfaceObservation {
            surface_faces: vec![TOP_LEFT, TOP_RIGHT],
            relation_to_next: SurfaceRelationFromTop::Incomplete {
                signed_angle_deg: 90.0,
            },
        })
    }
}

struct ErrorProvider(SpatialCreaseOnlyError);

impl TopSurfaceProvider for ErrorProvider {
    fn observe_from_top(
        &mut self,
        _depth: usize,
    ) -> Result<TopSurfaceObservation, SpatialCreaseOnlyError> {
        Err(self.0)
    }
}

struct FixedRelationProvider {
    relation: SurfaceRelationFromTop,
    calls: usize,
}

impl TopSurfaceProvider for FixedRelationProvider {
    fn observe_from_top(
        &mut self,
        depth: usize,
    ) -> Result<TopSurfaceObservation, SpatialCreaseOnlyError> {
        if depth != 0 {
            panic!("a non-incomplete top relation must not trigger a deeper read");
        }
        self.calls += 1;
        Ok(TopSurfaceObservation {
            surface_faces: vec![TOP_LEFT, TOP_RIGHT],
            relation_to_next: self.relation,
        })
    }
}

#[test]
fn every_preexisting_vertex_keeps_all_three_coordinate_bits() {
    let fixture = fixture();
    let before = vertex_position_bits(&fixture.pose.material_vertices);
    let result = run_success(&fixture);
    let after = vertex_position_bits(&result.material_vertices);

    for (vertex, expected_bits) in before {
        assert_eq!(
            after.get(&vertex),
            Some(&expected_bits),
            "pre-existing material vertex {vertex} moved"
        );
    }
}

#[test]
fn every_source_face_rigid_transform_is_bit_identical() {
    let fixture = fixture();
    let before = transform_bits(&fixture.pose.face_transforms);
    let result = run_success(&fixture);
    let after = transform_bits(&result.source_face_transforms);

    for (face, expected_bits) in before {
        assert_eq!(
            after.get(&face),
            Some(&expected_bits),
            "pre-existing face {face} changed its rigid transform"
        );
    }
}

#[test]
fn lower_paper_material_topology_is_identical() {
    let fixture = fixture();
    let lower_vertex_ids = BTreeSet::from([6, 7, 8, 9]);
    let lower_edge_ids = BTreeSet::from([7, 8, 9, 10]);
    let before_vertices = selected_vertices(&fixture.cp, &lower_vertex_ids);
    let before_edges = selected_edges(&fixture.cp, &lower_edge_ids);
    let result = run_success(&fixture);

    assert_eq!(
        selected_vertices(&result.cp, &lower_vertex_ids),
        before_vertices,
        "lower paper vertices changed"
    );
    assert_eq!(
        selected_edges(&result.cp, &lower_edge_ids),
        before_edges,
        "lower paper edges changed"
    );
}

#[test]
fn only_new_vertices_are_added_and_each_lies_on_its_parent_face() {
    let fixture = fixture();
    let before_ids = fixture
        .cp
        .vertices
        .iter()
        .map(|vertex| vertex.id)
        .collect::<BTreeSet<_>>();
    let result = run_success(&fixture);
    let after_ids = result
        .cp
        .vertices
        .iter()
        .map(|vertex| vertex.id)
        .collect::<BTreeSet<_>>();
    assert!(
        before_ids.is_subset(&after_ids),
        "a pre-existing material vertex was removed"
    );
    assert_eq!(
        result.cp.vertices.len(),
        after_ids.len(),
        "material vertex IDs must remain unique"
    );
    let actual_new_ids = result
        .cp
        .vertices
        .iter()
        .map(|vertex| vertex.id)
        .filter(|id| !before_ids.contains(id))
        .collect::<BTreeSet<_>>();
    let declared_new_ids = result
        .new_vertices
        .iter()
        .map(|vertex| vertex.vertex)
        .collect::<BTreeSet<_>>();

    assert_eq!(actual_new_ids, declared_new_ids);
    assert!(!actual_new_ids.is_empty(), "the crease must add vertices");
    for added in &result.new_vertices {
        let parent = fixture
            .faces
            .iter()
            .find(|face| face.id == added.parent_face)
            .expect("the result names an input parent face");
        assert!(
            [TOP_LEFT, TOP_RIGHT].contains(&parent.id),
            "a lower-paper face was split"
        );
        let position = result
            .cp
            .vertices
            .iter()
            .find(|vertex| vertex.id == added.vertex)
            .expect("the declared new vertex exists")
            .pos;
        assert!(
            point_in_face(&fixture.cp, parent, position),
            "new vertex {} is not on parent face {}",
            added.vertex,
            parent.id
        );
    }
}

#[test]
fn every_new_driver_is_explicit_positive_zero_degrees() {
    let result = run_success(&fixture());

    assert!(!result.step.drivers.is_empty(), "the crease needs a driver");
    assert!(
        result
            .step
            .drivers
            .iter()
            .all(|driver| driver.target_angle_deg.to_bits() == 0.0_f64.to_bits()),
        "crease-only drivers must all finish at explicit +0 degrees: {:?}",
        result.step.drivers
    );
}

#[test]
fn incomplete_top_is_observed_once_and_deeper_provider_is_never_called() {
    let fixture = fixture();
    let mut provider = IncompleteTopProvider::default();
    let result = crease_only_top_from_material_line(
        &fixture.cp,
        &fixture.faces,
        &fixture.pose,
        &fixture.input,
        &mut provider,
    );

    assert_eq!(
        (result.is_ok(), provider.calls),
        (true, 1),
        "the top relation is read exactly once; the panic sentinel rejects every deeper read"
    );
}

#[test]
fn keep_side_point_on_material_boundary_is_rejected_without_guessing() {
    let mut fixture = fixture();
    fixture.input.material_keep_side_point = [0.0, 0.5];
    assert_error(
        &fixture,
        SpatialCreaseOnlyError::MaterialKeepSidePointOnBoundary,
    );
}

#[test]
fn keep_side_point_outside_paper_is_rejected_without_guessing() {
    let mut fixture = fixture();
    fixture.input.material_keep_side_point = [1.25, 0.5];
    assert_error(
        &fixture,
        SpatialCreaseOnlyError::MaterialKeepSidePointOutsidePaper,
    );
}

#[test]
fn multiple_top_surface_candidates_are_rejected_without_face_id_tiebreaking() {
    let mut fixture = fixture();
    fixture
        .pose
        .frame
        .faces
        .push(fixture.pose.frame.faces[0].clone());
    assert_error(&fixture, SpatialCreaseOnlyError::AmbiguousTopSurface);
}

#[test]
fn provider_ambiguity_is_rejected_without_searching_for_a_convenient_surface() {
    let fixture = fixture();
    let mut provider = ErrorProvider(SpatialCreaseOnlyError::AmbiguousTopSurface);
    let actual = crease_only_top_from_material_line(
        &fixture.cp,
        &fixture.faces,
        &fixture.pose,
        &fixture.input,
        &mut provider,
    );
    assert_eq!(
        actual.err(),
        Some(SpatialCreaseOnlyError::AmbiguousTopSurface)
    );
}

#[test]
fn missing_zero_and_complete_top_relations_are_not_crease_only() {
    let fixture = fixture();
    let cases = [
        (
            "missing",
            SurfaceRelationFromTop::Missing,
            SpatialCreaseOnlyError::MissingTopRelation,
        ),
        (
            "zero",
            SurfaceRelationFromTop::Zero,
            SpatialCreaseOnlyError::ZeroTopRelation,
        ),
        (
            "positive complete fold",
            SurfaceRelationFromTop::CompletePositive180,
            SpatialCreaseOnlyError::CompleteTopRelation,
        ),
        (
            "negative complete fold",
            SurfaceRelationFromTop::CompleteNegative180,
            SpatialCreaseOnlyError::CompleteTopRelation,
        ),
    ];
    let actual = cases
        .into_iter()
        .map(|(name, relation, _)| {
            let mut provider = FixedRelationProvider { relation, calls: 0 };
            let error = crease_only_top_from_material_line(
                &fixture.cp,
                &fixture.faces,
                &fixture.pose,
                &fixture.input,
                &mut provider,
            )
            .err();
            (name, error, provider.calls)
        })
        .collect::<Vec<_>>();
    let expected = cases
        .into_iter()
        .map(|(name, _, error)| (name, Some(error), 1))
        .collect::<Vec<_>>();

    assert_eq!(
        actual, expected,
        "missing, zero, and either complete-fold sign are unavailable rather than crease-only"
    );
}

#[test]
fn material_line_that_maps_differently_across_surface_faces_is_rejected() {
    let mut fixture = fixture();
    let right = fixture
        .pose
        .frame
        .faces
        .iter_mut()
        .find(|face| face.face == TOP_RIGHT)
        .expect("right top face");
    for point in &mut right.polygon {
        point[1] += 0.01;
    }
    fixture
        .pose
        .face_transforms
        .iter_mut()
        .find(|transform| transform.face == TOP_RIGHT)
        .expect("right top transform")
        .world_origin[1] += 0.01;

    assert_error(
        &fixture,
        SpatialCreaseOnlyError::MaterialLineMismatchAcrossSurfaceFaces,
    );
}

fn run_success(fixture: &Fixture) -> ori3_layers::SpatialCreaseOnlyResult {
    let mut provider = IncompleteTopProvider::default();
    crease_only_top_from_material_line(
        &fixture.cp,
        &fixture.faces,
        &fixture.pose,
        &fixture.input,
        &mut provider,
    )
    .expect("an incomplete non-flat top surface receives a crease without moving")
}

fn assert_error(fixture: &Fixture, expected: SpatialCreaseOnlyError) {
    let mut provider = IncompleteTopProvider::default();
    let actual = crease_only_top_from_material_line(
        &fixture.cp,
        &fixture.faces,
        &fixture.pose,
        &fixture.input,
        &mut provider,
    );
    assert_eq!(actual.err(), Some(expected));
}

fn vertex_position_bits(vertices: &[MaterialVertex3D]) -> BTreeMap<u32, [u64; 3]> {
    vertices
        .iter()
        .map(|vertex| {
            (
                vertex.vertex,
                vertex.position.map(|coordinate| coordinate.to_bits()),
            )
        })
        .collect()
}

fn transform_bits(transforms: &[FaceRigidTransform3]) -> BTreeMap<u32, Vec<u64>> {
    transforms
        .iter()
        .map(|transform| {
            let bits = transform
                .material_origin
                .into_iter()
                .chain(transform.world_origin)
                .chain(transform.world_x_axis)
                .chain(transform.world_y_axis)
                .map(f64::to_bits)
                .collect();
            (transform.face, bits)
        })
        .collect()
}

fn selected_vertices(cp: &CreasePattern, ids: &BTreeSet<u32>) -> Vec<Vertex> {
    cp.vertices
        .iter()
        .filter(|vertex| ids.contains(&vertex.id))
        .cloned()
        .collect()
}

fn selected_edges(cp: &CreasePattern, ids: &BTreeSet<u32>) -> Vec<Edge> {
    cp.edges
        .iter()
        .filter(|edge| ids.contains(&edge.id))
        .cloned()
        .collect()
}

fn fixture() -> Fixture {
    let cp = crease_pattern();
    let faces = vec![
        Face {
            id: TOP_LEFT,
            vertices: vec![0, 1, 4, 5],
            edges: vec![0, 6, 4, 5],
        },
        Face {
            id: TOP_RIGHT,
            vertices: vec![1, 2, 3, 4],
            edges: vec![1, 2, 3, 6],
        },
        Face {
            id: LOWER,
            vertices: vec![6, 7, 8, 9],
            edges: vec![7, 8, 9, 10],
        },
    ];
    let top_world = |position: [f64; 2]| [position[0], 0.0, position[1]];
    let lower_world = |position: [f64; 2]| [position[0] - 2.0, 0.02, position[1]];
    let frame = Frame3D {
        faces: faces
            .iter()
            .map(|face| Face3D {
                face: face.id,
                polygon: face
                    .vertices
                    .iter()
                    .map(|vertex_id| {
                        let position = cp
                            .vertices
                            .iter()
                            .find(|vertex| vertex.id == *vertex_id)
                            .expect("fixture vertex")
                            .pos;
                        if face.id == LOWER {
                            lower_world(position)
                        } else {
                            top_world(position)
                        }
                    })
                    .collect(),
                layer: 0,
                surface_rank: if face.id == LOWER { 0 } else { 1 },
                mirrored: false,
            })
            .collect(),
        warnings: Vec::new(),
    };
    let material_vertices = cp
        .vertices
        .iter()
        .map(|vertex| MaterialVertex3D {
            vertex: vertex.id,
            position: if vertex.id >= 6 {
                lower_world(vertex.pos)
            } else {
                top_world(vertex.pos)
            },
        })
        .collect();
    let face_transforms = vec![
        transform(TOP_LEFT, [0.0, 0.0], [0.0, 0.0, 0.0]),
        transform(TOP_RIGHT, [0.5, 0.0], [0.5, 0.0, 0.0]),
        transform(LOWER, [2.0, 0.0], [0.0, 0.02, 0.0]),
    ];

    Fixture {
        cp,
        faces,
        pose: CanonicalNonflatPose {
            frame,
            material_vertices,
            face_transforms,
            signed_hinge_angles: vec![(6, 0.0), (11, 90.0)],
        },
        input: SpatialCreaseOnlyInput {
            material_line: [[0.0, 0.5], [1.0, 0.5]],
            material_keep_side_point: [0.25, 0.25],
            direction: FoldDirection::Up,
        },
    }
}

fn transform(face: u32, material_origin: [f64; 2], world_origin: [f64; 3]) -> FaceRigidTransform3 {
    FaceRigidTransform3 {
        face,
        material_origin,
        world_origin,
        world_x_axis: [1.0, 0.0, 0.0],
        world_y_axis: [0.0, 0.0, 1.0],
    }
}

fn crease_pattern() -> CreasePattern {
    let positions = [
        [0.0, 0.0],
        [0.5, 0.0],
        [1.0, 0.0],
        [1.0, 1.0],
        [0.5, 1.0],
        [0.0, 1.0],
        [2.0, 0.0],
        [3.0, 0.0],
        [3.0, 1.0],
        [2.0, 1.0],
    ];
    let vertices = positions
        .into_iter()
        .enumerate()
        .map(|(index, pos)| Vertex {
            id: u32::try_from(index).expect("fixture vertex ID"),
            pos,
        })
        .collect::<Vec<_>>();
    let edge_specs = [
        (0, 1, EdgeKind::Border),
        (1, 2, EdgeKind::Border),
        (2, 3, EdgeKind::Border),
        (3, 4, EdgeKind::Border),
        (4, 5, EdgeKind::Border),
        (5, 0, EdgeKind::Border),
        (1, 4, EdgeKind::Mountain),
        (6, 7, EdgeKind::Border),
        (7, 8, EdgeKind::Border),
        (8, 9, EdgeKind::Border),
        (9, 6, EdgeKind::Border),
    ];
    let edges = edge_specs
        .into_iter()
        .enumerate()
        .map(|(index, (v0, v1, kind))| Edge {
            id: u32::try_from(index).expect("fixture edge ID"),
            v0,
            v1,
            kind,
        })
        .collect::<Vec<_>>();
    CreasePattern {
        next_vertex_id: u32::try_from(vertices.len()).expect("next fixture vertex ID"),
        next_edge_id: u32::try_from(edges.len()).expect("next fixture edge ID"),
        vertices,
        edges,
    }
}
