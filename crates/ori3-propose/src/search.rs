//! 作業22「4つの目標で順位付けする決定的な探索」。
//!
//! ## 何をするか
//!
//! 作業21の [`FoldSession`] は「その状態から**実際に折れる手**」を返す。
//! ここではその手を1つずつ実際に折ってみて、折った後の形を作業20の
//! **4つの物差し**([`finish_gaps`])で測り、材料の層順が目標に指定されている場合は
//! その構造差も合わせて、**完成形へ近づく順**に試す。
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
use ori3_model::clock::{Duration, Instant};
use ori3_model::{CreasePattern, Document, FaceId, VertexId};

use crate::enumerate::{FoldSession, PoseScan, PreparedMove, SessionStateKey, VerifiedMove};
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
    /// 指定された完成形の材料上の層構造。指定がない既存の目標では4指標だけで測る。
    pub layer_target: Option<LayerTarget>,
}

/// 完成形に要求する、各平坦段階の材料上の下→上の層順。
///
/// 各点は面IDではなく、その面の原紙上の内部代表点で保存する。従って、CPの面IDや
/// 探索中の候補番号ではなく、紙のどの部分がどの順で重なったかを表す。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerTarget {
    stages: Vec<LayerStage>,
}

/// 量子化済みの材料点だけで表した1段階の層順。浮動小数点の丸め差をIDの差へ
/// すり替えないよう、座標は材料座標の `1e-9` 格子へ量子化する。
#[derive(Clone, Debug, PartialEq, Eq)]
struct LayerStage(Vec<[i64; 2]>);

const LAYER_TARGET_QUANTUM: f64 = 1e-9;

impl LayerTarget {
    /// 折り上がった文書が保存している各平坦段階の層順から、完成形の層目標を作る。
    ///
    /// 層順を保存していない手順だけの文書では `None` を返す。この場合に推測で
    /// 「何層なら正しい」と決めず、従来どおり4指標だけを使う。
    #[must_use]
    pub fn from_document(document: &Document) -> Option<Self> {
        let stages = document
            .sequence
            .iter()
            .filter_map(|step| {
                let order = step.layer_order.as_ref()?;
                (!order.is_empty() && order.iter().flatten().all(|value| value.is_finite())).then(
                    || {
                        LayerStage(
                            order
                                .iter()
                                .map(|point| {
                                    [
                                        quantize_layer_point(point[0]),
                                        quantize_layer_point(point[1]),
                                    ]
                                })
                                .collect(),
                        )
                    },
                )
            })
            .collect::<Vec<_>>();
        (!stages.is_empty()).then_some(Self { stages })
    }

    fn distance_to(&self, document: &Document) -> f64 {
        let actual = Self::from_document(document).map_or_else(Vec::new, |target| target.stages);
        let shared = longest_common_stage_subsequence(&self.stages, &actual);
        let missing = self.stages.len().saturating_sub(shared);
        let extra = actual.len().saturating_sub(shared);
        (missing + extra) as f64 / self.stages.len() as f64
    }
}

fn quantize_layer_point(value: f64) -> i64 {
    (value / LAYER_TARGET_QUANTUM).round() as i64
}

fn longest_common_stage_subsequence(left: &[LayerStage], right: &[LayerStage]) -> usize {
    let mut lengths = vec![0usize; right.len() + 1];
    for left_stage in left {
        let mut previous = 0usize;
        for (index, right_stage) in right.iter().enumerate() {
            let saved = lengths[index + 1];
            lengths[index + 1] = if left_stage == right_stage {
                previous + 1
            } else {
                lengths[index + 1].max(lengths[index])
            };
            previous = saved;
        }
    }
    lengths[right.len()]
}

impl FoldGoal {
    /// `document` が持つ材料上の層順を、この目標の追加の構造条件として付ける。
    ///
    /// 数・長さ・太さ・位置の4指標は変えず、層順が実際に指定されている場合だけ、
    /// その構造との差を総合点へ足す。
    #[must_use]
    pub fn with_layer_target_from(mut self, document: &Document) -> Self {
        self.layer_target = LayerTarget::from_document(document);
        self
    }

    /// 既存4指標とは別に、材料上の層構造が目標からどれだけ違うかを返す。
    ///
    /// 目標に層順が無い場合は、紙について根拠のない構造を仮定せず `0.0` とする。
    #[must_use]
    pub fn layer_gap(&self, document: &Document) -> f64 {
        self.layer_target
            .as_ref()
            .map_or(0.0, |target| target.distance_to(document))
    }

    /// 4指標の重み付き点数へ、明示された材料層構造の隔たりを単位重みで足す。
    ///
    /// 構造差は目標の保存層順に対する挿入・削除の比で、目標を全く共有しなければ
    /// `1.0`、途中段階を1つ共有すればその分だけ下がる。4指標の定義とその重みは
    /// 変更しない。
    #[must_use]
    pub fn score(&self, document: &Document, gaps: &FinishGaps, weights: GapWeights) -> f64 {
        weights.score(gaps) + self.layer_gap(document)
    }

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

/// 「完成した」と判断する、4つの物差しそれぞれの許容値。
///
/// 物差しの定義や値を変えるものではなく、作業20の [`FinishGaps`] を
/// **4項目とも**どこまで許すかを表す。総合点が小さくても1項目だけ大きい形を
/// 完成とはしない。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompletionTolerance {
    pub count: f64,
    pub length: f64,
    pub width: f64,
    pub position: f64,
}

impl CompletionTolerance {
    /// 作業24の終点実測を終えてから決めた既定値（作業25）。
    ///
    /// 実作品の完成形を独立に固定し、折り鶴・やっこさん・鳥の基本形について、
    /// 従来探索の返した手順を21姿勢/手で最後まで折って4値を測った。記録は
    /// 2026-08-28の正規再測定値（完成時一般制約違反0/101・破棄0、10回連続bit一致）である。
    ///
    /// 4許容は正規再測定後も変更していない。従来の決め方（実測または離散根拠の
    /// 約8割）より厳しい項目もあるが、基準を緩めないため、2026-08-28の利用者決定で
    /// 全て現行値に据え置いた。
    ///
    /// | 物差し | 作業24の記録／離散根拠 | 許容上限 | 根拠値に対する割合 | 余裕 |
    /// |---|---:|---:|---:|---:|
    /// | 角の数 | 3終点は`0.0`。最大12葉で1本欠ける最小非0値`1/12 = 0.0833333333` | `0.0666666667` | **80%** | **20%** |
    /// | 長さ | 鳥の基本形 `0.7071067812` | `0.5656854249` | **80%** | **20%** |
    /// | 太さ | やっこさん `0.4142135623730946` | `0.2485281374238571` | **60.000000000000085%** | **39.999999999999915%** |
    /// | 位置 | 折り鶴 `0.46687177208734676` | `0.3461183088254098` | **74.13562556544%** | **25.86437443456%** |
    ///
    /// 角数は整数の欠損数を葉数で割る離散値なので、計算機差のある小数を厳密比較
    /// していない。この上限なら対象範囲1〜12葉で1本欠けた形を完成扱いしない。
    /// 他3項目も [`Self::contains`] で有限性を確認してから正の上限と比べる。
    pub const DEFAULT: Self = Self {
        count: 0.066_666_666_666_666_67,
        length: 0.565_685_424_949_238_7,
        width: 0.248_528_137_423_857_1,
        position: 0.346_118_308_825_409_8,
    };

    /// 4項目がすべて有限で、それぞれの許容上限以内か。
    #[must_use]
    pub fn contains(self, gaps: &FinishGaps) -> bool {
        gaps.all_finite()
            && gaps.count <= self.count
            && gaps.length <= self.length
            && gaps.width <= self.width
            && gaps.position <= self.position
    }
}

impl Default for CompletionTolerance {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 探索結果を決める、決定的な打ち切り条件。
///
/// 状態数・深さ・分岐数と候補の走査順だけを持つ。壁時計watchdogと取消しは
/// [`SearchControl`] に分けてあり、この値を同じにすれば機械の速さにかかわらず
/// 同じ状態を同じ順で調べる。
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

/// 探索が異常に長く走り続けたときだけ中断する壁時計watchdog。
///
/// [`SearchBudget`] とは別型なので、watchdog到達を通常の探索結果や
/// [`SearchStop`] に混ぜることはできない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchWatchdog {
    /// 1回の探索に許す壁時計時間(ミリ秒)。
    pub max_millis: u64,
}

