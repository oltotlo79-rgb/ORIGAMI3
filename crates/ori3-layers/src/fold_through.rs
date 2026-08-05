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
//! - 折り線がどの面も横切らない指定はエラーにする。既存の折り線と完全に一致する
//!   「再折り」(新しい線を1本も引かない折り)もこの条件に該当し、未対応
//!   (折り目を開く・重なり順だけ変える動きは [`crate::flat_motion`] で表せる)。

use std::collections::{BTreeMap, HashMap};

use glam::DVec2;
use ori3_cp::Face;
use ori3_geometry::{Isometry2, point_on_segment, reflect_across_line};
use ori3_model::{
    CreasePattern, DriverLine, EPS, EdgeId, EdgeKind, FaceId, FoldStep, TechniqueKind, VertexId,
};

use crate::flat_motion::{
    FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform, run_motion,
};
use crate::flat_state::FlatState;

/// 折る向き(型定義は手順操作の引数として使うため [`ori3_model`] にある)。
pub use ori3_model::FoldDirection;

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

    // 可動側を示す点は、動かさない側の点を折り線で鏡映して作る(必ず反対側に来る)。
    let movable = reflect_across_line(keep, l0, l1);
    let motion = FlatMotionInput {
        parts: vec![MotionPart {
            layers: target_ids,
            region: vec![HalfPlane {
                line: input.line,
                inside_point: [movable.x, movable.y],
            }],
            transform: MotionTransform::Reflect(vec![input.line]),
            turn: LayerTurn::Outside(input.direction),
            reverse_layers: None,
        }],
        kind: TechniqueKind::Simple,
    };
    let out = run_motion(cp, faces, state, &motion)?;
    if !out.crossed_any {
        return Err(
            "折り線がどの層の面も横切っていません(既存の折り線での再折りには対応していません)"
                .to_string(),
        );
    }

    let mut result = out.result;
    warnings.append(&mut result.warnings);
    result.warnings = warnings;
    *cp = out.cp;
    Ok(result)
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
