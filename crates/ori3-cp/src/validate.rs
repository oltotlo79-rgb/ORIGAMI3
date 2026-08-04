//! CPの不変条件の検査(壊れた状態の検出)。

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec2;
use ori3_geometry::{collinear_overlap, seg_intersection};
use ori3_model::{CreasePattern, EPS, EdgeId, EdgeKind, VertexId};

/// CPの不変条件を検査し、違反を日本語の警告文字列で返す(空Vec=問題なし)。
/// UIの警告表示(「止めずに警告」原則)に接続する想定で、検査だけを行いCPは変更しない。
///
/// 検査項目:
/// - 存在しない頂点を参照している辺(参照切れ)がないか。
/// - 長さがほぼゼロ(EPS未満、v0==v1を含む)の退化辺がないか
///   (move_vertexで潰れた辺の検出)。
/// - 折り辺(Border/Mountain/Valley)のグラフが複数の連結成分に分かれていないか。
///   外周と連結していない浮いた線・閉ループがあると面抽出が信頼できない
///   (extract_facesの既知の制限を参照)。参照切れ辺・退化辺はグラフに数えない
///   (extract_facesも同じ辺を面抽出から除外するため、両者の判定基準は一致する。
///   潰れた橋辺で繋がって見える不一致もこれで防ぐ)。
/// - 辺同士が端点以外で交差・重なりしていないか(move_vertex後の破れ検出。
///   全辺ペアの総当たりで、重なり判定を交差判定より先に問い合わせる)。
///
/// 結果の順序は入力CPに対して決定的(辺ID順の走査に基づく)。
pub fn validate(cp: &CreasePattern) -> Vec<String> {
    let mut warnings = Vec::new();

    let vpos: BTreeMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();

    // --- 参照切れ辺・退化辺(辺ID順) ---
    let mut edges_by_id: Vec<(EdgeId, VertexId, VertexId, EdgeKind)> = cp
        .edges
        .iter()
        .map(|e| (e.id, e.v0, e.v1, e.kind))
        .collect();
    edges_by_id.sort_by_key(|&(id, _, _, _)| id);
    // 参照切れ・退化を除いた健全な辺(連結性判定はextract_facesとこの辺集合を共有する)
    let mut sound: Vec<(EdgeId, VertexId, VertexId, EdgeKind)> = Vec::new();
    for &(id, v0, v1, kind) in &edges_by_id {
        let (Some(&p0), Some(&p1)) = (vpos.get(&v0), vpos.get(&v1)) else {
            warnings.push(format!("辺{id}が存在しない点を参照している"));
            continue;
        };
        if v0 == v1 || (p1 - p0).length() < EPS {
            warnings.push(format!("辺{id}の長さがほぼゼロになっている(潰れた線)"));
            continue;
        }
        sound.push((id, v0, v1, kind));
    }

    // --- 折り辺グラフの連結性 ---
    let mut adj: BTreeMap<VertexId, Vec<VertexId>> = BTreeMap::new();
    for &(_, v0, v1, _) in sound.iter().filter(|&&(_, _, _, k)| k != EdgeKind::Aux) {
        adj.entry(v0).or_default().push(v1);
        adj.entry(v1).or_default().push(v0);
    }
    let mut seen: BTreeSet<VertexId> = BTreeSet::new();
    let mut components = 0u32;
    for &start in adj.keys() {
        if !seen.insert(start) {
            continue;
        }
        components += 1;
        let mut stack = vec![start];
        while let Some(u) = stack.pop() {
            for &w in &adj[&u] {
                if seen.insert(w) {
                    stack.push(w);
                }
            }
        }
    }
    if components > 1 {
        warnings.push(format!(
            "折り線のグラフが{components}個のまとまりに分かれている(外周と繋がらない浮いた線や閉ループがあり、面の検出結果が信頼できない)"
        ));
    }

    // --- 辺同士の交差・重なり(端点接触は許容。参照切れ辺は上で警告済みのため除外) ---
    let segs: Vec<(EdgeId, DVec2, DVec2)> = edges_by_id
        .iter()
        .filter(|(_, v0, v1, _)| vpos.contains_key(v0) && vpos.contains_key(v1))
        .map(|&(id, v0, v1, _)| (id, vpos[&v0], vpos[&v1]))
        .collect();
    let at_end = |p: DVec2, a: DVec2, b: DVec2| (p - a).length() <= EPS || (p - b).length() <= EPS;
    for i in 0..segs.len() {
        for j in (i + 1)..segs.len() {
            let (id1, a0, a1) = segs[i];
            let (id2, b0, b1) = segs[j];
            // ほぼ重なった2線分では両方がSomeを返し得るため、重なり判定を先に問い合わせる。
            if let Some((q0, q1)) = collinear_overlap(a0, a1, b0, b1) {
                // 長さ0の接触は両辺の端点同士の接触なので許容。区間の重なりは破れ。
                if (q1 - q0).length() > EPS {
                    warnings.push(format!("辺{id1}と辺{id2}が端点以外で重なっている"));
                }
            } else if let Some(p) = seg_intersection(a0, a1, b0, b1)
                && !(at_end(p, a0, a1) && at_end(p, b0, b1))
            {
                warnings.push(format!(
                    "辺{id1}と辺{id2}が端点以外で交差している(位置 ({:.4}, {:.4}))",
                    p.x, p.y
                ));
            }
        }
    }

    warnings
}
