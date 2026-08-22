//! 平坦に畳まれた状態(各面の配置と層順序)と、層順序の永続化用の面内代表点。
//!
//! 面ID([`FaceId`])は面抽出のたびに再採番される導出値なので、層順序をそのまま
//! 保存すると展開図の編集で意味が壊れる。そのため層順序は「CP座標系における面の
//! 内部代表点」の列(`FoldStep.layer_order`)として永続化し、読み込み時に現在の
//! 面IDへ解決する。

use std::collections::{HashMap, HashSet};

use glam::DVec2;
use ori3_cp::Face;
use ori3_geometry::{Isometry2, dist_point_segment};
use ori3_model::{CreasePattern, EPS, FaceId, VertexId};

/// 平坦に畳まれた状態: 各面のCP座標→畳んだ平面座標への等長変換と、層順序。
#[derive(Clone, Debug, PartialEq)]
pub struct FlatState {
    /// 面ID → その面のCP座標を畳んだ平面座標へ写す等長変換。
    pub placements: HashMap<FaceId, Isometry2>,
    /// 層順序(下→上)。面抽出で得た全ての面をちょうど1回ずつ含む。
    pub order: Vec<FaceId>,
}

impl FlatState {
    /// 平らな初期状態。全面が恒等変換で、層順序は面ID昇順。
    ///
    /// `cp` は引数の対称性のために受け取るが、初期状態の構築には使わない。
    pub fn initial(_cp: &CreasePattern, faces: &[Face]) -> FlatState {
        let mut order: Vec<FaceId> = faces.iter().map(|f| f.id).collect();
        order.sort_unstable();
        FlatState {
            placements: faces
                .iter()
                .map(|f| (f.id, Isometry2::identity()))
                .collect(),
            order,
        }
    }

    /// 永続化された代表点リスト(下→上)を現在の面IDへ解決する。
    ///
    /// 各点は、それを含む面のうち未使用で `faces` の並び順で最初のものへ解決する
    /// (`extract_faces` は面ID昇順で返すため、実質は最も面IDが小さいもの)。
    /// どの面にも入らない点と、既に使われた面しか指さない点はその層を飛ばし、
    /// 警告(日本語)として返す。解決されなかった面は元の相対順(`faces` の順)
    /// を保って末尾側に補うため、戻り値には常に全ての面がちょうど1回ずつ含まれる。
    pub fn resolve_order(
        cp: &CreasePattern,
        faces: &[Face],
        points: &[[f64; 2]],
    ) -> (Vec<FaceId>, Vec<String>) {
        // 境界多角形は面ごとに一度だけ組み立てる(点の数×面の数だけ組み直さない)。
        let pos = vertex_positions(cp);
        let polys: Vec<Vec<DVec2>> = faces.iter().map(|f| face_polygon(&pos, f)).collect();

        let mut order: Vec<FaceId> = Vec::with_capacity(faces.len());
        let mut used: HashSet<FaceId> = HashSet::with_capacity(faces.len());
        let mut warnings: Vec<String> = Vec::new();
        for &p in points {
            let q = DVec2::from(p);
            let hit = faces
                .iter()
                .zip(&polys)
                .find(|(f, poly)| !used.contains(&f.id) && point_in_polygon(poly, q));
            match hit {
                Some((f, _)) => {
                    used.insert(f.id);
                    order.push(f.id);
                }
                None if polys.iter().any(|poly| point_in_polygon(poly, q)) => warnings.push(format!(
                    "層順序の代表点 ({:.6}, {:.6}) が指す面は既に別の層で使われているため、この層を飛ばしました",
                    p[0], p[1]
                )),
                None => warnings.push(format!(
                    "層順序の代表点 ({:.6}, {:.6}) はどの面にも含まれないため、この層を飛ばしました",
                    p[0], p[1]
                )),
            }
        }
        // 解決されなかった面は元の相対順を保って末尾側に補う。
        for f in faces {
            if !used.contains(&f.id) {
                order.push(f.id);
            }
        }
        (order, warnings)
    }

