//! 作業22「4つの目標で順位付けする決定的な探索」。
//!
//! ## 何をするか
//!
//! 作業21の [`FoldSession`] は「その状態から**実際に折れる手**」を返す。
//! ここではその手を1つずつ実際に折ってみて、折った後の形を作業20の
//! **4つの物差し**([`finish_gaps`])で測り、**完成形へ近づく順**に試す。
//!
//! | 元にしたもの | 何を引き継ぐか |
//! |---|---|
//! | 作業20 [`crate::finish`] | 数・長さ・太さ・位置の4つの物差し(0.0が最良) |
//! | 作業21 [`crate::enumerate`] | 実際に折れる手だけを返す列挙 |
//! | 作業18 [`crate::plan`] | 折り線のまとまりの数え方と、打ち切りの考え方 |
//!
//! ## 折り上がった形をどう測るか
//!
//! 4つの物差しは「先端がどこに来て、どれだけ伸びて、どれだけ太いか」を要る。
//! 紙の上の配置だけでは位置が決まらない(作業20 §2.5)ので、ここでは
//! **手順を最後まで再生した姿勢**([`replay`] の `t = 1`)から測る。
//!
//! - どの紙の場所がどの先端になるかは [`FoldGoal::sites`] で**外から与える**。
//!   提案が作った展開図なら作業9の対応([`crate::LeafSite`])がそのまま使え、
//!   作品の展開図なら「紙のこの角が頭」と指定できる。座標から相手を当てにいく
//!   経路は1つも作っていない。
//! - **長さ**: 胴の中心から先端までの距離。全体の大きさは紙の大きさで決まって
//!   しまうので、**いちばん長い先端が指定のいちばん長い先端に合うようにそろえて**
//!   から比べる(作業20 §2.4 の位置と同じそろえ方)。
//! - **太さ**: 先端のまわり [`FLAP_RADIUS`] のところで、紙が軸からどれだけ横に
//!   広がっているか。広がりの幅 ÷ 見た半径 なので、紙の大きさにも見た半径にも
//!   依らない。角がとがるように折り込まれるほど 0 に近づく。
//! - **位置**: 先端の来た点を折り上がりの平面へ落とし、[`FinishedForm::with_tip_points`]
//!   に渡す。枠のそろえ方は作業20が決めたとおり。
//! - **数**: その紙の場所が姿勢の中に見つかり、胴の中心から [`MIN_TIP_LENGTH`]
//!   以上離れているものを「出ている」と数える。紙を1点へ畳んでしまった形には
//!   角が1本も無い、という数え方になる。
//!
//! ## 順位の付け方
//!
//! [`GapWeights`] が4つを1つの数へまとめる。**足し方は実測で決めた**。
//! 根拠は `scratchpad/propose-22-report.md` と [`GapWeights::DEFAULT`] のコメントにある。
//!
//! ## 決定的であること
//!
//! - 乱数を使わない。
//! - 順位が並んだときは、**折り線の番号の小さい順**で決める。
//! - 点数どうしの比較は [`SCORE_QUANTUM`] の刻みに丸めてから行う。
//!   計算した小数をそのまま比べると、最下位の桁の違いで順番が入れ替わりうる
//!   (`CLAUDE.md` §10.7.7)。

use std::collections::{BTreeMap, BTreeSet};

use ori3_cp::{Face, extract_faces};
use ori3_layers::replay::replay;
use ori3_model::{CreasePattern, Document, FaceId, VertexId};

use crate::enumerate::{FoldSession, PoseScan, VerifiedMove};
use crate::finish::{FinishGaps, FinishTarget, FinishedForm, MeasuredTip, finish_gaps};

/// 点数を比べるときの刻み。
///
/// 計算した小数をそのまま比べると、最下位の桁が計算機ごとに変わって順番が
/// 入れ替わりうる(`CLAUDE.md` §10.7.7)。この刻みへ丸めてから比べ、
/// 同じ刻みに入った手は折り線の番号の小さい順にする。
///
/// 実測(`crates/ori3-propose/tests/search.rs` の `measure_the_four_gaps_of_every_first_move`、
/// debugビルド): やっこさんの最初の手7つの点数のうち、
///
/// - **意味のある差**でいちばん小さいもの: `1.667e-1`
/// - **同点のはずの2手(手1と手5)の差**: `8.882e-16`
///
/// 刻み `1e-6` はこの2つのあいだにあり、意味のある差より **16万分の1** 細かく、
/// 丸め誤差より **10桁**粗い。手1と手5は同点として扱われ、
/// 折り線の番号の小さいほうが先になる。
///
/// この刻みは実際に効いている。太さを測る半径([`FLAP_RADIUS`])を
/// 0.1 から 0.05 に変えると、手1と手5の点数に `1e-6` より小さい差が出た。
/// 刻みが無ければ、半径の取り方で順番が入れ替わっていた。
pub const SCORE_QUANTUM: f64 = 1e-6;

