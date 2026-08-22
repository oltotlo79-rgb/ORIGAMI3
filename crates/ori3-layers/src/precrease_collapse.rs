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

/// Material-coordinate precrease lines to close in one simultaneous collapse.
#[derive(Clone, Debug)]
pub struct PrecreaseCollapseInput {
    pub lines: Vec<[[f64; 2]; 2]>,
    /// Optional old-face packet. Named hinges must be internal to this packet; auxiliary
    /// segments are activated only inside one of these faces.
    pub target_layers: Option<Vec<FaceId>>,
}

/// Close an intersecting network of existing auxiliary lines to 180 degrees in one flat step.
pub fn collapse_precrease_network(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &PrecreaseCollapseInput,
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
    let target_order = solved_layer_order(
        &work,
        &split_faces,
        &current,
        &target_placements,
        &undetermined,
    );
    settle_kinds_from_order(&mut work, &split_faces, &target_placements, &target_order)?;

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
    if outcome.result.state.order != target_order {
        return Err("precrease collapse did not preserve its solved layer order".to_string());
    }
    let mut result = outcome.result;
    result.added_edges = activated;
    result.added_edges.sort_unstable();
    result.added_edges.dedup();
    *cp = outcome.cp;
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
/// 与えられていないので上下を決めない。その分は `previous_order` の並びが残る。
///
/// 1本の直線で紙を折り返すだけの畳みは、これより強く決まる。動いた紙はまとめて
/// 重なりの外側へ回り、その中の並びはひっくり返る([`simple_fold_order`])。
///
/// 決まらなかった面どうしの並びは `previous_order` から引き継ぐ。上下が輪に
/// なっていても止まらず、押さえる制約がいちばん少ない面から順に出す
/// (`ori3_rigid` の重なり順導出と同じ受け止め方)。
fn solved_layer_order(
    cp: &CreasePattern,
    faces: &[Face],
    current: &FlatState,
    placements: &HashMap<FaceId, Isometry2>,
    undetermined: &HashSet<EdgeId>,
) -> Vec<FaceId> {
    let previous_order = &current.order;
    let kinds = cp
        .edges
        .iter()
        .map(|edge| (edge.id, edge.kind))
        .collect::<HashMap<_, _>>();
    let owners = edge_owners(faces)
        .into_iter()
        .filter(|(_, owners)| owners.len() == 2)
        .collect::<BTreeMap<_, _>>();

    let mut constraints = BTreeSet::<(FaceId, FaceId)>::new();
    // 折り目の番号順に並べた同じ内容。1回の折り返しの向きを数えるときに使う。
    let mut ordered = Vec::<(FaceId, FaceId)>::new();
    for (&edge_id, incident) in &owners {
        if undetermined.contains(&edge_id) {
            continue;
        }
        let (a, b) = (incident[0], incident[1]);
        let (Some(placement_a), Some(placement_b)) = (placements.get(&a), placements.get(&b))
        else {
            continue;
        };
        if placement_a.mirrored == placement_b.mirrored {
            continue; // この折り目は閉じていないので、上下を拘束しない
        }
        let Some(kind) = kinds.get(&edge_id) else {
            continue;
        };
        if !matches!(kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        // 「b が a の上に来るとしたら山谷はこうなる」と読み比べる。
        let pair = if want_kind(0, 1, placement_a.mirrored) == *kind {
            (a, b)
        } else {
            (b, a)
        };
        constraints.insert(pair);
        ordered.push(pair);
    }

    if let Some(order) = simple_fold_order(faces, current, placements, &ordered) {
        return order;
    }

    let shapes = face_shapes(cp, faces, placements);
    let seams = folded_seams(cp, &owners, &shapes);
    let derived = solve_stack_relation(&shapes, previous_order, &constraints, &seams);
    stable_topological_order(previous_order, &derived)
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
/// どちらにしても紙は紙をすり抜けない。多数決(同数なら折り目の番号が小さいほう)で
/// 決め、少数側の山谷は折った結果に合わせて付け直す
/// ([`settle_kinds_from_order`])。折れない手として断ることはしない
/// (`CLAUDE.md` §5「止めずに警告する」)。
///
/// 実測(2026-08-17、出っぱり4/6/8/12本・標本45件): 食い違いを理由にここで
/// 組み立てをやめ、一般の解き方へ渡していたときは、**7件**で折り返した紙が上と下に
/// 散らばった(=紙が紙をすり抜けた)。多数決で決めるようにしたら **0件**になった。
fn simple_fold_order(
    faces: &[Face],
    current: &FlatState,
    placements: &HashMap<FaceId, Isometry2>,
    senses: &[(FaceId, FaceId)],
) -> Option<Vec<FaceId>> {
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
    let moved_above = match votes_above.cmp(&votes_below) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => first_vote.unwrap_or(true),
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
    Some(if moved_above {
        [stayed, block].concat()
    } else {
        [block, stayed].concat()
    })
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
/// この2つで決まらない面対だけ、`previous_order` の並びを採る。採った並びも
/// 制約として入れ直し、そこから決まる上下を最後まで引き出す。
/// 逆向きの上下が同時に出てくる場合(紙がすり抜けている形)は、先に決まったほうを
/// 残す。止めない(「止めずに警告する」)。
fn solve_stack_relation(
    shapes: &[FaceShape],
    previous_order: &[FaceId],
    constraints: &BTreeSet<(FaceId, FaceId)>,
    seams: &[Seam],
) -> BTreeSet<(FaceId, FaceId)> {
    let index = shapes
        .iter()
        .enumerate()
        .map(|(index, shape)| (shape.id, index))
        .collect::<HashMap<_, _>>();
    let mut relation = StackRelation::new(shapes.len());

    for &(lower, upper) in constraints {
        let (Some(&lower), Some(&upper)) = (index.get(&lower), index.get(&upper)) else {
            continue;
        };
        relation.add(lower, upper);
    }

    let mut crossings = Vec::new();
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
                crossings.push((a, b, other));
            }
        }
    }

    let mut nests = Vec::new();
    let mut parallels = Vec::new();
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
                // 折り返しどうし。同じ側へ開いているときだけ重なりを持つ。
                (true, true) if same_side => nests.push((a, b, c, d)),
                // 1枚の続きの紙どうしが、同じ線で切り分けられている。
                // 線の両側で上下が入れ替わることはできない。
                (false, false) => {
                    let (near, far) = if same_side { (c, d) } else { (d, c) };
                    parallels.push((a, b, near, far));
                }
                _ => {}
            }
        }
    }

    relation.propagate(&crossings, &nests, &parallels);

    // 幾何が決めきれなかった面対は、前の重なり順の並びをそのまま採る。採った分も
    // 制約として入れ直し、そこから決まる上下を引き出す。対象は上の2つの条件に
    // 出てくる面だけにする(出てこない面の並びは、この後の並べ替えが
    // `previous_order` のまま残す)。
    let mut involved = BTreeSet::new();
    for &(first, second, other) in &crossings {
        involved.extend([first, second, other]);
    }
    for &(a, b, c, d) in nests.iter().chain(&parallels) {
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
    for (position, &lower) in ordered.iter().enumerate() {
        for &upper in &ordered[position + 1..] {
            if relation.is_below(lower, upper) || relation.is_below(upper, lower) {
                continue;
            }
            relation.add(lower, upper);
            relation.propagate(&crossings, &nests, &parallels);
        }
    }

    let mut out = BTreeSet::new();
    for (lower, lower_shape) in shapes.iter().enumerate() {
        for (upper, upper_shape) in shapes.iter().enumerate() {
            if relation.is_below(lower, upper) {
                out.insert((lower_shape.id, upper_shape.id));
            }
        }
    }
    out
}

/// 「どちらが下か」を面の組ごとに持ち、推移(aがbの下でbがcの下ならaはcの下)を
/// 常に保つ表。
struct StackRelation {
    count: usize,
    below: Vec<bool>,
}

impl StackRelation {
    fn new(count: usize) -> StackRelation {
        StackRelation {
            count,
            below: vec![false; count * count],
        }
    }

    fn is_below(&self, lower: usize, upper: usize) -> bool {
        self.below[lower * self.count + upper]
    }

    /// `lower` が `upper` の下だと決める。新しく決まったら真を返す。
    /// 逆向きが既に決まっている場合は、先に決まったほうを残して何もしない。
    fn add(&mut self, lower: usize, upper: usize) -> bool {
        if lower == upper || self.is_below(lower, upper) || self.is_below(upper, lower) {
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
    use ori3_model::{Document, Paper};

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
        assert!(result.warnings.is_empty());
        assert_eq!(extract_faces(&document.cp).len(), 4);
        assert_eq!(result.state.placements.len(), 4);
        assert!(
            document
                .cp
                .edges
                .iter()
                .all(|edge| edge.kind != EdgeKind::Aux)
        );
    }
}
