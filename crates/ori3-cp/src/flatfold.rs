//! 局所平坦折り判定(CPE-009): 内部の折り目の集まる点ごとに、平らに畳めるための
//! 必要条件を検査する。
//!
//! 検査する2つの条件(いずれも「その点のまわりだけ」を見る局所条件):
//! - 前川定理: 山の本数 − 谷の本数 = ±2
//! - 川崎定理: 点のまわりの角を一周順に並べ、1つおきに足した和 = 180°
//!
//! 判定は検査だけを行いCPは変更しない(「止めずに警告」原則: 違反していても
//! 操作は止めず、画面で色を変えて知らせるだけ)。
//!
//! 対象は「内部の点」= 紙の縁(Border)に接していない、折り目が2本以上集まる点。
//! 補助線(Aux)は折りに関与しないので数えない。参照切れ・長さゼロの辺も数えない
//! (validate が別途警告する)。
//!
//! 局所条件は必要条件であって十分条件ではない(通しても紙が重なりで畳めない
//! 場合はある)。層の重なりの判定は ori3-layers 側の仕事。

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec2;
use ori3_model::{CreasePattern, EPS, EdgeKind, VertexId};

/// 角の和の許容誤差(ラジアン)。座標は長辺=1.0に正規化され、折り線の端点は
/// 既存頂点へEPS吸着されるため、正しい図形なら誤差は丸め程度に収まる。
const ANGLE_TOL: f64 = 1e-6;

/// 「1本の曲線の途中」とみなす曲がりの上限(ラジアン、45°)。
/// これを超える折れ方は曲線の近似ではなく意図した折れ曲がりなので、
/// 印をまとめず1点ずつ知らせる。
const MAX_CURVE_BEND: f64 = std::f64::consts::PI / 4.0;

/// 平らに畳めない疑いのある内部頂点のIDを昇順で返す(空=問題なし)。
/// 結果は入力CPに対して決定的(BTreeMapの昇順走査に基づく)。
#[must_use]
pub fn local_violations(cp: &CreasePattern) -> Vec<VertexId> {
    let vpos: BTreeMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();

    // 頂点ごとの「出ていく折り目」(方向ベクトルと種類)と、紙の縁に接する頂点
    let mut incident: BTreeMap<VertexId, Vec<(DVec2, EdgeKind)>> = BTreeMap::new();
    let mut on_border: BTreeSet<VertexId> = BTreeSet::new();
    for e in &cp.edges {
        if e.kind == EdgeKind::Aux {
            continue; // 補助線は折りに関与しない
        }
        let (Some(&p0), Some(&p1)) = (vpos.get(&e.v0), vpos.get(&e.v1)) else {
            continue; // 参照切れ(validateが警告する)
        };
        if (p1 - p0).length() < EPS {
            continue; // 潰れた線(validateが警告する)
        }
        if e.kind == EdgeKind::Border {
            on_border.insert(e.v0);
            on_border.insert(e.v1);
        }
        incident.entry(e.v0).or_default().push((p1 - p0, e.kind));
        incident.entry(e.v1).or_default().push((p0 - p1, e.kind));
    }

    let violating: Vec<VertexId> = incident
        .iter()
        .filter(|(v, edges)| !on_border.contains(v) && edges.len() >= 2)
        .filter(|(_, edges)| !maekawa_ok(edges) || !kawasaki_ok(edges))
        .map(|(v, _)| *v)
        .collect();
    collapse_curve_chains(cp, &incident, &on_border, violating)
}

/// 曲線の折り目の途中の点か。
///
/// その点に集まる折り目のうち「1本につながって見える組」(同じ線種で向きが
/// ほぼ正反対)を探し、その曲がりがわずかなら1本の曲線の途中とみなす。
/// 曲がりが0(まっすぐ)なら普通の交点なので数えない(まっすぐな折り目が
/// 並んだだけの点まで間引いてしまわないため)。曲がりが[`MAX_CURVE_BEND`]を
/// 超える折れ方は曲線ではなく「折れ曲がり」なので、その点は個別に知らせる。
fn is_curve_vertex(edges: &[(DVec2, EdgeKind)]) -> bool {
    let mut best = f64::MAX;
    for (i, (d0, k0)) in edges.iter().enumerate() {
        for (d1, k1) in &edges[i + 1..] {
            if k0 != k1 {
                continue;
            }
            // 2本のなす角と180°との差 = この点での折れ線の曲がり。
            // ぴったりまっすぐな組(別の折り目が通り抜けているだけ)は数えない
            let bend = (std::f64::consts::PI - d0.angle_to(*d1).abs()).abs();
            if bend > ANGLE_TOL {
                best = best.min(bend);
            }
        }
    }
    best <= MAX_CURVE_BEND
}

