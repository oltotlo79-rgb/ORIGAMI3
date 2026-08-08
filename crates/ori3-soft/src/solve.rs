//! 位置ベースの反復計算(PBD)でたわみの形を求める。
//!
//! 拘束は「反復回数固定・走査順固定」で Gauss-Seidel 射影するだけなので、
//! 同じ入力からは常に同じ結果になる(SYS-004 決定性)。層の近傍探索は
//! [`REBUILD_EVERY`]反復ごとに決まった回数だけ組み直すので、反復の途中で
//! 新しく接近した層も拾える(これも回数・順序が固定なので決定的)。
//!
//! 曲げは**符号付きの二面角**で拘束する。距離で近似すると鏡像(+θと−θ)を
//! 区別できず、紙が逆向きに折れ曲がった形を正しいと判定してしまうため。

use std::collections::HashMap;

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
pub(crate) const LAYER_GAP: f64 = 0.002;
/// 隙間拘束を組む相手を探す半径。
const SEARCH_RADIUS: f64 = LAYER_GAP * 4.0;
/// 三角形の広域探索用セル。頂点用より粗くして、細分済みの網で候補が増えすぎない
/// ようにする。各三角形のAABBはSEARCH_RADIUSぶん膨らませて入れるため、
/// このセルの一点照会だけで近い三角形を取りこぼさない。
const TRIANGLE_CELL: f64 = SEARCH_RADIUS * 8.0;
/// 1枚の大三角形が格子を埋め尽くさないための上限。上限を超えたものだけは別枠で
/// 各頂点から照会する。通常の細分済み三角形はこの経路に入らない。
const MAX_TRIANGLE_CELLS: usize = 64;
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
/// 層の近傍探索を組み直す間隔(反復数)。
const REBUILD_EVERY: u32 = 5;
/// 袋の判定を層番号で見るときの緩め幅(折り目上の点は隣接層の中間値を持つ)。
const SEAL_SLACK: f64 = 0.5;

/// 曲げ拘束1本。`v` は [辺の端a, 辺の端b, 三角形1の向かい合う頂点, 三角形2の同じ点]で、
/// `rest` は基準の形の**符号付き**二面角(ラジアン)、`w` は1反復あたりの強さ。
pub(crate) struct Bend {
    pub v: [u32; 4],
    pub rest: f64,
    pub w: f64,
}

