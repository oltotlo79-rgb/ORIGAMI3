//! 完成の目標にどれだけ近いかを測る4つの物差し(作業20)。
//!
//! 利用者が指定するのは **角の数・長さ・太さ・位置** の4つである(PRO-001 / PRO-006 /
//! PRO-007)。折り順を探すときには「いま作れている形が、指定した完成形へどれだけ
//! 近づいたか」を数で言えなければ、手の良し悪しを比べられない。ここに置くのは
//! その4つの物差しだけで、**折り順の探索そのものは作らない**(作業21・22)。
//!
//! ## 4つの物差し
//!
//! | 物差し | 何を測るか | 関数 |
//! |---|---|---|
//! | 数 | 指定した本数の角が出ているか | [`count_gap`] |
//! | 長さ | 各角の長さが指定にどれだけ近いか | [`length_gap`] |
//! | 太さ | 各角の太さが指定にどれだけ近いか | [`width_gap`] |
//! | 位置 | 各先端が完成形で指定した位置にどれだけ近いか | [`position_gap`] |
//!
//! どれも **0.0 が最良** で、大きいほど完成形から遠い。4つは互いに独立した関数で、
//! 1つだけを取り出して使える。**4つを1つの順位へまとめる重み付けはここでは決めない**
//! (作業22)。[`finish_gaps`] は4つを並べて返すだけで、足し合わせない。
//!
//! ## 測る相手
//!
//! - 目標 [`FinishTarget`]: 骨格([`Skeleton`])に書かれた利用者の指定。
//! - 実際 [`FinishedForm`]: 候補が実際に作る完成形を測った値。
//!
//! [`FinishedForm`] は先端IDで目標と突き合わせる。どの先端がどの紙の場所から
//! 来たのかは **作業9で残した対応**([`LeafSite`])をそのまま使い、座標を後から
//! 当てはめて推測することはしない。
//!
//! ## いまこの場で測れること / 測れないこと
//!
//! [`FinishedForm::from_proposal`] は紙の上の配置だけを見て、数・長さ・太さを測る。
//! 完成形での先端の位置は紙を折り上げないと決まらないため、この道では
//! **位置を測らない**(`pos` は `None` のまま)。紙の上の配置の中心を利用者の
//! 完成位置として扱う道は1つも作っていない(PRO-007)。折り上げた紙で先端の位置を
//! 測ったら、[`FinishedForm::with_tip_points`] で入れる。
//!
//! 長さと太さについても、紙の上の測り方では2つを分けられない。円の半径は
//! 「長さ × 太さ」で決まるので([`Skeleton::leaf_radius`])、紙の上で足りない量は
//! 同じ割合で長さにも太さにも出る。折り上げた形なら「軸の根元から先端までの距離」と
//! 「先端の幅」を別々に測れるので、そこで分かれる(作業21以降)。

use std::f64::consts::SQRT_2;

use crate::generate::{LeafSite, ProposalResult};
use crate::packing::Packing;
use crate::skeleton::{Skeleton, TipPos2d};

/// 位置の隔たりが取りうる最大値。位置の枠は横も縦も `-1.0`〜`1.0` なので、
/// いちばん離れた2点は正方形の対角にあたる `2√2`(約2.828)離れている。
pub const POSITION_GAP_MAX: f64 = 2.0 * SQRT_2;

/// `2.0`より小さい最大の`f64`。有限の太さ係数を折り上がり目盛りへ写す上限。
const FOLDED_WIDTH_MAX: f64 = f64::from_bits(2.0_f64.to_bits() - 1);

/// 紙上の太さ係数`tan(theta)`を、折り上がり測定の`2 sin(theta)`へ写す。
///
/// `hypot`を使って`width_factor * width_factor`のoverflowを避ける。数学上は有限の
/// `width_factor`に対して必ず2未満だが、極端に大きい値は丸めで2になりうるため、
/// その場合だけ2の直前の有限値へ戻す。入力の妥当性は[`Skeleton::validate_structure`]が
/// 従来どおり検査し、ここでは新しい停止条件を加えない。
fn folded_width_from_factor(width_factor: f64) -> f64 {
    let folded = 2.0 * (width_factor / width_factor.hypot(1.0));
    if folded >= 2.0 {
        FOLDED_WIDTH_MAX
    } else {
        folded
    }
}

