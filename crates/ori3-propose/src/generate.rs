//! 充填結果から展開図を組み立てる(PRO-002/PRO-003、要件§8-3・§8-4)。
//!
//! 手順は要件§8-3の通り。
//! 1. 円中心と紙の四隅を頂点にしたドロネー三角形分割で軸多角形を作る。
//!    紙の四隅を必ず含めるので、三角形の集まりが紙全体をちょうど覆う。
//! 2. 各多角形を簡易分子で埋める。三角形はウサギ耳分子、四角形以上は先頭の
//!    頂点から扇状に三角形へ割ってから同じ処理をする。
//! 3. 山谷は「軸線(三角形の辺)=谷、稜線(内心へ向かう二等分線)=山」の既定則。
//!
//! ウサギ耳分子は「3本の角の二等分線が内心で交わり、そこから1辺へ垂線
//! (耳のちょうつがい線)を下ろす」形にした。内心のまわりが山3・谷1になり、
//! 前川定理(山−谷=±2)と川崎定理(1つおきの角の和=180°)を同時に満たす。
//! 3辺すべてへ垂線を下ろす形も試したが、軸線の途中に3叉の点が大量にでき、
//! 川崎定理を満たしようがない頂点が増えて平坦折り違反が3〜5倍になったため
//! 採らなかった(同じ理由で、垂線の行き先は紙の縁に乗る辺を優先している)。
//!
//! 4. 局所平坦折り判定に掛け、違反頂点数を結果に載せる(失敗扱いにはしない)。
//!
//! 厳密な最適性や完全な平坦折り可能性は保証しない(要件§8の精度目標)。
//! 無理のある箇所は日本語の警告として伝え、生成そのものは続行する。

use ori3_cp::{insert_segment, local_violations, validate};
use ori3_model::{CreasePattern, Edge, EdgeKind, Vertex};
use serde::{Deserialize, Serialize};

use crate::packing::{LeafCircle, Packing};
use crate::skeleton::Skeleton;
use crate::trace::{FoldPlanTrace, MoleculeCorner, MoleculeShape};
use crate::triangulate::{MERGE_TOL, dedup, index_of, triangles_at, triangulate};

/// 紙の縁に乗っているとみなす許容誤差。
const ON_EDGE_TOL: f64 = 1e-9;

/// 提案画面の入力行と同じ、出っぱりの表示名。
const LIMB_NAMES: [&str; 8] = [
    "頭",
    "尾",
    "右前足",
    "左前足",
    "右後足",
    "左後足",
    "右の羽",
    "左の羽",
];

const PAPER_SIZE_ERROR: &str = "紙の幅と高さを0より大きくして、もう一度お試しください";
const NO_PLACEMENT_ERROR: &str = "出っぱりを紙の上に配置できませんでした。出っぱりの数を減らすか、長さや太さを小さくして、もう一度お試しください";
const STRAIGHT_LINE_WARNING: &str = "出っぱりの置き方が一直線に並び、折り線を作れませんでした。「選び直す」で戻り、さらに「形を直す」を選んで、出っぱりの長さか太さを変えてください";
const INVALID_CREASES_WARNING: &str =
    "折り線に重なりやつながり方の問題があります。「選び直す」で別の候補を選んでください";

/// 内部IDではなく、提案画面で見分けられる入力行の名前を返す。
pub(crate) fn limb_name(skeleton: &Skeleton, id: u32) -> String {
    let Some(index) = skeleton.leaves().iter().position(|&leaf_id| leaf_id == id) else {
        return "該当する出っぱり".to_string();
    };
    LIMB_NAMES.get(index).map_or_else(
        || format!("出っぱり{}", index + 1),
        |name| (*name).to_string(),
    )
}