/// 反復で使う拘束一式。層の接触だけは反復中に組み直す(それ以外は不変)。
pub(crate) struct Constraints {
    /// (頂点a, 頂点b, 初期長) — 紙が伸びない拘束
    pub stretch: Vec<(u32, u32, f64)>,
    /// 曲げ拘束(面の中のたわみ)と折り目の角度拘束
    pub bend: Vec<Bend>,
    /// 頂点ごとの平均層番号(貫通防止と袋の判定に使う)
    pub layer: Vec<f64>,
    /// 三角形ごとの層番号。頂点対三角形の接触で、面側の上下を正しく決める。
    pub tri_layer: Vec<f64>,
    /// 袋(閉じた空間)になっている層の区間。空なら膨らませない
    pub sealed: Vec<(f64, f64)>,
    /// 空気圧の1反復あたりの押し出し量(0なら膨らませない)
    pub push_step: f64,
    /// 接触相手とみなす最小の層スコア差。
    pub min_layer_diff: f64,
    /// 中間層スコアの差に応じて目標隙間を滑らかに弱めるか。
    pub scale_gap_by_layer_diff: bool,
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

/// 頂点ごとの「層が積み上がる向き」を求める(反復中も現在位置から組み直す)。
///
/// 三角形の法線を**その場で**[`canonical_axis`]へそろえて足すので、平らに畳んだ
/// 部分では重なった面が同じ向きを向き、大きく3Dに開いた部分ではその場所の平面の
/// 法線になる(表示側 `layerOffset.ts` の `stackLifts` と同じ考え方)。
fn up_field(triangles: &[[u32; 3]], pos: &[DVec3]) -> Vec<DVec3> {
    let mut dir = vec![DVec3::ZERO; pos.len()];
    for t in triangles {
        let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
        let n = canonical_axis((b - a).cross(c - a));
        for &v in t {
            dir[v as usize] += n;
        }
    }
    dir.iter()
        .map(|d| d.try_normalize().unwrap_or(DVec3::Z))
        .collect()
}

/// 頂点ごとの (平均層番号, その頂点に集まる三角形の層の最小・最大)。
fn layer_field(mesh: &RawMesh, n: usize) -> (Vec<f64>, Vec<(u32, u32)>) {
    let (mut sum, mut cnt) = (vec![0.0f64; n], vec![0.0f64; n]);
    let mut span = vec![(u32::MAX, 0u32); n];
    for (ti, t) in mesh.triangles.iter().enumerate() {
        let l = mesh.tri_layer[ti];
        for &v in t {
            let v = v as usize;
            sum[v] += f64::from(l);
            cnt[v] += 1.0;
            span[v] = (span[v].0.min(l), span[v].1.max(l));
        }
    }
    let layer = sum
        .iter()
        .zip(&cnt)
        .map(|(s, &c)| if c > 0.0 { s / c } else { 0.0 })
        .collect();
    (layer, span)
}

/// 袋(閉じた空間)になっている層の区間を求める。
///
/// 網の点は折り目をまたいで隣の面と共有されるので、**1つの点に複数の層の面が
/// 集まっていれば、そこで紙がその層の間を回り込んで塞いでいる**(袋の縁)。
/// その最小層〜最大層の間を「袋の内側」とみなし、そこだけを膨らませる。
/// 回り込みが無ければ(重なっているだけの紙・1枚の紙)袋にはならない。
fn sealed_ranges(span: &[(u32, u32)]) -> Vec<(f64, f64)> {
    let mut out: Vec<(u32, u32)> = span.iter().copied().filter(|s| s.0 < s.1).collect();
    out.sort_unstable();
    out.dedup();
    out.iter()
        .map(|&(a, b)| (f64::from(a), f64::from(b)))
        .collect()
}

/// 重なった2点の「下の層→上の層」の向き。積み上げ向きは局所的に決まるので、
/// 2点の向きが逆を向いていたら下の層の側へそろえてから平均する。
#[inline]
fn stack_dir(u0: DVec3, u1: DVec3) -> DVec3 {
    let u1 = if u0.dot(u1) < 0.0 { -u1 } else { u1 };
    (u0 + u1).try_normalize().unwrap_or(u0)
}

/// 層`a`と`b`(a<b)の間が袋の内側か。
fn is_sealed(sealed: &[(f64, f64)], a: f64, b: f64) -> bool {
    sealed
        .iter()
        .any(|&(s0, s1)| s0 - SEAL_SLACK <= a && b <= s1 + SEAL_SLACK)
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
    let (layer, span) = layer_field(mesh, pos.len());
    let soft_w = 1.0 - (1.0 - stiffness).powf(1.0 / f64::from(iterations.max(1)));

    // 網の辺ごとに、それを共有する2三角形の符号付き二面角を基準の形から読み、
    // その角度を保つ拘束にする。別々の面にまたがる辺は折り目なので強さ1.0
    // (剛体折りが求めた角度を符号ごと保つ)、同じ面の中なら stiffness。
    // (辺の小さい方, 大きい方, 向かい合う頂点, 三角形番号, 三角形での辺の向き)を
    // 整列して同じ辺をまとめる(BTreeMapより速く、順序は決定的)。
    let mut sides: Vec<(u32, u32, u32, u32, bool)> = Vec::with_capacity(mesh.triangles.len() * 3);
    for (ti, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            sides.push((a.min(b), a.max(b), t[(k + 2) % 3], ti as u32, a < b));
        }
    }
    sides.sort_unstable();
    let mut bend = Vec::new();
    let mut stretch = Vec::new();
    let mut i = 0;
    while i < sides.len() {
        let (a, b, o0, t0, fwd) = sides[i];
        stretch.push((a, b, (pos[a as usize] - pos[b as usize]).length()));
        let mut j = i + 1;
        while j < sides.len() && (sides[j].0, sides[j].1) == (a, b) {
            j += 1;
        }
        if j > i + 1 {
            let (_, _, o1, t1, _) = sides[i + 1];
            let same = mesh.tri_face[t0 as usize] == mesh.tri_face[t1 as usize];
            let w = if same { soft_w } else { 1.0 };
            // 三角形1の巻き方向に合わせた辺の向き(法線の向きをそろえるため)
            let v = if fwd { [a, b, o0, o1] } else { [b, a, o0, o1] };
            if w > 0.0 {
                let rest = dihedral(pos, v);
                bend.push(Bend { v, rest, w });
            }
        }
        i = j;
    }

