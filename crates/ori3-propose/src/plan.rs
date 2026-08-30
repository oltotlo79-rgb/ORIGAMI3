//! 折り順の探し方を比べるための共通の土台(作業18)。
//!
//! **ここにあるのは比較のための試作であって、製品として使う計画器ではない。**
//! どちらの探し方を採るかは作業19でClaudeが決める。このモジュールが引き受けるのは
//! 「2つの探し方を、同じ標本・同じ数え方で比べられるようにする」ことだけである。
//!
//! ## 共通にしたもの
//!
//! | 共通にしたもの | 中身 |
//! |---|---|
//! | 手の単位 | [`CreaseLine`] 1本(1本の直線の上に並ぶ、同じ山谷の折り目のまとまり) |
//! | 状態 | どのまとまりを折り終えたか([`FoldedMask`] の1ビットが1本) |
//! | 終わり | 全部のまとまりを折り終えたとき |
//! | 探し方 | 折り線の番号の小さい順に進む深さ優先([`search`])。乱数は使わない |
//!
//! 2つの方式で違うのは **「今この状態から折れる手はどれか」を答える規則だけ**である。
//! 手の単位を [`CreaseLine`] にしたのは、手順の保存形式 `DriverLine` が
//! 「線分の上に乗る折り辺すべて」をまとめて動かす単位だからで、
//! ここで数えた手の数がそのまま `FoldStep` の本数に対応する。
//!
//! ## この土台では測れないこと
//!
//! 紙の重なり順・面のめり込み・途中の姿勢は一切見ていない。ここで「折れる手」と
//! 数えたものが実際に折れる保証はなく、**手の数は上限側の見積もり**である。
//! 実際に折れるかを確かめるには `ori3-layers` の折り操作が要る(作業21以降)。

use std::collections::{BTreeMap, BTreeSet};

use ori3_model::clock::Instant;
use ori3_model::{CreasePattern, EdgeId, EdgeKind, VertexId};

/// 同じ直線・同じ点とみなす許容誤差。展開図の座標は長辺=1.0に正規化されており、
/// 折り線の端点は生成時に1e-9で既存頂点へ吸着される。ここはそれより2桁ゆるい。
const LINE_TOL: f64 = 1e-7;

/// 状態を表すビット列。1ビットが [`CreaseLine`] 1本にあたる。
pub type FoldedMask = u128;

/// 1つの展開図で扱える折り線のまとまりの上限。状態を [`FoldedMask`] で表すため。
pub const MAX_LINES: usize = 128;

/// 一度に折る折り線のまとまり。
///
/// 1本の直線の上に並び、同じ山谷で、端どうしが繋がっている折り目を1つにまとめる。
/// 手順の保存形式 `DriverLine` が動かす範囲と同じ単位である。
#[derive(Clone, Debug, PartialEq)]
pub struct CreaseLine {
    /// 番号(端点の座標順。同じ展開図なら毎回同じ番号になる)。
    pub id: usize,
    pub kind: EdgeKind,
    /// このまとまりに入っている展開図の辺(昇順)。
    pub edges: Vec<EdgeId>,
    /// まとまり全体の端(`a` から `b` の向きは番号付けと同じ規則)。
    pub a: [f64; 2],
    pub b: [f64; 2],
}

/// 探索の打ち切り条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchLimits {
    /// たどってよい状態の数の上限。
    pub max_states: usize,
    /// かけてよい時間の上限(ミリ秒)。
    pub max_millis: u128,
}

/// 探索が止まった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// 全部の折り線を折り終える順番が見つかった。
    Completed,
    /// 最初の状態で折れる手が1つも無かった。
    NoFirstMove,
    /// 手を尽くしたが、全部を折り終える順番が見つからなかった。
    Exhausted,
    /// 状態の数の上限で打ち切った。
    StateCap,
    /// 時間の上限で打ち切った。
    TimeCap,
}

/// 1回の探索で測った値。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchStats {
    /// この展開図の折り線のまとまりの数。
    pub lines: usize,
    /// 最初の状態で列挙できた手の数。
    pub first_moves: usize,
    /// たどった状態の数(最初の状態を含む)。
    pub states: usize,
    /// 枝分かれの最大数(1つの状態から列挙した手の数の最大)。
    pub max_branching: usize,
    /// 見つかった折り順の手数。見つからなければ `None`。
    pub plan_len: Option<usize>,
    pub stop: StopReason,
    /// かかった時間(ミリ秒)。
    pub millis: f64,
}

impl SearchStats {
    /// 時間以外の値が同じか。決定性の確認に使う(時間だけは毎回変わる)。
    #[must_use]
    pub fn same_result(&self, other: &Self) -> bool {
        self.lines == other.lines
            && self.first_moves == other.first_moves
            && self.states == other.states
            && self.max_branching == other.max_branching
            && self.plan_len == other.plan_len
            && self.stop == other.stop
    }
}

