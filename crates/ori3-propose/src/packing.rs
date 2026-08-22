//! 円・川充填の数値最適化(PRO-002 / 要件§8-2)。
//!
//! 変数は各葉(角)の円中心と縮尺 `s`。目的は `s` の最大化、制約は
//! 「葉iとjの円中心の距離 ≥ s ×(円半径i + 経路上の川幅 + 円半径j)」と
//! 「円中心が紙の内側」。円そのものは紙からはみ出してよい(紙の角・辺に
//! 置いた角がそうなる)。
//!
//! 解き方は射影法(制約違反を押し戻す掃引)+ 段階的な拡大。乱数シードを
//! 変えたマルチスタートで局所解を避け、上位候補を返す(PRO-005)。

use crate::skeleton::{MAX_LEAVES, Skeleton};
use crate::triangulate::MERGE_TOL;
use rand::{RngExt, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};

/// 返せる候補の最大数(PRO-005)。
pub const MAX_CANDIDATES: usize = 4;

/// 「制約を満たしている」とみなす違反量の上限。
pub const PACK_TOL: f64 = 1e-9;

/// 縮尺の二分探索の刻み数、1刻みあたりの押し離し掃引数、揺さぶり直しの回数。
/// 12葉・8スタートで数百ms以内に収まり、かつ質が頭打ちになる値に合わせた。
const BISECT_STEPS: usize = 28;
const RELAX_SWEEPS: usize = 48;
const SHAKE_ROUNDS: usize = 3;
/// 局所解への停滞を判断する、既定のマルチスタート数。
const STAGNATION_STARTS: usize = 8;

/// 先端(葉)1本ぶんの円の記録(作業9)。
///
/// どの先端がどの円になったかを、番号で名指しできる形で残すための型。
/// 展開図側の対応(`crate::generate::LeafSite`)はこの型をそのまま持つので、
/// 先端 → 円 → 展開図の点、を推測なしにたどれる。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeafCircle {
    /// この円を使う先端(葉)のID。
    pub leaf_id: u32,
    /// 円の番号。`Packing::circles` と `Packing::centers` の並び順と同じ。
    pub circle_index: usize,
    /// 紙の上での円の中心。
    pub center: [f64; 2],
    /// 紙の上での円の半径(骨格の半径に縮尺を掛けた実寸)。
    pub radius: f64,
}

/// 充填の結果。`violation` は制約違反の最大量で、0に近いほど良い。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Packing {
    pub scale: f64,
    pub centers: Vec<(u32, [f64; 2])>,
    pub violation: f64,
    /// 先端と円の対応(作業9)。`centers` と同じ並びの同じ円を、番号と半径を
    /// 付けて名指しできる形にしたもの。`centers` は今までの呼び出し側が
    /// そのまま使えるよう形を変えずに残してある。
    ///
    /// 手で組み立てた `Packing` や、この欄が無かったころの保存から読んだ
    /// `Packing` では空になる。空のときは [`Packing::leaf_circles`] が
    /// `centers` と骨格から同じ内容を組み立てるので、使う側は空かどうかを
    /// 気にしなくてよい。
    #[serde(default)]
    pub circles: Vec<LeafCircle>,
}

impl Packing {
    /// 先端と円の対応を取り出す(作業9)。
    ///
    /// `circles` が入っていればそれを返し、空(手で組み立てた・古い保存)なら
    /// `centers` と骨格から同じ内容を組み立てる。どちらの道でも、円の番号は
    /// `centers` の並び順と一致する。
    pub fn leaf_circles(&self, skeleton: &Skeleton) -> Vec<LeafCircle> {
        if self.circles.len() == self.centers.len() {
            return self.circles.clone();
        }
        self.centers
            .iter()
            .enumerate()
            .map(|(circle_index, &(leaf_id, center))| LeafCircle {
                leaf_id,
                circle_index,
                center,
                radius: self.scale * skeleton.leaf_radius(leaf_id),
            })
            .collect()
    }
}