/// 目標の先端1本ぶん(利用者の指定)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetTip {
    /// 骨格の葉ID。
    pub leaf_id: u32,
    /// 指定した長さ(骨格の `length`)。
    pub length: f64,
    /// 指定した太さ。折り上がり目標では`2 sin(theta)`、紙上測定では
    /// `width_factor = tan(theta)`の目盛りを使う。作り方は[`FinishTarget`]を参照。
    pub width: f64,
    /// 完成形で指定した位置。`None` は「位置を指定していない」。
    pub pos: Option<TipPos2d>,
}

/// 完成の目標(利用者が指定した角の数・長さ・太さ・位置)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FinishTarget {
    /// 先端は葉IDの昇順で、1つの葉IDは高々1件。
    pub tips: Vec<TargetTip>,
}

impl FinishTarget {
    /// 骨格から、折り上がり探索の目標を取り出す。角(葉)でない節点は入らない。
    ///
    /// 骨格と画面の太さは紙上の比`width_factor = tan(theta)`、
    /// [`crate::search::FoldGoal::measure`]の太さは`2 sin(theta)`なので、ここで一度だけ
    /// `folded_width_from_factor`を通す。本番では`plan_folds`がこのconstructorを使う。
    #[must_use]
    pub fn from_skeleton(skeleton: &Skeleton) -> Self {
        Self::from_skeleton_with_width(skeleton, folded_width_from_factor)
    }

    /// 紙上の配置だけを測るときの目標を取り出す。
    ///
    /// [`FinishedForm::from_proposal`]は紙上円の半径しか見られず、長さと太さを分離できない。
    /// その経路に限っては、従来どおり生の`width_factor`を同じ目盛りのまま使う。
    #[must_use]
    pub fn from_skeleton_on_paper(skeleton: &Skeleton) -> Self {
        Self::from_skeleton_with_width(skeleton, std::convert::identity)
    }

    fn from_skeleton_with_width(skeleton: &Skeleton, width_of: impl Fn(f64) -> f64) -> Self {
        let mut ids = skeleton.leaves();
        ids.sort_unstable();
        let tips = ids
            .iter()
            .filter_map(|&id| skeleton.node(id))
            .map(|n| TargetTip {
                leaf_id: n.id,
                length: n.length,
                width: width_of(n.width_factor),
                pos: n.tip_pos_2d,
            })
            .collect();
        Self { tips }
    }

    /// 葉IDで目標の先端を引く。
    #[must_use]
    pub fn tip(&self, leaf_id: u32) -> Option<&TargetTip> {
        self.tips.iter().find(|t| t.leaf_id == leaf_id)
    }
}

/// 実際に作れた完成形の先端1本ぶん。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredTip {
    /// 骨格の葉ID。目標との突き合わせはこのIDだけで行う。
    pub leaf_id: u32,
    /// この先端の材料になる展開図の点(作業9の対応)。座標から推測した値ではない。
    pub material_vertex: Option<u32>,
    /// 届いた長さ(骨格の `length` と同じ単位)。出ていない先端は `0.0`。
    pub length: f64,
    /// 届いた太さ。折り上がり測定では`2 sin(theta)`、紙上測定では
    /// `width_factor = tan(theta)`の目盛りを使う。出ていない先端は`0.0`。
    pub width: f64,
    /// 完成形で測った位置。まだ測っていなければ `None`。
    pub pos: Option<TipPos2d>,
}

impl MeasuredTip {
    /// 角として出ているか。長さが0以下・数にならない値のときは出ていないとみなす。
    #[must_use]
    pub fn is_present(&self) -> bool {
        self.length.is_finite() && self.length > 0.0
    }
}

/// 実際に作れた完成形(測った値)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FinishedForm {
    /// 先端は葉IDの昇順で、1つの葉IDは高々1件。
    pub tips: Vec<MeasuredTip>,
}

impl FinishedForm {
    /// 目標をそのまま実現した完成形。4つの物差しがすべて最良(0.0)になる入力で、
    /// 物差しの目盛りが合っているかを確かめるときの基準になる。
    #[must_use]
    pub fn matching(target: &FinishTarget) -> Self {
        Self {
            tips: target
                .tips
                .iter()
                .map(|t| MeasuredTip {
                    leaf_id: t.leaf_id,
                    material_vertex: None,
                    length: t.length,
                    width: t.width,
                    pos: t.pos,
                })
                .collect(),
        }
    }

