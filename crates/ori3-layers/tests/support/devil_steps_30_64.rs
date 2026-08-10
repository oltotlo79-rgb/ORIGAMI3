//! 前川淳「悪魔」図30〜64の平坦折り支援。
//!
//! このモジュールは図29の正しい平坦状態から開始する。図から座標を写し取らず、
//! 材料CPに既に存在する辺を現在の面配置へ写した候補だけを折り線として使う。
//! 候補は複製した状態へ実際の技法を適用して検証し、図の矢印方向・局所層数・
//! 対象位置を満たすものを決定的に選ぶ。該当候補が無ければ、無操作の代用品を
//! 記録せず、どの本手順を解決できなかったかを `Err` で返す。

use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::{
    FoldLine, distance_to_line, existing_line, perpendicular_bisector, reflect_across_line,
};
use ori3_layers::{
    CompoundMotionSession, CompoundTechnique, FlatMotionInput, FlatState, FoldDirection,
    FoldThroughResult, LayerTurn, MotionPart, RabbitEarInput, TechniqueInput,
    compose_flat_motion_step, flat_motion, flat_state_at, inside_reverse, layers_at_point,
    layers_from_top_at_point, petal, point_in_face, rabbit_ear, representative_point, squash,
};
use ori3_model::{CreasePattern, Document, EPS, EdgeId, EdgeKind, FaceId, TechniqueKind};

const RESOLVE_EPS: f64 = EPS * 256.0;

/// 材料上の既存辺を、ある現在面を通して畳み平面へ写した線。
#[derive(Clone, Debug)]
pub struct MappedLine {
    pub edge_id: EdgeId,
    pub face_id: FaceId,
    pub material_line: FoldLine,
    pub line: FoldLine,
    pub kind: EdgeKind,
}

/// 現在文書を警告の無い平坦状態として評価する。
pub fn flat_context(document: &Document) -> Result<(Vec<Face>, FlatState), String> {
    let faces = extract_faces(&document.cp);
    if faces.is_empty() {
        return Err("悪魔の現在CPに面がありません".to_string());
    }
    let (state, warnings) = flat_state_at(document, &faces, document.sequence.len())?;
    if !warnings.is_empty() {
        return Err(format!("悪魔の現在平坦状態に警告があります: {warnings:?}"));
    }
    if state.placements.len() != faces.len() || state.order.len() != faces.len() {
        return Err("悪魔の現在平坦状態から面または層が失われています".to_string());
    }
    Ok((faces, state))
}

/// 材料CPの全既存辺を、それを含む現在面の配置で畳み平面へ写す。
///
/// `Aux` は現在の `Face` の境界に含まれないため、辺の材料中点を含む面を探して
/// 配置を得る。山谷辺では両側の面が返ることがあるが、同じ支持線は後段で統合する。
pub fn mapped_existing_lines(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
) -> Result<Vec<MappedLine>, String> {
    let positions = vertex_positions(cp);
    let mut mapped = Vec::new();
    for edge in &cp.edges {
        if edge.kind == EdgeKind::Border {
            continue;
        }
        let a = *positions
            .get(&edge.v0)
            .ok_or_else(|| format!("辺{}の始点がありません", edge.id))?;
        let b = *positions
            .get(&edge.v1)
            .ok_or_else(|| format!("辺{}の終点がありません", edge.id))?;
        if (b - a).length() <= EPS {
            continue;
        }
        let midpoint = (a + b) * 0.5;
        for face in faces
            .iter()
            .filter(|face| point_in_face(cp, face, [midpoint.x, midpoint.y]))
        {
            let placement = state
                .placements
                .get(&face.id)
                .ok_or_else(|| format!("面{}の配置がありません", face.id))?;
            let fa = placement.apply(a);
            let fb = placement.apply(b);
            let line = existing_line([[fa.x, fa.y], [fb.x, fb.y]])
                .ok_or_else(|| format!("辺{}を写した線が退化しました", edge.id))?;
            mapped.push(MappedLine {
                edge_id: edge.id,
                face_id: face.id,
                material_line: [[a.x, a.y], [b.x, b.y]],
                line,
                kind: edge.kind,
            });
        }
    }
    if mapped.is_empty() {
        return Err("現在面へ写せる既存折線がありません".to_string());
    }
    Ok(mapped)
}

/// 局所位置を覆う手前側の層を、指定枚数ちょうど選ぶ。
pub fn local_front_layers(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    point: [f64; 2],
    count: usize,
) -> Result<Vec<FaceId>, String> {
    let local = layers_at_point(cp, faces, state, point);
    if local.len() < count {
        return Err(format!(
            "局所位置には{count}枚必要ですが{}枚しかありません",
            local.len()
        ));
    }
    let selected = layers_from_top_at_point(cp, faces, state, point, 0, count);
    if selected.len() != count {
        return Err(format!(
            "手前{count}枚を選ぶはずが{}枚になりました",
            selected.len()
        ));
    }
    Ok(selected)
}