impl SearchWatchdog {
    /// 1回の探索に使ってよい壁時計時間の上限(ミリ秒)。
    ///
    /// これは既定のwatchdog安全弁である。状態数や分岐数を
    /// 個別に変える呼び出しでも、探索が無制限に走り続けないようにする。
    ///
    /// ## この値の決め方（実測、2026-08-22）
    ///
    /// 上限は、**正しく完成する探索を途中で切らない**ことが先に要る。切ってしまうと
    /// 受け入れ検査が `SearchAbort::WatchdogExpired` で赤くなり、CIも通らない
    /// （実際に600秒で起きた。
    /// `crate::enumerate` の候補の作り分けのコメントを参照）。
    /// そこで、いちばん重い正当な探索を測り、そこから余裕を取って決めた。
    ///
    /// | 測ったもの | 最適化あり | **最適化なし** |
    /// |---|---:|---:|
    /// | 折り鶴（既定12状態・分岐3、完成する） | 2.082秒 | **47.854秒** |
    /// | 鳥の基本形（同、12状態で打切り） | 0.772秒 | 18.951秒 |
    /// | やっこさん（同、1状態で完成） | 0.096秒 | 2.450秒 |
    ///
    /// CIの `cargo test --workspace` は**最適化なし**で走り、CIの計算機は手元より
    /// **約3.6倍遅い**実測がある（`CLAUDE.md` §10.6）。したがってCIで想定すべき
    /// いちばん重い1回は `47.854 × 3.6 = 172.3秒` である。
    /// このリポジトリの余裕の取り方（実測が上限のおよそ8割以内、`CLAUDE.md` §10.7.9）を
    /// 当てると `172.3 / 0.8 = 215.4秒` 以上が要る。人に読みやすい単位へ切り上げて
    /// **4分 = 240,000ms** とした。想定最大はこの **71.8%**、手元の最適化なしでは **19.9%**。
    ///
    /// **この値は [`SearchWatchdog::DEFAULT`] にだけ入る。** 検査用の固定標本
    /// (既定12状態)を異常扱いしないための値で、探索結果を決める予算ではない。
    ///
    /// ## 利用者の待ち時間との関係（2026-08-22に解決）
    ///
    /// この定数は検査用の固定標本にも同じように掛かるため、**利用者に許したい
    /// 待ち時間（数秒）をそのまま入れることはできなかった**。そこで時間上限を
    /// [`SearchWatchdog::max_millis`] という**呼び出しごとに変えられる項目**にした。
    /// 製品の `apps/desktop/src-tauri/src/commands.rs::PLAN_BUDGET`(2状態・2分岐、
    /// 検査の12状態よりずっと軽い)は、別のwatchdog型へ独自に **30,000ms** を入れる。
    /// 根拠(最適化ありの実測)は `commands.rs` のコメントと
    /// `scratchpad/search-budget-report.md` にある
    /// (`scratchpad/propose-search-subset-report.md` §16.7.5 の判断待ちに対する回答)。
    ///
    /// ## 2026-08-23に 240,000 → 600,000 へ上げた(実測)
    ///
    /// 花弁折り・つぶし折りの候補を既定で作るようにした
    /// (`crate::enumerate` の `WITH_EXTRA_CANDIDATES`)ため、1状態を広げる費用が上がった。
    ///
    /// | 標本 | 最適化あり(10回) | 最適化なし |
    /// |---|---|---|
    /// | 折り鶴 | 平均 92.939秒 / **最大 99.804秒** 完成 | **240秒で打ち切り、完成せず**(必要な10状態のうち5状態しか広げられない) |
    /// | 鳥の基本形 | 平均 4.958秒 完成 | 87.422秒 完成 |
    /// | やっこさん | 平均 0.204秒 完成 | 3.473秒 完成 |
    ///
    /// **決め方は前と同じ**(いちばん重い標本の実測 → CIは約3.6倍遅い → 8割余裕 → 切り上げ)。
    /// ただし**最適化ありの値を使う**。最適化なしでは折り鶴が完走せず、
    /// 対象の検査は最適化ありの `performance` ジョブで走らせるからである。
    ///
    /// - 最適化ありの折り鶴の最大 **99.804秒**
    /// - CI換算 `99.804 × 3.6 = 359.3秒`
    /// - 8割余裕 `359.3 ÷ 0.8 = 449.1秒`
    /// - 切り上げて **600,000ms(10分)**。CI換算の実測は上限の **59.9%** に収まる。
    ///
    /// **前の 240,000ms のままにできない理由**: CI換算の 359.3秒が 240秒を超えるので、
    /// **CIの `performance` ジョブで折り鶴が打ち切られる**(手元では95.5秒で通るので気づけない)。
    pub const MAX_MILLIS: u64 = 600_000;

    /// 恒久の既定watchdog。値は実測を根拠にした安全弁で、通常停止には使わない。
    pub const DEFAULT: Self = Self {
        max_millis: Self::MAX_MILLIS,
    };
}

impl Default for SearchWatchdog {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 探索取消し状態を読む窓口。
///
/// 状態の置き場所は所有しない。後続段階のjob registryが持つ単一snapshotをclosureで
/// 読めるため、取消しだけを別のatomicへ二重管理しない。
pub trait SearchCancellation: Send + Sync {
    /// 取消しが通知済みか。
    fn is_cancelled(&self) -> bool;
}

impl<F> SearchCancellation for F
where
    F: Fn() -> bool + Send + Sync,
{
    fn is_cancelled(&self) -> bool {
        self()
    }
}

/// 探索の実行だけを見張る制御。通常結果を決める値は持たない。
pub struct SearchControl<'a> {
    watchdog: SearchWatchdog,
    cancellation: &'a dyn SearchCancellation,
}

impl<'a> SearchControl<'a> {
    /// watchdogと、取消し状態を読む窓口を組み合わせる。
    #[must_use]
    pub fn new(watchdog: SearchWatchdog, cancellation: &'a dyn SearchCancellation) -> Self {
        Self {
            watchdog,
            cancellation,
        }
    }

    /// 設定したwatchdog。
    #[must_use]
    pub fn watchdog(&self) -> SearchWatchdog {
        self.watchdog
    }
}

/// 通常の探索結果を返さずに実行を中断した理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchAbort {
    /// 壁時計watchdogへ到達した。途中までの候補は返さない。
    WatchdogExpired,
    /// 呼び出し側から取消しが通知された。途中までの候補は返さない。
    Cancelled,
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
    /// | `max_states` | **12** | 部分集合候補追加後も、折り鶴は6状態、やっこさんは1状態で完成許容へ到達する。鳥の基本形は12状態で安全に打ち切る |
    /// | `branch` | **3** | 枝刈り前の候補最大は、折り鶴4・やっこさん9・鳥の基本形3。完成許容内の候補を順位の先頭へ置いた上で、保持する子を上位3件に絞る |
    /// | watchdog | **600,000**([`SearchWatchdog::MAX_MILLIS`]) | 最適化ありの折り鶴の最大99.804秒。CIは約3.6倍遅いので359.3秒を想定し、8割余裕の449.1秒を10分へ切り上げた(2026-08-23に240,000から変更)。**探索結果を決めるこのbudgetには含めない** |
    /// | `max_depth` | **8** | 完成した手数は折り鶴5・やっこさん1。既定予算内で両方を収める |
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
    /// (折り鶴が返す手順は、2026-08-17に候補の集め方を直してから `[3, 16]` に
    /// 変わっている。手3と手16はどちらも点 0.353553 で並び、折り線の番号が
    /// 小さいほうを先にするためで、2手折り終えた点はどちらも 0.000000 である。)
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
    /// 4つの物差しがすべて、探索に指定した [`CompletionTolerance`] 以内になった。
    GoalReached,
    /// 設定した枝刈りと重複排除の範囲で、広げる状態が尽きた。
    Exhausted,
    /// 状態の数の上限で打ち切った。
    StateCap,
    /// 深さの上限で打ち切った。
    DepthCap,
}

impl SearchStop {
    /// 候補の再現性検査でhashへ入れる、固定の通常停止tag。
    #[must_use]
    pub const fn contract_tag(self) -> &'static str {
        match self {
            Self::GoalReached => "goal_reached",
            Self::Exhausted => "exhausted",
            Self::StateCap => "state_cap",
            Self::DepthCap => "depth_cap",
        }
    }
}

