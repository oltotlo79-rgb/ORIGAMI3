//! 面抽出: half-edge構造を作り、各半辺から最左回りで面を辿る。

use std::collections::BTreeMap;

use glam::DVec2;
use ori3_model::{CreasePattern, EPS, EdgeId, EdgeKind, FaceId, VertexId};

/// 展開図の面。IDは抽出のたびに0から再採番される導出値。
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub id: FaceId,
    /// 面の境界を反時計回りに一周する頂点列。
    pub vertices: Vec<VertexId>,
    /// verticesと同順の境界辺列(`edges[i]` は `vertices[i]` から次の頂点への辺)。
    pub edges: Vec<EdgeId>,
}

/// half-edge構造を作り、各半辺から最左回りで面を辿る。外周面(符号付き面積が
/// 負になる時計回りの面)は除外する。ぶら下がり辺(片端が行き止まりの辺)は
/// 境界を両側から辿るスリットとして面の境界に現れる。
///
/// 補助線(`EdgeKind::Aux`)は折りに関与しないため面の境界には使わず、
/// Border/Mountain/Valley のみを対象とする。
///
/// 既知の制限: 外周と連結していない閉ループ(入れ子・穴)は面の穴として扱われず、
/// 領域が重複した面が返る(内側のループが囲む面と、それを含む外側の面の両方が
/// 返る)。Aux辺のみで外周に繋がるCPでも、面抽出はAuxを無視するため折り辺だけを
/// 見ると非連結になり同じ現象が発生し得る。この状態は validate 関数で検出できる。
///
/// 結果の面の順序とID採番は入力CPに対して決定的(辺ID順の走査に基づく)。
pub fn extract_faces(cp: &CreasePattern) -> Vec<Face> {
    let vpos: BTreeMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();

    // 対象辺: Aux・退化辺を除外し、辺ID順に整列(決定性のため)。
    let mut edges: Vec<(EdgeId, VertexId, VertexId)> = cp
        .edges
        .iter()
        .filter(|e| e.kind != EdgeKind::Aux && e.v0 != e.v1)
        .filter(|e| (vpos[&e.v1] - vpos[&e.v0]).length() >= EPS)
        .map(|e| (e.id, e.v0, e.v1))
        .collect();
    edges.sort_by_key(|&(id, _, _)| id);

    // half-edge: 2i が v0→v1、2i+1 が v1→v0。twinは h ^ 1。
    let half: Vec<(VertexId, VertexId, EdgeId)> = edges
        .iter()
        .flat_map(|&(id, v0, v1)| [(v0, v1, id), (v1, v0, id)])
        .collect();

    // 各頂点の出半辺を角度の昇順(反時計回り)に整列する。
    let mut outgoing: BTreeMap<VertexId, Vec<usize>> = BTreeMap::new();
    for (h, &(from, _, _)) in half.iter().enumerate() {
        outgoing.entry(from).or_default().push(h);
    }
    for (v, list) in outgoing.iter_mut() {
        let p = vpos[v];
        let angle = |h: usize| {
            let d = vpos[&half[h].1] - p;
            d.y.atan2(d.x)
        };
        list.sort_by(|&h1, &h2| angle(h1).total_cmp(&angle(h2)).then(h1.cmp(&h2)));
    }

    // 半辺 u→v の次: v において twin(v→u) から時計回りに次の半辺(最左回り)。
    let next: Vec<usize> = (0..half.len())
        .map(|h| {
            let twin = h ^ 1;
            let list = &outgoing[&half[h].1];
            let i = list
                .iter()
                .position(|&x| x == twin)
                .expect("twin半辺は必ず出半辺リストに含まれる");
            list[(i + list.len() - 1) % list.len()]
        })
        .collect();

    // 全半辺を辺ID順に走査して面の巡回を集める(採番はこの走査順で決定的)。
    let mut visited = vec![false; half.len()];
    let mut faces = Vec::new();
    for start in 0..half.len() {
        if visited[start] {
            continue;
        }
        let mut cycle = Vec::new();
        let mut h = start;
        loop {
            visited[h] = true;
            cycle.push(h);
            h = next[h];
            if h == start {
                break;
            }
        }
        // 符号付き面積(shoelace)。負=時計回り=外周面、0=スリット等の退化巡回。
        let area = 0.5
            * cycle
                .iter()
                .map(|&h| {
                    let p = vpos[&half[h].0];
                    let q = vpos[&half[h].1];
                    p.x * q.y - q.x * p.y
                })
                .sum::<f64>();
        if area <= EPS * EPS {
            continue;
        }
        faces.push(Face {
            id: faces.len() as FaceId,
            vertices: cycle.iter().map(|&h| half[h].0).collect(),
            edges: cycle.iter().map(|&h| half[h].2).collect(),
        });
    }
    faces
}
