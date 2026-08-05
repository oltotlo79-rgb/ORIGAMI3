//! 充填結果から展開図を組み立てる(PRO-002/PRO-003、要件§8-3・§8-4)。
//!
//! 手順は要件§8-3の通り。
//! 1. 円中心と紙の四隅を頂点にしたドロネー三角形分割で軸多角形を作る。
//!    紙の四隅を必ず含めるので、三角形の集まりが紙全体をちょうど覆う。
//! 2. 各多角形を簡易分子で埋める。三角形はウサギ耳分子、四角形以上は先頭の
//!    頂点から扇状に三角形へ割ってから同じ処理をする。
//! 3. 山谷は「軸線(三角形の辺)=谷、稜線(内心へ向かう二等分線)=山」の既定則。
//!
//! ウサギ耳分子は「3本の角の二等分線が内心で交わり、そこから1辺へ垂線
//! (耳のちょうつがい線)を下ろす」形にした。内心のまわりが山3・谷1になり、
//! 前川定理(山−谷=±2)と川崎定理(1つおきの角の和=180°)を同時に満たす。
//! 3辺すべてへ垂線を下ろす形も試したが、軸線の途中に3叉の点が大量にでき、
//! 川崎定理を満たしようがない頂点が増えて平坦折り違反が3〜5倍になったため
//! 採らなかった(同じ理由で、垂線の行き先は紙の縁に乗る辺を優先している)。
//!
//! 4. 局所平坦折り判定に掛け、違反頂点数を結果に載せる(失敗扱いにはしない)。
//!
//! 厳密な最適性や完全な平坦折り可能性は保証しない(要件§8の精度目標)。
//! 無理のある箇所は日本語の警告として伝え、生成そのものは続行する。

use ori3_cp::{insert_segment, local_violations, validate};
use ori3_model::{CreasePattern, Edge, EdgeKind, Vertex};
use serde::{Deserialize, Serialize};

use crate::packing::Packing;
use crate::skeleton::Skeleton;
use crate::triangulate::{dedup, index_of, triangulate};

/// 紙の縁に乗っているとみなす許容誤差。
const ON_EDGE_TOL: f64 = 1e-9;

/// 展開図の自動提案の結果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProposalResult {
    pub cp: CreasePattern,
    /// 局所平坦折り判定(CPE-009)で引っかかった頂点の数。0でなくても失敗ではない。
    pub violations: usize,
    /// 利用者へ見せる日本語の注意書き。
    pub warnings: Vec<String>,
}

/// 輪郭4辺だけが入ったCPを作る(左下(0,0)起点の反時計回り)。
fn border_cp(w: f64, h: f64) -> CreasePattern {
    let corners = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
    CreasePattern {
        vertices: corners
            .iter()
            .enumerate()
            .map(|(i, p)| Vertex {
                id: i as u32,
                pos: *p,
            })
            .collect(),
        edges: (0..4)
            .map(|i| Edge {
                id: i,
                v0: i,
                v1: (i + 1) % 4,
                kind: EdgeKind::Border,
            })
            .collect(),
        next_vertex_id: 4,
        next_edge_id: 4,
    }
}

/// 線分が紙の縁と重なっているか(重なる線分は既存のBorder辺なので引き直さない)。
fn on_paper_edge(p: [f64; 2], q: [f64; 2], w: f64, h: f64) -> bool {
    let same = |a: f64, b: f64, v: f64| (a - v).abs() < ON_EDGE_TOL && (b - v).abs() < ON_EDGE_TOL;
    same(p[0], q[0], 0.0) || same(p[0], q[0], w) || same(p[1], q[1], 0.0) || same(p[1], q[1], h)
}

/// 点pから線分abへ下ろした垂線の足(線分の外へは出さない)。
fn foot(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    if len2 <= 0.0 {
        return a;
    }
    let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0);
    [a[0] + ab[0] * t, a[1] + ab[1] * t]
}

/// 三角形をウサギ耳分子で埋める。`tri[i]` の対辺の長さを重みにすると内心が出る。
fn rabbit_ear(cp: &mut CreasePattern, tri: [[f64; 2]; 3], w: f64, h: f64) {
    let opp = |i: usize| {
        let (a, b) = (tri[(i + 1) % 3], tri[(i + 2) % 3]);
        (a[0] - b[0]).hypot(a[1] - b[1])
    };
    let len = [opp(0), opp(1), opp(2)];
    let sum = len[0] + len[1] + len[2];
    if sum <= 0.0 || !sum.is_finite() {
        return; // 潰れた三角形・非有限な座標
    }
    let incenter = [
        (len[0] * tri[0][0] + len[1] * tri[1][0] + len[2] * tri[2][0]) / sum,
        (len[0] * tri[0][1] + len[1] * tri[1][1] + len[2] * tri[2][1]) / sum,
    ];
    // 軸線(谷)。紙の縁と重なるものは既にBorder辺があるので引かない。
    for i in 0..3 {
        let (p, q) = (tri[i], tri[(i + 1) % 3]);
        if !on_paper_edge(p, q, w, h) {
            insert_segment(cp, p, q, EdgeKind::Valley);
        }
    }
    // 稜線(山): 各頂点から内心へ。二等分線に一致する。
    for p in tri {
        insert_segment(cp, p, incenter, EdgeKind::Mountain);
    }
    // ちょうつがい線(谷): 内心から1辺へ下ろす垂線。ウサギ耳分子の「耳」にあたる。
    // 内心のまわりは 山3 + 谷1 になり、前川定理(山−谷=2)と川崎定理を同時に満たす。
    // 下ろす先は「紙の縁に乗っている辺」を優先し、次に長い辺を選ぶ。紙の縁で
    // 終わるちょうつがい線は折りの妨げにならないが、内側の軸線の途中で終わると
    // その点が3叉になり平坦に折れなくなるため。
    let key = |i: usize| {
        let on_edge = on_paper_edge(tri[(i + 1) % 3], tri[(i + 2) % 3], w, h);
        (u8::from(on_edge), len[i])
    };
    let pick = (0..3)
        .max_by(|&a, &b| {
            let (x, y) = (key(a), key(b));
            x.0.cmp(&y.0).then(x.1.total_cmp(&y.1))
        })
        .unwrap_or(0);
    let f = foot(incenter, tri[(pick + 1) % 3], tri[(pick + 2) % 3]);
    insert_segment(cp, incenter, f, EdgeKind::Valley);
}

