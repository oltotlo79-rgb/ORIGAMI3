//! 面の細分割: 剛体折りが求めた面の姿勢を基準に、各面を細かい三角形へ分ける。
//!
//! 面の境界(折り線)上に生まれる頂点は、隣の面と**同じ鍵**([`MeshKey`])で
//! 引くことで共有する(共有しないとたわみ計算で紙が破れて見える)。鍵は
//! 面ID・辺ID・分割番号だけから決まるので、走査順に依らず決定的。

use std::collections::BTreeMap;

use glam::{DVec2, DVec3};
use ori3_cp::Face;
use ori3_model::{CreasePattern, EPS, EdgeId, FaceId, Frame3D, VertexId};

/// 網の頂点を面をまたいで共有するための鍵。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MeshKey {
    /// 展開図の頂点そのもの(その頂点に集まる全ての面で共有)
    Corner(VertexId),
    /// CP辺上のk番目の分割点(kは小さい方のVertexId側から数える)
    OnEdge(EdgeId, u32),
    /// 面の内部を横切る対角線上のk番目(kは小さい方の局所番号側から数える)
    Diag(FaceId, u32, u32, u32),
    /// 三角形の内部(その三角形だけのもの)
    Inner(FaceId, u32, u32, u32),
}

/// 細分割の生の結果。
pub(crate) struct RawMesh {
    pub positions: Vec<DVec3>,
    pub triangles: Vec<[u32; 3]>,
    pub tri_face: Vec<FaceId>,
    pub tri_layer: Vec<u32>,
    pub warnings: Vec<String>,
}

/// 反時計回りの単純多角形を耳切り法で三角形へ分ける(局所頂点番号の三つ組)。
/// 凹んだ面でも多角形の外へはみ出さない。退化してこれ以上切れなくなったら
/// 残りを扇形で切って返す(止めない)。
fn ear_clip(poly: &[DVec2]) -> Vec<[usize; 3]> {
    let m = poly.len();
    let mut out = Vec::with_capacity(m.saturating_sub(2));
    if m < 3 {
        return out;
    }
    let mut idx: Vec<usize> = (0..m).collect();
    let cross = |a: DVec2, b: DVec2, c: DVec2| (b - a).perp_dot(c - a);
    while idx.len() > 3 {
        let n = idx.len();
        let mut cut = None;
        for i in 0..n {
            let (a, b, c) = (idx[(i + n - 1) % n], idx[i], idx[(i + 1) % n]);
            if cross(poly[a], poly[b], poly[c]) <= EPS {
                continue;
            }
            let blocked = idx.iter().any(|&t| {
                t != a
                    && t != b
                    && t != c
                    && cross(poly[a], poly[b], poly[t]) >= 0.0
                    && cross(poly[b], poly[c], poly[t]) >= 0.0
                    && cross(poly[c], poly[a], poly[t]) >= 0.0
            });
            if !blocked {
                cut = Some(i);
                break;
            }
        }
        let Some(i) = cut else { break };
        let n = idx.len();
        out.push([idx[(i + n - 1) % n], idx[i], idx[(i + 1) % n]]);
        idx.remove(i);
    }
    for i in 1..idx.len().saturating_sub(1) {
        out.push([idx[0], idx[i], idx[i + 1]]);
    }
    out
}

/// 面の局所頂点番号 p→q を結ぶ線分上の、p側からk番目(0<k<n)の点の鍵。
/// 多角形の境界辺ならCP辺IDで隣の面と共有し、対角線なら面の中だけで共有する。
fn side_key(face: &Face, p: usize, q: usize, k: u32, n: u32) -> MeshKey {
    let m = face.vertices.len();
    let (va, vb) = (face.vertices[p], face.vertices[q]);
    let eid: Option<EdgeId> = if (p + 1) % m == q {
        Some(face.edges[p])
    } else if (q + 1) % m == p {
        Some(face.edges[q])
    } else {
        None
    };
    if let Some(e) = eid
        && va != vb
    {
        return MeshKey::OnEdge(e, if va < vb { k } else { n - k });
    }
    let (lo, hi, kk) = if p < q { (p, q, k) } else { (q, p, n - k) };
    MeshKey::Diag(face.id, lo as u32, hi as u32, kk)
}

