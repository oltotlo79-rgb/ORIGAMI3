//! 位置ベースの反復計算(PBD)でたわみの形を求める。
//!
//! 拘束は全て構築時に確定し、反復中は「反復回数固定・走査順固定」で
//! Gauss-Seidel射影するだけなので、同じ入力からは常に同じ結果になる
//! (SYS-004 決定性)。近傍探索も初期位置で一度だけ行い、反復のたびに
//! やり直さない(見た目の近似としては十分で、性能目標NFR-002bを守れる)。

use glam::DVec3;
use ori3_model::EPS;

use crate::grid::CellGrid;
use crate::subdivide::RawMesh;

/// 近傍探索で見るセルのずらし幅。周囲27セルのうち**辞書順で(0,0,0)以上の14個**
/// だけを見る。逆向きのずらし幅は相手側の頂点から同じ組を見つけるので、
/// これで全ての組をちょうど1回ずつ拾える(探索回数がほぼ半分になる)。
const NEIGHBOR_CELLS: [[i64; 3]; 14] = {
    let mut out = [[0i64; 3]; 14];
    let mut n = 0;
    let mut x = 0i64;
    while x <= 1 {
        let mut y = if x == 0 { 0 } else { -1 };
        while y <= 1 {
            let mut z = if x == 0 && y == 0 { 0 } else { -1 };
            while z <= 1 {
                out[n] = [x, y, z];
                n += 1;
                z += 1;
            }
            y += 1;
        }
        x += 1;
    }
    out
};

/// 層どうしを離す最小の隙間(紙の長辺=1.0の系での「紙の厚み」相当)。
const LAYER_GAP: f64 = 0.002;
/// 隙間拘束を組む相手を探す半径(初期位置で一度だけ探す)。
const SEARCH_RADIUS: f64 = LAYER_GAP * 4.0;
/// 空気圧1.0・硬さ0.0・層の両端で、1反復あたりに押し広げる距離。
const PRESSURE_STEP: f64 = 0.008;
/// 硬さが空気圧の効きを抑える割合(硬さ1.0で押し出しが 1−この値 倍になる)。
/// 位置ベースの押し出しは曲げ拘束だけでは戻しきれないため、「硬い紙ほど同じ
/// 空気圧では膨らみにくい」という物理的にもっともらしい関係をここで明示的に
/// 与える(見た目の近似。§4.2 のとおり厳密な力の釣り合いは解かない)。
const STIFFNESS_DAMPING: f64 = 0.75;
/// 層が違うとみなす層番号の差(折り線上の共有点は隣接層の中間値を持つため、
/// 1.0未満の差は「同じ重なりの中」として拘束しない)。
const LAYER_DIFF: f64 = 1.0;

/// 反復で使う拘束一式。すべて構築時に決まり、反復中は変わらない。
pub(crate) struct Constraints {
    /// (頂点a, 頂点b, 初期長) — 紙が伸びない拘束
    pub stretch: Vec<(u32, u32, f64)>,
    /// (向かい合う頂点a, b, 初期長, 強さ) — 曲げ拘束と折り目の角度拘束
    pub bend: Vec<(u32, u32, f64, f64)>,
    /// (下の層の頂点, 上の層の頂点, 押し戻す向き) — 層の貫通防止(SIM-014)
    pub gaps: Vec<(u32, u32, DVec3)>,
    /// 頂点ごとの空気圧の押し出し量(向き×層の位置。1反復ぶん)
    pub push: Vec<DVec3>,
}

/// 大きさの最大成分が正になるよう向きをそろえる(層の向きの符号を決定的にする)。
fn canonical_axis(n: DVec3) -> DVec3 {
    let a = n.abs();
    let k = if a.x >= a.y && a.x >= a.z {
        0
    } else if a.y >= a.z {
        1
    } else {
        2
    };
    if n[k] < 0.0 { -n } else { n }
}

