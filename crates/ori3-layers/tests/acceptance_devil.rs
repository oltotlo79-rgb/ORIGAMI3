//! 前川淳「悪魔」の手順17〜29を、平坦状態どうしの折り操作だけで再現する受け入れテスト。

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use glam::DVec2;
use ori3_cp::{Face, extract_faces, insert_segment, local_violations, validate};
use ori3_geometry::align::existing_line;
use ori3_layers::folded_query::FoldedQuery;
use ori3_layers::{
    CompoundMotionSession, FlatMotionInput, FlatState, FoldDirection, FoldThroughResult,
    HalfPlane, LayerTurn, MotionPart, MotionTransform, PrecreaseCollapseInput,
    PoseAngleTarget, PoseDepthExpectation, PoseEdgeActivation, PoseExpectation,
    PoseLandmarkExpectation, PoseMotionInput, PoseMotionResult, Ray3, ReverseFoldNetworkInput,
    collapse_precrease_network, compose_flat_motion_step, evaluate_pose, flat_motion,
    flat_state_at, inside_reverse, layers_at_point, replay, solve_and_apply_pose_step,
};
use ori3_layers::techniques::TechniqueInput;
use ori3_model::{CreasePattern, Document, EdgeKind, FoldStep, Paper, TechniqueKind};
use ori3_rigid::max_seam_gap;

#[path = "support/devil_verify.rs"]
mod devil_verify;
#[path = "support/devil_fixture.rs"]
mod devil_fixture;

fn devil_logical_lines() -> [([f64; 2], [f64; 2]); 22] {
    let sqrt2 = std::f64::consts::SQRT_2;
    let t = sqrt2 - 1.0;
    let q = 2.0 - sqrt2;
    let s = 2.0 * t;
    let e = 2.0 * q - 1.0;
    let a = sqrt2 / 4.0;
    let b = (2.0 + sqrt2) / 4.0;
    let k = 4.0 * t - 1.0;
    [
        ([0.0, 0.0], [1.0, 1.0]),
        ([1.0, 0.0], [0.0, 1.0]),
        ([1.0, 1.0], [0.0, q]),
        ([1.0, 1.0], [q, 0.0]),
        ([0.0, 1.0], [1.0, q]),
        ([1.0, 0.0], [q, 1.0]),
        ([0.0, q], [1.0, q]),
        ([q, 0.0], [q, 1.0]),
        ([0.0, t], [q, 1.0]),
        ([t, 0.0], [1.0, q]),
        ([0.0, t], [t, 0.0]),
        ([q, 1.0], [1.0, q]),
        ([s, 0.0], [t, 1.0]),
        ([0.0, s], [1.0, t]),
        ([q, 0.0], [e, 1.0]),
        ([0.0, q], [1.0, e]),
        ([0.0, s], [s, 0.0]),
        ([e, 1.0], [1.0, e]),
        ([0.0, a], [b, 0.0]),
        ([a, 0.0], [0.0, b]),
        ([0.0, k], [k, 0.0]),
        ([0.0, 0.5], [0.5, 0.0]),
    ]
}

fn precreased_devil() -> Document {
    let mut document = Document::new(Paper {
        width_mm: 250.0,
        height_mm: 250.0,
    });
    for (index, (a, b)) in devil_logical_lines().into_iter().enumerate() {
        insert_segment(
            &mut document.cp,
            a,
            b,
            if index == 0 {
                EdgeKind::Valley
            } else {
                EdgeKind::Aux
            },
        );
    }
    assert_eq!(document.cp.vertices.len(), 92);
    assert_eq!(document.cp.edges.len(), 201);
    assert_eq!(extract_faces(&document.cp).len(), 2);
    assert!(local_violations(&document.cp).is_empty());
    assert!(validate(&document.cp).is_empty());
    document
}

fn append_step(document: &mut Document, mut step: FoldStep) {
    step.id = u32::try_from(document.sequence.len()).expect("手順数はu32に収まる");
    document.sequence.push(step);
}

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<FoldThroughResult, String>;

fn state_of(document: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&document.cp);
    let (state, warnings) = flat_state_at(document, &faces, document.sequence.len())
        .expect("現在状態は平坦");
    assert!(warnings.is_empty(), "現在状態の警告: {warnings:?}");
    (faces, state)
}

fn apply_technique(
    document: &mut Document,
    technique: Technique,
    input: TechniqueInput,
) -> FlatState {
    let (faces, state) = state_of(document);
    let mut cp = document.cp.clone();
    let result = technique(&mut cp, &faces, &state, &input).expect("技法を適用できる");
    assert!(result.warnings.is_empty(), "技法の警告: {:?}", result.warnings);
    document.cp = cp;
    append_step(document, result.step);
    result.state
}

fn mapped_polygon(document: &Document, face: &Face, state: &FlatState) -> Vec<[f64; 2]> {
    let positions = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    face.vertices
        .iter()
        .map(|vertex| {
            let point = state.placements[&face.id].apply(positions[vertex]);
            [point.x, point.y]
        })
        .collect()
}

fn write_flat_svg(document: &Document, path: &Path) {
    let (faces, state) = state_of(document);
    let polygons = faces
        .iter()
        .map(|face| (face.id, mapped_polygon(document, face, &state)))
        .collect::<Vec<_>>();
    let mut minimum = DVec2::splat(f64::INFINITY);
    let mut maximum = DVec2::splat(f64::NEG_INFINITY);
    for (_, polygon) in &polygons {
        for point in polygon {
            minimum = minimum.min(DVec2::from(*point));
            maximum = maximum.max(DVec2::from(*point));
        }
    }
    let span = (maximum - minimum).max(DVec2::splat(1e-9));
    let scale = 900.0 / span.max_element();
    let rank = state
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let mut svg = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"960\" height=\"960\" viewBox=\"0 0 960 960\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/>",
    );
    for (face, polygon) in polygons {
        let _hue = (rank[&face] * 47) % 360;
        let points = polygon
            .iter()
            .map(|point| {
                format!(
                    "{:.3},{:.3}",
                    30.0 + (point[0] - minimum.x) * scale,
                    930.0 - (point[1] - minimum.y) * scale
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(
            svg,
            "<polygon points=\"{points}\" fill=\"none\" stroke=\"#007f7f\" stroke-width=\"1.2\"/><text x=\"{}\" y=\"{}\" font-size=\"8\">{face}</text>",
            30.0 + (polygon[0][0] - minimum.x) * scale,
            930.0 - (polygon[0][1] - minimum.y) * scale,
        )
        .unwrap();
    }
    svg.push_str("</svg>\n");
    std::fs::write(path, svg).expect("write flat-state SVG");
}

/// 畳み平面で軸を真に横切る全層を、現在の層順で返す。
fn crossing_faces(
    document: &Document,
    faces: &[Face],
    state: &FlatState,
    axis: [[f64; 2]; 2],
) -> Vec<u32> {
    let [a, b] = axis;
    let side = |point: [f64; 2]| {
        (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0])
    };
    state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let face = faces.iter().find(|face| face.id == *id).expect("面ID");
            let sides = mapped_polygon(document, face, state)
                .into_iter()
                .map(side)
                .collect::<Vec<_>>();
            sides.iter().copied().fold(f64::INFINITY, f64::min) < -1e-9
                && sides
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max)
                    > 1e-9
        })
        .collect()
}

