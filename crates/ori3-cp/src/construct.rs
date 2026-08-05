//! 作図補助(CPE-005): 折り紙でよく使う「あたり」の線・点を求める。
//!
//! 角の二等分線・点から線への垂線・線分のn等分点・一定角度刻みの方向線を返す。
//! どれも純粋な計算で、CPは変更しない(呼び出し側が補助線として挿入する)。
//! 座標は「紙の長辺 = 1.0」に正規化した系(ori3-model の規約)。
//!
//! 返す線は紙からはみ出し得る。紙の内側だけに引きたい場合は呼び出し側で
//! 紙の矩形に切り詰めること(画面側 `lib/construct.ts` がそうしている)。

use glam::DVec2;
use ori3_model::EPS;

/// 方向線の片側の長さ。紙の長辺が1.0なので、紙の内側のどの点から引いても
/// 反対の縁まで必ず届く(正方形の対角線 ≒ 1.415 の半分より長い)。
const RAY_LEN: f64 = 1.0;

/// 角ABC(頂点B)の二等分線。始点はB、長さは長い方の腕に合わせる。
///
/// まっすぐ(180°)に開いた角では腕に垂直な線を返す。腕の長さがゼロで角が
/// 決まらないときは潰れた線(始点=終点)を返し、呼び出し側が捨てられるようにする。
#[must_use]
pub fn bisector(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [[f64; 2]; 2] {
    let (pa, pb, pc) = (DVec2::from(a), DVec2::from(b), DVec2::from(c));
    let (u, v) = (pa - pb, pc - pb);
    let (lu, lv) = (u.length(), v.length());
    if lu < EPS || lv < EPS {
        return [b, b];
    }
    let sum = u / lu + v / lv;
    let dir = if sum.length() < EPS {
        // 180°に開いた角: 二等分線の向きが定まらないので腕に垂直な向きを使う
        (u / lu).perp()
    } else {
        sum.normalize()
    };
    let end = pb + dir * lu.max(lv);
    [b, [end.x, end.y]]
}

/// 点pから線分segへ下ろした垂線(始点p、終点は足)。
///
/// 足は線分を延長した直線の上に取る(線分の外へ出てもよい)。作図では
/// 「この線に直角な線」を引きたいので、区間内に押し込めると意図がずれるため。
/// 線分が潰れているときは潰れた線を返す。
#[must_use]
pub fn perpendicular(p: [f64; 2], seg: [[f64; 2]; 2]) -> [[f64; 2]; 2] {
    let (pp, s0, s1) = (DVec2::from(p), DVec2::from(seg[0]), DVec2::from(seg[1]));
    let d = s1 - s0;
    if d.length() < EPS {
        return [p, p];
    }
    let foot = s0 + d * ((pp - s0).dot(d) / d.length_squared());
    [p, [foot.x, foot.y]]
}

/// 線分をn等分する点(両端は含まない n-1 個)。
/// nが2〜8の外、または線分が潰れているときは空を返す(作図として意味がないため)。
#[must_use]
pub fn divide_points(seg: [[f64; 2]; 2], n: u32) -> Vec<[f64; 2]> {
    let (s0, s1) = (DVec2::from(seg[0]), DVec2::from(seg[1]));
    if !(2..=8).contains(&n) || (s1 - s0).length() < EPS {
        return Vec::new();
    }
    (1..n)
        .map(|i| {
            let q = s0.lerp(s1, f64::from(i) / f64::from(n));
            [q.x, q.y]
        })
        .collect()
}

/// 点pを通る方向線を、0°から`step_deg`刻みで180°未満まで並べて返す。
/// 22.5°なら8本(0/22.5/45/…/157.5°)。線は直線なので180°以上は同じ線の繰り返しになる。
/// 刻みが0以下・180°超のときは空を返す。
#[must_use]
pub fn direction_lines(p: [f64; 2], step_deg: f64) -> Vec<[[f64; 2]; 2]> {
    if !(step_deg > 0.0 && step_deg <= 180.0) {
        return Vec::new();
    }
    let center = DVec2::from(p);
    let mut lines = Vec::new();
    let mut deg: f64 = 0.0;
    while deg < 180.0 - 1e-9 {
        let (s, c) = deg.to_radians().sin_cos();
        let d = DVec2::new(c, s) * RAY_LEN;
        let (a, b) = (center - d, center + d);
        lines.push([[a.x, a.y], [b.x, b.y]]);
        deg += step_deg;
    }
    lines
}
