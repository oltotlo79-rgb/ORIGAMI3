use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::dist_point_segment;
use ori3_layers::folded_query::{ExtremeVertex, FoldedDirection, FoldedQuery, FoldedQueryError};
use ori3_layers::{
    FlatState, flat_state_at, layers_at_point as legacy_layers_at_point, point_in_face,
    representative_point,
};
use ori3_model::{
    CreasePattern, Document, DriverLine, Edge, EdgeKind, FoldStep, Paper, TechniqueKind, Vertex,
};

const FOLDED_SAMPLE: &str = include_str!("fixtures/folded-sample.ori3");
const TOLERANCE: f64 = 1.0e-9;

fn folded_sample_context() -> (Document, Vec<Face>, FlatState) {
    let document = parse_folded_fixture(FOLDED_SAMPLE);
    let faces = extract_faces(&document.cp);
    let (state, warnings) = flat_state_at(&document, &faces, document.sequence.len())
        .expect("folded-sample must replay to a flat state");
    assert!(warnings.is_empty(), "fixture replay warnings: {warnings:?}");
    assert_eq!(faces.len(), 46);
    assert_eq!(state.order.len(), faces.len());
    (document, faces, state)
}

#[test]
fn folded_sample_face_geometry_and_local_layers_are_queryable() {
    let (document, faces, state) = folded_sample_context();
    let query = FoldedQuery::new(&document.cp, &faces, &state).expect("build folded query");

    assert_eq!(query.face_geometries().len(), faces.len());
    let mut overall_min = [f64::INFINITY; 2];
    let mut overall_max = [f64::NEG_INFINITY; 2];
    for face in &faces {
        let geometry = query.face_geometry(face.id).expect("cached face geometry");
        assert_eq!(geometry.polygon.len(), face.vertices.len());
        assert!(
            geometry
                .polygon
                .iter()
                .all(|&point| geometry.bounds.contains(point))
        );

        let placement = state.placements.get(&face.id).expect("face placement");
        let expected_representative =
            placement.apply(DVec2::from(representative_point(&document.cp, face)));
        assert_point_close(
            geometry.representative_point,
            expected_representative.to_array(),
        );
        let cached_layers = query.layers_at_point(geometry.representative_point);
        assert!(
            cached_layers.contains(&face.id),
            "face {} must cover its representative point",
            face.id
        );
        assert_eq!(
            cached_layers,
            legacy_layers_at_point(&document.cp, &faces, &state, geometry.representative_point,),
            "cached and established point queries differ for face {}",
            face.id
        );

        for axis in 0..2 {
            overall_min[axis] = overall_min[axis].min(geometry.bounds.min[axis]);
            overall_max[axis] = overall_max[axis].max(geometry.bounds.max[axis]);
        }
    }
    assert_point_close(overall_min, [0.0, 0.0]);
    assert_point_close(overall_max, [1.0, 1.0]);

    let two_layer_point = [0.069_035_593_728_849_2, 0.092_724_864_350_673_44];
    assert_eq!(query.layers_at_point(two_layer_point), vec![6, 7]);
    assert_eq!(
        query
            .nth_from_top_at_point(two_layer_point, 1)
            .expect("front sheet"),
        7
    );
    assert_eq!(
        query
            .nth_from_top_at_point(two_layer_point, 2)
            .expect("second sheet from front"),
        6
    );
}

#[test]
fn folded_sample_extreme_face_vertex_is_really_extreme() {
    let (document, faces, state) = folded_sample_context();
    let query = FoldedQuery::new(&document.cp, &faces, &state).expect("build folded query");

    let right = query
        .extreme(FoldedDirection::Right)
        .expect("rightmost vertex");
    assert_is_extreme(&query, &faces, right, [1.0, 0.0]);
    assert!((right.point[0] - 1.0).abs() <= TOLERANCE);

    let top = query.extreme(FoldedDirection::Top).expect("topmost vertex");
    assert_is_extreme(&query, &faces, top, [0.0, 1.0]);
    assert!((top.point[1] - 1.0).abs() <= TOLERANCE);

    let bottom_left = query
        .extreme(FoldedDirection::BottomLeft)
        .expect("bottom-left vertex");
    assert_is_extreme(&query, &faces, bottom_left, [-1.0, -1.0]);
    assert!((bottom_left.point[0] + bottom_left.point[1]).abs() <= TOLERANCE);
}