/// 円が紙からはみ出すこと自体は正常な配置なので警告しない(要件§8-2は
/// 「円の中心が紙内」を制約としており、円そのものは紙からはみ出してよい)。
/// `scratchpad/containment-report.md` の実測: 実際に折った鶴・カエルの基本形
/// 9本すべてで、出っぱりの長さは要求どおり届いており(不足0、最大差
/// 6.8e-15)、はみ出し量を0%→75%に変えても不足は変わらなかった。
/// 一方、円の中心そのものが紙の外に出ると、案A(中心が紙内)の制約が
/// 破れており、軸多角形の頂点が紙の外へ出て展開図が壊れうる。
/// `pack()` は中心を常に紙内へ収めるため通常は起きないが、壊れた入力に
/// 対する安全網として警告する。
fn position_outside_paper_warning(name: &str) -> String {
    format!(
        "{name}の置き場所が紙からはみ出しています。「選び直す」で戻り、さらに「形を直す」を選んで、出っぱりの数や配置を見直してください"
    )
}

fn overlapping_limb_warning(name: &str) -> String {
    format!(
        "{name}が別の出っぱりと同じ位置にあります。「選び直す」で別の候補を選ぶか、さらに「形を直す」で戻って{name}の長さか太さを変えてください"
    )
}

fn foldability_warning(violations: usize) -> String {
    format!(
        "平らに折りたたみにくい点が{violations}か所あります。「選び直す」で別の候補を選ぶか、この展開図を使った後に折り線の交点を動かしてください"
    )
}

/// 先端の材料になる、展開図の上の点(作業9)。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeafVertex {
    /// 展開図の頂点ID。
    pub id: u32,
    /// その頂点の座標。
    pub pos: [f64; 2],
    /// 円の中心とのずれ。同じ点とみなせるのは [`MERGE_TOL`](crate::triangulate::MERGE_TOL)
    /// (1e-7)未満のときで、それより離れた点はここに入らない。
    pub gap: f64,
}

/// 先端(葉)1本ぶんの、置き場所から展開図までの対応(作業9 / PRO-007)。
///
/// 後から幾何の当てはめで推測しなくて済むよう、生成したその場で残す。
/// 先端ID → 円 → 展開図の材料点 → その先端を囲む分子、が1本につながる。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeafSite {
    /// 配置で決まった、この先端の円(先端IDと円の番号もここに入っている)。
    pub circle: LeafCircle,
    /// 展開図でこの先端の材料になる点。折り線を1本も引けなかったときだけ `None`。
    pub vertex: Option<LeafVertex>,
    /// この先端のまわりを埋めた分子の番号(軸多角形の三角形。昇順)。
    pub molecules: Vec<usize>,
}

/// 展開図の自動提案の結果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProposalResult {
    pub cp: CreasePattern,
    /// 局所平坦折り判定(CPE-009)で引っかかった頂点の数。0でなくても失敗ではない。
    pub violations: usize,
    /// 利用者へ見せる日本語の注意書き。
    pub warnings: Vec<String>,
    /// 先端と展開図の対応(作業9)。骨格の葉1本につきちょうど1件、
    /// `packing.centers` と同じ並びで入る。
    #[serde(default)]
    pub sites: Vec<LeafSite>,
    /// 折り方の計画に使う追跡情報(作業17)。分子1つにつき1件、軸多角形の
    /// 三角形と同じ番号で入る。折り順そのものはここには入っていない。
    #[serde(default)]
    pub trace: FoldPlanTrace,
}

/// 円の中心にいちばん近い展開図の頂点を、その先端の材料点として返す。
///
/// [`MERGE_TOL`](crate::triangulate::MERGE_TOL)(1e-7)より離れた点しか無ければ
/// `None`。`insert_segment` は線分の端点を1e-9以内の既存頂点へ吸着させるので、
/// 円の中心が軸多角形の頂点として使われていれば、ずれは1e-9以内に収まる。
/// 走査は頂点の並び順で、同じずれなら先に現れた方を採るため結果は決定的。
fn material_vertex(cp: &CreasePattern, center: [f64; 2]) -> Option<LeafVertex> {
    let mut best: Option<LeafVertex> = None;
    for v in &cp.vertices {
        let gap = (v.pos[0] - center[0]).hypot(v.pos[1] - center[1]);
        if gap < MERGE_TOL && best.is_none_or(|b| gap < b.gap) {
            best = Some(LeafVertex {
                id: v.id,
                pos: v.pos,
                gap,
            });
        }
    }
    best
}