    let sealed = if pressure > 0.0 {
        sealed_ranges(&span)
    } else {
        Vec::new()
    };
    Constraints {
        stretch,
        bend,
        layer,
        tri_layer: mesh.tri_layer.iter().map(|&v| f64::from(v)).collect(),
        sealed,
        push_step: pressure * PRESSURE_STEP * (1.0 - STIFFNESS_DAMPING * stiffness),
        min_layer_diff: LAYER_DIFF,
        scale_gap_by_layer_diff: false,
    }
}

/// 接触拘束。頂点対だけでなく、頂点が大きな三角形を突き抜ける場合も保持する。
#[derive(Clone, Copy)]
enum Contact {
    Vertices {
        lower: u32,
        upper: u32,
        dir: DVec3,
        sealed: bool,
        gap_scale: f64,
    },
    VertexTriangle {
        vertex: u32,
        triangle: [u32; 3],
        vertex_is_lower: bool,
        dir: DVec3,
        gap_scale: f64,
    },
}

/// 頂点対の格子とは別に、三角形のAABBを粗いセルへ入れる広域探索。
/// AABBを探索半径だけ膨らませるので、近い頂点は自分のセルを見るだけで候補を得る。
struct TriangleGrid {
    cells: HashMap<[i64; 3], Vec<u32>>,
    large: Vec<u32>,
}

impl TriangleGrid {
    fn cell_of(p: DVec3) -> [i64; 3] {
        [
            (p.x / TRIANGLE_CELL).floor() as i64,
            (p.y / TRIANGLE_CELL).floor() as i64,
            (p.z / TRIANGLE_CELL).floor() as i64,
        ]
    }

    fn new(pos: &[DVec3], triangles: &[[u32; 3]]) -> Self {
        let mut cells = HashMap::new();
        let mut large = Vec::new();
        for (ti, &triangle) in triangles.iter().enumerate() {
            let mut min = DVec3::splat(f64::INFINITY);
            let mut max = DVec3::splat(f64::NEG_INFINITY);
            for vertex in triangle {
                min = min.min(pos[vertex as usize]);
                max = max.max(pos[vertex as usize]);
            }
            let lo = Self::cell_of(min - DVec3::splat(SEARCH_RADIUS));
            let hi = Self::cell_of(max + DVec3::splat(SEARCH_RADIUS));
            let nx = (hi[0] - lo[0] + 1).max(0) as usize;
            let ny = (hi[1] - lo[1] + 1).max(0) as usize;
            let nz = (hi[2] - lo[2] + 1).max(0) as usize;
            if nx.saturating_mul(ny).saturating_mul(nz) > MAX_TRIANGLE_CELLS {
                large.push(ti as u32);
                continue;
            }
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        cells
                            .entry([x, y, z])
                            .or_insert_with(Vec::new)
                            .push(ti as u32);
                    }
                }
            }
        }
        Self { cells, large }
    }

    fn cell(&self, p: DVec3) -> Option<&[u32]> {
        self.cells.get(&Self::cell_of(p)).map(Vec::as_slice)
    }
}

/// 点から三角形への最近点と、その三角形内での重みを返す。
/// `weights`は接触を3頂点へ均等でなく配るために使う。
fn closest_point_on_triangle(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> (DVec3, [f64; 3]) {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let bc = c - b;
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + bc * w, [0.0, 1.0 - w, w]);
    }
    let denom = va + vb + vc;
    if denom.abs() <= EPS {
        return (a, [1.0, 0.0, 0.0]);
    }
    let v = vb / denom;
    let w = vc / denom;
    (a + ab * v + ac * w, [1.0 - v - w, v, w])
}