/// 生成済みの技法結果へ永続IDを付けて文書へ追加する。
/// 呼出側は、結果を作ったCPを先に `document.cp` へ反映しておく。
pub fn append_generated(
    document: &mut Document,
    mut result: FoldThroughResult,
) -> Result<FlatState, String> {
    if !result.warnings.is_empty() {
        return Err(format!(
            "悪魔の折り操作に警告があります: {:?}",
            result.warnings
        ));
    }
    result.step.id = u32::try_from(document.sequence.len())
        .map_err(|_| "悪魔の手順数がu32に収まりません".to_string())?;
    let state = result.state.clone();
    document.sequence.push(result.step);
    Ok(state)
}

/// 折り線を指定軸に対して厳密に鏡映する。
pub fn reflect_fold_line(line: FoldLine, axis: FoldLine) -> Result<FoldLine, String> {
    let a = reflect_across_line(
        DVec2::from(line[0]),
        DVec2::from(axis[0]),
        DVec2::from(axis[1]),
    );
    let b = reflect_across_line(
        DVec2::from(line[1]),
        DVec2::from(axis[0]),
        DVec2::from(axis[1]),
    );
    existing_line([[a.x, a.y], [b.x, b.y]])
        .ok_or_else(|| "鏡映した折り線が退化しました".to_string())
}

#[derive(Clone, Copy, Debug)]
enum Anchor {
    Center,
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug)]
enum Travel {
    Up,
    Down,
    Left,
    Right,
}

impl Travel {
    fn vector(self) -> DVec2 {
        match self {
            Travel::Up => DVec2::Y,
            Travel::Down => -DVec2::Y,
            Travel::Left => -DVec2::X,
            Travel::Right => DVec2::X,
        }
    }

    fn mirrored(self) -> Self {
        match self {
            Travel::Left => Travel::Right,
            Travel::Right => Travel::Left,
            other => other,
        }
    }
}

