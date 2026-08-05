//! 点群のドロネー三角形分割(軸多角形の下ごしらえ。要件§8-3)。
//!
//! 扱う点は「角(最大12本)の円中心 + 紙の四隅」程度なので、総当たり
//! (すべての三点組について外接円が空かを調べる)で十分に速い。
//!
//! 正方形の四隅のように4点が同一円周へ乗る配置では素朴な判定だと三角形が
//! 重複してしまうため、点を放物面へ持ち上げるときに添字へ比例した微小な
//! 重みを足して縮退を崩す。重みは決定的なので結果も決定的になる。

/// 同じ点とみなす距離。CPのEPS(1e-9)より十分大きく取り、
/// 充填の丸め誤差で二重の頂点ができるのを防ぐ。
pub const MERGE_TOL: f64 = 1e-7;

/// 近すぎる点・非有限な点を取り除く。並び順は入力順を保ち、先に現れた方を残す。
pub fn dedup(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::new();
    for p in points {
        if !p[0].is_finite() || !p[1].is_finite() {
            continue;
        }
        if out
            .iter()
            .any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) <= MERGE_TOL)
        {
            continue;
        }
        out.push(*p);
    }
    out
}

/// `points` の中で `p` に最も近い点の添字(MERGE_TOL以内)。
pub fn index_of(points: &[[f64; 2]], p: [f64; 2]) -> Option<usize> {
    points
        .iter()
        .position(|q| (q[0] - p[0]).hypot(q[1] - p[1]) <= MERGE_TOL)
}

/// o→a と o→b の外積(正なら反時計回り)。
fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// 持ち上げた三角形の外接円に d が入るなら正(三角形は反時計回り前提)。
fn in_circle(t: [[f64; 3]; 3], d: [f64; 3]) -> f64 {
    let r = |i: usize| [t[i][0] - d[0], t[i][1] - d[1], t[i][2] - d[2]];
    let (a, b, c) = (r(0), r(1), r(2));
    a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0])
}

/// ドロネー三角形分割。各三角形は反時計回りに並べた点の添字3つ。
/// 三角形の並びは添字の昇順で走査するので入力に対して決定的。
///
/// 点がすべて一直線上にあるときは空を返す。
pub fn triangulate(points: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let span = points
        .iter()
        .flat_map(|p| [p[0].abs(), p[1].abs()])
        .fold(1.0_f64, f64::max);
    let area_tol = 1e-12 * span * span;
    // 放物面への持ち上げ。第3成分へ添字比例の微小重みを足して同一円周を崩す。
    let bias = 1e-7 * span * span;
    let lift: Vec<[f64; 3]> = points
        .iter()
        .enumerate()
        .map(|(i, p)| [p[0], p[1], p[0] * p[0] + p[1] * p[1] + i as f64 * bias])
        .collect();
    let mut tris = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                let s = cross(points[i], points[j], points[k]);
                if s.abs() <= area_tol {
                    continue; // 潰れた三角形
                }
                let t = if s > 0.0 { [i, j, k] } else { [i, k, j] };
                let lt = [lift[t[0]], lift[t[1]], lift[t[2]]];
                let empty = (0..n)
                    .filter(|d| !t.contains(d))
                    .all(|d| in_circle(lt, lift[d]) <= 0.0);
                if empty {
                    tris.push(t);
                }
            }
        }
    }
    tris
}