/// 先端のまわりの紙を見る半径(材料座標。紙の長辺が1.0)。
///
/// この半径の円周上の紙の点が、先端の軸からどれだけ横に広がっているかで
/// 太さを測る。半径そのもので割るので、値は半径に依らない量になる。
///
/// **実測して確かめた**(`measure_the_four_gaps_of_every_first_move`、
/// やっこさんの最初の手7つ、debugビルド)。半径を **0.05 / 0.1 / 0.2** と
/// 4倍の幅で変えても、
///
/// - 4つの物差しの値は小数第6位まで**同じ**
/// - 振れ幅も**同じ**(数 0.250 / 長さ 0.299 / 太さ 1.655 / 位置 0.354)
/// - 手の順位も**同じ**(`[7, 1, 5, 6, 4, 2, 0]`)
///
/// だった。折り鶴・やっこさんの紙の角のまわりには、この範囲に折り目が
/// 入っていないためである。値は 0.1(紙の1割)を採る。
pub const FLAP_RADIUS: f64 = 0.1;

/// 先端のまわりの紙を何方向で見るか。
///
/// **実測**(同上): 12 / 24 / 48 方向で、4つの物差しの値も振れ幅も手の順位も
/// すべて同じだった。紙の角のまわりはまっすぐな扇形で、横の広がりが最大に
/// なるのは扇の端なので、方向を増やしても値が変わらない。24 を採る。
const FLAP_SAMPLES: usize = 24;

/// 紙の上の点が面に入っているとみなす許容誤差。
const CONTAIN_TOL: f64 = 1e-9;

/// 角が「出ている」と数える、胴の中心からの最小の距離(材料座標。紙の長辺が1.0)。
///
/// **これが無いと測り値が壊れる。実測して見つけた不具合への対策である。**
/// やっこさんで座布団折り4本を折り切ると、紙の4隅は**胴の中心にぴったり重なる**。
/// このとき胴からの距離は計算誤差そのもの(実測 `1.15e-16` 〜 `5.02e-16`)になる。
/// 長さも位置も「いちばん遠い先端に合わせてそろえる」(作業20 §2.4)ので、
/// **誤差どうしの比を長さとして拡大**してしまい、
/// 長さ `0.454 / 0.290 / 0.757 / 1.000`、位置 `(-0.4375, -0.25)` のような、
/// 意味のない値が出ていた。
///
/// 実際に出ている角の距離は `0.5` 〜 `0.707`(同じ実測)で、誤差とは **10桁以上**
/// 離れている。しきい値 `1e-6` はその間にあり、どちらに寄せても判定は変わらない。
/// 値そのものは、姿勢の表示精度として使われている `1e-6`
/// (`crates/ori3-layers/src/replay.rs` の `FLAT_EPS`)と同じにした。
///
/// この距離に届かない先端は「出ていない」ものとして、長さ0・太さ0・位置は
/// 測っていない扱いにする。**紙を1点へ畳んでしまった形は、角が1本も無い形である。**
pub const MIN_TIP_LENGTH: f64 = 1e-6;

/// どの紙の場所がどの先端になるか。
///
/// 座標から相手を当てにいくのではなく、**外から与える**。提案が作った展開図なら
/// 作業9の対応([`crate::LeafSite`])から作れる。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TipSite {
    /// 骨格の葉ID。
    pub leaf_id: u32,
    /// その先端になる紙の場所(材料座標)。
    pub material: [f64; 2],
}

/// 完成の目標(利用者の指定 + それがどの紙の場所から来るか)。
#[derive(Clone, Debug, PartialEq)]
pub struct FoldGoal {
    /// 利用者の指定(数・長さ・太さ・位置)。
    pub target: FinishTarget,
    /// 胴の中心にあたる紙の場所(材料座標)。長さと位置の原点になる。
    pub body: [f64; 2],
    /// 先端と紙の場所の対応。
    pub sites: Vec<TipSite>,
}

