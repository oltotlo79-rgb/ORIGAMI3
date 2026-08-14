//! 折り操作: 畳んだ状態の上に折り線を1本引き、対象の層をまとめて折る。
//!
//! 中身は汎用の折り操作([`crate::flat_motion`])の薄い包み紙で、
//! 「折り線の可動側にある紙をその線で鏡映し、重なり全体の上(または下)へ回す」
//! という1つの [`MotionPart`] に翻訳して渡す。折り線の引き戻し・展開図への追記・
//! 層順序の更新・手順の生成はすべて汎用側が行う。
//!
//! 手順の記録([`FoldStep`]の`drivers`)は辺IDではなく「CP座標の線分+角度」
//! ([`DriverLine`])で持つ。後続の折りで辺が分割されてIDが変わっても、
//! 再生時に [`resolve_driver_edges`] で線分上の全断片へ解決できる
//! (層順序の代表点方式と同じ思想)。
//!
//! 既知の制限(v1):
//! - 新しい層順序は「動いた面を旧順序の逆順でまとめて山全体の一番上(Up)または
//!   一番下(Down)に入れる」近似で決める。折り線が一部の層しか跨がない部分的な
//!   折りでは、物理的に厳密な挟み込み順にならないことがある(層の間へ差し込む
//!   入れ方が要る場合は [`crate::flat_motion`] を直接使う)。
//! - 折り線が新しい面を横切らず、既存の山谷折り線とも重ならない指定はエラーにする。
//!   既存の折り線と完全に一致する「再折り」は、その線上の既存断片をdriverとして
//!   記録して受理する(折り目を開く・重なり順だけ変える動きは
//!   [`crate::flat_motion`] で表せる)。

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::DVec2;
use ori3_cp::Face;
use ori3_geometry::{Isometry2, dist_point_segment, point_on_segment, reflect_across_line};
use ori3_model::{
    CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, FaceId, FoldStep, TechniqueKind, VertexId,
};

use crate::flat_motion::{
    FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform, run_motion,
};
use crate::flat_state::FlatState;

/// 折る向き(型定義は手順操作の引数として使うため [`ori3_model`] にある)。
pub use ori3_model::FoldDirection;

/// 折り切り時の層矛盾、または巻き込みが必要な衝突を知らせる警告文。
pub const FOLD_PENETRATION_WARNING: &str = "この折り方だと紙が突き抜けています";
pub(crate) const AUX_PROMOTION_WARNING_MARK: &str = "折り線と重なっていた補助線";

/// fold_throughの入力。座標は全て「畳んだ平面座標」。
#[derive(Clone, Debug)]
pub struct FoldThroughInput {
    /// 折り線(2点。無限直線として扱う)。
    pub line: [[f64; 2]; 2],
    /// 動かさない側を示す点。
    pub keep_side_point: [f64; 2],
    /// 折る対象の層。None = 折り線の可動側に幾何(面の一部でも)が乗る全ての層。
    pub target_layers: Option<Vec<FaceId>>,
    pub direction: FoldDirection,
}

/// fold_through / flat_motion の結果。
#[derive(Clone, Debug)]
pub struct FoldThroughResult {
    /// 折った後の平坦状態(新しい面ID体系)。
    ///
    /// 座標系は表示・[`crate::flat_state_at`] と同じ「根面(最小面ID)が恒等」に
    /// そろえてある(層順序もこの座標系での下→上)。
    pub state: FlatState,
    /// CPへ追記された折り線の辺ID(折りの線種へ昇格させた既存の補助線の断片を含む)。
    pub added_edges: Vec<EdgeId>,
    /// 記録用のステップ(drivers+layer_order設定済み。idは呼び出し側で振り直す前提の0)。
    pub step: FoldStep,
    pub warnings: Vec<String>,
}

/// 紙の縁を回り込むために追加する誘導折り目のプレビュー。
///
/// `folded_line` は現在の畳み平面(3D表示のxy)での線分、`crease_segments` は
/// 展開図へ実際に入るCP座標の線分。典型的な単一衝突縁だけを提案し、複数の縁へ
/// 同時に当たる場合などは提案自体を返さない。
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct FoldThroughProposal {
    pub folded_line: [[f64; 2]; 2],
    pub crease_segments: Vec<[[f64; 2]; 2]>,
    pub message: String,
}

struct ResolvedFold {
    line: [[f64; 2]; 2],
    l0: DVec2,
    l1: DVec2,
    u: DVec2,
    keep_sign: f64,
    movable: DVec2,
    target_ids: Vec<FaceId>,
    warnings: Vec<String>,
    direction: FoldDirection,
}