/// 一続きの曲線の折り目については、印を1つだけ残す。
///
/// 曲線は細かい折れ線として入るため、そのままだと1本の曲線に数十個の橙色の丸が
/// 並んで画面が読めなくなる。問題を隠さないよう「1本につき1つ」は必ず残し、
/// 数だけ減らす(曲線以外の点は1つも間引かない)。
fn collapse_curve_chains(
    cp: &CreasePattern,
    incident: &BTreeMap<VertexId, Vec<(DVec2, EdgeKind)>>,
    on_border: &BTreeSet<VertexId>,
    violating: Vec<VertexId>,
) -> Vec<VertexId> {
    let curve: BTreeSet<VertexId> = incident
        .iter()
        .filter(|(v, e)| !on_border.contains(v) && is_curve_vertex(e))
        .map(|(v, _)| *v)
        .collect();
    if curve.is_empty() {
        return violating;
    }
    // 曲線の途中の点から折り目1本でつながる先までを「その曲線のまわり」とみなす。
    // 曲がるための線の行き止まり(曲線から出て隣の折り目に突き当たる点)も
    // 曲線があるせいでできた点なので、同じまとまりに入れる
    let mut adj: BTreeMap<VertexId, Vec<VertexId>> = BTreeMap::new();
    for e in &cp.edges {
        if e.kind == EdgeKind::Aux || !(curve.contains(&e.v0) || curve.contains(&e.v1)) {
            continue;
        }
        adj.entry(e.v0).or_default().push(e.v1);
        adj.entry(e.v1).or_default().push(e.v0);
    }
    // 連結成分ごとに、違反している点のうち先頭の1つだけを残す
    let violating_set: BTreeSet<VertexId> = violating.iter().copied().collect();
    let mut seen: BTreeSet<VertexId> = BTreeSet::new();
    let mut drop: BTreeSet<VertexId> = BTreeSet::new();
    for &start in &curve {
        if !seen.insert(start) {
            continue;
        }
        let mut stack = vec![start];
        let mut group = vec![start];
        while let Some(v) = stack.pop() {
            for &w in adj.get(&v).map(Vec::as_slice).unwrap_or(&[]) {
                if seen.insert(w) {
                    group.push(w);
                    // 広げていくのは曲線の途中の点だけ(隣は行き止まりとして入れる)
                    if curve.contains(&w) {
                        stack.push(w);
                    }
                }
            }
        }
        // 曲線とみなすのは、曲がった点が3つ以上つながっているときだけ。
        // たまたま少し曲がって見える点1つ2つで、まわりの印まで消さないための歯止め
        if group.iter().filter(|v| curve.contains(v)).count() < 3 {
            continue;
        }
        group.sort_unstable();
        let mut hit = group.iter().filter(|v| violating_set.contains(v));
        hit.next(); // 先頭の1つは残す
        drop.extend(hit);
    }
    violating.into_iter().filter(|v| !drop.contains(v)).collect()
}

/// 前川定理: 山の本数 − 谷の本数 = ±2。
/// 内部頂点には縁の辺が来ないので、数えるのは山と谷だけになる。
fn maekawa_ok(edges: &[(DVec2, EdgeKind)]) -> bool {
    let count = |k: EdgeKind| edges.iter().filter(|(_, ek)| *ek == k).count();
    count(EdgeKind::Mountain).abs_diff(count(EdgeKind::Valley)) == 2
}

/// 川崎定理: 一周の角を1つおきに足した和が180°(= 残り半分も180°)。
/// 折り目の本数が奇数なら1つおきに分けられないので、その時点で条件を満たさない。
fn kawasaki_ok(edges: &[(DVec2, EdgeKind)]) -> bool {
    if !edges.len().is_multiple_of(2) {
        return false;
    }
    let mut dirs: Vec<f64> = edges.iter().map(|(d, _)| d.y.atan2(d.x)).collect();
    dirs.sort_by(f64::total_cmp);
    // 隣り合う方向の間の角(最後の1つは一周して戻る角)を1つおきに足す
    let mut alt = 0.0;
    for i in (0..dirs.len()).step_by(2) {
        let next = dirs[(i + 1) % dirs.len()];
        let mut gap = next - dirs[i];
        if gap < 0.0 {
            gap += std::f64::consts::TAU;
        }
        alt += gap;
    }
    (alt - std::f64::consts::PI).abs() <= ANGLE_TOL
}
