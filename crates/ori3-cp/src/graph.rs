//! 展開図(平面グラフ)の編集: 線分挿入・辺削除・頂点移動。
//!
//! 線分挿入は既存の頂点・辺への吸着(EPS)と、既存辺との交点での自動分割を行い、
//! CPを常に「辺同士は端点でのみ接する平面グラフ」に保つ。

use glam::DVec2;
use ori3_geometry::{collinear_overlap, dist_point_segment, seg_intersection};
use ori3_model::{CreasePattern, EPS, Edge, EdgeId, EdgeKind, Vertex, VertexId};

/// 頂点IDから座標を引く。
fn pos(cp: &CreasePattern, id: VertexId) -> DVec2 {
    let v = cp
        .vertices
        .iter()
        .find(|v| v.id == id)
        .unwrap_or_else(|| panic!("頂点ID {id} が存在しない"));
    DVec2::from(v.pos)
}

/// 頂点IDから座標を引く(不在ならNone。参照切れ辺の許容用)。
fn try_pos(cp: &CreasePattern, id: VertexId) -> Option<DVec2> {
    cp.vertices
        .iter()
        .find(|v| v.id == id)
        .map(|v| DVec2::from(v.pos))
}

/// 点からEPS以内にある最近傍の既存頂点を探す。
fn find_vertex_within_eps(cp: &CreasePattern, p: DVec2) -> Option<VertexId> {
    let mut best: Option<(f64, VertexId)> = None;
    for v in &cp.vertices {
        let d = (DVec2::from(v.pos) - p).length();
        if d <= EPS && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, v.id));
        }
    }
    best.map(|(_, id)| id)
}

fn add_vertex(cp: &mut CreasePattern, p: DVec2) -> VertexId {
    let id = cp.next_vertex_id;
    cp.next_vertex_id += 1;
    cp.vertices.push(Vertex {
        id,
        pos: [p.x, p.y],
    });
    id
}

/// 点をEPS以内の既存頂点に解決し、なければ新頂点を作る。
fn get_or_create_vertex(cp: &mut CreasePattern, p: DVec2) -> VertexId {
    find_vertex_within_eps(cp, p).unwrap_or_else(|| add_vertex(cp, p))
}

fn add_edge(cp: &mut CreasePattern, v0: VertexId, v1: VertexId, kind: EdgeKind) -> EdgeId {
    let id = cp.next_edge_id;
    cp.next_edge_id += 1;
    cp.edges.push(Edge { id, v0, v1, kind });
    id
}

/// 点を線分に射影する(区間内にクランプ)。
fn project_to_segment(p: DVec2, a: DVec2, b: DVec2) -> DVec2 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < EPS * EPS {
        return a;
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    a + ab * t
}

/// 既存辺を指定位置の列で分割する(kindは保たれる)。
/// 端点からEPS以内の位置は無視する。実際に分割が起きた場合、元の辺は削除され
/// 新しいIDの辺の連なりに置き換わる。
fn split_edge_at_points(cp: &mut CreasePattern, eid: EdgeId, points: &[DVec2]) {
    let idx = cp
        .edges
        .iter()
        .position(|e| e.id == eid)
        .unwrap_or_else(|| panic!("辺ID {eid} が存在しない"));
    let (v0, v1, kind) = {
        let e = &cp.edges[idx];
        (e.v0, e.v1, e.kind)
    };
    let p0 = pos(cp, v0);
    let p1 = pos(cp, v1);
    let len = (p1 - p0).length();
    if len < EPS {
        return; // 退化辺は分割しない
    }
    let dirn = (p1 - p0) / len;
    let mut ts: Vec<f64> = points
        .iter()
        .map(|&q| (q - p0).dot(dirn))
        .filter(|&t| t > EPS && t < len - EPS)
        .collect();
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|a, b| (*a - *b).abs() <= EPS);
    if ts.is_empty() {
        return;
    }
    let mut chain = vec![v0];
    for t in ts {
        let vid = get_or_create_vertex(cp, p0 + dirn * t);
        if *chain.last().unwrap() != vid {
            chain.push(vid);
        }
    }
    if *chain.last().unwrap() != v1 {
        chain.push(v1);
    }
    if chain.len() < 3 {
        return; // 実質的な分割なし
    }
    cp.edges.remove(idx);
    for w in chain.windows(2) {
        add_edge(cp, w[0], w[1], kind);
    }
}

/// 線分の端点を頂点に解決する。
/// EPS以内の既存頂点 → その頂点。既存辺上(EPS以内)→ 辺を分割して新頂点。
/// どちらでもなければ新頂点。
fn resolve_endpoint(cp: &mut CreasePattern, p: DVec2) -> VertexId {
    if let Some(v) = find_vertex_within_eps(cp, p) {
        return v;
    }
    let mut best: Option<(f64, EdgeId)> = None;
    for e in &cp.edges {
        let d = dist_point_segment(p, pos(cp, e.v0), pos(cp, e.v1));
        if d <= EPS && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, e.id));
        }
    }
    if let Some((_, eid)) = best {
        let e = cp.edges.iter().find(|e| e.id == eid).unwrap();
        let q = project_to_segment(p, pos(cp, e.v0), pos(cp, e.v1));
        split_edge_at_points(cp, eid, &[q]);
        return get_or_create_vertex(cp, q);
    }
    add_vertex(cp, p)
}