struct CollisionCandidate {
    edge: [DVec2; 2],
    targets: HashSet<FaceId>,
}

struct ResolvedProposal {
    public: FoldThroughProposal,
    collision_edge: [[f64; 2]; 2],
}

struct CollisionAnalysis {
    /// 候補を安全に一意化できなかった場合もtrue。通常折りでは警告を残す。
    collision: bool,
    proposal: Option<ResolvedProposal>,
}

/// 畳んだ状態の上に折り線を引き、対象の層をまとめて折る。
///
/// 1. 折り線を挟んで `keep_side_point` と反対の側を可動側とし、対象面を決める
/// 2. 可動側の紙を折り線で鏡映し、重なり全体の上(Up)/下(Down)へ回す動きとして
///    [`crate::flat_motion`] に渡す(折り線の引き戻し・挿入・層順序はそちらで行う)
/// 3. 挿入する線種は Up=谷 / Down=山。裏返っている層(mirrored)では反転する
///
/// CPの更新は複製上で行い、成功した場合のみ元の `cp` に反映する(原子性)。
/// 危うい指定(山谷が食い違う重なり・紙が裂ける接続)は警告を付けて続行する。
pub fn fold_through(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &FoldThroughInput,
) -> Result<FoldThroughResult, String> {
    // 技法実装も内部プリミティブとしてこの関数を使う。技法の途中形へ単純折り向けの
    // 衝突ヒューリスティックを掛けないよう、従来経路は幾何操作だけに保つ。画面の
    // SeqOp::FoldThrough は下の拡張関数を使い、提案・警告まで行う。
    let resolved = resolve_fold(cp, faces, state, input)?;
    let out = run_motion(cp, faces, state, &simple_motion(&resolved))?;
    if !out.crossed_any {
        return Err("折り線がどの層の面も横切らず、既存の折り筋にも重なっていません".to_string());
    }
    let promoted_aux_edges = out.promoted_aux_edges;
    let mut result = out.result;
    let mut warnings = resolved.warnings;
    if promoted_aux_edges > 0 {
        warnings.push(format!(
            "{AUX_PROMOTION_WARNING_MARK}{promoted_aux_edges}本を折り線に変更しました"
        ));
    }
    warnings.append(&mut result.warnings);
    result.warnings = warnings;
    *cp = out.cp;
    Ok(result)
}

/// 貫通回避の誘導折り目を事前計算する。文書は変更しない。
///
/// 対応するのは、折る向きにある非対象層の外周縁を鏡映後のフラップが横切り、
/// その縁が主折り線と平行かつ幾何的に1本へ定まる場合だけ。複数縁・非平行・
/// 複数区間へ引き戻される形は複雑ケースとして`None`へフォールバックする。
pub fn propose_fold_through(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &FoldThroughInput,
) -> Result<Option<FoldThroughProposal>, String> {
    let resolved = resolve_fold(cp, faces, state, input)?;
    let Some(proposal) = analyze_collision(cp, faces, state, &resolved).proposal else {
        return Ok(None);
    };
    // 表示した提案が確定時に紙を裂くことがないよう、CP複製相当の非破壊試行まで
    // 成功した候補だけを返す。run_motionは入力CPを書き換えない。
    let trial = run_motion(cp, faces, state, &guided_motion(&resolved, &proposal));
    match trial {
        Ok(out)
            if out.crossed_any
                && !out
                    .result
                    .warnings
                    .iter()
                    .any(|warning| warning.contains(TEAR_MARK)) =>
        {
            Ok(Some(proposal.public))
        }
        _ => Ok(None),
    }
}

