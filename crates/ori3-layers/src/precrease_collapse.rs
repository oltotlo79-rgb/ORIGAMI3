//! Simultaneous flat collapse of an existing auxiliary crease network.
//!
//! Several precreases meeting at one vertex cannot be represented faithfully as a sequence of
//! unrelated book folds. This module activates named material lines, solves all face placements
//! from exact 180-degree hinge reflections, and records the collapse atomically.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::{Isometry2, collinear_overlap};
use ori3_model::{CreasePattern, EPS, EdgeId, EdgeKind, FaceId, TechniqueKind, VertexId};

use crate::flat_motion::{
    FlatMotionInput, LayerTurn, MotionPart, MotionTransform, run_motion, want_kind,
};
use crate::flat_state::{FlatState, point_in_face, representative_point};
use crate::fold_through::{FoldDirection, FoldThroughResult};

const PLACEMENT_EPS: f64 = 1e-7;

/// 自動collapseの表示用tie-breakに、展開図からは決まらない重なりが残った警告。
/// replayはこのprefixの警告だけを、検証済みの明示layer oracleで置き換えられる。
pub const PRECREASE_ORDER_UNDETERMINED_WARNING_PREFIX: &str =
    "重なり順が展開図だけでは決まらない面の組が";

/// Material-coordinate precrease lines to close in one simultaneous collapse.
#[derive(Clone, Debug)]
pub struct PrecreaseCollapseInput {
    pub lines: Vec<[[f64; 2]; 2]>,
    /// Optional old-face packet. Named hinges must be internal to this packet; auxiliary
    /// segments are activated only inside one of these faces.
    pub target_layers: Option<Vec<FaceId>>,
}

/// 展開図と平坦配置から独立に数えた、紙の重なり順の一般制約。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrecreaseConstraintCounts {
    /// 山谷と、折り目の両側の鏡映状態から直接決まる上下。
    pub adjacent_folds: usize,
    /// 1枚の紙が折り目をまたぐとき、その折り目の両面と同じ側にいる条件。
    pub taco_tortilla: usize,
    /// 同じ側へ開く折り返し2組が交互に並ばない条件。
    pub taco_taco: usize,
    /// 0°でつながる面を1枚の連続した紙として扱う条件。
    pub continuous: usize,
}

/// 候補の下→上順が破った一般制約。
///
/// tuple中の面IDは、それぞれの規則を構成する順であり、鶴など特定作品の部位を
/// 表すものではない。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrecreaseConstraintViolations {
    pub duplicate_faces: Vec<FaceId>,
    pub missing_faces: Vec<FaceId>,
    pub unexpected_faces: Vec<FaceId>,
    /// `(edge, expected_lower, expected_upper)`。
    pub adjacent_folds: Vec<(EdgeId, FaceId, FaceId)>,
    /// `(seam_face_a, seam_face_b, crossing_face)`。
    pub taco_tortilla: Vec<(FaceId, FaceId, FaceId)>,
    /// `(first_a, first_b, second_a, second_b)`。
    pub taco_taco: Vec<(FaceId, FaceId, FaceId, FaceId)>,
    /// 0°のseamをまたぐ面 `(seam_face_a, seam_face_b, crossing_face)`。
    pub continuous_crossings: Vec<(FaceId, FaceId, FaceId)>,
    /// 0°seamどうし `(first_a, first_b, corresponding_a, corresponding_b)`。
    pub continuous: Vec<(FaceId, FaceId, FaceId, FaceId)>,
}

impl PrecreaseConstraintViolations {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.duplicate_faces.is_empty()
            && self.missing_faces.is_empty()
            && self.unexpected_faces.is_empty()
            && self.adjacent_folds.is_empty()
            && self.taco_tortilla.is_empty()
            && self.taco_taco.is_empty()
            && self.continuous_crossings.is_empty()
            && self.continuous.is_empty()
    }
}

/// 保存された層順を採用してよいかを、作品固有情報なしで調べた結果。
///
/// `mandatory_constraints` と `unresolved_overlap_pairs` は候補順を読む前に導く。
/// 従って、Face ID順などのtie-breakを「物理的に証明された上下」へ混ぜない。
/// `unresolved_overlap_pairs` が残っていても、外部から明示された完全順が全制約を
/// 満たすなら、その順は展開図だけでは決まらないtieを解く有効な層oracleである。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrecreaseOrderValidation {
    pub counts: PrecreaseConstraintCounts,
    pub violations: PrecreaseConstraintViolations,
    /// 一般制約の推移閉包。各tupleは `(lower, upper)`。
    pub mandatory_constraints: Vec<(FaceId, FaceId)>,
    /// 正の面積で重なるが、展開図由来の制約だけでは上下が決まらない面対。
    pub unresolved_overlap_pairs: Vec<(FaceId, FaceId)>,
    /// 物理規則の逆向きが既に必然だったため採れなかった制約。
    /// 各tupleは `(requested_lower, requested_upper)`。
    pub discarded_relations: Vec<(FaceId, FaceId)>,
    /// 表示用の完全順を作る探索が失敗した際に返した診断用の面対。
    ///
    /// 探索順やseedで変わり得る値であり、物理規則そのものではない。このため
    /// [`Self::is_valid`] の判定や `discarded_relations` の件数には含めない。
    pub display_resolution_failure: Option<(FaceId, FaceId)>,
}

impl PrecreaseOrderValidation {
    /// 完全面permutationであり、保存順が一般制約をすべて満たすか。
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.violations.is_empty() && self.discarded_relations.is_empty()
    }
}

/// Close an intersecting network of existing auxiliary lines to 180 degrees in one flat step.
///
/// Close a network using only layer-order evidence from the input crease pattern.
///
/// This remains the strict default for proposal generation, replay, and independent audits. A
/// candidate operation cannot replace the target-line M/V that decides whether it is admissible.
pub fn collapse_precrease_network(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &PrecreaseCollapseInput,
) -> Result<FoldThroughResult, String> {
    collapse_precrease_network_impl(cp, faces, state, input, None)
}

/// Close a network as an explicitly requested physical operation.
///
/// A single mirror motion with a strict M/V majority (or an all-Aux line using the ordinary-fold
/// default) may use that operation direction for target-line evidence. Every non-target adjacent
/// rule and every taco/continuity rule remains strict. Proposal generation and replay must use
/// [`collapse_precrease_network`] instead.
pub fn collapse_precrease_network_for_operation(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &PrecreaseCollapseInput,
) -> Result<FoldThroughResult, String> {
    collapse_precrease_network_impl(cp, faces, state, input, Some(()))
}

fn collapse_precrease_network_impl(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &PrecreaseCollapseInput,
    operation_authority: Option<()>,
) -> Result<FoldThroughResult, String> {
    validate_input(input)?;
    let positions = vertex_positions(cp);
    let selected = input
        .target_layers
        .as_ref()
        .map(|layers| layers.iter().copied().collect::<HashSet<_>>());
    if let Some(layers) = &selected {
        if layers.is_empty() {
            return Err("precrease collapse target layer packet is empty".to_string());
        }
        if layers
            .iter()
            .any(|id| !faces.iter().any(|face| face.id == *id))
        {
            return Err("precrease collapse target layer does not exist".to_string());
        }
    }
    let old_owners = edge_owners(faces);
    let mut work = cp.clone();
    let mut activated = Vec::<EdgeId>::new();
    let mut network = HashSet::<EdgeId>::new();
    let mut hit = vec![false; input.lines.len()];
    for edge in &mut work.edges {
        if edge.kind == EdgeKind::Border {
            continue;
        }
        let (Some(a), Some(b)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
            continue;
        };
        let selected_here = selected.as_ref().is_none_or(|selected| {
            if edge.kind == EdgeKind::Aux {
                let midpoint = (*a + *b) * 0.5;
                faces.iter().any(|face| {
                    selected.contains(&face.id) && point_in_face(cp, face, [midpoint.x, midpoint.y])
                })
            } else {
                old_owners.get(&edge.id).is_some_and(|owners| {
                    owners.len() == 2 && owners.iter().all(|owner| selected.contains(owner))
                })
            }
        });
        if !selected_here {
            continue;
        }
        for (index, line) in input.lines.iter().enumerate() {
            if segment_on_line(*a, *b, *line) {
                network.insert(edge.id);
                if edge.kind == EdgeKind::Aux {
                    edge.kind = EdgeKind::Valley;
                    activated.push(edge.id);
                }
                hit[index] = true;
                break;
            }
        }
    }
    if let Some(index) = hit.iter().position(|hit| !hit) {
        return Err(format!(
            "precrease collapse line {} has no auxiliary material segment",
            index + 1
        ));
    }
    if network.is_empty() {
        return Err("precrease collapse did not resolve any material segment".to_string());
    }

    let split_faces = extract_faces(&work);
    let (current, parent_of) = expanded_state(cp, faces, state, &work, &split_faces)?;
    let target_placements =
        reflected_placements(&work, &split_faces, &current, &parent_of, &network, state)?;
    let undetermined = activated.iter().copied().collect::<HashSet<_>>();
    let solved_order = solved_layer_order(
        &work,
        &split_faces,
        &current,
        &target_placements,
        &undetermined,
        operation_authority.map(|()| &network),
    )?;
    let target_order = &solved_order.order;
    settle_kinds_from_order(&mut work, &split_faces, &target_placements, target_order)?;

    let parts = target_order
        .iter()
        .map(|face| MotionPart {
            layers: vec![*face],
            region: Vec::new(),
            transform: MotionTransform::Isometry(
                target_placements[face].compose(&current.placements[face].inverse()),
            ),
            turn: LayerTurn::Outside(FoldDirection::Up),
            reverse_layers: Some(false),
        })
        .collect();
    let outcome = run_motion(
        &work,
        &split_faces,
        &current,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Twist,
        },
    )?;
    if !outcome.result.warnings.is_empty() {
        return Err(format!(
            "precrease collapse produced warnings: {:?}",
            outcome.result.warnings
        ));
    }
    for face in &split_faces {
        if !outcome.result.state.placements[&face.id]
            .approx_eq(&target_placements[&face.id], PLACEMENT_EPS)
        {
            return Err(format!(
                "precrease collapse did not reach solved placement for face {}",
                face.id
            ));
        }
    }
    if outcome.result.state.order.as_slice() != target_order.as_slice() {
        return Err("precrease collapse did not preserve its solved layer order".to_string());
    }
    let mut result = outcome.result;
    if !solved_order.operation_resolved && !solved_order.unresolved_overlap_pairs.is_empty() {
        // `state.order` は表示を続けるための暫定順として残す。一方、保存欄を空にして、
        // 後のreplayがFace ID tie-breakを明示oracleと誤認する経路を断つ。
        result.step.layer_order = None;
        result.warnings.push(format!(
            "{PRECREASE_ORDER_UNDETERMINED_WARNING_PREFIX}{}組あります",
            solved_order.unresolved_overlap_pairs.len()
        ));
    }
    if solved_order.display_resolution_failure.is_some() {
        // 表示用の全順序化に失敗したseed対は物理的な両立不能へ数えない。一方、
        // fallback表示順を保存oracleへ昇格させないことで、診断情報を握りつぶさない。
        result.step.layer_order = None;
    }
    if !solved_order.discarded_relations.is_empty() {
        result.step.layer_order = None;
        result.warnings.push(format!(
            "紙の重なり順の条件が{}組で両立しません",
            solved_order.discarded_relations.len()
        ));
    }
    if solved_order.overlap_analysis_error.is_some() {
        // 多角形の重なりを数えられない場合も、折りと表示は完了させる。ただし未知の
        // 比較を0組と誤記せず、保存authorityを外して再生側へ判定不能を伝える。
        result.step.layer_order = None;
        result
            .warnings
            .push("紙の重なり順を判定できないため推定した順で表示します".to_string());
    }
    result.added_edges = activated;
    result.added_edges.sort_unstable();
    result.added_edges.dedup();
    *cp = outcome.cp;
    Ok(result)
}

