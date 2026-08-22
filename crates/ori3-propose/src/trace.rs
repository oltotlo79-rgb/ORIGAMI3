//! 折り方の計画に使う追跡情報(作業17 / PRO-009の前提)。
//!
//! 生成器は三角形1つを「ウサギ耳分子」で埋める。1つの分子が引く折り線は
//! 軸線(三角形の辺)・稜線(角から内心への二等分線)・ちょうつがい線(内心から
//! 1辺への垂線)の3種類しかなく、**線を引いた瞬間には「この線は誰の何なのか」が
//! 分かっている**。ここまでは今までその情報を捨てており、折り順を決める段になって
//! 座標を突き合わせて推測し直すしかなかった。このモジュールは捨てていた情報を
//! そのまま残す。
//!
//! 残すのは次の4つ(作業17の必須項目)。
//!
//! 1. 折り線が骨格のどの辺・どの先端に由来するか → [`CreaseTrace::leaves`] /
//!    [`CreaseTrace::body_edges`] / [`CreaseRole`]
//! 2. 折り線が完成形でどちら側の紙になるか → [`CreaseTrace::sides`] / [`PaperSide`]
//! 3. 分子が完成形のどの部分を作るか → [`RegionTrace::part`] / [`FinishedPart`]
//! 4. 隣り合う分子が重なるのか並ぶのか → [`MoleculePair::relation`] /
//!    [`MoleculeRelation`]
//!
//! **展開図そのものには一切手を入れない。** 折り線を引く順も本数も座標も変えず、
//! 引いたものを読んで書き留めるだけである。

use ori3_model::{CreasePattern, EdgeKind};
use serde::{Deserialize, Serialize};

use crate::skeleton::Skeleton;

/// 同じ点・同じ線分とみなす距離。展開図側のEPS(1e-9)と同じ桁に合わせる。
const TOUCH_TOL: f64 = 1e-9;

/// 区間が重なっているとみなす最小の長さ(辺の長さを1としたときの割合)。
const SPAN_TOL: f64 = 1e-12;

/// 分子の角が何に由来するか。
///
/// 軸多角形の頂点は「先端(葉)の置き場所」か「紙の隅」のどちらかである。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MoleculeCorner {
    /// 骨格の先端(葉)の置き場所。`circle_index` は `Packing::centers` の並び順。
    Leaf { leaf_id: u32, circle_index: usize },
    /// 紙の隅。骨格に対応する先端はない(左下0・右下1・右上2・左上3)。
    PaperCorner { corner_index: usize },
    /// どちらにもたどれなかった点。組み立て方からは起きないが、
    /// 壊れた入力で欠損を黙って埋めないために残してある。
    Unknown,
}

impl MoleculeCorner {
    /// 骨格の先端ならそのID。紙の隅なら `None`。
    pub fn leaf_id(self) -> Option<u32> {
        match self {
            Self::Leaf { leaf_id, .. } => Some(leaf_id),
            _ => None,
        }
    }
}

/// 折り線の役目。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreaseRole {
    /// 軸線。分子の外周(三角形の辺)で、完成形では軸の上に乗る。
    /// ここで紙が折り返るので、両側の分子は必ず重なる。
    Axial,
    /// 稜線。角から内心へ向かう二等分線で、完成形では先端の背になる。
    Ridge,
    /// ちょうつがい線。内心から1辺へ下ろした垂線で、隣り合う先端を分ける。
    Hinge,
}

/// 完成形での紙の面。
///
/// 折り線をまたぐと紙は必ず裏返る。先頭の領域を [`PaperSide::Front`] とした
/// ときの相対的な向きで、紙の物理的な表裏を指すわけではない。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaperSide {
    Front,
    Back,
}

impl PaperSide {
    /// 折り線を1本またいだ先の面。
    pub fn flipped(self) -> Self {
        match self {
            Self::Front => Self::Back,
            Self::Back => Self::Front,
        }
    }

    fn from_parity(parity: usize) -> Self {
        if parity.is_multiple_of(2) {
            Self::Front
        } else {
            Self::Back
        }
    }
}

/// 完成形のどの部分を作るか。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishedPart {
    /// 骨格の経路。両端の先端と、その間の胴を作る。
    ///
    /// `from_leaf` < `to_leaf` に正規化してあるので、隣り合う分子どうしで
    /// そのまま等しいかを比べられる。
    Path {
        from_leaf: u32,
        to_leaf: u32,
        /// 経路にある骨格の辺。辺は「その節点と親をつなぐ辺」なので節点IDで表す。昇順。
        body_edges: Vec<u32>,
    },
    /// 紙の隅など、骨格の先端に対応しない余りの紙。
    Margin { corners: [MoleculeCorner; 2] },
}

