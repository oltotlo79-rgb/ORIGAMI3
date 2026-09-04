//! Rabbit-ear (pull-together) folds made from three concurrent creases.

use std::collections::{HashMap, HashSet};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::reflect_across_line;
use ori3_model::{CreasePattern, EPS, FaceId, TechniqueKind};

use crate::flat_motion::{
    EvidenceWanted, FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform, run_motion,
};
use crate::flat_state::{FlatState, point_in_face, representative_point};
use crate::fold_through::{FoldDirection, FoldThroughResult};

const CONCURRENT_EPS: f64 = 1e-7;
const ANGLE_EPS: f64 = 1e-9;
const AREA_EPS: f64 = 1e-12;
const STATE_EPS: f64 = 1e-7;

type ResolvedCreases = (DVec2, [DVec2; 3], [[[f64; 2]; 2]; 3]);

/// Input for [`rabbit_ear`].
#[derive(Clone, Debug)]
pub struct RabbitEarInput {
    /// Three crease lines, ordered from the stationary side toward the free
    /// side of the flap. Their directions must all point away from the common
    /// vertex and turn consistently through less than 180 degrees.
    pub creases: [[[f64; 2]; 2]; 3],
    /// Current local face IDs belonging to the flap. Every one of the three
    /// moving sectors must contain at least one selected layer.
    pub target_layers: Vec<FaceId>,
    /// Put the collapsed fan toward the top or bottom of the layer stack.
    pub direction: FoldDirection,
}

/// Collapse three consecutive sectors around one boundary vertex in one step.
///
/// The sector next to the stationary paper is reflected once, the next sector
/// twice, and the free-side sector three times. Thus adjacent regions remain
/// joined along every crease while all three folds complete simultaneously.
/// Missing guide segments are inserted; existing auxiliary segments are
/// promoted to mountain/valley creases by `flat_motion`.
pub fn rabbit_ear(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &RabbitEarInput,
) -> Result<FoldThroughResult, String> {
    let (vertex, directions, creases) = resolve_creases(input.creases)?;
    let regions = rabbit_regions(vertex, directions, creases);
    let layers = resolve_region_layers(cp, faces, state, input, vertex, &regions)?;

    let parts = vec![
        MotionPart {
            layers: layers[0].clone(),
            region: regions[0].clone(),
            transform: MotionTransform::Reflect(vec![creases[0]]),
            turn: LayerTurn::Outside(input.direction),
            reverse_layers: None,
        },
        MotionPart {
            layers: layers[1].clone(),
            region: regions[1].clone(),
            transform: MotionTransform::Reflect(vec![creases[1], creases[0]]),
            turn: LayerTurn::Outside(input.direction),
            reverse_layers: None,
        },
        MotionPart {
            layers: layers[2].clone(),
            region: regions[2].clone(),
            transform: MotionTransform::Reflect(vec![creases[2], creases[1], creases[0]]),
            turn: LayerTurn::Outside(input.direction),
            reverse_layers: None,
        },
    ];

    let out = run_motion(
        cp,
        faces,
        state,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Pleat,
        },
        EvidenceWanted::No,
    )?;
    if !out.crossed_any {
        return Err("rabbit-ear creases do not cross the selected local layers".to_string());
    }
    if !out.result.warnings.is_empty() {
        return Err(format!(
            "rabbit-ear motion produced warnings: {:?}",
            out.result.warnings
        ));
    }
    for (index, crease) in creases.iter().enumerate() {
        if !has_fold_driver(&out.result, *crease) {
            return Err(format!(
                "rabbit-ear crease {} did not produce a folded region",
                index + 1
            ));
        }
    }

    let next_faces = extract_faces(&out.cp);
    validate_face_coverage(cp, faces, &out.cp, &next_faces)?;
    validate_state(&out.result.state, &next_faces)?;

    *cp = out.cp;
    Ok(out.result)
}

