//! 曲線の折り目(CPE-011): 円弧・3次ベジェを、指定した誤差以内の折れ線にする。
//!
//! 展開図のデータ構造は直線の辺だけなので、曲線は「十分細かい折れ線」として
//! 入れる。実際の紙でも曲線折りは連続した曲がりなので、分割を細かくすれば
//! 見た目・折れ方ともに近づく(残る差はたわみ計算 `ori3-soft` が埋める)。
//!
//! 同じ計算は画面側(apps/desktop/src/lib/curve.ts)にもある。Tauriコマンドは
//! 増やさない約束なので、描いている最中の形は画面側で作り、確定したときに
//! 既存の `AddSegment` を折れ線の本数だけ送る。数式は両方に同じテストを置く。

use std::f64::consts::TAU;

use glam::DVec2;
use ori3_model::{CreasePattern, EdgeId, EdgeKind};

use crate::graph::insert_segment;

/// 折れ線の分割数の上限。これ以上細かくしても画面では見分けが付かず、
/// 辺数だけが増えて操作が重くなる。
pub const MAX_CURVE_SEGMENTS: u32 = 200;
/// 既定の許容誤差(紙の長辺=1.0 に対する、曲線と弦の離れ方の上限)。
pub const DEFAULT_CURVE_TOL: f64 = 0.005;

const EPS: f64 = 1e-12;

/// 3点を通る円の中心。3点がほぼ一直線ならNone。
fn circumcenter(a: DVec2, b: DVec2, c: DVec2) -> Option<DVec2> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < EPS {
        return None;
    }
    let (qa, qb, qc) = (a.length_squared(), b.length_squared(), c.length_squared());
    Some(DVec2::new(
        (qa * (b.y - c.y) + qb * (c.y - a.y) + qc * (a.y - b.y)) / d,
        (qa * (c.x - b.x) + qb * (a.x - c.x) + qc * (b.x - a.x)) / d,
    ))
}

/// 円弧を折れ線にするときの分割数。1区間の弦の膨らみ r(1−cos(Δ/2)) が
/// tol 以下になる最小の数を返す。
#[must_use]
pub fn arc_segment_count(radius: f64, sweep: f64, tol: f64) -> u32 {
    let tol = tol.max(1e-9);
    if radius <= tol {
        return 1;
    }
    let step = 2.0 * (1.0 - tol / radius).clamp(-1.0, 1.0).acos();
    if step <= EPS {
        return MAX_CURVE_SEGMENTS;
    }
    ((sweep.abs() / step).ceil() as u32).clamp(1, MAX_CURVE_SEGMENTS)
}

/// 始点・通過点・終点で決まる円弧の折れ線(端点を含む)。
/// `segments` を指定するとその数で等分し、Noneなら `tol` から自動で決める。
/// 3点が一直線・退化していればただの線分を返す。
#[must_use]
pub fn arc_polyline(
    p0: [f64; 2],
    through: [f64; 2],
    p1: [f64; 2],
    tol: f64,
    segments: Option<u32>,
) -> Vec<[f64; 2]> {
    let (a, m, b) = (DVec2::from(p0), DVec2::from(through), DVec2::from(p1));
    let Some(c) = circumcenter(a, m, b) else {
        return vec![p0, p1];
    };
    let r = (a - c).length();
    if r < EPS {
        return vec![p0, p1];
    }
    let ang = |p: DVec2| (p - c).y.atan2((p - c).x);
    let (a0, am, a1) = (ang(a), ang(m), ang(b));
    // 通過点が始点から見て終点より手前にあるなら反時計回り、そうでなければ時計回り
    let d1 = (a1 - a0).rem_euclid(TAU);
    let sweep = if (am - a0).rem_euclid(TAU) < d1 {
        d1
    } else {
        d1 - TAU
    };
    let n = segments.map_or_else(
        || arc_segment_count(r, sweep, tol),
        |n| n.clamp(1, MAX_CURVE_SEGMENTS),
    );
    let mut pts: Vec<[f64; 2]> = (0..=n)
        .map(|i| {
            let t = a0 + sweep * f64::from(i) / f64::from(n);
            [c.x + r * t.cos(), c.y + r * t.sin()]
        })
        .collect();
    // 端点は丸め誤差を入れず指定値そのものにする(既存頂点へ確実に吸着させる)
    pts[0] = p0;
    pts[n as usize] = p1;
    pts
}