/// 現在位置から層の接触を探す。既存の頂点対に加え、頂点対三角形も一様格子で
/// 枝刈りして拾うため、頂点が互いに遠い大三角形どうしの交差も見逃さない。
fn find_contacts(
    pos: &[DVec3],
    triangles: &[[u32; 3]],
    up: &[DVec3],
    c: &Constraints,
) -> Vec<Contact> {
    let (lmin, lmax) = c
        .layer
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let mut out = Vec::new();
    if lmax - lmin < c.min_layer_diff {
        return out;
    }
    let grid = CellGrid::new(pos, SEARCH_RADIUS);
    // 頂点対で十分近い接触を見つけた点には、三角形拘束を重ねない。
    // 細分済みの通常ケースで同じ接触を何十本も二重に縛るのを避けつつ、
    // 頂点が遠い大三角形の交差だけを頂点対三角形で補う。
    let mut has_vertex_contact = vec![false; pos.len()];
    for (i, p) in pos.iter().enumerate() {
        let b = grid.cell_of(*p);
        for d in NEIGHBOR_CELLS {
            let same_cell = d == [0, 0, 0];
            for &j in grid.cell([b[0] + d[0], b[1] + d[1], b[2] + d[2]]) {
                let (li, lj) = (c.layer[i], c.layer[j as usize]);
                let layer_diff = (li - lj).abs();
                if (same_cell && j as usize <= i)
                    || layer_diff < c.min_layer_diff
                    || (pos[j as usize] - *p).length() > SEARCH_RADIUS
                {
                    continue;
                }
                let (lo, hi) = if li < lj {
                    (i as u32, j)
                } else {
                    (j, i as u32)
                };
                let dir = stack_dir(up[lo as usize], up[hi as usize]);
                has_vertex_contact[i] = true;
                has_vertex_contact[j as usize] = true;
                out.push(Contact::Vertices {
                    lower: lo,
                    upper: hi,
                    dir,
                    sealed: is_sealed(&c.sealed, li.min(lj), li.max(lj)),
                    gap_scale: if c.scale_gap_by_layer_diff {
                        layer_diff.min(1.0)
                    } else {
                        1.0
                    },
                });
            }
        }
    }
    let triangles_grid = TriangleGrid::new(pos, triangles);
    for (vertex, &p) in pos.iter().enumerate() {
        if has_vertex_contact[vertex] {
            continue;
        }
        let mut consider = |triangle_id: u32| {
            let triangle = triangles[triangle_id as usize];
            if triangle.contains(&(vertex as u32)) {
                return;
            }
            let triangle_layer = c.tri_layer[triangle_id as usize];
            let vertex_layer = c.layer[vertex];
            let layer_diff = (triangle_layer - vertex_layer).abs();
            if layer_diff < c.min_layer_diff {
                return;
            }
            let points = triangle.map(|id| pos[id as usize]);
            // 細分済みの小三角形は既存の頂点対拘束で十分に拾える。ここでは
            // 頂点が互いに遠くなり得る大三角形だけを補い、同じ紙面を過剰に
            // 二重拘束しないようにする。
            let longest_side = (points[1] - points[0])
                .length()
                .max((points[2] - points[1]).length())
                .max((points[0] - points[2]).length());
            if longest_side < TRIANGLE_CELL * 8.0 {
                return;
            }
            let (nearest, _) = closest_point_on_triangle(p, points[0], points[1], points[2]);
            if (nearest - p).length() > SEARCH_RADIUS {
                return;
            }
            let triangle_up =
                (up[triangle[0] as usize] + up[triangle[1] as usize] + up[triangle[2] as usize])
                    .try_normalize()
                    .unwrap_or(up[triangle[0] as usize]);
            let vertex_is_lower = vertex_layer < triangle_layer;
            let dir = if vertex_is_lower {
                stack_dir(up[vertex], triangle_up)
            } else {
                stack_dir(triangle_up, up[vertex])
            };
            out.push(Contact::VertexTriangle {
                vertex: vertex as u32,
                triangle,
                vertex_is_lower,
                dir,
                gap_scale: if c.scale_gap_by_layer_diff {
                    layer_diff.min(1.0)
                } else {
                    1.0
                },
            });
        };
        if let Some(candidates) = triangles_grid.cell(p) {
            for &triangle_id in candidates {
                consider(triangle_id);
            }
        }
        for &triangle_id in &triangles_grid.large {
            consider(triangle_id);
        }
    }
    out
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

/// 4頂点の符号付き二面角(ラジアン、−π〜π)。`v`は[辺の端a, 辺の端b,
/// 三角形1(a→bの巻き)の頂点c, 三角形2(b→aの巻き)の頂点d]。
/// 平らなら0、cの側が`a→b`の右ねじ向きへ持ち上がると負になる。
#[inline]
fn dihedral(pos: &[DVec3], v: [u32; 4]) -> f64 {
    let p = v.map(|i| pos[i as usize]);
    let e = p[1] - p[0];
    let (n1, n2) = (e.cross(p[2] - p[0]), (p[3] - p[1]).cross(e));
    let (el, l1, l2) = (e.length(), n1.length(), n2.length());
    if el <= EPS || l1 <= EPS || l2 <= EPS {
        return 0.0;
    }
    let (u1, u2) = (n1 / l1, n2 / l2);
    u1.cross(u2).dot(e / el).atan2(u1.dot(u2))
}

/// 曲げ拘束を1本射影する(質量は全頂点そろえる)。角度の差が0なら1ビットも動かさない。
///
/// 二面角θの勾配は、向かい合う頂点では「その頂点から辺までの距離ぶんの逆数×法線」、
/// 辺の両端では平行移動と辺まわりの回転で不変になるよう2点へ配分したものになる。
#[inline]
fn project_bend(pos: &mut [DVec3], b: &Bend) {
    let i = b.v.map(|k| k as usize);
    let p = b.v.map(|k| pos[k as usize]);
    let e = p[1] - p[0];
    let (n1, n2) = (e.cross(p[2] - p[0]), (p[3] - p[1]).cross(e));
    let (e2, l1, l2) = (e.length_squared(), n1.length(), n2.length());
    if e2 <= EPS || l1 <= EPS || l2 <= EPS {
        return;
    }
    let el = e2.sqrt();
    let (u1, u2) = (n1 / l1, n2 / l2);
    let theta = u1.cross(u2).dot(e / el).atan2(u1.dot(u2));
    let mut diff = theta - b.rest;
    if diff == 0.0 {
        return;
    }
    diff -= std::f64::consts::TAU * (diff / std::f64::consts::TAU).round();
    let (g2, g3) = (u1 * (-el / l1), u2 * (-el / l2));
    let (w2, w3) = ((p[2] - p[0]).dot(e) / e2, (p[3] - p[0]).dot(e) / e2);
    let g0 = g2 * (w2 - 1.0) + g3 * (w3 - 1.0);
    let g1 = g2 * -w2 + g3 * -w3;
    let g = [g0, g1, g2, g3];
    let denom: f64 = g.iter().map(|v| v.length_squared()).sum();
    if denom <= EPS {
        return;
    }
    let s = -b.w * diff / denom;
    for k in 0..4 {
        pos[i[k]] += g[k] * s;
    }
}

/// 袋の内側の重なりから、頂点ごとの1反復ぶんの押し出しを作る。下の層は−・上の層は+の
/// 向きへ押す。どの層とも重なっていない紙(重なりに入っていない頂点)は動かさない。
///
/// `stack` は**基準の形で重なっていた**層の組(反復中に層が離れても膨らみ続ける
/// ように、組は最初のまま固定する)。押し出す向きだけは今の形の法線から取り直す。
fn pressure_push(n: usize, stack: &[(u32, u32, bool)], up: &[DVec3], step: f64) -> Vec<DVec3> {
    let (mut sum, mut cnt) = (vec![DVec3::ZERO; n], vec![0.0f64; n]);
    for &(a, b, sealed) in stack {
        cnt[a as usize] += 1.0;
        cnt[b as usize] += 1.0;
        if sealed {
            let d = stack_dir(up[a as usize], up[b as usize]);
            sum[a as usize] -= d;
            sum[b as usize] += d;
        }
    }
    sum.iter()
        .zip(&cnt)
        .map(|(s, &c)| {
            if c > 0.0 {
                *s * (step / c)
            } else {
                DVec3::ZERO
            }
        })
        .collect()
}

/// 下の層`a`から見て上の層`b`が`dir`方向へ最低`LAYER_GAP`だけ離れるよう押し戻す。
#[inline]
fn separate(pos: &mut [DVec3], a: u32, b: u32, dir: DVec3, gap: f64) {
    let (a, b) = (a as usize, b as usize);
    let s = (pos[b] - pos[a]).dot(dir);
    if s >= gap {
        return;
    }
    let c = dir * (0.5 * (gap - s));
    pos[a] -= c;
    pos[b] += c;
}

/// 頂点と三角形を、下の層から上の層へ最低`LAYER_GAP`だけ離す。
/// 最近点の重みで三角形側へ配るため、接触点が辺上・内部のどちらでも不自然に
/// 1頂点だけが引き伸ばされない。
fn separate_vertex_triangle(
    pos: &mut [DVec3],
    vertex: u32,
    triangle: [u32; 3],
    vertex_is_lower: bool,
    dir: DVec3,
    target_gap: f64,
) {
    let point = pos[vertex as usize];
    let triangle_points = triangle.map(|id| pos[id as usize]);
    let (nearest, weights) = closest_point_on_triangle(
        point,
        triangle_points[0],
        triangle_points[1],
        triangle_points[2],
    );
    let gap = if vertex_is_lower {
        (nearest - point).dot(dir)
    } else {
        (point - nearest).dot(dir)
    };
    if gap >= target_gap {
        return;
    }
    let correction = dir * (0.5 * (target_gap - gap));
    if vertex_is_lower {
        pos[vertex as usize] -= correction;
        for (id, weight) in triangle.into_iter().zip(weights) {
            pos[id as usize] += correction * weight;
        }
    } else {
        pos[vertex as usize] += correction;
        for (id, weight) in triangle.into_iter().zip(weights) {
            pos[id as usize] -= correction * weight;
        }
    }
}

fn project_contact(pos: &mut [DVec3], contact: Contact, gap: f64) {
    match contact {
        Contact::Vertices {
            lower,
            upper,
            dir,
            gap_scale,
            ..
        } => separate(pos, lower, upper, dir, gap * gap_scale),
        Contact::VertexTriangle {
            vertex,
            triangle,
            vertex_is_lower,
            dir,
            gap_scale,
        } => separate_vertex_triangle(pos, vertex, triangle, vertex_is_lower, dir, gap * gap_scale),
    }
}

/// 現在見つかる接触について、層順序に反した食い込みの量を合計する。
fn penetration_depth(pos: &[DVec3], contact: Contact) -> f64 {
    match contact {
        Contact::Vertices {
            lower, upper, dir, ..
        } => (-(pos[upper as usize] - pos[lower as usize]).dot(dir)).max(0.0),
        Contact::VertexTriangle {
            vertex,
            triangle,
            vertex_is_lower,
            dir,
            ..
        } => {
            let point = pos[vertex as usize];
            let points = triangle.map(|id| pos[id as usize]);
            let (nearest, _) = closest_point_on_triangle(point, points[0], points[1], points[2]);
            let signed = if vertex_is_lower {
                (nearest - point).dot(dir)
            } else {
                (point - nearest).dot(dir)
            };
            (-signed).max(0.0)
        }
    }
}

/// 接触補正の前後を検査するための食い込み量。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PenetrationMeasure {
    pub count: usize,
    pub total_depth: f64,
    pub max_depth: f64,
}