fn reverse_on_axis(
    document: &mut Document,
    axis: [[f64; 2]; 2],
    reference_point: [f64; 2],
) -> FlatState {
    let (faces, state) = state_of(document);
    let flap = crossing_faces(document, &faces, &state, axis);
    apply_technique(
        document,
        inside_reverse,
        TechniqueInput {
            flap,
            line: axis,
            reference_point,
            open_to_back: None,
            polygon: None,
            center: None,
        },
    )
}

fn apply_motion(document: &mut Document, input: FlatMotionInput) -> ori3_layers::FlatState {
    let faces = extract_faces(&document.cp);
    let (state, warnings) = flat_state_at(document, &faces, document.sequence.len())
        .expect("直前の手順まで平らに畳める");
    assert!(warnings.is_empty(), "直前状態の警告: {warnings:?}");
    let result = flat_motion(&mut document.cp, &faces, &state, &input).expect("折り操作を適用");
    assert!(
        result.warnings.is_empty(),
        "折り操作の警告: {:?}",
        result.warnings
    );
    append_step(document, result.step);
    result.state
}

fn apply_compound<F>(document: &mut Document, build: F) -> FlatState
where
    F: FnOnce(&mut CompoundMotionSession) -> Result<(), String>,
{
    let result = compose_flat_motion_step(document, build).expect("複合手順を1手へ合成できる");
    assert!(result.warnings.is_empty(), "複合手順の警告: {:?}", result.warnings);
    let state = result.state.clone();
    append_step(document, result.step);
    state
}

fn diagonal_sum(sum: f64) -> [[f64; 2]; 2] {
    [[0.0, sum], [sum, 0.0]]
}

fn point_to_point_line(moving: [f64; 2], target: [f64; 2]) -> [[f64; 2]; 2] {
    let midpoint = [
        (moving[0] + target[0]) * 0.5,
        (moving[1] + target[1]) * 0.5,
    ];
    let delta = [target[0] - moving[0], target[1] - moving[1]];
    [
        midpoint,
        [midpoint[0] - delta[1], midpoint[1] + delta[0]],
    ]
}

fn edge_on_line(cp: &CreasePattern, edge_id: u32, line: [[f64; 2]; 2]) -> bool {
    let edge = cp.edges.iter().find(|edge| edge.id == edge_id).expect("辺ID");
    let point = |vertex| {
        DVec2::from(
            cp.vertices
                .iter()
                .find(|candidate| candidate.id == vertex)
                .expect("頂点ID")
                .pos,
        )
    };
    let a = DVec2::from(line[0]);
    let direction = (DVec2::from(line[1]) - a).normalize();
    direction.perp_dot(point(edge.v0) - a).abs() < 1e-8
        && direction.perp_dot(point(edge.v1) - a).abs() < 1e-8
}

/// Figure 24's W consists of these four exact precreases from Figures 1--16.
fn w_material_lines() -> [[[f64; 2]; 2]; 4] {
    let logical = devil_logical_lines();
    [
        [logical[20].0, logical[20].1],
        [logical[16].0, logical[16].1],
        [logical[1].0, logical[1].1],
        [logical[17].0, logical[17].1],
    ]
}

fn front_w_packet(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
) -> (Vec<u32>, Vec<[[f64; 2]; 2]>) {
    let sample_layers = layers_at_point(cp, faces, state, [0.5, 0.85]);
    let front = *sample_layers.last().expect("中央Wを覆う最前層");
    let front_face = faces.iter().find(|face| face.id == front).expect("最前面");
    let front_point = ori3_layers::representative_point(cp, front_face);
    let front_is_lower = front_point[0] > front_point[1];
    let material_lines = w_material_lines();
    let positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();

    let packet = state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let face = faces.iter().find(|face| face.id == *id).expect("面ID");
            let point = ori3_layers::representative_point(cp, face);
            (point[0] > point[1]) == front_is_lower
                && face
                    .edges
                    .iter()
                    .any(|edge| material_lines.iter().any(|line| edge_on_line(cp, *edge, *line)))
        })
        .collect::<Vec<_>>();
    let selected = packet.iter().copied().collect::<std::collections::HashSet<_>>();
    let mut visible = Vec::new();
    for face in faces.iter().filter(|face| selected.contains(&face.id)) {
        let placement = state.placements[&face.id];
        for edge_id in &face.edges {
            if !material_lines
                .iter()
                .any(|line| edge_on_line(cp, *edge_id, *line))
            {
                continue;
            }
            let edge = cp.edges.iter().find(|edge| edge.id == *edge_id).expect("辺ID");
            if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                continue;
            }
            let a = placement.apply(positions[&edge.v0]);
            let b = placement.apply(positions[&edge.v1]);
            visible.push([[a.x, a.y], [b.x, b.y]]);
        }
    }
    assert!(packet.len() >= 2, "W反転には2層以上の前面packetが必要");
    assert!(!visible.is_empty(), "Wの既存折線が前面packetにある");
    (packet, visible)
}

fn fold_step17(document: &mut Document) -> ori3_layers::FlatState {
    let moving = [1.0, 0.0];
    let target = [0.0, 1.0];
    let line = point_to_point_line(moving, target);
    apply_motion(
        document,
        FlatMotionInput {
            parts: vec![MotionPart::fold(
                Vec::new(),
                line,
                moving,
                FoldDirection::Up,
            )],
            kind: TechniqueKind::Simple,
        },
    )
}