/// 指定した完成形の位置を、紙の上のどこへ置くかへ写した結果(作業10)。
///
/// ## なぜ完成形の位置が紙の上の位置に写せるのか
///
/// 円・川充填で作る形(一軸基本形)では、**紙が骨格の木の上へ畳み込まれる**。
/// 各先端の円中心はその先端の節点へ、胴は根へ移る。制約
/// 「2つの円中心の距離 ≥ 縮尺 ×(木の上の距離)」は、紙の上に描いた木が
/// 完成形の木を縮めずに写した図であることを言っている。
/// つまり **紙の上に並んだ円中心の並び方が、そのまま完成形を正面から見た
/// 先端の並び方になる**。だから完成形での位置指定は、紙の上の目標点へ写せる。
///
/// ## 写し方
///
/// 指定 `p_i`(横も縦も `-1.0`〜`1.0`、原点が胴)を、紙の上の点
/// `t_i = body + K * p_i` へ写す。`body` は紙の上で胴が来る場所、`K` は共通の倍率。
///
/// - **並び方だけを写し、大きさは写さない。** 作品の大きさは角の長さ
///   (`SkeletonNode::length`)が決めるもので、位置の指定は「どちらの向きに、
///   どれくらいの割合で離れて並ぶか」しか言っていない。
///   これは完成形を測る側([`crate::finish::FinishedForm::with_tip_points`])が
///   大きさをそろえてから比べるのと同じ考え方である。
/// - `K` は **紙の中で目標の並びをいちばん大きく使える値**にする。`K` が大きい
///   ほど作品も大きくなるので、これは同時に「指定した並びの中で作品を最大に
///   する」ことになる。
/// - 囲む四角には **原点(胴)を必ず入れる**。胴も紙の上に無ければならない。
///
/// ## 収まらない指定
///
/// - 枠の外の指定は、いちばん近い縁へ寄せて [`TipTargets::notices`] で知らせる。
/// - 2本の先端を同じ場所へ指定すると、その2本は紙の上で離せない。このときは
///   [`TipTargets::conflicting`] を立て、目標点は出発点としてだけ使い、
///   離れた置き方を自動で探す。どちらの場合も計算は止めない(`CLAUDE.md` §8)。
#[derive(Clone, Debug, PartialEq)]
pub struct TipTargets {
    /// 紙の上で胴(骨格の根)が来る場所。
    pub body: [f64; 2],
    /// (葉ID, 紙の上の目標点)。位置を指定した先端だけが葉ID昇順で入る。
    pub points: Vec<(u32, [f64; 2])>,
    /// 利用者へ見せる日本語の知らせ。指定に無理が無ければ空。
    pub notices: Vec<String>,
    /// 指定した並びのままでは置けない(同じ場所を指した先端がある)。
    pub conflicting: bool,
}

fn out_of_frame_notice(name: &str) -> String {
    format!("{name}を出したい場所が決められる範囲の外だったので、いちばん近いところへ寄せました")
}

const SAME_PLACE_NOTICE: &str = "同じ場所を出したい先端が2本以上あるため、そのままでは重なってしまいます。いちばん近い置き方にしました";

