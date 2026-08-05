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

    incident
        .iter()
        .filter(|(v, edges)| !on_border.contains(v) && edges.len() >= 2)
        .filter(|(_, edges)| !maekawa_ok(edges) || !kawasaki_ok(edges))
        .map(|(v, _)| *v)
        .collect()
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