#[test]
fn strict_front_selection_reports_outside_empty_and_insufficient_cases() {
    let (document, faces, state) = folded_sample_context();
    let query = FoldedQuery::new(&document.cp, &faces, &state).expect("build folded query");
    let point = [0.069_035_593_728_849_2, 0.092_724_864_350_673_44];

    assert!(matches!(
        query.nth_from_top_at_point(point, 3),
        Err(FoldedQueryError::InsufficientLayers {
            requested: 3,
            available: 2,
            ..
        })
    ));
    assert!(matches!(
        query.nth_from_top_at_point(point, 0),
        Err(FoldedQueryError::InvalidSheetNumber { sheet_number: 0 })
    ));

    let outside = [2.0, 2.0];
    assert!(query.layers_at_point(outside).is_empty());
    assert!(matches!(
        query.nth_from_top_at_point(outside, 1),
        Err(FoldedQueryError::PointOutsidePaper { point }) if point == outside
    ));

    let boundary = query.face_geometries()[0].polygon[0];
    assert!(matches!(
        query.nth_from_top_at_point(boundary, 1),
        Err(FoldedQueryError::PointOnBoundary { point }) if point == boundary
    ));
    assert!(matches!(
        ori3_layers::folded_query::nth_from_top_at_point(
            &document.cp,
            &faces,
            &state,
            boundary,
            1,
        ),
        Err(FoldedQueryError::PointOnBoundary { point }) if point == boundary
    ));

    let empty_state = FlatState {
        placements: Default::default(),
        order: Vec::new(),
    };
    assert!(matches!(
        FoldedQuery::new(&document.cp, &[], &empty_state),
        Err(FoldedQueryError::EmptyState)
    ));
}

#[test]
fn folded_sample_nearest_edge_returns_an_existing_edge_and_distance() {
    let (document, faces, state) = folded_sample_context();
    let query = FoldedQuery::new(&document.cp, &faces, &state).expect("build folded query");
    let point = DVec2::new(1.1, 0.9);
    let nearest = query.nearest_edge(point.to_array()).expect("nearest edge");

    assert!(
        document
            .cp
            .edges
            .iter()
            .any(|edge| edge.id == nearest.edge_id)
    );

    let positions: HashMap<_, _> = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect();
    let mut expected: Option<(f64, u32, u32)> = None;
    for edge in &document.cp.edges {
        let a = positions[&edge.v0];
        let b = positions[&edge.v1];
        let midpoint = (a + b) * 0.5;
        for face in faces
            .iter()
            .filter(|face| point_in_face(&document.cp, face, midpoint.to_array()))
        {
            let placement = state
                .placements
                .get(&face.id)
                .expect("edge owner placement");
            let distance = dist_point_segment(point, placement.apply(a), placement.apply(b));
            let candidate = (distance, edge.id, face.id);
            if expected.is_none_or(|current| {
                candidate
                    .0
                    .total_cmp(&current.0)
                    .then(candidate.1.cmp(&current.1))
                    .then(candidate.2.cmp(&current.2))
                    .is_lt()
            }) {
                expected = Some(candidate);
            }
        }
    }
    let (expected_distance, expected_edge_id, _) = expected.expect("mapped fixture edge");
    assert_eq!(nearest.edge_id, expected_edge_id);
    assert!((nearest.distance - expected_distance).abs() <= TOLERANCE);

    let selects_auxiliary_edge = document
        .cp
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Aux)
        .any(|edge| {
            let midpoint = (positions[&edge.v0] + positions[&edge.v1]) * 0.5;
            faces
                .iter()
                .filter(|face| point_in_face(&document.cp, face, midpoint.to_array()))
                .any(|face| {
                    let folded_midpoint = state
                        .placements
                        .get(&face.id)
                        .expect("auxiliary edge owner placement")
                        .apply(midpoint);
                    query
                        .nearest_edge(folded_midpoint.to_array())
                        .is_ok_and(|nearest| {
                            nearest.edge_id == edge.id && nearest.distance <= TOLERANCE
                        })
                })
        });
    assert!(
        selects_auxiliary_edge,
        "at least one auxiliary edge must be selectable"
    );
}