impl FinishedPart {
    /// この部分が占める骨格の辺。余りの紙では空。
    pub fn body_edges(&self) -> &[u32] {
        match self {
            Self::Path { body_edges, .. } => body_edges,
            Self::Margin { .. } => &[],
        }
    }
}

/// 領域の名指し。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RegionRef {
    pub molecule: usize,
    pub region: usize,
}

/// 分子の中の1領域。折りたたむと1枚の面になる部分で、1分子につき4つできる。
///
/// 分子は3本の稜線で3つに分かれ、そのうち1つがちょうつがい線でさらに2つに割れる。
/// 内心のまわりを一周する順に並べてあるので、隣り合う番号の領域は必ず
/// 折り線1本で隣り合い、表裏が入れ替わる。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionTrace {
    /// この分子の中での番号(0から、内心のまわりを一周する順)。
    pub index: usize,
    /// 展開図の上でのこの領域。
    pub polygon: Vec<[f64; 2]>,
    /// 完成形でこの領域が作る部分。
    pub part: FinishedPart,
    /// 完成形でこの領域の紙が向く面。
    pub side: PaperSide,
    /// この領域が接している分子の外周の辺の番号(辺sは角s→角s+1)。
    pub axial_side: usize,
    /// その辺の上で、この領域が占める区間(角sを0、角s+1を1とした割合)。
    pub axial_span: [f64; 2],
}

/// 折り線1本ぶんの追跡情報。展開図へ線を引く呼び出し1回に1件が対応し、
/// 端点 `a`・`b` も引いたときの値そのままである。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreaseTrace {
    /// この折り線を引いた分子の番号。
    pub molecule: usize,
    pub role: CreaseRole,
    /// 折り線の端点(展開図の座標)。手順の折り線指定と同じ形。
    pub a: [f64; 2],
    pub b: [f64; 2],
    /// 展開図へ書き込んだ山谷。
    pub kind: EdgeKind,
    /// 平坦に折り切ったときの目標角(度)。山=+180、谷=-180。
    pub target_angle_deg: f64,
    /// この折り線が由来する骨格の先端(葉ID、昇順)。
    /// 軸線とちょうつがい線は両端の先端、稜線はその角の先端。
    /// 紙の隅から出る線では少なくなる。
    pub leaves: Vec<u32>,
    /// この折り線が由来する骨格の辺(節点IDで表す。昇順)。
    pub body_edges: Vec<u32>,
    /// この折り線に接している領域。軸線は隣の分子の領域も入る。
    pub regions: Vec<RegionRef>,
    /// `regions` と同じ並びの、完成形での紙の面。
    pub sides: Vec<PaperSide>,
}

/// 分子1つぶんの追跡情報。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoleculeTrace {
    /// 分子の番号。作業9の `LeafSite::molecules` と同じ番号(軸多角形の三角形の番号)。
    pub index: usize,
    /// 分子の3つの角の正体。
    pub corners: [MoleculeCorner; 3],
    /// 分子の3つの角の座標(反時計回り)。
    pub polygon: [[f64; 2]; 3],
    /// 稜線が集まる内心。
    pub incenter: [f64; 2],
    /// ちょうつがい線を下ろした辺の番号(辺sは角s→角s+1)。
    pub hinge_side: usize,
    /// ちょうつがい線の行き先。
    pub hinge_foot: [f64; 2],
    /// 分子の中の領域(4つ)。
    pub regions: Vec<RegionTrace>,
    /// この分子が引いた折り線。
    pub creases: Vec<CreaseTrace>,
}

/// 隣り合う分子が完成形でどうなるか。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoleculeRelation {
    /// 完成形で重なる。折り順に「どちらを先に折るか」の制約が出る。
    Stacked,
    /// 完成形では軸に沿って並び、重ならない。折り順の制約が出ない。
    SideBySide,
}

/// 隣り合う分子の組1件ぶん。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoleculePair {
    pub a: usize,
    pub b: usize,
    /// 共有している軸線の両端。角だけを共有する組では `None`。
    pub shared_edge: Option<[MoleculeCorner; 2]>,
    /// 共有している角(昇順)。
    pub shared_corners: Vec<MoleculeCorner>,
    pub relation: MoleculeRelation,
    /// 完成形で共通に作る骨格の辺(昇順)。重なる根拠のひとつ。
    pub shared_body_edges: Vec<u32>,
}

/// 候補1つぶんの追跡情報。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FoldPlanTrace {
    pub molecules: Vec<MoleculeTrace>,
    /// 隣り合う分子の組(`a` < `b`。`a`、`b` の順で昇順)。
    pub neighbors: Vec<MoleculePair>,
    /// 表裏を塗り分けたときに、折り線をまたいでも面が入れ替わらなかった箇所の数。
    /// 0でないなら、その展開図は平坦に折り切れない点を持っている。
    pub side_conflicts: usize,
}