impl FoldGoal {
    /// 手順を最後まで再生した姿勢から、4つの物差しで測る値を取り出す。
    ///
    /// 姿勢が求まらない・面が取り出せないなど測りようがない場合でも止めず、
    /// 「1本も出ていない」形(長さも太さも0)を返す。**止めずに警告する**という
    /// 設計原則(`CLAUDE.md` §8)に合わせ、探索は次の手へ進める。
    #[must_use]
    pub fn measure(&self, document: &Document) -> FinishedForm {
        let empty = || FinishedForm {
            tips: self
                .sites
                .iter()
                .map(|s| MeasuredTip {
                    leaf_id: s.leaf_id,
                    material_vertex: None,
                    length: 0.0,
                    width: 0.0,
                    pos: None,
                })
                .collect(),
        };
        let faces = extract_faces(&document.cp);
        if faces.is_empty() {
            return empty();
        }
        let replayed = replay(document, document.sequence.len(), 1.0);
        let Some(placer) = Placer::new(&document.cp, &faces, &replayed.frame) else {
            return empty();
        };
        let Some(body) = placer.place(self.body) else {
            return empty();
        };

        // 先端ごとに「来た点」「胴からの距離」「まわりの紙の横の広がり」を測る。
        struct Raw {
            leaf_id: u32,
            point: Option<[f64; 3]>,
            length: f64,
            half_width: f64,
        }
        let raws: Vec<Raw> = self
            .sites
            .iter()
            .map(|site| {
                let Some(point) = placer.place(site.material) else {
                    return Raw {
                        leaf_id: site.leaf_id,
                        point: None,
                        length: 0.0,
                        half_width: 0.0,
                    };
                };
                let axis = sub(point, body);
                let length = norm(axis);
                if !(length.is_finite() && length >= MIN_TIP_LENGTH) {
                    // 胴の中心に重なってしまった先端は「出ていない」。
                    return Raw {
                        leaf_id: site.leaf_id,
                        point: None,
                        length: 0.0,
                        half_width: 0.0,
                    };
                }
                let unit = scale(axis, 1.0 / length);
                let half_width = flap_half_width(&placer, site.material, point, unit);
                Raw {
                    leaf_id: site.leaf_id,
                    point: Some(point),
                    length,
                    half_width,
                }
            })
            .collect();

        // 長さの目盛りをそろえる。紙の大きさは自由に選べるので、そのままでは
        // 指定と比べられない。いちばん長い先端が、指定のいちばん長い先端に
        // 合うようにそろえる(作業20 §2.4 の位置と同じ考え方)。
        let measured_max = raws.iter().map(|r| r.length).fold(0.0_f64, f64::max);
        let target_max = self
            .target
            .tips
            .iter()
            .map(|t| t.length)
            .filter(|v| v.is_finite() && *v > 0.0)
            .fold(0.0_f64, f64::max);
        let target_max = if target_max > 0.0 { target_max } else { 1.0 };
        let length_factor = if measured_max > 0.0 {
            target_max / measured_max
        } else {
            0.0
        };

        let tips = raws
            .iter()
            .map(|r| MeasuredTip {
                leaf_id: r.leaf_id,
                material_vertex: None,
                length: r.length * length_factor,
                // 太さは「見た半径 [`FLAP_RADIUS`] のところで紙が横に広がっている幅 ÷ その半径」。
                // 見る半径で割るので、**紙の大きさにも見る半径にも依らない**値になる
                // (先端の開き角を θ とすると 2 sin θ にあたる)。とがるほど 0 に近づく。
                width: 2.0 * r.half_width / FLAP_RADIUS,
                pos: None,
            })
            .collect();

        // 位置は折り上がりの平面(x, y)で測り、枠のそろえ方は作業20に任せる。
        let points: Vec<(u32, [f64; 2])> = raws
            .iter()
            .filter_map(|r| r.point.map(|p| (r.leaf_id, [p[0], p[1]])))
            .collect();
        FinishedForm { tips }.with_tip_points(&self.target, [body[0], body[1]], &points)
    }
}

/// 4つの物差しを1つの点数へまとめる重み。小さいほど完成形に近い。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GapWeights {
    pub count: f64,
    pub length: f64,
    pub width: f64,
    pub position: f64,
}