/// 選んだ手1つぶんと、折った後の測り値。
#[derive(Clone, Debug, PartialEq)]
pub struct RankedMove {
    /// 折った手(作業21が確かめたもの)。
    pub mv: VerifiedMove,
    /// 折った後の4つの物差し。
    pub gaps: FinishGaps,
    /// 折った後の総合点。
    ///
    /// 4指標の重み付き点数に、目標が指定するときだけ材料の層構造差を足す。
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

/// 順位の同点を決める履歴。**折り線の番号も面の番号も1つも使わない。**
///
/// 1手ぶんが「動かした直線の端点と、その目標角」の集合である。端点は
/// [`HISTORY_LINE_TOL`] きざみ、角は100万分の1度きざみの整数へ丸めて持つ。
///
/// ## なぜ番号をやめたか
///
/// 前は `(手の番号, 閉じた線の番号, 閉鎖mask)` を使っていた。折り線の番号は
/// 端点の座標順で毎手付け直されるので、**同じ番号が別の線を指しうる**
/// (実測: 折り鶴の探索経路で折り線のまとまりが 34 → 33 → 32 → 30 → 31 → 28 → 28 と動く)。
/// 折り目が増える手(花弁折りなど)を扱えるようにしたことで、番号のずれはさらに
/// 大きくなった。番号どうしを比べる同点解消は、「どちらの状態を先に広げるか」を
/// 意味の無い値で決めることになり、番号の付き方が変わるだけで結果が揺れうる。
///
/// 辺のIDも使わない。折り目が増えるとき、既にある辺は**分割されて別のIDになる**ので、
/// 辺のIDも安定しないからである。動かした直線の位置と角だけが、
/// 展開図の作り直しに左右されない。
///
/// ## これで区別しきれない組はどうなるか
///
/// 同じ直線・同じ角でも対象の層が違う手(つぶし折りなど)は同じ値になりうる。
/// その場合は [`RankKey`] の最後の [`SessionStateKey`] が決める。あちらは
/// 展開図の中身・面の置き方・重なり順そのものを持つ**最終的な権威**なので、
/// ここで取りこぼしても順位が不定になることはない。
type HistoryKey = Vec<Vec<([i64; 2], [i64; 2], i64)>>;

/// 同じ折り線とみなす、履歴の端点の刻み。
///
/// 展開図の座標は長辺=1.0に正規化されており、折り線の端点は生成時に `1e-9` で
/// 既存頂点へ吸着される。`crate::plan` の同名の許容差と同じ考え方で、
/// 吸着の刻みより2桁ゆるい値にする。
const HISTORY_LINE_TOL: f64 = 1e-7;

/// 順位の最後に使う手番号列。部分集合は状態同一性だけに使い、既存契約どおり
/// 全手番号列を先に比較する。
type IdKey = Vec<usize>;

/// 探索の途中の1状態。
#[derive(Clone, Debug)]
struct Node {
    session: FoldSession,
    steps: Vec<RankedMove>,
    score: f64,
    gaps: FinishGaps,
    /// 形を直接改善しない層準備を、通常候補に飢餓させないための連続深さ。
    preparation_depth: usize,
}

/// 分岐上限3の中で、形を変える手・層を組み替える準備手・開き直しを競合させない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateClass {
    Regular,
    Directional,
    Reactivate,
    Reopen,
}

impl CandidateClass {
    const fn index(self) -> usize {
        match self {
            Self::Regular => 0,
            Self::Directional => 1,
            Self::Reactivate => 2,
            Self::Reopen => 3,
        }
    }

    const fn is_preparation(self) -> bool {
        !matches!(self, Self::Regular)
    }
}

/// 1つの状態から残す `branch` 件の内訳。
///
/// [`Self::regular`] と [`Self::preparation`] の合計は常に `branch - 1` である。
/// 残る1件は**粗順位の全体1位**で、種類を問わずに必ず確かめる
/// ([`next_candidate_index`] が最初に返す)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ClassQuotas {
    /// **形を変える手**([`CandidateClass::Regular`])へ必ず残す枠。
    regular: usize,
    /// **準備手**(形をまだ変えない手。方向付き・つぶし折り・花弁折り)へ残す枠。
    /// 種類は問わず、**粗順位の良いものから**入れる。
    preparation: usize,
}

/// 1状態から残す子の内訳を決める。
///
/// **規則(2026-08-22に変更)**: 予約枠 `branch - 1` のうち、
/// **準備手へ1枠**、**残りは形を変える手へ**。準備手の枠は種類で分けず、
/// 方向付き・つぶし折り・花弁折りが**同じ1枠を粗順位で取り合う**。
///
/// # なぜ規則を変えたか(実測)
///
/// 前の規則は、つぶし折り／花弁折りの候補が**1件でも挙がった瞬間**に
/// 「つぶし折り1枠・花弁折り1枠」を予約し、形を変える手の枠を
/// `branch - 3` にしていた。**`branch = 3`(既定)ではこれが 0 になる。**
/// 残るのは全体1位の1枠だけで、その1位が準備手だと、
/// **その状態からは形を変える手が1つも残らない**。
///
/// 実際に折り鶴が壊れた(最適化あり・既定12状態・分岐3、
/// `enumerate.rs` の `WITH_EXTRA_CANDIDATES` を `true` にしたとき)。
/// 前の規則では `StateCap`・**1手**(ID 16のみ)・長さ **1.142733** で未達だった。
///
/// 準備手を捨ててはいない。1枠は必ず残るので、つぶし折りも花弁折りも
/// 粗順位が良ければ入る。次の状態ではもう片方が1位になり得る。
/// **上限(状態12・分岐3・深さ8)は1つも上げていない。**
fn candidate_class_quotas(branch: usize) -> ClassQuotas {
    let reserved = branch.saturating_sub(1);
    let preparation = reserved.min(1);
    ClassQuotas {
        regular: reserved - preparation,
        preparation,
    }
}

/// 形を変える通常の状態を何件広げるごとに、層準備の状態へ1件ぶんの順番を回すか。
///
/// # なぜ「後回し」だけでは足りなかったか(2026-08-23の実測)
///
/// 2026-08-22に「通常の状態を必ず先に広げる」規則を入れた。そのときの記録には
/// 「鳥の基本形は4指標とも変わらなかった」とあるが、**当時は別の欠陥
/// ([`crate::enumerate`] の `PART_LAYER_SKIP_MARK`)が、鳥を完成させる花弁折りを
/// 候補の段階で落としていた**ため、差が出ようがなかった。
///
/// その欠陥を直したうえで測り直すと、鳥の基本形では次のことが起きていた。
///
/// - 予備基本形の状態(手順 `[2, 7]` の後)で、鳥を完成させる花弁折りは
///   **粗い順位で1位**(許容超過 4.2858。2位も花弁折り、5位以下は 6.2192)。
///   **枝刈りでは落ちていない。**
/// - しかしその子は準備状態なので、**通常の状態が尽きるまで一度も広げられない**。
///   鳥では通常の状態が尽きず、状態上限12に達して打ち切られていた。
///   **2手目の花弁折りが生成すらされない。**
/// - 分岐上限を 3 → 6 → 10 と広げても、`length` は `0.7071067811865483` から
///   **16桁すべて動かなかった**。分岐の問題ではない。
///
/// # なぜ「全部やめる」ではなく「間隔」なのか
///
/// 後回しを丸ごとやめると、鳥は予備基本形から2状態で `GoalReached`(4指標とも0)に
/// なるが、**根から探すと 240秒(時間上限)に当たった**。2026-08-22の記録どおり、
/// 折り鶴も1状態ずつの交互では完成を失う。
/// そこで**上限(状態12・分岐3・深さ8)は1つも上げず**、
/// 通常の状態を `PREPARATION_TURN` 件広げるごとに準備状態へ1件ぶんの順番を回す。
///
/// # 値の決め方(2026-08-23の掃引。最適化あり、`WITH_EXTRA_CANDIDATES = true`、
/// 既定の上限、1標本1回)
///
/// | 間隔 | 折り鶴 | やっこさん | 鳥の基本形 |
/// |---:|---|---|---|
/// | 1(＝後回しをやめる) | TimeCap 240.0秒 長さ0.4178 **未完成** | 完成 0.2秒 | StateCap 229.1秒 長さ0.2929 **未完成** |
/// | 2(＝1状態ずつ交互) | TimeCap 240.1秒 長さ0.6990 **未完成** | 完成 1.3秒 | StateCap 21.7秒 長さ0.5000 **未完成** |
/// | 3 | TimeCap 240.0秒 長さ0.7586 **未完成** | 完成 0.2秒 | **完成** 8.3秒 長さ0.3536 |
/// | **4** | **完成** 207.2秒 長さ0.3591 | 完成 0.2秒 | **完成** 5.4秒 長さ0.3536 |
/// | 5 | 完成 74.3秒 長さ0.3591 | 完成 0.7秒 | StateCap 103.1秒 長さ0.7071 **未完成** |
/// | 6 | 完成 67.4秒 | 完成 0.6秒 | StateCap 91.0秒 **未完成** |
/// | 8 | 完成 23.1秒 | 完成 0.2秒 | StateCap 27.3秒 **未完成** |
/// | 12 | 完成 27.7秒 | 完成 0.2秒 | StateCap 31.9秒 **未完成** |
///
/// **掃引した8つの値のうち、3標本すべてが完成するのは 4 だけだった。**
/// 小さすぎると準備手が通常の手を押しのけて折り鶴が完成せず、
/// 大きすぎると鳥の花弁折りが状態上限12までに順番をもらえない。
///
/// **この値は、いまの候補の作り方と上限(状態12・分岐3・深さ8)に合わせた実測値である。**
/// 候補の作り方・順位の付け方・上限のどれかを変えたら、**この表を測り直すこと**。
/// 幅が狭いので、「動いているから触らない」では済まない。
const PREPARATION_TURN: usize = 4;