/// 3次ベジェを折れ線にするときの分割数。
/// 折れ線の誤差は max|B''| / (8n²) 以下なので、それが tol 以下になる数を返す。
#[must_use]
pub fn cubic_segment_count(
    p0: [f64; 2],
    c1: [f64; 2],
    c2: [f64; 2],
    p1: [f64; 2],
    tol: f64,
) -> u32 {
    let tol = tol.max(1e-9);
    let (p0, c1, c2, p1) = (
        DVec2::from(p0),
        DVec2::from(c1),
        DVec2::from(c2),
        DVec2::from(p1),
    );
    // 制御点が始点と終点を結ぶ直線に乗っていれば曲線もその直線上にある
    let chord = p1 - p0;
    if chord.length() > EPS {
        let u = chord.normalize();
        let off = |p: DVec2| (p - p0).perp_dot(u).abs();
        if off(c1) <= tol && off(c2) <= tol {
            return 1;
        }
    }
    let m = 6.0
        * (p0 - 2.0 * c1 + c2)
            .length()
            .max((c1 - 2.0 * c2 + p1).length());
    if m <= EPS {
        return 1;
    }
    ((m / (8.0 * tol)).sqrt().ceil() as u32).clamp(1, MAX_CURVE_SEGMENTS)
}

/// 3次ベジェの折れ線(端点を含む)。S字も引けるので自由度が高い。
#[must_use]
pub fn cubic_polyline(
    p0: [f64; 2],
    c1: [f64; 2],
    c2: [f64; 2],
    p1: [f64; 2],
    tol: f64,
    segments: Option<u32>,
) -> Vec<[f64; 2]> {
    let n = segments.map_or_else(
        || cubic_segment_count(p0, c1, c2, p1, tol),
        |n| n.clamp(1, MAX_CURVE_SEGMENTS),
    );
    let (a, b, c, d) = (
        DVec2::from(p0),
        DVec2::from(c1),
        DVec2::from(c2),
        DVec2::from(p1),
    );
    let mut pts: Vec<[f64; 2]> = (0..=n)
        .map(|i| {
            let t = f64::from(i) / f64::from(n);
            let u = 1.0 - t;
            let p =
                a * (u * u * u) + b * (3.0 * u * u * t) + c * (3.0 * u * t * t) + d * (t * t * t);
            [p.x, p.y]
        })
        .collect();
    pts[0] = p0;
    pts[n as usize] = p1;
    pts
}

/// 線分を紙の矩形(0,0)-(w,h)で切り取る(Liang-Barsky法)。掛からなければNone。
fn clip_to_paper(a: DVec2, d: DVec2, w: f64, h: f64) -> Option<(DVec2, DVec2)> {
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for (p, q) in [(-d.x, a.x), (d.x, w - a.x), (-d.y, a.y), (d.y, h - a.y)] {
        if p.abs() < EPS {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            t0 = t0.max(r);
        } else {
            t1 = t1.min(r);
        }
    }
    if t1 - t0 < EPS {
        return None;
    }
    Some((a + d * t0, a + d * t1))
}

