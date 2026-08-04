//! 線分の交差・距離・鏡映などの基本演算。

use glam::DVec2;
use ori3_model::EPS;

/// 平行判定の相対許容量。denom(=|da||db|·sinθ)は線分長の2乗でスケールするため、
/// 絶対閾値ではなく |da||db| に対する相対値で判定する(f64の丸め誤差より十分大きい値)。
const REL_TOL: f64 = 1e-12;

/// 線分同士の交点。端点接触も交差として返す。平行・重なりはNone。
/// 同一直線上の重なり区間が必要な場合は [`collinear_overlap`] を使う。
///
/// 端点接触はEPS許容で判定するため、返る交点は最大EPS程度線分の外に
/// はみ出し得る(呼び出し側で既存頂点へのスナップを行う前提)。
pub fn seg_intersection(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> Option<DVec2> {
    let da = a1 - a0;
    let db = b1 - b0;
    let len_a = da.length();
    let len_b = db.length();
    if len_a < EPS || len_b < EPS {
        // 退化線分(点)は交差計算の対象にしない。
        return None;
    }
    let denom = da.perp_dot(db);
    if denom.abs() < REL_TOL * len_a * len_b {
        // 平行(同一直線上の重なりを含む)。
        return None;
    }
    let diff = b0 - a0;
    let t = diff.perp_dot(db) / denom;
    let u = diff.perp_dot(da) / denom;
    // パラメータ許容量は座標系のEPS(距離)に合わせて線分長で正規化する。
    let tol_t = EPS / len_a;
    let tol_u = EPS / len_b;
    if (-tol_t..=1.0 + tol_t).contains(&t) && (-tol_u..=1.0 + tol_u).contains(&u) {
        Some(a0 + da * t)
    } else {
        None
    }
}

/// 2線分が同一直線上(EPS許容)にあり区間が重なる場合、その重なり区間の両端を返す。
/// 点接触(端点のみ共有)は同一点のペアを返す。同一直線上でない・重ならない場合はNone。
pub fn collinear_overlap(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> Option<(DVec2, DVec2)> {
    let da = a1 - a0;
    let len_a = da.length();
    // 退化線分は「点と線分の重なり」として扱う。
    if len_a < EPS {
        return if point_on_segment(a0, b0, b1) {
            Some((a0, a0))
        } else {
            None
        };
    }
    if (b1 - b0).length() < EPS {
        return if point_on_segment(b0, a0, a1) {
            Some((b0, b0))
        } else {
            None
        };
    }
    // bの両端点が線分aの乗る直線からEPS以内 ⇔ 同一直線上。
    let dir = da / len_a;
    if (b0 - a0).perp_dot(dir).abs() > EPS || (b1 - a0).perp_dot(dir).abs() > EPS {
        return None;
    }
    // 直線に沿った弧長パラメータで区間の共通部分を求める(aは[0, len_a]に対応)。
    let tb0 = (b0 - a0).dot(dir);
    let tb1 = (b1 - a0).dot(dir);
    let lo = tb0.min(tb1).max(0.0);
    let hi = tb0.max(tb1).min(len_a);
    if hi < lo - EPS {
        return None;
    }
    // 点接触(端点のみ共有)は同一点のペアに潰す。
    let hi = hi.max(lo);
    Some((a0 + dir * lo, a0 + dir * hi))
}

/// 点が線分上にあるか(EPS許容)
pub fn point_on_segment(p: DVec2, a: DVec2, b: DVec2) -> bool {
    dist_point_segment(p, a, b) <= EPS
}

/// 直線(l0,l1)に対する点の鏡映
pub fn reflect_across_line(p: DVec2, l0: DVec2, l1: DVec2) -> DVec2 {
    let d = l1 - l0;
    let len_sq = d.length_squared();
    debug_assert!(
        len_sq >= EPS * EPS,
        "reflect_across_line: 直線の2点が一致している(l0={l0:?}, l1={l1:?})"
    );
    if len_sq < EPS * EPS {
        // 直線が退化している場合は点対称とみなす(リリースビルドでの防御)。
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