/// 展開図の折り目を、一度に折るまとまりへ分ける。
///
/// 補助線と紙の縁は折らないので入れない。並びは端点の座標順で、同じ展開図なら
/// 毎回同じ番号が付く。
#[must_use]
pub fn crease_lines(cp: &CreasePattern) -> Vec<CreaseLine> {
    struct Group {
        dir: [f64; 2],
        off: f64,
        kind: EdgeKind,
        /// (区間の始まり, 区間の終わり, 辺ID)
        spans: Vec<(f64, f64, EdgeId)>,
    }

    let pos: BTreeMap<VertexId, [f64; 2]> = cp.vertices.iter().map(|v| (v.id, v.pos)).collect();
    let mut groups: Vec<Group> = Vec::new();
    let mut edges: Vec<(EdgeId, VertexId, VertexId, EdgeKind)> = cp
        .edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|e| (e.id, e.v0, e.v1, e.kind))
        .collect();
    edges.sort_unstable_by_key(|e| e.0);

    for (id, v0, v1, kind) in edges {
        let (Some(&p), Some(&q)) = (pos.get(&v0), pos.get(&v1)) else {
            continue; // 参照切れ(validateが警告する)
        };
        let d = [q[0] - p[0], q[1] - p[1]];
        let len = d[0].hypot(d[1]);
        if !len.is_finite() || len <= LINE_TOL {
            continue; // 潰れた辺・非有限な座標
        }
        let mut dir = [d[0] / len, d[1] / len];
        // 向きをそろえる(逆向きの同じ直線を別物にしないため)。
        if dir[0] < -LINE_TOL || (dir[0].abs() <= LINE_TOL && dir[1] < 0.0) {
            dir = [-dir[0], -dir[1]];
        }
        let off = dir[0] * p[1] - dir[1] * p[0];
        let along = |x: [f64; 2]| x[0] * dir[0] + x[1] * dir[1];
        let (t0, t1) = {
            let (a, b) = (along(p), along(q));
            (a.min(b), a.max(b))
        };
        let hit = groups.iter_mut().find(|g| {
            g.kind == kind
                && (g.dir[0] - dir[0]).abs() <= LINE_TOL
                && (g.dir[1] - dir[1]).abs() <= LINE_TOL
                && (g.off - off).abs() <= LINE_TOL
        });
        match hit {
            Some(g) => g.spans.push((t0, t1, id)),
            None => groups.push(Group {
                dir,
                off,
                kind,
                spans: vec![(t0, t1, id)],
            }),
        }
    }

    // 同じ直線の上でも、離れている折り目は別のまとまりにする。
    let mut out: Vec<CreaseLine> = Vec::new();
    for g in &mut groups {
        g.spans
            .sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let base = [-g.dir[1] * g.off, g.dir[0] * g.off];
        let point = |t: f64| [base[0] + g.dir[0] * t, base[1] + g.dir[1] * t];
        let mut run: Option<(f64, f64, Vec<EdgeId>)> = None;
        for &(t0, t1, id) in &g.spans {
            match &mut run {
                Some((_, hi, ids)) if t0 <= *hi + LINE_TOL => {
                    *hi = hi.max(t1);
                    ids.push(id);
                }
                Some((lo, hi, ids)) => {
                    out.push(CreaseLine {
                        id: 0,
                        kind: g.kind,
                        edges: std::mem::take(ids),
                        a: point(*lo),
                        b: point(*hi),
                    });
                    run = Some((t0, t1, vec![id]));
                }
                None => run = Some((t0, t1, vec![id])),
            }
        }
        if let Some((lo, hi, ids)) = run {
            out.push(CreaseLine {
                id: 0,
                kind: g.kind,
                edges: ids,
                a: point(lo),
                b: point(hi),
            });
        }
    }

    out.sort_by(|x, y| {
        x.a[0]
            .total_cmp(&y.a[0])
            .then(x.a[1].total_cmp(&y.a[1]))
            .then(x.b[0].total_cmp(&y.b[0]))
            .then(x.b[1].total_cmp(&y.b[1]))
            .then((x.kind as u8).cmp(&(y.kind as u8)))
    });
    for (i, line) in out.iter_mut().enumerate() {
        line.id = i;
        line.edges.sort_unstable();
    }
    out
}

/// 展開図の辺IDから、それが属する折り線のまとまりの番号を引く表。
pub(crate) fn line_of_edge(lines: &[CreaseLine]) -> BTreeMap<EdgeId, usize> {
    let mut out = BTreeMap::new();
    for line in lines {
        for &e in &line.edges {
            out.insert(e, line.id);
        }
    }
    out
}

/// 全部を折り終えた状態。
fn full_mask(lines: usize) -> FoldedMask {
    if lines >= MAX_LINES {
        FoldedMask::MAX
    } else {
        (1 << lines) - 1
    }
}