/// 曲線の折り目の両側に入れる「紙が曲がるための線」(ruling)。
///
/// 曲線の折り目は、両側の紙が曲がらないと折れない(平らな板2枚を曲線で
/// つなぐと、角度0以外では紙がちぎれる)。実際の紙では折り目の両側が
/// 円錐状に曲がっており、その曲がりを表すのがこの線。
///
/// 折れ線の各内点で、折れ線に直角な向きへ紙の縁まで伸ばした線を返す。
/// 各要素は `[へこむ側の端, 折れ線上の点, 膨らむ側の端]`(向きは曲がる向きで決まる)。
#[must_use]
pub fn ruling_lines(points: &[[f64; 2]], paper: [f64; 2]) -> Vec<[[f64; 2]; 3]> {
    let mut out = Vec::new();
    for i in 1..points.len().saturating_sub(1) {
        let (prev, cur, next) = (
            DVec2::from(points[i - 1]),
            DVec2::from(points[i]),
            DVec2::from(points[i + 1]),
        );
        let tan = next - prev;
        if tan.length() < EPS {
            continue;
        }
        let n = DVec2::new(-tan.y, tan.x).normalize();
        let Some((p, q)) = clip_to_paper(cur, n, paper[0], paper[1]) else {
            continue;
        };
        // 左へ曲がる(外積が正)なら、へこむ側は法線の正の向き(=q側)
        let left = (cur - prev).perp_dot(next - cur) > 0.0;
        let (concave, convex) = if left { (q, p) } else { (p, q) };
        if (concave - cur).length() < EPS || (convex - cur).length() < EPS {
            continue;
        }
        out.push([concave.into(), cur.into(), convex.into()]);
    }
    out
}

/// 折れ線を展開図へ入れる(区間ごとに `insert_segment`)。
/// 戻り値は新しくできた辺のID(後の区間の挿入で再分割され得る点は同じ)。
pub fn insert_polyline(cp: &mut CreasePattern, points: &[[f64; 2]], kind: EdgeKind) -> Vec<EdgeId> {
    points
        .windows(2)
        .flat_map(|w| insert_segment(cp, w[0], w[1], kind))
        .collect()
}

/// 曲がるための線を展開図へ入れる(先に `insert_polyline` で曲線を入れておく)。
/// 折り目の両側で曲がる向きが逆になるので、へこむ側は曲線と反対の線種、
/// 膨らむ側は同じ線種にする(画面の山谷の色を実際の曲がりに合わせる)。
pub fn insert_rulings(
    cp: &mut CreasePattern,
    points: &[[f64; 2]],
    paper: [f64; 2],
    curve_kind: EdgeKind,
) -> Vec<EdgeId> {
    let opposite = if curve_kind == EdgeKind::Mountain {
        EdgeKind::Valley
    } else {
        EdgeKind::Mountain
    };
    let mut ids = Vec::new();
    for [concave, cur, convex] in ruling_lines(points, paper) {
        ids.extend(insert_segment(
            cp,
            cur,
            first_crossing(cp, cur, concave),
            opposite,
        ));
        ids.extend(insert_segment(
            cp,
            cur,
            first_crossing(cp, cur, convex),
            curve_kind,
        ));
    }
    ids
}

/// `from`から`to`へ向かって、最初にぶつかる既存の折り目までで切った終点。
/// ぶつからなければ`to`のまま。曲がるための線が他の折り目を突き抜けて
/// 関係のない場所まで伸びるのを防ぐ(実際の紙でも紙が曲がる範囲は
/// 隣の折り目までで区切られる)。
fn first_crossing(cp: &CreasePattern, from: [f64; 2], to: [f64; 2]) -> [f64; 2] {
    let (a, b) = (DVec2::from(from), DVec2::from(to));
    let len = (b - a).length();
    if len < EPS {
        return to;
    }
    let dir = (b - a) / len;
    let vpos: std::collections::BTreeMap<_, _> = cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();
    let mut best = len;
    for e in &cp.edges {
        if e.kind == EdgeKind::Aux {
            continue;
        }
        let (Some(&p0), Some(&p1)) = (vpos.get(&e.v0), vpos.get(&e.v1)) else {
            continue;
        };
        if let Some(q) = ori3_geometry::seg_intersection(a, b, p0, p1) {
            let t = (q - a).dot(dir);
            // 出発点そのもの(曲線の上)での交わりは無視する
            if t > 1e-6 && t < best {
                best = t;
            }
        }
    }
    (a + dir * best).into()
}
