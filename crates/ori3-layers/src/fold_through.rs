//! 折り操作プリミティブ: 畳んだ状態の上に折り線を引き、対象の層をまとめて折る。
//!
//! 折り線は「畳んだ平面座標」で無限直線として与え、各対象面のplacement逆変換で
//! 展開図(CP)座標へ引き戻し、面と交わる区間だけを挿入する。CPへの折り線追記・
//! 新しい平坦状態の構築・FoldStep生成を原子的に行い、途中で失敗した場合は
//! CPを一切変更しない。
//!
//! 手順の記録([`FoldStep`]の`drivers`)は辺IDではなく「CP座標の線分+角度」
//! ([`DriverLine`])で持つ。後続の折りで辺が分割されてIDが変わっても、
//! 再生時に [`resolve_driver_edges`] で線分上の全断片へ解決できる
//! (層順序の代表点方式と同じ思想)。
//!
//! 既知の制限(v1):
//! - 新しい層順序は「動いた面を旧順序の逆順でまとめて山全体の一番上(Up)または
//!   一番下(Down)に入れる」近似で決める。折り線が一部の層しか跨がない部分的な
//!   折りでは、物理的に厳密な挟み込み順にならないことがある。
//! - 折り線がどの面も横切らない指定はエラーにする。既存の折り線と完全に一致する
//!   「再折り」(新しい線を1本も引かない折り)もこの条件に該当し、未対応。

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::DVec2;
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_geometry::{Isometry2, collinear_overlap, dist_point_segment, point_on_segment};
use ori3_model::{
    CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, FaceId, FoldStep, TechniqueKind, VertexId,
};

use crate::flat_state::{FlatState, point_in_face, representative_point};

/// 折る向き。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldDirection {
    /// 動く側の層を反転して山の一番上に載せる(紙の表から見て谷折りに相当)。
    Up,
    /// 動く側の層を反転して山の一番下に入れる(山折りに相当)。
    Down,
}

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

/// fold_throughの結果。
#[derive(Clone, Debug)]
pub struct FoldThroughResult {
    /// 折った後の平坦状態(新しい面ID体系)。
    pub state: FlatState,
    /// CPへ追記された折り線の辺ID(折りの線種へ昇格させた既存の補助線の断片を含む)。
    pub added_edges: Vec<EdgeId>,
    /// 記録用のステップ(kind=Simple、drivers+layer_order設定済み。idは呼び出し側で振り直す前提の0)。
    pub step: FoldStep,
    pub warnings: Vec<String>,
}