/// 三角形1枚の格子点の鍵。(a,b)は P = A + (B−A)a/n + (C−A)b/n の添字。
fn grid_key(face: &Face, tri: usize, c: [usize; 3], a: u32, b: u32, n: u32) -> MeshKey {
    match (a, b) {
        (0, 0) => MeshKey::Corner(face.vertices[c[0]]),
        _ if a == n => MeshKey::Corner(face.vertices[c[1]]),
        _ if b == n => MeshKey::Corner(face.vertices[c[2]]),
        (_, 0) => side_key(face, c[0], c[1], a, n),
        (0, _) => side_key(face, c[0], c[2], b, n),
        _ if a + b == n => side_key(face, c[1], c[2], b, n),
        _ => MeshKey::Inner(face.id, tri as u32, a, b),
    }
}

/// 展開図・面・剛体折りのフレームから、各三角形の1辺を`div`等分した網を作る。
/// 面をまたいで共有される点は、寄与した全ての面の位置の平均に置く(剛体折りが
/// 収束していれば一致するので、平均は食い違いをならすだけ)。
pub(crate) fn build_mesh(cp: &CreasePattern, faces: &[Face], frame: &Frame3D, div: u32) -> RawMesh {
    let n = div.max(1);
    let vpos: BTreeMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();
    let f3d: BTreeMap<FaceId, &ori3_model::Face3D> =
        frame.faces.iter().map(|f| (f.face, f)).collect();
    let mut index: BTreeMap<MeshKey, u32> = BTreeMap::new();
    let (mut sum, mut cnt): (Vec<DVec3>, Vec<f64>) = (Vec::new(), Vec::new());
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let (mut tri_face, mut tri_layer) = (Vec::new(), Vec::new());
    let (mut warnings, mut skipped) = (Vec::new(), 0usize);

    for face in faces {
        let poly2: Vec<DVec2> = face
            .vertices
            .iter()
            .filter_map(|v| vpos.get(v).copied())
            .collect();
        let ok = f3d.get(&face.id).filter(|fd| {
            face.vertices.len() >= 3
                && fd.polygon.len() == face.vertices.len()
                && poly2.len() == face.vertices.len()
        });
        let Some(fd) = ok else {
            skipped += 1;
            continue;
        };
        let poly3: Vec<DVec3> = fd.polygon.iter().map(|&p| DVec3::from(p)).collect();
        for (ti, c) in ear_clip(&poly2).iter().enumerate() {
            let (a3, b3, c3) = (poly3[c[0]], poly3[c[1]], poly3[c[2]]);
            let mut grid = vec![u32::MAX; ((n + 1) * (n + 1)) as usize];
            for a in 0..=n {
                for b in 0..=(n - a) {
                    let key = grid_key(face, ti, *c, a, b, n);
                    let p = a3
                        + (b3 - a3) * (f64::from(a) / f64::from(n))
                        + (c3 - a3) * (f64::from(b) / f64::from(n));
                    let id = *index.entry(key).or_insert_with(|| {
                        sum.push(DVec3::ZERO);
                        cnt.push(0.0);
                        (sum.len() - 1) as u32
                    });
                    sum[id as usize] += p;
                    cnt[id as usize] += 1.0;
                    grid[(a * (n + 1) + b) as usize] = id;
                }
            }
            let at = |a: u32, b: u32| grid[(a * (n + 1) + b) as usize];
            for a in 0..n {
                for b in 0..(n - a) {
                    triangles.push([at(a, b), at(a + 1, b), at(a, b + 1)]);
                    if a + b + 2 <= n {
                        triangles.push([at(a + 1, b), at(a + 1, b + 1), at(a, b + 1)]);
                    }
                }
            }
            while tri_face.len() < triangles.len() {
                tri_face.push(face.id);
                tri_layer.push(fd.layer);
            }
        }
    }
    if skipped > 0 {
        warnings.push(format!(
            "{skipped}個の面は立体の姿勢が求まっていないため、たわみ計算から外しました"
        ));
    }
    RawMesh {
        positions: sum.iter().zip(&cnt).map(|(s, &c)| *s / c).collect(),
        triangles,
        tri_face,
        tri_layer,
        warnings,
    }
}