impl Anchor {
    fn mirrored(self) -> Self {
        match self {
            Anchor::Left => Anchor::Right,
            Anchor::Right => Anchor::Left,
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum LayerRequest {
    Auto,
    One,
    Exact(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewSide {
    Front,
    Back,
}

impl ViewSide {
    fn flip(&mut self) {
        *self = match self {
            ViewSide::Front => ViewSide::Back,
            ViewSide::Back => ViewSide::Front,
        };
    }

    fn fold_direction(self) -> FoldDirection {
        match self {
            ViewSide::Front => FoldDirection::Up,
            ViewSide::Back => FoldDirection::Down,
        }
    }

    fn opens_to_back(self) -> bool {
        matches!(self, ViewSide::Back)
    }
}

#[derive(Clone, Copy, Debug)]
enum PlanOp {
    Simple {
        anchor: Anchor,
        travel: Travel,
        layers: LayerRequest,
    },
    AlignPoints {
        anchor: Anchor,
        travel: Travel,
        layers: LayerRequest,
    },
    Technique {
        technique: CompoundTechnique,
        anchor: Anchor,
        travel: Travel,
        layers: LayerRequest,
    },
    RabbitEar {
        anchor: Anchor,
        travel: Travel,
        layers: LayerRequest,
    },
    Restack {
        anchor: Anchor,
        count: usize,
    },
    FlipView,
}

impl PlanOp {
    fn mirrored(self) -> Self {
        match self {
            PlanOp::Simple {
                anchor,
                travel,
                layers,
            } => PlanOp::Simple {
                anchor: anchor.mirrored(),
                travel: travel.mirrored(),
                layers,
            },
            PlanOp::AlignPoints {
                anchor,
                travel,
                layers,
            } => PlanOp::AlignPoints {
                anchor: anchor.mirrored(),
                travel: travel.mirrored(),
                layers,
            },
            PlanOp::Technique {
                technique,
                anchor,
                travel,
                layers,
            } => PlanOp::Technique {
                technique,
                anchor: anchor.mirrored(),
                travel: travel.mirrored(),
                layers,
            },
            PlanOp::RabbitEar {
                anchor,
                travel,
                layers,
            } => PlanOp::RabbitEar {
                anchor: anchor.mirrored(),
                travel: travel.mirrored(),
                layers,
            },
            PlanOp::Restack { anchor, count } => PlanOp::Restack {
                anchor: anchor.mirrored(),
                count,
            },
            PlanOp::FlipView => PlanOp::FlipView,
        }
    }
}

#[derive(Clone, Debug)]
enum ResolvedOperation {
    Flat(FlatMotionInput),
    Technique(CompoundTechnique, TechniqueInput),
    RabbitEar(RabbitEarInput),
}

#[derive(Clone, Debug)]
struct AnchorSample {
    face_id: FaceId,
    material: [f64; 2],
    folded: [f64; 2],
}

#[derive(Clone, Copy, Debug, Default)]
struct MotionScore {
    desired: f64,
    total: f64,
}

impl MotionScore {
    fn better_than(self, other: Self) -> bool {
        self.desired
            .total_cmp(&other.desired)
            .then(self.total.total_cmp(&other.total))
            .is_gt()
    }

    fn moved(self) -> bool {
        self.total > RESOLVE_EPS
    }
}

fn vertex_positions(cp: &CreasePattern) -> HashMap<u32, DVec2> {
    cp.vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect()
}

fn placed_face_point(
    cp: &CreasePattern,
    face: &Face,
    state: &FlatState,
) -> Result<([f64; 2], DVec2), String> {
    let material = representative_point(cp, face);
    let placement = state
        .placements
        .get(&face.id)
        .ok_or_else(|| format!("面{}の配置がありません", face.id))?;
    Ok((material, placement.apply(DVec2::from(material))))
}

fn select_anchor(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    anchor: Anchor,
) -> Result<AnchorSample, String> {
    let rank = state
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let mut samples = faces
        .iter()
        .map(|face| {
            let (material, folded) = placed_face_point(cp, face, state)?;
            Ok(AnchorSample {
                face_id: face.id,
                material,
                folded: [folded.x, folded.y],
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if samples.is_empty() {
        return Err("対象位置を選べる面がありません".to_string());
    }
    let min = samples.iter().fold(DVec2::splat(f64::INFINITY), |out, s| {
        out.min(DVec2::from(s.folded))
    });
    let max = samples
        .iter()
        .fold(DVec2::splat(f64::NEG_INFINITY), |out, s| {
            out.max(DVec2::from(s.folded))
        });
    let center = (min + max) * 0.5;
    samples.sort_by(|a, b| {
        let pa = DVec2::from(a.folded);
        let pb = DVec2::from(b.folded);
        let primary = match anchor {
            Anchor::Center => (pb - center)
                .length_squared()
                .total_cmp(&(pa - center).length_squared()),
            Anchor::Top => pa.y.total_cmp(&pb.y),
            Anchor::Bottom => pb.y.total_cmp(&pa.y),
            Anchor::Left => pb.x.total_cmp(&pa.x),
            Anchor::Right => pa.x.total_cmp(&pb.x),
        };
        primary.then(rank[&a.face_id].cmp(&rank[&b.face_id]))
    });
    samples
        .pop()
        .ok_or_else(|| "対象位置を選べませんでした".to_string())
}

fn local_packets(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    point: [f64; 2],
    request: LayerRequest,
    minimum: usize,
) -> Result<Vec<Vec<FaceId>>, String> {
    let local = layers_at_point(cp, faces, state, point);
    if local.len() < minimum {
        return Err(format!(
            "対象位置には最低{minimum}枚必要ですが{}枚しかありません",
            local.len()
        ));
    }
    match request {
        LayerRequest::Exact(count) => Ok(vec![local_front_layers(cp, faces, state, point, count)?]),
        LayerRequest::One => Ok(vec![local_front_layers(cp, faces, state, point, 1)?]),
        LayerRequest::Auto => Ok((minimum..=local.len())
            .map(|count| layers_from_top_at_point(cp, faces, state, point, 0, count))
            .collect()),
    }
}

fn lines_for_face(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    face_id: FaceId,
) -> Result<Vec<MappedLine>, String> {
    let mut lines = Vec::<MappedLine>::new();
    for candidate in mapped_existing_lines(cp, faces, state)?
        .into_iter()
        .filter(|line| line.face_id == face_id)
    {
        if lines
            .iter()
            .any(|line| same_supporting_line(line.line, candidate.line))
        {
            continue;
        }
        lines.push(candidate);
    }
    if lines.is_empty() {
        return Err(format!("対象面{face_id}を通る既存折線がありません"));
    }
    lines.sort_by_key(|line| line.edge_id);
    Ok(lines)
}

fn same_supporting_line(first: FoldLine, second: FoldLine) -> bool {
    distance_to_line(first, second[0]) <= RESOLVE_EPS
        && distance_to_line(first, second[1]) <= RESOLVE_EPS
}

fn mapped_material_point(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    material: [f64; 2],
) -> Option<DVec2> {
    state.order.iter().rev().find_map(|face_id| {
        let face = faces.iter().find(|face| face.id == *face_id)?;
        if !point_in_face(cp, face, material) {
            return None;
        }
        Some(state.placements.get(face_id)?.apply(DVec2::from(material)))
    })
}

fn score_motion(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    next_cp: &CreasePattern,
    next_faces: &[Face],
    next_state: &FlatState,
    selected: &[FaceId],
    travel: Travel,
) -> MotionScore {
    let direction = travel.vector();
    let mut score = MotionScore::default();
    for face_id in selected {
        let Some(face) = faces.iter().find(|face| face.id == *face_id) else {
            continue;
        };
        let material = representative_point(cp, face);
        let Some(before) = mapped_material_point(cp, faces, state, material) else {
            continue;
        };
        let Some(after) = mapped_material_point(next_cp, next_faces, next_state, material) else {
            continue;
        };
        let delta = after - before;
        score.desired += delta.dot(direction).max(0.0);
        score.total += delta.length();
    }
    score
}

fn resolve_simple(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    anchor_kind: Anchor,
    travel: Travel,
    layers: LayerRequest,
    view: ViewSide,
) -> Result<FlatMotionInput, String> {
    let anchor = select_anchor(cp, faces, state, anchor_kind)?;
    let packets = local_packets(cp, faces, state, anchor.folded, layers, 1)?;
    let lines = lines_for_face(cp, faces, state, anchor.face_id)?;
    let mut best: Option<(MotionScore, usize, EdgeId, FlatMotionInput)> = None;
    let mut attempted = 0usize;
    for mapped in lines {
        if distance_to_line(mapped.line, anchor.folded) <= RESOLVE_EPS {
            continue;
        }
        for packet in &packets {
            attempted += 1;
            let input = FlatMotionInput {
                parts: vec![MotionPart::fold(
                    packet.clone(),
                    mapped.line,
                    anchor.folded,
                    view.fold_direction(),
                )],
                kind: TechniqueKind::Simple,
            };
            let mut trial_cp = cp.clone();
            let Ok(result) = flat_motion(&mut trial_cp, faces, state, &input) else {
                continue;
            };
            if !result.warnings.is_empty() {
                continue;
            }
            let next_faces = extract_faces(&trial_cp);
            let score = score_motion(
                cp,
                faces,
                state,
                &trial_cp,
                &next_faces,
                &result.state,
                packet,
                travel,
            );
            if !score.moved() || score.desired <= RESOLVE_EPS {
                continue;
            }
            let replace = best.as_ref().is_none_or(|(old, old_len, old_edge, _)| {
                score.better_than(*old)
                    || (score.desired.total_cmp(&old.desired).is_eq()
                        && score.total.total_cmp(&old.total).is_eq()
                        && (packet.len(), mapped.edge_id) < (*old_len, *old_edge))
            });
            if replace {
                best = Some((score, packet.len(), mapped.edge_id, input));
            }
        }
    }
    best.map(|(_, _, _, input)| input).ok_or_else(|| {
        format!("既存折線を使う{travel:?}方向の単純折りを解決できませんでした（{attempted}候補）")
    })
}

fn resolve_point_alignment(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    anchor_kind: Anchor,
    travel: Travel,
    layers: LayerRequest,
    view: ViewSide,
) -> Result<FlatMotionInput, String> {
    let anchor = select_anchor(cp, faces, state, anchor_kind)?;
    let face = faces
        .iter()
        .find(|face| face.id == anchor.face_id)
        .ok_or_else(|| format!("対象面{}がありません", anchor.face_id))?;
    let placement = state
        .placements
        .get(&face.id)
        .ok_or_else(|| format!("対象面{}の配置がありません", face.id))?;
    let positions = vertex_positions(cp);
    let points = face
        .vertices
        .iter()
        .filter_map(|vertex| {
            let material = positions.get(vertex).copied()?;
            let folded = placement.apply(material);
            Some(([material.x, material.y], [folded.x, folded.y]))
        })
        .collect::<Vec<_>>();
    let packets = local_packets(cp, faces, state, anchor.folded, layers, 1)?;
    let direction = travel.vector();
    let mut best: Option<(MotionScore, FlatMotionInput)> = None;
    let mut attempted = 0usize;
    for (moving_material, moving) in &points {
        for (_, target) in &points {
            if (DVec2::from(*target) - DVec2::from(*moving)).dot(direction) <= RESOLVE_EPS {
                continue;
            }
            let Some(line) = perpendicular_bisector(*moving, *target) else {
                continue;
            };
            for packet in &packets {
                attempted += 1;
                let input = FlatMotionInput {
                    parts: vec![MotionPart::fold(
                        packet.clone(),
                        line,
                        *moving,
                        view.fold_direction(),
                    )],
                    kind: TechniqueKind::Simple,
                };
                let mut trial_cp = cp.clone();
                let Ok(result) = flat_motion(&mut trial_cp, faces, state, &input) else {
                    continue;
                };
                if !result.warnings.is_empty() {
                    continue;
                }
                let next_faces = extract_faces(&trial_cp);
                let score = score_motion(
                    cp,
                    faces,
                    state,
                    &trial_cp,
                    &next_faces,
                    &result.state,
                    packet,
                    travel,
                );
                let Some(after) =
                    mapped_material_point(&trial_cp, &next_faces, &result.state, *moving_material)
                else {
                    continue;
                };
                if (after - DVec2::from(*target)).length() > RESOLVE_EPS || !score.moved() {
                    continue;
                }
                if best.as_ref().is_none_or(|(old, _)| score.better_than(*old)) {
                    best = Some((score, input));
                }
            }
        }
    }
    best.map(|(_, input)| input).ok_or_else(|| {
        format!("黒点同士を合わせる{travel:?}方向の折りを解決できませんでした（{attempted}候補）")
    })
}

fn apply_technique_trial(
    technique: CompoundTechnique,
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &TechniqueInput,
) -> Result<FoldThroughResult, String> {
    match technique {
        CompoundTechnique::InsideReverse => inside_reverse(cp, faces, state, input),
        CompoundTechnique::Petal => petal(cp, faces, state, input),
        CompoundTechnique::Squash => squash(cp, faces, state, input),
        other => Err(format!("図30〜64の解決器は{other:?}を使用しません")),
    }
}

fn resolve_technique(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    technique: CompoundTechnique,
    anchor_kind: Anchor,
    travel: Travel,
    layers: LayerRequest,
    view: ViewSide,
) -> Result<TechniqueInput, String> {
    let anchor = select_anchor(cp, faces, state, anchor_kind)?;
    let minimum = usize::from(technique == CompoundTechnique::InsideReverse) + 1;
    let packets = local_packets(cp, faces, state, anchor.folded, layers, minimum)?;
    let lines = lines_for_face(cp, faces, state, anchor.face_id)?;
    let span = model_span(cp, faces, state)?;
    let direction = travel.vector();
    let references = [
        DVec2::from(anchor.folded) + direction * span,
        DVec2::from(anchor.folded) - direction * span,
    ];
    let mut best: Option<(MotionScore, usize, EdgeId, TechniqueInput)> = None;
    let mut attempted = 0usize;
    for mapped in lines {
        for packet in &packets {
            for reference in references {
                if distance_to_line(mapped.line, [reference.x, reference.y]) <= RESOLVE_EPS {
                    continue;
                }
                attempted += 1;
                let input = TechniqueInput {
                    flap: packet.clone(),
                    line: mapped.line,
                    reference_point: [reference.x, reference.y],
                    open_to_back: Some(view.opens_to_back()),
                    polygon: None,
                    center: None,
                };
                let mut trial_cp = cp.clone();
                let Ok(result) =
                    apply_technique_trial(technique, &mut trial_cp, faces, state, &input)
                else {
                    continue;
                };
                if !result.warnings.is_empty() {
                    continue;
                }
                let next_faces = extract_faces(&trial_cp);
                let score = score_motion(
                    cp,
                    faces,
                    state,
                    &trial_cp,
                    &next_faces,
                    &result.state,
                    packet,
                    travel,
                );
                if !score.moved() {
                    continue;
                }
                let replace = best.as_ref().is_none_or(|(old, old_len, old_edge, _)| {
                    score.better_than(*old)
                        || (score.desired.total_cmp(&old.desired).is_eq()
                            && score.total.total_cmp(&old.total).is_eq()
                            && (packet.len(), mapped.edge_id) < (*old_len, *old_edge))
                });
                if replace {
                    best = Some((score, packet.len(), mapped.edge_id, input));
                }
            }
        }
    }
    best.map(|(_, _, _, input)| input).ok_or_else(|| {
        format!("{technique:?}を既存折線から解決できませんでした（{attempted}候補）")
    })
}

fn model_span(cp: &CreasePattern, faces: &[Face], state: &FlatState) -> Result<f64, String> {
    let positions = vertex_positions(cp);
    let mut min = DVec2::splat(f64::INFINITY);
    let mut max = DVec2::splat(f64::NEG_INFINITY);
    let mut seen = false;
    for face in faces {
        let placement = state
            .placements
            .get(&face.id)
            .ok_or_else(|| format!("面{}の配置がありません", face.id))?;
        for vertex in &face.vertices {
            let Some(material) = positions.get(vertex) else {
                continue;
            };
            let folded = placement.apply(*material);
            min = min.min(folded);
            max = max.max(folded);
            seen = true;
        }
    }
    let span = (max - min).length();
    if !seen || !span.is_finite() || span <= EPS {
        return Err("悪魔の現在外形が退化しています".to_string());
    }
    Ok(span)
}

fn line_intersection(first: FoldLine, second: FoldLine) -> Option<DVec2> {
    let a = DVec2::from(first[0]);
    let r = DVec2::from(first[1]) - a;
    let b = DVec2::from(second[0]);
    let s = DVec2::from(second[1]) - b;
    let denominator = r.perp_dot(s);
    if denominator.abs() <= RESOLVE_EPS * r.length() * s.length() {
        return None;
    }
    Some(a + r * (b - a).perp_dot(s) / denominator)
}

fn ray_from(line: FoldLine, vertex: DVec2) -> Option<FoldLine> {
    let a = DVec2::from(line[0]);
    let b = DVec2::from(line[1]);
    if distance_to_line(line, [vertex.x, vertex.y]) > RESOLVE_EPS {
        return None;
    }
    let far = if (a - vertex).length() >= (b - vertex).length() {
        a
    } else {
        b
    };
    ((far - vertex).length() > RESOLVE_EPS).then_some([[vertex.x, vertex.y], [far.x, far.y]])
}

fn resolve_rabbit_ear(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    anchor_kind: Anchor,
    travel: Travel,
    layers: LayerRequest,
    view: ViewSide,
) -> Result<RabbitEarInput, String> {
    let anchor = select_anchor(cp, faces, state, anchor_kind)?;
    let lines = lines_for_face(cp, faces, state, anchor.face_id)?;
    let packets = local_packets(cp, faces, state, anchor.folded, layers, 1)?;
    let mut best: Option<(MotionScore, RabbitEarInput)> = None;
    let mut attempted = 0usize;
    for i in 0..lines.len() {
        for j in (i + 1)..lines.len() {
            let Some(vertex) = line_intersection(lines[i].line, lines[j].line) else {
                continue;
            };
            for k in (j + 1)..lines.len() {
                let Some(r0) = ray_from(lines[i].line, vertex) else {
                    continue;
                };
                let Some(r1) = ray_from(lines[j].line, vertex) else {
                    continue;
                };
                let Some(r2) = ray_from(lines[k].line, vertex) else {
                    continue;
                };
                let rays = [r0, r1, r2];
                for order in [
                    [0usize, 1usize, 2usize],
                    [0, 2, 1],
                    [1, 0, 2],
                    [1, 2, 0],
                    [2, 0, 1],
                    [2, 1, 0],
                ] {
                    for packet in &packets {
                        attempted += 1;
                        let input = RabbitEarInput {
                            creases: [rays[order[0]], rays[order[1]], rays[order[2]]],
                            target_layers: packet.clone(),
                            direction: view.fold_direction(),
                        };
                        let mut trial_cp = cp.clone();
                        let Ok(result) = rabbit_ear(&mut trial_cp, faces, state, &input) else {
                            continue;
                        };
                        if !result.warnings.is_empty() {
                            continue;
                        }
                        let next_faces = extract_faces(&trial_cp);
                        let score = score_motion(
                            cp,
                            faces,
                            state,
                            &trial_cp,
                            &next_faces,
                            &result.state,
                            packet,
                            travel,
                        );
                        if !score.moved() {
                            continue;
                        }
                        if best.as_ref().is_none_or(|(old, _)| score.better_than(*old)) {
                            best = Some((score, input));
                        }
                    }
                }
            }
        }
    }
    best.map(|(_, input)| input).ok_or_else(|| {
        format!("3本の同頂点既存線によるひきよせを解決できませんでした（{attempted}候補）")
    })
}

fn resolve_restack(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    anchor_kind: Anchor,
    count: usize,
    view: ViewSide,
) -> Result<FlatMotionInput, String> {
    let sample = select_anchor(cp, faces, state, anchor_kind)?;
    let local = layers_at_point(cp, faces, state, sample.folded);
    if local.len() <= count {
        return Err(format!(
            "重ね替え位置には移動{count}枚と基準層が必要ですが{}枚しかありません",
            local.len()
        ));
    }
    let selected = layers_from_top_at_point(cp, faces, state, sample.folded, 0, count);
    let anchor_index = local.len() - count - 1;
    let base = local[anchor_index];
    let input = FlatMotionInput {
        parts: vec![MotionPart::restack(
            selected,
            LayerTurn::Beside {
                anchor: base,
                direction: view.fold_direction(),
            },
        )],
        kind: TechniqueKind::Simple,
    };
    let mut trial_cp = cp.clone();
    let result = flat_motion(&mut trial_cp, faces, state, &input)?;
    if !result.warnings.is_empty() {
        return Err(format!(
            "指定層の重ね替えに警告があります: {:?}",
            result.warnings
        ));
    }
    if result.state.order == state.order {
        return Err("指定層の重ね替えで層順序が変わりませんでした".to_string());
    }
    Ok(input)
}

fn resolve_plan(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    plan: PlanOp,
    view: ViewSide,
) -> Result<Option<ResolvedOperation>, String> {
    match plan {
        PlanOp::Simple {
            anchor,
            travel,
            layers,
        } => resolve_simple(cp, faces, state, anchor, travel, layers, view)
            .map(ResolvedOperation::Flat)
            .map(Some),
        PlanOp::AlignPoints {
            anchor,
            travel,
            layers,
        } => resolve_point_alignment(cp, faces, state, anchor, travel, layers, view)
            .map(ResolvedOperation::Flat)
            .map(Some),
        PlanOp::Technique {
            technique,
            anchor,
            travel,
            layers,
        } => resolve_technique(cp, faces, state, technique, anchor, travel, layers, view)
            .map(|input| ResolvedOperation::Technique(technique, input))
            .map(Some),
        PlanOp::RabbitEar {
            anchor,
            travel,
            layers,
        } => resolve_rabbit_ear(cp, faces, state, anchor, travel, layers, view)
            .map(ResolvedOperation::RabbitEar)
            .map(Some),
        PlanOp::Restack { anchor, count } => resolve_restack(cp, faces, state, anchor, count, view)
            .map(ResolvedOperation::Flat)
            .map(Some),
        PlanOp::FlipView => Ok(None),
    }
}

fn execute_resolved(
    document: &mut Document,
    faces: &[Face],
    state: &FlatState,
    operation: ResolvedOperation,
) -> Result<FlatState, String> {
    let mut cp = document.cp.clone();
    let result = match operation {
        ResolvedOperation::Flat(input) => flat_motion(&mut cp, faces, state, &input)?,
        ResolvedOperation::Technique(technique, input) => {
            apply_technique_trial(technique, &mut cp, faces, state, &input)?
        }
        ResolvedOperation::RabbitEar(input) => rabbit_ear(&mut cp, faces, state, &input)?,
    };
    document.cp = cp;
    append_generated(document, result)
}

fn execute_resolved_in_session(
    session: &mut CompoundMotionSession,
    operation: ResolvedOperation,
) -> Result<(), String> {
    match operation {
        ResolvedOperation::Flat(input) => {
            session.apply_flat_motion(&input)?;
        }
        ResolvedOperation::Technique(technique, input) => {
            session.apply_technique(technique, &input)?;
        }
        ResolvedOperation::RabbitEar(input) => {
            session.apply_rabbit_ear(&input)?;
        }
    }
    Ok(())
}

fn apply_one_plan(
    document: &mut Document,
    plan: PlanOp,
    view: &mut ViewSide,
) -> Result<FlatState, String> {
    if matches!(plan, PlanOp::FlipView) {
        view.flip();
        return flat_context(document).map(|(_, state)| state);
    }
    let (faces, state) = flat_context(document)?;
    let operation = resolve_plan(&document.cp, &faces, &state, plan, *view)?
        .ok_or_else(|| "折り操作が解決されませんでした".to_string())?;
    execute_resolved(document, &faces, &state, operation)
}

fn apply_compound_plans(
    document: &mut Document,
    plans: &[PlanOp],
    view: &mut ViewSide,
) -> Result<FlatState, String> {
    let mut planned_view = *view;
    let result = compose_flat_motion_step(document, |session| {
        for &plan in plans {
            if matches!(plan, PlanOp::FlipView) {
                planned_view.flip();
                continue;
            }
            let operation = resolve_plan(
                session.crease_pattern(),
                session.faces(),
                session.state(),
                plan,
                planned_view,
            )?
            .ok_or_else(|| "複合折り操作が解決されませんでした".to_string())?;
            execute_resolved_in_session(session, operation)?;
        }
        Ok(())
    })?;
    *view = planned_view;
    append_generated(document, result)
}

fn plan_for(step: u32) -> Result<PlanOp, String> {
    let plan = match step {
        30 => PlanOp::Simple {
            anchor: Anchor::Center,
            travel: Travel::Left,
            layers: LayerRequest::Auto,
        },
        31 => PlanOp::Simple {
            anchor: Anchor::Bottom,
            travel: Travel::Up,
            layers: LayerRequest::Auto,
        },
        32 => PlanOp::Simple {
            anchor: Anchor::Left,
            travel: Travel::Right,
            layers: LayerRequest::One,
        },
        33 => PlanOp::Simple {
            anchor: Anchor::Right,
            travel: Travel::Left,
            layers: LayerRequest::One,
        },
        34 => PlanOp::Technique {
            technique: CompoundTechnique::Petal,
            anchor: Anchor::Center,
            travel: Travel::Up,
            layers: LayerRequest::Auto,
        },
        35 => PlanOp::Restack {
            anchor: Anchor::Center,
            count: 1,
        },
        36 => PlanOp::Simple {
            anchor: Anchor::Right,
            travel: Travel::Left,
            layers: LayerRequest::Auto,
        },
        37 => PlanOp::Technique {
            technique: CompoundTechnique::Petal,
            anchor: Anchor::Center,
            travel: Travel::Up,
            layers: LayerRequest::Auto,
        },
        38 => PlanOp::Simple {
            anchor: Anchor::Top,
            travel: Travel::Down,
            layers: LayerRequest::Auto,
        },
        40 => PlanOp::Simple {
            anchor: Anchor::Top,
            travel: Travel::Right,
            layers: LayerRequest::Auto,
        },
        41 => PlanOp::Simple {
            anchor: Anchor::Right,
            travel: Travel::Left,
            layers: LayerRequest::One,
        },
        42 => PlanOp::Technique {
            technique: CompoundTechnique::InsideReverse,
            anchor: Anchor::Right,
            travel: Travel::Left,
            layers: LayerRequest::Auto,
        },
        43 => PlanOp::Simple {
            anchor: Anchor::Top,
            travel: Travel::Down,
            layers: LayerRequest::One,
        },
        44 => PlanOp::Simple {
            anchor: Anchor::Right,
            travel: Travel::Left,
            layers: LayerRequest::Auto,
        },
        45 => PlanOp::Technique {
            technique: CompoundTechnique::InsideReverse,
            anchor: Anchor::Top,
            travel: Travel::Right,
            layers: LayerRequest::Auto,
        },
        46 => PlanOp::Technique {
            technique: CompoundTechnique::Squash,
            anchor: Anchor::Top,
            travel: Travel::Down,
            layers: LayerRequest::One,
        },
        47 => PlanOp::Simple {
            anchor: Anchor::Top,
            travel: Travel::Right,
            layers: LayerRequest::Auto,
        },
        48 => PlanOp::RabbitEar {
            anchor: Anchor::Bottom,
            travel: Travel::Up,
            layers: LayerRequest::Auto,
        },
        49 => PlanOp::AlignPoints {
            anchor: Anchor::Top,
            travel: Travel::Up,
            layers: LayerRequest::One,
        },
        50 => PlanOp::Simple {
            anchor: Anchor::Top,
            travel: Travel::Down,
            layers: LayerRequest::One,
        },
        51 => PlanOp::Simple {
            anchor: Anchor::Center,
            travel: Travel::Right,
            layers: LayerRequest::Exact(4),
        },
        53 => PlanOp::Simple {
            anchor: Anchor::Center,
            travel: Travel::Left,
            layers: LayerRequest::Auto,
        },
        54 => PlanOp::FlipView,
        55 => PlanOp::Simple {
            anchor: Anchor::Center,
            travel: Travel::Right,
            layers: LayerRequest::Exact(3),
        },
        56 => PlanOp::Technique {
            technique: CompoundTechnique::InsideReverse,
            anchor: Anchor::Center,
            travel: Travel::Up,
            layers: LayerRequest::Auto,
        },
        57 => PlanOp::Technique {
            technique: CompoundTechnique::InsideReverse,
            anchor: Anchor::Right,
            travel: Travel::Left,
            layers: LayerRequest::Auto,
        },
        58 => PlanOp::Simple {
            anchor: Anchor::Left,
            travel: Travel::Right,
            layers: LayerRequest::Auto,
        },
        59 => PlanOp::Restack {
            anchor: Anchor::Center,
            count: 1,
        },
        60 => PlanOp::Simple {
            anchor: Anchor::Right,
            travel: Travel::Left,
            layers: LayerRequest::Auto,
        },
        61 => PlanOp::Simple {
            anchor: Anchor::Top,
            travel: Travel::Down,
            layers: LayerRequest::Auto,
        },
        62 => PlanOp::FlipView,
        63 => PlanOp::Simple {
            anchor: Anchor::Right,
            travel: Travel::Left,
            layers: LayerRequest::Auto,
        },
        _ => return Err(format!("図{step}には単独planがありません")),
    };
    Ok(plan)
}

fn plans_for_range(first: u32, last: u32, mirror: bool) -> Result<Vec<PlanOp>, String> {
    let mut plans = Vec::new();
    for step in first..=last {
        // 図30の反復（図39）にも、開始時の裏返し記号を含める。
        if step == 30 {
            plans.push(PlanOp::FlipView);
        }
        let plan = plan_for(step)?;
        plans.push(if mirror { plan.mirrored() } else { plan });
        // 図60は折った直後に裏返す。図64の反対側反復にも同じ順序を保つ。
        if step == 60 {
            plans.push(PlanOp::FlipView);
        }
    }
    Ok(plans)
}

/// 図29の正しい平坦状態から図64の操作完了までを実行する。
///
/// `verify` は各book step（視点だけを裏返す54・62を含む）の直後に呼ばれる。
/// 図39・52・64の「同じ」は、内部の各技法を検証してから1つの永続手順へ合成する。
pub fn run_steps_30_64<F>(document: &mut Document, mut verify: F) -> Result<(), String>
where
    F: FnMut(&Document, u32) -> Result<(), String>,
{
    let mut view = ViewSide::Front;
    // 図30の左にある裏返し記号。折線は作らず、以後の山谷方向へ反映する。
    view.flip();
    for step in 30..=64 {
        let result = match step {
            39 => {
                let plans = plans_for_range(30, 38, false)?;
                apply_compound_plans(document, &plans, &mut view)
            }
            52 => {
                let plans = plans_for_range(48, 50, true)?;
                apply_compound_plans(document, &plans, &mut view)
            }
            64 => {
                let plans = plans_for_range(56, 63, true)?;
                apply_compound_plans(document, &plans, &mut view)
            }
            60 => {
                let result = apply_one_plan(document, plan_for(step)?, &mut view)?;
                view.flip();
                Ok(result)
            }
            _ => apply_one_plan(document, plan_for(step)?, &mut view),
        }
        .map_err(|error| format!("悪魔 手順{step}: {error}"))?;
        let _ = result;
        verify(document, step).map_err(|error| format!("悪魔 手順{step}の検証: {error}"))?;
    }
    Ok(())
}
