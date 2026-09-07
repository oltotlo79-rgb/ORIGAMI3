//! Endpoint audits from declared signed angles, independent of replay/display order selection.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::Isometry2;
use ori3_layers::fold_through::resolve_driver_edges;
use ori3_layers::precrease_collapse::{
    PrecreaseStackDiagnosis, PrecreaseStackRule, PrecreaseStackSatisfiability,
    diagnose_precrease_layer_order,
};
use ori3_model::{CreasePattern, DriverLine, EPS, Edge, EdgeKind, FaceId, Vertex};

// Same 34 material edges as acceptance_crane::crane's second stage. Their signs come from
// the declared drivers below; no preferred/saved order participates in this audit.
const BIRD_EDGES: [u32; 34] = [
    0, 1, 4, 6, 7, 8, 9, 11, 12, 15, 18, 21, 22, 27, 31, 40, 41, 46, 47, 52, 55, 58, 62, 63, 72,
    73, 81, 82, 84, 85, 101, 102, 103, 104,
];

fn fixture(source: &str) -> (CreasePattern, Vec<Vec<DriverLine>>) {
    let cp = json_field(source, "cp");
    let pattern = CreasePattern {
        vertices: json_array_items(json_field(cp, "vertices"))
            .into_iter()
            .map(|vertex| Vertex {
                id: json_u32(json_field(vertex, "id")),
                pos: json_point(json_field(vertex, "pos")),
            })
            .collect(),
        edges: json_array_items(json_field(cp, "edges"))
            .into_iter()
            .map(|edge| Edge {
                id: json_u32(json_field(edge, "id")),
                v0: json_u32(json_field(edge, "v0")),
                v1: json_u32(json_field(edge, "v1")),
                kind: match json_text(json_field(edge, "kind")) {
                    "Border" => EdgeKind::Border,
                    "Mountain" => EdgeKind::Mountain,
                    "Valley" => EdgeKind::Valley,
                    "Aux" => EdgeKind::Aux,
                    other => panic!("unknown edge kind {other}"),
                },
            })
            .collect(),
        next_vertex_id: json_u32(json_field(cp, "next_vertex_id")),
        next_edge_id: json_u32(json_field(cp, "next_edge_id")),
    };
    let sequence = json_array_items(json_field(source, "sequence"))
        .into_iter()
        .map(|step| {
            json_array_items(json_field(step, "drivers"))
                .into_iter()
                .map(|driver| DriverLine {
                    a: json_point(json_field(driver, "a")),
                    b: json_point(json_field(driver, "b")),
                    target_angle_deg: json_f64(json_field(driver, "target_angle_deg")),
                })
                .collect()
        })
        .collect();
    (pattern, sequence)
}

pub(crate) fn angles_for(cp: &CreasePattern, sequence: &[Vec<DriverLine>]) -> BTreeMap<u32, f64> {
    let mut angles = BTreeMap::new();
    for driver in sequence.iter().flatten() {
        assert!(driver.target_angle_deg.is_finite());
        assert!(
            driver.target_angle_deg.abs() <= EPS
                || (driver.target_angle_deg.abs() - 180.0).abs() <= EPS
        );
        for edge in resolve_driver_edges(cp, driver) {
            angles.insert(edge, driver.target_angle_deg);
        }
    }
    angles
}

fn declared_pose(
    cp: &CreasePattern,
    angles: &BTreeMap<u32, f64>,
) -> (CreasePattern, Vec<Face>, HashMap<FaceId, Isometry2>) {
    let faces = extract_faces(cp);
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let edges = cp
        .edges
        .iter()
        .map(|edge| (edge.id, edge))
        .collect::<HashMap<_, _>>();
    let mut owners = BTreeMap::<u32, Vec<FaceId>>::new();
    for face in &faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().push(face.id);
        }
    }
    let mut neighbors = HashMap::<FaceId, Vec<(FaceId, u32)>>::new();
    for (&edge, incident) in &owners {
        if let [a, b] = incident.as_slice() {
            neighbors.entry(*a).or_default().push((*b, edge));
            neighbors.entry(*b).or_default().push((*a, edge));
        }
    }
    let mut placements = HashMap::new();
    for face in &faces {
        if placements.contains_key(&face.id) {
            continue;
        }
        placements.insert(face.id, Isometry2::identity());
        let mut queue = VecDeque::from([face.id]);
        while let Some(parent) = queue.pop_front() {
            for &(child, id) in neighbors.get(&parent).into_iter().flatten() {
                if placements.contains_key(&child) {
                    continue;
                }
                let edge = edges[&id];
                let angle = angles.get(&id).copied().unwrap_or(0.0);
                let placement = if angle.abs() > 90.0 {
                    placements[&parent].compose(&Isometry2::reflection(
                        positions[&edge.v0],
                        positions[&edge.v1],
                    ))
                } else {
                    placements[&parent]
                };
                placements.insert(child, placement);
                queue.push_back(child);
            }
        }
    }
    let mut posed = cp.clone();
    for edge in &mut posed.edges {
        // +180 is Mountain and -180 is Valley, the existing angle_of convention.
        if let Some(&angle) = angles.get(&edge.id) {
            if angle > 90.0 {
                edge.kind = EdgeKind::Mountain;
            }
            if angle < -90.0 {
                edge.kind = EdgeKind::Valley;
            }
        }
    }
    (posed, faces, placements)
}