/// 折り順を深さ優先で探し、たどった状態の数・枝分かれの最大数・時間を測る。
///
/// `next` は「その状態から折れる手」を、番号の小さい順に返す。乱数は使わないので
/// 同じ入力なら毎回同じ結果になる(時間を除く)。最初に見つけた1つの順番で止める。
pub fn search<F>(lines: usize, mut next: F, limits: SearchLimits) -> SearchStats
where
    F: FnMut(FoldedMask) -> Vec<FoldedMask>,
{
    let start = Instant::now();
    let full = full_mask(lines);
    let first = next(0);
    let first_moves = first.len();
    let mut stats = SearchStats {
        lines,
        first_moves,
        states: 1,
        max_branching: first_moves,
        plan_len: None,
        stop: StopReason::Exhausted,
        millis: 0.0,
    };
    if lines == 0 {
        stats.stop = StopReason::Completed;
        stats.plan_len = Some(0);
        stats.millis = start.elapsed().as_secs_f64() * 1000.0;
        return stats;
    }
    if first_moves == 0 {
        stats.stop = StopReason::NoFirstMove;
        stats.millis = start.elapsed().as_secs_f64() * 1000.0;
        return stats;
    }

    let mut visited: BTreeSet<FoldedMask> = BTreeSet::from([0]);
    let mut stack: Vec<(FoldedMask, Vec<FoldedMask>, usize)> = vec![(0, first, 0)];
    while !stack.is_empty() {
        if stats.states >= limits.max_states {
            stats.stop = StopReason::StateCap;
            break;
        }
        if start.elapsed().as_millis() > limits.max_millis {
            stats.stop = StopReason::TimeCap;
            break;
        }
        let top = stack.len() - 1;
        let picked = {
            let frame = &mut stack[top];
            if frame.2 < frame.1.len() {
                let mv = frame.1[frame.2];
                frame.2 += 1;
                Some((frame.0, mv))
            } else {
                None
            }
        };
        let Some((state, mv)) = picked else {
            stack.pop();
            continue;
        };
        let ns = state | mv;
        if !visited.insert(ns) {
            continue;
        }
        stats.states += 1;
        if ns == full {
            stats.plan_len = Some(stack.len());
            stats.stop = StopReason::Completed;
            break;
        }
        let moves = next(ns);
        stats.max_branching = stats.max_branching.max(moves.len());
        stack.push((ns, moves, 0));
    }
    stats.millis = start.elapsed().as_secs_f64() * 1000.0;
    stats
}

/// 「最初の`depth`手までに行ける状態」を全部数える(枝分かれの広さの測定)。
///
/// [`search`] は折り順を1つ見つけた時点で止まるため、行き止まりが無い規則では
/// 「たどった状態の数 = 手数 + 1」にしかならず、2つの方式の広さの差が出ない。
/// こちらは深さを区切って**その中の状態を全部**数えるので、
/// 探索の枝分かれが実際にどれだけ広がるかを同じ物差しで比べられる。
pub fn search_breadth<F>(
    lines: usize,
    mut next: F,
    depth: usize,
    limits: SearchLimits,
) -> SearchStats
where
    F: FnMut(FoldedMask) -> Vec<FoldedMask>,
{
    let start = Instant::now();
    let first = next(0);
    let mut stats = SearchStats {
        lines,
        first_moves: first.len(),
        states: 1,
        max_branching: first.len(),
        plan_len: None,
        stop: StopReason::Exhausted,
        millis: 0.0,
    };
    if first.is_empty() {
        stats.stop = StopReason::NoFirstMove;
        stats.millis = start.elapsed().as_secs_f64() * 1000.0;
        return stats;
    }
    let full = full_mask(lines);
    let mut visited: BTreeSet<FoldedMask> = BTreeSet::from([0]);
    let mut frontier: Vec<FoldedMask> = vec![0];
    'outer: for _ in 0..depth {
        let mut nextier: Vec<FoldedMask> = Vec::new();
        for &state in &frontier {
            if stats.states >= limits.max_states {
                stats.stop = StopReason::StateCap;
                break 'outer;
            }
            if start.elapsed().as_millis() > limits.max_millis {
                stats.stop = StopReason::TimeCap;
                break 'outer;
            }
            if state == full {
                continue; // 折り終わった状態からは先が無い
            }
            let moves = next(state);
            stats.max_branching = stats.max_branching.max(moves.len());
            for mv in moves {
                let ns = state | mv;
                if visited.insert(ns) {
                    stats.states += 1;
                    nextier.push(ns);
                }
            }
        }
        if nextier.is_empty() {
            break;
        }
        frontier = nextier;
    }
    stats.millis = start.elapsed().as_secs_f64() * 1000.0;
    stats
}