fn fold_step18_probe(document: &mut Document) -> ori3_layers::FlatState {
    let sqrt2 = std::f64::consts::SQRT_2;
    let t = sqrt2 - 1.0;
    let q = 2.0 - sqrt2;
    let a = diagonal_sum(4.0 * t - 1.0);
    let b = diagonal_sum(2.0 * t);
    let a_b_mid = (a[0][1] + b[0][1]) * 0.25;
    let reflected_b = diagonal_sum(4.0 * t - 2.0 * q);
    let first = apply_motion(
        document,
        FlatMotionInput {
            parts: vec![
                MotionPart {
                    layers: Vec::new(),
                    region: vec![
                        HalfPlane {
                            line: a,
                            inside_point: [a_b_mid; 2],
                        },
                        HalfPlane {
                            line: b,
                            inside_point: [a_b_mid; 2],
                        },
                    ],
                    transform: MotionTransform::Reflect(vec![a]),
                    turn: LayerTurn::Outside(FoldDirection::Up),
                    reverse_layers: None,
                },
                MotionPart {
                    layers: Vec::new(),
                    region: vec![HalfPlane {
                        line: b,
                        inside_point: [1.0, 1.0],
                    }],
                    transform: MotionTransform::Reflect(vec![a, reflected_b]),
                    turn: LayerTurn::Outside(FoldDirection::Up),
                    reverse_layers: None,
                },
            ],
            kind: TechniqueKind::Pleat,
        },
    );

    let faces = extract_faces(&document.cp);
    let selected = faces
        .iter()
        .filter(|face| {
            let p = ori3_layers::representative_point(&document.cp, face);
            p[0] + p[1] > 1.0 + 1e-9
        })
        .map(|face| face.id)
        .collect::<Vec<_>>();
    let c = diagonal_sum(1.0);
    let d = diagonal_sum(2.0 * q);
    let c_d_mid = (c[0][1] + d[0][1]) * 0.25;
    let _ = first;
    apply_motion(
        document,
        FlatMotionInput {
            parts: vec![
                MotionPart {
                    layers: selected.clone(),
                    region: vec![
                        HalfPlane {
                            line: c,
                            inside_point: [c_d_mid; 2],
                        },
                        HalfPlane {
                            line: d,
                            inside_point: [c_d_mid; 2],
                        },
                    ],
                    transform: MotionTransform::Reflect(vec![c]),
                    turn: LayerTurn::Outside(FoldDirection::Up),
                    reverse_layers: None,
                },
                MotionPart {
                    layers: selected,
                    region: vec![HalfPlane {
                        line: d,
                        inside_point: [1.0, 1.0],
                    }],
                    transform: MotionTransform::Reflect(vec![c, b]),
                    turn: LayerTurn::Outside(FoldDirection::Up),
                    reverse_layers: None,
                },
            ],
            kind: TechniqueKind::Pleat,
        },
    )
}

/// 複数の既存予備線を同時に畳む手順を、1つのFlatMotionへまとめる。
///
/// `planned` は同じ開始状態の複製上で厳密な既存線を順に動かして求めた到達状態。
/// 実文書では、そこで実際に折り目になった予備線だけを有効化し、各面を開始配置から
/// 到達配置へ直接動かす。したがって書籍の1手が途中の操作数に分裂しない。
fn apply_planned_as_one_step(
    document: &mut Document,
    planned: &Document,
    kind: TechniqueKind,
) -> ori3_layers::FlatState {
    let work_cp = planned.cp.clone();
    let faces = extract_faces(&work_cp);
    let (target, target_warnings) = flat_state_at(planned, &faces, planned.sequence.len())
        .expect("計画した到達状態は平坦");
    assert!(
        target_warnings.is_empty(),
        "計画した到達状態の警告: {target_warnings:?}"
    );

    let mut current_doc = document.clone();
    current_doc.cp = work_cp.clone();
    let (current, current_warnings) =
        flat_state_at(&current_doc, &faces, current_doc.sequence.len())
            .expect("予備線を有効化しても直前状態は平坦");
    assert!(
        current_warnings.is_empty(),
        "予備線を有効化した直前状態の警告: {current_warnings:?}"
    );

    let parts = target
        .order
        .iter()
        .map(|face| MotionPart {
            layers: vec![*face],
            region: Vec::new(),
            transform: MotionTransform::Isometry(
                target.placements[face].compose(&current.placements[face].inverse()),
            ),
            turn: LayerTurn::Outside(FoldDirection::Up),
            reverse_layers: Some(false),
        })
        .collect();
    // `flat_motion` preserves the pre-existing mountain/valley mismatch when a
    // crease stays folded but the two incident layers exchange order.  A
    // compound book step can exchange them several times before landing, so
    // seed such creases with the inverse kind; the normal settling pass then
    // lands on exactly the kind produced by the planned elementary motions.
    let mut owners: HashMap<u32, Vec<u32>> = HashMap::new();
    for face in &faces {
        for edge in &face.edges {
            owners.entry(*edge).or_default().push(face.id);
        }
    }
    let current_rank = current
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let target_rank = target
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let mut committed_cp = work_cp;
    for edge in &mut committed_cp.edges {
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let Some(adjacent) = owners.get(&edge.id).filter(|faces| faces.len() == 2) else {
            continue;
        };
        let (a, b) = (adjacent[0], adjacent[1]);
        let (ca, cb) = (current.placements[&a], current.placements[&b]);
        let (ta, tb) = (target.placements[&a], target.placements[&b]);
        if ca.mirrored == cb.mirrored || ta.mirrored == tb.mirrored {
            continue;
        }
        let current_kind = expected_kind(
            current_rank[&a],
            current_rank[&b],
            ca.mirrored,
        );
        let target_kind = expected_kind(target_rank[&a], target_rank[&b], ta.mirrored);
        if current_kind != target_kind {
            edge.kind = opposite_kind(edge.kind);
        }
    }
    let result = flat_motion(
        &mut committed_cp,
        &faces,
        &current,
        &FlatMotionInput { parts, kind },
    )
    .expect("計画した平坦状態を1手で適用");
    assert!(
        result.warnings.is_empty(),
        "1手にまとめた折りの警告: {:?}",
        result.warnings
    );
    for face in &faces {
        assert!(
            result.state.placements[&face.id].approx_eq(&target.placements[&face.id], 1e-8),
            "面{}の1手化後の配置が計画と一致する",
            face.id
        );
    }
    assert_eq!(result.state.order, target.order, "1手化後の層順");
    let kind_differences = planned
        .cp
        .edges
        .iter()
        .filter_map(|expected| {
            let got = committed_cp.edges.iter().find(|edge| edge.id == expected.id)?;
            (got.kind != expected.kind).then_some((expected.id, expected.kind, got.kind))
        })
        .collect::<Vec<_>>();
    assert!(
        kind_differences.is_empty(),
        "1手化後の山谷が計画と一致する: {kind_differences:?}"
    );
    document.cp = committed_cp;
    append_step(document, result.step);
    result.state
}

