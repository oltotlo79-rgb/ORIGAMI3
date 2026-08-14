//! Adjacent-face seam measurements.

use std::collections::HashMap;

use glam::DVec3;
use ori3_cp::Face;
use ori3_model::{CreasePattern, EPS, EdgeId, Face3D, FaceId, Frame3D, VertexId};

/// Returns the largest separation between copies of the same material vertex.
///
/// A crease shared by two faces has one copy of each endpoint in each face's 3D
/// polygon. Faces can also meet at only one material vertex without sharing an
/// edge; every polygon copy of that vertex must still coincide while the paper
/// is moving. The returned distance is divided by the crease pattern's
/// long-axis extent, so it is expressed relative to a paper whose long edge is
/// `1.0`.
///
/// Border edges and edges that are not shared by exactly two distinct faces do
/// not form edge seams and are ignored by the shared-edge pass. The vertex pass
/// still compares their incident face copies. Missing faces, unknown material
/// vertices, or malformed polygons are ignored; callers should validate the
/// crease pattern and frame separately.
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

    // Shared-edge checks do not cover two polygons that touch at only one material vertex.
    // Collect every polygon occurrence by VertexId as well. Keeping occurrences rather than
    // deduplicating by face also catches malformed/slit polygons whose repeated copies diverge.
    let mut vertex_copies: HashMap<VertexId, Vec<DVec3>> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, Vec::new()))
        .collect();
    for face in faces {
        let Some(frame_face) = frame_faces.get(&face.id) else {
            continue;
        };
        for (index, vertex_id) in face.vertices.iter().copied().enumerate() {
            let (Some(copies), Some(point)) = (
                vertex_copies.get_mut(&vertex_id),
                frame_face.polygon.get(index),
            ) else {
                continue;
            };
            copies.push(DVec3::from(*point));
        }
    }
    for copies in vertex_copies.values() {
        for (index, first) in copies.iter().enumerate() {
            for second in &copies[index + 1..] {
                worst = worst.max((*first - *second).length());
            }
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

    use ori3_cp::{Face, extract_faces};
    use ori3_model::{CreasePattern, Edge, EdgeKind, Face3D, Frame3D, Vertex};

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

    #[test]
    fn point_only_shared_vertex_gap_is_detected() {
        let vertex = |id, x, y| Vertex { id, pos: [x, y] };
        let cp = CreasePattern {
            // The two triangles share vertex 0 but no edge. The 2.0 long-axis extent also
            // verifies that point-only gaps use the same normalization as edge seams.
            vertices: vec![
                vertex(0, 0.0, 0.0),
                vertex(1, 1.0, 0.0),
                vertex(2, 0.0, 1.0),
                vertex(3, -1.0, 0.0),
                vertex(4, 0.0, -1.0),
            ],
            edges: Vec::new(),
            next_vertex_id: 5,
            next_edge_id: 0,
        };
        let faces = vec![
            Face {
                id: 10,
                vertices: vec![0, 1, 2],
                edges: Vec::new(),
            },
            Face {
                id: 11,
                vertices: vec![0, 3, 4],
                edges: Vec::new(),
            },
        ];
        let frame = Frame3D {
            faces: vec![
                Face3D {
                    face: 10,
                    polygon: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    layer: 0,
                    surface_rank: 0,
                    mirrored: false,
                },
                Face3D {
                    face: 11,
                    polygon: vec![[0.0, 0.0, 0.5], [-1.0, 0.0, 0.5], [0.0, -1.0, 0.5]],
                    layer: 1,
                    surface_rank: 1,
                    mirrored: false,
                },
            ],
            warnings: Vec::new(),
        };

        let gap = max_seam_gap(&cp, &faces, &frame);
        assert!((gap - 0.25).abs() < 1e-12, "gap={gap}");
    }
}