/// 単一衝突縁の提案を承諾した場合だけ、誘導折り目を加えて巻き込み折りを行う。
///
/// 承諾していない場合、または複雑ケースで安全な提案を作れない場合は従来の単純折りを
/// そのまま適用し、貫通警告を返す(操作自体は止めない)。
pub fn fold_through_with_additional_crease(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &FoldThroughInput,
    accept_additional_crease: bool,
) -> Result<FoldThroughResult, String> {
    let resolved = resolve_fold(cp, faces, state, input)?;
    let analysis = analyze_collision(cp, faces, state, &resolved);

    // まず従来の単純折りで、主折り線が実際に面を横切るか既存の折り筋に重なることを検査する。
    // 誘導折り目だけが横切る入力を誤って成功させないため、承諾時もこの検査を行う。
    let simple = simple_motion(&resolved);
    let simple_out = run_motion(cp, faces, state, &simple)?;
    if !simple_out.crossed_any {
        return Err("折り線がどの層の面も横切らず、既存の折り筋にも重なっていません".to_string());
    }

    let mut used_proposal = false;
    let out = match analysis.proposal {
        Some(proposal) if accept_additional_crease => {
            let guided = guided_motion(&resolved, &proposal);
            match run_motion(cp, faces, state, &guided) {
                Ok(guided_out)
                    if guided_out.crossed_any
                        && !guided_out
                            .result
                            .warnings
                            .iter()
                            .any(|warning| warning.contains(TEAR_MARK)) =>
                {
                    used_proposal = true;
                    guided_out
                }
                _ => simple_out,
            }
        }
        _ => simple_out,
    };

    let promoted_aux_edges = out.promoted_aux_edges;
    let mut result = out.result;
    let mut warnings = resolved.warnings;
    if promoted_aux_edges > 0 {
        warnings.push(format!(
            "{AUX_PROMOTION_WARNING_MARK}{promoted_aux_edges}本を折り線に変更しました"
        ));
    }
    warnings.append(&mut result.warnings);
    if analysis.collision
        && !used_proposal
        && !warnings
            .iter()
            .any(|warning| warning == FOLD_PENETRATION_WARNING)
    {
        warnings.push(FOLD_PENETRATION_WARNING.to_string());
    }
    result.warnings = warnings;
    *cp = out.cp;
    Ok(result)
}

fn resolve_fold(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &FoldThroughInput,
) -> Result<ResolvedFold, String> {
    let l0 = DVec2::from(input.line[0]);
    let l1 = DVec2::from(input.line[1]);
    if (l1 - l0).length() < EPS {
        return Err("折り線の2点が一致しています".to_string());
    }
    let u = (l1 - l0).normalize();
    let keep = DVec2::from(input.keep_side_point);
    let keep_side = u.perp_dot(keep - l0);
    if keep_side.abs() <= EPS {
        return Err("動かさない側を示す点が折り線上にあります".to_string());
    }
    let keep_sign = keep_side.signum();

    for f in faces {
        if !state.placements.contains_key(&f.id) {
            return Err(format!("面 {} の配置が平坦状態に見つかりません", f.id));
        }
    }

    let vpos = vertex_positions(cp);
    // 面の一部でも可動側に乗るか。直線から最も離れた点は必ず頂点に現れるので頂点だけ見る。
    let has_movable_part = |f: &Face| -> bool {
        let pl = &state.placements[&f.id];
        f.vertices
            .iter()
            .filter_map(|id| vpos.get(id).copied())
            .any(|p| keep_sign * u.perp_dot(pl.apply(p) - l0) < -EPS)
    };

    let mut warnings: Vec<String> = Vec::new();
    let target_ids: Vec<FaceId> = match &input.target_layers {
        None => faces
            .iter()
            .filter(|f| has_movable_part(f))
            .map(|f| f.id)
            .collect(),
        Some(list) => {
            let mut out: Vec<FaceId> = Vec::new();
            for &id in list {
                if out.contains(&id) {
                    continue;
                }
                match faces.iter().find(|f| f.id == id) {
                    None => warnings.push(format!(
                        "対象層 {id} は現在の面に存在しないため除外しました"
                    )),
                    Some(f) if !has_movable_part(f) => warnings.push(format!(
                        "対象層 {id} は折り線の可動側に掛かっていないため除外しました"
                    )),
                    Some(_) => out.push(id),
                }
            }
            out
        }
    };
    if target_ids.is_empty() {
        return Err("折り線の可動側に折る対象の層がありません".to_string());
    }

    // 可動側を示す点は、動かさない側の点を折り線で鏡映して作る(必ず反対側に来る)。
    let movable = reflect_across_line(keep, l0, l1);
    Ok(ResolvedFold {
        line: input.line,
        l0,
        l1,
        u,
        keep_sign,
        movable,
        target_ids,
        warnings,
        direction: input.direction,
    })
}

fn simple_motion(resolved: &ResolvedFold) -> FlatMotionInput {
    FlatMotionInput {
        parts: vec![MotionPart {
            layers: resolved.target_ids.clone(),
            region: vec![HalfPlane {
                line: resolved.line,
                inside_point: [resolved.movable.x, resolved.movable.y],
            }],
            transform: MotionTransform::Reflect(vec![resolved.line]),
            turn: LayerTurn::Outside(resolved.direction),
            reverse_layers: None,
        }],
        kind: TechniqueKind::Simple,
    }
}