fn rule_edges(rule: &PrecreaseStackRule) -> Vec<u32> {
    match rule {
        PrecreaseStackRule::AdjacentFold { edge, .. } => vec![*edge],
        PrecreaseStackRule::Crossing { edges, .. } => edges.clone(),
        PrecreaseStackRule::Nest { edges, .. } | PrecreaseStackRule::Parallel { edges, .. } => {
            edges.to_vec()
        }
    }
}

fn rule_faces(rule: &PrecreaseStackRule) -> Vec<FaceId> {
    match rule {
        PrecreaseStackRule::AdjacentFold { lower, upper, .. } => vec![*lower, *upper],
        PrecreaseStackRule::Crossing { faces, .. } => faces.to_vec(),
        PrecreaseStackRule::Nest { faces, .. } | PrecreaseStackRule::Parallel { faces, .. } => {
            faces.to_vec()
        }
    }
}

// Independent permutation oracle: it never uses StackRelation or its propagation rules.
// Assigned faces form a bottom prefix; every unassigned face must be above that prefix.
fn prefix_respects(rule: &PrecreaseStackRule, rank: &HashMap<FaceId, usize>) -> bool {
    let below = |a, b| match (rank.get(&a), rank.get(&b)) {
        (Some(a), Some(b)) => Some(a < b),
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    };
    let agree = |a: Option<bool>, b: Option<bool>| a.zip(b).is_none_or(|(a, b)| a == b);
    let between = |middle, a, b| below(a, middle).zip(below(middle, b)).map(|(a, b)| a == b);
    match rule {
        PrecreaseStackRule::AdjacentFold { lower, upper, .. } => {
            below(*lower, *upper) != Some(false)
        }
        PrecreaseStackRule::Crossing {
            faces: [a, b, c], ..
        } => agree(below(*c, *a), below(*c, *b)),
        PrecreaseStackRule::Nest {
            faces: [a, b, c, d],
            ..
        } => {
            agree(between(*c, *a, *b), between(*d, *a, *b))
                && agree(between(*a, *c, *d), between(*b, *c, *d))
        }
        PrecreaseStackRule::Parallel {
            faces: [a, b, c, d],
            ..
        } => agree(below(*a, *c), below(*b, *d)),
    }
}

fn independent_sat(rules: &[PrecreaseStackRule]) -> bool {
    fn search(
        faces: &[FaceId],
        rules: &[PrecreaseStackRule],
        rank: &mut HashMap<FaceId, usize>,
    ) -> bool {
        if !rules.iter().all(|rule| prefix_respects(rule, rank)) {
            return false;
        }
        if rank.len() == faces.len() {
            return true;
        }
        for &face in faces {
            if rank.contains_key(&face) {
                continue;
            }
            rank.insert(face, rank.len());
            if search(faces, rules, rank) {
                return true;
            }
            rank.remove(&face);
        }
        false
    }
    let faces = rules
        .iter()
        .flat_map(rule_faces)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    search(&faces, rules, &mut HashMap::new())
}

