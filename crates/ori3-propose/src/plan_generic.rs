//! 折り順の探し方 その2「任意の展開図の汎用探索」(作業18の比較用の試作)。
//!
//! 入力は**展開図だけ**である。提案が作った履歴も、分子の情報も使わない。
//! したがって利用者が自分で描いた展開図にもそのまま使える。
//!
//! ## 「次に折れる手」をどう決めるか
//!
//! 平らに折れる紙では、折り目が集まる点のまわりを見ると必ず
//! **「両隣より狭い(か同じ)角を、山と谷ではさんでいる組」**があり、
//! そこから先に畳んでいける。逆に、その組が1つも無い点は、その状態では
//! もう畳めない。この規則だけを使う。
//!
//! - 点に残っている折り目を、向きの順に一周並べる。
//! - 隣り合う2本ではさまれた角が両隣以下で、その2本の山谷が違えば、
//!   その2本は「今折れる」。
//! - 折り線のまとまりは、**通っているすべての点で「今折れる」ときだけ**列挙する。
//!
//! 紙の縁に触れている点は条件を課さない(縁は折らないので一周の条件が立たない)。
//!
//! ## 意図的にゆるめたところ(そのまま製品にはできない)
//!
//! 1. 上の規則は本来「2本を同時に畳む」ものだが、ここでは
//!    まとまり1本ずつ折る形にゆるめている。
//! 2. **どの点でも組が1つも作れない状態になったら、その点は妨げとして数えない。**
//!    そうしないと、平らに折り切れない点を持つ展開図(作業17の不具合A・B)で
//!    探索が最初の一手も出せず、比較の材料にならないため。ゆるめた点の数は
//!    [`GenericPlanner::relaxed_vertices`] で数えられる。
//! 3. 紙の重なり順・めり込みは一切見ていない。
//!
//! つまりここで数えた手は**上限側の見積もり**である。

use std::collections::BTreeSet;

use ori3_model::{CreasePattern, EdgeKind, VertexId};

use crate::plan::{
    CreaseLine, FoldedMask, SearchLimits, SearchStats, crease_lines, line_of_edge, search,
    search_breadth,
};

/// 角が「両隣以下」かを見るときの許容誤差(ラジアン)。
/// 展開図の座標は長辺=1.0で、折り線の端点は1e-9で吸着されるため、
/// 同じ角どうしの差は丸め程度に収まる。
const ANGLE_TOL: f64 = 1e-9;

/// 点に集まる折り目1本ぶん。
#[derive(Clone, Copy, Debug)]
struct Incident {
    /// その点から出ていく向き(ラジアン、-π..π)。
    angle: f64,
    kind: EdgeKind,
    /// 属する折り線のまとまりの番号。
    line: usize,
}

/// 展開図だけを見て「次に折れる手」を列挙する試作。
#[derive(Clone, Debug)]
pub struct GenericPlanner {
    lines: Vec<CreaseLine>,
    /// 紙の縁に触れていない点ごとの、そこに集まる折り目(向きの順)。
    vertices: Vec<Vec<Incident>>,
}

impl GenericPlanner {
    /// 展開図から作る。骨格も提案の履歴も要らない。
    #[must_use]
    pub fn new(cp: &CreasePattern) -> Self {
        let lines = crease_lines(cp);
        let of_edge = line_of_edge(&lines);
        let pos: std::collections::BTreeMap<VertexId, [f64; 2]> =
            cp.vertices.iter().map(|v| (v.id, v.pos)).collect();

        let mut on_border: BTreeSet<VertexId> = BTreeSet::new();
        for e in cp.edges.iter().filter(|e| e.kind == EdgeKind::Border) {
            on_border.insert(e.v0);
            on_border.insert(e.v1);
        }

        let mut by_vertex: std::collections::BTreeMap<VertexId, Vec<Incident>> =
            std::collections::BTreeMap::new();
        for e in &cp.edges {
            if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                continue;
            }
            let Some(&line) = of_edge.get(&e.id) else {
                continue;
            };
            let (Some(&p), Some(&q)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
                continue;
            };
            for (from, to, v) in [(p, q, e.v0), (q, p, e.v1)] {
                if on_border.contains(&v) {
                    continue;
                }
                let d = [to[0] - from[0], to[1] - from[1]];
                if !d[0].is_finite() || !d[1].is_finite() {
                    continue;
                }
                by_vertex.entry(v).or_default().push(Incident {
                    angle: d[1].atan2(d[0]),
                    kind: e.kind,
                    line,
                });
            }
        }