fn expected_kind(rank_a: usize, rank_b: usize, a_mirrored: bool) -> EdgeKind {
    if (rank_b > rank_a) == a_mirrored {
        EdgeKind::Mountain
    } else {
        EdgeKind::Valley
    }
}

fn opposite_kind(kind: EdgeKind) -> EdgeKind {
    match kind {
        EdgeKind::Mountain => EdgeKind::Valley,
        EdgeKind::Valley => EdgeKind::Mountain,
        other => other,
    }
}

/// 計画用の途中操作が「境界として使っただけで、到達時には開いている」予備線を
/// 補助線へ戻す。180度折れている区間だけを本当の折り目として残す。
fn keep_only_folded_precreases(planned: &mut Document) {
    loop {
        let faces = extract_faces(&planned.cp);
        let (state, warnings) = flat_state_at(planned, &faces, planned.sequence.len())
            .expect("計画状態は平坦");
        assert!(warnings.is_empty(), "計画状態の警告: {warnings:?}");
        let mut owners: HashMap<u32, Vec<u32>> = HashMap::new();
        for face in &faces {
            for edge in &face.edges {
                owners.entry(*edge).or_default().push(face.id);
            }
        }
        let opened = planned
            .cp
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .filter_map(|edge| {
                let adjacent = owners.get(&edge.id)?;
                (adjacent.len() == 2
                    && state.placements[&adjacent[0]].mirrored
                        == state.placements[&adjacent[1]].mirrored)
                    .then_some(edge.id)
            })
            .collect::<Vec<_>>();
        if opened.is_empty() {
            break;
        }
        for edge in &mut planned.cp.edges {
            if opened.contains(&edge.id) {
                edge.kind = EdgeKind::Aux;
            }
        }
    }
}

fn fold_step18(document: &mut Document) -> ori3_layers::FlatState {
    let mut planned = document.clone();
    fold_step18_probe(&mut planned);
    keep_only_folded_precreases(&mut planned);
    println!(
        "step18 planned faces={} violations={:?}",
        extract_faces(&planned.cp).len(),
        local_violations(&planned.cp)
    );
    apply_planned_as_one_step(document, &planned, TechniqueKind::InsideReverse)
}

fn fold_step19(document: &mut Document) -> FlatState {
    let q = 2.0 - std::f64::consts::SQRT_2;
    reverse_on_axis(document, [[q, 0.0], [q, 1.0]], [0.0, 1.0])
}

fn fold_step20(document: &mut Document) -> FlatState {
    let sqrt2 = std::f64::consts::SQRT_2;
    let c = 3.5 - 2.0 * sqrt2;
    reverse_on_axis(
        document,
        [[c, c], [6.0 - 4.0 * sqrt2, 1.0]],
        [0.0, 0.0],
    )
}

fn fold_step21(document: &mut Document) -> FlatState {
    let half_diagonal = std::f64::consts::SQRT_2 * 0.5;
    reverse_on_axis(
        document,
        [[half_diagonal, half_diagonal], [0.0, 1.0]],
        [0.0, 0.0],
    )
}

fn fold_step22(document: &mut Document) -> FlatState {
    let sqrt2 = std::f64::consts::SQRT_2;
    let t = sqrt2 - 1.0;
    let k = 4.0 * sqrt2 - 5.0;
    let mut planned = document.clone();
    reverse_on_axis(&mut planned, [[0.0, t], [1.0, t]], [1.0, 1.0]);
    reverse_on_axis(&mut planned, diagonal_sum(k), [1.0, 1.0]);
    let c = 1.0 - sqrt2 * 0.5;
    reverse_on_axis(&mut planned, [[c, c], [0.0, 1.0]], [1.0, 1.0]);
    apply_planned_as_one_step(document, &planned, TechniqueKind::InsideReverse)
}

fn fold_step23(document: &mut Document) -> FlatState {
    let mut planned = document.clone();
    planned.sequence.truncate(1);
    apply_planned_as_one_step(document, &planned, TechniqueKind::Simple)
}

fn fold_step24(document: &mut Document) -> FlatState {
    apply_compound(document, |session| {
        let (packet, creases) =
            front_w_packet(session.crease_pattern(), session.faces(), session.state());
        println!(
            "step24 front_w_layers={} visible_crease_segments={}",
            packet.len(),
            creases.len()
        );
        session.apply_reverse_fold_network(&ReverseFoldNetworkInput {
            target_layers: packet,
            creases,
        })?;
        Ok(())
    })
}

/// Resolve the five step-25 crease fragments on the visible sheet.  The probes deliberately use
/// a separate strictly-interior layer seed: the crease point itself lies on a boundary and cannot
/// identify front/back depth.
fn step25_visible_edges(document: &Document) -> [u32; 5] {
    let (faces, state) = state_of(document);
    let query = FoldedQuery::new(&document.cp, &faces, &state).expect("手順24の層付き幾何");
    let face2_seed = [0.1262265521467857, 0.8333333333333334];
    let face3_seed = [0.16666666666666655, 0.8737734478532143];
    let probes = [
        ([0.3786796564403573, 0.5], face2_seed),
        ([0.318_019_484_660_535_9, 0.6464466094067263], face2_seed),
        ([0.439_339_828_220_178_4, 0.6464466094067263], face3_seed),
        ([0.14644660940672605, 0.8535533905932737], face3_seed),
        ([0.5, 0.5857864376269049], face3_seed),
    ];
    let selected = probes.map(|(edge_point, layer_seed)| {
        let nearest = query
            .nearest_edge_on_sheet(edge_point, layer_seed, 1)
            .expect("手前1枚目の折り筋を選択");
        assert!(nearest.distance < 1e-9, "折り筋probe距離={}", nearest.distance);
        nearest.edge_id
    });
    assert_eq!(selected, [149, 150, 108, 17, 63]);
    selected
}