/// 輪郭4辺だけが入ったCPを作る(左下(0,0)起点の反時計回り)。
fn border_cp(w: f64, h: f64) -> CreasePattern {
    let corners = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
    CreasePattern {
        vertices: corners
            .iter()
            .enumerate()
            .map(|(i, p)| Vertex {
                id: i as u32,
                pos: *p,
            })
            .collect(),
        edges: (0..4)
            .map(|i| Edge {
                id: i,
                v0: i,
                v1: (i + 1) % 4,
                kind: EdgeKind::Border,
            })
            .collect(),
        next_vertex_id: 4,
        next_edge_id: 4,
    }
}

/// 線分が紙の縁と重なっているか(重なる線分は既存のBorder辺なので引き直さない)。
pub(crate) fn on_paper_edge(p: [f64; 2], q: [f64; 2], w: f64, h: f64) -> bool {
    let same = |a: f64, b: f64, v: f64| (a - v).abs() < ON_EDGE_TOL && (b - v).abs() < ON_EDGE_TOL;
    same(p[0], q[0], 0.0) || same(p[0], q[0], w) || same(p[1], q[1], 0.0) || same(p[1], q[1], h)
}

/// 点pから線分abへ下ろした垂線の足(線分の外へは出さない)。
fn foot(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    if len2 <= 0.0 {
        return a;
    }
    let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0);
    [a[0] + ab[0] * t, a[1] + ab[1] * t]
}

/// 三角形をウサギ耳分子で埋める。`tri[i]` の対辺の長さを重みにすると内心が出る。
///
/// 引いた折り線の形(内心・ちょうつがい線の行き先・軸線を引いた辺)を返す。
/// 返した値は [`crate::trace`] がそのまま追跡情報にする。折り線を引けなかった
/// (潰れた三角形・非有限な座標)ときは `None`。
fn rabbit_ear(cp: &mut CreasePattern, tri: [[f64; 2]; 3], w: f64, h: f64) -> Option<MoleculeShape> {
    let opp = |i: usize| {
        let (a, b) = (tri[(i + 1) % 3], tri[(i + 2) % 3]);
        libm::hypot(a[0] - b[0], a[1] - b[1])
    };
    let len = [opp(0), opp(1), opp(2)];
    let sum = len[0] + len[1] + len[2];
    if sum <= 0.0 || !sum.is_finite() {
        return None; // 潰れた三角形・非有限な座標
    }
    let incenter = [
        (len[0] * tri[0][0] + len[1] * tri[1][0] + len[2] * tri[2][0]) / sum,
        (len[0] * tri[0][1] + len[1] * tri[1][1] + len[2] * tri[2][1]) / sum,
    ];
    // 軸線(谷)。紙の縁と重なるものは既にBorder辺があるので引かない。
    let mut axial_drawn = [false; 3];
    for i in 0..3 {
        let (p, q) = (tri[i], tri[(i + 1) % 3]);
        if !on_paper_edge(p, q, w, h) {
            insert_segment(cp, p, q, EdgeKind::Valley);
            axial_drawn[i] = true;
        }
    }
    // 稜線(山): 各頂点から内心へ。二等分線に一致する。
    for p in tri {
        insert_segment(cp, p, incenter, EdgeKind::Mountain);
    }
    // ちょうつがい線(谷): 内心から1辺へ下ろす垂線。ウサギ耳分子の「耳」にあたる。
    // 内心のまわりは 山3 + 谷1 になり、前川定理(山−谷=2)と川崎定理を同時に満たす。
    // 下ろす先は「紙の縁に乗っている辺」を優先し、次に長い辺を選ぶ。紙の縁で
    // 終わるちょうつがい線は折りの妨げにならないが、内側の軸線の途中で終わると
    // その点が3叉になり平坦に折れなくなるため。
    let key = |i: usize| {
        let on_edge = on_paper_edge(tri[(i + 1) % 3], tri[(i + 2) % 3], w, h);
        (u8::from(on_edge), len[i])
    };
    let pick = (0..3)
        .max_by(|&a, &b| {
            let (x, y) = (key(a), key(b));
            x.0.cmp(&y.0).then(x.1.total_cmp(&y.1))
        })
        .unwrap_or(0);
    let f = foot(incenter, tri[(pick + 1) % 3], tri[(pick + 2) % 3]);
    insert_segment(cp, incenter, f, EdgeKind::Valley);
    Some(MoleculeShape {
        incenter,
        // 辺sは tri[s] → tri[s+1]。垂線の行き先は tri[pick+1] → tri[pick+2] の辺。
        hinge_side: (pick + 1) % 3,
        foot: f,
        axial_drawn,
    })
}