fn resolve_creases(input: [[[f64; 2]; 2]; 3]) -> Result<ResolvedCreases, String> {
    let points = input.map(|line| [DVec2::from(line[0]), DVec2::from(line[1])]);
    let mut directions = [DVec2::ZERO; 3];
    for (index, [a, b]) in points.iter().copied().enumerate() {
        if (b - a).length() <= EPS {
            return Err(format!("rabbit-ear crease {} is degenerate", index + 1));
        }
        directions[index] = (b - a).normalize();
    }

    let denominator = directions[0].perp_dot(directions[1]);
    if denominator.abs() <= ANGLE_EPS {
        return Err("rabbit-ear creases 1 and 2 do not define one vertex".to_string());
    }
    let vertex = points[0][0]
        + directions[0] * (points[1][0] - points[0][0]).perp_dot(directions[1]) / denominator;
    for (index, [a, _]) in points.iter().copied().enumerate() {
        let distance = directions[index].perp_dot(vertex - a).abs();
        if distance > CONCURRENT_EPS {
            return Err(format!(
                "rabbit-ear crease {} misses the common vertex by {distance:.3e}",
                index + 1
            ));
        }
    }

    let first_turn = signed_angle(directions[0], directions[1]);
    if first_turn.abs() <= ANGLE_EPS || (std::f64::consts::PI - first_turn.abs()).abs() <= ANGLE_EPS
    {
        return Err("rabbit-ear creases 1 and 2 are not distinct rays".to_string());
    }
    let handedness = first_turn.signum();
    let second_turn = handedness * signed_angle(directions[1], directions[2]);
    let outer_turn = handedness * signed_angle(directions[0], directions[2]);
    let total_turn = first_turn.abs() + second_turn;
    if second_turn <= ANGLE_EPS
        || total_turn >= std::f64::consts::PI - ANGLE_EPS
        || outer_turn <= ANGLE_EPS
        || (outer_turn - total_turn).abs() > CONCURRENT_EPS
    {
        return Err(
            "rabbit-ear creases must be ordered, consistently directed rays spanning less than 180 degrees"
                .to_string(),
        );
    }

    let creases = directions.map(|direction| {
        [
            [vertex.x, vertex.y],
            [vertex.x + direction.x, vertex.y + direction.y],
        ]
    });
    Ok((vertex, directions, creases))
}

fn signed_angle(from: DVec2, to: DVec2) -> f64 {
    from.perp_dot(to).atan2(from.dot(to))
}

fn rabbit_regions(
    vertex: DVec2,
    directions: [DVec2; 3],
    creases: [[[f64; 2]; 2]; 3],
) -> [Vec<HalfPlane>; 3] {
    let first = vertex + directions[0] + directions[1];
    let second = vertex + directions[1] + directions[2];
    let third = reflect_across_line(vertex + directions[1], vertex, vertex + directions[2]);
    [
        vec![
            HalfPlane {
                line: creases[0],
                inside_point: [first.x, first.y],
            },
            HalfPlane {
                line: creases[1],
                inside_point: [first.x, first.y],
            },
        ],
        vec![
            HalfPlane {
                line: creases[1],
                inside_point: [second.x, second.y],
            },
            HalfPlane {
                line: creases[2],
                inside_point: [second.x, second.y],
            },
        ],
        vec![HalfPlane {
            line: creases[2],
            inside_point: [third.x, third.y],
        }],
    ]
}

fn resolve_region_layers(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &RabbitEarInput,
    vertex: DVec2,
    regions: &[Vec<HalfPlane>; 3],
) -> Result<[Vec<FaceId>; 3], String> {
    if input.target_layers.is_empty() {
        return Err("rabbit-ear needs at least one local target layer".to_string());
    }
    let unique = input.target_layers.iter().copied().collect::<HashSet<_>>();
    if unique.len() != input.target_layers.len() {
        return Err("rabbit-ear target layers contain duplicates".to_string());
    }

    let positions = vertex_positions(cp);
    let mut result: [Vec<FaceId>; 3] = std::array::from_fn(|_| Vec::new());
    for &id in &input.target_layers {
        let face = faces
            .iter()
            .find(|face| face.id == id)
            .ok_or_else(|| format!("rabbit-ear target layer {id} does not exist"))?;
        let placement = state
            .placements
            .get(&id)
            .ok_or_else(|| format!("rabbit-ear target layer {id} has no flat placement"))?;
        let local_vertex = placement.inverse().apply(vertex);
        if !point_in_face(cp, face, [local_vertex.x, local_vertex.y]) {
            return Err(format!(
                "rabbit-ear target layer {id} does not meet the common vertex"
            ));
        }
        let polygon = face
            .vertices
            .iter()
            .filter_map(|vertex| positions.get(vertex).copied())
            .map(|point| placement.apply(point))
            .collect::<Vec<_>>();
        let mut used = false;
        for (index, region) in regions.iter().enumerate() {
            if clipped_area(&polygon, region) > AREA_EPS {
                result[index].push(id);
                used = true;
            }
        }
        if !used {
            return Err(format!(
                "rabbit-ear target layer {id} does not cover a moving sector"
            ));
        }
    }
    for (index, layers) in result.iter().enumerate() {
        if layers.is_empty() {
            return Err(format!(
                "rabbit-ear has insufficient local layers in moving sector {}",
                index + 1
            ));
        }
    }
    Ok(result)
}