    /// 現在の層順序を代表点リスト(下→上)へ変換する
    /// (`FoldStep.layer_order` への永続化用)。
    ///
    /// `faces` に存在しない面IDは飛ばす。
    pub fn to_layer_points(&self, cp: &CreasePattern, faces: &[Face]) -> Vec<[f64; 2]> {
        let pos = vertex_positions(cp);
        self.order
            .iter()
            .filter_map(|id| faces.iter().find(|f| f.id == *id))
            .map(|f| polygon_representative_point(&face_polygon(&pos, f)))
            .collect()
    }
}

/// 面の内部代表点(CP座標系)。凹面(スリットのある面)でも面の内部に落ちる。
///
/// 面の境界多角形からスリットの往復による尖りを取り除いて単純多角形にし、
/// 耳刈り(ear clipping)で最初に見つかった耳(三角形)の重心を返す。
/// 走査順が決まっているため、同じ面からは常に同じ点が返る(決定的)。
pub fn representative_point(cp: &CreasePattern, face: &Face) -> [f64; 2] {
    polygon_representative_point(&face_polygon(&vertex_positions(cp), face))
}

fn polygon_representative_point(boundary: &[DVec2]) -> [f64; 2] {
    let poly = simple_polygon(boundary);
    let n = poly.len();
    let mut first_ear: Option<DVec2> = None;
    if n >= 3 {
        for k in 0..n {
            let a = poly[(k + n - 1) % n];
            let b = poly[k];
            let c = poly[(k + 1) % n];
            // 反時計回りの多角形なので、凸な角は外積が正になる。
            if (b - a).perp_dot(c - b) <= EPS * EPS {
                continue;
            }
            // 三角形の中に他の頂点が入っていれば耳ではない
            // (頂点と重なる点は、スリット等で重複した頂点なので除外する)。
            let blocked = poly.iter().enumerate().any(|(j, &q)| {
                j != k
                    && j != (k + n - 1) % n
                    && j != (k + 1) % n
                    && (q - a).length() > EPS
                    && (q - b).length() > EPS
                    && (q - c).length() > EPS
                    && point_in_triangle(q, a, b, c)
            });
            if blocked {
                continue;
            }
            let g = (a + b + c) / 3.0;
            first_ear.get_or_insert(g);
            // 尖りを潰した多角形が元の面と食い違う場合に備え、元の境界で確かめる。
            if strictly_inside(boundary, g) {
                return [g.x, g.y];
            }
        }
    }
    // 耳が見つからない/元の境界の内部に落ちない退化した面への防御。
    let g = first_ear.unwrap_or_else(|| {
        if boundary.is_empty() {
            DVec2::ZERO
        } else {
            boundary.iter().sum::<DVec2>() / boundary.len() as f64
        }
    });
    [g.x, g.y]
}

/// 点が面の内部(境界含む)にあるか。凹面(スリットのある面)にも対応する。
/// 境界からEPS以内の点は内部として扱う。
pub fn point_in_face(cp: &CreasePattern, face: &Face, p: [f64; 2]) -> bool {
    let poly = face_polygon(&vertex_positions(cp), face);
    point_in_polygon(&poly, DVec2::from(p))
}

/// Return the layers covering `point` in the folded plane, ordered bottom to top.
///
/// A face is tested after pulling the folded-plane point back through that
/// face's placement.  This is the stable selection primitive needed by book
/// instructions such as "the front four sheets" or "the second sheet from
/// the front"; a slice of the global layer order is not sufficient when only
/// some of those layers cover the indicated point.
#[must_use]
pub fn layers_at_point(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    point: [f64; 2],
) -> Vec<FaceId> {
    let folded = DVec2::from(point);
    state
        .order
        .iter()
        .copied()
        .filter(|face_id| {
            let Some(face) = faces.iter().find(|face| face.id == *face_id) else {
                return false;
            };
            let Some(placement) = state.placements.get(face_id) else {
                return false;
            };
            let local = placement.inverse().apply(folded);
            point_in_face(cp, face, [local.x, local.y])
        })
        .collect()
}

/// Select `count` local layers from the top, after skipping `skip` top layers.
///
/// The returned order is still bottom-to-top so it can be passed directly to
/// layer-motion operations without silently reversing a packet.
#[must_use]
pub fn layers_from_top_at_point(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    point: [f64; 2],
    skip: usize,
    count: usize,
) -> Vec<FaceId> {
    let layers = layers_at_point(cp, faces, state, point);
    let end = layers.len().saturating_sub(skip);
    let start = end.saturating_sub(count);
    layers[start..end].to_vec()
}