/// 明示された完全な層順oracleを検証してから、precrease networkを一括で畳む。
///
/// `layer_order_oracle` はcollapse後の面IDによる下→上の完全順である。通常の
/// [`collapse_precrease_network`] が表示継続のために作るFace ID tie-breakを権威には
/// せず、[`validate_precrease_layer_order`] が展開図から独立に導く一般制約をすべて
/// 満たす場合だけ採用する。不正・不完全なoracleなら、呼出し元の展開図を変更しない。
pub fn collapse_precrease_network_with_layer_order_oracle(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &PrecreaseCollapseInput,
    layer_order_oracle: &[FaceId],
) -> Result<FoldThroughResult, String> {
    let mut collapsed_cp = cp.clone();
    let mut result = collapse_precrease_network(&mut collapsed_cp, faces, state, input)?;
    let collapsed_faces = extract_faces(&collapsed_cp);
    let validation = validate_precrease_layer_order(
        &collapsed_cp,
        &collapsed_faces,
        &result.state.placements,
        layer_order_oracle,
    )?;
    if !validation.is_valid() {
        return Err(format!(
            "precrease layer-order oracle is invalid or incomplete: violations={:?}, discarded_relations={:?}",
            validation.violations, validation.discarded_relations
        ));
    }

    result.state.order = layer_order_oracle.to_vec();
    result.step.layer_order = Some(
        result
            .state
            .to_layer_points(&collapsed_cp, &collapsed_faces),
    );
    if let Some(index) = result
        .warnings
        .iter()
        .position(|warning| warning.starts_with(PRECREASE_ORDER_UNDETERMINED_WARNING_PREFIX))
    {
        result.warnings.remove(index);
    }
    *cp = collapsed_cp;
    Ok(result)
}

fn validate_input(input: &PrecreaseCollapseInput) -> Result<(), String> {
    if input.lines.is_empty() {
        return Err("precrease collapse needs at least one material line".to_string());
    }
    for (index, line) in input.lines.iter().enumerate() {
        let a = DVec2::from(line[0]);
        let b = DVec2::from(line[1]);
        if !a.is_finite() || !b.is_finite() || (b - a).length() <= EPS {
            return Err(format!(
                "precrease collapse line {} is degenerate",
                index + 1
            ));
        }
    }
    Ok(())
}

fn expanded_state(
    old_cp: &CreasePattern,
    old_faces: &[Face],
    old_state: &FlatState,
    work: &CreasePattern,
    split_faces: &[Face],
) -> Result<(FlatState, HashMap<FaceId, FaceId>), String> {
    let old_rank = old_state
        .order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    let mut parent_of = HashMap::new();
    let mut placements = HashMap::new();
    for face in split_faces {
        let point = representative_point(work, face);
        let parent = old_faces
            .iter()
            .find(|candidate| point_in_face(old_cp, candidate, point))
            .ok_or_else(|| format!("new collapse face {} has no parent face", face.id))?;
        let placement = old_state
            .placements
            .get(&parent.id)
            .copied()
            .ok_or_else(|| format!("parent face {} has no flat placement", parent.id))?;
        parent_of.insert(face.id, parent.id);
        placements.insert(face.id, placement);
    }
    let mut order = split_faces.iter().map(|face| face.id).collect::<Vec<_>>();
    order.sort_by_key(|face| (old_rank[&parent_of[face]], *face));
    Ok((FlatState { placements, order }, parent_of))
}