/// 形を変える通常の状態を先に広げ、形を変えない層準備の状態は後回しにする。
/// ただし [`PREPARATION_TURN`] 回に1回だけ、準備状態へ順番を回す。
///
/// 返す最善手の順位は [`rank_key`] のまま変えない。ここで決めるのは
/// 「状態上限12のうち、どの状態に手を広げるか」だけである。
/// どちらの側も、選ぶのは `frontier` の並び順(＝ [`rank_key`])で先頭のものなので、
/// 同じ入力なら毎回同じ状態を選ぶ。
///
/// **実測(2026-08-22、最適化あり、既定12状態・分岐3)**: 準備状態と通常状態を
/// 1状態ずつ**交互に**広げていたとき、折り鶴は12状態を使い切っても完成せず
/// (`StateCap`、長さ0.750927・太さ0.471180・位置0.325450)、完成には**13状態**を
/// 要した。交互を止めて通常状態を先に広げると、同じ12状態の上限で**6状態**で
/// `GoalReached` になり、長さ0.359121・太さ0.172464・位置0.184029 になる。
///
/// **実測(2026-08-23、最適化あり、同じ上限、`WITH_EXTRA_CANDIDATES = true`)**:
/// 後回しを丸ごとやめる(＝間隔1。毎回いちばん良い状態を広げる)と、鳥の基本形は
/// 予備基本形から**2状態で `GoalReached`**(4指標とも `0.000000`)になる一方、
/// **根から探すと240秒の時間上限**に当たり、折り鶴も完成しなくなった。
/// 間隔ごとの実測は [`PREPARATION_TURN`] の表にある。
///
/// 準備状態を捨ててはいない。通常状態が尽きれば、同じ順位で準備状態を広げる。
fn pop_frontier(
    frontier: &mut BTreeMap<RankKey, Node>,
    expanded: usize,
) -> Option<(RankKey, Node)> {
    // `expanded` はここまでに手を広げ終えた状態の数。0件目(根)は必ず通常側から取る。
    let take_preparation = expanded > 0 && expanded.is_multiple_of(PREPARATION_TURN);
    let pick = |preparation: bool| -> Option<RankKey> {
        frontier.iter().find_map(|(key, node)| {
            ((node.preparation_depth > 0) == preparation).then(|| key.clone())
        })
    };
    // 欲しい側が空なら、もう片方から取る(順番を空回りさせない)。
    let key = pick(take_preparation).or_else(|| pick(!take_preparation))?;
    let node = frontier
        .remove(&key)
        .expect("selected frontier key disappeared");
    Some((key, node))
}

fn next_candidate_index(
    classes: &[CandidateClass],
    attempted: &BTreeSet<usize>,
    kept: [usize; 4],
    quotas: ClassQuotas,
) -> Option<usize> {
    if classes.is_empty() {
        return None;
    }
    if attempted.is_empty() {
        // 粗順位の全体1位は、候補種別の予約枠に関係なく必ず21姿勢で確かめる。
        return Some(0);
    }
    let kept_regular = kept[CandidateClass::Regular.index()];
    let kept_preparation = kept.iter().sum::<usize>() - kept_regular;
    classes
        .iter()
        .enumerate()
        .find(|(index, class)| {
            !attempted.contains(index)
                && if class.is_preparation() {
                    // 準備手は種類で分けず、1枠を粗順位で取り合う。
                    kept_preparation < quotas.preparation
                } else {
                    kept_regular < quotas.regular
                }
        })
        .or_else(|| {
            classes
                .iter()
                .enumerate()
                .find(|(index, _)| !attempted.contains(index))
        })
        .map(|(index, _)| index)
}

/// 順位を決める鍵。
///
/// 通常探索では「総合点 → 手数 → 番号列」。完成探索では「許容値からの超過 →
/// 総合点 → 手数 → 番号列」の順にする。小数はすべて刻みへ丸める。
type RankKey = (i64, i64, usize, IdKey, HistoryKey, SessionStateKey);

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

fn history_key(node: &Node) -> HistoryKey {
    let quantize_coordinate = |value: f64| -> i64 {
        if !value.is_finite() {
            return if value.is_sign_negative() {
                i64::MIN
            } else {
                i64::MAX
            };
        }
        let scaled = (value / HISTORY_LINE_TOL).round();
        if scaled >= i64::MAX as f64 {
            i64::MAX
        } else if scaled <= i64::MIN as f64 {
            i64::MIN
        } else {
            scaled as i64
        }
    };
    let quantize_point = |point: [f64; 2]| point.map(quantize_coordinate);
    let sequence = &node.session.document().sequence;
    // 探索で進めた手だけを見る。渡された作品が既に何手か折られていることがあるので、
    // 手順の末尾から `node.steps` の数だけを取る。
    let skip = sequence.len().saturating_sub(node.steps.len());
    sequence[skip..]
        .iter()
        .map(|step| {
            let mut driven = BTreeSet::new();
            for driver in &step.drivers {
                if !driver.target_angle_deg.is_finite() {
                    continue;
                }
                // 同じ直線を逆向きに書いた手を別物にしない。
                let (mut a, mut b) = (quantize_point(driver.a), quantize_point(driver.b));
                if b < a {
                    std::mem::swap(&mut a, &mut b);
                }
                let angle = (driver.target_angle_deg * 1_000_000.0).round() as i64;
                driven.insert((a, b, angle));
            }
            driven.into_iter().collect()
        })
        .collect()
}

fn id_key(node: &Node) -> IdKey {
    node.steps.iter().map(|step| step.mv.id).collect()
}

/// 4項目が許容値をどれだけ超えたか。0なら4/4を満たす。
///
/// 許容値0の項目（角数は欠損0本を要求できる）は [`SCORE_QUANTUM`] を目盛りに使い、
/// 0除算を避ける。これは物差しや許容値を変えず、完成探索の順番だけを決める値である。
fn completion_excess(gaps: &FinishGaps, tolerance: CompletionTolerance) -> f64 {
    let excess = |gap: f64, limit: f64| {
        if !gap.is_finite() || !limit.is_finite() || limit < 0.0 {
            f64::INFINITY
        } else {
            (gap - limit).max(0.0) / limit.max(SCORE_QUANTUM)
        }
    };
    excess(gaps.count, tolerance.count)
        + excess(gaps.length, tolerance.length)
        + excess(gaps.width, tolerance.width)
        + excess(gaps.position, tolerance.position)
}

fn rank_key(node: &Node, completion: Option<CompletionTolerance>) -> RankKey {
    let (primary, secondary) = completion.map_or_else(
        || (quantize(node.score), 0),
        |tolerance| {
            (
                quantize(completion_excess(&node.gaps, tolerance)),
                quantize(node.score),
            )
        },
    );
    (
        primary,
        secondary,
        node.steps.len(),
        id_key(node),
        history_key(node),
        node.session.state_key(),
    )
}

