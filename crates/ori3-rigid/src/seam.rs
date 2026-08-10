//! Adjacent-face seam measurements.

use std::collections::HashMap;

use glam::DVec3;
use ori3_cp::Face;
use ori3_model::{CreasePattern, EPS, EdgeId, Face3D, FaceId, Frame3D};

/// Returns the largest separation between the two copies of a shared edge endpoint.
///
/// A crease shared by two faces has one copy of each endpoint in each face's 3D
/// polygon. Those copies must coincide even while the paper is moving. The
/// returned distance is divided by the crease pattern's long-axis extent, so it
/// is expressed relative to a paper whose long edge is `1.0`.
///
/// Border edges and edges that are not shared by exactly two distinct faces do
/// not form seams and are ignored. Missing faces or malformed polygons are also
/// ignored; callers should validate the crease pattern and frame separately.
#[must_use]
pub fn max_seam_gap(cp: &CreasePattern, faces: &[Face], frame: &Frame3D) -> f64 {
    let frame_faces: HashMap<FaceId, &Face3D> =
        frame.faces.iter().map(|face| (face.face, face)).collect();

    let mut edge_faces: HashMap<EdgeId, Vec<&Face>> = HashMap::new();
    for face in faces {
        let mut edge_ids = face.edges.clone();
        edge_ids.sort_unstable();
        edge_ids.dedup();
        for edge_id in edge_ids {
            edge_faces.entry(edge_id).or_default().push(face);
        }
    }

    let mut worst = 0.0_f64;
    for edge in &cp.edges {
        let Some(adjacent) = edge_faces.get(&edge.id) else {
            continue;
        };
        if adjacent.len() != 2 || adjacent[0].id == adjacent[1].id {
            continue;
        }

        let Some(first) = frame_faces.get(&adjacent[0].id) else {
            continue;
        };
        let Some(second) = frame_faces.get(&adjacent[1].id) else {
            continue;
        };
        for vertex in [edge.v0, edge.v1] {
            let Some(first_index) = adjacent[0].vertices.iter().position(|&id| id == vertex) else {
                continue;
            };
            let Some(second_index) = adjacent[1].vertices.iter().position(|&id| id == vertex)
            else {
                continue;
            };
            let (Some(&first_point), Some(&second_point)) = (
                first.polygon.get(first_index),
                second.polygon.get(second_index),
            ) else {
                continue;
            };
            worst = worst.max((DVec3::from(first_point) - DVec3::from(second_point)).length());
        }
    }

    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for vertex in &cp.vertices {
        for axis in 0..2 {
            min[axis] = min[axis].min(vertex.pos[axis]);
            max[axis] = max[axis].max(vertex.pos[axis]);
        }
    }
    let long_edge = (max[0] - min[0]).max(max[1] - min[1]);
    if long_edge.is_finite() && long_edge > EPS {
        worst / long_edge
    } else {
        worst
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ori3_cp::extract_faces;
    use ori3_model::{CreasePattern, Edge, EdgeKind, Vertex};

    use super::max_seam_gap;
    use crate::tree::{propagate, to_frame3d};

    fn split_paper() -> CreasePattern {
        let vertex = |id, x, y| Vertex { id, pos: [x, y] };
        let edge = |id, v0, v1, kind| Edge { id, v0, v1, kind };
        CreasePattern {
            // Use a 2:1 paper to verify normalization by the long edge.
            vertices: vec![
                vertex(0, 0.0, 0.0),
                vertex(1, 1.0, 0.0),
                vertex(2, 2.0, 0.0),
                vertex(3, 2.0, 1.0),
                vertex(4, 1.0, 1.0),
                vertex(5, 0.0, 1.0),
            ],
            edges: vec![
                edge(0, 0, 1, EdgeKind::Border),
                edge(1, 1, 2, EdgeKind::Border),
                edge(2, 2, 3, EdgeKind::Border),
                edge(3, 3, 4, EdgeKind::Border),
                edge(4, 4, 5, EdgeKind::Border),
                edge(5, 5, 0, EdgeKind::Border),
                edge(6, 1, 4, EdgeKind::Mountain),
            ],
            next_vertex_id: 6,
            next_edge_id: 7,
        }
    }

    #[test]
    fn folded_hinge_has_no_seam_gap() {
        let cp = split_paper();
        let faces = extract_faces(&cp);
        let folded = propagate(&cp, &faces, &HashMap::from([(6, 73.0)]));
        let frame = to_frame3d(&cp, &faces, &folded);

        assert!(max_seam_gap(&cp, &faces, &frame) < 1e-12);
    }

    #[test]
    fn deliberately_broken_face_has_a_normalized_gap() {
        let cp = split_paper();
        let faces = extract_faces(&cp);
        let folded = propagate(&cp, &faces, &HashMap::from([(6, 73.0)]));
        let mut frame = to_frame3d(&cp, &faces, &folded);
        for point in &mut frame.faces[0].polygon {
            point[2] += 0.25;
        }

        // The physical gap is 0.25 and the paper's long edge is 2.0.
        let gap = max_seam_gap(&cp, &faces, &frame);
        assert!((gap - 0.125).abs() < 1e-12, "gap={gap}");
    }
}