pub(crate) fn measure_penetration(
    pos: &[DVec3],
    triangles: &[[u32; 3]],
    c: &Constraints,
) -> PenetrationMeasure {
    let up = up_field(triangles, pos);
    let contacts = find_contacts(pos, triangles, &up, c);
    contacts
        .iter()
        .fold(PenetrationMeasure::default(), |mut m, &contact| {
            let depth = penetration_depth(pos, contact);
            if depth > 0.0 {
                m.count += 1;
                m.total_depth += depth;
                m.max_depth = m.max_depth.max(depth);
            }
            m
        })
}

fn is_penetrating(pos: &[DVec3], contact: Contact) -> bool {
    match contact {
        Contact::Vertices {
            lower, upper, dir, ..
        } => (pos[upper as usize] - pos[lower as usize]).dot(dir) < 0.0,
        Contact::VertexTriangle {
            vertex,
            triangle,
            vertex_is_lower,
            dir,
            ..
        } => {
            let point = pos[vertex as usize];
            let points = triangle.map(|id| pos[id as usize]);
            let (nearest, _) = closest_point_on_triangle(point, points[0], points[1], points[2]);
            if vertex_is_lower {
                (nearest - point).dot(dir) < 0.0
            } else {
                (point - nearest).dot(dir) < 0.0
            }
        }
    }
}