/// 指定した完成形の位置を、紙の上の目標点へ写す(作業10 / PRO-006・PRO-007)。
///
/// 位置を1つも指定していない骨格、または紙の寸法が使えないときは `None`。
/// 写し方の考え方は [`TipTargets`] を参照。
#[must_use]
pub fn tip_targets(skeleton: &Skeleton, paper_w: f64, paper_h: f64) -> Option<TipTargets> {
    if !(paper_w > 0.0 && paper_h > 0.0 && paper_w.is_finite() && paper_h.is_finite()) {
        return None;
    }
    let given = skeleton.leaf_tip_positions();
    if given.is_empty() {
        return None;
    }

    // 枠の外・数にならない指定は、いちばん近い縁へ寄せて知らせる(止めない)。
    let mut notices = Vec::new();
    let mut wanted: Vec<(u32, [f64; 2])> = Vec::with_capacity(given.len());
    for (leaf_id, p) in given {
        let fit = p.clamped();
        if !(p.x == fit.x && p.y == fit.y) {
            notices.push(out_of_frame_notice(&crate::generate::limb_name(
                skeleton, leaf_id,
            )));
        }
        wanted.push((leaf_id, [fit.x, fit.y]));
    }

    // 指定を囲む四角(胴の原点を必ず含む)を、紙いっぱいに広げる倍率。
    let (mut x0, mut x1, mut y0, mut y1) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (_, p) in &wanted {
        x0 = x0.min(p[0]);
        x1 = x1.max(p[0]);
        y0 = y0.min(p[1]);
        y1 = y1.max(p[1]);
    }
    let (ex, ey) = (x1 - x0, y1 - y0);
    let k = match (ex > 0.0, ey > 0.0) {
        (true, true) => (paper_w / ex).min(paper_h / ey),
        (true, false) => paper_w / ex,
        (false, true) => paper_h / ey,
        (false, false) => f64::INFINITY,
    };
    // 並びが1点に潰れていると倍率が決まらない(すべての先端を胴と同じ場所へ
    // 指定した場合や、広がりが小さすぎて割り算が数にならない場合)。
    // このときは目標点を作らず、知らせだけを返して自動の置き方に任せる。
    if !(k.is_finite() && k > 0.0) {
        notices.push(SAME_PLACE_NOTICE.to_string());
        return Some(TipTargets {
            body: [paper_w * 0.5, paper_h * 0.5],
            points: Vec::new(),
            notices,
            conflicting: true,
        });
    }
    // 囲む四角を紙いっぱいに広げているので、胴も目標点も紙の中に収まる。
    // 小数の丸めで縁を1つ分だけ跨ぐことがあるので、念のため紙の中へ収める
    // (案A: 円の中心は紙の内側)。
    let fit = |p: [f64; 2]| [p[0].clamp(0.0, paper_w), p[1].clamp(0.0, paper_h)];
    let body = fit([
        paper_w * 0.5 - k * (x0 + x1) * 0.5,
        paper_h * 0.5 - k * (y0 + y1) * 0.5,
    ]);
    let points: Vec<(u32, [f64; 2])> = wanted
        .iter()
        .map(|&(leaf_id, p)| (leaf_id, fit([body[0] + k * p[0], body[1] + k * p[1]])))
        .collect();

    // 同じ点になってしまった先端どうしは、紙の上で離せない。境目は展開図が
    // 「同じ点」とみなす距離([`MERGE_TOL`] = 1e-7)で、これより近い2つの中心は
    // 展開図の上で1つの点にまとめられてしまう。
    let mut conflicting = false;
    for a in 0..points.len() {
        for b in (a + 1)..points.len() {
            let need = skeleton.leaf_distance(points[a].0, points[b].0);
            if need > 0.0 && dist(points[a].1, points[b].1) < MERGE_TOL {
                conflicting = true;
            }
        }
    }
    if conflicting {
        notices.push(SAME_PLACE_NOTICE.to_string());
    }

    Some(TipTargets {
        body,
        points,
        notices,
        conflicting,
    })
}

/// 紙の上で胴(骨格の根)が来る場所(作業10)。
///
/// 完成形の位置を測るときの原点になる。位置の指定があるときは、指定の枠の原点を
/// 紙へ写した点([`TipTargets::body`])をそのまま返す。指定が無いときは、
/// 紙の上の情報だけから見積もるほかないので、円中心の重心を返す。
#[must_use]
pub fn body_on_paper(
    skeleton: &Skeleton,
    packing: &Packing,
    paper_w: f64,
    paper_h: f64,
) -> [f64; 2] {
    if let Some(t) = tip_targets(skeleton, paper_w, paper_h)
        && !t.points.is_empty()
    {
        return t.body;
    }
    let n = packing.centers.len();
    if n == 0 {
        return [paper_w * 0.5, paper_h * 0.5];
    }
    let mut sum = [0.0f64; 2];
    for &(_, c) in &packing.centers {
        sum[0] += c[0];
        sum[1] += c[1];
    }
    [sum[0] / n as f64, sum[1] / n as f64]
}