impl GapWeights {
    /// 既定の重み。**実測を根拠に決めた**(`scratchpad/propose-22-report.md` 段階1)。
    ///
    /// ## 測ったこと
    ///
    /// やっこさんで最初に折れる7手それぞれについて4つの物差しを測り、
    /// **手ごとの振れ幅**(いちばん大きい手といちばん小さい手の差)を並べた。
    /// 順位を決めるのは値の大きさではなく**手による差**だからである。
    ///
    /// | 物差し | 振れ幅 | 幅 |
    /// |---|---:|---|
    /// | 数 | **0.250** | 0.000〜0.250 |
    /// | 長さ | **0.299** | 0.285〜0.583 |
    /// | 太さ | **1.655** | 3.036〜4.690 |
    /// | 位置 | **0.354** | 0.177〜0.530 |
    ///
    /// 太さだけが他の **5.5倍**(1.655 ÷ 他3つの平均0.301)広い。
    /// 単純な足し算(4つとも重み1)にすると、太さだけで順位が決まってしまう。
    ///
    /// ## 重み1のままにすると実際に起きたこと
    ///
    /// 4つとも重み1で探索すると、やっこさんは
    /// **角を4本とも胴の中心へ畳んで消す手順**(点 4.047 → 4.000)を選んだ。
    /// 角が消えると太さの隔たりが `3.714 → 1.000` と大きく下がり、
    /// 数・長さ・位置がそろって悪化した分(+2.67)を上回るためである。
    /// **角を無くすほど良い点になる**という、目標と正反対の順位が付いていた。
    ///
    /// ## 決めた重み
    ///
    /// 太さだけを **0.2**(振れ幅の比 1/5.5 ≒ 0.18 に近い、きりのよい値)にし、
    /// 残り3つは1.0にする。これで4つの寄与が
    /// 数 0.250 / 長さ 0.299 / 太さ 0.331 / 位置 0.354 とそろい、
    /// 上の「角を消す手順」は 1.076 → 3.200 と**はっきり悪い点**になる。
    ///
    /// 数は、折り鶴・やっこさんの最初の手では振れ幅0.250だが、
    /// 紙の角が消えない状態では0のこともある。**それでも重みは1のまま**にする。
    /// 提案が作った展開図では先端そのものが出ないことがあり(作業20 §3.4)、
    /// そこでは効くためである。
    pub const DEFAULT: Self = Self {
        count: 1.0,
        length: 1.0,
        width: 0.2,
        position: 1.0,
    };

    /// 4つを1つの点数へまとめる。小さいほど良い。
    #[must_use]
    pub fn score(&self, gaps: &FinishGaps) -> f64 {
        self.count * gaps.count
            + self.length * gaps.length
            + self.width * gaps.width
            + self.position * gaps.position
    }
}

