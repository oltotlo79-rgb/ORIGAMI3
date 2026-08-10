---
[package]
edition = "2024"

[dependencies]
ori3-model = { path = "../../../ori3-model" }
ori3-cp = { path = "../../../ori3-cp" }
ori3-layers = { path = "../.." }
serde_json = "1.0.151"
---

use std::collections::HashMap;

use ori3_cp::extract_faces;
use ori3_layers::{flat_state_at, layers_at_point, representative_point};
use ori3_model::{Document, FaceId, VertexId};

fn folded_point(
    local: [f64; 2],
    placement: &ori3_geometry_placeholder::PlacementView,
) -> [f64; 2] {
    let x = local[0];
    let y = if placement.mirrored { -local[1] } else { local[1] };
    let (sin, cos) = placement.rotation.sin_cos();
    [
        cos * x - sin * y + placement.tx,
        sin * x + cos * y + placement.ty,
    ]
}

// Keep the calculation above independent of glam in this diagnostic script.
mod ori3_geometry_placeholder {
    pub struct PlacementView {
        pub rotation: f64,
        pub tx: f64,
        pub ty: f64,
        pub mirrored: bool,
    }
}

fn point_in_polygon(poly: &[[f64; 2]], point: [f64; 2]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    for index in 0..poly.len() {
        let a = poly[index];
        let b = poly[(index + 1) % poly.len()];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let crossing_x = a[0] + (point[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if point[0] < crossing_x {
                inside = !inside;
            }
        }
    }
    inside
}

fn main() {
    let json = include_str!("../fixtures/devil-024.ori3");
    let document: Document = serde_json::from_str(json).expect("devil-024.ori3 deserialize");
    let faces = extract_faces(&document.cp);
    let (state, warnings) = flat_state_at(&document, &faces, document.sequence.len())
        .expect("step 24 must replay to a flat state");
    let vertex_positions: HashMap<VertexId, [f64; 2]> = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect();

    println!(
        "SUMMARY vertices={} edges={} saved_steps={} faces={} global_order_entries={} warnings={}",
        document.cp.vertices.len(),
        document.cp.edges.len(),
        document.sequence.len(),
        faces.len(),
        state.order.len(),
        warnings.len()
    );
    for warning in &warnings {
        println!("WARNING {warning}");
    }
    println!("ORDER_BOTTOM_TO_TOP {:?}", state.order);
    println!("LAYERS_BEGIN");

    let mut folded_by_face: HashMap<FaceId, Vec<[f64; 2]>> = HashMap::new();
    let mut representative_by_face: HashMap<FaceId, [f64; 2]> = HashMap::new();
    let mut all_min = [f64::INFINITY; 2];
    let mut all_max = [f64::NEG_INFINITY; 2];
    for (rank, face_id) in state.order.iter().copied().enumerate() {
        let face = faces
            .iter()
            .find(|candidate| candidate.id == face_id)
            .expect("ordered face exists");
        let iso = state.placements.get(&face_id).expect("placement exists");
        let placement = ori3_geometry_placeholder::PlacementView {
            rotation: iso.rotation,
            tx: iso.translation.x,
            ty: iso.translation.y,
            mirrored: iso.mirrored,
        };
        let folded_vertices: Vec<[f64; 2]> = face
            .vertices
            .iter()
            .map(|id| folded_point(vertex_positions[id], &placement))
            .collect();
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in &folded_vertices {
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
                all_min[axis] = all_min[axis].min(point[axis]);
                all_max[axis] = all_max[axis].max(point[axis]);
            }
        }
        let rep = folded_point(representative_point(&document.cp, face), &placement);
        let local_stack = layers_at_point(&document.cp, &faces, &state, rep);
        println!(
            "rank_bottom={rank:02} rank_top={:02} face={face_id:03} rep=({:.9},{:.9}) bbox=({:.9},{:.9})-({:.9},{:.9}) vertices={} mirrored={} local_stack={:?}",
            state.order.len() - 1 - rank,
            rep[0],
            rep[1],
            min[0],
            min[1],
            max[0],
            max[1],
            face.vertices.len(),
            iso.mirrored,
            local_stack
        );
        folded_by_face.insert(face_id, folded_vertices);
        representative_by_face.insert(face_id, rep);
    }
    println!("LAYERS_END");
    println!(
        "FOLDED_BBOX ({:.9},{:.9})-({:.9},{:.9})",
        all_min[0], all_min[1], all_max[0], all_max[1]
    );

    println!("REPRESENTATIVE_LOCAL_STACKS_BEGIN");
    for face_id in state.order.iter().copied() {
        let point = representative_by_face[&face_id];
        let sampled: Vec<FaceId> = state
            .order
            .iter()
            .copied()
            .filter(|candidate| point_in_polygon(&folded_by_face[candidate], point))
            .collect();
        let public = layers_at_point(&document.cp, &faces, &state, point);
        assert_eq!(sampled, public, "cached polygon query must match public API");
        println!(
            "face={face_id:03} point=({:.9},{:.9}) bottom_to_top={public:?}",
            point[0], point[1]
        );
    }
    println!("REPRESENTATIVE_LOCAL_STACKS_END");

    // Probe open cell interiors on a dense grid. This avoids the double-counting
    // inherent at shared boundaries and reports a practical maximum local stack.
    let divisions = 300usize;
    let mut max_depth = 0usize;
    let mut max_point = [0.0, 0.0];
    let mut max_layers = Vec::new();
    for iy in 0..divisions {
        for ix in 0..divisions {
            let point = [
                all_min[0] + (all_max[0] - all_min[0]) * (ix as f64 + 0.371) / divisions as f64,
                all_min[1] + (all_max[1] - all_min[1]) * (iy as f64 + 0.619) / divisions as f64,
            ];
            let layers: Vec<FaceId> = state
                .order
                .iter()
                .copied()
                .filter(|face_id| point_in_polygon(&folded_by_face[face_id], point))
                .collect();
            if layers.len() > max_depth {
                max_depth = layers.len();
                max_point = point;
                max_layers = layers;
            }
        }
    }
    println!(
        "SAMPLED_MAX_LOCAL_DEPTH depth={} point=({:.9},{:.9}) bottom_to_top={:?}",
        max_depth, max_point[0], max_point[1], max_layers
    );
    println!(
        "PUBLIC_LAYERS_AT_MAX bottom_to_top={:?}",
        layers_at_point(&document.cp, &faces, &state, max_point)
    );
}