/// 畳んだ状態の上に折り線を引き、対象の層をまとめて折る。
///
/// 1. 折り線を挟んで `keep_side_point` と反対の側を可動側とし、対象面を決める
/// 2. 各対象面へ折り線をplacement逆変換で引き戻し、面と交わる区間をCPへ挿入する
///    (面を横切らない対象面は線を引かず丸ごと動く)
/// 3. 挿入する線種は Up=谷 / Down=山。裏返っている層(mirrored)では反転する。
///    重なった補助線は折りの線種へ昇格し、既存の山/谷線は線種を維持したまま駆動対象になる
/// 4. 面を再抽出し、可動側の新しい面へ折り線の鏡映を重ね、層順序を更新する
///
/// CPの更新は複製上で行い、成功した場合のみ元の `cp` に反映する(原子性)。
/// 折り線が横切ったのに面を分割できなかった場合は状態を壊す前にErrで止める。
/// 危うい指定(山谷が食い違う重なり・紙が裂ける接続)は警告を付けて続行する。
pub fn fold_through(
    cp: &mut CreasePattern,
    faces: &[Face],
    state: &FlatState,
    input: &FoldThroughInput,
) -> Result<FoldThroughResult, String> {
    let mut warnings: Vec<String> = Vec::new();

    let l0 = DVec2::from(input.line[0]);
    let l1 = DVec2::from(input.line[1]);
    if (l1 - l0).length() < EPS {
        return Err("折り線の2点が一致しています".to_string());
    }
    let u = (l1 - l0).normalize();
    let keep_side = u.perp_dot(DVec2::from(input.keep_side_point) - l0);
    if keep_side.abs() <= EPS {
        return Err("動かさない側を示す点が折り線上にあります".to_string());
    }
    let keep_sign = keep_side.signum();
    // 折り線に対する符号付き距離(畳んだ平面座標)。正=動かさない側、負=可動側。
    let signed_dist = |q: DVec2| keep_sign * u.perp_dot(q - l0);

    for f in faces {
        if !state.placements.contains_key(&f.id) {
            return Err(format!("面 {} の配置が平坦状態に見つかりません", f.id));
        }
    }

    let vpos: HashMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();
    // 面の境界多角形(CP座標)。存在しない頂点は飛ばす(flat_stateと同じ方針)。
    let polygon = |f: &Face| -> Vec<DVec2> {
        f.vertices
            .iter()
            .filter_map(|id| vpos.get(id).copied())
            .collect()
    };
    // 面の一部でも可動側に乗るか。直線から最も離れた点は必ず頂点に現れるので頂点だけ見る。
    let has_movable_part = |f: &Face| -> bool {
        let pl = &state.placements[&f.id];
        polygon(f).iter().any(|&p| signed_dist(pl.apply(p)) < -EPS)
    };

    // 1. 対象面の決定
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
                    None => warnings
                        .push(format!("対象層 {id} は現在の面に存在しないため除外しました")),
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

    // 2〜3. 複製したCPへ折り線を引き戻して挿入する(原子性: 成功するまで元のcpは触らない)
    let mut work = cp.clone();
    let mut added: Vec<EdgeId> = Vec::new();
    let mut driver_lines: Vec<DriverLine> = Vec::new();
    let mut warned_overlap: HashSet<EdgeId> = HashSet::new();
    let mut promoted_aux = 0usize;
    let mut crossed_any = false;
    // 頂点座標の引き当て表。workを書き換えた直後にだけ作り直す。
    let mut wvpos = vertex_positions(&work);
    for f in faces.iter().filter(|f| target_ids.contains(&f.id)) {
        let pl = &state.placements[&f.id];
        let inv = pl.inverse();
        let a = inv.apply(l0);
        let b = inv.apply(l1);
        let dir = (b - a).normalize(); // 等長変換なので長さは保たれ、正規化できる
        let poly = polygon(f);
        let n = poly.len();
        if n < 3 {
            continue;
        }

        // 引き戻した無限直線と面境界の交点を、直線上の弧長パラメータとして集める。
        let t_of = |q: DVec2| (q - a).dot(dir);
        let mut ts: Vec<f64> = Vec::new();
        for i in 0..n {
            let p0 = poly[i];
            let p1 = poly[(i + 1) % n];
            let s0 = dir.perp_dot(p0 - a);
            let s1 = dir.perp_dot(p1 - a);
            if s0.abs() <= EPS && s1.abs() <= EPS {
                // 境界辺が直線に沿っている: 両端が区切りになる
                ts.push(t_of(p0));
                ts.push(t_of(p1));
            } else if s0.abs() <= EPS {
                ts.push(t_of(p0));
            } else if s1.abs() <= EPS {
                ts.push(t_of(p1));
            } else if s0 * s1 < 0.0 {
                ts.push(t_of(p0 + (p1 - p0) * (s0 / (s0 - s1))));
            }
        }
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() <= EPS);

        // 3. 山谷の決定: Up=谷 / Down=山を基準に、裏返っている層では反転する。
        let base = match input.direction {
            FoldDirection::Up => EdgeKind::Valley,
            FoldDirection::Down => EdgeKind::Mountain,
        };
        let kind = if pl.mirrored { flip(base) } else { base };

        for w in ts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            if t1 - t0 <= EPS {
                continue;
            }
            let mid = a + dir * (0.5 * (t0 + t1));
            if !point_in_face(cp, f, [mid.x, mid.y]) {
                continue;
            }
            let q0 = a + dir * t0;
            let q1 = a + dir * t1;
            let on_boundary =
                (0..n).any(|i| dist_point_segment(mid, poly[i], poly[(i + 1) % n]) <= EPS);
            if on_boundary {
                // 面の縁に沿う区間は面を横切らないので線は引かない。ただし既存の
                // 山/谷線(スリット含む)に沿っている場合は、再生時にその断片群を
                // 駆動できるようDriverLineだけ生成する(角度は既存の線種に従う)。
                let edge_on_line = work.edges.iter().find_map(|e| {
                    if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                        return None;
                    }
                    let (p0, p1) = (wvpos.get(&e.v0)?, wvpos.get(&e.v1)?);
                    let (o0, o1) = collinear_overlap(q0, q1, *p0, *p1)?;
                    ((o1 - o0).length() > EPS).then_some((e.id, e.kind))
                });
                if let Some((eid, k)) = edge_on_line {
                    push_driver_line(&mut driver_lines, q0, q1, angle_of(k));
                    // 折り線の一部が反対向きの既存の折り目に乗っている状態。平坦
                    // (±180°)では見分けが付かないが、折り途中の角度では山と谷が
                    // 打ち消し合って形が求まらないため知らせる。
                    if k != kind && warned_overlap.insert(eid) {
                        warnings.push(opposite_crease_warning(eid));
                    }
                }
                continue;
            }
            crossed_any = true;
            push_driver_line(&mut driver_lines, q0, q1, angle_of(kind));
            // 既存辺と重なる区間の扱い: 補助線は挿入後に折りの線種へ昇格させる。
            // 既存の山/谷線はinsert_segmentが線種を維持する(食い違いは警告)。
            // 折り線と同一直線上の山/谷辺は必ず面の境界になるため、通常はこの分岐
            // ではなく上のon_boundary分岐で検出される(ここは面が壊れた場合の防御)。
            let mut has_aux_overlap = false;
            {
                for e in &work.edges {
                    let (Some(&p0), Some(&p1)) = (wvpos.get(&e.v0), wvpos.get(&e.v1)) else {
                        continue;
                    };
                    let Some((o0, o1)) = collinear_overlap(q0, q1, p0, p1) else {
                        continue;
                    };
                    if (o1 - o0).length() <= EPS {
                        continue;
                    }
                    if e.kind == EdgeKind::Aux {
                        has_aux_overlap = true;
                    } else if e.kind != kind && warned_overlap.insert(e.id) {
                        warnings.push(opposite_crease_warning(e.id));
                    }
                }
            }
            added.extend(insert_segment(&mut work, [q0.x, q0.y], [q1.x, q1.y], kind));
            wvpos = vertex_positions(&work); // 挿入で頂点が増えたので作り直す
            // 重なった補助線はinsert_segmentが重なりの端で分割済みなので、
            // 区間内に収まる断片を折りの線種へ昇格させる(面が正しく分割されるようにする)。
            if has_aux_overlap {
                for e in work.edges.iter_mut() {
                    if e.kind == EdgeKind::Aux
                        && let (Some(&p0), Some(&p1)) = (wvpos.get(&e.v0), wvpos.get(&e.v1))
                        && (p1 - p0).length() >= EPS
                        && point_on_segment(p0, q0, q1)
                        && point_on_segment(p1, q0, q1)
                    {
                        e.kind = kind;
                        added.push(e.id);
                        promoted_aux += 1;
                    }
                }
            }
        }
    }
    if !crossed_any {
        return Err(
            "折り線がどの層の面も横切っていません(既存の折り線での再折りには対応していません)"
                .to_string(),
        );
    }
    if promoted_aux > 0 {
        warnings.push(format!(
            "折り線と重なっていた補助線{promoted_aux}本を折り線に変更しました"
        ));
    }

    added.sort_unstable();
    added.dedup();
    // 後続の挿入で分割されて消えた辺IDを取り除く(通常は起きない防御)。
    added.retain(|id| work.edges.iter().any(|e| e.id == *id));

    // 4〜5. 面を再抽出し、代表点で親面を特定して新しい配置を決める。
    let new_faces = extract_faces(&work);
    // 昇格処理は線種しか変えないので、挿入直後に作り直した引き当て表をそのまま使える。
    let wpos = wvpos;
    let refl = Isometry2::reflection(l0, l1);
    let order_index: HashMap<FaceId, usize> = state
        .order
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i))
        .collect();

    let mut placements: HashMap<FaceId, Isometry2> = HashMap::with_capacity(new_faces.len());
    // (旧orderでの親の位置, 新しい面ID, 動くか)
    let mut infos: Vec<(usize, FaceId, bool)> = Vec::with_capacity(new_faces.len());
    for nf in &new_faces {
        let r = representative_point(&work, nf);
        let parent = faces.iter().find(|f| point_in_face(cp, f, r));
        let (parent_pl, parent_order, moving) = match parent {
            Some(pf) => {
                let ppl = state.placements[&pf.id];
                let idx = order_index.get(&pf.id).copied().unwrap_or(usize::MAX);
                let is_target = target_ids.contains(&pf.id);
                if is_target {
                    // 防御: 対象面の子が折り線を挟んで両側に頂点を持つ場合、面の分割に
                    // 失敗している(このまま進めると面全体が誤って反転し得る)。
                    let mut on_keep = false;
                    let mut on_move = false;
                    for vid in &nf.vertices {
                        if let Some(&p) = wpos.get(vid) {
                            let d = signed_dist(ppl.apply(p));
                            if d > EPS {
                                on_keep = true;
                            } else if d < -EPS {
                                on_move = true;
                            }
                        }
                    }
                    if on_keep && on_move {
                        return Err(
                            "折り線が面を横切っているのに面を分割できませんでした。折り線と重なる線の近くの展開図を確認してください"
                                .to_string(),
                        );
                    }
                }
                let moving = is_target && signed_dist(ppl.apply(DVec2::from(r))) < -EPS;
                (ppl, idx, moving)
            }
            None => {
                warnings.push(format!(
                    "新しい面 {} の親面が特定できないため、動かさず元の配置のままにします",
                    nf.id
                ));
                (Isometry2::identity(), usize::MAX, false)
            }
        };
        placements.insert(nf.id, if moving { refl.compose(&parent_pl) } else { parent_pl });
        infos.push((parent_order, nf.id, moving));
    }

    // 6. 新しい層順序: 動かない面は親の順序を維持。動いた面は親の順序を保って取り出し、
    //    反転して山全体の上(Up)または下(Down)に入れる。
    let mut keep_faces: Vec<(usize, FaceId)> = infos
        .iter()
        .filter(|&&(_, _, m)| !m)
        .map(|&(i, id, _)| (i, id))
        .collect();
    keep_faces.sort_unstable();
    let mut moving_faces: Vec<(usize, FaceId)> = infos
        .iter()
        .filter(|&&(_, _, m)| m)
        .map(|&(i, id, _)| (i, id))
        .collect();
    moving_faces.sort_unstable();
    moving_faces.reverse();
    let order: Vec<FaceId> = match input.direction {
        FoldDirection::Up => keep_faces
            .iter()
            .chain(moving_faces.iter())
            .map(|&(_, id)| id)
            .collect(),
        FoldDirection::Down => moving_faces
            .iter()
            .chain(keep_faces.iter())
            .map(|&(_, id)| id)
            .collect(),
    };

    // 8. 紙が裂ける指定の検出: 動く面と動かない面をつなぐ辺が折り線上に無い場合は警告。
    let moving_of: HashMap<FaceId, bool> = infos.iter().map(|&(_, id, m)| (id, m)).collect();
    let mut edge_faces: BTreeMap<EdgeId, Vec<FaceId>> = BTreeMap::new();
    for nf in &new_faces {
        let mut ids: Vec<EdgeId> = nf.edges.clone();
        ids.sort_unstable();
        ids.dedup();
        for eid in ids {
            edge_faces.entry(eid).or_default().push(nf.id);
        }
    }
    for (eid, fs) in &edge_faces {
        if fs.len() != 2 || moving_of[&fs[0]] == moving_of[&fs[1]] {
            continue;
        }
        let keep_id = if moving_of[&fs[0]] { fs[1] } else { fs[0] };
        let pl = placements[&keep_id];
        let Some(e) = work.edges.iter().find(|e| e.id == *eid) else {
            continue;
        };
        let (Some(&p0), Some(&p1)) = (wpos.get(&e.v0), wpos.get(&e.v1)) else {
            continue;
        };
        // 動かない側の配置で畳んだ平面へ写し、両端とも折り線上にあればヒンジとして正常。
        let d0 = u.perp_dot(pl.apply(p0) - l0).abs();
        let d1 = u.perp_dot(pl.apply(p1) - l0).abs();
        if d0 > EPS || d1 > EPS {
            warnings.push(format!(
                "動く面と動かない面をつなぐ辺(ID {eid})が折り線上に無いため、このままでは紙が裂けます(指定のまま続行します)"
            ));
        }
    }

    // 7. FoldStepの生成。driversは折り線のCP座標区間ごとのDriverLine
    //    (辺IDに依存しないため、後続の折りで辺が分割されても再生できる)。
    let new_state = FlatState { placements, order };
    let layer_points = new_state.to_layer_points(&work, &new_faces);
    let step = FoldStep {
        id: 0,
        kind: TechniqueKind::Simple,
        drivers: driver_lines,
        layer_order: Some(layer_points),
        note: String::new(),
    };

    *cp = work;
    Ok(FoldThroughResult {
        state: new_state,
        added_edges: added,
        step,
        warnings,
    })
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

