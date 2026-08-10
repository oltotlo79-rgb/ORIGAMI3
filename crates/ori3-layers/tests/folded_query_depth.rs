use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_geometry::Isometry2;
use ori3_layers::folded_query::{FoldedQuery, FoldedQueryError};
use ori3_layers::{FlatState, representative_point};
use ori3_model::{CreasePattern, Document, EdgeId, EdgeKind, FaceId, Paper, Vertex, VertexId};

struct OverlapFixture {
    cp: CreasePattern,
    faces: Vec<Face>,
    state: FlatState,
    bottom_face: FaceId,
    top_face: FaceId,
    bottom_edge: EdgeId,
    top_edge: EdgeId,
}

fn vertex_position(cp: &CreasePattern, vertex_id: VertexId) -> [f64; 2] {
    cp.vertices
        .iter()
        .find(|vertex| vertex.id == vertex_id)
        .expect("edge vertex exists")
        .pos
}

fn vertical_edge_at(cp: &CreasePattern, x: f64, kind: EdgeKind) -> EdgeId {
    cp.edges
        .iter()
        .find(|edge| {
            let a = vertex_position(cp, edge.v0);
            let b = vertex_position(cp, edge.v1);
            edge.kind == kind && (a[0] - x).abs() < 1e-12 && (b[0] - x).abs() < 1e-12
        })
        .expect("requested vertical edge exists")
        .id
}

fn overlapping_halves() -> OverlapFixture {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Valley);
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 2);

    let left_face = faces
        .iter()
        .find(|face| representative_point(&document.cp, face)[0] < 0.5)
        .expect("left half")
        .id;
    let right_face = faces
        .iter()
        .find(|face| representative_point(&document.cp, face)[0] > 0.5)
        .expect("right half")
        .id;
    let left_outer_edge = vertical_edge_at(&document.cp, 0.0, EdgeKind::Border);
    let center_edge = vertical_edge_at(&document.cp, 0.5, EdgeKind::Valley);

    // Translate the right half exactly onto the left half. At folded x=0, the left outer edge
    // and the right face's instance of the center edge coincide. Put the smaller edge ID on the
    // bottom so the unqualified deterministic tie-break demonstrably chooses the wrong depth.
    let placements = HashMap::from([
        (left_face, Isometry2::identity()),
        (
            right_face,
            Isometry2 {
                rotation: 0.0,
                translation: DVec2::new(-0.5, 0.0),
                mirrored: false,
            },
        ),
    ]);
    let (bottom_face, top_face, bottom_edge, top_edge) = if left_outer_edge < center_edge {
        (left_face, right_face, left_outer_edge, center_edge)
    } else {
        (right_face, left_face, center_edge, left_outer_edge)
    };
    let state = FlatState {
        placements,
        order: vec![bottom_face, top_face],
    };

    OverlapFixture {
        cp: document.cp,
        faces,
        state,
        bottom_face,
        top_face,
        bottom_edge,
        top_edge,
    }
}

#[test]
fn nearest_edge_on_sheet_respects_depth_at_a_boundary_probe() {
    let fixture = overlapping_halves();
    let query = FoldedQuery::new(&fixture.cp, &fixture.faces, &fixture.state).unwrap();
    let edge_point = [0.0, 0.5];
    let layer_seed = [0.25, 0.5];

    let unqualified = query.nearest_edge(edge_point).unwrap();
    assert_eq!(unqualified.edge_id, fixture.bottom_edge);

    let front = query
        .nearest_edge_on_sheet(edge_point, layer_seed, 1)
        .unwrap();
    assert_eq!(front.edge_id, fixture.top_edge);
    assert!(front.distance < 1e-12);

    let back = query
        .nearest_edge_on_sheet(edge_point, layer_seed, 2)
        .unwrap();
    assert_eq!(back.edge_id, fixture.bottom_edge);
    assert!(back.distance < 1e-12);

    assert_eq!(
        query
            .nearest_edge_on_face(edge_point, fixture.top_face)
            .unwrap(),
        front
    );
    assert_eq!(
        query
            .nearest_edge_on_face(edge_point, fixture.bottom_face)
            .unwrap(),
        back
    );
}

#[test]
fn nearest_edge_on_sheet_reports_strict_selection_errors() {
    let fixture = overlapping_halves();
    let query = FoldedQuery::new(&fixture.cp, &fixture.faces, &fixture.state).unwrap();
    let edge_point = [0.0, 0.5];
    let layer_seed = [0.25, 0.5];

    assert_eq!(
        query
            .nearest_edge_on_sheet(edge_point, layer_seed, 3)
            .unwrap_err(),
        FoldedQueryError::InsufficientLayers {
            point: layer_seed,
            requested: 3,
            available: 2,
        }
    );
    assert_eq!(
        query
            .nearest_edge_on_sheet(edge_point, [0.5, 0.5], 1)
            .unwrap_err(),
        FoldedQueryError::PointOnBoundary { point: [0.5, 0.5] }
    );
    assert_eq!(
        query
            .nearest_edge_on_sheet(edge_point, layer_seed, 0)
            .unwrap_err(),
        FoldedQueryError::InvalidSheetNumber { sheet_number: 0 }
    );

    let invalid_seed = query
        .nearest_edge_on_sheet(edge_point, [f64::NAN, 0.5], 1)
        .unwrap_err();
    assert!(matches!(
        invalid_seed,
        FoldedQueryError::InvalidPoint { point } if point[0].is_nan()
    ));

    let invalid_edge = query
        .nearest_edge_on_sheet([f64::INFINITY, 0.5], layer_seed, 1)
        .unwrap_err();
    assert!(matches!(
        invalid_edge,
        FoldedQueryError::InvalidPoint { point } if point[0].is_infinite()
    ));
}

#[test]
fn nearest_edge_on_sheet_reports_when_the_selected_face_has_no_edges() {
    let cp = CreasePattern {
        vertices: vec![
            Vertex {
                id: 0,
                pos: [0.0, 0.0],
            },
            Vertex {
                id: 1,
                pos: [1.0, 0.0],
            },
            Vertex {
                id: 2,
                pos: [0.0, 1.0],
            },
        ],
        edges: Vec::new(),
        next_vertex_id: 3,
        next_edge_id: 0,
    };
    let faces = vec![Face {
        id: 0,
        vertices: vec![0, 1, 2],
        // FoldedQuery validates the derived face shape independently of CP-edge availability.
        edges: vec![10, 11, 12],
    }];
    let state = FlatState {
        placements: HashMap::from([(0, Isometry2::identity())]),
        order: vec![0],
    };
    let query = FoldedQuery::new(&cp, &faces, &state).unwrap();

    assert_eq!(
        query
            .nearest_edge_on_sheet([0.25, 0.0], [0.2, 0.2], 1)
            .unwrap_err(),
        FoldedQueryError::NoEdges
    );
}