fn fold_step25_pose(document: &mut Document) -> PoseMotionResult {
    let [am, mj, ej, jb, he] = step25_visible_edges(document);
    solve_and_apply_pose_step(
        document,
        PoseMotionInput {
            // Step 24 looks at the mirrored side, so the book's visible M/V signs are inverted in
            // material-space.  All five exact front-sheet fragments are activated; four remain
            // open at this intermediate sample and are available to the following collapse.
            activations: vec![
                PoseEdgeActivation { edge_id: am, kind: EdgeKind::Valley },
                PoseEdgeActivation { edge_id: mj, kind: EdgeKind::Valley },
                PoseEdgeActivation { edge_id: ej, kind: EdgeKind::Valley },
                PoseEdgeActivation { edge_id: jb, kind: EdgeKind::Valley },
                PoseEdgeActivation { edge_id: he, kind: EdgeKind::Mountain },
            ],
            // The book gives no numeric intermediate angle.  -60 degrees is a deterministic open
            // sample on the branch whose central packet moves left; it is not claimed as a reading
            // from the drawing.
            drivers: vec![PoseAngleTarget { edge_id: jb, target_angle_deg: -60.0 }],
            // The flat state is a singular branch point.  These hints only select the outgoing
            // branch; their solved/final angles return to zero and all exact solved angles are
            // saved instead of these seed values.
            branch_hints: vec![
                PoseAngleTarget { edge_id: am, target_angle_deg: -30.0 },
                PoseAngleTarget { edge_id: mj, target_angle_deg: -30.0 },
                PoseAngleTarget { edge_id: ej, target_angle_deg: -30.0 },
                PoseAngleTarget { edge_id: he, target_angle_deg: 30.0 },
            ],
            note: "悪魔 手順25: 中央packetを左へ送る非平坦途中姿勢".to_string(),
        },
    )
    .expect("手順25の閉じた剛体Poseを解いて保存できる")
}

fn collapse_logical_lines(document: &mut Document, indices: &[usize]) -> FlatState {
    try_collapse_logical_lines(document, indices).expect("既存予備線ネットワークを同時に畳める")
}

fn try_collapse_logical_lines(
    document: &mut Document,
    indices: &[usize],
) -> Result<FlatState, String> {
    try_collapse_logical_lines_for(document, indices, None)
}