impl Default for GapWeights {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 探索の打ち切り条件。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchBudget {
    /// 手を広げてよい状態の数の上限。
    pub max_states: usize,
    /// たどってよい深さ(手数)の上限。
    pub max_depth: usize,
    /// 1つの状態から先へ進めてよい手の数(順位の上位だけを残す)。
    pub branch: usize,
    /// **順位を付けるために**候補をざっと見るときの、途中の姿勢の数。
    ///
    /// ここで確かめられた手はまだ「候補」で、そのまま返すことはしない。
    pub rank_scan: PoseScan,
    /// **返す手を確かめる**ときの、途中の姿勢の数。
    ///
    /// 順位の上位に残った手だけをこの細かさで確かめ直し、
    /// 通らなかった手は捨てる。**返る手はすべてこの細かさを通っている。**
    pub scan: PoseScan,
}

impl SearchBudget {
    /// 既定の打ち切り。**実測を根拠に決めた**(`scratchpad/propose-22-report.md` 段階1)。
    ///
    /// ## なぜ打ち切りが要るか(実測)
    ///
    /// 1つの状態を広げる(折れる手を確かめて、1つずつ折って測る)のに、この既定の
    /// 組み合わせで **折り鶴 2.67秒 / やっこさん 3.02秒**かかる(debugビルド)。
    /// 作業18の測定では深さ4までに最大 **54,988状態**まで広がるので、
    /// 打ち切りが無ければ何時間もかかる。**状態の数で切る**のはこのためである。
    ///
    /// ## 決めた値と根拠
    ///
    /// | 項目 | 値 | 根拠(実測) |
    /// |---|---:|---|
    /// | `max_states` | **12** | 折り鶴は3状態(8.0秒)で手が尽きる。やっこさんは尽きるまで28〜40状態かかり、1状態3.02秒なので85〜120秒になる。12に切ると **36.2秒**で、それでも目標の形へ届いた(点 0.000000) |
    /// | `branch` | **3** | 1つの状態から候補に挙がる手の最大は、やっこさん7・折り鶴1。上位3件に絞ると広がりが 7→3 に下がる |
    /// | `max_depth` | **8** | 実際に折り切れた手数は折り鶴2・やっこさん4。その2倍 |
    /// | `rank_scan` | **3点**(`steps = 2`) | 順位を付けるだけの粗い確認 |
    /// | `scan` | **21点**([`PoseScan::DEFAULT`]) | 作業21が使うのと同じ細かさ。**返す手はすべてこれを通る** |
    ///
    /// ## なぜ2段構えにしたか(実測して分かったこと)
    ///
    /// はじめは探索全体を粗い5点で回していた。ところが**やっこさんの手3は
    /// 5点では折れると判定され、21点では折れない**。粗いまま返すと
    /// 「確かめていない手を返す」ことになる(検査
    /// `every_move_in_the_returned_order_really_folds` が実際に落ちて分かった)。
    ///
    /// そこで、**順位を付けるための粗い確認**と、**返す手のための細かい確認**を
    /// 分けた。粗い確認で落ちた手は候補にも入らず、上位に残った手は
    /// [`FoldSession::verify_move`] で 21点をやり直し、通らなければ捨てる。
    ///
    /// **速さも測った**。`rank_scan` だけを 3点 と 21点 に変え、
    /// 他は同じにして同じ探索を回した結果は次のとおりで、
    /// **返した手順は両方とも同じ**(折り鶴 `[16, 3]`・やっこさん `[0, 7]`)だった。
    ///
    /// | `rank_scan` | 折り鶴 | やっこさん |
    /// |---|---:|---:|
    /// | 21点 | 15.3秒(1状態 5.09秒) | 62.3秒(1状態 5.19秒) |
    /// | **3点** | **8.0秒**(1状態 2.67秒) | **36.2秒**(1状態 3.02秒) |
    ///
    /// **1.7〜1.9倍速い。**
    pub const DEFAULT: Self = Self {
        max_states: 12,
        max_depth: 8,
        branch: 3,
        rank_scan: PoseScan { steps: 2 },
        scan: PoseScan::DEFAULT,
    };
}

/// 探索が止まった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchStop {
    /// 折れる手が尽きた(行ける先を全部見た)。
    Exhausted,
    /// 状態の数の上限で打ち切った。
    StateCap,
    /// 深さの上限で打ち切った。
    DepthCap,
}

/// 選んだ手1つぶんと、折った後の測り値。
#[derive(Clone, Debug, PartialEq)]
pub struct RankedMove {
    /// 折った手(作業21が確かめたもの)。
    pub mv: VerifiedMove,
    /// 折った後の4つの物差し。
    pub gaps: FinishGaps,
    /// 折った後の点数([`GapWeights::score`])。
    pub score: f64,
}

/// 探索の結果。
#[derive(Clone, Debug, PartialEq)]
pub struct SearchOutcome {
    /// 選ばれた手順(先頭から順に折る)。
    pub steps: Vec<RankedMove>,
    /// 折り始める前の4つの物差し。
    pub start_gaps: FinishGaps,
    /// 折り始める前の点数。
    pub start_score: f64,
    /// 返した手順を折り終えたときの4つの物差し。
    pub best_gaps: FinishGaps,
    /// 返した手順を折り終えたときの点数。
    pub best_score: f64,
    /// 手を広げた状態の数。
    pub states_expanded: usize,
    /// 作った状態の数(重複を除く)。
    pub states_generated: usize,
    /// 1つの状態から候補に挙がった手の数の最大([`SearchBudget::branch`] で絞る前)。
    pub max_branching: usize,
    /// 深さの上限に当たった回数。
    pub depth_capped: usize,
    /// 探索が止まった理由。
    pub stop: SearchStop,
}

/// 状態を表す鍵。同じ折り目を折り終えた状態は1度しか広げない。
type StateKey = crate::plan::FoldedMask;

/// 探索の途中の1状態。
#[derive(Clone, Debug)]
struct Node {
    session: FoldSession,
    steps: Vec<RankedMove>,
    score: f64,
    gaps: FinishGaps,
}