impl FoldPlanTrace {
    /// 分子番号から追跡情報を引く。
    pub fn molecule(&self, index: usize) -> Option<&MoleculeTrace> {
        self.molecules.get(index)
    }

    /// 記録した折り線の総数。
    pub fn crease_count(&self) -> usize {
        self.molecules.iter().map(|m| m.creases.len()).sum()
    }
}

/// 追跡情報と展開図・山谷が食い違っていないかの件数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TraceChecks {
    /// 展開図にある、紙の縁でない折り線の数。
    pub creases: usize,
    /// そのうち、どの追跡情報にも載っていないものの数。
    pub unassigned_creases: usize,
    /// 追跡情報の山谷が、展開図に書かれた山谷と違う折り線の数。
    pub kind_mismatches: usize,
    /// 折り線をまたいでも表裏が入れ替わらなかった箇所の数。
    pub side_conflicts: usize,
    /// 内心のまわりが「山3+谷1」でも「谷3+山1」でもない分子の数(前川定理)。
    pub maekawa_violations: usize,
    /// 同じ軸線を、両側の分子が違う山谷で記録した数。
    pub shared_kind_conflicts: usize,
    /// 同じ軸線の両側で、作る完成部位の記録が食い違った数。
    pub shared_part_conflicts: usize,
    /// 完成部位・目標角・表裏のどれかが欠けている領域・折り線の数。
    pub missing_fields: usize,
    /// 紙の内側にある先端の材料点の数(次の行の母数)。
    pub interior_limbs: usize,
    /// そのうち、まわりの山谷が前川定理(|山−谷|=2)を満たさないものの数。
    /// 紙の縁に乗っている先端は、縁を折らないので数えない。
    pub limb_maekawa_violations: usize,
}

/// 追跡情報が展開図の実物と合っているかを数える(検査用)。
///
/// 生成そのものは重くしないため、`generate()` からは呼ばない。
pub fn check_trace(cp: &CreasePattern, trace: &FoldPlanTrace) -> TraceChecks {
    let mut out = TraceChecks::default();
    let all: Vec<&CreaseTrace> = trace
        .molecules
        .iter()
        .flat_map(|m| m.creases.iter())
        .collect();

    // 1. 展開図の折り線が、どれか1件の追跡情報の線分に乗っているか。
    for edge in &cp.edges {
        if edge.kind == EdgeKind::Border {
            continue;
        }
        let (Some(v0), Some(v1)) = (vertex_pos(cp, edge.v0), vertex_pos(cp, edge.v1)) else {
            continue;
        };
        out.creases += 1;
        let mut matched = false;
        let mut kind_ok = false;
        for c in &all {
            if on_segment(v0, c.a, c.b) && on_segment(v1, c.a, c.b) {
                matched = true;
                kind_ok |= c.kind == edge.kind;
            }
        }
        if !matched {
            out.unassigned_creases += 1;
        } else if !kind_ok {
            out.kind_mismatches += 1;
        }
    }

    // 2. 欠けの点検と、内心まわりの前川定理。
    for m in &trace.molecules {
        for r in &m.regions {
            if r.polygon.len() < 3 {
                out.missing_fields += 1;
            }
        }
        for c in &m.creases {
            let want = match c.kind {
                EdgeKind::Mountain => 180.0,
                EdgeKind::Valley => -180.0,
                _ => 0.0,
            };
            if c.target_angle_deg != want
                || c.regions.len() != c.sides.len()
                || c.regions.is_empty()
            {
                out.missing_fields += 1;
            }
        }
        let around: Vec<&CreaseTrace> = m
            .creases
            .iter()
            .filter(|c| matches!(c.role, CreaseRole::Ridge | CreaseRole::Hinge))
            .collect();
        let mountains = around
            .iter()
            .filter(|c| c.kind == EdgeKind::Mountain)
            .count() as i64;
        let valleys = around.iter().filter(|c| c.kind == EdgeKind::Valley).count() as i64;
        if !around.is_empty() && (mountains - valleys).abs() != 2 {
            out.maekawa_violations += 1;
        }
    }

    // 3. 同じ軸線を共有する分子どうしで、山谷と完成部位がそろっているか。
    for pair in &trace.neighbors {
        let Some(shared) = pair.shared_edge else {
            continue;
        };
        let (Some(a), Some(b)) = (trace.molecule(pair.a), trace.molecule(pair.b)) else {
            continue;
        };
        let kinds_a = axial_kinds(a, shared);
        let kinds_b = axial_kinds(b, shared);
        if !kinds_a.is_empty() && !kinds_b.is_empty() && kinds_a != kinds_b {
            out.shared_kind_conflicts += 1;
        }
        let parts_a = axial_parts(a, shared);
        let parts_b = axial_parts(b, shared);
        if !parts_a.is_empty() && !parts_b.is_empty() && parts_a != parts_b {
            out.shared_part_conflicts += 1;
        }
    }

    // 4. 先端の材料点のまわりの山谷(分子をまたいで数える)。
    let (interior_limbs, limb_maekawa_violations) = limb_maekawa(trace);
    out.interior_limbs = interior_limbs;
    out.limb_maekawa_violations = limb_maekawa_violations;

    out.side_conflicts = trace.side_conflicts;
    out
}