/// 拘束を`iterations`回、決まった順(空気圧→伸び→曲げ→層)で射影する。
/// 層の接触と積み上げ向きは[`REBUILD_EVERY`]反復ごとに現在位置から組み直すので、
/// 反復の途中で新しく接近した層も拾える。
/// 戻り値は最後まで上下が入れ替わったままだった層の組の数(SIM-014の警告用)。
pub(crate) fn run(
    pos: &mut [DVec3],
    triangles: &[[u32; 3]],
    c: &Constraints,
    iterations: u32,
) -> usize {
    run_with_gap(pos, triangles, c, iterations, LAYER_GAP)
}

/// [`run`] と同じ拘束を、接触の目標隙間だけ縮めて適用する。
/// 折り上がり直前に層順序が切り替わるときの滑らかな減衰に使う。
pub(crate) fn run_with_gap(
    pos: &mut [DVec3],
    triangles: &[[u32; 3]],
    c: &Constraints,
    iterations: u32,
    contact_gap: f64,
) -> usize {
    let contact_gap = contact_gap.clamp(0.0, LAYER_GAP);
    let mut contacts = Vec::new();
    let mut push = Vec::new();
    // 積み上げ向きの**符号**は基準の形(剛体折りの結果)で決める。組み直しでは
    // 向きの大きさだけを今の形に合わせ、符号は最初のものへそろえる。そうしないと
    // 紙が曲がるにつれて上下の判定が裏返り、層どうしが押し合ってしまう。
    let mut anchor: Vec<DVec3> = Vec::new();
    // 空気圧をかける重なりの組(基準の形で決めて反復中は変えない)
    let mut stack: Vec<(u32, u32, bool)> = Vec::new();
    for it in 0..iterations {
        if it % REBUILD_EVERY == 0 {
            let mut up = up_field(triangles, pos);
            if anchor.is_empty() {
                anchor = up.clone();
            } else {
                for (u, a) in up.iter_mut().zip(&anchor) {
                    if u.dot(*a) < 0.0 {
                        *u = -*u;
                    }
                }
            }
            contacts = find_contacts(pos, triangles, &up, c);
            if stack.is_empty() {
                stack = contacts
                    .iter()
                    .filter_map(|contact| match contact {
                        Contact::Vertices {
                            lower,
                            upper,
                            sealed,
                            ..
                        } => Some((*lower, *upper, *sealed)),
                        Contact::VertexTriangle { .. } => None,
                    })
                    .collect();
            }
            push = if c.push_step > 0.0 && !stack.is_empty() {
                pressure_push(pos.len(), &stack, &up, c.push_step)
            } else {
                Vec::new()
            };
        }
        for (v, d) in pos.iter_mut().zip(&push) {
            *v += *d;
        }
        for &(a, b, rest) in &c.stretch {
            project(pos, a, b, rest, 1.0);
        }
        for b in &c.bend {
            project_bend(pos, b);
        }
        for &contact in &contacts {
            project_contact(pos, contact, contact_gap);
        }
    }
    contacts
        .iter()
        .filter(|&&contact| is_penetrating(pos, contact))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 平らに並べた三角形2枚を、目標の二面角`target`だけ曲げた結果を返す。
    /// (頂点0,1が共有辺、2と3が向かい合う頂点。距離で近似していたときは
    /// +θと−θで同じ距離になるため、この2つを区別できなかった)
    fn fold_two_triangles(target: f64) -> (Vec<DVec3>, f64) {
        let mut pos = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
            DVec3::new(0.0, -1.0, 0.0),
        ];
        let d = 2.0f64.sqrt();
        let c = Constraints {
            stretch: vec![(0, 1, 1.0), (0, 2, 1.0), (1, 2, d), (0, 3, 1.0), (1, 3, d)],
            bend: vec![Bend {
                v: [0, 1, 2, 3],
                rest: target,
                w: 1.0,
            }],
            layer: vec![0.0; 4],
            tri_layer: vec![0.0, 0.0],
            sealed: Vec::new(),
            push_step: 0.0,
            min_layer_diff: LAYER_DIFF,
            scale_gap_by_layer_diff: false,
        };
        let triangles = [[0u32, 1, 2], [1, 0, 3]];
        run(&mut pos, &triangles, &c, 60);
        let angle = dihedral(&pos, [0, 1, 2, 3]);
        (pos, angle)
    }

    #[test]
    fn the_bending_constraint_tells_plus_and_minus_apart() {
        let quarter = std::f64::consts::FRAC_PI_2;
        let (plus, a) = fold_two_triangles(quarter);
        let (minus, b) = fold_two_triangles(-quarter);
        assert!((a - quarter).abs() < 0.01, "+90°に折れる: {a}");
        assert!((b + quarter).abs() < 0.01, "−90°に折れる: {b}");
        // 形そのものも鏡像になっている(向かい合う頂点が逆側へ立ち上がる)
        assert!(
            plus[2].z * minus[2].z < 0.0 && (plus[2].z + minus[2].z).abs() < 1e-9,
            "鏡像の形になる: {:?} {:?}",
            plus[2],
            minus[2]
        );
    }

    /// 2層ぶんの三角形の網。`shared`なら2枚が2点を共有する(折り目でつながった袋)。
    fn two_layers(shared: bool) -> RawMesh {
        let mut positions = vec![DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::new(1.0, 1.0, 0.0)];
        let second = if shared {
            [1, 2, 3]
        } else {
            positions.extend([DVec3::X, DVec3::Y, DVec3::new(1.0, 1.0, 0.0)]);
            [4, 5, 6]
        };
        RawMesh {
            positions,
            triangles: vec![[0, 1, 2], second],
            tri_face: vec![0, 1],
            tri_layer: vec![0, 1],
            corners: Default::default(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn a_bag_is_only_where_the_paper_wraps_around_the_layers() {
        let wrapped = two_layers(true);
        let (_, span) = layer_field(&wrapped, wrapped.positions.len());
        let sealed = sealed_ranges(&span);
        assert_eq!(sealed, vec![(0.0, 1.0)], "折り目でつながった2層は袋の内側");
        assert!(is_sealed(&sealed, 0.0, 1.0));

        let loose = two_layers(false);
        let (_, span) = layer_field(&loose, loose.positions.len());
        let sealed = sealed_ranges(&span);
        assert!(sealed.is_empty(), "重なっているだけの2層は袋ではない");
        assert!(!is_sealed(&sealed, 0.0, 1.0));
    }

    #[test]
    fn a_flat_rest_angle_keeps_the_paper_flat() {
        let (pos, angle) = fold_two_triangles(0.0);
        assert!(angle == 0.0, "平らなまま: {angle}");
        assert!(pos.iter().all(|p| p.z == 0.0), "1ビットも動かない: {pos:?}");
    }

    #[test]
    fn vertex_triangle_contacts_catch_large_triangles_without_near_vertices() {
        // 上の三角形の3頂点は下の三角形のどの頂点からもSEARCH_RADIUSより遠い。
        // それでも三角形の内側を突き抜けているので、頂点対だけでは見逃す。
        let mut pos = vec![
            DVec3::new(-1.0, -1.0, 0.0),
            DVec3::new(1.0, -1.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
            DVec3::new(-0.5, -0.3, -0.0005),
            DVec3::new(0.6, -0.2, -0.0005),
            DVec3::new(0.0, 0.7, -0.0005),
        ];
        let triangles = [[0, 1, 2], [3, 4, 5]];
        let c = Constraints {
            stretch: Vec::new(),
            bend: Vec::new(),
            layer: vec![0.0, 0.0, 0.0, 2.0, 2.0, 2.0],
            tri_layer: vec![0.0, 2.0],
            sealed: Vec::new(),
            push_step: 0.0,
            min_layer_diff: LAYER_DIFF,
            scale_gap_by_layer_diff: false,
        };
        let up = up_field(&triangles, &pos);
        let contacts = find_contacts(&pos, &triangles, &up, &c);
        assert!(
            contacts
                .iter()
                .any(|contact| matches!(contact, Contact::VertexTriangle { .. })),
            "大三角形との接触を検出していない"
        );
        assert!(
            !contacts
                .iter()
                .any(|contact| matches!(contact, Contact::Vertices { .. })),
            "この配置は頂点対では検出できないはず"
        );

        assert_eq!(run(&mut pos, &triangles, &c, 1), 0, "貫通が残っている");
    }
}