fn clipped_area(polygon: &[DVec2], region: &[HalfPlane]) -> f64 {
    let mut clipped = polygon.to_vec();
    for half_plane in region {
        if clipped.len() < 3 {
            return 0.0;
        }
        let a = DVec2::from(half_plane.line[0]);
        let direction = (DVec2::from(half_plane.line[1]) - a).normalize();
        let sign = direction
            .perp_dot(DVec2::from(half_plane.inside_point) - a)
            .signum();
        let distance = |point: DVec2| sign * direction.perp_dot(point - a);
        let mut next = Vec::with_capacity(clipped.len() + 1);
        for index in 0..clipped.len() {
            let p = clipped[index];
            let q = clipped[(index + 1) % clipped.len()];
            let dp = distance(p);
            let dq = distance(q);
            if dp >= -EPS {
                next.push(p);
            }
            if (dp > EPS && dq < -EPS) || (dp < -EPS && dq > EPS) {
                next.push(p + (q - p) * (dp / (dp - dq)));
            }
        }
        clipped = next;
    }
    if clipped.len() < 3 {
        return 0.0;
    }
    clipped
        .iter()
        .zip(clipped.iter().cycle().skip(1))
        .take(clipped.len())
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f64>()
        .abs()
        * 0.5
}

fn has_fold_driver(result: &FoldThroughResult, crease: [[f64; 2]; 2]) -> bool {
    let a = DVec2::from(crease[0]);
    let direction = (DVec2::from(crease[1]) - a).normalize();
    result.step.drivers.iter().any(|driver| {
        let p = DVec2::from(driver.a);
        let q = DVec2::from(driver.b);
        direction.perp_dot(p - a).abs() <= CONCURRENT_EPS
            && direction.perp_dot(q - a).abs() <= CONCURRENT_EPS
            && driver.target_angle_deg.abs() > 90.0
    })
}

fn validate_state(state: &FlatState, faces: &[Face]) -> Result<(), String> {
    let expected = faces.iter().map(|face| face.id).collect::<HashSet<_>>();
    let placements = state.placements.keys().copied().collect::<HashSet<_>>();
    let order = state.order.iter().copied().collect::<HashSet<_>>();
    if placements != expected
        || state.placements.len() != faces.len()
        || order != expected
        || state.order.len() != faces.len()
    {
        return Err("rabbit-ear result did not retain every face exactly once".to_string());
    }
    Ok(())
}

fn validate_face_coverage(
    before_cp: &CreasePattern,
    before: &[Face],
    after_cp: &CreasePattern,
    after: &[Face],
) -> Result<(), String> {
    if before.is_empty() || after.is_empty() {
        return Err("rabbit-ear face extraction returned no paper faces".to_string());
    }
    let mut area_by_parent = before
        .iter()
        .map(|face| (face.id, 0.0))
        .collect::<HashMap<_, _>>();
    let mut children_by_parent = before
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
                "rabbit-ear result face {} has {} original parents",
                child.id,
                parents.len()
            ));
        }
        let parent = parents[0].id;
        *children_by_parent.get_mut(&parent).expect("known parent") += 1;
        *area_by_parent.get_mut(&parent).expect("known parent") += face_area(after_cp, child)?;
    }
    for parent in before {
        if children_by_parent[&parent.id] == 0 {
            return Err(format!("rabbit-ear lost original face {}", parent.id));
        }
        let expected = face_area(before_cp, parent)?;
        let actual = area_by_parent[&parent.id];
        if (actual - expected).abs() > STATE_EPS.max(expected * STATE_EPS) {
            return Err(format!(
                "rabbit-ear descendants of face {} cover area {actual:.12}, expected {expected:.12}",
                parent.id
            ));
        }
    }
    Ok(())
}

fn face_area(cp: &CreasePattern, face: &Face) -> Result<f64, String> {
    let positions = vertex_positions(cp);
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
    Ok(points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f64>()
        .abs()
        * 0.5)
}

fn vertex_positions(cp: &CreasePattern) -> HashMap<u32, DVec2> {
    cp.vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect()
}