/// 順位を決める鍵。点数の刻み → 手数 → 折り線の番号列、の順で比べる。
///
/// 点数を刻みへ丸めてから比べるので、最下位の桁の違いで順番が入れ替わらない。
/// 同じ刻みに入った状態は、**手数が少ないほう**、それも同じなら
/// **折り線の番号が小さいほう**を先にする。どちらも計算した小数を含まないので、
/// 並び方は毎回同じになる。
type RankKey = (i64, usize, Vec<usize>);

fn quantize(score: f64) -> i64 {
    if !score.is_finite() {
        return i64::MAX;
    }
    let scaled = (score / SCORE_QUANTUM).round();
    if scaled >= i64::MAX as f64 {
        i64::MAX
    } else if scaled <= i64::MIN as f64 {
        i64::MIN
    } else {
        scaled as i64
    }
}

fn rank_key(node: &Node) -> RankKey {
    (
        quantize(node.score),
        node.steps.len(),
        node.steps.iter().map(|s| s.mv.id).collect(),
    )
}

/// 完成形へ近づく順に手を選んで探索する。
///
/// ## 何を返すか
///
/// **たどった状態のうち、4つの物差しの点数がいちばん良かったところまでの手順**を返す。
/// 目標は「完成形の形」であって「折り目を全部折ること」ではないので、
/// 途中で目標にいちばん近くなるならそこで返す。打ち切りに達した場合も、
/// **そこまでで見つけたいちばん良い手順**が同じ規則で返る。
///
/// どの手を折っても目標から遠ざかる展開図では、**手順0件**(折らないのが最良)が返る。
/// これは異常ではなく、与えた目標が平らな紙ですでに満たされているという意味である。
///
/// ## どこで4つの物差しが効くか
///
/// 1. 広げる状態を選ぶ順(点数の良い状態から先に広げる)。
/// 2. 1つの状態から先へ残す手([`SearchBudget::branch`] 件だけ、点数の良い順)。
/// 3. 返す手順を選ぶとき。
///
/// 同じ入力なら毎回同じ手順を返す(乱数を使わず、点数は [`SCORE_QUANTUM`] の
/// 刻みへ丸めてから比べ、並んだときは折り線の番号で決める)。
#[must_use]
pub fn search_to_finish(
    session: &FoldSession,
    goal: &FoldGoal,
    weights: GapWeights,
    budget: SearchBudget,
) -> SearchOutcome {
    let start_form = goal.measure(session.document());
    let start_gaps = finish_gaps(&goal.target, &start_form);
    let start_score = weights.score(&start_gaps);

    let root = Node {
        session: session.clone(),
        steps: Vec::new(),
        score: start_score,
        gaps: start_gaps,
    };
    let mut outcome = SearchOutcome {
        steps: Vec::new(),
        start_gaps,
        start_score,
        best_gaps: start_gaps,
        best_score: start_score,
        states_expanded: 0,
        states_generated: 1,
        max_branching: 0,
        depth_capped: 0,
        stop: SearchStop::Exhausted,
    };
    // たどった中でいちばん点数の良かった状態。打ち切りに達してもこれを返す。
    let mut best: (RankKey, Node) = (rank_key(&root), root.clone());
    let mut capped = false;

    let mut seen: BTreeSet<StateKey> = BTreeSet::from([session.folded_mask()]);
    let mut frontier: BTreeMap<RankKey, Node> = BTreeMap::from([(rank_key(&root), root)]);

    while let Some((_, node)) = frontier.pop_first() {
        if node.steps.len() >= budget.max_depth {
            outcome.depth_capped += 1;
            outcome.stop = SearchStop::DepthCap;
            capped = true;
            continue;
        }
        if outcome.states_expanded >= budget.max_states {
            outcome.stop = SearchStop::StateCap;
            capped = true;
            break;
        }
        outcome.states_expanded += 1;

        let (children, candidates) = expand(&node, goal, weights, budget);
        outcome.max_branching = outcome.max_branching.max(candidates);
        for child in children {
            if !seen.insert(child.session.folded_mask()) {
                continue;
            }
            outcome.states_generated += 1;
            let key = rank_key(&child);
            if key < best.0 {
                best = (key, child.clone());
            }
            frontier.insert(rank_key(&child), child);
        }
    }
    if !capped {
        outcome.stop = SearchStop::Exhausted;
    }

    outcome.steps = best.1.steps;
    outcome.best_gaps = best.1.gaps;
    outcome.best_score = best.1.score;
    outcome
}