        let vertices = by_vertex
            .into_values()
            .filter(|list| list.len() >= 2)
            .map(|mut list| {
                list.sort_by(|a, b| a.angle.total_cmp(&b.angle).then(a.line.cmp(&b.line)));
                list
            })
            .collect();

        Self { lines, vertices }
    }

    /// まだ折っていないまとまりのうち、この状態で折れるものを番号順に返す。
    #[must_use]
    pub fn next_moves(&self, state: FoldedMask) -> Vec<FoldedMask> {
        let blocked = self.blocked_lines(state).0;
        (0..self.lines.len())
            .map(|i| 1 << i)
            .filter(|bit| (state & bit) == 0 && (blocked & bit) == 0)
            .collect()
    }

    /// この状態で、条件を課せる点が1つも無くなり規則をゆるめた点の数。
    #[must_use]
    pub fn relaxed_vertices(&self, state: FoldedMask) -> usize {
        self.blocked_lines(state).1
    }

    /// (折れないまとまりのビット, 規則をゆるめた点の数)
    fn blocked_lines(&self, state: FoldedMask) -> (FoldedMask, usize) {
        let mut blocked: FoldedMask = 0;
        let mut relaxed = 0usize;
        let mut rem: Vec<Incident> = Vec::new();
        let mut ready: Vec<bool> = Vec::new();
        for list in &self.vertices {
            rem.clear();
            rem.extend(
                list.iter()
                    .copied()
                    .filter(|i| (state & (1 << i.line)) == 0),
            );
            let n = rem.len();
            if n < 2 {
                continue; // 残り1本以下の点は、もう一周の条件を持たない
            }
            let gap = |i: usize| {
                let mut g = rem[(i + 1) % n].angle - rem[i].angle;
                if g < 0.0 {
                    g += std::f64::consts::TAU;
                }
                g
            };
            ready.clear();
            ready.resize(n, false);
            let mut any = false;
            for i in 0..n {
                let here = gap(i);
                let prev = gap((i + n - 1) % n);
                let next = gap((i + 1) % n);
                if here <= prev + ANGLE_TOL
                    && here <= next + ANGLE_TOL
                    && rem[i].kind != rem[(i + 1) % n].kind
                {
                    ready[i] = true;
                    ready[(i + 1) % n] = true;
                    any = true;
                }
            }
            if !any {
                relaxed += 1; // この点はもう畳めない。妨げとしては数えない(上の§2)
                continue;
            }
            for (i, ok) in ready.iter().enumerate() {
                if !ok {
                    blocked |= 1 << rem[i].line;
                }
            }
        }
        (blocked, relaxed)
    }

    /// 条件を課している点(紙の縁に触れず、折り目が2本以上集まる点)の数。
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// 折り順を探して、たどった状態の数・枝分かれの最大数・時間を測る。
    #[must_use]
    pub fn measure(&self, limits: SearchLimits) -> SearchStats {
        search(self.lines.len(), |s| self.next_moves(s), limits)
    }

    /// 最初の`depth`手までに行ける状態を全部数える(枝分かれの広さ)。
    #[must_use]
    pub fn measure_breadth(&self, depth: usize, limits: SearchLimits) -> SearchStats {
        search_breadth(self.lines.len(), |s| self.next_moves(s), depth, limits)
    }
}