/// 最適化しやすい形にほぐした問題。
struct Problem {
    ids: Vec<u32>,
    radii: Vec<f64>,
    /// (葉の添字i, 葉の添字j, 必要距離d)。実距離が `s*d` 以上であればよい。
    pairs: Vec<(usize, usize, f64)>,
    w: f64,
    h: f64,
    /// 位置を指定した先端の、紙の上の目標点(葉の並び順)。指定が無ければ全て `None`。
    targets: Vec<Option<[f64; 2]>>,
    /// 目標点から動かさないか。指定どおりでは置けないときだけ `false` で、
    /// そのときは目標点を出発点としてだけ使う。
    freeze: bool,
    /// 縮尺の上限。指定した並びを動かさないときは、その並びが成り立たせる縮尺。
    /// 指定が無ければ無限大で、今までの計算と1つも違わない。
    scale_cap: f64,
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

impl Problem {
    fn new(skeleton: &Skeleton, w: f64, h: f64) -> Self {
        let ids = skeleton.leaves();
        let radii = ids.iter().map(|&id| skeleton.leaf_radius(id)).collect();
        let mut pairs = Vec::new();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                pairs.push((i, j, skeleton.leaf_distance(ids[i], ids[j])));
            }
        }
        let targets = vec![None; ids.len()];
        Self {
            ids,
            radii,
            pairs,
            w,
            h,
            targets,
            freeze: false,
            scale_cap: f64::INFINITY,
        }
    }

    /// 指定した完成形の位置から作った目標点を問題へ入れる(作業10)。
    ///
    /// 指定どおりに置けるとき(`conflicting` でないとき)は、その先端を目標点へ
    /// 固定する。固定した先端どうしの間で成り立つ縮尺が、そのまま作品全体の
    /// 縮尺の上限になる。指定していない先端は今までどおり自由に動く。
    fn set_targets(&mut self, guide: &TipTargets) {
        for &(leaf_id, point) in &guide.points {
            if let Some(i) = self.ids.iter().position(|id| *id == leaf_id) {
                self.targets[i] = Some(point);
            }
        }
        self.freeze = !guide.conflicting;
        self.scale_cap = if self.freeze {
            self.targets_scale()
        } else {
            f64::INFINITY
        };
    }

    /// 目標点へ固定した先端どうしの間で成り立つ縮尺。固定が1本以下なら無限大。
    fn targets_scale(&self) -> f64 {
        let mut s = f64::INFINITY;
        for &(i, j, d) in &self.pairs {
            let (Some(a), Some(b)) = (self.targets[i], self.targets[j]) else {
                continue;
            };
            if d > 0.0 {
                s = s.min(dist(a, b) / d);
            }
        }
        s
    }

    /// 目標点へ固定されている先端か。
    fn is_frozen(&self, i: usize) -> bool {
        self.freeze && self.targets[i].is_some()
    }

    /// この配置で成り立つ最大の縮尺。制約が1つもなければ無限大。
    fn scale_of(&self, c: &[[f64; 2]]) -> f64 {
        let mut s = f64::INFINITY;
        for &(i, j, d) in &self.pairs {
            if d > 0.0 {
                s = s.min(dist(c[i], c[j]) / d);
            }
        }
        s
    }

    /// 指定縮尺での制約違反の最大量(円の重なり / 紙のはみ出し)。
    fn violation_of(&self, s: f64, c: &[[f64; 2]]) -> f64 {
        let mut v: f64 = 0.0;
        for &(i, j, d) in &self.pairs {
            v = v.max(s * d - dist(c[i], c[j]));
        }
        for p in c {
            v = v
                .max(-p[0])
                .max(p[0] - self.w)
                .max(-p[1])
                .max(p[1] - self.h);
        }
        v.max(0.0)
    }

    /// 先端と円の対応を、この問題の葉の並び順(=`ids`)で組み立てる(作業9)。
    fn circles(&self, scale: f64, centers: &[[f64; 2]]) -> Vec<LeafCircle> {
        self.ids
            .iter()
            .enumerate()
            .map(|(circle_index, &leaf_id)| LeafCircle {
                leaf_id,
                circle_index,
                center: centers[circle_index],
                radius: scale * self.radii[circle_index],
            })
            .collect()
    }

    /// 最大葉数の高密度配置を、紙の縦横比に合う千鳥格子から構成する。
    ///
    /// 数値探索は対称な直交格子に停滞すると、満足済みの対を並べ替えられない。
    /// 12点の因数対と千鳥の向きを列挙し、同じ制約式で最良の構成解を選ぶ。
    fn staggered_fallback(&self) -> Option<(f64, Vec<[f64; 2]>)> {
        if self.ids.len() != MAX_LEAVES {
            return None;
        }

        let mut best: Option<(f64, Vec<[f64; 2]>)> = None;
        for rows in 2..=MAX_LEAVES {
            if !MAX_LEAVES.is_multiple_of(rows) {
                continue;
            }
            let columns = MAX_LEAVES / rows;
            if columns < 2 {
                continue;
            }
            let dy = self.h / (rows - 1) as f64;
            let dx = self.w / (columns as f64 - 0.5);
            for first_row_offset in [false, true] {
                let mut centers = Vec::with_capacity(MAX_LEAVES);
                for row in 0..rows {
                    let offset = if (row % 2 == 1) ^ first_row_offset {
                        dx * 0.5
                    } else {
                        0.0
                    };
                    for column in 0..columns {
                        let mut center = [offset + column as f64 * dx, row as f64 * dy];
                        self.clamp(&mut center);
                        centers.push(center);
                    }
                }
                let scale = self.scale_of(&centers);
                if scale.is_finite()
                    && scale > 0.0
                    && best
                        .as_ref()
                        .is_none_or(|(best_scale, _)| scale > *best_scale)
                {
                    best = Some((scale, centers));
                }
            }
        }
        best
    }

    fn clamp(&self, p: &mut [f64; 2]) {
        p[0] = p[0].clamp(0.0, self.w);
        p[1] = p[1].clamp(0.0, self.h);
    }

    /// 目標縮尺を満たすよう、近すぎる対を押し離す掃引を繰り返す(射影)。
    /// 動かした量は関わった制約の数で割って平均化し、振動を抑える。
    fn relax(&self, c: &mut [[f64; 2]], target: f64, sweeps: usize) {
        let mut disp = vec![[0.0f64; 2]; c.len()];
        let mut cnt = vec![0u32; c.len()];
        for _ in 0..sweeps {
            disp.fill([0.0, 0.0]);
            cnt.fill(0);
            let mut moved = false;
            for &(i, j, d) in &self.pairs {
                let need = target * d;
                let cur = dist(c[i], c[j]);
                if cur >= need {
                    continue;
                }
                moved = true;
                // 中心がぴったり重なったときは添字で決まる向きへ逃がす(決定的)。
                let dir = if cur > 1e-12 {
                    [(c[j][0] - c[i][0]) / cur, (c[j][1] - c[i][1]) / cur]
                } else {
                    let a = (i * 7 + j * 13) as f64 * 0.7;
                    [a.cos(), a.sin()]
                };
                let push = (need - cur) * 0.5;
                for (k, sign) in [(i, -1.0), (j, 1.0)] {
                    disp[k][0] += sign * dir[0] * push;
                    disp[k][1] += sign * dir[1] * push;
                    cnt[k] += 1;
                }
            }
            if !moved {
                break;
            }
            for (i, ((p, d), k)) in c.iter_mut().zip(disp.iter()).zip(cnt.iter()).enumerate() {
                // 位置を指定した先端は目標点から動かさない。押し離しには参加する
                // (相手を押しのける)が、自分は動かない。
                if self.is_frozen(i) {
                    continue;
                }
                let m = f64::from((*k).max(1));
                p[0] += d[0] / m;
                p[1] += d[1] / m;
                self.clamp(p);
            }
        }
    }

    /// 初期配置。紙の角・辺・中心という有利な場所へ大きい円から貪欲に置き、
    /// シードごとに並びと揺らぎを変える(要件§8-2)。
    fn initial(&self, start: usize, rng: &mut StdRng) -> Vec<[f64; 2]> {
        let (w, h) = (self.w, self.h);
        // 角は対角どうしを先に使う(2本の角が同じ辺に並んで動けなくなるのを防ぐ)。
        let anchors = [
            [0.0, 0.0],
            [w, h],
            [w, 0.0],
            [0.0, h],
            [w * 0.5, 0.0],
            [w * 0.5, h],
            [0.0, h * 0.5],
            [w, h * 0.5],
            [w * 0.5, h * 0.5],
        ];
        let n = self.ids.len();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| self.radii[b].total_cmp(&self.radii[a]));
        if start > 0 {
            for k in (1..n).rev() {
                order.swap(k, rng.random_range(0..=k));
            }
        }
        // 偶数番のシードは角・辺を使わず完全な乱数配置にして候補を散らす。
        let anchor_count = match start {
            0 => anchors.len(),
            s if s % 2 == 1 => 4,
            _ => 0,
        };
        let jitter = if start == 0 { 0.0 } else { 0.15 * w.min(h) };
        let mut pos = vec![[0.0; 2]; n];
        for (k, &li) in order.iter().enumerate() {
            let mut p = match anchors.get(k) {
                Some(a) if k < anchor_count && jitter <= 0.0 => *a,
                Some(a) if k < anchor_count => [
                    a[0] + rng.random_range(-jitter..jitter),
                    a[1] + rng.random_range(-jitter..jitter),
                ],
                _ => [rng.random_range(0.0..w), rng.random_range(0.0..h)],
            };
            self.clamp(&mut p);
            pos[li] = p;
        }
        // 位置を指定した先端は、その指定から作った目標点から始める(作業10)。
        // 指定が1つも無ければ何も起きず、今までの置き方と1つも違わない。
        for (p, t) in pos.iter_mut().zip(self.targets.iter()) {
            if let Some(target) = t {
                *p = *target;
            }
        }
        pos
    }

    /// 1スタート分の最適化。二分探索で縮尺を上げ、行き詰まったら少し揺さぶって
    /// やり直す(格子状の行き止まりから抜けるため)。
    /// 返す縮尺はその配置が実際に満たす値なので、制約違反は生じない。
    fn solve_one(&self, start: usize, rng: &mut StdRng) -> (f64, Vec<[f64; 2]>) {
        // 紙の対角線より遠くへは離せないので、これが縮尺の上界になる。
        let d_max = self.pairs.iter().map(|p| p.2).fold(0.0_f64, f64::max);
        let s_hi = if d_max > 0.0 {
            self.w.hypot(self.h) / d_max
        } else {
            0.0
        }
        // 位置を指定した先端を動かさないなら、その並びが決める縮尺より上へは行けない。
        // 指定が無いときの `scale_cap` は無限大で、この行は値を変えない。
        .min(self.scale_cap);
        let mut best = self.initial(start, rng);
        let mut best_s = self.scale_of(&best).min(s_hi);
        for round in 0..=SHAKE_ROUNDS {
            let mut work = best.clone();
            if round > 0 {
                let amp = 0.08 * self.w.min(self.h);
                for (i, p) in work.iter_mut().enumerate() {
                    let shake = [rng.random_range(-amp..amp), rng.random_range(-amp..amp)];
                    // 目標点へ固定した先端は揺さぶらない。乱数は同じだけ引くので、
                    // 指定が無いときの並びは今までと1つも変わらない。
                    if self.is_frozen(i) {
                        continue;
                    }
                    p[0] += shake[0];
                    p[1] += shake[1];
                    self.clamp(p);
                }
            }
            let (s, c) = self.bisect(work, self.scale_of(&best).min(s_hi), s_hi);
            if s > best_s {
                best_s = s;
                best = c;
            }
        }
        (best_s.max(0.0), best)
    }

    /// 与えた配置を出発点に、達成できる縮尺を二分探索で詰める。
    fn bisect(&self, start: Vec<[f64; 2]>, from: f64, s_hi: f64) -> (f64, Vec<[f64; 2]>) {
        let mut best = start;
        let mut best_s = self.scale_of(&best).min(s_hi);
        // 出発点が既に満たしている縮尺より下は探さない。
        let (mut lo, mut hi) = (best_s.max(from.min(s_hi)), s_hi);
        for _ in 0..BISECT_STEPS {
            if hi - lo <= 1e-12 * s_hi.max(1.0) {
                break;
            }
            let mid = 0.5 * (lo + hi);
            let mut c = best.clone();
            self.relax(&mut c, mid, RELAX_SWEEPS);
            let s = self.scale_of(&c);
            // 目標に届いたら下限を上げ、届かなければ上限を下げる。
            if s >= mid * (1.0 - 1e-9) {
                lo = mid;
            } else {
                hi = mid;
            }
            // 届かなかった配置でも、これまでより広ければ採用する。
            if s > best_s {
                best_s = s;
                best = c;
            }
        }
        (best_s.max(0.0), best)
    }
}