/// 1つの状態から、次に折れる手を順位の良い順に並べて返す。
///
/// 2段構えである。
///
/// 1. [`SearchBudget::rank_scan`] の粗さで候補を集め、1つずつ折って点数を付ける。
/// 2. 点数の良い順に [`SearchBudget::branch`] 件だけ残し、
///    その手を [`SearchBudget::scan`] の細かさで**確かめ直す**。
///    通らなかった手は捨てる。
///
/// 返る手はすべて細かい確認を通っているので、
/// **確かめていない手が手順に入ることはない**。
fn expand(
    node: &Node,
    goal: &FoldGoal,
    weights: GapWeights,
    budget: SearchBudget,
) -> (Vec<Node>, usize) {
    let report = node.session.verified_moves(budget.rank_scan);
    let mut ranked: Vec<(usize, FinishGaps, f64)> = Vec::new();
    for mv in &report.verified {
        let mut next = node.session.clone();
        if next.apply(mv).is_err() {
            continue; // 確かめた直後の状態から動いている場合。止めずに次の手へ。
        }
        let gaps = finish_gaps(&goal.target, &goal.measure(next.document()));
        ranked.push((mv.id, gaps, weights.score(&gaps)));
    }
    ranked.sort_by(|a, b| quantize(a.2).cmp(&quantize(b.2)).then_with(|| a.0.cmp(&b.0)));
    let candidates = ranked.len();

    let mut children: Vec<Node> = Vec::new();
    for (id, gaps, score) in ranked {
        if children.len() >= budget.branch {
            break;
        }
        let Some(mv) = node.session.verify_move(id, budget.scan) else {
            continue; // 粗く見たときは折れたが、細かく見ると折れない手。捨てる。
        };
        let mut next = node.session.clone();
        if next.apply(&mv).is_err() {
            continue;
        }
        let mut steps = node.steps.clone();
        steps.push(RankedMove { mv, gaps, score });
        children.push(Node {
            session: next,
            steps,
            score,
            gaps,
        });
    }
    (children, candidates)
}

/// 材料座標の点を、折り上がりの姿勢の点へ移す。
///
/// 面の中の点は面ごとの剛体移動で移るので、面の頂点3つが分かれば残りは決まる。
/// 面は番号の小さい順に見て、**最初にその点を含んだ面**を使う(毎回同じ面になる)。
struct Placer {
    faces: Vec<PlacedFace>,
}

/// 面1つぶんの、材料座標と姿勢座標の対応。
struct PlacedFace {
    /// 材料座標の多角形(点がこの面に入っているかを見る)。
    polygon: Vec<[f64; 2]>,
    /// 一直線に並んでいない頂点3つの材料座標。
    material: [[f64; 2]; 3],
    /// 同じ3点の姿勢座標。
    placed: [[f64; 3]; 3],
}

impl Placer {
    fn new(cp: &CreasePattern, faces: &[Face], frame: &ori3_model::Frame3D) -> Option<Self> {
        let pos: BTreeMap<VertexId, [f64; 2]> = cp.vertices.iter().map(|v| (v.id, v.pos)).collect();
        let placed: BTreeMap<FaceId, &ori3_model::Face3D> =
            frame.faces.iter().map(|f| (f.face, f)).collect();
        let mut out = Vec::new();
        for face in faces {
            let Some(face3d) = placed.get(&face.id) else {
                continue;
            };
            if face3d.polygon.len() != face.vertices.len() {
                continue;
            }
            let material: Vec<[f64; 2]> = face
                .vertices
                .iter()
                .filter_map(|v| pos.get(v).copied())
                .collect();
            if material.len() != face.vertices.len() {
                continue;
            }
            let Some(basis) = pick_basis(&material) else {
                continue;
            };
            let m = [material[basis[0]], material[basis[1]], material[basis[2]]];
            let q = [
                face3d.polygon[basis[0]],
                face3d.polygon[basis[1]],
                face3d.polygon[basis[2]],
            ];
            if !q.iter().flatten().all(|v| v.is_finite()) {
                continue;
            }
            out.push(PlacedFace {
                polygon: material,
                material: m,
                placed: q,
            });
        }
        if out.is_empty() { None } else { Some(Self { faces: out }) }
    }