fn guided_motion(resolved: &ResolvedFold, proposal: &ResolvedProposal) -> FlatMotionInput {
    let guide = proposal.public.folded_line;
    let g0 = DVec2::from(guide[0]);
    let g1 = DVec2::from(guide[1]);
    let gu = (g1 - g0).normalize();
    let side = gu.perp_dot((resolved.l0 + resolved.l1) * 0.5 - g0);
    // 主折り線と反対側がフラップの先端(縁を越えてしまう遠側)。解析で平行かつ
    // 離れていることを確認済みなので、この点は必ずguideの遠側に来る。
    let normal = DVec2::new(-gu.y, gu.x);
    let far = (g0 + g1) * 0.5 - normal * side.signum();
    let movable = [resolved.movable.x, resolved.movable.y];
    FlatMotionInput {
        // 同じ紙が重なる領域では先頭が優先。遠側は主折りの後に衝突縁で折り返し、
        // 近側は従来どおり主折りだけなのでguide上で位置が連続する。
        parts: vec![
            MotionPart {
                layers: resolved.target_ids.clone(),
                region: vec![
                    HalfPlane {
                        line: resolved.line,
                        inside_point: movable,
                    },
                    HalfPlane {
                        line: guide,
                        inside_point: [far.x, far.y],
                    },
                ],
                transform: MotionTransform::Reflect(vec![resolved.line, proposal.collision_edge]),
                turn: LayerTurn::Outside(resolved.direction),
                reverse_layers: None,
            },
            MotionPart {
                layers: resolved.target_ids.clone(),
                region: vec![HalfPlane {
                    line: resolved.line,
                    inside_point: movable,
                }],
                transform: MotionTransform::Reflect(vec![resolved.line]),
                turn: LayerTurn::Outside(resolved.direction),
                reverse_layers: None,
            },
        ],
        kind: TechniqueKind::Simple,
    }
}