fn vertex_positions(cp: &CreasePattern) -> HashMap<VertexId, DVec2> {
    cp.vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect()
}

/// 同じ線分(向きの違いは同一視)+同じ角度のDriverLineを重複させずに追加する。
/// 既存の折り目に沿う区間は、隣接する2面の引き戻しから同じ線分が2回出るため。
fn push_driver_line(lines: &mut Vec<DriverLine>, q0: DVec2, q1: DVec2, angle: f64) {
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

/// 折り線の一部が反対向きの既存の折り目に乗っている場合の警告文。
fn opposite_crease_warning(eid: EdgeId) -> String {
    format!(
        "折り線の一部に反対向きの折り線(山/谷)が既にあります(辺ID {eid})。折り上がりは同じですが、そのままでは折り途中の形が正しく表示されません"
    )
}

/// 線種に対応する完全折りの角度(+180=山, -180=谷)。
fn angle_of(kind: EdgeKind) -> f64 {
    match kind {
        EdgeKind::Mountain => 180.0,
        _ => -180.0,
    }
}

/// 山谷の反転(Border/Auxはそのまま)。
fn flip(kind: EdgeKind) -> EdgeKind {
    match kind {
        EdgeKind::Valley => EdgeKind::Mountain,
        EdgeKind::Mountain => EdgeKind::Valley,
        k => k,
    }
}