/// 先端の材料点のまわりを、分子をまたいで数える。
///
/// 材料点に集まる折り線は「そこから出る軸線」と「そこから内心へ向かう稜線」の
/// 2種類だけである。軸線は両側の分子が同じものを記録するので、辺の両端の角で
/// 重複を取り除いてから数える。
/// 返すのは (紙の内側にある先端の数, そのうち前川定理を満たさない数)。
fn limb_maekawa(trace: &FoldPlanTrace) -> (usize, usize) {
    use std::collections::BTreeMap;

    struct Around {
        axial: BTreeMap<[MoleculeCorner; 2], EdgeKind>,
        ridges: Vec<EdgeKind>,
        on_border: bool,
    }

    let mut by_leaf: BTreeMap<u32, Around> = BTreeMap::new();
    for m in &trace.molecules {
        for s in 0..3 {
            let Some(leaf) = m.corners[s].leaf_id() else {
                continue;
            };
            let entry = by_leaf.entry(leaf).or_insert_with(|| Around {
                axial: BTreeMap::new(),
                ridges: Vec::new(),
                on_border: false,
            });
            // 角sから出る辺は「辺s」と「辺s-1」の2本。
            for side in [s, (s + 2) % 3] {
                let key = shared_key([m.corners[side], m.corners[(side + 1) % 3]]);
                let drawn = m.creases.iter().find(|c| {
                    c.role == CreaseRole::Axial
                        && corner_pair_of_crease(m, c).map(shared_key) == Some(key)
                });
                match drawn {
                    Some(c) => {
                        entry.axial.insert(key, c.kind);
                    }
                    // 引かれていない辺は紙の縁。縁は折らないので、この先端は
                    // 紙の内側にいない。
                    None => entry.on_border = true,
                }
            }
            if let Some(c) = m
                .creases
                .iter()
                .find(|c| c.role == CreaseRole::Ridge && same_point(c.a, m.polygon[s]))
            {
                entry.ridges.push(c.kind);
            }
        }
    }

    let mut interior = 0usize;
    let mut violations = 0usize;
    for around in by_leaf.values() {
        if around.on_border {
            continue;
        }
        interior += 1;
        let kinds = around
            .axial
            .values()
            .copied()
            .chain(around.ridges.iter().copied());
        let mut mountains = 0i64;
        let mut valleys = 0i64;
        for kind in kinds {
            match kind {
                EdgeKind::Mountain => mountains += 1,
                EdgeKind::Valley => valleys += 1,
                _ => {}
            }
        }
        if (mountains - valleys).abs() != 2 {
            violations += 1;
        }
    }
    (interior, violations)
}

fn vertex_pos(cp: &CreasePattern, id: ori3_model::VertexId) -> Option<[f64; 2]> {
    cp.vertices.iter().find(|v| v.id == id).map(|v| v.pos)
}

/// 点が線分の上に乗っているか(端点を含む)。
fn on_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len = ab[0].hypot(ab[1]);
    if len <= TOUCH_TOL {
        return (p[0] - a[0]).hypot(p[1] - a[1]) <= TOUCH_TOL;
    }
    let cross = ((p[0] - a[0]) * ab[1] - (p[1] - a[1]) * ab[0]).abs() / len;
    if cross > TOUCH_TOL {
        return false;
    }
    let t = ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / (len * len);
    (-TOUCH_TOL..=1.0 + TOUCH_TOL).contains(&t)
}

fn shared_key(shared: [MoleculeCorner; 2]) -> [MoleculeCorner; 2] {
    let mut k = shared;
    k.sort_unstable();
    k
}

fn axial_kinds(m: &MoleculeTrace, shared: [MoleculeCorner; 2]) -> Vec<EdgeKind> {
    let want = shared_key(shared);
    m.creases
        .iter()
        .filter(|c| c.role == CreaseRole::Axial)
        .filter(|c| corner_pair_of_crease(m, c).map(shared_key) == Some(want))
        .map(|c| c.kind)
        .collect()
}

fn axial_parts(m: &MoleculeTrace, shared: [MoleculeCorner; 2]) -> Vec<FinishedPart> {
    let want = shared_key(shared);
    let mut out: Vec<FinishedPart> = m
        .regions
        .iter()
        .filter(|r| {
            let (i, j) = (r.axial_side, (r.axial_side + 1) % 3);
            shared_key([m.corners[i], m.corners[j]]) == want
        })
        .map(|r| r.part.clone())
        .collect();
    out.dedup();
    out
}