fn try_collapse_logical_lines_for(
    document: &mut Document,
    indices: &[usize],
    target_layers: Option<Vec<u32>>,
) -> Result<FlatState, String> {
    let logical = devil_logical_lines();
    let lines = indices
        .iter()
        .map(|&index| {
            existing_line([logical[index].0, logical[index].1])
                .ok_or_else(|| format!("logical[{index}] の既存折り筋が退化している"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (faces, state) = state_of(document);
    let result = collapse_precrease_network(
        &mut document.cp,
        &faces,
        &state,
        &PrecreaseCollapseInput {
            lines,
            target_layers,
        },
    )?;
    if !result.warnings.is_empty() {
        return Err(format!("予備線collapseの警告: {:?}", result.warnings));
    }
    let collapsed = result.state.clone();
    append_step(document, result.step);
    Ok(collapsed)
}

#[test]
fn devil_steps_17_and_18_probe() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    devil_verify::verify_book_step(&document, 17);
    println!(
        "step17 faces={} violations={:?}",
        extract_faces(&document.cp).len(),
        local_violations(&document.cp)
    );
    fold_step18(&mut document);
    devil_verify::verify_book_step(&document, 18);
    fold_step19(&mut document);
    devil_verify::verify_book_step(&document, 19);
    fold_step20(&mut document);
    devil_verify::verify_book_step(&document, 20);
    fold_step21(&mut document);
    devil_verify::verify_book_step(&document, 21);
    fold_step22(&mut document);
    devil_verify::verify_book_step(&document, 22);
    fold_step23(&mut document);
    devil_verify::verify_book_step(&document, 23);
    fold_step24(&mut document);
    let verified_gap = devil_verify::verify_book_step(&document, 24);
    let faces = extract_faces(&document.cp);
    let replayed = replay(&document, document.sequence.len(), 1.0);
    let gap = max_seam_gap(&document.cp, &faces, &replayed.frame);
    println!(
        "step24 seq={} faces={} violations={:?} replay_warnings={:?} frame_warnings={:?} skipped={:?} gap={gap:.12}",
        document.sequence.len(),
        faces.len(),
        local_violations(&document.cp),
        replayed.warnings,
        replayed.frame.warnings,
        replayed.skipped,
    );
    assert!(gap < 1e-6);
    assert!((gap - verified_gap).abs() < 1e-15);
    let checkpoint = devil_fixture::write_checkpoint(&document, 24);
    assert!(checkpoint.is_file(), "手順24のチェックポイントを保存");
    println!("手順24まで通過。max_seam_gap={gap:.3e}");
}

#[test]
fn devil_step25_is_a_closed_non_flat_pose() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    fold_step18(&mut document);
    fold_step19(&mut document);
    fold_step20(&mut document);
    fold_step21(&mut document);
    fold_step22(&mut document);
    fold_step23(&mut document);
    fold_step24(&mut document);

    let result = fold_step25_pose(&mut document);
    let faces = extract_faces(&document.cp);
    assert_eq!(result.step_id, 8);
    assert_eq!(faces.len(), 49);
    assert_eq!(result.frame.faces.len(), faces.len());
    assert!(result.frame.warnings.is_empty());
    assert!(result.max_seam_gap < 1e-6);
    assert!(validate(&document.cp).is_empty());
    assert_eq!(document.sequence.last().unwrap().kind, TechniqueKind::Pose);
    assert!((result.hinge_angles[&17] + 60.0).abs() < 1e-9);

    // The safe material point inside the front packet moves towards smaller book-coordinate u
    // and out of the original plane.  This distinguishes the intended left-moving branch from
    // the other solution at the flat singularity.
    let p_material = glam::DVec3::new(0.8737734478532143, 0.16666666666666669, 0.0);
    let folded = ori3_rigid::propagate(&document.cp, &faces, &result.hinge_angles);
    let (rotation, translation) = folded.transforms[&3];
    let p_pose = rotation * p_material + translation;
    let initial_u = (0.16666666666666655 + 0.8737734478532143) * 0.5;
    let pose_u = (p_pose.x + p_pose.y) * 0.5;
    assert!(pose_u < initial_u - 1e-3, "packetの移動方向: {initial_u} -> {pose_u}");
    assert!(p_pose.z.abs() > 1e-3, "packetは平面外へ持ち上がる: {p_pose:?}");

    let mut expectation = PoseExpectation::from_faces(&faces);
    expectation.min_z_span = 0.1;
    expectation.landmarks = vec![
        PoseLandmarkExpectation::Position {
            material: [0.4999999999999999, 0.4999999999999999],
            expected: [0.4999999999999999, 0.4999999999999999, 0.0],
        },
        PoseLandmarkExpectation::Position {
            material: [0.41421356237309503, 0.41421356237309503],
            expected: [0.41421356237309503, 0.41421356237309503, 0.0],
        },
        PoseLandmarkExpectation::Position {
            material: [0.5857864376269047, 0.5857864376269047],
            expected: [
                0.5428932188134523,
                0.5428932188134523,
                -0.1050664995185061,
            ],
        },
        PoseLandmarkExpectation::Position {
            material: [0.5857864376269049, 0.414213562373095],
            expected: [0.414213562373095, 0.5857864376269049, 0.0],
        },
        PoseLandmarkExpectation::Position {
            material: [0.7071067811865475, 0.29289321881345237],
            expected: [0.29289321881345226, 0.7071067811865475, 0.0],
        },
        PoseLandmarkExpectation::Position {
            material: [1.0, 0.0],
            expected: [0.0, 1.0, 0.0],
        },
    ];
    expectation.depth_probes = vec![PoseDepthExpectation {
        ray: Ray3 {
            origin: [0.15655663803669626, 0.8636634192232443, 1.0],
            direction: [0.0, 0.0, -1.0],
        },
        expected_near_to_far: vec![1, 3],
    }];
    let report = evaluate_pose(&document.cp, &faces, &result.frame, &expectation);
    assert!(report.is_match(), "立体Pose差分: {:?}", report.explanations());

    let replayed = replay(&document, document.sequence.len(), 1.0);
    assert!(replayed.skipped.is_empty());
    assert!(replayed.warnings.is_empty(), "再生警告: {:?}", replayed.warnings);
    assert!(replayed.frame.warnings.is_empty());
    assert_eq!(replayed.frame.faces.len(), faces.len());
    assert!(max_seam_gap(&document.cp, &faces, &replayed.frame) < 1e-6);
    for progress in [0.25, 0.5, 0.75] {
        let intermediate = replay(&document, document.sequence.len(), progress);
        assert!(intermediate.skipped.is_empty());
        assert!(
            intermediate.warnings.is_empty(),
            "Pose再生 t={progress} の警告: {:?}",
            intermediate.warnings
        );
        assert!(intermediate.frame.warnings.is_empty());
        assert_eq!(intermediate.frame.faces.len(), faces.len());
        assert!(max_seam_gap(&document.cp, &faces, &intermediate.frame) < 1e-6);
    }

    let checkpoint = devil_fixture::write_checkpoint(&document, 25);
    assert!(checkpoint.ends_with("devil-025.ori3"));
    assert!(checkpoint.is_file(), "手順25のチェックポイントを保存");
    println!(
        "手順25 Pose通過。faces={} max_seam_gap={:.3e} z_span={:.6} delta_u={:.6}",
        faces.len(),
        report.actual.max_seam_gap,
        report.actual.z_span,
        pose_u - initial_u,
    );
}

#[test]
#[ignore]
fn dump_step23_local_layers() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    fold_step18(&mut document);
    fold_step19(&mut document);
    fold_step20(&mut document);
    fold_step21(&mut document);
    fold_step22(&mut document);
    fold_step23(&mut document);
    let (faces, state) = state_of(&document);
    println!("faces={} order={:?}", faces.len(), state.order);
    for y in [0.95, 0.85, 0.75, 0.65, 0.55, 0.45, 0.35, 0.25] {
        for x in [0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9] {
            let ids = layers_at_point(&document.cp, &faces, &state, [x, y]);
            if !ids.is_empty() {
                println!("p=({x:.2},{y:.2}) n={} ids={ids:?}", ids.len());
            }
        }
    }
    for (rank, id) in state.order.iter().copied().enumerate() {
        let face = faces.iter().find(|face| face.id == id).unwrap();
        let p = ori3_layers::representative_point(&document.cp, face);
        let q = state.placements[&id].apply(DVec2::from(p));
        println!("rank={rank:02} face={id:02} rep=({:.6},{:.6}) poly={:?}", q.x, q.y, mapped_polygon(&document, face, &state));
    }
}

#[test]
#[ignore]
fn dump_step24_logical_line_kinds() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    fold_step18(&mut document);
    fold_step19(&mut document);
    fold_step20(&mut document);
    fold_step21(&mut document);
    fold_step22(&mut document);
    fold_step23(&mut document);
    fold_step24(&mut document);
    for (index, (a, b)) in devil_logical_lines().into_iter().enumerate() {
        let line = [a, b];
        let mut counts = [0_usize; 4];
        for edge in &document.cp.edges {
            if edge_on_line(&document.cp, edge.id, line) {
                let slot = match edge.kind {
                    EdgeKind::Border => 0,
                    EdgeKind::Mountain => 1,
                    EdgeKind::Valley => 2,
                    EdgeKind::Aux => 3,
                };
                counts[slot] += 1;
            }
        }
        println!(
            "logical[{index:02}] {a:?}->{b:?} border={} mountain={} valley={} aux={}",
            counts[0], counts[1], counts[2], counts[3]
        );
    }
}