/// 骨格を紙の上に円・川充填する(PRO-002)。
///
/// `seed` と `starts` が同じなら結果も同じ(決定的)。スコア順に最大
/// [`MAX_CANDIDATES`] 件を返す。骨格や紙寸法が不正なときは空を返す。
///
/// 完成形の先端位置([`crate::skeleton::TipPos2d`])を指定した先端は、その並びを
/// 保つ場所へ置く(作業10)。写し方は [`TipTargets`] を参照。指定していない先端は
/// 今までどおり自動で置く。**位置を1つも指定していない骨格の結果は、この仕組みを
/// 入れる前と1ビットも変わらない。**
///
/// 枠の外の指定や、同じ場所を指した指定でも**止まらない**。いちばん近い置き方に
/// して、[`tip_targets`] が返す日本語の知らせで伝える(`CLAUDE.md` §8)。
pub fn pack(
    skeleton: &Skeleton,
    paper_w: f64,
    paper_h: f64,
    seed: u64,
    starts: usize,
) -> Vec<Packing> {
    let ok_paper = paper_w > 0.0 && paper_h > 0.0 && paper_w.is_finite() && paper_h.is_finite();
    // 位置の枠外は止めずに寄せるので、ここでは骨格の形だけを見る。
    if !ok_paper || skeleton.validate_structure().is_err() {
        return Vec::new();
    }
    let mut p = Problem::new(skeleton, paper_w, paper_h);
    let guide = tip_targets(skeleton, paper_w, paper_h);
    let position_unspecified = guide.is_none();
    if let Some(guide) = &guide {
        p.set_targets(guide);
    }
    let guided = p.targets.iter().any(Option::is_some);
    // 角が1本だけなら対の制約がない。円の直径が紙の短辺に収まる縮尺を返す。
    if p.pairs.is_empty() {
        let r = p.radii.first().copied().unwrap_or(0.0);
        let scale = if r > 0.0 {
            paper_w.min(paper_h) / (2.0 * r)
        } else {
            1.0
        };
        let position = p
            .targets
            .first()
            .copied()
            .flatten()
            .unwrap_or([paper_w * 0.5, paper_h * 0.5]);
        return vec![Packing {
            scale,
            centers: vec![(p.ids[0], position)],
            violation: 0.0,
            circles: p.circles(scale, &[position]),
        }];
    }
    let mut out: Vec<Packing> = Vec::new();
    let start_count = starts.clamp(1, 64);
    for start in 0..start_count {
        let mix = (start as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut rng = StdRng::seed_from_u64(seed ^ mix);
        let (scale, centers) = p.solve_one(start, &mut rng);
        out.push(Packing {
            violation: p.violation_of(scale, &centers),
            circles: p.circles(scale, &centers),
            centers: p.ids.iter().copied().zip(centers).collect(),
            scale,
        });
    }
    // 既定8開始の上位候補枠が同じ実行可能縮尺で埋まったときだけ、対称な
    // 局所解から抜ける構成候補を加える。位置指定経路と、既に別の最良配置を
    // 見つけた探索の候補・乱数列は一切変えない。
    let stalled = position_unspecified
        && out.len() >= STAGNATION_STARTS
        && out[..STAGNATION_STARTS]
            .iter()
            .all(|candidate| candidate.scale.is_finite() && candidate.violation <= PACK_TOL)
        && {
            let sampled = &out[..STAGNATION_STARTS];
            let max_scale = sampled
                .iter()
                .map(|candidate| candidate.scale)
                .fold(f64::NEG_INFINITY, f64::max);
            sampled
                .iter()
                .filter(|candidate| max_scale - candidate.scale <= PACK_TOL)
                .count()
                >= MAX_CANDIDATES
        };
    if stalled {
        let old_best = out
            .iter()
            .map(|candidate| candidate.scale)
            .fold(f64::NEG_INFINITY, f64::max);
        if let Some((scale, centers)) = p.staggered_fallback()
            && scale > old_best + PACK_TOL
        {
            out.push(Packing {
                violation: p.violation_of(scale, &centers),
                circles: p.circles(scale, &centers),
                centers: p.ids.iter().copied().zip(centers).collect(),
                scale,
            });
        }
    }
    // 制約を満たすものを先に、その中では縮尺の大きい順に並べる。
    let feasible = |x: &Packing| u8::from(x.violation > PACK_TOL);
    out.sort_by(|a, b| {
        feasible(a)
            .cmp(&feasible(b))
            .then(b.scale.total_cmp(&a.scale))
    });
    // 位置を指定すると、指定した先端は毎回同じ場所へ固定される。すべての先端に
    // 指定があると、やり直しの回数だけ同じ配置が並ぶので、同じものは1つにまとめる。
    if guided {
        out.dedup_by(|a, b| a.scale.to_bits() == b.scale.to_bits() && a.centers == b.centers);
    }
    out.truncate(MAX_CANDIDATES);
    out
}