/// 頂点ごとの「層が積み上がる向き」と、その頂点の平均層番号を求める。
///
/// 平らに畳んだ形では全ての面が同一平面に乗るため、面の法線だけでは
/// 層の上下を決められない。そこで**最大面積の三角形の法線**を全体の
/// 積み上げ軸とし、各三角形の法線をその軸に合う向きへそろえて足し込む。
fn layer_field(mesh: &RawMesh, pos: &[DVec3]) -> (Vec<DVec3>, Vec<f64>) {
    let mut best = (0.0f64, DVec3::Z);
    let normals: Vec<DVec3> = mesh
        .triangles
        .iter()
        .map(|t| {
            let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
            let n = (b - a).cross(c - a);
            if n.length() > best.0 {
                best = (n.length(), n);
            }
            n
        })
        .collect();
    let axis = canonical_axis(best.1.try_normalize().unwrap_or(DVec3::Z));
    let mut dir = vec![DVec3::ZERO; pos.len()];
    let (mut lsum, mut lcnt) = (vec![0.0f64; pos.len()], vec![0.0f64; pos.len()]);
    for (ti, t) in mesh.triangles.iter().enumerate() {
        let s = if normals[ti].dot(axis) < 0.0 { -1.0 } else { 1.0 };
        for &v in t {
            dir[v as usize] += normals[ti] * s;
            lsum[v as usize] += f64::from(mesh.tri_layer[ti]);
            lcnt[v as usize] += 1.0;
        }
    }
    let updir = dir
        .iter()
        .map(|d| d.try_normalize().unwrap_or(axis))
        .collect();
    let layer = lsum
        .iter()
        .zip(&lcnt)
        .map(|(s, &c)| if c > 0.0 { s / c } else { 0.0 })
        .collect();
    (updir, layer)
}

/// 初期位置から拘束一式を組む(`stiffness`・`pressure`は0〜1に丸め済みのもの)。
///
/// 曲げの強さは反復回数で薄まる(弱い拘束でも何度も掛ければ効いてしまう)ため、
/// 1回あたりの強さを `1 − (1 − stiffness)^(1/iterations)` に変換し、
/// **反復し終えたときの効き目が `stiffness` になる**ようそろえる。
pub(crate) fn build(
    mesh: &RawMesh,
    pos: &[DVec3],
    stiffness: f64,
    pressure: f64,
    iterations: u32,
) -> Constraints {
    let (updir, layer) = layer_field(mesh, pos);
    let soft_w = 1.0 - (1.0 - stiffness).powf(1.0 / f64::from(iterations.max(1)));

    // 網の辺ごとに、それを共有する2三角形の「向かい合う頂点」を結んで曲げ拘束にする。
    // 2頂点の距離は(辺長が保たれる限り)二面角の単調な関数なので、距離を初期値へ
    // 戻すことが「角度を初期の形へ戻す」ことになる。別々の面にまたがる辺は折り目
    // なので強さ1.0(剛体折りが求めた角度を保つ)、同じ面の中なら stiffness。
    // (辺の小さい方, 大きい方, 向かい合う頂点, 三角形番号)を整列して同じ辺をまとめる
    // (BTreeMapより速い。整列鍵に三角形番号を含めるので順序は決定的)。
    let mut sides: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(mesh.triangles.len() * 3);
    for (ti, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            sides.push((a.min(b), a.max(b), t[(k + 2) % 3], ti as u32));
        }
    }
    sides.sort_unstable();
    let mut bend = Vec::new();
    let mut stretch = Vec::new();
    let mut i = 0;
    while i < sides.len() {
        let (a, b, o0, t0) = sides[i];
        stretch.push((a, b, (pos[a as usize] - pos[b as usize]).length()));
        let mut j = i + 1;
        while j < sides.len() && (sides[j].0, sides[j].1) == (a, b) {
            j += 1;
        }
        if j > i + 1 {
            let (_, _, o1, t1) = sides[i + 1];
            let same = mesh.tri_face[t0 as usize] == mesh.tri_face[t1 as usize];
            let w = if same { soft_w } else { 1.0 };
            let rest = (pos[o0 as usize] - pos[o1 as usize]).length();
            if w > 0.0 && rest > EPS {
                bend.push((o0, o1, rest, w));
            }
        }
        i = j;
    }

    // 層の貫通防止の相手探し: 一様格子(セル鍵で整列した配列)で近傍を拾い、
    // 層番号が LAYER_DIFF 以上離れた組だけを拘束にする(折り線上の共有点は
    // 隣接層の中間値になるので外れる)。層がひとつしかなければ何もしない。
    let (lmin, lmax) = layer
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let mut gaps = Vec::new();
    if lmax - lmin >= LAYER_DIFF {
        let grid = CellGrid::new(pos, SEARCH_RADIUS);
        for (i, p) in pos.iter().enumerate() {
            let b = grid.cell_of(*p);
            for d in NEIGHBOR_CELLS {
                let same_cell = d == [0, 0, 0];
                for &j in grid.cell([b[0] + d[0], b[1] + d[1], b[2] + d[2]]) {
                    let (li, lj) = (layer[i], layer[j as usize]);
                    if (same_cell && j as usize <= i)
                        || (li - lj).abs() < LAYER_DIFF
                        || (pos[j as usize] - *p).length() > SEARCH_RADIUS
                    {
                        continue;
                    }
                    let (lo, hi) = if li < lj { (i as u32, j) } else { (j, i as u32) };
                    let dir = (updir[lo as usize] + updir[hi as usize])
                        .try_normalize()
                        .unwrap_or(updir[lo as usize]);
                    gaps.push((lo, hi, dir));
                }
            }
        }
    }

    // 空気圧: 層順序の方向へ、いちばん下の層を−側・いちばん上の層を+側へ押し広げる。
    let half = (lmax - lmin) * 0.5;
    let push = if pressure <= 0.0 || half <= 0.0 {
        Vec::new()
    } else {
        let mid = (lmin + lmax) * 0.5;
        let scale = pressure * PRESSURE_STEP * (1.0 - STIFFNESS_DAMPING * stiffness);
        layer
            .iter()
            .zip(&updir)
            .map(|(&l, &d)| d * ((l - mid) / half * scale))
            .collect()
    };
    Constraints {
        stretch,
        bend,
        gaps,
        push,
    }
}