pub(crate) fn audit(
    name: &str,
    cp: &CreasePattern,
    angles: &BTreeMap<u32, f64>,
) -> PrecreaseStackDiagnosis {
    let (posed, faces, placements) = declared_pose(cp, angles);
    let started = std::time::Instant::now();
    let diagnosis = diagnose_precrease_layer_order(&posed, &faces, &placements)
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    let (status, core_len) = match &diagnosis.satisfiability {
        PrecreaseStackSatisfiability::Sat { order } => {
            let rank = order
                .iter()
                .enumerate()
                .map(|(rank, &face)| (face, rank))
                .collect::<HashMap<_, _>>();
            assert_eq!(order.len(), faces.len());
            assert_eq!(rank.len(), faces.len());
            assert!(faces.iter().all(|face| rank.contains_key(&face.id)));
            assert!(
                diagnosis
                    .rules
                    .iter()
                    .all(|rule| prefix_respects(rule, &rank)),
                "{name}: generated SAT order must satisfy every rule independently"
            );
            ("SAT", 0)
        }
        PrecreaseStackSatisfiability::Unsat { minimal_rules } => {
            assert!(!minimal_rules.is_empty());
            assert!(
                !independent_sat(minimal_rules),
                "{name}: the reported core is UNSAT independently"
            );
            for index in 0..minimal_rules.len() {
                let mut subset = minimal_rules.clone();
                subset.remove(index);
                assert!(
                    independent_sat(&subset),
                    "{name}: removing core rule {index} must make it SAT"
                );
            }
            println!("G0_CORE {name} {minimal_rules:?}");
            ("UNSAT", minimal_rules.len())
        }
    };
    assert!(
        diagnosis
            .diagnostics
            .rejected_branches
            .iter()
            .all(|branch| !branch.conflicts.is_empty() || branch.violated_rule.is_some())
    );
    let gap = diagnosis
        .diagnostics
        .seam_residuals
        .iter()
        .flat_map(|seam| seam.endpoint_gaps)
        .fold(0.0_f64, f64::max);
    assert!(gap <= EPS);
    let mixed = diagnosis
        .rules
        .iter()
        .filter(
            |rule| matches!(rule, PrecreaseStackRule::Crossing { edges, .. } if edges.len() == 2),
        )
        .count();
    println!(
        "G0_GENERATION {name} mixed_seam_crossings={mixed} total_rules={}",
        diagnosis.rules.len()
    );
    println!(
        "G0_AUDIT {name} adjacent={} taco_tortilla={} taco_taco={} continuous={} mandatory={} status={status} core={core_len} rejected_constraints={} rejected_branches={} max_seam_gap={gap:.12e} elapsed={:.6}s",
        diagnosis.counts.adjacent_folds,
        diagnosis.counts.taco_tortilla,
        diagnosis.counts.taco_taco,
        diagnosis.counts.continuous,
        diagnosis.mandatory_constraints.len(),
        diagnosis.diagnostics.rejected_constraints.len(),
        diagnosis.diagnostics.rejected_branch_count(),
        started.elapsed().as_secs_f64()
    );
    diagnosis
}