/// 軸線の追跡情報から、その辺の両端の角を引く。
fn corner_pair_of_crease(m: &MoleculeTrace, c: &CreaseTrace) -> Option<[MoleculeCorner; 2]> {
    (0..3).find_map(|s| {
        let (p, q) = (m.polygon[s], m.polygon[(s + 1) % 3]);
        (same_point(p, c.a) && same_point(q, c.b) || same_point(p, c.b) && same_point(q, c.a))
            .then_some([m.corners[s], m.corners[(s + 1) % 3]])
    })
}

fn same_point(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) <= TOUCH_TOL
}

/// 折り線を引くときに決まる、分子1つぶんの形。
///
/// `generate::rabbit_ear` が線を引くのと同じ場所で作る。あとから幾何を当てはめて
/// 求め直すのではなく、決めた値をそのまま持ち回るための型。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MoleculeShape {
    pub incenter: [f64; 2],
    /// ちょうつがい線を下ろした辺の番号(辺sは角s→角s+1)。
    pub hinge_side: usize,
    pub foot: [f64; 2],
    /// 辺sを軸線として引いたか(紙の縁と重なる辺は引かない)。
    pub axial_drawn: [bool; 3],
}

/// 追跡情報を組み立てる(作業17)。
///
/// 引数はすべて展開図を作った側が既に持っている値で、ここで幾何の当てはめ直しは
/// しない。`shapes[i]` が `None` なのは潰れた三角形で折り線を引けなかったときで、
/// その分子は領域も折り線も0件になる。
pub(crate) fn build(
    skeleton: &Skeleton,
    pts: &[[f64; 2]],
    corner_of: &[MoleculeCorner],
    tris: &[[usize; 3]],
    shapes: &[Option<MoleculeShape>],
) -> FoldPlanTrace {
    let mut molecules: Vec<MoleculeTrace> = Vec::with_capacity(tris.len());
    for (index, tri) in tris.iter().enumerate() {
        let corners = [
            corner_at(corner_of, tri[0]),
            corner_at(corner_of, tri[1]),
            corner_at(corner_of, tri[2]),
        ];
        let polygon = [pts[tri[0]], pts[tri[1]], pts[tri[2]]];
        let Some(shape) = shapes.get(index).copied().flatten() else {
            molecules.push(MoleculeTrace {
                index,
                corners,
                polygon,
                incenter: polygon[0],
                hinge_side: 0,
                hinge_foot: polygon[0],
                regions: Vec::new(),
                creases: Vec::new(),
            });
            continue;
        };
        let regions = build_regions(skeleton, &corners, &polygon, &shape);
        let creases = build_creases(skeleton, index, &corners, &polygon, &shape);
        molecules.push(MoleculeTrace {
            index,
            corners,
            polygon,
            incenter: shape.incenter,
            hinge_side: shape.hinge_side,
            hinge_foot: shape.foot,
            regions,
            creases,
        });
    }

    let side_conflicts = assign_sides(tris, &mut molecules);
    attach_regions(tris, &mut molecules);
    let neighbors = build_neighbors(tris, &molecules);

    FoldPlanTrace {
        molecules,
        neighbors,
        side_conflicts,
    }
}

fn corner_at(corner_of: &[MoleculeCorner], point: usize) -> MoleculeCorner {
    corner_of
        .get(point)
        .copied()
        .unwrap_or(MoleculeCorner::Unknown)
}

/// 辺sの両端が作る完成部位。両端が先端なら骨格の経路、片方でも紙の隅なら余りの紙。
fn part_of_side(skeleton: &Skeleton, corners: &[MoleculeCorner; 3], s: usize) -> FinishedPart {
    let (a, b) = (corners[s], corners[(s + 1) % 3]);
    match (a.leaf_id(), b.leaf_id()) {
        (Some(x), Some(y)) if x != y => FinishedPart::Path {
            from_leaf: x.min(y),
            to_leaf: x.max(y),
            body_edges: skeleton.path_edges(x, y),
        },
        _ => {
            let mut pair = [a, b];
            pair.sort_unstable();
            FinishedPart::Margin { corners: pair }
        }
    }
}

/// 内心のまわりを一周する順に領域を作る。
///
/// 辺sごとに三角形[角s, 角s+1, 内心]を作り、ちょうつがい線が下りている辺だけは
/// その足で2つに割る。合計4つになり、隣り合う番号どうしは折り線1本で接する。
fn build_regions(
    skeleton: &Skeleton,
    corners: &[MoleculeCorner; 3],
    polygon: &[[f64; 2]; 3],
    shape: &MoleculeShape,
) -> Vec<RegionTrace> {
    let mut out: Vec<RegionTrace> = Vec::with_capacity(4);
    for s in 0..3 {
        let (p, q) = (polygon[s], polygon[(s + 1) % 3]);
        let part = part_of_side(skeleton, corners, s);
        if s == shape.hinge_side {
            let t = param_on(p, q, shape.foot);
            out.push(RegionTrace {
                index: out.len(),
                polygon: vec![p, shape.foot, shape.incenter],
                part: part.clone(),
                side: PaperSide::Front,
                axial_side: s,
                axial_span: [0.0, t],
            });
            out.push(RegionTrace {
                index: out.len(),
                polygon: vec![shape.foot, q, shape.incenter],
                part,
                side: PaperSide::Front,
                axial_side: s,
                axial_span: [t, 1.0],
            });
        } else {
            out.push(RegionTrace {
                index: out.len(),
                polygon: vec![p, q, shape.incenter],
                part,
                side: PaperSide::Front,
                axial_side: s,
                axial_span: [0.0, 1.0],
            });
        }
    }
    out
}