/// 2頂点の距離を`rest`へ強さ`w`で戻す(質量は全頂点そろえて半分ずつ動かす)。
/// 既に満たしているときは1ビットも動かさない(たわみをかけない入力を素通しする)。
#[inline]
fn project(pos: &mut [DVec3], a: u32, b: u32, rest: f64, w: f64) {
    let (a, b) = (a as usize, b as usize);
    let d = pos[b] - pos[a];
    let l = d.length();
    if l <= EPS || l == rest {
        return;
    }
    let c = d * (w * 0.5 * (l - rest) / l);
    pos[a] += c;
    pos[b] -= c;
}

/// 下の層`a`から見て上の層`b`が`dir`方向へ最低`LAYER_GAP`だけ離れるよう押し戻す。
#[inline]
fn separate(pos: &mut [DVec3], a: u32, b: u32, dir: DVec3) {
    let (a, b) = (a as usize, b as usize);
    let s = (pos[b] - pos[a]).dot(dir);
    if s >= LAYER_GAP {
        return;
    }
    let c = dir * (0.5 * (LAYER_GAP - s));
    pos[a] -= c;
    pos[b] += c;
}

/// 拘束を`iterations`回、決まった順(空気圧→伸び→曲げ→層)で射影する。
/// 戻り値は最後まで上下が入れ替わったままだった層の組の数(SIM-014の警告用)。
pub(crate) fn run(pos: &mut [DVec3], c: &Constraints, iterations: u32) -> usize {
    for _ in 0..iterations {
        if !c.push.is_empty() {
            for (v, d) in pos.iter_mut().zip(&c.push) {
                *v += *d;
            }
        }
        for &(a, b, rest) in &c.stretch {
            project(pos, a, b, rest, 1.0);
        }
        for &(a, b, rest, w) in &c.bend {
            project(pos, a, b, rest, w);
        }
        for &(a, b, d) in &c.gaps {
            separate(pos, a, b, d);
        }
    }
    c.gaps
        .iter()
        .filter(|&&(a, b, d)| (pos[b as usize] - pos[a as usize]).dot(d) < 0.0)
        .count()
}
