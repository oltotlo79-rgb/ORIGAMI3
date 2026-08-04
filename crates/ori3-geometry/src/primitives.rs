//! 線分の交差・距離・鏡映などの基本演算。

use glam::DVec2;
use ori3_model::EPS;

/// 線分同士の交点。端点接触も交差として返す。平行・重なりはNone
pub fn seg_intersection(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> Option<DVec2> {
    let da = a1 - a0;
    let db = b1 - b0;
    let denom = da.perp_dot(db);
    if denom.abs() < EPS {
        // 平行(同一直線上の重なりを含む)。
        return None;
    }
    let diff = b0 - a0;
    let t = diff.perp_dot(db) / denom;
    let u = diff.perp_dot(da) / denom;
    // パラメータ許容量は座標系のEPSに合わせて線分長で正規化する。
    let tol_t = EPS / da.length().max(EPS);
    let tol_u = EPS / db.length().max(EPS);
    if (-tol_t..=1.0 + tol_t).contains(&t) && (-tol_u..=1.0 + tol_u).contains(&u) {
        Some(a0 + da * t)
    } else {
        None
    }
}

/// 点が線分上にあるか(EPS許容)
pub fn point_on_segment(p: DVec2, a: DVec2, b: DVec2) -> bool {
    dist_point_segment(p, a, b) <= EPS
}

/// 直線(l0,l1)に対する点の鏡映
pub fn reflect_across_line(p: DVec2, l0: DVec2, l1: DVec2) -> DVec2 {
    let d = l1 - l0;
    let len_sq = d.length_squared();
    if len_sq < EPS * EPS {
        // 直線が退化している場合は点対称とみなす。
        return l0 * 2.0 - p;
    }
    let v = p - l0;
    let proj = d * (v.dot(d) / len_sq);
    l0 + proj * 2.0 - v
}

/// 点と線分の距離
pub fn dist_point_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < EPS * EPS {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}