#[test]
#[ignore]
fn dump_complete_base_collapse() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    fold_step18(&mut document);
    fold_step19(&mut document);
    fold_step20(&mut document);
    fold_step21(&mut document);
    fold_step22(&mut document);
    fold_step23(&mut document);
    fold_step24(&mut document);
    collapse_logical_lines(
        &mut document,
        &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 18, 19, 21],
    );
    let gap = devil_verify::verify_book_step(&document, 29);
    println!(
        "complete base faces={} gap={gap:.3e} violations={:?}",
        extract_faces(&document.cp).len(),
        local_violations(&document.cp)
    );
}

#[test]
#[ignore]
fn probe_step25_single_logical_collapses() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    fold_step18(&mut document);
    fold_step19(&mut document);
    fold_step20(&mut document);
    fold_step21(&mut document);
    fold_step22(&mut document);
    fold_step23(&mut document);
    fold_step24(&mut document);
    let remaining = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 18, 19, 21];
    for index in remaining {
        let mut candidate = document.clone();
        let logical = devil_logical_lines();
        let (faces, state) = state_of(&candidate);
        let result = collapse_precrease_network(
            &mut candidate.cp,
            &faces,
            &state,
            &PrecreaseCollapseInput {
                lines: vec![[logical[index].0, logical[index].1]],
                target_layers: None,
            },
        );
        match result {
            Ok(result) => println!(
                "logical[{index:02}] OK faces={} drivers={} added={}",
                extract_faces(&candidate.cp).len(),
                result.step.drivers.len(),
                result.added_edges.len()
            ),
            Err(error) => println!("logical[{index:02}] ERR {error}"),
        }
    }
}

#[test]
#[ignore]
fn probe_step25_group_collapses() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    fold_step18(&mut document);
    fold_step19(&mut document);
    fold_step20(&mut document);
    fold_step21(&mut document);
    fold_step22(&mut document);
    fold_step23(&mut document);
    fold_step24(&mut document);
    let groups: &[&[usize]] = &[
        &[20, 16, 1, 17],
        &[2, 3],
        &[4, 5, 6, 7],
        &[8, 9, 10, 11],
        &[12, 13, 14, 15],
        &[18, 19, 21],
        &[2, 3, 8, 9, 10, 11, 21],
        &[2, 3, 4, 5, 6, 7],
        &[8, 9, 10, 11, 12, 13, 14, 15],
    ];
    for group in groups {
        let mut candidate = document.clone();
        let logical = devil_logical_lines();
        let lines = group
            .iter()
            .map(|&index| [logical[index].0, logical[index].1])
            .collect();
        let (faces, state) = state_of(&candidate);
        let result = collapse_precrease_network(
            &mut candidate.cp,
            &faces,
            &state,
            &PrecreaseCollapseInput {
                lines,
                target_layers: None,
            },
        );
        match result {
            Ok(result) => println!(
                "group={group:?} OK faces={} drivers={} added={}",
                extract_faces(&candidate.cp).len(),
                result.step.drivers.len(),
                result.added_edges.len()
            ),
            Err(error) => println!("group={group:?} ERR {error}"),
        }
    }
}

#[test]
#[ignore]
fn probe_step25_collapse_sequences() {
    let mut start = precreased_devil();
    fold_step17(&mut start);
    fold_step18(&mut start);
    fold_step19(&mut start);
    fold_step20(&mut start);
    fold_step21(&mut start);
    fold_step22(&mut start);
    fold_step23(&mut start);
    fold_step24(&mut start);
    let primary: [&[usize]; 3] = [&[2, 3], &[8, 9, 10, 11], &[18, 19, 21]];
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let remaining: [&[usize]; 5] = [
        &[4, 5, 6, 7],
        &[12, 13, 14, 15],
        &[4, 5, 6, 7, 12, 13, 14, 15],
        &[4, 6, 12, 14],
        &[5, 7, 13, 15],
    ];
    for order in permutations {
        let mut candidate = start.clone();
        let mut okay = true;
        for slot in order {
            if let Err(error) = try_collapse_logical_lines(&mut candidate, primary[slot]) {
                println!("sequence={order:?} primary {:?} ERR {error}", primary[slot]);
                okay = false;
                break;
            }
        }
        if !okay {
            continue;
        }
        println!(
            "sequence={order:?} primary OK faces={}",
            extract_faces(&candidate.cp).len()
        );
        for group in remaining {
            let mut tail = candidate.clone();
            match try_collapse_logical_lines(&mut tail, group) {
                Ok(_) => println!(
                    "  tail={group:?} OK faces={} aux={}",
                    extract_faces(&tail.cp).len(),
                    tail.cp.edges.iter().filter(|edge| edge.kind == EdgeKind::Aux).count()
                ),
                Err(error) => println!("  tail={group:?} ERR {error}"),
            }
        }
    }
}

#[test]
#[ignore]
fn render_step25_group_candidates() {
    let mut start = precreased_devil();
    fold_step17(&mut start);
    fold_step18(&mut start);
    fold_step19(&mut start);
    fold_step20(&mut start);
    fold_step21(&mut start);
    fold_step22(&mut start);
    fold_step23(&mut start);
    fold_step24(&mut start);
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/devil");
    write_flat_svg(&start, &out.join("probe-step24.svg"));
    for (name, group) in [
        ("w", &[20, 16, 1, 17][..]),
        ("02-03", &[2, 3][..]),
        ("08-11", &[8, 9, 10, 11][..]),
        ("18-19-21", &[18, 19, 21][..]),
    ] {
        let mut candidate = start.clone();
        collapse_logical_lines(&mut candidate, group);
        write_flat_svg(&candidate, &out.join(format!("probe-step25-{name}.svg")));
    }
}