    /// 提案した展開図から、数・長さ・太さを紙上の目盛りで測る。
    ///
    /// どの先端がどの紙の場所になったかは、生成のその場で残した対応
    /// ([`LeafSite`]、作業9)をそのまま使う。座標を突き合わせて当て直すことはしない。
    ///
    /// - **出ているか**: その先端の材料点が展開図にあり、まわりを埋めた分子が
    ///   1つ以上あること。どちらも無ければ紙の上に角の材料が無いので「出ていない」。
    /// - **届く長さ**: 円の半径が、紙の上で他の先端とぶつかっているぶんだけ短くなる。
    ///   2つの先端の中心の距離が必要な距離に足りないとき、足りない量を2本で
    ///   半分ずつ負担するとみなし、いちばん厳しい相手のぶんを引く。
    /// - **長さと太さの分け方**: 円の半径は「長さ × 太さ」で決まるため、紙の上では
    ///   2つを分けられない。足りない割合を長さと太さへ同じだけ配る。折り上げた形で
    ///   別々に測れるようになるのは作業21以降。
    ///
    /// 目標には[`FinishTarget::from_skeleton_on_paper`]を使う。完成形での位置はここでは
    /// 測らない(`pos`は`None`)。紙の上の配置の中心を完成位置として扱わないため
    /// (PRO-007)。
    #[must_use]
    pub fn from_proposal(skeleton: &Skeleton, packing: &Packing, result: &ProposalResult) -> Self {
        let sites: &[LeafSite] = &result.sites;
        let scale = packing.scale;
        let usable_scale = scale.is_finite() && scale > 0.0;
        let mut ids = skeleton.leaves();
        ids.sort_unstable();

        let tips = ids
            .iter()
            .map(|&leaf_id| {
                let target_length = skeleton.node(leaf_id).map_or(0.0, |n| n.length);
                let target_width = skeleton.node(leaf_id).map_or(0.0, |n| n.width_factor);
                let site = sites.iter().find(|s| s.circle.leaf_id == leaf_id);
                let material_vertex = site.and_then(|s| s.vertex).map(|v| v.id);
                let materialised =
                    site.is_some_and(|s| s.vertex.is_some() && !s.molecules.is_empty());
                let reach = if usable_scale && materialised {
                    site.map_or(0.0, |s| reached_radius(s, sites, skeleton, scale))
                } else {
                    0.0
                };
                // 円の半径 = 縮尺 × 長さ × 太さ。届いた割合を長さと太さへ同じだけ配る。
                let want_radius = scale * target_length * target_width;
                let ratio = if want_radius.is_finite() && want_radius > 0.0 {
                    (reach / want_radius).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                MeasuredTip {
                    leaf_id,
                    material_vertex,
                    length: target_length * ratio,
                    width: target_width * ratio,
                    pos: None,
                }
            })
            .collect();
        Self { tips }
    }

    /// 折り上げた形で測った先端の位置を入れる。
    ///
    /// `body_center` は完成形の胴の中心(骨格の根にあたる点)、`tip_points` は
    /// 「葉ID → その先端が来た点」で、単位は何でもよい(同じ物差しで測った点なら
    /// mmでも画面の点でもよい)。突き合わせは葉IDだけで行い、座標から相手を
    /// 当てにいくことはしない。
    ///
    /// ## 枠のそろえ方
    ///
    /// 胴の中心を原点に置き、**いちばん遠い先端が、指定のいちばん遠い先端と
    /// 同じ遠さに来るように**縮める。位置が測る相手は「先端の並び方」であり、
    /// 全体の大きさは長さの物差し([`length_gap`])が測るので、位置で大きさを
    /// 二重に数えないための決め方である。遠さは [`TipPos2d`] の枠と同じく
    /// 「中心から正方形の辺までの長さ」で測る。
    ///
    /// 位置の指定が1つも無いときは、いちばん遠い先端が枠の縁(`1.0`)に乗る
    /// ようにそろえる。
    ///
    /// - 葉IDが一致する先端にだけ入る。知らない葉IDの点は捨てる。
    /// - 数にならない点は入れない(`pos` は `None` のまま = 測っていない扱い)。
    /// - すべての点が胴の中心と重なっているときは、全部を原点 `(0,0)` とする。
    #[must_use]
    pub fn with_tip_points(
        mut self,
        target: &FinishTarget,
        body_center: [f64; 2],
        tip_points: &[(u32, [f64; 2])],
    ) -> Self {
        let center_ok = body_center[0].is_finite() && body_center[1].is_finite();
        let usable = |p: &[f64; 2]| center_ok && p[0].is_finite() && p[1].is_finite();
        let measured_half = tip_points
            .iter()
            .filter(|(_, p)| usable(p))
            .map(|(_, p)| {
                (p[0] - body_center[0])
                    .abs()
                    .max((p[1] - body_center[1]).abs())
            })
            .fold(0.0_f64, f64::max);
        let target_half = target
            .tips
            .iter()
            .filter_map(|t| t.pos)
            .filter(TipPos2d::is_valid)
            .map(|p| p.x.abs().max(p.y.abs()))
            .fold(0.0_f64, f64::max);
        let target_half = if target_half > 0.0 { target_half } else { 1.0 };
        let factor = if measured_half > 0.0 {
            target_half / measured_half
        } else {
            0.0
        };
        for (leaf_id, p) in tip_points {
            if !usable(p) {
                continue;
            }
            let Some(tip) = self.tips.iter_mut().find(|t| t.leaf_id == *leaf_id) else {
                continue;
            };
            tip.pos = Some(TipPos2d::new(
                (p[0] - body_center[0]) * factor,
                (p[1] - body_center[1]) * factor,
            ));
        }
        self
    }

    /// 葉IDで測った先端を引く。
    #[must_use]
    pub fn tip(&self, leaf_id: u32) -> Option<&MeasuredTip> {
        self.tips.iter().find(|t| t.leaf_id == leaf_id)
    }
}

/// 1本の先端が紙の上で届く半径。他の先端と近すぎるぶんだけ短くなる。
///
/// 2つの先端の円の中心は、縮尺 × (半径a + 間の胴の長さ + 半径b) だけ離れている
/// 必要がある([`Skeleton::leaf_distance`])。足りないときは、足りない量を2本で
/// 半分ずつ負担するとみなす。いちばん厳しい相手のぶんだけ半径を縮める。
fn reached_radius(site: &LeafSite, sites: &[LeafSite], skeleton: &Skeleton, scale: f64) -> f64 {
    let radius = site.circle.radius;
    if !radius.is_finite() || radius <= 0.0 {
        return 0.0;
    }
    let mut worst = 0.0_f64;
    for other in sites {
        if other.circle.leaf_id == site.circle.leaf_id {
            continue;
        }
        let need = scale * skeleton.leaf_distance(site.circle.leaf_id, other.circle.leaf_id);
        let gap = (site.circle.center[0] - other.circle.center[0])
            .hypot(site.circle.center[1] - other.circle.center[1]);
        if !need.is_finite() || !gap.is_finite() {
            continue;
        }
        worst = worst.max(need - gap);
    }
    (radius - worst.max(0.0) * 0.5).max(0.0)
}

/// 【数】指定した本数の角が出ているか。0.0が最良。
///
/// 出ていない角と、指定していないのに出ている角を数え、指定した本数で割る。
/// 例: 4本の指定のうち1本が出ていなければ `0.25`。角の指定が0本なら `0.0`。
#[must_use]
pub fn count_gap(target: &FinishTarget, form: &FinishedForm) -> f64 {
    let wanted = target.tips.len();
    if wanted == 0 {
        return 0.0;
    }
    let missing = target
        .tips
        .iter()
        .filter(|t| !form.tip(t.leaf_id).is_some_and(MeasuredTip::is_present))
        .count();
    let extra = form
        .tips
        .iter()
        .filter(|m| m.is_present() && target.tip(m.leaf_id).is_none())
        .count();
    (missing + extra) as f64 / wanted as f64
}

/// 【長さ】各角の長さが指定にどれだけ近いか。0.0が最良。
///
/// 先端ごとに `|届いた長さ − 指定した長さ| ÷ 指定した長さ` を出し、平均する。
/// 出ていない角は「届いた長さ0」として数えるので、その先端は `1.0` になる。
#[must_use]
pub fn length_gap(target: &FinishTarget, form: &FinishedForm) -> f64 {
    relative_gap(target, form, |t| t.length, |m| m.length)
}

/// 【太さ】各角の太さが指定にどれだけ近いか。0.0が最良。
///
/// 数え方は [`length_gap`] と同じで、見るのが太さになる。
/// 目標と実測は同じ目盛り同士を渡す。折り上がり探索では
/// [`FinishTarget::from_skeleton`]、紙上測定では
/// [`FinishTarget::from_skeleton_on_paper`]を使う。
#[must_use]
pub fn width_gap(target: &FinishTarget, form: &FinishedForm) -> f64 {
    relative_gap(target, form, |t| t.width, |m| m.width)
}

/// 指定した値との食い違いを、指定した値に対する割合で平均する。
///
/// 指定した値が0以下・数にならない先端は測りようがないので母数から外す。
/// 測った値が0以下・数にならない場合は「届いていない(0)」として数える。
fn relative_gap(
    target: &FinishTarget,
    form: &FinishedForm,
    want: impl Fn(&TargetTip) -> f64,
    got: impl Fn(&MeasuredTip) -> f64,
) -> f64 {
    let mut sum = 0.0;
    let mut counted = 0usize;
    for t in &target.tips {
        let wanted = want(t);
        if !(wanted.is_finite() && wanted > 0.0) {
            continue;
        }
        let reached = form.tip(t.leaf_id).map_or(0.0, |m| {
            let v = got(m);
            if v.is_finite() && v > 0.0 { v } else { 0.0 }
        });
        sum += (reached - wanted).abs() / wanted;
        counted += 1;
    }
    if counted == 0 {
        0.0
    } else {
        sum / counted as f64
    }
}

/// 【位置】各先端が完成形で指定した位置にどれだけ近いか(PRO-006 / PRO-007)。0.0が最良。
///
/// 先端ごとに、指定した位置と測った位置の距離を、枠の対角([`POSITION_GAP_MAX`])で
/// 割って平均する。したがって枠の中で測れているかぎり0.0〜1.0に収まる。
///
/// - **位置を指定していない先端は母数から外す。** 指定していないものに勝手な
///   正解を作ると、利用者が実際に指定した先端の一致がその分だけ薄まるため。
///   1本も指定が無ければ 0.0 を返し、順位付けに影響しない。
/// - **指定があるのに測っていない先端は、いちばん遠い(1.0)として数える。**
///   「測っていない」を「合っている」と扱うと、位置を測れない候補が最良に
///   見えてしまうため。
/// - 指定が枠の外・数にならない値のときは測りようがないので母数から外す。
#[must_use]
pub fn position_gap(target: &FinishTarget, form: &FinishedForm) -> f64 {
    let mut sum = 0.0;
    let mut counted = 0usize;
    for t in &target.tips {
        let Some(wanted) = t.pos else {
            continue; // 位置を指定していない先端
        };
        if !wanted.is_valid() {
            continue; // 枠の外・数にならない指定は測れない
        }
        counted += 1;
        let measured = form.tip(t.leaf_id).and_then(|m| m.pos);
        let distance = match measured {
            Some(p) if p.x.is_finite() && p.y.is_finite() => (p.x - wanted.x).hypot(p.y - wanted.y),
            _ => POSITION_GAP_MAX,
        };
        sum += distance / POSITION_GAP_MAX;
    }
    if counted == 0 {
        0.0
    } else {
        sum / counted as f64
    }
}

/// 4つの物差しを並べた結果。
///
/// **1つの順位へまとめる重み付けはここでは決めない**(作業22)。この型は4つを
/// 並べて運ぶだけで、足したり掛けたりしない。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinishGaps {
    /// 数の隔たり([`count_gap`])。
    pub count: f64,
    /// 長さの隔たり([`length_gap`])。
    pub length: f64,
    /// 太さの隔たり([`width_gap`])。
    pub width: f64,
    /// 位置の隔たり([`position_gap`])。
    pub position: f64,
}

impl FinishGaps {
    /// いちばん良い値(4つとも0.0)。
    pub const BEST: Self = Self {
        count: 0.0,
        length: 0.0,
        width: 0.0,
        position: 0.0,
    };

    /// 4つとも数になっているか(無限大やNaNでないか)。
    #[must_use]
    pub fn all_finite(&self) -> bool {
        self.count.is_finite()
            && self.length.is_finite()
            && self.width.is_finite()
            && self.position.is_finite()
    }
}

/// 4つの物差しをまとめて測る。合成はしない。
#[must_use]
pub fn finish_gaps(target: &FinishTarget, form: &FinishedForm) -> FinishGaps {
    FinishGaps {
        count: count_gap(target, form),
        length: length_gap(target, form),
        width: width_gap(target, form),
        position: position_gap(target, form),
    }
}