/// 多角形を簡易分子で埋める。四角形以上は先頭の頂点から扇状に三角形へ割る。
fn fill_polygon(cp: &mut CreasePattern, poly: &[[f64; 2]], w: f64, h: f64) {
    for i in 1..poly.len().saturating_sub(1) {
        rabbit_ear(cp, [poly[0], poly[i], poly[i + 1]], w, h);
    }
}

/// 充填結果から展開図を組み立てる(要件§8-3・§8-4)。
///
/// 骨格や紙寸法が壊れているときだけ `Err`(日本語1文)を返す。幾何的に無理の
/// ある配置は失敗にせず、`warnings` に理由を入れて展開図を返す。
/// 同じ入力からは必ず同じCPが得られる(決定的)。
pub fn generate(
    skeleton: &Skeleton,
    packing: &Packing,
    paper_w: f64,
    paper_h: f64,
) -> Result<ProposalResult, String> {
    skeleton.validate()?;
    if !(paper_w > 0.0 && paper_h > 0.0 && paper_w.is_finite() && paper_h.is_finite()) {
        return Err("紙の寸法は正の有限値にしてください".to_string());
    }
    if packing.centers.is_empty() {
        return Err("充填結果に円の中心が1つも入っていません".to_string());
    }
    let mut warnings = Vec::new();

    // 円が紙をはみ出す角は、その角が想定より短くなりうることを伝える。
    for &(id, c) in &packing.centers {
        let r = skeleton.leaf_radius(id) * packing.scale;
        let out = c[0] - r < -ON_EDGE_TOL
            || c[0] + r > paper_w + ON_EDGE_TOL
            || c[1] - r < -ON_EDGE_TOL
            || c[1] + r > paper_h + ON_EDGE_TOL;
        if out {
            warnings.push(format!(
                "角{id}の円が紙からはみ出しています。この角は想定より短くなる可能性があります"
            ));
        }
    }

    // 軸多角形の頂点候補: 円中心を先に、紙の四隅を後に置いて重複をまとめる。
    let mut pts: Vec<[f64; 2]> = packing.centers.iter().map(|&(_, c)| c).collect();
    pts.extend([
        [0.0, 0.0],
        [paper_w, 0.0],
        [paper_w, paper_h],
        [0.0, paper_h],
    ]);
    let pts = dedup(&pts);
    let tris = triangulate(&pts);
    if tris.is_empty() {
        warnings.push(
            "円の中心と紙の角が一直線に並んでいるため、折り線を作れませんでした".to_string(),
        );
    }

    let mut cp = border_cp(paper_w, paper_h);
    for t in &tris {
        fill_polygon(&mut cp, &[pts[t[0]], pts[t[1]], pts[t[2]]], paper_w, paper_h);
    }

    // 三角形の頂点として使われなかった円中心があれば知らせる。
    for &(id, c) in &packing.centers {
        let used = index_of(&pts, c)
            .is_some_and(|i| tris.iter().any(|t| t.contains(&i)));
        if !used {
            warnings.push(format!(
                "角{id}の位置が他の角と重なっているため、専用の折り線を作れませんでした"
            ));
        }
    }

    let violations = local_violations(&cp).len();
    if violations > 0 {
        warnings.push(format!(
            "平らに折りたたむ条件を満たさない点が{violations}個あります。そのまま提示しますので、必要に応じて手直ししてください"
        ));
    }
    for w in validate(&cp) {
        warnings.push(format!("展開図の点検: {w}"));
    }
    Ok(ProposalResult {
        cp,
        violations,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 四角形の軸多角形を扇状分割で埋められること(ドロネー分割は三角形しか
    /// 返さないので、この経路はここで確認しておく)。
    #[test]
    fn quad_is_filled_by_fanning_into_triangles() {
        let mut cp = border_cp(1.0, 1.0);
        let quad = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        fill_polygon(&mut cp, &quad, 1.0, 1.0);
        // 扇状分割で2つの三角形になり、それぞれに内心ができる。
        assert!(ori3_cp::extract_faces(&cp).len() >= 4);
        assert!(ori3_cp::validate(&cp).is_empty());
        assert!(cp.edges.iter().any(|e| e.kind == EdgeKind::Mountain));
    }

    /// 頂点が2つ以下の多角形は何もしない(潰れた入力で落ちないこと)。
    #[test]
    fn degenerate_polygon_is_ignored() {
        let mut cp = border_cp(1.0, 1.0);
        let before = cp.edges.len();
        fill_polygon(&mut cp, &[[0.0, 0.0], [1.0, 1.0]], 1.0, 1.0);
        fill_polygon(&mut cp, &[], 1.0, 1.0);
        assert_eq!(cp.edges.len(), before);
    }
}