/// 多角形を簡易分子で埋める。四角形以上は先頭の頂点から扇状に三角形へ割る。
/// 割った三角形1つにつき1件、分子の形を返す。
fn fill_polygon(
    cp: &mut CreasePattern,
    poly: &[[f64; 2]],
    w: f64,
    h: f64,
) -> Vec<Option<MoleculeShape>> {
    (1..poly.len().saturating_sub(1))
        .map(|i| rabbit_ear(cp, [poly[0], poly[i], poly[i + 1]], w, h))
        .collect()
}

/// 充填結果から展開図を組み立てる(要件§8-3・§8-4)。
///
/// 骨格や紙寸法が壊れているときだけ `Err`(日本語1文)を返す。幾何的に無理の
/// ある配置は失敗にせず、`warnings` に理由を入れて展開図を返す。
/// 同じ入力からは必ず同じCPが得られる(決定的)。
pub fn generate(
    skeleton: &Skeleton,
    packing: &Packing,
    paper_w: f64,
    paper_h: f64,
) -> Result<ProposalResult, String> {
    // 完成形の位置が枠の外でも止めない。いちばん近い置き方にして知らせる
    // (`CLAUDE.md` §8、作業10)。骨格の形そのものが壊れているときだけ `Err`。
    skeleton.validate_structure()?;
    if !(paper_w > 0.0 && paper_h > 0.0 && paper_w.is_finite() && paper_h.is_finite()) {
        return Err(PAPER_SIZE_ERROR.to_string());
    }
    if packing.centers.is_empty() {
        return Err(NO_PLACEMENT_ERROR.to_string());
    }
    // 指定した位置に無理があった場合の知らせ(作業10)。位置を指定していなければ
    // 1件も増えないので、今までの結果は変わらない。
    let mut warnings: Vec<String> = crate::packing::tip_targets(skeleton, paper_w, paper_h)
        .map(|t| t.notices)
        .unwrap_or_default();

    // 円そのものが紙からはみ出すのは正常(上のdocコメント参照)。本当に問題
    // なのは、案Aの制約である「円の中心が紙内」が破れている場合だけなので、
    // 中心の位置だけを見て知らせる。
    for &(id, c) in &packing.centers {
        let out = c[0] < -ON_EDGE_TOL
            || c[0] > paper_w + ON_EDGE_TOL
            || c[1] < -ON_EDGE_TOL
            || c[1] > paper_h + ON_EDGE_TOL;
        if out {
            let name = limb_name(skeleton, id);
            warnings.push(position_outside_paper_warning(&name));
        }
    }

    // 軸多角形の頂点候補: 円中心を先に、紙の四隅を後に置いて重複をまとめる。
    let paper_corners = [
        [0.0, 0.0],
        [paper_w, 0.0],
        [paper_w, paper_h],
        [0.0, paper_h],
    ];
    let mut pts: Vec<[f64; 2]> = packing.centers.iter().map(|&(_, c)| c).collect();
    pts.extend(paper_corners);
    let pts = dedup(&pts);
    let tris = triangulate(&pts);
    if tris.is_empty() {
        warnings.push(STRAIGHT_LINE_WARNING.to_string());
    }

    let mut cp = border_cp(paper_w, paper_h);
    let mut shapes: Vec<Option<MoleculeShape>> = Vec::with_capacity(tris.len());
    for t in &tris {
        // 三角形1つは分子1つになるので、返るのも1件。番号は三角形の番号と一致する。
        let made = fill_polygon(
            &mut cp,
            &[pts[t[0]], pts[t[1]], pts[t[2]]],
            paper_w,
            paper_h,
        );
        shapes.push(made.into_iter().next().flatten());
    }

    // 三角形の頂点として使われなかった円中心があれば知らせる。
    for &(id, c) in &packing.centers {
        let used = index_of(&pts, c).is_some_and(|i| tris.iter().any(|t| t.contains(&i)));
        if !used {
            let name = limb_name(skeleton, id);
            warnings.push(overlapping_limb_warning(&name));
        }
    }

    let violations = local_violations(&cp).len();
    if violations > 0 {
        warnings.push(foldability_warning(violations));
    }
    for _ in validate(&cp) {
        warnings.push(INVALID_CREASES_WARNING.to_string());
    }

    // 先端と展開図の対応(作業9)。出来上がった展開図を読むだけで、展開図には
    // 手を入れない。どの先端がどの点・どの分子になったかをここで残しておくと、
    // 後から幾何を当てはめて推測する必要がなくなる。
    let sites: Vec<LeafSite> = packing
        .leaf_circles(skeleton)
        .into_iter()
        .map(|circle| LeafSite {
            vertex: material_vertex(&cp, circle.center),
            molecules: index_of(&pts, circle.center)
                .map(|point| triangles_at(&tris, point))
                .unwrap_or_default(),
            circle,
        })
        .collect();

    // 折り方の計画に使う追跡情報(作業17)。折り線を引いたときに決まっていた値
    // (分子の角の正体・内心・ちょうつがい線の行き先)をそのまま書き留めるだけで、
    // 展開図には手を入れない。
    let mut corner_of = vec![MoleculeCorner::Unknown; pts.len()];
    for (circle_index, &(leaf_id, center)) in packing.centers.iter().enumerate() {
        if let Some(i) = index_of(&pts, center)
            && corner_of[i] == MoleculeCorner::Unknown
        {
            corner_of[i] = MoleculeCorner::Leaf {
                leaf_id,
                circle_index,
            };
        }
    }
    for (corner_index, corner) in paper_corners.iter().enumerate() {
        if let Some(i) = index_of(&pts, *corner)
            && corner_of[i] == MoleculeCorner::Unknown
        {
            corner_of[i] = MoleculeCorner::PaperCorner { corner_index };
        }
    }
    let trace = crate::trace::build(skeleton, &pts, &corner_of, &tris, &shapes);

    Ok(ProposalResult {
        cp,
        violations,
        warnings,
        sites,
        trace,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::SkeletonNode;

    fn named_limb_skeleton() -> Skeleton {
        let root_id = 90;
        let mut nodes = vec![SkeletonNode::new(root_id, None, 0.0)];
        for id in [41, 7, 83, 2, 68, 15, 56, 31, 74] {
            nodes.push(SkeletonNode::new(id, Some(root_id), 1.0));
        }
        Skeleton { nodes }
    }

    #[test]
    fn limb_names_match_the_proposal_input_rows_without_using_ids() {
        let skeleton = named_limb_skeleton();
        let ids = skeleton.leaves();
        let names: Vec<String> = ids.iter().map(|&id| limb_name(&skeleton, id)).collect();
        assert_eq!(
            names,
            [
                "頭",
                "尾",
                "右前足",
                "左前足",
                "右後足",
                "左後足",
                "右の羽",
                "左の羽",
                "出っぱり9",
            ]
        );
        assert_eq!(limb_name(&skeleton, 999), "該当する出っぱり");
    }

    #[test]
    fn generation_messages_use_screen_words_and_explain_the_next_action() {
        assert_eq!(
            position_outside_paper_warning("頭"),
            "頭の置き場所が紙からはみ出しています。「選び直す」で戻り、さらに「形を直す」を選んで、出っぱりの数や配置を見直してください"
        );
        assert_eq!(
            overlapping_limb_warning("尾"),
            "尾が別の出っぱりと同じ位置にあります。「選び直す」で別の候補を選ぶか、さらに「形を直す」で戻って尾の長さか太さを変えてください"
        );
        assert_eq!(
            foldability_warning(3),
            "平らに折りたたみにくい点が3か所あります。「選び直す」で別の候補を選ぶか、この展開図を使った後に折り線の交点を動かしてください"
        );

        let messages = [
            PAPER_SIZE_ERROR.to_string(),
            NO_PLACEMENT_ERROR.to_string(),
            STRAIGHT_LINE_WARNING.to_string(),
            INVALID_CREASES_WARNING.to_string(),
            position_outside_paper_warning("頭"),
            overlapping_limb_warning("尾"),
            foldability_warning(3),
        ];
        let forbidden = [
            "骨格",
            "充填",
            "ソルバー",
            "ヤコビアン",
            "hard",
            "soft",
            "warm start",
            "イテレーション",
            "角",
            "円",
        ];
        for message in messages {
            let lower = message.to_ascii_lowercase();
            for word in forbidden {
                assert!(
                    !lower.contains(word),
                    "利用者向けの文に内部用語「{word}」が含まれている: {message}"
                );
            }
        }
    }

    #[test]
    fn generated_warning_identifies_the_same_limb_name_as_the_screen() {
        // `pack()`は中心を必ず紙内へclampするため、この状況(中心そのものが
        // 紙の外)は`generate()`が壊れた入力から利用者を守れているかを見る
        // ための人為的な構成。案A(中心が紙内)の制約が破れた本当の問題。
        let skeleton = named_limb_skeleton();
        let packing = Packing {
            scale: 1.0,
            centers: vec![(41, [-0.1, 0.5]), (7, [0.75, 1.2])],
            violation: 0.1,
            circles: Vec::new(),
        };
        let result = generate(&skeleton, &packing, 1.0, 1.0).unwrap();
        assert!(
            result
                .warnings
                .contains(&position_outside_paper_warning("頭"))
        );
        assert!(
            result
                .warnings
                .contains(&position_outside_paper_warning("尾"))
        );
    }

    /// 円が紙からはみ出しても、中心が紙内にあれば警告しない(案Aは正常な配置)。
    /// 実測根拠: `scratchpad/containment-report.md` §1.2。鶴・カエルの基本形
    /// は実際に折った9本すべてで、はみ出しても出っぱりの長さが要求どおり
    /// 届いていた(不足0)。
    #[test]
    fn circle_overflow_with_center_inside_paper_is_not_warned() {
        let skeleton = named_limb_skeleton();
        // 半径1.0の円を紙の隅(中心は紙内)に置く。円は紙を大きくはみ出す。
        let packing = Packing {
            scale: 1.0,
            centers: vec![(41, [0.0, 0.0]), (7, [1.0, 1.0])],
            violation: 0.0,
            circles: Vec::new(),
        };
        let result = generate(&skeleton, &packing, 1.0, 1.0).unwrap();
        assert!(
            result.warnings.iter().all(|w| !w.contains("はみ出")),
            "円のはみ出しだけで警告が出た(誤警告): {:?}",
            result.warnings
        );
    }

    /// 四角形の軸多角形を扇状分割で埋められること(ドロネー分割は三角形しか
    /// 返さないので、この経路はここで確認しておく)。
    #[test]
    fn quad_is_filled_by_fanning_into_triangles() {
        let mut cp = border_cp(1.0, 1.0);
        let quad = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        fill_polygon(&mut cp, &quad, 1.0, 1.0);
        // 扇状分割で2つの三角形になり、それぞれに内心ができる。
        assert!(ori3_cp::extract_faces(&cp).len() >= 4);
        assert!(ori3_cp::validate(&cp).is_empty());
        assert!(cp.edges.iter().any(|e| e.kind == EdgeKind::Mountain));
    }

    /// 頂点が2つ以下の多角形は何もしない(潰れた入力で落ちないこと)。
    #[test]
    fn degenerate_polygon_is_ignored() {
        let mut cp = border_cp(1.0, 1.0);
        let before = cp.edges.len();
        fill_polygon(&mut cp, &[[0.0, 0.0], [1.0, 1.0]], 1.0, 1.0);
        fill_polygon(&mut cp, &[], 1.0, 1.0);
        assert_eq!(cp.edges.len(), before);
    }
}