/// 引いた折り線をそのまま書き留める。順番も端点も `rabbit_ear` の呼び出しと同じ。
fn build_creases(
    skeleton: &Skeleton,
    molecule: usize,
    corners: &[MoleculeCorner; 3],
    polygon: &[[f64; 2]; 3],
    shape: &MoleculeShape,
) -> Vec<CreaseTrace> {
    let mut out = Vec::with_capacity(7);
    // 軸線(谷)。紙の縁と重なる辺は引いていないので記録もしない。
    for s in 0..3 {
        if !shape.axial_drawn[s] {
            continue;
        }
        let part = part_of_side(skeleton, corners, s);
        out.push(CreaseTrace {
            molecule,
            role: CreaseRole::Axial,
            a: polygon[s],
            b: polygon[(s + 1) % 3],
            kind: EdgeKind::Valley,
            target_angle_deg: -180.0,
            leaves: leaves_of(&[corners[s], corners[(s + 1) % 3]]),
            body_edges: part.body_edges().to_vec(),
            regions: Vec::new(),
            sides: Vec::new(),
        });
    }
    // 稜線(山): 各角から内心へ。
    for s in 0..3 {
        out.push(CreaseTrace {
            molecule,
            role: CreaseRole::Ridge,
            a: polygon[s],
            b: shape.incenter,
            kind: EdgeKind::Mountain,
            target_angle_deg: 180.0,
            leaves: leaves_of(&[corners[s]]),
            body_edges: corners[s].leaf_id().into_iter().collect(),
            regions: Vec::new(),
            sides: Vec::new(),
        });
    }
    // ちょうつがい線(谷): 内心から辺へ。隣り合う先端を分ける。
    let hs = shape.hinge_side;
    let hinge_part = part_of_side(skeleton, corners, hs);
    out.push(CreaseTrace {
        molecule,
        role: CreaseRole::Hinge,
        a: shape.incenter,
        b: shape.foot,
        kind: EdgeKind::Valley,
        target_angle_deg: -180.0,
        leaves: leaves_of(&[corners[hs], corners[(hs + 1) % 3]]),
        body_edges: hinge_part.body_edges().to_vec(),
        regions: Vec::new(),
        sides: Vec::new(),
    });
    out
}