    /// 材料座標の点を姿勢の点へ移す。どの面にも入っていなければ `None`。
    fn place(&self, point: [f64; 2]) -> Option<[f64; 3]> {
        for face in &self.faces {
            let (m, q) = (&face.material, &face.placed);
            if !contains(&face.polygon, point) {
                continue;
            }
            let u = [m[1][0] - m[0][0], m[1][1] - m[0][1]];
            let v = [m[2][0] - m[0][0], m[2][1] - m[0][1]];
            let d = u[0] * v[1] - u[1] * v[0];
            if d.abs() <= CONTAIN_TOL {
                continue;
            }
            let w = [point[0] - m[0][0], point[1] - m[0][1]];
            let a = (w[0] * v[1] - w[1] * v[0]) / d;
            let b = (u[0] * w[1] - u[1] * w[0]) / d;
            let mut out = [0.0; 3];
            for k in 0..3 {
                out[k] = q[0][k] + a * (q[1][k] - q[0][k]) + b * (q[2][k] - q[0][k]);
            }
            if out.iter().all(|v| v.is_finite()) {
                return Some(out);
            }
        }
        None
    }
}

/// 一直線に並んでいない頂点3つを選ぶ。
fn pick_basis(polygon: &[[f64; 2]]) -> Option<[usize; 3]> {
    let n = polygon.len();
    if n < 3 {
        return None;
    }
    let mut best: Option<([usize; 3], f64)> = None;
    for j in 1..n {
        for k in (j + 1)..n {
            let u = [polygon[j][0] - polygon[0][0], polygon[j][1] - polygon[0][1]];
            let v = [polygon[k][0] - polygon[0][0], polygon[k][1] - polygon[0][1]];
            let area = (u[0] * v[1] - u[1] * v[0]).abs();
            if best.is_none_or(|(_, b)| area > b) {
                best = Some(([0, j, k], area));
            }
        }
    }
    best.filter(|(_, area)| *area > CONTAIN_TOL).map(|(i, _)| i)
}

/// 多角形の内側か縁にあるか(縁は含む)。
fn contains(polygon: &[[f64; 2]], p: [f64; 2]) -> bool {
    let n = polygon.len();
    if n < 3 {
        return false;
    }
    // 縁の上(頂点を含む)は内側として扱う。
    for i in 0..n {
        let a = polygon[i];
        let b = polygon[(i + 1) % n];
        if point_on_segment(a, b, p) {
            return true;
        }
    }
    let mut inside = false;
    for i in 0..n {
        let a = polygon[i];
        let b = polygon[(i + 1) % n];
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let t = (p[1] - a[1]) / (b[1] - a[1]);
            if p[0] < a[0] + t * (b[0] - a[0]) {
                inside = !inside;
            }
        }
    }
    inside
}

fn point_on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = d[0].hypot(d[1]);
    if len <= CONTAIN_TOL {
        return (p[0] - a[0]).hypot(p[1] - a[1]) <= CONTAIN_TOL;
    }
    let w = [p[0] - a[0], p[1] - a[1]];
    let cross = (d[0] * w[1] - d[1] * w[0]).abs() / len;
    if cross > CONTAIN_TOL {
        return false;
    }
    let t = (d[0] * w[0] + d[1] * w[1]) / (len * len);
    (-CONTAIN_TOL..=1.0 + CONTAIN_TOL).contains(&t)
}

/// 先端のまわりの紙が、軸からどれだけ横に広がっているか。
///
/// 先端の材料点のまわり [`FLAP_RADIUS`] の円周を [`FLAP_SAMPLES`] 方向で見て、
/// 紙の上にある点だけを姿勢へ移し、軸(胴 → 先端)からの横のずれの最大を返す。
/// 先端がとがるように折り込まれているほど小さくなる。
fn flap_half_width(placer: &Placer, material: [f64; 2], tip: [f64; 3], axis: [f64; 3]) -> f64 {
    let mut worst = 0.0_f64;
    for i in 0..FLAP_SAMPLES {
        let a = std::f64::consts::TAU * i as f64 / FLAP_SAMPLES as f64;
        let probe = [
            material[0] + FLAP_RADIUS * a.cos(),
            material[1] + FLAP_RADIUS * a.sin(),
        ];
        let Some(q) = placer.place(probe) else {
            continue; // 紙の外
        };
        let w = sub(q, tip);
        let along = w[0] * axis[0] + w[1] * axis[1] + w[2] * axis[2];
        let perp = sub(w, scale(axis, along));
        worst = worst.max(norm(perp));
    }
    worst
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