/// 線分を挿入し、既存辺との交点で双方を自動分割する。
/// 端点は既存頂点(EPS以内)へ吸着し、なければ既存辺上(EPS以内)なら辺を分割して
/// 頂点を作る。既存辺と同一直線上で重なる区間には新しい辺を作らず、既存辺の
/// kindをそのまま維持する(上書きしない)。
///
/// 戻り値は新規作成した辺のID(新線分に沿った区間の辺のみ)。分割で生じた
/// 既存辺の断片は含まない。なお、後続の挿入で辺が再分割されると返したIDは
/// 無効になり得る。
pub fn insert_segment(
    cp: &mut CreasePattern,
    a: [f64; 2],
    b: [f64; 2],
    kind: EdgeKind,
) -> Vec<EdgeId> {
    let pa_raw = DVec2::from(a);
    let pb_raw = DVec2::from(b);
    if (pa_raw - pb_raw).length() < EPS {
        return Vec::new(); // 退化線分(点)は挿入しない
    }
    let va = resolve_endpoint(cp, pa_raw);
    let vb = resolve_endpoint(cp, pb_raw);
    if va == vb {
        return Vec::new();
    }
    let pa = pos(cp, va);
    let pb = pos(cp, vb);
    let len = (pb - pa).length();
    if len < EPS {
        return Vec::new();
    }
    let dirn = (pb - pa) / len;
    let param = |q: DVec2| (q - pa).dot(dirn).clamp(0.0, len);

    // この後の分割で辺リストが変わるため、交差判定は挿入前の辺のスナップショットで行う。
    // 存在しない頂点を参照する辺(参照切れ)はpanicさせず判定対象外にする
    // (validate/extract_facesと同じ基準。壊れたCPでも編集を続けられるようにする)。
    let snapshot: Vec<(EdgeId, DVec2, DVec2)> = cp
        .edges
        .iter()
        .filter_map(|e| Some((e.id, try_pos(cp, e.v0)?, try_pos(cp, e.v1)?)))
        .collect();

    let mut cut_ts: Vec<f64> = vec![0.0, len]; // 新線分上の分割位置(弧長パラメータ)
    let mut covered: Vec<(f64, f64)> = Vec::new(); // 既存辺と重なる区間
    let mut edge_cuts: Vec<(EdgeId, Vec<DVec2>)> = Vec::new(); // 既存辺側の分割位置

    for &(eid, e0, e1) in &snapshot {
        // ほぼ重なった2線分では両方がSomeを返し得るため、重なり判定を先に問い合わせる。
        if let Some((q0, q1)) = collinear_overlap(pa, pb, e0, e1) {
            let (t0, t1) = (param(q0), param(q1));
            cut_ts.push(t0);
            cut_ts.push(t1);
            edge_cuts.push((eid, vec![q0, q1]));
            if (t1 - t0).abs() > EPS {
                covered.push((t0.min(t1), t0.max(t1)));
            }
        } else if let Some(q) = seg_intersection(pa, pb, e0, e1) {
            cut_ts.push(param(q));
            edge_cuts.push((eid, vec![q]));
        }
    }

    // 既存辺を交点で分割する(kindは保たれる)。
    for (eid, pts) in edge_cuts {
        split_edge_at_points(cp, eid, &pts);
    }

    // 新線分を分割位置で区切り、区間ごとに辺を作る。
    cut_ts.sort_by(f64::total_cmp);
    let mut chain: Vec<(f64, VertexId)> = Vec::new();
    for t in cut_ts {
        let vid = get_or_create_vertex(cp, pa + dirn * t);
        if chain.last().is_none_or(|&(_, last)| last != vid) {
            chain.push((t, vid));
        }
    }

    let mut added = Vec::new();
    for w in chain.windows(2) {
        let ((t0, v0), (t1, v1)) = (w[0], w[1]);
        let tm = 0.5 * (t0 + t1);
        // 既存辺と重なる区間には新しい辺を作らない(既存辺のkindを維持)。
        if covered
            .iter()
            .any(|&(lo, hi)| lo - EPS <= tm && tm <= hi + EPS)
        {
            continue;
        }
        // 同じ頂点対を結ぶ辺が既にあれば作らない(重複防止の安全網)。
        if cp
            .edges
            .iter()
            .any(|e| (e.v0 == v0 && e.v1 == v1) || (e.v0 == v1 && e.v1 == v0))
        {
            continue;
        }
        added.push(add_edge(cp, v0, v1, kind));
    }
    added
}

/// 指定した辺を削除する。削除の結果どの辺からも参照されなくなった頂点も掃除する。
pub fn remove_edges(cp: &mut CreasePattern, ids: &[EdgeId]) {
    cp.edges.retain(|e| !ids.contains(&e.id));
    let used: std::collections::BTreeSet<VertexId> =
        cp.edges.iter().flat_map(|e| [e.v0, e.v1]).collect();
    cp.vertices.retain(|v| used.contains(&v.id));
}

/// 頂点の座標のみを更新する(接続やkindは変えない)。
/// 移動によって他の辺と交差が生じた場合の再分割は呼び出し側の責務
/// (UI側で挿入し直す等の対応を行う予定)。存在しないIDは何もしない。
pub fn move_vertex(cp: &mut CreasePattern, id: VertexId, to: [f64; 2]) {
    if let Some(v) = cp.vertices.iter_mut().find(|v| v.id == id) {
        v.pos = to;
    }
}