fn leaves_of(corners: &[MoleculeCorner]) -> Vec<u32> {
    let mut out: Vec<u32> = corners.iter().filter_map(|c| c.leaf_id()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn param_on(p: [f64; 2], q: [f64; 2], x: [f64; 2]) -> f64 {
    let d = [q[0] - p[0], q[1] - p[1]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    if len2 <= 0.0 || !len2.is_finite() {
        return 0.0;
    }
    (((x[0] - p[0]) * d[0] + (x[1] - p[1]) * d[1]) / len2).clamp(0.0, 1.0)
}

/// 領域どうしが折り線1本で隣り合っている組を全て作る。
///
/// - 同じ分子の中: 内心のまわりを一周する順で隣どうし(稜線・ちょうつがい線)。
/// - 分子をまたぐ: 同じ軸線を共有し、その辺の上で区間が重なっている領域どうし。
fn region_links(tris: &[[usize; 3]], molecules: &[MoleculeTrace]) -> Vec<(usize, usize)> {
    let offset = region_offsets(molecules);
    let mut links = Vec::new();
    for (m, mol) in molecules.iter().enumerate() {
        let n = mol.regions.len();
        for r in 0..n {
            links.push((offset[m] + r, offset[m] + (r + 1) % n));
        }
    }
    // 点の組 → その辺を持つ(分子, 辺番号)
    let mut by_edge: std::collections::BTreeMap<(usize, usize), Vec<(usize, usize)>> =
        std::collections::BTreeMap::new();
    for (m, tri) in tris.iter().enumerate() {
        if molecules.get(m).is_none_or(|x| x.regions.is_empty()) {
            continue;
        }
        for s in 0..3 {
            let (i, j) = (tri[s], tri[(s + 1) % 3]);
            by_edge
                .entry((i.min(j), i.max(j)))
                .or_default()
                .push((m, s));
        }
    }
    for ((lo, hi), owners) in by_edge {
        for x in 0..owners.len() {
            for y in (x + 1)..owners.len() {
                let (ma, sa) = owners[x];
                let (mb, sb) = owners[y];
                for ra in spans_on(&molecules[ma], sa, tris[ma], lo, hi) {
                    for rb in spans_on(&molecules[mb], sb, tris[mb], lo, hi) {
                        let lo_t = ra.1[0].max(rb.1[0]);
                        let hi_t = ra.1[1].min(rb.1[1]);
                        if hi_t - lo_t > SPAN_TOL {
                            links.push((offset[ma] + ra.0, offset[mb] + rb.0));
                        }
                    }
                }
            }
        }
    }
    for link in links.iter_mut() {
        if link.0 > link.1 {
            *link = (link.1, link.0);
        }
    }
    links.sort_unstable();
    links.dedup();
    links
}

fn region_offsets(molecules: &[MoleculeTrace]) -> Vec<usize> {
    let mut offset = Vec::with_capacity(molecules.len() + 1);
    let mut acc = 0usize;
    for m in molecules {
        offset.push(acc);
        acc += m.regions.len();
    }
    offset.push(acc);
    offset
}

/// 辺 `(lo,hi)` の向き(点loから点hiへ)にそろえた、辺sの各領域の区間。
fn spans_on(
    mol: &MoleculeTrace,
    s: usize,
    tri: [usize; 3],
    lo: usize,
    hi: usize,
) -> Vec<(usize, [f64; 2])> {
    let forward = tri[s] == lo && tri[(s + 1) % 3] == hi;
    mol.regions
        .iter()
        .filter(|r| r.axial_side == s)
        .map(|r| {
            let span = if forward {
                r.axial_span
            } else {
                [1.0 - r.axial_span[1], 1.0 - r.axial_span[0]]
            };
            (r.index, span)
        })
        .collect()
}

/// 表裏を塗り分ける。折り線をまたぐと必ず裏返る、という規則だけを使う。
/// 塗り分けが合わない箇所の数を返す。
fn assign_sides(tris: &[[usize; 3]], molecules: &mut [MoleculeTrace]) -> usize {
    let offset = region_offsets(molecules);
    let total = *offset.last().unwrap_or(&0);
    let links = region_links(tris, molecules);
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); total];
    for &(a, b) in &links {
        adj[a].push(b);
        adj[b].push(a);
    }
    let mut parity: Vec<Option<usize>> = vec![None; total];
    let mut conflicts = 0usize;
    for start in 0..total {
        if parity[start].is_some() {
            continue;
        }
        parity[start] = Some(0);
        let mut queue = std::collections::VecDeque::from([start]);
        while let Some(cur) = queue.pop_front() {
            let here = parity[cur].unwrap_or(0);
            for &next in &adj[cur] {
                match parity[next] {
                    None => {
                        parity[next] = Some((here + 1) % 2);
                        queue.push_back(next);
                    }
                    Some(p) if p == here => conflicts += 1,
                    Some(_) => {}
                }
            }
        }
    }
    // 同じ辺を2回数えるので半分にする(a→bとb→aの両方で見つかる)。
    conflicts /= 2;
    for (m, mol) in molecules.iter_mut().enumerate() {
        for r in mol.regions.iter_mut() {
            r.side = PaperSide::from_parity(parity[offset[m] + r.index].unwrap_or(0));
        }
    }
    conflicts
}

/// 折り線に、接している領域と表裏を書き込む。
fn attach_regions(tris: &[[usize; 3]], molecules: &mut [MoleculeTrace]) {
    // 軸線をまたいだ先の分子を先に引けるようにしておく。
    let mut by_edge: std::collections::BTreeMap<(usize, usize), Vec<(usize, usize)>> =
        std::collections::BTreeMap::new();
    for (m, tri) in tris.iter().enumerate() {
        if molecules.get(m).is_none_or(|x| x.regions.is_empty()) {
            continue;
        }
        for s in 0..3 {
            let (i, j) = (tri[s], tri[(s + 1) % 3]);
            by_edge
                .entry((i.min(j), i.max(j)))
                .or_default()
                .push((m, s));
        }
    }
    let sides: Vec<Vec<PaperSide>> = molecules
        .iter()
        .map(|m| m.regions.iter().map(|r| r.side).collect())
        .collect();
    let axial_sides: Vec<Vec<usize>> = molecules
        .iter()
        .map(|m| m.regions.iter().map(|r| r.axial_side).collect())
        .collect();
    let spans: Vec<Vec<[f64; 2]>> = molecules
        .iter()
        .map(|m| m.regions.iter().map(|r| r.axial_span).collect())
        .collect();

    for (m, tri) in tris.iter().enumerate() {
        let mut updates: Vec<(usize, Vec<RegionRef>)> = Vec::new();
        for (ci, crease) in molecules[m].creases.iter().enumerate() {
            let mut refs: Vec<RegionRef> = Vec::new();
            match crease.role {
                CreaseRole::Axial => {
                    let s = side_index_of(molecules[m].polygon, crease.a, crease.b).unwrap_or(0);
                    for (r, &a) in axial_sides[m].iter().enumerate() {
                        if a == s {
                            refs.push(RegionRef {
                                molecule: m,
                                region: r,
                            });
                        }
                    }
                    let (i, j) = (tri[s], tri[(s + 1) % 3]);
                    if let Some(owners) = by_edge.get(&(i.min(j), i.max(j))) {
                        for &(mo, so) in owners {
                            if mo == m {
                                continue;
                            }
                            for (r, &a) in axial_sides[mo].iter().enumerate() {
                                if a == so {
                                    refs.push(RegionRef {
                                        molecule: mo,
                                        region: r,
                                    });
                                }
                            }
                        }
                    }
                }
                CreaseRole::Ridge => {
                    // 角sから内心へ。角sを含む領域は、辺sの手前側と辺s-1の奥側。
                    let s = corner_index_of(molecules[m].polygon, crease.a).unwrap_or(0);
                    let prev = (s + 2) % 3;
                    for (r, &a) in axial_sides[m].iter().enumerate() {
                        let span = spans[m][r];
                        if (a == s && span[0] <= SPAN_TOL)
                            || (a == prev && span[1] >= 1.0 - SPAN_TOL)
                        {
                            refs.push(RegionRef {
                                molecule: m,
                                region: r,
                            });
                        }
                    }
                }
                CreaseRole::Hinge => {
                    let hs = molecules[m].hinge_side;
                    for (r, &a) in axial_sides[m].iter().enumerate() {
                        if a == hs {
                            refs.push(RegionRef {
                                molecule: m,
                                region: r,
                            });
                        }
                    }
                }
            }
            refs.sort_unstable();
            refs.dedup();
            updates.push((ci, refs));
        }
        for (ci, refs) in updates {
            let paper: Vec<PaperSide> = refs
                .iter()
                .map(|r| {
                    sides
                        .get(r.molecule)
                        .and_then(|s| s.get(r.region))
                        .copied()
                        .unwrap_or(PaperSide::Front)
                })
                .collect();
            let crease = &mut molecules[m].creases[ci];
            crease.regions = refs;
            crease.sides = paper;
        }
    }
}

fn side_index_of(polygon: [[f64; 2]; 3], a: [f64; 2], b: [f64; 2]) -> Option<usize> {
    (0..3).find(|&s| {
        let (p, q) = (polygon[s], polygon[(s + 1) % 3]);
        (same_point(p, a) && same_point(q, b)) || (same_point(p, b) && same_point(q, a))
    })
}

fn corner_index_of(polygon: [[f64; 2]; 3], p: [f64; 2]) -> Option<usize> {
    (0..3).find(|&s| same_point(polygon[s], p))
}

/// 隣り合う分子の組を作る。角を1つでも共有していれば「隣り合う」とみなす。
fn build_neighbors(tris: &[[usize; 3]], molecules: &[MoleculeTrace]) -> Vec<MoleculePair> {
    let mut out = Vec::new();
    for a in 0..tris.len() {
        for b in (a + 1)..tris.len() {
            let mut shared_points: Vec<usize> = tris[a]
                .iter()
                .copied()
                .filter(|p| tris[b].contains(p))
                .collect();
            shared_points.sort_unstable();
            shared_points.dedup();
            if shared_points.is_empty() {
                continue;
            }
            let mut shared_corners: Vec<MoleculeCorner> = shared_points
                .iter()
                .map(|&p| {
                    let s = tris[a].iter().position(|&x| x == p).unwrap_or(0);
                    molecules[a].corners[s]
                })
                .collect();
            shared_corners.sort_unstable();
            let shared_edge =
                (shared_corners.len() == 2).then(|| [shared_corners[0], shared_corners[1]]);

            let edges_a = body_edges_of(&molecules[a]);
            let edges_b = body_edges_of(&molecules[b]);
            let shared_body_edges: Vec<u32> = edges_a
                .iter()
                .copied()
                .filter(|e| edges_b.contains(e))
                .collect();
            let relation = if shared_edge.is_some() || !shared_body_edges.is_empty() {
                MoleculeRelation::Stacked
            } else {
                MoleculeRelation::SideBySide
            };
            out.push(MoleculePair {
                a,
                b,
                shared_edge,
                shared_corners,
                relation,
                shared_body_edges,
            });
        }
    }
    out
}

fn body_edges_of(m: &MoleculeTrace) -> Vec<u32> {
    let mut out: Vec<u32> = m
        .regions
        .iter()
        .flat_map(|r| r.part.body_edges().iter().copied())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}
