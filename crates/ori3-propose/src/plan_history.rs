//! 折り順の探し方 その1「生成履歴を使う専用の計画」(作業18の比較用の試作)。
//!
//! 入力は展開図と、提案が展開図を作ったときの追跡情報([`FoldPlanTrace`])である。
//! 分子1つが引く折り線は 稜線(角→内心)・ちょうつがい線(内心→辺)・軸線(三角形の辺)
//! の3種類しかなく、**それぞれ折る順番の役割がはっきり決まっている**。
//!
//! ## 「次に折れる手」をどう決めるか
//!
//! | 折り線の役目 | 折れるようになる条件 |
//! |---|---|
//! | 稜線 | その分子が「手を付け始められる」状態(下記)であること |
//! | ちょうつがい線 | その分子の稜線をすべて折り終えていること |
//! | 軸線 | その軸線を共有する**両方**の分子で、稜線とちょうつがい線を折り終えていること |
//!
//! 「手を付け始められる」は、分子0か、**稜線とちょうつがい線を折り終えた分子と
//! 隣り合っている**ことである。1か所から外へ広げていく折り方に対応し、
//! ばらばらの場所を同時に折り始める順番を数えない。
//!
//! 折り線のまとまりが2つ以上の役目を兼ねることがある(別々の分子の稜線が
//! 一直線に繋がった場合など)。そのときは**すべての条件を満たしたときだけ**折れる。
//!
//! ## この方式が使えない相手
//!
//! 追跡情報が無い展開図(利用者が自分で描いたもの、既存の作品を読み込んだもの)では
//! 役目が1つも付かないため、**手を1つも列挙できない**。作業18ではその点も測る。
//!
//! ## 意図的にゆるめたところ(そのまま製品にはできない)
//!
//! 紙の重なり順・めり込み・途中の姿勢は一切見ていない。ここで数えた手は
//! **上限側の見積もり**である。

use ori3_model::CreasePattern;

use crate::plan::{
    CreaseLine, FoldedMask, SearchLimits, SearchStats, crease_lines, line_of_edge, search,
    search_breadth,
};
use crate::trace::{CreaseRole, FoldPlanTrace};

/// 折り線の追跡情報と展開図の辺が同じ線分に乗っているとみなす許容誤差。
const TOUCH_TOL: f64 = 1e-9;

/// 生成履歴(追跡情報)を使って「次に折れる手」を列挙する試作。
#[derive(Clone, Debug)]
pub struct HistoryPlanner {
    lines: Vec<CreaseLine>,
    /// 折り線のまとまりごとの役目(分子番号と役目。昇順・重複なし)。
    roles: Vec<Vec<(usize, CreaseRole)>>,
    /// 分子ごとの稜線のまとまり。
    ridges: Vec<FoldedMask>,
    /// 分子ごとのちょうつがい線のまとまり。
    hinges: Vec<FoldedMask>,
    /// 分子ごとの隣(角を1つでも共有する相手)。
    neighbors: Vec<Vec<usize>>,
}

impl HistoryPlanner {
    /// 展開図と追跡情報から作る。
    #[must_use]
    pub fn new(cp: &CreasePattern, trace: &FoldPlanTrace) -> Self {
        let lines = crease_lines(cp);
        let of_edge = line_of_edge(&lines);
        let pos: std::collections::BTreeMap<u32, [f64; 2]> =
            cp.vertices.iter().map(|v| (v.id, v.pos)).collect();

        let mut roles: Vec<Vec<(usize, CreaseRole)>> = vec![Vec::new(); lines.len()];
        for e in &cp.edges {
            let (Some(&line), Some(&p), Some(&q)) =
                (of_edge.get(&e.id), pos.get(&e.v0), pos.get(&e.v1))
            else {
                continue;
            };
            for m in &trace.molecules {
                for c in &m.creases {
                    if on_segment(p, c.a, c.b) && on_segment(q, c.a, c.b) {
                        roles[line].push((m.index, c.role));
                    }
                }
            }
        }
        for r in &mut roles {
            r.sort_unstable_by_key(|&(m, role)| (m, role_key(role)));
            r.dedup();
        }

        let count = trace.molecules.len();
        let mut ridges: Vec<FoldedMask> = vec![0; count];
        let mut hinges: Vec<FoldedMask> = vec![0; count];
        for (line, list) in roles.iter().enumerate() {
            for &(m, role) in list {
                match role {
                    CreaseRole::Ridge => ridges[m] |= 1 << line,
                    CreaseRole::Hinge => hinges[m] |= 1 << line,
                    CreaseRole::Axial => {}
                }
            }
        }

        let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); count];
        for pair in &trace.neighbors {
            if pair.a < count && pair.b < count {
                neighbors[pair.a].push(pair.b);
                neighbors[pair.b].push(pair.a);
            }
        }
        for n in &mut neighbors {
            n.sort_unstable();
            n.dedup();
        }

        Self {
            lines,
            roles,
            ridges,
            hinges,
            neighbors,
        }
    }

    /// 役目が1つも付かなかったまとまりの数。追跡情報が無い展開図では全部になる。
    #[must_use]
    pub fn lines_without_role(&self) -> usize {
        self.roles.iter().filter(|r| r.is_empty()).count()
    }

    /// 2つ以上の役目を兼ねているまとまりの数。
    #[must_use]
    pub fn lines_with_many_roles(&self) -> usize {
        self.roles.iter().filter(|r| r.len() >= 2).count()
    }

    fn done(&self, m: usize, state: FoldedMask) -> bool {
        ((self.ridges[m] | self.hinges[m]) & !state) == 0
    }

    fn open(&self, m: usize, state: FoldedMask) -> bool {
        m == 0 || self.neighbors[m].iter().any(|&n| self.done(n, state))
    }

    /// まだ折っていないまとまりのうち、この状態で折れるものを番号順に返す。
    #[must_use]
    pub fn next_moves(&self, state: FoldedMask) -> Vec<FoldedMask> {
        (0..self.lines.len())
            .filter(|&i| (state & (1 << i)) == 0)
            .filter(|&i| {
                let list = &self.roles[i];
                !list.is_empty()
                    && list.iter().all(|&(m, role)| match role {
                        CreaseRole::Ridge => self.open(m, state),
                        CreaseRole::Hinge => (self.ridges[m] & !state) == 0,
                        CreaseRole::Axial => self.done(m, state),
                    })
            })
            .map(|i| 1 << i)
            .collect()
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

fn role_key(role: CreaseRole) -> u8 {
    match role {
        CreaseRole::Axial => 0,
        CreaseRole::Ridge => 1,
        CreaseRole::Hinge => 2,
    }
}

/// 点が線分の上に乗っているか(端点を含む)。
fn on_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len = ab[0].hypot(ab[1]);
    if len <= TOUCH_TOL {
        return (p[0] - a[0]).hypot(p[1] - a[1]) <= TOUCH_TOL;
    }
    let cross = ((p[0] - a[0]) * ab[1] - (p[1] - a[1]) * ab[0]).abs() / len;
    if cross > TOUCH_TOL {
        return false;
    }
    let t = ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / (len * len);
    (-TOUCH_TOL..=1.0 + TOUCH_TOL).contains(&t)
}