fn reflected_placements(
    cp: &CreasePattern,
    faces: &[Face],
    current: &FlatState,
    parent_of: &HashMap<FaceId, FaceId>,
    network: &HashSet<EdgeId>,
    old_state: &FlatState,
) -> Result<HashMap<FaceId, Isometry2>, String> {
    let positions = vertex_positions(cp);
    let owners = edge_owners(faces);
    let mut adjacency = HashMap::<FaceId, Vec<(FaceId, EdgeId)>>::new();
    for (edge, incident) in owners {
        if incident.len() != 2 {
            continue;
        }
        let crease = cp
            .edges
            .iter()
            .find(|candidate| candidate.id == edge)
            .ok_or_else(|| format!("crease edge {edge} disappeared"))?;
        if !matches!(crease.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        adjacency
            .entry(incident[0])
            .or_default()
            .push((incident[1], edge));
        adjacency
            .entry(incident[1])
            .or_default()
            .push((incident[0], edge));
    }

    let root = faces
        .iter()
        .map(|face| face.id)
        .min()
        .ok_or("precrease collapse has no faces")?;
    let mut placements = HashMap::from([(root, current.placements[&root])]);
    let mut queue = VecDeque::from([root]);
    while let Some(face) = queue.pop_front() {
        let placement = placements[&face];
        for &(neighbor, edge_id) in adjacency.get(&face).map(Vec::as_slice).unwrap_or(&[]) {
            let edge = cp
                .edges
                .iter()
                .find(|candidate| candidate.id == edge_id)
                .ok_or_else(|| format!("crease edge {edge_id} disappeared"))?;
            let old_a = parent_of[&face];
            let old_b = parent_of[&neighbor];
            let closes = network.contains(&edge_id)
                || old_state.placements[&old_a].mirrored != old_state.placements[&old_b].mirrored;
            let candidate = if closes {
                let reflection = Isometry2::reflection(positions[&edge.v0], positions[&edge.v1]);
                placement.compose(&reflection)
            } else {
                placement
            };
            if let Some(existing) = placements.get(&neighbor) {
                if !existing.approx_eq(&candidate, PLACEMENT_EPS) {
                    return Err(format!(
                        "precrease network is inconsistent around edge {edge_id}"
                    ));
                }
            } else {
                placements.insert(neighbor, candidate);
                queue.push_back(neighbor);
            }
        }
    }
    if placements.len() != faces.len() {
        let missing = faces
            .iter()
            .filter(|face| !placements.contains_key(&face.id))
            .map(|face| face.id)
            .collect::<Vec<_>>();
        return Err(format!(
            "precrease collapse face graph is disconnected; missing {missing:?}"
        ));
    }
    Ok(placements)
}

/// 平らに畳んだ紙の重なり順(下→上)を、畳んだ形の幾何から決める。
///
/// # なぜ幾何から決め直すのか
///
/// 以前はここで「元の重なり順を、分かれた面へそのまま細かくしたもの」
/// ([`expanded_state`] が作る順)をそのまま答えにしていた。平らな1枚の紙から
/// 始めると親面は1つしか無いので、その順は**面の番号順**そのものになる。
/// 面の番号は面を取り出した順に振る導出値で、紙とも幾何とも関係が無い。
///
/// 実測(2026-08-17): 提案の展開図を1手畳んだ標本 **45件すべて**(面8〜48)で
/// 重なり順が `[0, 1, 2, …]` そのものになり、うち **33件**では折り返した紙が
/// 折り返さなかった紙の上と下に散らばっていた(=紙が紙をすり抜けていた)。
/// 数え方と結果は `crates/ori3-propose/tests/proposal_stack.rs` にある。
///
/// # 決め方
///
/// 180°に折り切った折り目でつながる2面 a・b は、
///
/// - a の紙のどちら側が上を向いているか(配置の `mirrored`。畳んだ形から出る)
/// - その折り目の山谷(展開図が持っている設計そのもの)
///
/// の2つで、どちらが上かが**一意に決まる**。規則は [`want_kind`] 1か所にあり、
/// ここではそれを逆向きに読む。3D側の同じ規則は `ori3_rigid::derive_layer_order`。
///
/// この動きで初めて折り目へ昇格した補助線(`undetermined`)は、山谷が設計として
/// 与えられていないので上下を決めない。
///
/// 1本の直線で紙を折り返すだけの畳みは、これより強く決まる。動いた紙はまとめて
/// 重なりの外側へ回り、その中の並びはひっくり返る([`simple_fold_order`])。
///
/// 決まらなかった面どうしは、表示を止めないためだけに `previous_order` から暫定順を
/// 引き継ぐ。その順は保存欄へ入れず、未決定の正面積重なり対の件数を警告する。
/// したがって暫定順が、後の再生で物理的に証明済みの順へ昇格することはない。
struct SolvedLayerOrder {
    order: Vec<FaceId>,
    unresolved_overlap_pairs: Vec<(FaceId, FaceId)>,
    discarded_relations: Vec<(FaceId, FaceId)>,
    display_resolution_failure: Option<(FaceId, FaceId)>,
    overlap_analysis_error: Option<String>,
    /// 単一book foldでは、動くpacketを外側へ返す操作自体がtieを一意に解く。
    operation_resolved: bool,
}

fn adjacent_fold_rules(
    cp: &CreasePattern,
    owners: &BTreeMap<EdgeId, Vec<FaceId>>,
    placements: &HashMap<FaceId, Isometry2>,
    undetermined: &HashSet<EdgeId>,
) -> Vec<AdjacentFoldRule> {
    let kinds = cp
        .edges
        .iter()
        .map(|edge| (edge.id, edge.kind))
        .collect::<HashMap<_, _>>();
    let mut rules = Vec::new();
    for (&edge_id, incident) in owners {
        if undetermined.contains(&edge_id) {
            continue;
        }
        let (a, b) = (incident[0], incident[1]);
        let (Some(placement_a), Some(placement_b)) = (placements.get(&a), placements.get(&b))
        else {
            continue;
        };
        if placement_a.mirrored == placement_b.mirrored {
            continue;
        }
        let Some(kind) = kinds.get(&edge_id) else {
            continue;
        };
        if !matches!(kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let (lower, upper) = if want_kind(0, 1, placement_a.mirrored) == *kind {
            (a, b)
        } else {
            (b, a)
        };
        rules.push(AdjacentFoldRule {
            edge: edge_id,
            lower,
            upper,
        });
    }
    rules
}

fn simple_fold_order_for_operation(
    faces: &[Face],
    current: &FlatState,
    placements: &HashMap<FaceId, Isometry2>,
    adjacent: &[AdjacentFoldRule],
    operation_edges: &HashSet<EdgeId>,
) -> Option<SimpleFoldOrder> {
    let operation_senses = adjacent
        .iter()
        .filter(|rule| operation_edges.contains(&rule.edge))
        .map(|rule| (rule.lower, rule.upper))
        .collect::<Vec<_>>();
    simple_fold_order(faces, current, placements, &operation_senses)
}

fn solved_layer_order(
    cp: &CreasePattern,
    faces: &[Face],
    current: &FlatState,
    placements: &HashMap<FaceId, Isometry2>,
    undetermined: &HashSet<EdgeId>,
    operation_edges: Option<&HashSet<EdgeId>>,
) -> Result<SolvedLayerOrder, String> {
    let previous_order = &current.order;
    let owners = edge_owners(faces)
        .into_iter()
        .filter(|(_, owners)| owners.len() == 2)
        .collect::<BTreeMap<_, _>>();
    let adjacent = adjacent_fold_rules(cp, &owners, placements, undetermined);
    let simple = if let Some(operation_edges) = operation_edges {
        simple_fold_order_for_operation(faces, current, placements, &adjacent, operation_edges)
    } else {
        let ordered = adjacent
            .iter()
            .map(|rule| (rule.lower, rule.upper))
            .collect::<Vec<_>>();
        simple_fold_order(faces, current, placements, &ordered)
    };
    let shapes = face_shapes(cp, faces, placements);
    let seams = folded_seams(cp, &owners, &shapes);
    let solution = solve_stack_relation(
        &shapes,
        previous_order,
        &adjacent,
        &seams,
        OverlapAnalysisFailure::ContinueWithWarning,
    )?;

    if let Some(simple) = simple {
        if simple.direction_authoritative
            && let Some(operation_edges) = operation_edges
        {
            // A strict M/V majority (or an all-Aux line with the ordinary-fold default) fixes
            // which outside of the stack receives the moved packet. The operation therefore
            // replaces the pre-operation M/V evidence only on the line being folded. Validate
            // the resulting order against every other adjacent fold and every taco/continuity
            // rule; unlike settle_kinds_from_order, this authority comes from the operation and
            // is established before reading the candidate order.
            let validation = validate_precrease_layer_order_impl(
                cp,
                faces,
                placements,
                &simple.order,
                operation_edges,
            )?;
            if !validation.violations.is_empty() {
                return Err(format!(
                    "single book-fold order violates a non-target precrease constraint: {:?}",
                    validation.violations
                ));
            }
            let unresolved_overlap_pairs = simple_fold_internal_unresolved_pairs(
                &validation.unresolved_overlap_pairs,
                &simple.moved,
                &shapes,
                &seams,
            );
            let operation_resolved = unresolved_overlap_pairs.is_empty();
            return Ok(SolvedLayerOrder {
                order: simple.order,
                unresolved_overlap_pairs,
                discarded_relations: validation.discarded_relations,
                display_resolution_failure: validation.display_resolution_failure,
                overlap_analysis_error: None,
                operation_resolved,
            });
        }
        if operation_edges.is_none() {
            // This is the proposal/replay/default path that predates operation authority. The
            // input CP supplies every M/V sense, and any unresolved or discarded general relation
            // remains a warning so callers reject the candidate. The operation does not get to
            // replace its target-line evidence here.
            let unresolved_overlap_pairs = simple_fold_internal_unresolved_pairs(
                &solution.unresolved_overlap_pairs,
                &simple.moved,
                &shapes,
                &seams,
            );
            let operation_resolved =
                solution.overlap_analysis_error.is_none() && unresolved_overlap_pairs.is_empty();
            return Ok(SolvedLayerOrder {
                order: simple.order,
                unresolved_overlap_pairs,
                discarded_relations: solution.discarded_relations,
                display_resolution_failure: solution.display_resolution_failure,
                overlap_analysis_error: solution.overlap_analysis_error,
                operation_resolved,
            });
        }
        // A non-zero M/V tie has no operation-authoritative direction. In particular,
        // `first_vote` may choose a deterministic display side, but that unvalidated choice must
        // not replace the full input-CP constraints. Use the general solution exactly as the
        // non-book-fold path does; any unresolved pair or discarded relation remains visible to
        // the warning/saved-order guard in `collapse_precrease_network`.
        return Ok(SolvedLayerOrder {
            order: stable_topological_order(previous_order, &solution.display_constraints),
            unresolved_overlap_pairs: solution.unresolved_overlap_pairs,
            discarded_relations: solution.discarded_relations,
            display_resolution_failure: solution.display_resolution_failure,
            overlap_analysis_error: solution.overlap_analysis_error,
            operation_resolved: false,
        });
    }
    Ok(SolvedLayerOrder {
        order: stable_topological_order(previous_order, &solution.display_constraints),
        unresolved_overlap_pairs: solution.unresolved_overlap_pairs,
        discarded_relations: solution.discarded_relations,
        display_resolution_failure: solution.display_resolution_failure,
        overlap_analysis_error: solution.overlap_analysis_error,
        operation_resolved: false,
    })
}

/// 保存された下→上順が、展開図と平坦配置から導く一般制約の有効な拡張か調べる。
///
/// 候補順そのものからmandatory constraintを作らない。まず山谷・鏡映・紙の連続性
/// だけで制約と未決定の正面積重なり対を求め、その後で候補が全規則を満たすかを
/// 読み合わせる。このため、保存されたFace ID tie-breakによる自己認証にはならない。
pub fn validate_precrease_layer_order(
    cp: &CreasePattern,
    faces: &[Face],
    placements: &HashMap<FaceId, Isometry2>,
    candidate_order: &[FaceId],
) -> Result<PrecreaseOrderValidation, String> {
    validate_precrease_layer_order_impl(cp, faces, placements, candidate_order, &HashSet::new())
}

/// Product-internal operation-aware validation core.
///
/// `operation_edges` are not dropped from the physical model: the book-fold operation replaces
/// their pre-operation M/V with its authoritative outside-packet direction. All other adjacent
/// M/V constraints and every taco/continuity rule remain in the independent validation. Product
/// collapse calls this with operation edges only after [`simple_fold_order_for_operation`] proves
/// a strict-majority/all-Aux direction. [`validate_precrease_layer_order`] always passes an empty
/// set, so the saved-order audit remains strict and cannot excuse an input-CP violation.
fn validate_precrease_layer_order_impl(
    cp: &CreasePattern,
    faces: &[Face],
    placements: &HashMap<FaceId, Isometry2>,
    candidate_order: &[FaceId],
    operation_edges: &HashSet<EdgeId>,
) -> Result<PrecreaseOrderValidation, String> {
    let owners = edge_owners(faces)
        .into_iter()
        .filter(|(_, owners)| owners.len() == 2)
        .collect::<BTreeMap<_, _>>();
    let adjacent = adjacent_fold_rules(cp, &owners, placements, operation_edges);
    let shapes = face_shapes(cp, faces, placements);
    if shapes.len() != faces.len() {
        let present = shapes.iter().map(|shape| shape.id).collect::<BTreeSet<_>>();
        let missing = faces
            .iter()
            .map(|face| face.id)
            .filter(|face| !present.contains(face))
            .collect::<Vec<_>>();
        return Err(format!(
            "precrease layer-order validation has no flat placement for faces {missing:?}"
        ));
    }
    let seams = folded_seams(cp, &owners, &shapes);
    let rules = stack_rules(&shapes, &seams);
    let solution = solve_stack_relation(
        &shapes,
        candidate_order,
        &adjacent,
        &seams,
        OverlapAnalysisFailure::Reject,
    )?;

    let expected = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    let mut occurrences = BTreeMap::<FaceId, usize>::new();
    for &face in candidate_order {
        *occurrences.entry(face).or_default() += 1;
    }
    let actual = occurrences.keys().copied().collect::<BTreeSet<_>>();
    let mut violations = PrecreaseConstraintViolations {
        duplicate_faces: occurrences
            .iter()
            .filter_map(|(&face, &count)| (count > 1).then_some(face))
            .collect(),
        missing_faces: expected.difference(&actual).copied().collect(),
        unexpected_faces: actual.difference(&expected).copied().collect(),
        ..PrecreaseConstraintViolations::default()
    };
    let rank = candidate_order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();

    for rule in &adjacent {
        if let (Some(&lower), Some(&upper)) = (rank.get(&rule.lower), rank.get(&rule.upper))
            && lower >= upper
        {
            violations
                .adjacent_folds
                .push((rule.edge, rule.lower, rule.upper));
        }
    }
    for (rule_index, &(a, b, other)) in rules.crossings.iter().enumerate() {
        let ids = (shapes[a].id, shapes[b].id, shapes[other].id);
        if let (Some(&a), Some(&b), Some(&other)) =
            (rank.get(&ids.0), rank.get(&ids.1), rank.get(&ids.2))
        {
            let outside = (other < a && other < b) || (other > a && other > b);
            if !outside {
                if rules.crossing_folded[rule_index] {
                    violations.taco_tortilla.push(ids);
                } else {
                    violations.continuous_crossings.push(ids);
                }
            }
        }
    }
    for &(a, b, c, d) in &rules.nests {
        let ids = (shapes[a].id, shapes[b].id, shapes[c].id, shapes[d].id);
        if let (Some(&a), Some(&b), Some(&c), Some(&d)) = (
            rank.get(&ids.0),
            rank.get(&ids.1),
            rank.get(&ids.2),
            rank.get(&ids.3),
        ) {
            let between = |middle: usize, first: usize, second: usize| {
                (first < middle && middle < second) || (second < middle && middle < first)
            };
            if between(c, a, b) != between(d, a, b) || between(a, c, d) != between(b, c, d) {
                violations.taco_taco.push(ids);
            }
        }
    }
    for &(first, second, near, far) in &rules.parallels {
        let ids = (
            shapes[first].id,
            shapes[second].id,
            shapes[near].id,
            shapes[far].id,
        );
        if let (Some(&first), Some(&second), Some(&near), Some(&far)) = (
            rank.get(&ids.0),
            rank.get(&ids.1),
            rank.get(&ids.2),
            rank.get(&ids.3),
        ) && (first < near) != (second < far)
        {
            violations.continuous.push(ids);
        }
    }

    Ok(PrecreaseOrderValidation {
        counts: solution.counts,
        violations,
        mandatory_constraints: solution.mandatory_constraints,
        unresolved_overlap_pairs: solution.unresolved_overlap_pairs,
        discarded_relations: solution.discarded_relations,
        display_resolution_failure: solution.display_resolution_failure,
    })
}

/// 明示された部分的な上下制約を、展開図から独立に導いた一般制約へ加えて完全順へ延長する。
///
/// `required_constraints` の各要素は `(lower, upper)`。候補順を物理制約へ混ぜず、まず
/// 隣接M/V・taco-tortilla・taco-taco・0°連続面の関係を作る。その後で明示制約を加え、
/// 条件付き規則を再伝播する。まだ分岐が残る場合だけ `preferred_order` の向きを先に試すが、
/// 矛盾した枝はcloneごと捨てて逆向きを試す。最後に [`validate_precrease_layer_order`] で
/// 全規則を独立に再検証するため、優先順自身による自己認証にはならない。
pub fn resolve_precrease_layer_order_with_constraints(
    cp: &CreasePattern,
    faces: &[Face],
    placements: &HashMap<FaceId, Isometry2>,
    preferred_order: &[FaceId],
    required_constraints: &[(FaceId, FaceId)],
) -> Result<Vec<FaceId>, String> {
    let expected = faces.iter().map(|face| face.id).collect::<BTreeSet<_>>();
    let preferred = preferred_order.iter().copied().collect::<BTreeSet<_>>();
    if preferred_order.len() != faces.len() || preferred != expected {
        return Err("precrease preferred layer order is not a complete face permutation".into());
    }

    let owners = edge_owners(faces)
        .into_iter()
        .filter(|(_, owners)| owners.len() == 2)
        .collect::<BTreeMap<_, _>>();
    let adjacent = adjacent_fold_rules(cp, &owners, placements, &HashSet::new());
    let shapes = face_shapes(cp, faces, placements);
    if shapes.len() != faces.len() {
        let present = shapes.iter().map(|shape| shape.id).collect::<BTreeSet<_>>();
        let missing = faces
            .iter()
            .map(|face| face.id)
            .filter(|face| !present.contains(face))
            .collect::<Vec<_>>();
        return Err(format!(
            "precrease layer-order resolution has no flat placement for faces {missing:?}"
        ));
    }
    let seams = folded_seams(cp, &owners, &shapes);
    let rules = stack_rules(&shapes, &seams);
    let index = shapes
        .iter()
        .enumerate()
        .map(|(index, shape)| (shape.id, index))
        .collect::<HashMap<_, _>>();
    let mut relation = StackRelation::new(shapes.len());
    for rule in &adjacent {
        let (Some(&lower), Some(&upper)) = (index.get(&rule.lower), index.get(&rule.upper)) else {
            continue;
        };
        relation.add(lower, upper);
    }
    relation.propagate(&rules.crossings, &rules.nests, &rules.parallels);
    if !relation.discarded.is_empty() {
        return Err("precrease general layer constraints contradict each other".into());
    }

    let mut involved = BTreeSet::new();
    for &(first, second, other) in &rules.crossings {
        involved.extend([first, second, other]);
    }
    for &(a, b, c, d) in rules.nests.iter().chain(&rules.parallels) {
        involved.extend([a, b, c, d]);
    }
    for &(lower_id, upper_id) in required_constraints {
        if lower_id == upper_id {
            return Err(format!(
                "precrease required layer constraint contains self pair {lower_id}"
            ));
        }
        let Some(&lower) = index.get(&lower_id) else {
            return Err(format!(
                "precrease required layer constraint has unknown face {lower_id}"
            ));
        };
        let Some(&upper) = index.get(&upper_id) else {
            return Err(format!(
                "precrease required layer constraint has unknown face {upper_id}"
            ));
        };
        involved.extend([lower, upper]);
        if relation.is_below(upper, lower) {
            return Err(format!(
                "precrease required layer constraint {lower_id} < {upper_id} contradicts the general constraints"
            ));
        }
        let discarded_before = relation.discarded.len();
        relation.add(lower, upper);
        relation.propagate(&rules.crossings, &rules.nests, &rules.parallels);
        if relation.discarded.len() != discarded_before {
            return Err(format!(
                "precrease required layer constraint {lower_id} < {upper_id} makes the general constraints inconsistent"
            ));
        }
    }
    if !relation_respects_resolved_stack_rules(&relation, &rules) {
        return Err("precrease required layer constraints violate a stack rule".into());
    }

    let preferred_rank = preferred_order
        .iter()
        .enumerate()
        .map(|(rank, face)| (*face, rank))
        .collect::<HashMap<_, _>>();
    let mut ordered = involved.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|&face| (preferred_rank[&shapes[face].id], shapes[face].id));
    PrecreaseOrderSearch {
        cp,
        faces,
        placements,
        shapes: &shapes,
        rules: &rules,
        preferred_order,
        involved: &ordered,
        required_constraints,
    }
    .search(relation)?
    .ok_or_else(|| {
        "precrease required layer constraints have no generally valid total order".into()
    })
}

/// 1本の直線で紙を折り返すだけの畳みなら、重なり順は幾何から一意に組み立てられる。
///
/// 動いた紙はまとめて重なりの**外側**(上または下)へ回り、その中の並びは
/// ひっくり返る。これは普通の折り操作が使う規則
/// ([`crate::flat_motion`] の [`LayerTurn::Outside`])とまったく同じで、
/// 同じ展開図を両方の道で通したときに重なり順が一致する根拠になる。
///
/// 上下どちらへ回るかは、閉じる折り目の山谷が決める。山谷がまだ無い補助線だけを
/// 閉じる場合は、普通の折り操作の既定と同じく手前(上)へ回す。
///
/// 次のどれかに当てはまるときは、1回の折り返しでは説明できない畳みなので
/// [`None`] を返し、一般の解き方([`solve_stack_relation`])へ渡す。
///
/// - 動いた紙が2通り以上の動き方に分かれている(直線を2本以上同時に閉じた場合)
/// - 動きが裏返しでない(鏡映が偶数回=回転になっている)
///
/// # 閉じる折り目どうしで山谷が食い違っているとき
///
/// 展開図が持っている山谷は**折り上がった作品**の山谷である。平らな紙から
/// その直線を最初に折るという動き方は、作品の折り順とは限らない。そのため
/// 1本の直線の上で山と谷が混ざっていることがあり、そのときは「動いた紙を上へ」と
/// 「下へ」の両方の言い分が出る。
///
/// 平らな1枚の紙をその直線で折る動きは、上へ回すか下へ回すかのどちらかしかない。
/// 山谷の**厳密な多数**がある場合は、その向きを操作の根拠として採り、少数側の山谷を
/// 折った結果に合わせて付け直す([`settle_kinds_from_order`])。全て補助線なら普通の折り
/// 操作の既定で上へ回す。一方、非0票の同数は向きを決める根拠にならない。最小edgeの票は
/// 表示用の決定性にだけ使い、入力CPの一般制約を除外せず警告・拒否を判断する。
///
/// 実測(2026-08-17、出っぱり4/6/8/12本・標本45件): 食い違いを理由にここで
/// 組み立てをやめ、一般の解き方へ渡していたときは、**7件**で折り返した紙が上と下に
/// 散らばった(=紙が紙をすり抜けた)。厳密な多数を操作の向きとしてまとめたら **0件**に
/// なった。同数を表示tie-breakだけで物理的に有効と扱わない点は2026-08-28に固定した。
struct SimpleFoldOrder {
    order: Vec<FaceId>,
    moved: HashSet<FaceId>,
    /// A strict M/V majority, or the ordinary-fold default when every target edge is Aux.
    /// A non-zero tie may still choose a deterministic display side, but is not physical authority.
    direction_authoritative: bool,
}

fn simple_fold_order(
    faces: &[Face],
    current: &FlatState,
    placements: &HashMap<FaceId, Isometry2>,
    senses: &[(FaceId, FaceId)],
) -> Option<SimpleFoldOrder> {
    let identity = Isometry2::identity();
    let mut moved = HashSet::<FaceId>::new();
    let mut motion: Option<Isometry2> = None;
    for face in faces {
        let before = *current.placements.get(&face.id)?;
        let after = *placements.get(&face.id)?;
        let step = after.compose(&before.inverse());
        if step.approx_eq(&identity, PLACEMENT_EPS) {
            continue;
        }
        if !step.mirrored {
            return None;
        }
        match motion {
            None => motion = Some(step),
            Some(known) if known.approx_eq(&step, PLACEMENT_EPS) => {}
            Some(_) => return None,
        }
        moved.insert(face.id);
    }
    if moved.is_empty() || moved.len() >= faces.len() {
        return None;
    }

    let (mut votes_above, mut votes_below) = (0usize, 0usize);
    let mut first_vote: Option<bool> = None;
    for &(below, above) in senses {
        let (below_moved, above_moved) = (moved.contains(&below), moved.contains(&above));
        if below_moved == above_moved {
            continue; // この折り目は折り線をまたいでいないので、向きを決めない
        }
        if above_moved {
            votes_above += 1;
        } else {
            votes_below += 1;
        }
        first_vote.get_or_insert(above_moved);
    }
    // 山谷が1本も決まっていない(補助線だけを閉じる)ときは、普通の折り操作の
    // 既定と同じく手前(上)へ回す。
    let (moved_above, direction_authoritative) = match votes_above.cmp(&votes_below) {
        std::cmp::Ordering::Greater => (true, true),
        std::cmp::Ordering::Less => (false, true),
        std::cmp::Ordering::Equal => (
            first_vote.unwrap_or(true),
            votes_above == 0 && votes_below == 0,
        ),
    };

    let stayed = current
        .order
        .iter()
        .copied()
        .filter(|face| !moved.contains(face))
        .collect::<Vec<_>>();
    let mut block = current
        .order
        .iter()
        .copied()
        .filter(|face| moved.contains(face))
        .collect::<Vec<_>>();
    block.reverse();
    let order = if moved_above {
        [stayed, block].concat()
    } else {
        [block, stayed].concat()
    };
    Some(SimpleFoldOrder {
        order,
        moved,
        direction_authoritative,
    })
}

fn simple_fold_internal_unresolved_pairs(
    unresolved_overlap_pairs: &[(FaceId, FaceId)],
    moved: &HashSet<FaceId>,
    shapes: &[FaceShape],
    seams: &[Seam],
) -> Vec<(FaceId, FaceId)> {
    let components = flat_seam_components(shapes, seams);
    unresolved_overlap_pairs
        .iter()
        .copied()
        .filter(|&(left, right)| {
            moved.contains(&left) == moved.contains(&right)
                && components.get(&left) != components.get(&right)
        })
        .collect()
}

/// 0° seamでつながる面は、層比較では分割面の集合でなく1枚の連続した紙として扱う。
fn flat_seam_components(shapes: &[FaceShape], seams: &[Seam]) -> HashMap<FaceId, usize> {
    let mut adjacency = shapes
        .iter()
        .map(|shape| (shape.id, Vec::<FaceId>::new()))
        .collect::<HashMap<_, _>>();
    for seam in seams.iter().filter(|seam| !seam.folded) {
        if adjacency.contains_key(&seam.a) && adjacency.contains_key(&seam.b) {
            adjacency
                .get_mut(&seam.a)
                .expect("known seam face")
                .push(seam.b);
            adjacency
                .get_mut(&seam.b)
                .expect("known seam face")
                .push(seam.a);
        }
    }
    let mut components = HashMap::new();
    let mut next_component = 0usize;
    for shape in shapes {
        if components.contains_key(&shape.id) {
            continue;
        }
        let mut pending = VecDeque::from([shape.id]);
        components.insert(shape.id, next_component);
        while let Some(face) = pending.pop_front() {
            for &neighbor in &adjacency[&face] {
                if let std::collections::hash_map::Entry::Vacant(entry) = components.entry(neighbor)
                {
                    entry.insert(next_component);
                    pending.push_back(neighbor);
                }
            }
        }
        next_component += 1;
    }
    components
}

/// 畳んだ平面での面の形。上下の判定に要る量だけを面ごとに1度だけ作る。
struct FaceShape {
    id: FaceId,
    /// 展開図座標の境界(点が面の中にあるかを、この座標系で見る)。
    polygon: Vec<DVec2>,
    placement: Isometry2,
    /// 畳んだ平面での外接四角形(高い枝刈り用)。
    minimum: DVec2,
    maximum: DVec2,
}

/// 面が線分をまたいでいるとみなすとき、線分の両側へ取る距離。
///
/// 紙は長辺1に正規化してある。展開図の組み立てが「同じ点」とみなす距離
/// (`ori3_model::EPS` = 1e-9)より**3桁大きく**、提案の展開図で実測した
/// いちばん近い頂点どうしの間隔(`crates/ori3-propose/tests/support/mod.rs` の
/// 実測 1.29e-3)より**3桁小さい**。境界に沿っているだけの面をまたぎと
/// 数えず、本当にまたいでいる面を取りこぼさない幅として、この間に取った。
const CROSSING_OFFSET: f64 = 1.0e-6;

/// 線分の上で両側を確かめる点の数。端は面の角に当たりやすいので内側だけを見る。
const CROSSING_SAMPLES: usize = 9;

fn face_shapes(
    cp: &CreasePattern,
    faces: &[Face],
    placements: &HashMap<FaceId, Isometry2>,
) -> Vec<FaceShape> {
    let positions = crate::flat_state::vertex_positions(cp);
    faces
        .iter()
        .filter_map(|face| {
            let placement = *placements.get(&face.id)?;
            let polygon = crate::flat_state::face_polygon(&positions, face);
            let (minimum, maximum) = polygon.iter().fold(
                (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
                |(minimum, maximum), &point| {
                    let folded = placement.apply(point);
                    (minimum.min(folded), maximum.max(folded))
                },
            );
            Some(FaceShape {
                id: face.id,
                polygon,
                placement,
                minimum,
                maximum,
            })
        })
        .collect()
}

/// 畳んだ平面での、2面が縁でつながっている線分(折り目の像)。
struct Seam {
    a: FaceId,
    b: FaceId,
    start: DVec2,
    end: DVec2,
    /// 180°に折り切っているか。平らにつながっているだけなら偽。
    folded: bool,
    /// 面 `a` が伸びている側。`end - start` から見た符号で表す。
    /// 180°に折れていれば面 `b` も同じ側、平らなら面 `b` は反対側にある。
    side: Option<f64>,
}

/// 2面が縁でつながっている折り目を、畳んだ平面の線分として集める。
///
/// 裂けている(2面が同じ場所へ写らない)辺は、この後の判定に使えないので外す。
fn folded_seams(
    cp: &CreasePattern,
    owners: &BTreeMap<EdgeId, Vec<FaceId>>,
    shapes: &[FaceShape],
) -> Vec<Seam> {
    let positions = crate::flat_state::vertex_positions(cp);
    let by_id = shapes
        .iter()
        .map(|shape| (shape.id, shape))
        .collect::<HashMap<_, _>>();
    let mut out = Vec::new();
    for (&edge_id, incident) in owners {
        let Some(edge) = cp.edges.iter().find(|candidate| candidate.id == edge_id) else {
            continue;
        };
        let (Some(&v0), Some(&v1)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
            continue;
        };
        let (a, b) = (incident[0], incident[1]);
        let (Some(shape_a), Some(shape_b)) = (by_id.get(&a), by_id.get(&b)) else {
            continue;
        };
        let (start, end) = (shape_a.placement.apply(v0), shape_a.placement.apply(v1));
        if (shape_b.placement.apply(v0) - start).length() > EPS
            || (shape_b.placement.apply(v1) - end).length() > EPS
            || (end - start).length() <= EPS
        {
            continue;
        }
        out.push(Seam {
            a,
            b,
            start,
            end,
            folded: shape_a.placement.mirrored != shape_b.placement.mirrored,
            side: seam_side(shape_a, start, end),
        });
    }
    out
}

/// 折り目から見て、その面が伸びている側。
///
/// 折り目の真ん中から法線の向きへわずかに寄った点が面の中にあるかで決める。
fn seam_side(shape: &FaceShape, start: DVec2, end: DVec2) -> Option<f64> {
    let inverse = shape.placement.inverse();
    let (local_start, local_end) = (inverse.apply(start), inverse.apply(end));
    let direction = local_end - local_start;
    if direction.length() <= EPS {
        return None;
    }
    let middle = (local_start + local_end) * 0.5;
    let normal = direction.normalize().perp() * CROSSING_OFFSET;
    for offset in [normal, -normal] {
        if crate::flat_state::point_in_polygon(&shape.polygon, middle + offset) {
            let folded = shape.placement.apply(middle + offset) - shape.placement.apply(middle);
            return Some((end - start).perp_dot(folded).signum());
        }
    }
    None
}

/// 畳んだ平面の線分を、面が**またいでいる**か。
///
/// 線分の上の点をいくつか取り、その両側にずらした点が2つとも面の中にあれば
/// またいでいる。面の縁が線分に沿っているだけの場合は、片側が外に出るので
/// またぎにならない。
fn crosses_segment(shape: &FaceShape, start: DVec2, end: DVec2) -> bool {
    let inverse = shape.placement.inverse();
    let (local_start, local_end) = (inverse.apply(start), inverse.apply(end));
    let direction = local_end - local_start;
    if direction.length() <= EPS {
        return false;
    }
    let normal = direction.normalize().perp() * CROSSING_OFFSET;
    (1..=CROSSING_SAMPLES).any(|step| {
        let point = local_start + direction * (step as f64 / (CROSSING_SAMPLES + 1) as f64);
        crate::flat_state::point_in_polygon(&shape.polygon, point + normal)
            && crate::flat_state::point_in_polygon(&shape.polygon, point - normal)
    })
}

/// 折り目が決める上下と、紙に厚みが無いことから決まる上下を、まとめて解く。
///
/// 使う条件は2つだけで、どちらも畳んだ形の幾何から出る。
///
/// 1. **またぎ**: 2面が縁でつながっている線を**またいでいる**紙は、そのつなぎ目の
///    内側へ潜り込めない。折り目が平らなら1枚の続きの紙をまたぐということ、
///    180°に折れているならその折り返しの内側へ入れないということで、どちらも
///    「片方の上なら、もう片方の上」を意味する。
/// 2. **折り返しどうし**: 同じ場所に重なる2本の折り目が同じ側へ開いているとき、
///    その2組の紙は互いに食い込めない。下から順に並べたとき2組は入れ子か離れて
///    いるかのどちらかで、交互には並べない。
/// 3. **同じ線で切れている紙どうし**: 1枚の続きの紙が2枚、同じ線の上で
///    切り分けられて重なっているとき、線の片側で上にある紙は反対側でも上にある。
///    線をまたいで上下が入れ替わるには、紙が紙をすり抜けるしかない。
///
/// 物理条件だけで決まる関係を先に確定し、その後で表示継続用のtie-breakを別に作る。
/// 両者を分けることで、Face IDや以前の表示順を物理的な証明として保存しない。
#[derive(Clone, Copy)]
struct AdjacentFoldRule {
    edge: EdgeId,
    lower: FaceId,
    upper: FaceId,
}

#[derive(Default)]
struct StackRules {
    crossings: Vec<(usize, usize, usize)>,
    crossing_folded: Vec<bool>,
    nests: Vec<(usize, usize, usize, usize)>,
    parallels: Vec<(usize, usize, usize, usize)>,
}

struct StackSolution {
    display_constraints: BTreeSet<(FaceId, FaceId)>,
    mandatory_constraints: Vec<(FaceId, FaceId)>,
    unresolved_overlap_pairs: Vec<(FaceId, FaceId)>,
    discarded_relations: Vec<(FaceId, FaceId)>,
    display_resolution_failure: Option<(FaceId, FaceId)>,
    overlap_analysis_error: Option<String>,
    counts: PrecreaseConstraintCounts,
}

#[derive(Clone, Copy)]
enum OverlapAnalysisFailure {
    Reject,
    ContinueWithWarning,
}

fn stack_rules(shapes: &[FaceShape], seams: &[Seam]) -> StackRules {
    let index = shapes
        .iter()
        .enumerate()
        .map(|(index, shape)| (shape.id, index))
        .collect::<HashMap<_, _>>();
    let mut rules = StackRules::default();

    for seam in seams {
        let (Some(&a), Some(&b)) = (index.get(&seam.a), index.get(&seam.b)) else {
            continue;
        };
        let low = seam.start.min(seam.end);
        let high = seam.start.max(seam.end);
        for (other, shape) in shapes.iter().enumerate() {
            if other == a
                || other == b
                || shape.maximum.x + EPS < low.x
                || shape.minimum.x - EPS > high.x
                || shape.maximum.y + EPS < low.y
                || shape.minimum.y - EPS > high.y
            {
                continue;
            }
            if crosses_segment(shape, seam.start, seam.end) {
                rules.crossings.push((a, b, other));
                rules.crossing_folded.push(seam.folded);
            }
        }
    }

    for (first, left) in seams.iter().enumerate() {
        let Some(left_side) = left.side else {
            continue;
        };
        for right in seams.iter().skip(first + 1) {
            if right.a == left.a || right.a == left.b || right.b == left.a || right.b == left.b {
                continue;
            }
            let Some(right_side) = right.side else {
                continue;
            };
            let Some((overlap_start, overlap_end)) =
                collinear_overlap(left.start, left.end, right.start, right.end)
            else {
                continue;
            };
            if (overlap_end - overlap_start).length() <= EPS {
                continue;
            }
            let (Some(&a), Some(&b), Some(&c), Some(&d)) = (
                index.get(&left.a),
                index.get(&left.b),
                index.get(&right.a),
                index.get(&right.b),
            ) else {
                continue;
            };
            // 側の符号はそれぞれの折り目の向きに対して測っている。
            // 2本の向きが逆なら符号も反転するので、向きをそろえてから比べる。
            let turn = if (left.end - left.start).dot(right.end - right.start) > 0.0 {
                1.0
            } else {
                -1.0
            };
            let same_side = left_side * right_side * turn > 0.0;
            match (left.folded, right.folded) {
                (true, true) if same_side => rules.nests.push((a, b, c, d)),
                (false, false) => {
                    let (near, far) = if same_side { (c, d) } else { (d, c) };
                    rules.parallels.push((a, b, near, far));
                }
                _ => {}
            }
        }
    }
    rules
}

fn relation_pairs(shapes: &[FaceShape], relation: &StackRelation) -> Vec<(FaceId, FaceId)> {
    let mut pairs = Vec::new();
    for (lower, lower_shape) in shapes.iter().enumerate() {
        for (upper, upper_shape) in shapes.iter().enumerate() {
            if relation.is_below(lower, upper) {
                pairs.push((lower_shape.id, upper_shape.id));
            }
        }
    }
    pairs
}

/// 正面積の重なりだけを列挙する。
///
/// `pose_motion::overlap_witnesses` と同じ三角形分割・凸clipを共有するため、表示順の
/// 認証経路と面積の意味がずれない。同関数の境目は `1e-14`。共有頂点・共有辺の
/// 理論面積0を除き、既存の鶴検査が採る `1e-12` より100倍小さい余裕側にある。
fn positive_overlap_pairs(shapes: &[FaceShape]) -> Result<Vec<(usize, usize)>, String> {
    let folded = shapes
        .iter()
        .map(|shape| {
            shape
                .polygon
                .iter()
                .map(|&point| shape.placement.apply(point))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut pairs = Vec::new();
    for left in 0..shapes.len() {
        for right in left + 1..shapes.len() {
            if shapes[left].maximum.x + EPS < shapes[right].minimum.x
                || shapes[right].maximum.x + EPS < shapes[left].minimum.x
                || shapes[left].maximum.y + EPS < shapes[right].minimum.y
                || shapes[right].maximum.y + EPS < shapes[left].minimum.y
            {
                continue;
            }
            let witnesses = crate::pose_motion::overlap_witnesses(&folded[left], &folded[right])
                .map_err(|error| {
                    format!(
                        "precrease overlap analysis failed for faces {} and {}: {error}",
                        shapes[left].id, shapes[right].id
                    )
                })?;
            if !witnesses.is_empty() {
                pairs.push((left, right));
            }
        }
    }
    Ok(pairs)
}

fn solve_stack_relation(
    shapes: &[FaceShape],
    previous_order: &[FaceId],
    adjacent: &[AdjacentFoldRule],
    seams: &[Seam],
    overlap_failure: OverlapAnalysisFailure,
) -> Result<StackSolution, String> {
    let index = shapes
        .iter()
        .enumerate()
        .map(|(index, shape)| (shape.id, index))
        .collect::<HashMap<_, _>>();
    let mut relation = StackRelation::new(shapes.len());

    for rule in adjacent {
        let (Some(&lower), Some(&upper)) = (index.get(&rule.lower), index.get(&rule.upper)) else {
            continue;
        };
        relation.add(lower, upper);
    }

    let rules = stack_rules(shapes, seams);
    relation.propagate(&rules.crossings, &rules.nests, &rules.parallels);
    let mandatory_constraints = relation_pairs(shapes, &relation);
    let (positive_overlaps, overlap_analysis_error) = match positive_overlap_pairs(shapes) {
        Ok(pairs) => (pairs, None),
        Err(error) => match overlap_failure {
            OverlapAnalysisFailure::Reject => return Err(error),
            OverlapAnalysisFailure::ContinueWithWarning => (Vec::new(), Some(error)),
        },
    };
    let unresolved_overlap_pairs = positive_overlaps
        .into_iter()
        .filter(|&(left, right)| !relation.is_below(left, right) && !relation.is_below(right, left))
        .map(|(left, right)| (shapes[left].id, shapes[right].id))
        .collect::<Vec<_>>();

    // ここからは表示を止めないための暫定順であり、上のmandatory relationには混ぜない。
    let mut involved = BTreeSet::new();
    for &(first, second, other) in &rules.crossings {
        involved.extend([first, second, other]);
    }
    for &(a, b, c, d) in rules.nests.iter().chain(&rules.parallels) {
        involved.extend([a, b, c, d]);
    }
    let seed_rank = previous_order
        .iter()
        .enumerate()
        .map(|(rank, face)| (*face, rank))
        .collect::<HashMap<_, _>>();
    let mut ordered = involved.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|&face| {
        (
            seed_rank
                .get(&shapes[face].id)
                .copied()
                .unwrap_or(usize::MAX),
            shapes[face].id,
        )
    });
    let mandatory_relation = relation.clone();
    let physical_conflicts = relation.discarded.clone();
    let (relation, display_resolution_failure) =
        match resolve_display_relation(relation, &ordered, &rules) {
            Ok(resolved) => (resolved, None),
            Err((first, second)) => {
                // Display totalization failed. Preserve only the candidate-independent relation.
                // The returned pair identifies the search location for diagnostics; it is not
                // itself another physical constraint.
                (
                    mandatory_relation,
                    Some((shapes[first].id, shapes[second].id)),
                )
            }
        };
    let discarded_relations = physical_conflicts
        .iter()
        .map(|&(lower, upper)| (shapes[lower].id, shapes[upper].id))
        .collect::<Vec<_>>();

    Ok(StackSolution {
        display_constraints: relation_pairs(shapes, &relation).into_iter().collect(),
        mandatory_constraints,
        unresolved_overlap_pairs,
        discarded_relations,
        display_resolution_failure,
        overlap_analysis_error,
        counts: PrecreaseConstraintCounts {
            adjacent_folds: adjacent.len(),
            taco_tortilla: rules
                .crossing_folded
                .iter()
                .filter(|&&folded| folded)
                .count(),
            taco_taco: rules.nests.len(),
            continuous: rules
                .crossing_folded
                .iter()
                .filter(|&&folded| !folded)
                .count()
                + rules.parallels.len(),
        },
    })
}

/// 「どちらが下か」を面の組ごとに持ち、推移(aがbの下でbがcの下ならaはcの下)を
/// 常に保つ表。
#[derive(Clone)]
struct StackRelation {
    count: usize,
    below: Vec<bool>,
    discarded: BTreeSet<(usize, usize)>,
}

impl StackRelation {
    fn new(count: usize) -> StackRelation {
        StackRelation {
            count,
            below: vec![false; count * count],
            discarded: BTreeSet::new(),
        }
    }

    fn is_below(&self, lower: usize, upper: usize) -> bool {
        self.below[lower * self.count + upper]
    }

    /// `lower` が `upper` の下だと決める。新しく決まったら真を返す。
    /// 逆向きが既に決まっている場合は、先に決まったほうを残して何もしない。
    fn add(&mut self, lower: usize, upper: usize) -> bool {
        if lower == upper || self.is_below(lower, upper) {
            return false;
        }
        if self.is_below(upper, lower) {
            self.discarded.insert((lower, upper));
            return false;
        }
        let lowers = (0..self.count)
            .filter(|&other| other == lower || self.is_below(other, lower))
            .collect::<Vec<_>>();
        let uppers = (0..self.count)
            .filter(|&other| other == upper || self.is_below(upper, other))
            .collect::<Vec<_>>();
        for &low in &lowers {
            for &high in &uppers {
                if low != high {
                    self.below[low * self.count + high] = true;
                }
            }
        }
        true
    }

    /// `other` が `first` と `second` の**同じ側**にいることを使って上下を広げる。
    fn keep_same_side(&mut self, first: usize, second: usize, other: usize) -> bool {
        let mut changed = false;
        if self.is_below(other, first) {
            changed |= self.add(other, second);
        }
        if self.is_below(first, other) {
            changed |= self.add(second, other);
        }
        if self.is_below(other, second) {
            changed |= self.add(other, first);
        }
        if self.is_below(second, other) {
            changed |= self.add(first, other);
        }
        changed
    }

    /// `inner` が `low` と `high` の間に入るなら、その相方 `mate` も同じ向きで
    /// 間に入れる(2組が交互に並ぶことを禁じる)。
    fn keep_nested(&mut self, low: usize, high: usize, inner: usize, mate: usize) -> bool {
        let mut changed = false;
        if self.is_below(low, inner) && self.is_below(inner, high) {
            changed |= self.add(low, mate);
            changed |= self.add(mate, high);
        }
        if self.is_below(high, inner) && self.is_below(inner, low) {
            changed |= self.add(high, mate);
            changed |= self.add(mate, low);
        }
        changed
    }

    /// 決まる上下が増えなくなるまで、2つの条件を当て続ける。
    /// `near` は `first` と同じ側、`far` は `second` と同じ側にある。
    /// 線の片側で上なら、反対側でも上でなければならない。
    fn keep_parallel(&mut self, first: usize, second: usize, near: usize, far: usize) -> bool {
        let mut changed = false;
        if self.is_below(first, near) {
            changed |= self.add(second, far);
        }
        if self.is_below(near, first) {
            changed |= self.add(far, second);
        }
        if self.is_below(second, far) {
            changed |= self.add(first, near);
        }
        if self.is_below(far, second) {
            changed |= self.add(near, first);
        }
        changed
    }

    fn propagate(
        &mut self,
        crossings: &[(usize, usize, usize)],
        nests: &[(usize, usize, usize, usize)],
        parallels: &[(usize, usize, usize, usize)],
    ) {
        // 1回まわるごとに少なくとも1組は決まるので、面対の数を超えて回ることはない。
        let rounds = self.count * self.count + 1;
        for _ in 0..rounds {
            let mut changed = false;
            for &(first, second, other) in crossings {
                changed |= self.keep_same_side(first, second, other);
            }
            for &(a, b, c, d) in nests {
                changed |= self.keep_nested(a, b, c, d);
                changed |= self.keep_nested(a, b, d, c);
                changed |= self.keep_nested(c, d, a, b);
                changed |= self.keep_nested(c, d, b, a);
            }
            for &(first, second, near, far) in parallels {
                changed |= self.keep_parallel(first, second, near, far);
            }
            if !changed {
                return;
            }
        }
    }
}

fn relation_direction(relation: &StackRelation, first: usize, second: usize) -> Option<bool> {
    if relation.is_below(first, second) {
        Some(true)
    } else if relation.is_below(second, first) {
        Some(false)
    } else {
        None
    }
}

fn relation_between(
    relation: &StackRelation,
    middle: usize,
    first: usize,
    second: usize,
) -> Option<bool> {
    let first_to_middle = relation_direction(relation, first, middle)?;
    let middle_to_second = relation_direction(relation, middle, second)?;
    Some(first_to_middle == middle_to_second)
}

/// 既に向きが決まった比較だけを見る。未決定は分岐探索に残すが、決定済み部分だけで
/// taco/連続面規則を破った枝は、それ以上total化しても直らないので早く捨てる。
fn relation_respects_resolved_stack_rules(relation: &StackRelation, rules: &StackRules) -> bool {
    for &(first, second, other) in &rules.crossings {
        if (relation.is_below(first, other) && relation.is_below(other, second))
            || (relation.is_below(second, other) && relation.is_below(other, first))
        {
            return false;
        }
    }
    for &(a, b, c, d) in &rules.nests {
        if let (Some(c_between), Some(d_between)) = (
            relation_between(relation, c, a, b),
            relation_between(relation, d, a, b),
        ) && c_between != d_between
        {
            return false;
        }
        if let (Some(a_between), Some(b_between)) = (
            relation_between(relation, a, c, d),
            relation_between(relation, b, c, d),
        ) && a_between != b_between
        {
            return false;
        }
    }
    for &(first, second, near, far) in &rules.parallels {
        if let (Some(first_to_near), Some(second_to_far)) = (
            relation_direction(relation, first, near),
            relation_direction(relation, second, far),
        ) && first_to_near != second_to_far
        {
            return false;
        }
    }
    true
}

/// 表示を続けるためのtotal化。優先向きはauthorityではなく探索順にだけ使う。
/// propagationが既存の物理関係を捨てる枝はcloneごと破棄する。total化できなかった場合に
/// 返す面対は表示探索の診断位置であり、物理的に破棄された関係そのものではない。
fn resolve_display_relation(
    relation: StackRelation,
    ordered: &[usize],
    rules: &StackRules,
) -> Result<StackRelation, (usize, usize)> {
    let undecided = ordered.iter().enumerate().find_map(|(position, &first)| {
        ordered[position + 1..]
            .iter()
            .copied()
            .find(|&second| !relation.is_below(first, second) && !relation.is_below(second, first))
            .map(|second| (first, second))
    });
    let Some((preferred_lower, preferred_upper)) = undecided else {
        if relation_respects_resolved_stack_rules(&relation, rules) {
            return Ok(relation);
        }
        let conflict = rules
            .crossings
            .first()
            .map(|&(first, second, _)| (first, second))
            .or_else(|| {
                rules
                    .nests
                    .first()
                    .map(|&(first, second, _, _)| (first, second))
            })
            .or_else(|| {
                rules
                    .parallels
                    .first()
                    .map(|&(first, second, _, _)| (first, second))
            })
            .expect("an invalid resolved stack relation has a rule");
        return Err(conflict);
    };

    for (lower, upper) in [
        (preferred_lower, preferred_upper),
        (preferred_upper, preferred_lower),
    ] {
        let mut branch = relation.clone();
        let discarded_before = branch.discarded.len();
        branch.add(lower, upper);
        branch.propagate(&rules.crossings, &rules.nests, &rules.parallels);
        if branch.discarded.len() != discarded_before
            || !relation_respects_resolved_stack_rules(&branch, rules)
        {
            continue;
        }
        if let Ok(resolved) = resolve_display_relation(branch, ordered, rules) {
            return Ok(resolved);
        }
    }
    Err((preferred_lower, preferred_upper))
}

struct PrecreaseOrderSearch<'a> {
    cp: &'a CreasePattern,
    faces: &'a [Face],
    placements: &'a HashMap<FaceId, Isometry2>,
    shapes: &'a [FaceShape],
    rules: &'a StackRules,
    preferred_order: &'a [FaceId],
    involved: &'a [usize],
    required_constraints: &'a [(FaceId, FaceId)],
}

impl PrecreaseOrderSearch<'_> {
    fn search(&self, relation: StackRelation) -> Result<Option<Vec<FaceId>>, String> {
        let undecided = self
            .involved
            .iter()
            .enumerate()
            .find_map(|(position, &first)| {
                self.involved[position + 1..]
                    .iter()
                    .copied()
                    .find(|&second| {
                        !relation.is_below(first, second) && !relation.is_below(second, first)
                    })
                    .map(|second| (first, second))
            });
        let Some((preferred_lower, preferred_upper)) = undecided else {
            let constraints = relation_pairs(self.shapes, &relation)
                .into_iter()
                .collect::<BTreeSet<_>>();
            let candidate = stable_topological_order(self.preferred_order, &constraints);
            let rank = candidate
                .iter()
                .enumerate()
                .map(|(rank, &face)| (face, rank))
                .collect::<HashMap<_, _>>();
            if self
                .required_constraints
                .iter()
                .any(|&(lower, upper)| rank.get(&lower) >= rank.get(&upper))
            {
                return Ok(None);
            }
            let validation =
                validate_precrease_layer_order(self.cp, self.faces, self.placements, &candidate)?;
            return Ok(validation.is_valid().then_some(candidate));
        };

        for (lower, upper) in [
            (preferred_lower, preferred_upper),
            (preferred_upper, preferred_lower),
        ] {
            let mut branch = relation.clone();
            let discarded_before = branch.discarded.len();
            branch.add(lower, upper);
            branch.propagate(
                &self.rules.crossings,
                &self.rules.nests,
                &self.rules.parallels,
            );
            if branch.discarded.len() != discarded_before
                || !relation_respects_resolved_stack_rules(&branch, self.rules)
            {
                continue;
            }
            if let Some(order) = self.search(branch)? {
                return Ok(Some(order));
            }
        }
        Ok(None)
    }
}

/// 「下→上」の制約を満たす順を、`previous_order` の並びを最大限保って返す。
///
/// 輪になっていても止まらない(「止めずに警告する」)。まだ出していない面のうち、
/// 下から押さえる制約がいちばん少ない面を出す。同数なら `previous_order` の並びで
/// 決めるので、同じ入力には常に同じ順を返す。
fn stable_topological_order(
    previous_order: &[FaceId],
    constraints: &BTreeSet<(FaceId, FaceId)>,
) -> Vec<FaceId> {
    if constraints.is_empty() {
        return previous_order.to_vec();
    }
    let mut above_of = previous_order
        .iter()
        .copied()
        .map(|face| (face, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut below_count = previous_order
        .iter()
        .copied()
        .map(|face| (face, 0usize))
        .collect::<BTreeMap<_, _>>();
    for &(below, above) in constraints {
        let (Some(neighbors), true) = (above_of.get_mut(&below), below_count.contains_key(&above))
        else {
            continue;
        };
        if neighbors.insert(above) {
            *below_count.get_mut(&above).expect("checked above face") += 1;
        }
    }
    let mut emitted = BTreeSet::new();
    let mut order = Vec::with_capacity(previous_order.len());
    while order.len() < previous_order.len() {
        let next = previous_order
            .iter()
            .copied()
            .find(|face| !emitted.contains(face) && below_count[face] == 0)
            .or_else(|| {
                previous_order
                    .iter()
                    .copied()
                    .filter(|face| !emitted.contains(face))
                    .min_by_key(|face| below_count[face])
            });
        let Some(next) = next else {
            break;
        };
        emitted.insert(next);
        order.push(next);
        for &above in &above_of[&next] {
            if !emitted.contains(&above) {
                let count = below_count.get_mut(&above).expect("known above face");
                *count = count.saturating_sub(1);
            }
        }
    }
    order
}

fn settle_kinds_from_order(
    cp: &mut CreasePattern,
    faces: &[Face],
    placements: &HashMap<FaceId, Isometry2>,
    order: &[FaceId],
) -> Result<(), String> {
    let rank = order
        .iter()
        .enumerate()
        .map(|(rank, &face)| (face, rank))
        .collect::<HashMap<_, _>>();
    for (edge_id, owners) in edge_owners(faces) {
        if owners.len() != 2 {
            continue;
        }
        let (a, b) = (owners[0], owners[1]);
        // まだ閉じていない折り目の山谷は、この動きでは決まらない。展開図が持って
        // いる設計をそのまま残す。以前はここも重なり順から書き換えていたので、
        // 1回目の畳みで**展開図じゅうの山谷が面の番号順に従って塗り替えられ**、
        // 2回目以降は上下を決める手がかりが消えていた。
        if placements[&a].mirrored == placements[&b].mirrored {
            continue;
        }
        let edge = cp
            .edges
            .iter_mut()
            .find(|candidate| candidate.id == edge_id)
            .ok_or_else(|| format!("crease edge {edge_id} disappeared"))?;
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        edge.kind = want_kind(rank[&a], rank[&b], placements[&a].mirrored);
    }
    Ok(())
}

fn edge_owners(faces: &[Face]) -> HashMap<EdgeId, Vec<FaceId>> {
    let mut owners = HashMap::<EdgeId, Vec<FaceId>>::new();
    for face in faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().push(face.id);
        }
    }
    owners
}

fn vertex_positions(cp: &CreasePattern) -> HashMap<VertexId, DVec2> {
    cp.vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect()
}

fn segment_on_line(a: DVec2, b: DVec2, line: [[f64; 2]; 2]) -> bool {
    let l0 = DVec2::from(line[0]);
    let direction = (DVec2::from(line[1]) - l0).normalize();
    direction.perp_dot(a - l0).abs() <= PLACEMENT_EPS
        && direction.perp_dot(b - l0).abs() <= PLACEMENT_EPS
}

#[cfg(test)]
mod tests {
    use super::*;
    use ori3_cp::insert_segment;
    use ori3_model::{Document, Edge, Paper, Vertex};

    fn flat_continuity_cp(vertices: Vec<Vertex>, shared_edges: Vec<Edge>) -> CreasePattern {
        let next_vertex_id = vertices.iter().map(|vertex| vertex.id).max().unwrap_or(0) + 1;
        let next_edge_id = shared_edges.iter().map(|edge| edge.id).max().unwrap_or(0) + 1;
        CreasePattern {
            vertices,
            edges: shared_edges,
            next_vertex_id,
            next_edge_id,
        }
    }

    fn translated(x: f64) -> Isometry2 {
        Isometry2 {
            rotation: 0.0,
            translation: DVec2::new(x, 0.0),
            mirrored: false,
        }
    }

    fn test_face_shape(id: FaceId, polygon: Vec<DVec2>) -> FaceShape {
        let (minimum, maximum) = polygon.iter().fold(
            (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
            |(minimum, maximum), &point| (minimum.min(point), maximum.max(point)),
        );
        FaceShape {
            id,
            polygon,
            placement: Isometry2::identity(),
            minimum,
            maximum,
        }
    }

    #[test]
    fn display_tie_seed_rolls_back_a_branch_that_discards_a_physical_relation() {
        // Existing physical relation 2<0 and the continuous crossing (0,1,2) require 2<1.
        // The preferred 1<2 seed makes propagation discard that requirement; the reverse branch
        // is valid. The rejected clone must not leak its relation or discarded marker.
        let mut physical = StackRelation::new(3);
        physical.add(2, 0);
        let rules = StackRules {
            crossings: vec![(0, 1, 2)],
            crossing_folded: vec![false],
            ..StackRules::default()
        };
        let resolved = resolve_display_relation(physical.clone(), &[1, 2, 0], &rules)
            .expect("reverse display branch satisfies the physical crossing");
        assert!(physical.discarded.is_empty());
        assert!(resolved.discarded.is_empty());
        assert!(resolved.is_below(2, 1));
        assert!(!resolved.is_below(1, 2));
    }

    #[test]
    fn display_resolution_failure_is_not_a_physical_order_violation() {
        let validation = PrecreaseOrderValidation {
            display_resolution_failure: Some((0, 1)),
            ..PrecreaseOrderValidation::default()
        };

        assert!(validation.discarded_relations.is_empty());
        assert!(validation.is_valid());
    }

    #[test]
    fn simple_fold_only_carries_unresolved_pairs_between_distinct_flat_components() {
        let triangle = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(0.0, 1.0),
        ];
        let shapes = (0..5)
            .map(|id| test_face_shape(id, triangle.clone()))
            .collect::<Vec<_>>();
        let seams = vec![Seam {
            a: 0,
            b: 1,
            start: DVec2::ZERO,
            end: DVec2::X,
            folded: false,
            side: Some(1.0),
        }];
        let moved = HashSet::from([2, 3]);
        let unresolved = vec![(0, 1), (0, 4), (0, 2), (2, 3)];
        assert_eq!(
            simple_fold_internal_unresolved_pairs(&unresolved, &moved, &shapes, &seams),
            vec![(0, 4), (2, 3)]
        );
    }

    #[test]
    fn overlap_analysis_failure_warns_for_collapse_but_rejects_validation() {
        let shapes = vec![
            test_face_shape(
                0,
                vec![
                    DVec2::new(0.0, 0.0),
                    DVec2::new(1.0, 0.0),
                    DVec2::new(0.0, 1.0),
                ],
            ),
            test_face_shape(
                1,
                vec![
                    DVec2::new(0.0, 0.0),
                    DVec2::new(0.5, 0.0),
                    DVec2::new(1.0, 0.0),
                ],
            ),
        ];
        let continued = solve_stack_relation(
            &shapes,
            &[0, 1],
            &[],
            &[],
            OverlapAnalysisFailure::ContinueWithWarning,
        )
        .expect("collapse keeps a display fallback when overlap analysis is unavailable");
        assert!(continued.overlap_analysis_error.is_some());
        assert!(continued.unresolved_overlap_pairs.is_empty());

        let rejected =
            solve_stack_relation(&shapes, &[0, 1], &[], &[], OverlapAnalysisFailure::Reject)
                .err()
                .expect("saved-order validation must reject an uncheckable overlap");
        assert!(rejected.contains("degenerate face polygon"));
    }

    #[test]
    fn collapses_crossing_precreases_without_sampling_an_angle() {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Aux);
        insert_segment(&mut document.cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Aux);
        let faces = extract_faces(&document.cp);
        let state = FlatState::initial(&document.cp, &faces);
        let result = collapse_precrease_network(
            &mut document.cp,
            &faces,
            &state,
            &PrecreaseCollapseInput {
                lines: vec![[[0.5, 0.0], [0.5, 1.0]], [[0.0, 0.5], [1.0, 0.5]]],
                target_layers: None,
            },
        )
        .unwrap();
        assert_eq!(
            result.warnings,
            vec![format!(
                "{PRECREASE_ORDER_UNDETERMINED_WARNING_PREFIX}6組あります"
            )]
        );
        assert!(
            result.step.layer_order.is_none(),
            "M/V指定の無い2本の同時collapseのFace ID tie-breakをoracleとして保存しない"
        );
        assert_eq!(extract_faces(&document.cp).len(), 4);
        assert_eq!(result.state.placements.len(), 4);
        assert_eq!(
            result.state.order.len(),
            4,
            "authorityでなくても表示用の完全順は維持する"
        );
        assert!(
            document
                .cp
                .edges
                .iter()
                .all(|edge| edge.kind != EdgeKind::Aux)
        );
    }

    #[test]
    fn flat_seam_crossing_rejects_an_interleaved_layer_order() {
        // Faces 0 and 1 are one flat sheet joined at x=0. Face 2 is translated onto both
        // sides of that seam. It must therefore be below both halves or above both halves.
        // All distances are O(1), while CROSSING_OFFSET is 1e-6, so the sampled crossing is
        // six orders of magnitude away from the boundary tolerance.
        let cp = flat_continuity_cp(
            vec![
                Vertex {
                    id: 0,
                    pos: [-1.0, -1.0],
                },
                Vertex {
                    id: 1,
                    pos: [0.0, -1.0],
                },
                Vertex {
                    id: 2,
                    pos: [0.0, 1.0],
                },
                Vertex {
                    id: 3,
                    pos: [-1.0, 1.0],
                },
                Vertex {
                    id: 4,
                    pos: [1.0, -1.0],
                },
                Vertex {
                    id: 5,
                    pos: [1.0, 1.0],
                },
                Vertex {
                    id: 6,
                    pos: [2.0, -0.5],
                },
                Vertex {
                    id: 7,
                    pos: [3.0, -0.5],
                },
                Vertex {
                    id: 8,
                    pos: [3.0, 0.5],
                },
                Vertex {
                    id: 9,
                    pos: [2.0, 0.5],
                },
            ],
            vec![Edge {
                id: 0,
                v0: 1,
                v1: 2,
                kind: EdgeKind::Aux,
            }],
        );
        let faces = vec![
            Face {
                id: 0,
                vertices: vec![0, 1, 2, 3],
                edges: vec![1, 0, 2, 3],
            },
            Face {
                id: 1,
                vertices: vec![1, 4, 5, 2],
                edges: vec![4, 5, 6, 0],
            },
            Face {
                id: 2,
                vertices: vec![6, 7, 8, 9],
                edges: vec![7, 8, 9, 10],
            },
        ];
        let placements = HashMap::from([
            (0, Isometry2::identity()),
            (1, Isometry2::identity()),
            (2, translated(-2.5)),
        ]);

        let valid = validate_precrease_layer_order(&cp, &faces, &placements, &[2, 0, 1])
            .expect("flat-seam crossing can be validated");
        assert_eq!(valid.counts.continuous, 1);
        assert!(valid.is_valid(), "outside order is physical: {valid:?}");

        let interleaved = validate_precrease_layer_order(&cp, &faces, &placements, &[0, 2, 1])
            .expect("interleaved flat-seam crossing can be diagnosed");
        assert_eq!(interleaved.counts.continuous, 1);
        assert_eq!(interleaved.violations.continuous_crossings, vec![(0, 1, 2)]);
        assert!(!interleaved.is_valid());
    }

    #[test]
    fn collinear_flat_seams_reject_a_one_sided_order_reversal() {
        // Two separate flat sheets have coincident, equally oriented seams. Face 0 corresponds
        // to face 2 on the left and face 1 to face 3 on the right. Their relative order cannot
        // reverse across the seam without one continuous sheet passing through the other.
        let cp = flat_continuity_cp(
            vec![
                Vertex {
                    id: 0,
                    pos: [-1.0, -1.0],
                },
                Vertex {
                    id: 1,
                    pos: [0.0, -1.0],
                },
                Vertex {
                    id: 2,
                    pos: [0.0, 1.0],
                },
                Vertex {
                    id: 3,
                    pos: [-1.0, 1.0],
                },
                Vertex {
                    id: 4,
                    pos: [1.0, -1.0],
                },
                Vertex {
                    id: 5,
                    pos: [1.0, 1.0],
                },
                Vertex {
                    id: 6,
                    pos: [2.0, -1.0],
                },
                Vertex {
                    id: 7,
                    pos: [3.0, -1.0],
                },
                Vertex {
                    id: 8,
                    pos: [3.0, 1.0],
                },
                Vertex {
                    id: 9,
                    pos: [2.0, 1.0],
                },
                Vertex {
                    id: 10,
                    pos: [4.0, -1.0],
                },
                Vertex {
                    id: 11,
                    pos: [4.0, 1.0],
                },
            ],
            vec![
                Edge {
                    id: 0,
                    v0: 1,
                    v1: 2,
                    kind: EdgeKind::Aux,
                },
                Edge {
                    id: 10,
                    v0: 7,
                    v1: 8,
                    kind: EdgeKind::Aux,
                },
            ],
        );
        let faces = vec![
            Face {
                id: 0,
                vertices: vec![0, 1, 2, 3],
                edges: vec![1, 0, 2, 3],
            },
            Face {
                id: 1,
                vertices: vec![1, 4, 5, 2],
                edges: vec![4, 5, 6, 0],
            },
            Face {
                id: 2,
                vertices: vec![6, 7, 8, 9],
                edges: vec![11, 10, 12, 13],
            },
            Face {
                id: 3,
                vertices: vec![7, 10, 11, 8],
                edges: vec![14, 15, 16, 10],
            },
        ];
        let placements = HashMap::from([
            (0, Isometry2::identity()),
            (1, Isometry2::identity()),
            (2, translated(-3.0)),
            (3, translated(-3.0)),
        ]);

        let valid = validate_precrease_layer_order(&cp, &faces, &placements, &[0, 1, 2, 3])
            .expect("collinear flat seams can be validated");
        assert_eq!(valid.counts.continuous, 1);
        assert!(
            valid.is_valid(),
            "corresponding order is physical: {valid:?}"
        );

        let reversed = validate_precrease_layer_order(&cp, &faces, &placements, &[0, 2, 3, 1])
            .expect("one-sided flat-seam reversal can be diagnosed");
        assert_eq!(reversed.counts.continuous, 1);
        assert_eq!(reversed.violations.continuous, vec![(0, 1, 2, 3)]);
        assert!(!reversed.is_valid());
    }
}