/// 主折り後のフラップと、進行方向にある層の外周縁との衝突を調べる。
fn analyze_collision(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    resolved: &ResolvedFold,
) -> CollisionAnalysis {
    let vpos = vertex_positions(cp);
    let rank: HashMap<FaceId, usize> = state
        .order
        .iter()
        .enumerate()
        .map(|(index, &id)| (id, index))
        .collect();
    let edge_kind: HashMap<EdgeId, EdgeKind> =
        cp.edges.iter().map(|edge| (edge.id, edge.kind)).collect();
    let target_set: HashSet<FaceId> = resolved.target_ids.iter().copied().collect();
    let main_reflection = Isometry2::reflection(resolved.l0, resolved.l1);
    let mut candidates: Vec<CollisionCandidate> = Vec::new();
    let mut saw_collision = false;
    let mut unsupported_collision = false;

    for target in faces.iter().filter(|face| target_set.contains(&face.id)) {
        let Some(&target_rank) = rank.get(&target.id) else {
            continue;
        };
        let target_poly = folded_face_polygon(target, &vpos, state);
        if target_poly.len() < 3 {
            continue;
        }
        // 凹フラップは半平面で複数片へ分かれ得るので提案はしない。ただし下の
        // 簡易走査は続け、縁を横断した事実を警告へフォールバックできるようにする。
        let target_is_convex = is_convex(&target_poly);
        let movable = clip_to_side(&target_poly, resolved.l0, resolved.u, -resolved.keep_sign);
        if polygon_area(&movable).abs() <= EPS * EPS {
            continue;
        }
        let folded: Vec<DVec2> = movable
            .iter()
            .map(|&point| main_reflection.apply(point))
            .collect();

        for stationary in faces.iter().filter(|face| !target_set.contains(&face.id)) {
            let Some(&stationary_rank) = rank.get(&stationary.id) else {
                continue;
            };
            let blocks = match resolved.direction {
                FoldDirection::Up => stationary_rank > target_rank,
                FoldDirection::Down => stationary_rank < target_rank,
            };
            if !blocks {
                continue;
            }
            let stationary_poly = folded_face_polygon(stationary, &vpos, state);
            // もともと重なっていない紙の縁は、単に隣へはみ出すだけで衝突ではない。
            // 可動部そのものは障害の外にあり、主折りで初めて中へ入る場合だけを扱う。
            if !polygons_overlap_interior(&target_poly, &stationary_poly)
                || polygons_overlap_interior(&movable, &stationary_poly)
                || !polygons_overlap_interior(&folded, &stationary_poly)
            {
                continue;
            }

            for index in 0..stationary.vertices.len() {
                let Some(&edge_id) = stationary.edges.get(index) else {
                    continue;
                };
                if edge_kind.get(&edge_id) != Some(&EdgeKind::Border) {
                    continue; // 典型ケースは重なった紙の外周。内部折り目は提案しない
                }
                let Some(&a) = stationary_poly.get(index) else {
                    continue;
                };
                let Some(&b) = stationary_poly.get((index + 1) % stationary_poly.len()) else {
                    continue;
                };
                let edge_dir = b - a;
                if edge_dir.length() <= EPS {
                    continue;
                }
                if !folded_flap_crosses_edge(&folded, [a, b]) {
                    continue;
                }
                saw_collision = true;
                if !target_is_convex {
                    unsupported_collision = true;
                    continue;
                }
                // 非平行な縁ではguideと主折り線が交わり、遠側を1半平面で一意に
                // 決められない。衝突は記録したうえで警告のみにする。
                if resolved.u.perp_dot(edge_dir.normalize()).abs() > 1e-6 {
                    unsupported_collision = true;
                    continue;
                }

                if let Some(existing) = candidates
                    .iter_mut()
                    .find(|candidate| same_segment(candidate.edge, [a, b]))
                {
                    existing.targets.insert(target.id);
                } else {
                    candidates.push(CollisionCandidate {
                        edge: [a, b],
                        targets: HashSet::from([target.id]),
                    });
                }
            }
        }
    }

    if candidates.is_empty() {
        return CollisionAnalysis {
            collision: saw_collision,
            proposal: None,
        };
    }
    if unsupported_collision || candidates.len() != 1 || resolved.target_ids.len() != 1 {
        return CollisionAnalysis {
            collision: true,
            proposal: None,
        };
    }

    let candidate = candidates.pop().expect("候補が1件であることを確認済み");
    let guide = [
        main_reflection.apply(candidate.edge[0]),
        main_reflection.apply(candidate.edge[1]),
    ];
    if (guide[1] - guide[0]).length() <= EPS
        || (guide[1] - guide[0]).normalize().perp_dot(resolved.u).abs() > 1e-6
    {
        return CollisionAnalysis {
            collision: true,
            proposal: None,
        };
    }
    let guide_side = (guide[1] - guide[0])
        .normalize()
        .perp_dot((resolved.l0 + resolved.l1) * 0.5 - guide[0]);
    if guide_side.abs() <= EPS {
        return CollisionAnalysis {
            collision: true,
            proposal: None,
        };
    }

    let mut crease_segments: Vec<[[f64; 2]; 2]> = Vec::new();
    for target_id in &candidate.targets {
        let Some(target) = faces.iter().find(|face| face.id == *target_id) else {
            continue;
        };
        let placement = state.placements[&target.id];
        let inverse = placement.inverse();
        let pulled = [inverse.apply(guide[0]), inverse.apply(guide[1])];
        let poly: Vec<DVec2> = target
            .vertices
            .iter()
            .filter_map(|vertex| vpos.get(vertex).copied())
            .collect();
        let intervals = segment_inside_polygon(pulled, &poly);
        // 凹面を複数区間で横切る引き戻しは、どこを巻くか一意に示せない。
        if intervals.len() != 1 {
            return CollisionAnalysis {
                collision: true,
                proposal: None,
            };
        }
        let interval = intervals[0];
        let midpoint = placement.apply((interval[0] + interval[1]) * 0.5);
        if resolved.keep_sign * resolved.u.perp_dot(midpoint - resolved.l0) >= -EPS {
            return CollisionAnalysis {
                collision: true,
                proposal: None,
            };
        }
        let segment = [interval[0].to_array(), interval[1].to_array()];
        if !crease_segments.iter().any(|existing| {
            same_segment(
                [DVec2::from(existing[0]), DVec2::from(existing[1])],
                interval,
            )
        }) {
            crease_segments.push(segment);
        }
    }
    if crease_segments.is_empty()
        || wrapped_flap_crosses_edge(cp, faces, state, resolved, &candidate, guide)
    {
        return CollisionAnalysis {
            collision: true,
            proposal: None,
        };
    }
    crease_segments.sort_by(|a, b| {
        a[0][0]
            .total_cmp(&b[0][0])
            .then(a[0][1].total_cmp(&b[0][1]))
            .then(a[1][0].total_cmp(&b[1][0]))
            .then(a[1][1].total_cmp(&b[1][1]))
    });

    CollisionAnalysis {
        collision: true,
        proposal: Some(ResolvedProposal {
            public: FoldThroughProposal {
                folded_line: [guide[0].to_array(), guide[1].to_array()],
                crease_segments,
                message: "指定した場所以外に、ここへ折り目がつきます。追加折り目で紙を縁に沿って巻き込み、突き抜けを避けます"
                    .to_string(),
            },
            collision_edge: [candidate.edge[0].to_array(), candidate.edge[1].to_array()],
        }),
    }
}