/// 完成形へ近づく順に手を選んで探索する。
///
/// ## 何を返すか
///
/// **たどった状態のうち、4つの物差しと、指定がある場合の材料層構造を合わせた
/// 総合点がいちばん良かったところまでの手順**を返す。
/// 目標は「完成形の形」であって「折り目を全部折ること」ではないので、
/// 途中で目標にいちばん近くなるならそこで返す。打ち切りに達した場合も、
/// **そこまでで見つけたいちばん良い手順**が同じ規則で返る。
///
/// 通常の単純本折りで改善する手が無い場合だけ、残す側と上/下を明示した
/// fold-throughと方向付きflat poseを追加して同じ予算で再探索する。この再探索は
/// 4項目の許容超過を先に比べるので、形をまだ変えない層準備手も次の候補へつなげられる。
/// それでも改善が無い展開図では、**手順0件**(折らないのが最良)が返る。これは探索失敗では
/// ないが、完成を意味するとは限らない。
///
/// ## どこで4つの物差しと層構造が効くか
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
    let execution = SearchExecution::Deterministic;
    let ordinary = search(
        session,
        goal,
        weights,
        budget,
        None,
        SearchCandidateSet::Ordinary,
        &execution,
    )
    .expect("決定的探索にはwatchdogも取消しも無い");
    if !ordinary.steps.is_empty() {
        return ordinary;
    }
    search(
        session,
        goal,
        weights,
        budget,
        Some(CompletionTolerance::DEFAULT),
        SearchCandidateSet::DirectionalFallback,
        &execution,
    )
    .expect("決定的探索にはwatchdogも取消しも無い")
}

/// [`search_to_finish`] をwatchdogと取消しで見張る。
///
/// 中断時は途中までの [`SearchOutcome`] を返さず、専用の [`SearchAbort`] だけを返す。
/// したがって壁時計や取消しが通常の [`SearchStop`] を変えることはない。
pub fn search_to_finish_with_control(
    session: &FoldSession,
    goal: &FoldGoal,
    weights: GapWeights,
    budget: SearchBudget,
    control: &SearchControl<'_>,
) -> Result<SearchOutcome, SearchAbort> {
    let execution = SearchExecution::controlled(control);
    let ordinary = search(
        session,
        goal,
        weights,
        budget,
        None,
        SearchCandidateSet::Ordinary,
        &execution,
    )?;
    if !ordinary.steps.is_empty() {
        return Ok(ordinary);
    }
    search(
        session,
        goal,
        weights,
        budget,
        Some(CompletionTolerance::DEFAULT),
        SearchCandidateSet::DirectionalFallback,
        &execution,
    )
}

/// 4つの物差しがすべて指定の許容値以内になるまで探索する。
///
/// 許容内の候補を枝刈り前から最優先し、未完成の候補も4項目の許容超過が小さい順に
/// 広げる。これにより、総合点では4位以下でも4/4を満たす手を捨てない。見つからなければ
/// 打ち切りまでで許容値に最も近い状態を返す。同点の決め方は [`search_to_finish`] と同じ。
#[must_use]
pub fn search_to_completion(
    session: &FoldSession,
    goal: &FoldGoal,
    weights: GapWeights,
    budget: SearchBudget,
    tolerance: CompletionTolerance,
) -> SearchOutcome {
    search(
        session,
        goal,
        weights,
        budget,
        Some(tolerance),
        SearchCandidateSet::Completion,
        &SearchExecution::Deterministic,
    )
    .expect("決定的探索にはwatchdogも取消しも無い")
}

/// [`search_to_completion`] をwatchdogと取消しで見張る。
///
/// `Ok`だけが通常の探索結果である。`Err`には途中結果を持たせないため、呼び出し側が
/// watchdog/cancelを [`crate::VerifiedPlan::Partial`] へ変換する経路を作れない。
pub fn search_to_completion_with_control(
    session: &FoldSession,
    goal: &FoldGoal,
    weights: GapWeights,
    budget: SearchBudget,
    tolerance: CompletionTolerance,
    control: &SearchControl<'_>,
) -> Result<SearchOutcome, SearchAbort> {
    search(
        session,
        goal,
        weights,
        budget,
        Some(tolerance),
        SearchCandidateSet::Completion,
        &SearchExecution::controlled(control),
    )
}