#[test]
fn legacy_crane_crossings_are_a_subset_of_analytic_crossings() {
    // Exact HEAD sampler, confined to this regression test. point_in_face uses the same
    // boundary-inclusive point_in_polygon predicate that the old crosses_segment used.
    fn legacy_nine_points(cp: &CreasePattern, face: &Face, segment: [DVec2; 2]) -> bool {
        let direction = segment[1] - segment[0];
        if direction.length() <= EPS {
            return false;
        }
        let normal = direction.normalize().perp() * 1e-6;
        (1..=9).any(|step| {
            let point = segment[0] + direction * (f64::from(step) / 10.0);
            ori3_layers::flat_state::point_in_face(cp, face, (point + normal).to_array())
                && ori3_layers::flat_state::point_in_face(cp, face, (point - normal).to_array())
        })
    }

    // Measure the actual intervals independently of the product's rule catalogue.
    // Boundary intersections partition the finite segment; no fixed sample grid is used.
    fn interior_intervals(
        cp: &CreasePattern,
        face: &Face,
        polygon: &[DVec2],
        segment: [DVec2; 2],
    ) -> Vec<[f64; 2]> {
        let (a, b) = (segment[0], segment[1]);
        let direction = b - a;
        let length2 = direction.length_squared();
        let mut parameters = vec![0.0, 1.0];
        for (index, &c) in polygon.iter().enumerate() {
            let d = polygon[(index + 1) % polygon.len()];
            let edge = d - c;
            let denominator = direction.perp_dot(edge);
            if denominator.abs() <= EPS {
                if direction.perp_dot(c - a).abs() <= EPS * direction.length() {
                    parameters.push(((c - a).dot(direction) / length2).clamp(0.0, 1.0));
                    parameters.push(((d - a).dot(direction) / length2).clamp(0.0, 1.0));
                }
                continue;
            }
            let t = (c - a).perp_dot(edge) / denominator;
            let u = (c - a).perp_dot(direction) / denominator;
            if (-EPS..=1.0 + EPS).contains(&t) && (-EPS..=1.0 + EPS).contains(&u) {
                parameters.push(t.clamp(0.0, 1.0));
            }
        }
        parameters.sort_by(f64::total_cmp);
        parameters.dedup_by(|x, y| (*x - *y).abs() <= EPS);
        parameters
            .windows(2)
            .filter_map(|pair| {
                if pair[1] - pair[0] <= EPS {
                    return None;
                }
                let midpoint = a + direction * (0.5 * (pair[0] + pair[1]));
                let on_boundary = polygon.iter().enumerate().any(|(index, &c)| {
                    let edge = polygon[(index + 1) % polygon.len()] - c;
                    let projection = if edge.length_squared() <= EPS * EPS {
                        c
                    } else {
                        c + edge
                            * ((midpoint - c).dot(edge) / edge.length_squared()).clamp(0.0, 1.0)
                    };
                    midpoint.distance_squared(projection) <= EPS * EPS
                });
                (!on_boundary
                    && ori3_layers::flat_state::point_in_face(cp, face, midpoint.to_array()))
                .then_some([pair[0], pair[1]])
            })
            .collect()
    }

    let (cp, sequence) = fixture(include_str!(
        "fixtures/traditional-crane/traditional-crane-cp.ori3"
    ));
    let (posed, faces, placements) = declared_pose(&cp, &angles_for(&cp, &sequence));
    let diagnosis = diagnose_precrease_layer_order(&posed, &faces, &placements)
        .expect("canonical crane geometry");
    assert!(matches!(
        diagnosis.satisfiability,
        PrecreaseStackSatisfiability::Sat { .. }
    ));
    assert_eq!(diagnosis.mandatory_constraints.len(), 1388);
    let analytic = diagnosis
        .rules
        .iter()
        .filter_map(|rule| match rule {
            PrecreaseStackRule::Crossing {
                edges,
                folded: true,
                faces: [_, _, crossing],
            } => {
                assert_eq!(edges.len(), 1, "crane has no mixed-seam crossing rules");
                Some((edges[0], *crossing))
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let positions = posed
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let mut legacy = BTreeSet::new();
    let mut measured_new = BTreeSet::new();
    for edge in &posed.edges {
        let owners = faces
            .iter()
            .filter(|face| face.edges.contains(&edge.id))
            .collect::<Vec<_>>();
        let [a, b] = owners.as_slice() else {
            continue;
        };
        if placements[&a.id].mirrored == placements[&b.id].mirrored {
            continue;
        }
        let start = placements[&a.id].apply(positions[&edge.v0]);
        let end = placements[&a.id].apply(positions[&edge.v1]);
        for face in &faces {
            if face.id == a.id || face.id == b.id {
                continue;
            }
            let polygon = face
                .vertices
                .iter()
                .map(|id| positions[id])
                .collect::<Vec<_>>();
            let (minimum, maximum) = polygon.iter().fold(
                (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
                |(minimum, maximum), &point| {
                    let point = placements[&face.id].apply(point);
                    (minimum.min(point), maximum.max(point))
                },
            );
            let (low, high) = (start.min(end), start.max(end));
            if maximum.x + EPS < low.x
                || minimum.x - EPS > high.x
                || maximum.y + EPS < low.y
                || minimum.y - EPS > high.y
            {
                continue;
            }
            let inverse = placements[&face.id].inverse();
            let segment = [inverse.apply(start), inverse.apply(end)];
            let key = (edge.id, face.id);
            if legacy_nine_points(&posed, face, segment) {
                legacy.insert(key);
                assert!(
                    analytic.contains(&key),
                    "lost legacy crossing edge/face={key:?}"
                );
            } else if analytic.contains(&key) {
                measured_new.insert(key);
                let intervals = interior_intervals(&posed, face, &polygon, segment);
                assert!(
                    intervals.iter().all(|pair| {
                        (1..=9).all(|step| {
                            let t = f64::from(step) / 10.0;
                            t <= pair[0] + EPS || t >= pair[1] - EPS
                        })
                    }),
                    "edge/face={key:?}: all nine old samples must miss the interior intervals"
                );
                let widths = intervals
                    .iter()
                    .map(|pair| (pair[1] - pair[0]) * (end - start).length())
                    .collect::<Vec<_>>();
                assert!(widths.iter().any(|&width| width > EPS));
                println!(
                    "G0_ADDED_CROSSING edge={} face={} seam_faces=[{},{}] start={:?} end={:?} t_intervals={intervals:?} widths={widths:?}",
                    edge.id,
                    face.id,
                    a.id,
                    b.id,
                    start.to_array(),
                    end.to_array()
                );
            }
        }
    }
    assert_eq!(legacy.len(), 987);
    assert_eq!(analytic.len(), 1049);
    assert!(legacy.is_subset(&analytic));
    assert_eq!(
        measured_new,
        analytic.difference(&legacy).copied().collect()
    );
    assert_eq!(measured_new.len(), 62);
    println!("G0_SUBSET legacy=987 analytic=1049 lost=0 added=62 mandatory=1388 status=SAT");
}

#[test]
fn canonical_crane_and_bird_have_auditable_stack_rules() {
    let (cp, sequence) = fixture(include_str!(
        "fixtures/traditional-crane/traditional-crane-cp.ori3"
    ));
    let full = angles_for(&cp, &sequence);
    let crane = audit("crane_complete", &cp, &full);
    assert!(matches!(
        crane.satisfiability,
        PrecreaseStackSatisfiability::Sat { .. }
    ));
    let bird = full
        .into_iter()
        .filter(|(edge, _)| BIRD_EDGES.contains(edge))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(bird.len(), 34);
    let center = cp
        .edges
        .iter()
        .filter(|edge| edge.v0 == 4 || edge.v1 == 4)
        .filter(|edge| bird.contains_key(&edge.id))
        .map(|edge| edge.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(center, BTreeSet::from([7, 9, 18, 21]));
    assert!(center.iter().all(|edge| bird[edge] > 90.0));
    let diagnosis = audit("crane_34_edge_bird", &cp, &bird);
    let PrecreaseStackSatisfiability::Unsat { minimal_rules } = diagnosis.satisfiability else {
        panic!("four mountains at central vertex 4 cannot fold flat")
    };
    assert!(
        minimal_rules
            .iter()
            .flat_map(rule_edges)
            .any(|edge| center.contains(&edge))
    );
}

#[test]
fn all_eight_declared_sample_endpoints_have_auditable_stack_rules() {
    let (cp, sequence) = fixture(include_str!("fixtures/folded-sample.ori3"));
    assert_eq!(sequence.len(), 8);
    for step in 1..=sequence.len() {
        audit(
            &format!("folded_sample_step_{step}"),
            &cp,
            &angles_for(&cp, &sequence[..step]),
        );
    }
}

#[test]
fn yakko_declared_endpoint_has_auditable_stack_rules() {
    let (cp, sequence) = fixture(include_str!(
        "../../ori3-rigid/tests/fixtures/check-yakko.ori3"
    ));
    let diagnosis = audit("yakko_complete", &cp, &angles_for(&cp, &sequence));
    assert!(matches!(
        diagnosis.satisfiability,
        PrecreaseStackSatisfiability::Sat { .. }
    ));
}

// Fixed-fixture reader copied from flat_endpoint.rs; ori3-layers has no serde_json dependency.
fn json_field<'a>(source: &'a str, key: &str) -> &'a str {
    let marker = format!("\"{key}\"");
    let key_end = source
        .find(&marker)
        .map(|index| index + marker.len())
        .unwrap_or_else(|| panic!("項目 {key} がない"));
    json_value(
        source[key_end..]
            .trim_start()
            .strip_prefix(':')
            .unwrap_or_else(|| panic!("項目 {key} のあとにコロンがない"))
            .trim_start(),
    )
}

fn json_array_items(array: &str) -> Vec<&str> {
    let inner = array
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .expect("JSONの配列");
    let mut rest = inner.trim();
    let mut items = Vec::new();
    while !rest.is_empty() {
        let value = json_value(rest);
        items.push(value);
        rest = rest[value.len()..].trim_start();
        if let Some(after_comma) = rest.strip_prefix(',') {
            rest = after_comma.trim_start();
        } else {
            assert!(rest.is_empty(), "配列の要素のあいだにコンマがない");
        }
    }
    items
}

fn json_value(source: &str) -> &str {
    let source = source.trim_start();
    let first = *source.as_bytes().first().expect("空でない値");
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
    panic!("閉じていないJSON")
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
    panic!("閉じていない文字列")
}

fn json_text(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .expect("JSONの文字列")
}

fn json_u32(value: &str) -> u32 {
    value.parse().expect("JSONの整数")
}

fn json_f64(value: &str) -> f64 {
    value.parse().expect("JSONの小数")
}

fn json_point(value: &str) -> [f64; 2] {
    let coordinates = json_array_items(value);
    assert_eq!(coordinates.len(), 2, "2次元の点");
    [json_f64(coordinates[0]), json_f64(coordinates[1])]
}