#[test]
#[ignore]
fn dump_step24_face_packets() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    fold_step18(&mut document);
    fold_step19(&mut document);
    fold_step20(&mut document);
    fold_step21(&mut document);
    fold_step22(&mut document);
    fold_step23(&mut document);
    fold_step24(&mut document);
    let (faces, state) = state_of(&document);
    let logical = devil_logical_lines();
    let (front_packet, _) = front_w_packet(&document.cp, &faces, &state);
    println!("front_packet={front_packet:?}");
    println!("order={:?}", state.order);
    for (rank, id) in state.order.iter().copied().enumerate() {
        let face = faces.iter().find(|face| face.id == id).unwrap();
        let p = ori3_layers::representative_point(&document.cp, face);
        let q = state.placements[&id].apply(DVec2::from(p));
        let lines = logical
            .iter()
            .enumerate()
            .filter(|(_, (a, b))| {
                face.edges
                    .iter()
                    .any(|edge| edge_on_line(&document.cp, *edge, [*a, *b]))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        println!(
            "rank={rank:02} face={id:02} mirrored={} material=({:.6},{:.6}) flat=({:.6},{:.6}) lines={lines:?}",
            state.placements[&id].mirrored, p[0], p[1], q.x, q.y,
        );
    }
}

#[test]
#[ignore]
fn dump_step24_aux_packets() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    fold_step18(&mut document);
    fold_step19(&mut document);
    fold_step20(&mut document);
    fold_step21(&mut document);
    fold_step22(&mut document);
    fold_step23(&mut document);
    fold_step24(&mut document);
    let (faces, state) = state_of(&document);
    let logical = devil_logical_lines();
    let rank = state
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let positions = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    for edge in document.cp.edges.iter().filter(|edge| edge.kind == EdgeKind::Aux) {
        let a = positions[&edge.v0];
        let b = positions[&edge.v1];
        let midpoint = (a + b) * 0.5;
        let Some(face) = faces
            .iter()
            .find(|face| ori3_layers::point_in_face(&document.cp, face, [midpoint.x, midpoint.y]))
        else {
            continue;
        };
        let Some(index) = logical
            .iter()
            .position(|(p, q)| edge_on_line(&document.cp, edge.id, [*p, *q]))
        else {
            continue;
        };
        let placement = state.placements[&face.id];
        let qa = placement.apply(a);
        let qb = placement.apply(b);
        let qm = (qa + qb) * 0.5;
        let layers = layers_at_point(&document.cp, &faces, &state, [qm.x, qm.y]);
        let visible = layers.last() == Some(&face.id);
        println!(
            "line={index:02} edge={:03} face={:02} rank={:02} visible={visible} flat=({:.6},{:.6})->({:.6},{:.6})",
            edge.id, face.id, rank[&face.id], qa.x, qa.y, qb.x, qb.y,
        );
    }
}

#[test]
#[ignore]
fn probe_step25_front_w_packet() {
    let mut document = precreased_devil();
    fold_step17(&mut document);
    fold_step18(&mut document);
    fold_step19(&mut document);
    fold_step20(&mut document);
    fold_step21(&mut document);
    fold_step22(&mut document);
    fold_step23(&mut document);
    fold_step24(&mut document);
    let (faces, state) = state_of(&document);
    let (packet, _) = front_w_packet(&document.cp, &faces, &state);
    println!("packet={packet:?}");
    let mut packets = vec![("front", packet)];
    for (name, lower) in [("lower", true), ("upper", false)] {
        packets.push((
            name,
            state
                .order
                .iter()
                .copied()
                .filter(|id| {
                    let face = faces.iter().find(|face| face.id == *id).unwrap();
                    let p = ori3_layers::representative_point(&document.cp, face);
                    (p[0] > p[1]) == lower
                })
                .collect(),
        ));
    }
    for (name, packet) in packets {
        let mut candidate = document.clone();
        match try_collapse_logical_lines_for(
            &mut candidate,
            &[20, 16, 1, 17],
            Some(packet),
        ) {
            Ok(_) => {
                let gap = devil_verify::verify_book_step(&candidate, 25);
                println!(
                    "{name} W OK faces={} gap={gap:.3e}",
                    extract_faces(&candidate.cp).len()
                );
            }
            Err(error) => println!("{name} W ERR {error}"),
        }
    }
}

#[test]
#[ignore]
fn probe_w_then_remaining_sequences() {
    let mut start = precreased_devil();
    fold_step17(&mut start);
    fold_step18(&mut start);
    fold_step19(&mut start);
    fold_step20(&mut start);
    fold_step21(&mut start);
    fold_step22(&mut start);
    fold_step23(&mut start);
    fold_step24(&mut start);
    collapse_logical_lines(&mut start, &[20, 16, 1, 17]);
    let groups: [&[usize]; 3] = [&[2, 3], &[8, 9, 10, 11], &[18, 19, 21]];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut candidate = start.clone();
        let mut result = Ok(state_of(&candidate).1);
        for slot in order {
            result = try_collapse_logical_lines(&mut candidate, groups[slot]);
            if result.is_err() {
                break;
            }
        }
        println!("W then {order:?}: {result:?}");
    }
}

#[test]
#[ignore]
fn probe_step25_front_w_slices() {
    let mut start = precreased_devil();
    fold_step17(&mut start);
    fold_step18(&mut start);
    fold_step19(&mut start);
    fold_step20(&mut start);
    fold_step21(&mut start);
    fold_step22(&mut start);
    fold_step23(&mut start);
    fold_step24(&mut start);
    let (faces, state) = state_of(&start);
    let (packet, _) = front_w_packet(&start.cp, &faces, &state);
    for begin in 0..packet.len() {
        for end in begin + 2..=packet.len() {
            let selected = packet[begin..end].to_vec();
            let mut candidate = start.clone();
            if try_collapse_logical_lines_for(
                &mut candidate,
                &[20, 16, 1, 17],
                Some(selected.clone()),
            )
            .is_ok()
            {
                println!("slice {begin}..{end} OK {selected:?}");
            }
        }
    }
}

#[test]
#[ignore]
fn probe_step25_front_w_subsets() {
    let mut start = precreased_devil();
    fold_step17(&mut start);
    fold_step18(&mut start);
    fold_step19(&mut start);
    fold_step20(&mut start);
    fold_step21(&mut start);
    fold_step22(&mut start);
    fold_step23(&mut start);
    fold_step24(&mut start);
    let (faces, state) = state_of(&start);
    let (packet, _) = front_w_packet(&start.cp, &faces, &state);
    for mask in 0_u16..(1_u16 << packet.len()) {
        if mask.count_ones() < 2 {
            continue;
        }
        let selected = packet
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(_, face)| *face)
            .collect::<Vec<_>>();
        let mut candidate = start.clone();
        if try_collapse_logical_lines_for(
            &mut candidate,
            &[20, 16, 1, 17],
            Some(selected.clone()),
        )
        .is_ok()
        {
            println!("mask={mask:03x} OK {selected:?}");
        }
    }
}