/// 決定的探索には時計を持たせず、見張る呼び出しだけが開始時刻を持つ。
enum SearchExecution<'a> {
    Deterministic,
    Controlled {
        watchdog: SearchWatchdog,
        cancellation: &'a dyn SearchCancellation,
        started: Instant,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchCandidateSet {
    Ordinary,
    DirectionalFallback,
    Completion,
}

impl<'a> SearchExecution<'a> {
    fn controlled(control: &SearchControl<'a>) -> Self {
        Self::Controlled {
            watchdog: control.watchdog,
            cancellation: control.cancellation,
            started: Instant::now(),
        }
    }

    fn interruption(&self) -> Option<SearchAbort> {
        let Self::Controlled {
            watchdog,
            cancellation,
            started,
        } = self
        else {
            return None;
        };
        if cancellation.is_cancelled() {
            Some(SearchAbort::Cancelled)
        } else if started.elapsed() >= Duration::from_millis(watchdog.max_millis) {
            Some(SearchAbort::WatchdogExpired)
        } else {
            None
        }
    }

    fn check(&self) -> Result<(), SearchAbort> {
        self.interruption().map_or(Ok(()), Err)
    }
}

fn search(
    session: &FoldSession,
    goal: &FoldGoal,
    weights: GapWeights,
    budget: SearchBudget,
    completion: Option<CompletionTolerance>,
    candidates: SearchCandidateSet,
    execution: &SearchExecution<'_>,
) -> Result<SearchOutcome, SearchAbort> {
    let start_form = goal.measure(session.document());
    let start_gaps = finish_gaps(&goal.target, &start_form);
    let start_score = goal.score(session.document(), &start_gaps, weights);

    let root = Node {
        session: session.clone(),
        steps: Vec::new(),
        score: start_score,
        gaps: start_gaps,
        preparation_depth: 0,
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
    execution.check()?;
    if completion.is_some_and(|tolerance| tolerance.contains(&start_gaps)) {
        outcome.stop = SearchStop::GoalReached;
        return Ok(outcome);
    }
    // たどった中でいちばん点数の良かった状態。打ち切りに達してもこれを返す。
    let mut best: (RankKey, Node) = (rank_key(&root, completion), root.clone());
    let mut capped = false;
    let mut goal_reached = false;

    let mut seen = BTreeSet::from([root.session.state_key()]);
    let mut frontier: BTreeMap<RankKey, Node> =
        BTreeMap::from([(rank_key(&root, completion), root)]);

    while let Some((_, node)) = pop_frontier(&mut frontier, outcome.states_expanded) {
        execution.check()?;
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

        let (children, candidates) = expand(
            &node, goal, weights, budget, completion, candidates, execution, &seen,
        )?;
        outcome.max_branching = outcome.max_branching.max(candidates);
        let mut completed: Option<(RankKey, Node)> = None;
        for child in children {
            if !seen.insert(child.session.state_key()) {
                continue;
            }
            outcome.states_generated += 1;
            let key = rank_key(&child, completion);
            if key < best.0 {
                best = (key.clone(), child.clone());
            }
            if completion.is_some_and(|tolerance| tolerance.contains(&child.gaps))
                && completed.as_ref().is_none_or(|(done, _)| key < *done)
            {
                completed = Some((key.clone(), child.clone()));
            }
            frontier.insert(key, child);
        }
        execution.check()?;
        if let Some(done) = completed {
            // 4項目すべてを満たすことを、総合点の改善より優先する。
            // 同じ展開で複数見つかった場合は既存の決定的な順位で1つに決める。
            best = done;
            outcome.stop = SearchStop::GoalReached;
            goal_reached = true;
            break;
        }
    }
    if !capped && !goal_reached {
        outcome.stop = SearchStop::Exhausted;
    }

    outcome.steps = best.1.steps;
    outcome.best_gaps = best.1.gaps;
    outcome.best_score = best.1.score;
    Ok(outcome)
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
///
/// # 候補は、いまの展開図の折り線を**すべて**試して集める
///
/// 1で候補を集めるのに [`FoldSession::verified_moves`] は使わない。あれの
/// [`crate::enumerate::MoveReport::verified`] も見積もりの内外を問わず折れた手をすべて
/// 返すが、手動操作が明示する単純本折りの方向を物理検査の根拠にする。
/// 提案は入力CPのM/Vを根拠に [`FoldSession::verify_move`] で確かめる。
///
/// 作業18の見積もり([`crate::plan_generic::GenericPlanner`])は会計にだけ残す。
/// 見積もり内だけに絞ってはならない。この見積もりは
/// **上限とも下限とも言えない**ことが作業21で実測されている
/// (検査 `the_estimate_from_task_18_is_neither_an_upper_nor_a_lower_bound`)。
/// 見積もりで絞ると、**実際に折れる手を探索が見られなくなる**。
///
/// 実測(2026-08-17、debugビルド): 折り鶴の `y = 0.5`(紙を端から端まで横切る
/// 折り目、手3)は姿勢3点でも21点でも折れるのに、見積もりの外にある。
/// 手16を折った後もこれは変わらないので、絞っていたときの探索は
/// **2手目の候補が0件**になり、`[16]` の1手で止まっていた
/// (点 0.353553。`scratchpad/search-fail-report.md`)。
///
/// 折る手間は増えない。[`FoldSession::verified_moves`] も、もともと
/// **すべての折り線を1本ずつ実際に折って**確かめており、見積もりは
/// 結果を振り分けるのに使われているだけだからである。
/// ここでは同じ確認を [`FoldSession::verify_move`] で1本ずつ行い、
/// 見積もりによる振り分けをしない。
///
/// # 折れることと、提案の層順authorityは別に確かめる
///
/// operation-awareな単純本折りは、手が明示する山谷を根拠に途中姿勢まで物理検査できる。
/// しかし、その終点の層順を提案へ採用してよいかは別の契約である。適用後CPで検査すると、
/// 手自身が整えた山谷が候補順を自己認証するため、ここでは必ず適用前CPの一般制約へ照合する。
fn expand(
    node: &Node,
    goal: &FoldGoal,
    weights: GapWeights,
    budget: SearchBudget,
    completion: Option<CompletionTolerance>,
    candidates: SearchCandidateSet,
    execution: &SearchExecution<'_>,
    seen: &BTreeSet<SessionStateKey>,
) -> Result<(Vec<Node>, usize), SearchAbort> {
    let mut ranked: Vec<(Option<PreparedMove>, FinishGaps, f64, CandidateClass)> = Vec::new();
    let mut safe_single_lines = BTreeSet::new();
    for fold_line in node.session.fold_lines() {
        execution.check()?;
        let Some(prepared) = node.session.prepare_move(fold_line.id, budget.rank_scan) else {
            execution.check()?;
            continue; // もう折り終えている手か、粗く見ても折れない手。止めずに次の手へ。
        };
        let gaps = finish_gaps(&goal.target, &goal.measure(prepared.successor().document()));
        let score = goal.score(prepared.successor().document(), &gaps, weights);
        safe_single_lines.insert(fold_line.id);
        ranked.push((Some(prepared), gaps, score, CandidateClass::Regular));
        execution.check()?;
    }
    if candidates == SearchCandidateSet::DirectionalFallback {
        let mut callback_abort = None;
        let (directional_moves, interrupted) =
            node.session
                .prepared_directional_moves_until(budget.rank_scan, || {
                    if callback_abort.is_none() {
                        callback_abort = execution.interruption();
                    }
                    callback_abort.is_some()
                });
        if let Some(abort) = callback_abort {
            return Err(abort);
        }
        debug_assert!(!interrupted, "中断理由を保存せず方向付き候補を打ち切った");
        for prepared in directional_moves {
            execution.check()?;
            let gaps = finish_gaps(&goal.target, &goal.measure(prepared.successor().document()));
            let score = goal.score(prepared.successor().document(), &gaps, weights);
            ranked.push((Some(prepared), gaps, score, CandidateClass::Directional));
            execution.check()?;
        }
    } else if candidates == SearchCandidateSet::Completion {
        // 単一直線を順に閉じると行き止まる花弁折り等のため、完成探索だけは
        // 全網と、畳んだ平面で同一直線へ重なる局所部分集合も同じ物差しで順位付けする。
        // 通常の `search_to_finish` には足さず、作業22の既存結果を変えない。
        let mut callback_abort = None;
        let (network_moves, interrupted) = node.session.prepared_completion_moves_until(
            budget.rank_scan,
            &safe_single_lines,
            || {
                if callback_abort.is_none() {
                    callback_abort = execution.interruption();
                }
                callback_abort.is_some()
            },
        );
        if let Some(abort) = callback_abort {
            return Err(abort);
        }
        debug_assert!(!interrupted, "中断理由を保存せず網候補を打ち切った");
        for prepared in network_moves {
            execution.check()?;
            let gaps = finish_gaps(&goal.target, &goal.measure(prepared.successor().document()));
            let score = goal.score(prepared.successor().document(), &gaps, weights);
            let edge_changes = node.session.transition_edge_changes(prepared.successor());
            let id = prepared.verified().id;
            let class = if node.session.move_is_directional_fold(id) {
                CandidateClass::Directional
            } else {
                match edge_changes {
                    (true, true) => CandidateClass::Reopen,
                    (false, false) if node.session.move_reactivates_layer_packet(id) => {
                        CandidateClass::Reactivate
                    }
                    _ => CandidateClass::Regular,
                }
            };
            ranked.push((Some(prepared), gaps, score, class));
            execution.check()?;
        }
    }
    ranked.sort_by(|a, b| {
        let completion_key = |gaps: &FinishGaps| {
            completion.map_or(0, |tolerance| quantize(completion_excess(gaps, tolerance)))
        };
        completion_key(&a.1)
            .cmp(&completion_key(&b.1))
            .then_with(|| quantize(a.2).cmp(&quantize(b.2)))
            .then_with(|| {
                a.0.as_ref()
                    .expect("順位付け前の候補が消費された")
                    .verified()
                    .id
                    .cmp(
                        &b.0.as_ref()
                            .expect("順位付け前の候補が消費された")
                            .verified()
                            .id,
                    )
            })
    });
    let candidates = ranked.len();
    execution.check()?;

    // 幾何点数だけのbeamでは、形をまだ変えない「層を持ち替える準備手」が常に
    // 単線候補の後ろへ落ち、次の花弁折りへ到達できない。分岐上限は増やさず、
    // 「全体最良1・準備手1・残りは形を変える手」に層化する
    // (内訳と、規則を変えた理由は candidate_class_quotas を見ること)。
    // 粗検査だけ通って21点検査で落ちた候補や既訪問状態は枠へ数えず、次候補で補充する。
    // 返す最善の順位は変えず、状態上限の配分だけを公平にする。
    let quotas = candidate_class_quotas(budget.branch);
    let mut children: Vec<Node> = Vec::new();
    let mut attempted = BTreeSet::new();
    let mut child_states = BTreeSet::new();
    let mut kept = [0usize; 4];
    let classes = ranked
        .iter()
        .map(|(_, _, _, class)| *class)
        .collect::<Vec<_>>();
    while attempted.len() < ranked.len() && children.len() < budget.branch {
        let Some(index) = next_candidate_index(&classes, &attempted, kept, quotas) else {
            break;
        };
        attempted.insert(index);
        let prepared = ranked[index]
            .0
            .take()
            .expect("順位付け済み候補は1回だけ細走査する");
        execution.check()?;
        let fine = match node.session.reverify_prepared_move(prepared, budget.scan) {
            Ok(fine) => fine,
            Err(_) => {
                execution.check()?;
                continue; // 粗く見たときは折れたが、細かく見ると折れない手。捨てる。
            }
        };
        let (mv, next) = fine.into_parts();
        let child_state = next.state_key();
        if seen.contains(&child_state) || !child_states.insert(child_state) {
            // 同じ物理状態へ戻るcycleや、別IDから同じ終点へ来る兄弟で分岐枠を使わない。
            continue;
        }
        // 結果へ載せる4値は、実際に子状態として保持する細かい確認後の文書から改めて測る。
        let gaps = finish_gaps(&goal.target, &goal.measure(next.document()));
        let score = goal.score(next.document(), &gaps, weights);
        let mut steps = node.steps.clone();
        steps.push(RankedMove { mv, gaps, score });
        children.push(Node {
            session: next,
            steps,
            score,
            gaps,
            preparation_depth: if ranked[index].3.is_preparation() {
                node.preparation_depth + 1
            } else {
                0
            },
        });
        kept[ranked[index].3.index()] += 1;
        execution.check()?;
    }
    Ok((children, candidates))
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
        if out.is_empty() {
            None
        } else {
            Some(Self { faces: out })
        }
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

#[cfg(test)]
mod tests {
    use ori3_model::{Document, Paper};

    use super::*;

    /// 分岐の枠は「全体1位1・準備手1・残りは形を変える手」で、上限は上げない。
    ///
    /// **前の規則を置き換えた検査である**(落ちないように緩めたのではない)。
    /// 前は「つぶし折り1枠・花弁折り1枠」を種類ごとに予約しており、
    /// 技法候補が1件でも挙がると、既定の `branch = 3` では
    /// **形を変える手の枠が `3 - 3 = 0` になっていた**。
    /// そのため折り鶴が1手目から先へ進めなくなった(実測: `StateCap`・1手・
    /// 長さ 1.142733・未達)。ここで主張するのは、**どの分岐数でも
    /// 形を変える手の枠が消えないこと**である。
    #[test]
    fn every_branch_size_keeps_room_for_a_move_that_changes_the_shape() {
        // 既定の分岐3: 全体1位1 + 形を変える手1 + 準備手1。
        assert_eq!(
            candidate_class_quotas(3),
            ClassQuotas {
                regular: 1,
                preparation: 1
            }
        );
        for branch in 0..=8 {
            let quotas = candidate_class_quotas(branch);
            assert_eq!(
                quotas.regular + quotas.preparation,
                branch.saturating_sub(1),
                "予約枠の合計は分岐数-1(全体1位の1枠を差し引く)。上限は上げない"
            );
            assert!(
                quotas.preparation <= 1,
                "準備手は種類を問わず1枠まで(分岐{branch})"
            );
            if branch >= 3 {
                assert!(
                    quotas.regular >= 1,
                    "分岐{branch}で、形を変える手の枠が消えてはいけない"
                );
            }
        }
    }

    /// 準備手の1枠は、種類ではなく**粗順位**で取り合う。
    ///
    /// つぶし折りと花弁折りに別々の枠を与えると、既定の分岐3では
    /// 形を変える手の枠が無くなる。ここでは、準備手を1つ残した後は
    /// 別の種類の準備手であっても予約枠では選ばれないことを固定する。
    /// 角度も解も通さない、枠の配り方だけの検査である。
    #[test]
    fn the_single_preparation_slot_is_shared_by_every_preparation_kind() {
        use CandidateClass::{Directional, Reactivate, Regular, Reopen};
        let quotas = candidate_class_quotas(3);
        let classes = [Regular, Reactivate, Reopen, Directional, Regular];

        // 全体1位(index 0、形を変える手)は種類を問わず必ず確かめる。
        let mut attempted = BTreeSet::new();
        let mut kept = [0usize; 4];
        assert_eq!(
            next_candidate_index(&classes, &attempted, kept, quotas),
            Some(0)
        );
        attempted.insert(0);
        kept[Regular.index()] += 1;

        // 形を変える手の枠は1で、いま埋まった。次は準備手の枠から選ぶ。
        assert_eq!(
            next_candidate_index(&classes, &attempted, kept, quotas),
            Some(1),
            "準備手の枠には、粗順位のいちばん良い準備手が入る"
        );
        attempted.insert(1);
        kept[Reactivate.index()] += 1;

        // 準備手の枠は使い切った。別の種類(花弁折り)でも予約では選ばれず、
        // 予約を使い切った後は粗順位の順に補充する。
        assert_eq!(
            next_candidate_index(&classes, &attempted, kept, quotas),
            Some(2),
            "予約枠が尽きたら、種類を問わず粗順位の順に補充する"
        );
    }

    fn node_with_moves(session: &FoldSession, moves: &[(usize, Vec<usize>)]) -> Node {
        let steps = moves
            .iter()
            .map(|(id, closes)| RankedMove {
                mv: VerifiedMove {
                    id: *id,
                    line: [[0.0, 0.0], [1.0, 0.0]],
                    closes: closes.clone(),
                    mask: 0,
                    max_seam_gap: 0.0,
                    penetrations: 0,
                    poses_checked: 1,
                },
                gaps: FinishGaps::BEST,
                score: 0.0,
            })
            .collect();
        Node {
            session: session.clone(),
            steps,
            score: 0.0,
            gaps: FinishGaps::BEST,
            preparation_depth: 0,
        }
    }

    /// 「全体順位では準備側が先」という最悪の並びを作る。
    fn two_state_frontier(session: &FoldSession) -> (BTreeMap<RankKey, Node>, RankKey, RankKey) {
        let mut preparation_node = node_with_moves(session, &[(1, vec![1])]);
        preparation_node.preparation_depth = 2;
        let regular_node = node_with_moves(session, &[(2, vec![2])]);
        let preparation_key = rank_key(&preparation_node, None);
        let regular_key = rank_key(&regular_node, None);
        assert!(
            preparation_key < regular_key,
            "準備側を全体順位では先にしておく"
        );
        let frontier = BTreeMap::from([
            (preparation_key.clone(), preparation_node),
            (regular_key.clone(), regular_node),
        ]);
        (frontier, preparation_key, regular_key)
    }

    fn flat_square_session() -> FoldSession {
        let document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        FoldSession::new(&document).expect("平らな正方形を読み込めない")
    }

    /// 形を変える通常状態を先に広げ、準備状態は捨てずに後回しにする。
    ///
    /// 1状態ずつ交互に広げていたときは折り鶴が12状態で完成しなくなった
    /// (実測は [`pop_frontier`] のコメント)。
    /// [`PREPARATION_TURN`] の倍数**でない**回では、全体順位が下でも通常側を先に広げる。
    #[test]
    fn regular_states_are_expanded_before_preparation_states() {
        let session = flat_square_session();
        for expanded in 0..PREPARATION_TURN {
            let (mut frontier, preparation_key, regular_key) = two_state_frontier(&session);
            let (selected, _) = pop_frontier(&mut frontier, expanded).expect("通常状態を選べない");
            assert_eq!(
                selected, regular_key,
                "{expanded}件目で、全体順位が下でも形を変える状態を先に広げる"
            );
            let (next, _) = pop_frontier(&mut frontier, expanded + 1).expect("準備状態を選べない");
            assert_eq!(
                next, preparation_key,
                "通常状態が尽きたら準備状態を広げる(捨てていない)"
            );
            assert!(frontier.is_empty());
            assert!(pop_frontier(&mut frontier, expanded + 2).is_none());
        }
    }

    /// [`PREPARATION_TURN`] 件ごとに、準備状態へ順番が回ること。
    ///
    /// # なぜこの検査が要るか
    ///
    /// 準備状態(方向付き・つぶし折り・**花弁折り**)を無条件に後回しにしていたため、
    /// 鳥の基本形を完成させる花弁折りは、**粗い順位で1位に付けていながら
    /// 一度も広げられず**、状態上限12で打ち切られていた
    /// (`scratchpad/petal-tear-cause-report.md` 第1部 §2.4.2)。
    /// 分岐上限を 3 → 6 → 10 と広げても `length` は16桁すべて動かなかった。
    #[test]
    fn every_preparation_turn_gives_the_preparation_states_a_turn() {
        let session = flat_square_session();
        let (mut frontier, preparation_key, regular_key) = two_state_frontier(&session);
        let (selected, _) =
            pop_frontier(&mut frontier, PREPARATION_TURN).expect("準備状態を選べない");
        assert_eq!(
            selected, preparation_key,
            "{PREPARATION_TURN}件ごとの順番で準備状態を広げていない"
        );
        let (next, _) =
            pop_frontier(&mut frontier, PREPARATION_TURN + 1).expect("通常状態を選べない");
        assert_eq!(next, regular_key, "次はまた通常状態へ戻る");

        // 準備状態が1つも無いときは、順番を空回りさせずに通常状態を広げる。
        let (mut only_regular, _, regular_key) = two_state_frontier(&session);
        only_regular.retain(|key, _| *key == regular_key);
        let (selected, _) =
            pop_frontier(&mut only_regular, PREPARATION_TURN).expect("通常状態を選べない");
        assert_eq!(
            selected, regular_key,
            "準備状態が無いのに順番を空回りさせた"
        );
    }

    /// 粗順位の全体1位は、予約枠に関係なく必ず確かめる。
    ///
    /// **2026-08-22に主張を1つ足した**。前は「全体1位のあと、つぶし折りと花弁折りが
    /// それぞれ予約枠を持つ」ことを固定していたが、その規則だと既定の分岐3で
    /// **形を変える手の枠が0**になり、折り鶴が1手目から進めなくなった
    /// (理由は [`candidate_class_quotas`])。いまは
    /// 「全体1位 → 形を変える手 → 準備手」の順で枠が埋まる。
    #[test]
    fn overall_best_candidate_is_checked_before_class_quotas() {
        let classes = [
            CandidateClass::Directional,
            CandidateClass::Regular,
            CandidateClass::Reactivate,
            CandidateClass::Reopen,
        ];
        let quotas = candidate_class_quotas(3);
        let mut attempted = BTreeSet::new();
        let mut kept = [0; 4];

        assert_eq!(
            next_candidate_index(&classes, &attempted, kept, quotas),
            Some(0),
            "予約枠を持たない種類でも、粗順位1位を捨てない"
        );
        attempted.insert(0);
        kept[CandidateClass::Directional.index()] = 1;
        assert_eq!(
            next_candidate_index(&classes, &attempted, kept, quotas),
            Some(1),
            "全体1位が準備手だったときは、次に形を変える手の枠を必ず使う"
        );
        attempted.insert(1);
        kept[CandidateClass::Regular.index()] = 1;
        assert_eq!(
            next_candidate_index(&classes, &attempted, kept, quotas),
            Some(2),
            "全体1位が準備手でも、準備手の枠はまだ残っている(1位は種類を問わない枠で入った)"
        );
    }

    /// 順位の同点解消は「全ID列 → 履歴 → 物理状態」の順である。
    ///
    /// 手番号の列を先に比べることは従来どおり。履歴だけを、折り線の番号ではなく
    /// **動かした直線と目標角**で比べるように変えた([`HistoryKey`])。
    #[test]
    fn ranking_compares_the_full_id_sequence_before_subset_identity() {
        let document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        let session = FoldSession::new(&document).expect("平らな正方形を読み込めない");
        let lower_ids = node_with_moves(&session, &[(1, vec![99]), (2, vec![99])]);
        let higher_ids = node_with_moves(&session, &[(1, vec![0]), (3, vec![0])]);
        let same_ids_other_subset = node_with_moves(&session, &[(1, vec![0]), (2, vec![0])]);

        assert!(
            rank_key(&lower_ids, None) < rank_key(&higher_ids, None),
            "早い手の部分集合ではなく、全ID列[1,2]を[1,3]より先にする"
        );
        assert_eq!(id_key(&lower_ids), id_key(&same_ids_other_subset));
        assert_eq!(
            lower_ids.session.state_key(),
            same_ids_other_subset.session.state_key(),
            "手順履歴が違っても、同じCP・配置・層順・角度なら同じ物理状態"
        );
        let mut seen = BTreeSet::new();
        assert!(seen.insert(lower_ids.session.state_key()));
        assert!(
            !seen.insert(same_ids_other_subset.session.state_key()),
            "履歴だけ違う同じ物理状態を二重に広げた"
        );
    }

    /// 折り目が1本増えて**折り線の番号が全部ずれても**、同点解消の履歴は変わらない。
    ///
    /// 折る途中で折り目が増える手(花弁折りなど)を扱えるようにしたので、これは
    /// 実際に起きる。前の履歴は折り線の番号と閉鎖maskを持っていたため、
    /// 同じ折り方でも番号の付き方が変わるだけで別の値になり、順位が揺れうる形だった。
    #[test]
    fn the_ranking_history_does_not_move_when_crease_lines_are_renumbered() {
        // 対角の谷折り1本だけを持つ紙と、そこへ水平の山折りを1本足した紙。
        // `crease_lines` の番号は端点の座標順なので、足すと番号の付き方が変わる。
        let with_one_crease = creased_document(false);
        let with_extra_crease = creased_document(true);
        assert_ne!(
            crate::plan::crease_lines(&with_one_crease.cp).len(),
            crate::plan::crease_lines(&with_extra_crease.cp).len(),
            "折り線のまとまりの本数が変わっていない。番号がずれる状況を作れていない"
        );

        // どちらの紙でも「同じ対角を180°へ閉じる」1手だけを記録する。
        let plain = node_with_recorded_fold(&with_one_crease);
        let renumbered = node_with_recorded_fold(&with_extra_crease);
        assert_eq!(
            history_key(&plain),
            history_key(&renumbered),
            "折り線の番号がずれただけで同点解消の履歴が変わった"
        );

        // 別の直線を動かした手は、きちんと別物として区別する。
        let mut other = with_one_crease.clone();
        other.sequence[0].drivers[0].a = [0.0, 1.0];
        other.sequence[0].drivers[0].b = [1.0, 0.0];
        let other = node_with_recorded_fold(&other);
        assert_ne!(
            history_key(&plain),
            history_key(&other),
            "違う直線を動かした手が同じ履歴になった"
        );

        // 同じ直線でも目標角が違えば別物である。
        let mut half = with_one_crease.clone();
        half.sequence[0].drivers[0].target_angle_deg = 90.0;
        let half = node_with_recorded_fold(&half);
        assert_ne!(
            history_key(&plain),
            history_key(&half),
            "目標角が違う手が同じ履歴になった"
        );
    }

    /// 対角の谷折りを1本持つ紙。`extra` を立てると水平の山折りを1本足す。
    /// どちらにも「その対角を180°へ閉じる」手順を1手だけ記録しておく。
    fn creased_document(extra: bool) -> Document {
        let mut document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        ori3_cp::insert_segment(
            &mut document.cp,
            [0.0, 0.0],
            [1.0, 1.0],
            ori3_model::EdgeKind::Valley,
        );
        if extra {
            ori3_cp::insert_segment(
                &mut document.cp,
                [0.0, 0.75],
                [0.75, 0.0],
                ori3_model::EdgeKind::Mountain,
            );
        }
        document.sequence.push(ori3_model::FoldStep {
            id: 0,
            kind: ori3_model::TechniqueKind::Simple,
            drivers: vec![ori3_model::DriverLine {
                a: [0.0, 0.0],
                b: [1.0, 1.0],
                target_angle_deg: 180.0,
            }],
            layer_order: None,
            alignment: None,
            finish_soft: None,
            note: String::new(),
        });
        document
    }

    /// 1手だけ記録された作品から、探索の1状態を作る。
    fn node_with_recorded_fold(document: &Document) -> Node {
        let session = FoldSession::new(document).expect("折り筋のある紙を読み込めない");
        node_with_moves(&session, &[(0, vec![0])])
    }

    /// 0ms watchdogを100回注入しても、通常結果を1件も返さないこと。
    ///
    /// 旧契約は `SearchStop::TimeCap` つきの最善途中値を返していたため、機械の負荷が
    /// 候補の中身を変えた。新契約は専用Errだけを返し、`SearchOutcome`を作らない。
    #[test]
    fn watchdog_injection_returns_one_hundred_aborts_and_no_outcomes() {
        let document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        let session = FoldSession::new(&document).expect("平らな正方形を読み込めない");
        let goal = FoldGoal {
            target: FinishTarget::default(),
            body: [0.5, 0.5],
            sites: Vec::new(),
            layer_target: None,
        };
        let watchdog = SearchWatchdog { max_millis: 0 };
        let mut watchdog_aborts = 0;
        let mut outcomes = 0;
        for _ in 0..100 {
            let not_cancelled = || false;
            let control = SearchControl::new(watchdog, &not_cancelled);
            match search_to_completion_with_control(
                &session,
                &goal,
                GapWeights::DEFAULT,
                SearchBudget::DEFAULT,
                CompletionTolerance::DEFAULT,
                &control,
            ) {
                Err(SearchAbort::WatchdogExpired) => watchdog_aborts += 1,
                Err(other) => panic!("watchdog注入が別の理由になった: {other:?}"),
                Ok(_) => outcomes += 1,
            }
        }
        assert_eq!(watchdog_aborts, 100);
        assert_eq!(outcomes, 0, "watchdogが通常の探索結果へ化けた");
        assert_eq!(SearchWatchdog::MAX_MILLIS, 600_000);
        assert_eq!(SearchWatchdog::DEFAULT.max_millis, 600_000);
    }

    /// 取消し済みの札を100回注入しても、通常結果を1件も返さないこと。
    #[test]
    fn cancellation_injection_returns_one_hundred_aborts_and_no_outcomes() {
        let document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        let session = FoldSession::new(&document).expect("平らな正方形を読み込めない");
        let goal = FoldGoal {
            target: FinishTarget::default(),
            body: [0.5, 0.5],
            sites: Vec::new(),
            layer_target: None,
        };
        let mut cancelled = 0;
        let mut outcomes = 0;
        for _ in 0..100 {
            let is_cancelled = || true;
            let control = SearchControl::new(SearchWatchdog::DEFAULT, &is_cancelled);
            match search_to_finish_with_control(
                &session,
                &goal,
                GapWeights::DEFAULT,
                SearchBudget::DEFAULT,
                &control,
            ) {
                Err(SearchAbort::Cancelled) => cancelled += 1,
                Err(other) => panic!("取消し注入が別の理由になった: {other:?}"),
                Ok(_) => outcomes += 1,
            }
        }
        assert_eq!(cancelled, 100);
        assert_eq!(outcomes, 0, "取消しが通常の探索結果へ化けた");
    }
}