fn assert_is_extreme(
    query: &FoldedQuery<'_>,
    faces: &[Face],
    extreme: ExtremeVertex,
    direction: [f64; 2],
) {
    let face = faces
        .iter()
        .find(|face| face.id == extreme.face_id)
        .expect("extreme face exists");
    let vertex_index = face
        .vertices
        .iter()
        .position(|&vertex_id| vertex_id == extreme.vertex_id)
        .expect("extreme vertex belongs to extreme face");
    assert_point_close(
        query
            .face_geometry(extreme.face_id)
            .expect("extreme geometry")
            .polygon[vertex_index],
        extreme.point,
    );

    let direction = DVec2::from(direction);
    let expected = query
        .face_geometries()
        .iter()
        .flat_map(|geometry| geometry.polygon.iter())
        .map(|&point| DVec2::from(point).dot(direction))
        .fold(f64::NEG_INFINITY, f64::max);
    assert!((extreme.projection - expected).abs() <= TOLERANCE);
}

fn assert_point_close(actual: [f64; 2], expected: [f64; 2]) {
    assert!(
        (DVec2::from(actual) - DVec2::from(expected)).length() <= TOLERANCE,
        "actual={actual:?}, expected={expected:?}"
    );
}

// ori3-layers deliberately has no serde_json dependency.  This small loader
// reads the fields needed to replay the checked-in fixture without changing
// the shared Cargo.toml.
fn parse_folded_fixture(source: &str) -> Document {
    let paper_json = json_field(source, "paper");
    let paper = Paper {
        width_mm: json_f64(json_field(paper_json, "width_mm")),
        height_mm: json_f64(json_field(paper_json, "height_mm")),
    };
    let mut document = Document::new(paper);
    document.schema_version = json_u32(json_field(source, "schema_version"));

    let cp_json = json_field(source, "cp");
    let vertices = json_array_items(json_field(cp_json, "vertices"))
        .into_iter()
        .map(|vertex| Vertex {
            id: json_u32(json_field(vertex, "id")),
            pos: json_point(json_field(vertex, "pos")),
        })
        .collect();
    let edges = json_array_items(json_field(cp_json, "edges"))
        .into_iter()
        .map(|edge| Edge {
            id: json_u32(json_field(edge, "id")),
            v0: json_u32(json_field(edge, "v0")),
            v1: json_u32(json_field(edge, "v1")),
            kind: edge_kind(json_text(json_field(edge, "kind"))),
        })
        .collect();
    document.cp = CreasePattern {
        vertices,
        edges,
        next_vertex_id: json_u32(json_field(cp_json, "next_vertex_id")),
        next_edge_id: json_u32(json_field(cp_json, "next_edge_id")),
    };

    document.sequence = json_array_items(json_field(source, "sequence"))
        .into_iter()
        .map(|step| FoldStep {
            id: json_u32(json_field(step, "id")),
            kind: technique_kind(json_text(json_field(step, "kind"))),
            drivers: json_array_items(json_field(step, "drivers"))
                .into_iter()
                .map(|driver| DriverLine {
                    a: json_point(json_field(driver, "a")),
                    b: json_point(json_field(driver, "b")),
                    target_angle_deg: json_f64(json_field(driver, "target_angle_deg")),
                })
                .collect(),
            layer_order: Some(
                json_array_items(json_field(step, "layer_order"))
                    .into_iter()
                    .map(json_point)
                    .collect(),
            ),
            alignment: None,
            curved_inside_reverse: None,
            finish_soft: None,
            note: json_text(json_field(step, "note")).to_owned(),
        })
        .collect();

    assert_eq!(document.cp.vertices.len(), 140, "fixture vertex count");
    assert_eq!(document.cp.edges.len(), 295, "fixture edge count");
    assert_eq!(document.sequence.len(), 8, "fixture saved-step count");
    document
}