fn folded_face_polygon(
    face: &Face,
    positions: &HashMap<VertexId, DVec2>,
    state: &FlatState,
) -> Vec<DVec2> {
    let placement = state.placements[&face.id];
    face.vertices
        .iter()
        .filter_map(|vertex| positions.get(vertex).copied())
        .map(|point| placement.apply(point))
        .collect()
}

fn clip_to_side(poly: &[DVec2], line_point: DVec2, line_dir: DVec2, sign: f64) -> Vec<DVec2> {
    if poly.len() < 3 {
        return Vec::new();
    }
    let signed = |point: DVec2| sign * line_dir.perp_dot(point - line_point);
    let mut out = Vec::with_capacity(poly.len() + 1);
    for index in 0..poly.len() {
        let (a, b) = (poly[index], poly[(index + 1) % poly.len()]);
        let (da, db) = (signed(a), signed(b));
        if da >= -EPS {
            out.push(a);
        }
        if (da > EPS && db < -EPS) || (da < -EPS && db > EPS) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    dedup_polygon(out)
}

fn dedup_polygon(points: Vec<DVec2>) -> Vec<DVec2> {
    let mut out: Vec<DVec2> = Vec::with_capacity(points.len());
    for point in points {
        if out
            .last()
            .is_none_or(|previous| (point - *previous).length() > EPS)
        {
            out.push(point);
        }
    }
    if out.len() > 1 && (out[0] - out[out.len() - 1]).length() <= EPS {
        out.pop();
    }
    out
}

fn polygon_area(poly: &[DVec2]) -> f64 {
    0.5 * (0..poly.len())
        .map(|index| poly[index].perp_dot(poly[(index + 1) % poly.len()]))
        .sum::<f64>()
}

fn is_convex(poly: &[DVec2]) -> bool {
    let mut turn = 0.0f64;
    for index in 0..poly.len() {
        let a = poly[index];
        let b = poly[(index + 1) % poly.len()];
        let c = poly[(index + 2) % poly.len()];
        let cross = (b - a).perp_dot(c - b);
        if cross.abs() <= EPS {
            continue;
        }
        if turn != 0.0 && cross.signum() != turn.signum() {
            return false;
        }
        turn = cross;
    }
    turn != 0.0
}

fn point_on_polygon_boundary(poly: &[DVec2], point: DVec2) -> bool {
    (0..poly.len())
        .any(|index| dist_point_segment(point, poly[index], poly[(index + 1) % poly.len()]) <= EPS)
}

fn point_in_polygon(poly: &[DVec2], point: DVec2) -> bool {
    let mut inside = false;
    for index in 0..poly.len() {
        let (a, b) = (poly[index], poly[(index + 1) % poly.len()]);
        if (a.y > point.y) != (b.y > point.y) {
            let t = (point.y - a.y) / (b.y - a.y);
            if point.x < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
    }
    inside
}

fn point_strictly_inside(poly: &[DVec2], point: DVec2) -> bool {
    !point_on_polygon_boundary(poly, point) && point_in_polygon(poly, point)
}

fn proper_segment_intersection(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let r = b - a;
    let s = d - c;
    let denominator = r.perp_dot(s);
    if denominator.abs() <= EPS {
        return false;
    }
    let t = (c - a).perp_dot(s) / denominator;
    let u = (c - a).perp_dot(r) / denominator;
    t > EPS && t < 1.0 - EPS && u > EPS && u < 1.0 - EPS
}

fn polygons_overlap_interior(a: &[DVec2], b: &[DVec2]) -> bool {
    if a.iter().any(|&point| point_strictly_inside(b, point))
        || b.iter().any(|&point| point_strictly_inside(a, point))
        || (0..a.len())
            .any(|index| point_strictly_inside(b, (a[index] + a[(index + 1) % a.len()]) * 0.5))
        || (0..b.len())
            .any(|index| point_strictly_inside(a, (b[index] + b[(index + 1) % b.len()]) * 0.5))
    {
        return true;
    }
    if (0..a.len()).any(|i| {
        (0..b.len()).any(|j| {
            proper_segment_intersection(a[i], a[(i + 1) % a.len()], b[j], b[(j + 1) % b.len()])
        })
    }) {
        return true;
    }
    // 同じ輪郭で頂点が全て境界上にある重なりを拾う。
    let ca = a.iter().copied().sum::<DVec2>() / a.len() as f64;
    let cb = b.iter().copied().sum::<DVec2>() / b.len() as f64;
    point_strictly_inside(b, ca) || point_strictly_inside(a, cb)
}

fn same_segment(a: [DVec2; 2], b: [DVec2; 2]) -> bool {
    ((a[0] - b[0]).length() <= 1e-6 && (a[1] - b[1]).length() <= 1e-6)
        || ((a[0] - b[1]).length() <= 1e-6 && (a[1] - b[0]).length() <= 1e-6)
}

/// フラップ多角形が縁の有限区間を通って、その無限直線の両側へ出ているか。
fn folded_flap_crosses_edge(poly: &[DVec2], edge: [DVec2; 2]) -> bool {
    if poly.len() < 3 || (edge[1] - edge[0]).length() <= EPS {
        return false;
    }
    let direction = (edge[1] - edge[0]).normalize();
    let mut negative = false;
    let mut positive = false;
    for &point in poly {
        let side = direction.perp_dot(point - edge[0]);
        negative |= side < -EPS;
        positive |= side > EPS;
    }
    negative && positive && !segment_inside_polygon(edge, poly).is_empty()
}

/// 線分のうち多角形の厳密な内部にある区間。複数区間なら凹面を横切っている。
fn segment_inside_polygon(segment: [DVec2; 2], poly: &[DVec2]) -> Vec<[DVec2; 2]> {
    let (a, b) = (segment[0], segment[1]);
    let direction = b - a;
    let length2 = direction.length_squared();
    if poly.len() < 3 || length2 <= EPS * EPS {
        return Vec::new();
    }
    let mut parameters = vec![0.0, 1.0];
    for index in 0..poly.len() {
        let (c, d) = (poly[index], poly[(index + 1) % poly.len()]);
        let edge = d - c;
        let denominator = direction.perp_dot(edge);
        if denominator.abs() <= EPS {
            if direction.perp_dot(c - a).abs() <= EPS * direction.length() {
                parameters.push((c - a).dot(direction) / length2);
                parameters.push((d - a).dot(direction) / length2);
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
    let mut intervals: Vec<[DVec2; 2]> = Vec::new();
    for pair in parameters.windows(2) {
        let (t0, t1) = (pair[0], pair[1]);
        if t1 - t0 <= EPS {
            continue;
        }
        let midpoint = a + direction * (0.5 * (t0 + t1));
        if point_strictly_inside(poly, midpoint) {
            intervals.push([a + direction * t0, a + direction * t1]);
        }
    }
    intervals
}

/// 近側=主鏡映、遠側=主鏡映後に衝突縁で鏡映した結果を、検出時と同じ述語で検査。
fn wrapped_flap_crosses_edge(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    resolved: &ResolvedFold,
    candidate: &CollisionCandidate,
    guide: [DVec2; 2],
) -> bool {
    let vpos = vertex_positions(cp);
    let main = Isometry2::reflection(resolved.l0, resolved.l1);
    let obstacle = Isometry2::reflection(candidate.edge[0], candidate.edge[1]);
    let guide_direction = (guide[1] - guide[0]).normalize();
    let near_sign = guide_direction
        .perp_dot((resolved.l0 + resolved.l1) * 0.5 - guide[0])
        .signum();
    for target in faces
        .iter()
        .filter(|face| candidate.targets.contains(&face.id))
    {
        let poly = folded_face_polygon(target, &vpos, state);
        let movable = clip_to_side(&poly, resolved.l0, resolved.u, -resolved.keep_sign);
        let near = clip_to_side(&movable, guide[0], guide_direction, near_sign);
        let far = clip_to_side(&movable, guide[0], guide_direction, -near_sign);
        let near_after: Vec<DVec2> = near.iter().map(|&point| main.apply(point)).collect();
        let far_after: Vec<DVec2> = far
            .iter()
            .map(|&point| obstacle.apply(main.apply(point)))
            .collect();
        if folded_flap_crosses_edge(&near_after, candidate.edge)
            || folded_flap_crosses_edge(&far_after, candidate.edge)
        {
            return true;
        }
    }
    false
}

/// DriverLineの線分上に乗る折り辺(山/谷)を現在のCPから解決する。
///
/// 「乗る」= 辺の両端点が線分から EPS 以内(同一直線上かつ区間内)。
/// 後続の折りで辺が分割されていても全ての断片が返る。
/// 順序は `cp.edges` の並び順で決定的。線分が退化している場合は空。
pub fn resolve_driver_edges(cp: &CreasePattern, line: &DriverLine) -> Vec<EdgeId> {
    let a = DVec2::from(line.a);
    let b = DVec2::from(line.b);
    if (b - a).length() < EPS {
        return Vec::new();
    }
    let vpos = vertex_positions(cp);
    cp.edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .filter(|e| {
            let (Some(&p0), Some(&p1)) = (vpos.get(&e.v0), vpos.get(&e.v1)) else {
                return false;
            };
            (p1 - p0).length() >= EPS && point_on_segment(p0, a, b) && point_on_segment(p1, a, b)
        })
        .map(|e| e.id)
        .collect()
}

/// 平坦状態を「根面(最小面ID)が恒等変換」の座標系へそろえる。
///
/// 根面の配置を n とすると、全ての配置へ左から n⁻¹ を掛ける
/// (`compose` は self∘other = otherが先なので `n_inv.compose(&p)` = p を先に適用)。
/// n が裏返し(`mirrored`)のときは紙全体をひっくり返して見ることになるので、
/// 層順序(下→上)も反転する。面が1つも無い場合はそのまま返す。
pub(crate) fn normalize_to_root(
    placements: HashMap<FaceId, Isometry2>,
    order: Vec<FaceId>,
) -> (HashMap<FaceId, Isometry2>, Vec<FaceId>) {
    let Some(root) = placements.keys().copied().min() else {
        return (placements, order);
    };
    let n = placements[&root];
    let n_inv = n.inverse();
    let placements = placements
        .into_iter()
        .map(|(id, p)| (id, n_inv.compose(&p)))
        .collect();
    let mut order = order;
    if n.mirrored {
        order.reverse();
    }
    (placements, order)
}

/// 辺ID → その辺を境界に持つ面ID(面ごとに重複を除く)。
/// ちょうど2面を持つ辺が折り目(ヒンジ)にあたる。
pub(crate) fn faces_by_edge(faces: &[Face]) -> BTreeMap<EdgeId, Vec<FaceId>> {
    let mut out: BTreeMap<EdgeId, Vec<FaceId>> = BTreeMap::new();
    for f in faces {
        let mut ids: Vec<EdgeId> = f.edges.clone();
        ids.sort_unstable();
        ids.dedup();
        for eid in ids {
            out.entry(eid).or_default().push(f.id);
        }
    }
    out
}

pub(crate) fn vertex_positions(cp: &CreasePattern) -> HashMap<VertexId, DVec2> {
    cp.vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect()
}

/// 同じ線分(向きの違いは同一視)+同じ角度のDriverLineを重複させずに追加する。
/// 既存の折り目に沿う区間は、隣接する2面の引き戻しから同じ線分が2回出るため。
pub(crate) fn push_driver_line(lines: &mut Vec<DriverLine>, q0: DVec2, q1: DVec2, angle: f64) {
    let dup = lines.iter().any(|x| {
        let xa = DVec2::from(x.a);
        let xb = DVec2::from(x.b);
        x.target_angle_deg == angle
            && (((xa - q0).length() <= EPS && (xb - q1).length() <= EPS)
                || ((xa - q1).length() <= EPS && (xb - q0).length() <= EPS))
    });
    if !dup {
        lines.push(DriverLine {
            a: [q0.x, q0.y],
            b: [q1.x, q1.y],
            target_angle_deg: angle,
        });
    }
}

/// 「紙が裂ける」警告の目印。技法([`crate::techniques`])は複数回の折りで1つの形を
/// 作るため、途中の折りで必ず出るこの警告を選り分けて捨てる。文言が離れないよう、
/// 判定はこの定数を通して行う。
pub(crate) const TEAR_MARK: &str = "紙が裂けます";

/// 折り線の一部が反対向きの既存の折り目に乗っている場合の警告文。
pub(crate) fn opposite_crease_warning(eid: EdgeId) -> String {
    format!(
        "折り線の一部に反対向きの折り線(山/谷)が既にあります(辺ID {eid})。折り上がりは同じですが、そのままでは折り途中の形が正しく表示されません"
    )
}

/// 線種に対応する完全折りの角度(+180=山, -180=谷)。
pub(crate) fn angle_of(kind: EdgeKind) -> f64 {
    match kind {
        EdgeKind::Mountain => 180.0,
        _ => -180.0,
    }
}

/// 山谷の反転(Border/Auxはそのまま)。
pub(crate) fn flip_kind(kind: EdgeKind) -> EdgeKind {
    match kind {
        EdgeKind::Valley => EdgeKind::Mountain,
        EdgeKind::Mountain => EdgeKind::Valley,
        k => k,
    }
}

/// 平坦な紙を上/下へ折る従来の山谷規則。
/// `None` は従来どおり `Up` と同じ谷折りとして扱う。
pub(crate) fn flat_fold_kind(direction: Option<FoldDirection>, mirrored: bool) -> EdgeKind {
    let base = match direction {
        Some(FoldDirection::Down) => EdgeKind::Mountain,
        _ => EdgeKind::Valley,
    };
    if mirrored { flip_kind(base) } else { base }
}