/// 多角形の内部(境界からEPS以内を含む)に点があるか。
pub(crate) fn point_in_polygon(poly: &[DVec2], p: DVec2) -> bool {
    on_boundary(poly, p) || crossing_number_is_odd(poly, p)
}

pub(crate) fn vertex_positions(cp: &CreasePattern) -> HashMap<VertexId, DVec2> {
    cp.vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect()
}

/// 面の境界を一周する頂点座標列(CP座標系)。存在しない頂点は飛ばす
/// (壊れたCPでもpanicさせない。extract_faces / validate と同じ方針)。
pub(crate) fn face_polygon(pos: &HashMap<VertexId, DVec2>, face: &Face) -> Vec<DVec2> {
    face.vertices
        .iter()
        .filter_map(|id| pos.get(id).copied())
        .collect()
}

/// 耳刈りできる単純多角形にする: 連続した重複点を潰し、スリットの往復でできる
/// 尖り(前後の点が一致する頂点)を無くなるまで取り除く。
/// 念のため反時計回りに揃える。
fn simple_polygon(boundary: &[DVec2]) -> Vec<DVec2> {
    let mut pts = dedup_consecutive(boundary);
    loop {
        let n = pts.len();
        if n < 3 {
            break;
        }
        // 尖りの先端(前後の点が一致する頂点)と、その次の重複点を取り除く。
        let Some(tip) = (0..n).find(|&i| (pts[(i + n - 1) % n] - pts[(i + 1) % n]).length() <= EPS)
        else {
            break;
        };
        let dup = (tip + 1) % n;
        pts = pts
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != tip && i != dup)
            .map(|(_, &p)| p)
            .collect();
        // 枝分かれしたスリットでは尖りを取り除いた跡で重複点が隣り合うことがあるので、
        // 長さ0の辺を残さないよう毎回潰し直す(点数は必ず減るので必ず止まる)。
        pts = dedup_consecutive(&pts);
    }
    if signed_area(&pts) < 0.0 {
        pts.reverse();
    }
    pts
}

/// 連続した重複点(一周して隣り合う始点と終点の重複を含む)を潰す。
fn dedup_consecutive(pts: &[DVec2]) -> Vec<DVec2> {
    let mut out: Vec<DVec2> = Vec::with_capacity(pts.len());
    for &p in pts {
        if out.last().is_none_or(|&q| (q - p).length() > EPS) {
            out.push(p);
        }
    }
    while out.len() > 1 && (out[0] - out[out.len() - 1]).length() <= EPS {
        out.pop();
    }
    out
}

fn signed_area(poly: &[DVec2]) -> f64 {
    let n = poly.len();
    0.5 * (0..n)
        .map(|i| poly[i].perp_dot(poly[(i + 1) % n]))
        .sum::<f64>()
}

/// 反時計回りの三角形の内部(境界含む)に点があるか。
fn point_in_triangle(p: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    (b - a).perp_dot(p - a) >= -EPS
        && (c - b).perp_dot(p - b) >= -EPS
        && (a - c).perp_dot(p - c) >= -EPS
}

/// 点が多角形の境界からEPS以内にあるか。
fn on_boundary(poly: &[DVec2], p: DVec2) -> bool {
    let n = poly.len();
    (0..n).any(|i| dist_point_segment(p, poly[i], poly[(i + 1) % n]) <= EPS)
}

/// 境界に乗らず、多角形の内部にあるか。
fn strictly_inside(poly: &[DVec2], p: DVec2) -> bool {
    !on_boundary(poly, p) && crossing_number_is_odd(poly, p)
}

/// 交差数の偶奇判定(+x方向への半直線)。凹面に対応し、スリットのように同じ辺が
/// 往復して2回現れる境界では交差が打ち消し合うため正しく内部と判定される。
fn crossing_number_is_odd(poly: &[DVec2], p: DVec2) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        // 上下どちらかの端点を半開区間で扱い、頂点の二重計上を防ぐ。
        if (a.y > p.y) != (b.y > p.y) {
            let t = (p.y - a.y) / (b.y - a.y);
            if p.x < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
    }
    inside
}