fn edge_kind(value: &str) -> EdgeKind {
    match value {
        "Border" => EdgeKind::Border,
        "Mountain" => EdgeKind::Mountain,
        "Valley" => EdgeKind::Valley,
        "Aux" => EdgeKind::Aux,
        _ => panic!("unknown edge kind: {value}"),
    }
}

fn technique_kind(value: &str) -> TechniqueKind {
    match value {
        "Simple" => TechniqueKind::Simple,
        "InsideReverse" => TechniqueKind::InsideReverse,
        "Pleat" => TechniqueKind::Pleat,
        "OutsideReverse" => TechniqueKind::OutsideReverse,
        "Petal" => TechniqueKind::Petal,
        "Squash" => TechniqueKind::Squash,
        "OpenSink" => TechniqueKind::OpenSink,
        "Swivel" => TechniqueKind::Swivel,
        "Twist" => TechniqueKind::Twist,
        "Pose" => TechniqueKind::Pose,
        _ => panic!("unknown technique kind: {value}"),
    }
}

fn json_field<'a>(source: &'a str, key: &str) -> &'a str {
    let marker = format!("\"{key}\"");
    let key_end = source
        .find(&marker)
        .map(|index| index + marker.len())
        .unwrap_or_else(|| panic!("missing JSON field {key}"));
    let after_key = source[key_end..].trim_start();
    let value = after_key
        .strip_prefix(':')
        .unwrap_or_else(|| panic!("missing colon after JSON field {key}"))
        .trim_start();
    json_value(value)
}

fn json_array_items(array: &str) -> Vec<&str> {
    let inner = array
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .expect("JSON array");
    let mut rest = inner.trim();
    let mut items = Vec::new();
    while !rest.is_empty() {
        let value = json_value(rest);
        items.push(value);
        rest = rest[value.len()..].trim_start();
        if let Some(after_comma) = rest.strip_prefix(',') {
            rest = after_comma.trim_start();
        } else {
            assert!(rest.is_empty(), "expected comma between JSON array items");
        }
    }
    items
}

fn json_value(source: &str) -> &str {
    let source = source.trim_start();
    let first = *source.as_bytes().first().expect("non-empty JSON value");
    match first {
        b'[' | b'{' => json_container(source, first),
        b'"' => json_quoted(source),
        _ => {
            let end = source.find([',', ']', '}']).unwrap_or(source.len());
            source[..end].trim_end()
        }
    }
}

fn json_container(source: &str, opening: u8) -> &str {
    let closing = if opening == b'[' { b']' } else { b'}' };
    let mut depth = 0_u32;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in source.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
        } else if byte == opening {
            depth += 1;
        } else if byte == closing {
            depth -= 1;
            if depth == 0 {
                return &source[..=index];
            }
        }
    }
    panic!("unterminated JSON container")
}

fn json_quoted(source: &str) -> &str {
    let mut escaped = false;
    for (index, byte) in source.bytes().enumerate().skip(1) {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return &source[..=index];
        }
    }
    panic!("unterminated JSON string")
}

fn json_text(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .expect("JSON string")
}

fn json_u32(value: &str) -> u32 {
    value.parse().expect("JSON u32")
}

fn json_f64(value: &str) -> f64 {
    value.parse().expect("JSON f64")
}

fn json_point(value: &str) -> [f64; 2] {
    let coordinates = json_array_items(value);
    assert_eq!(coordinates.len(), 2, "JSON 2D point");
    [json_f64(coordinates[0]), json_f64(coordinates[1])]
}
