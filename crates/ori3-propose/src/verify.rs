//! 作業23「全手順と完成形を検証する仕組み」。
//!
//! ## なぜこれが要るか
//!
//! 作業21([`crate::enumerate`])は「その状態から**1手**が折れるか」を確かめ、
//! 作業22([`crate::search`])はその手を完成形へ近づく順に選んで**手順**を返す。
//! ところが、**返した手順を最初から最後まで通したときに本当にその形になるか**を
//! 確かめる道が無かった。
//!
//! 探索は「その場その場で折れる手」を積み上げるだけなので、
//! 手順を渡された側(利用者や画面)から見ると、
//!
//! - 途中の手が抜けている
//! - 手の順番が入れ替わっている
//! - そもそも折れない手が混ざっている
//!
//! といった手順を受け取っても、**折り始めるまで気づけない**。
//! このモジュールは手順を最初から順に全部折り直し、
//! **どこで、どの確認で落ちたか**を返す。
//!
//! ## 何を見るか
//!
//! | いつ | 見ていること | 判断のもと |
//! |---|---|---|
//! | 各手 | その番号の折り線が、その時点の展開図にあるか | [`FoldSession::has_fold_line`] |
//! | 各手 | その折り線がまだ折り終えていないか | [`FoldSession::check_move`] |
//! | 各手 | 平らに畳める・裂けない・すり抜けない・途中の姿勢が成り立つ | [`FoldSession::check_move`](作業21の4条件) |
//! | 各手 | 折った後も紙の重なり順が保てる | [`FoldSession::apply`] と [`flat_state_at`] |
//! | 最後の形 | 手順が飛ばされていない・警告が無い・面が欠けていない・座標がすべて有限 | [`replay`] を `t = 1` で |
//! | 最後の形 | 裂けない・すり抜けない | [`max_seam_gap`] / [`self_intersection_pairs`] |
//! | 最後の形 | 指定した完成形にどれだけ近いか | 作業20の4つの物差し([`finish_gaps`]) |
//!
//! **各手の確認は、折り終わった形だけでなく折っている途中も見る。**
//! [`PoseScan::DEFAULT`] なら `t = 0, 0.05, …, 1` の21点で、
//! そのつど裂けとすり抜けを測る(作業21と同じ細かさ)。
//!
//! ## 止めずに、どこで落ちたかを返す
//!
//! 落ちても panic せず、[`VerifyReport::failure`] に
//! **何手目の・どの折り線で・どの確認で**落ちたかを入れて返す
//! (`CLAUDE.md` §8「止めずに警告する」)。
//! そこまでに通った手は [`VerifyReport::steps`] にそのまま残るので、
//! 「4手のうち2手までは折れた」という言い方ができる。

use ori3_cp::extract_faces;
use ori3_layers::replay::{flat_state_at, replay};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

use crate::enumerate::{FoldSession, MAX_SEAM_GAP, PoseScan, Unverified, VerifiedMove};
use crate::finish::{FinishGaps, finish_gaps};
use crate::search::{CompletionTolerance, FoldGoal, GapWeights, SearchOutcome, SearchStop};

/// 手が1つ通ったときの測り値。
#[derive(Clone, Debug, PartialEq)]
pub struct StepCheck {
    /// 何手目か(0から数える)。利用者へ見せるときは `+1` する。
    pub index: usize,
    /// 折り線の番号([`crate::FoldLine::id`])。
    pub id: usize,
    /// 閉じた直線の端から端(材料座標)。
    pub line: [[f64; 2]; 2],
    /// この手の途中の姿勢すべてを通した、裂けの最大量。[`MAX_SEAM_GAP`] 未満。
    pub max_seam_gap: f64,
    /// この手の途中の姿勢すべてを通した、すり抜けの最大件数。通った手では0。
    pub penetrations: usize,
    /// この手で見た途中の姿勢の数。
    pub poses_checked: usize,
    /// 折った後に紙の重なり順を求め直したときの、食い違いの件数。通った手では0。
    pub layer_warnings: usize,
}

impl StepCheck {
    fn from_move(index: usize, mv: &VerifiedMove, layer_warnings: usize) -> Self {
        Self {
            index,
            id: mv.id,
            line: mv.line,
            max_seam_gap: mv.max_seam_gap,
            penetrations: mv.penetrations,
            poses_checked: mv.poses_checked,
            layer_warnings,
        }
    }
}

/// 手順が落ちた理由。
///
/// **文章をそのまま持たない。** 折る手続きが返す説明文には実行のたびに変わる
/// 辺の番号が入るため、結果へ載せると同じ入力でも一致しなくなる
/// (作業21 §5、`CLAUDE.md` §10.7.7)。理由の**種類**は毎回同じなので種類だけを持つ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepFailure {
    /// その番号の折り線が、その時点の展開図に無い。
    ///
    /// 折り線の番号は**折るたびに振り直される**(展開図が変わるため)。
    /// 手の順番が入れ替わった手順は、多くの場合ここで落ちる。
    NoSuchFoldLine,
    /// その折り線は、その時点でもう全部折り終えている。
    AlreadyFolded,
    /// 手は選べるが、折れない(作業21の4条件のどれかを満たさない)。
    NotFoldable(Unverified),
    /// 折れると確かめられたのに、進めると紙の重なり順が保てない。
    LayerOrderBroken,
}

impl StepFailure {
    /// 報告用の短い名前。
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            StepFailure::NoSuchFoldLine => "その折り線が無い",
            StepFailure::AlreadyFolded => "もう折り終えている",
            StepFailure::NotFoldable(reason) => reason.label(),
            StepFailure::LayerOrderBroken => "紙の重なり順が保てない",
        }
    }
}

/// どこで落ちたか。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerifyFailure {
    /// 何手目で落ちたか(0から数える)。
    pub index: usize,
    /// 落ちた手の折り線の番号。
    pub id: usize,
    /// どの確認で落ちたか。
    pub cause: StepFailure,
}

impl VerifyFailure {
    /// 「3手目(折り線7): 紙がすり抜ける」の形で1行にする。
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{}手目(折り線{}): {}",
            self.index + 1,
            self.id,
            self.cause.label()
        )
    }
}

/// 手順を全部折り終えた、最後の形の測り値。
///
/// 手順が途中で落ちた場合は、**そこまで折れたところの形**を測る。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinalCheck {
    /// 折り上がり(`t = 1`)で見えている面の数。
    pub faces: usize,
    /// 展開図から取り出せる面の数。`faces` と食い違えば面が欠けている。
    pub expected_faces: usize,
    /// 座標がすべて有限か。
    pub finite: bool,
    /// 再生が飛ばした手順の数。
    pub skipped: usize,
    /// 再生が出した警告の数(紙の重なり順の食い違いを含む)。
    pub warnings: usize,
    /// 最後の形の裂けの量。
    pub max_seam_gap: f64,
    /// 最後の形のすり抜けの件数。
    pub penetrations: usize,
}

impl FinalCheck {
    /// 最後の形が、6つの項目すべてで問題ないか。
    ///
    /// 面が欠けていない・座標がすべて有限・飛ばした手順0・警告0・
    /// 裂けが [`MAX_SEAM_GAP`] 未満・すり抜け0。
    #[must_use]
    pub fn is_sound(&self) -> bool {
        self.faces == self.expected_faces
            && self.finite
            && self.skipped == 0
            && self.warnings == 0
            && self.max_seam_gap < MAX_SEAM_GAP
            && self.penetrations == 0
    }
}

/// 手順の全体を通して検証した結果。
#[derive(Clone, Debug, PartialEq)]
pub struct VerifyReport {
    /// 渡された手の数。
    pub requested: usize,
    /// 通った手(先頭から順に)。落ちた手はここに入らない。
    pub steps: Vec<StepCheck>,
    /// どこで落ちたか。最後まで通れば [`None`]。
    pub failure: Option<VerifyFailure>,
    /// 通った手すべてを通した、裂けの最大量(最後の形も含む)。
    pub max_seam_gap: f64,
    /// 通った手すべてを通した、すり抜けの最大件数(最後の形も含む)。
    pub penetrations: usize,
    /// 見た途中の姿勢の数の合計(最後の形の1点を含む)。
    pub poses_checked: usize,
    /// 最後の形の測り値。
    pub final_check: FinalCheck,
    /// 折り始める前の4つの物差し。
    pub start_gaps: FinishGaps,
    /// 折り始める前の点数([`GapWeights::score`])。
    pub start_score: f64,
    /// 最後の形の4つの物差し。
    pub final_gaps: FinishGaps,
    /// 最後の形の点数。
    pub final_score: f64,
}

impl VerifyReport {
    /// 手順が最初から最後まで通り、最後の形も問題なかったか。
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failure.is_none() && self.requested == self.steps.len() && self.final_check.is_sound()
    }

    /// 通った手の数。
    #[must_use]
    pub fn cleared(&self) -> usize {
        self.steps.len()
    }

    /// 報告用の1行。落ちた場合は落ちた場所も入る。
    #[must_use]
    pub fn describe(&self) -> String {
        let head = format!(
            "{}手中{}手 / 裂け{:.3e} / すり抜け{} / 姿勢{}点",
            self.requested,
            self.cleared(),
            self.max_seam_gap,
            self.penetrations,
            self.poses_checked
        );
        match &self.failure {
            Some(f) => format!("{head} / 落ちた場所: {}", f.describe()),
            None => format!("{head} / 最後まで通った"),
        }
    }
}

/// 完成形まで確認できた探索手順。
///
/// フィールドを公開せず、[`verify_search_completion`] だけが作る。これにより、
/// 状態数や深さの上限で打ち切った探索を、呼び出し側が誤って「完成まで確認済み」へ
/// 組み替えることはできない。
#[derive(Clone, Debug, PartialEq)]
pub struct CheckedToFinish {
    report: VerifyReport,
}

impl CheckedToFinish {
    /// 最初から完成形まで通して確かめた報告。
    #[must_use]
    pub fn report(&self) -> &VerifyReport {
        &self.report
    }

    /// 確認結果を所有値として取り出す。
    #[must_use]
    pub fn into_report(self) -> VerifyReport {
        self.report
    }
}

/// 探索の打ち切りまでに確認できた参考手順。
///
/// この型から [`CheckedToFinish`] への変換は用意しない。探索をやり直して
/// [`verify_search_completion`] の完成条件をすべて満たした場合だけ、完成手順になる。
#[derive(Clone, Debug, PartialEq)]
pub struct PartialPlan {
    report: VerifyReport,
    stop: SearchStop,
}

impl PartialPlan {
    /// 打ち切りまでに見つかった手順を、最初から確かめ直した報告。
    #[must_use]
    pub fn report(&self) -> &VerifyReport {
        &self.report
    }

    /// 探索が止まった理由。
    ///
    /// `GoalReached` でも、再検証または最後の4指標が完成条件を満たさなければ
    /// [`VerifiedPlan::Partial`] になる。
    #[must_use]
    pub fn stop(&self) -> SearchStop {
        self.stop
    }

    /// 確認結果を所有値として取り出す。
    #[must_use]
    pub fn into_report(self) -> VerifyReport {
        self.report
    }
}

/// 探索手順を最初から確認し直した結果。
///
/// `CheckedToFinish` と `Partial` を別variantにすることで、途中までの参考手順と
/// 完成まで確認できた手順を同じ値として扱えないようにする。watchdog/cancelは
/// [`crate::SearchAbort`] であり、このenumへ入るvariantを持たない。
#[derive(Clone, Debug, PartialEq)]
pub enum VerifiedPlan {
    /// 探索・全手順の再検証・完成形の4指標がすべて完成条件を満たした。
    CheckedToFinish(CheckedToFinish),
    /// 完成条件のどれかを満たさず、確認できたところまでを参考として返す。
    Partial(PartialPlan),
}

impl VerifiedPlan {
    /// どちらの状態でも、確かめ直した手順の報告を返す。
    #[must_use]
    pub fn report(&self) -> &VerifyReport {
        match self {
            Self::CheckedToFinish(checked) => checked.report(),
            Self::Partial(partial) => partial.report(),
        }
    }
}

/// 手順を最初から順に全部折り直し、各手のあいだと最後の形を確かめる。
///
/// ## 渡すもの
///
/// - `session`: **まだその手順を折っていない**折りかけの作品。ふつうは
///   [`FoldSession::new`] で作った平らな紙。複製して使うので、渡した側は動かない。
/// - `order`: 折り線の番号を折る順に並べたもの。
///   [`SearchOutcome::steps`] から取るなら [`verify_search_outcome`] が使える。
/// - `goal`: 完成の目標。最後の形を4つの物差しで測るのに使う。
/// - `weights`: 4つを1つの点数へまとめる重み。
/// - `scan`: 各手で見る途中の姿勢の数。作業21・22と同じ [`PoseScan::DEFAULT`](21点)を推す。
///
/// ## 止まらない
///
/// 途中で落ちても panic せず、[`VerifyReport::failure`] に
/// **何手目の・どの折り線で・どの確認で**落ちたかを入れて返す。
/// そこまで通った手は [`VerifyReport::steps`] に残り、
/// 最後の形はそこまで折れた形として測る。
///
/// ## 同じ入力なら同じ結果
///
/// 乱数を使わず、結果に実行ごとに変わる文章を入れていないので、
/// 何度実行しても [`VerifyReport`] は完全に一致する。
#[must_use]
pub fn verify_fold_order(
    session: &FoldSession,
    order: &[usize],
    goal: &FoldGoal,
    weights: GapWeights,
    scan: PoseScan,
) -> VerifyReport {
    let start_gaps = finish_gaps(&goal.target, &goal.measure(session.document()));
    let start_score = goal.score(session.document(), &start_gaps, weights);

    let mut walk = session.clone();
    let mut steps: Vec<StepCheck> = Vec::new();
    let mut failure: Option<VerifyFailure> = None;
    let mut worst_gap: f64 = 0.0;
    let mut worst_pairs = 0usize;
    let mut poses_checked = 0usize;

    for (index, &id) in order.iter().enumerate() {
        let cause = match walk.check_move(id, scan) {
            Some(Ok(mv)) => {
                let movement = mv.movement();
                worst_gap = worst_gap.max(movement.max_seam_gap);
                worst_pairs = worst_pairs.max(movement.penetrations);
                poses_checked += movement.poses_checked;
                match walk.apply(&mv) {
                    // 折った後の紙の重なり順を**測り直して**結果に載せる
                    // (作業23が返すことになっている「層矛盾」の項目)。
                    Ok(()) => match layer_warnings(&walk) {
                        Some(0) => {
                            steps.push(StepCheck::from_move(index, movement, 0));
                            continue;
                        }
                        _ => StepFailure::LayerOrderBroken,
                    },
                    // 折れると確かめた直後なので、ここへ来るのは
                    // 折った後の紙の重なり順が保てない場合だけである。
                    Err(_) => StepFailure::LayerOrderBroken,
                }
            }
            Some(Err(reason)) => StepFailure::NotFoldable(reason),
            None if walk.has_fold_line(id) => StepFailure::AlreadyFolded,
            None => StepFailure::NoSuchFoldLine,
        };
        failure = Some(VerifyFailure { index, id, cause });
        break;
    }

    let final_check = check_final_form(&walk);
    worst_gap = worst_gap.max(final_check.max_seam_gap);
    worst_pairs = worst_pairs.max(final_check.penetrations);
    poses_checked += 1;

    let final_gaps = finish_gaps(&goal.target, &goal.measure(walk.document()));
    VerifyReport {
        requested: order.len(),
        steps,
        failure,
        max_seam_gap: worst_gap,
        penetrations: worst_pairs,
        poses_checked,
        final_check,
        start_gaps,
        start_score,
        final_gaps,
        final_score: goal.score(walk.document(), &final_gaps, weights),
    }
}

/// 作業22の探索が返した手順を、そのまま全体で検証する。
///
/// 探索は速さのために粗い確認で候補を集める([`crate::SearchBudget::rank_scan`])。
/// ここでは返ってきた手順を**改めて最初から**、`scan` の細かさで折り直す。
#[must_use]
pub fn verify_search_outcome(
    session: &FoldSession,
    outcome: &SearchOutcome,
    goal: &FoldGoal,
    weights: GapWeights,
    scan: PoseScan,
) -> VerifyReport {
    let order: Vec<usize> = outcome.steps.iter().map(|s| s.mv.id).collect();
    verify_fold_order(session, &order, goal, weights, scan)
}

/// 完成許容値を使った探索結果を最初から確かめ直し、完成手順と参考手順を型で分ける。
///
/// [`VerifiedPlan::CheckedToFinish`] を返すのは、次の3条件を**すべて**満たす場合だけ。
///
/// 1. 探索自身が [`SearchStop::GoalReached`] で止まった。
/// 2. 返された全手順を指定の姿勢数で折り直し、[`VerifyReport::passed`] が真になった。
/// 3. 折り直した最後の形も、4指標すべてが `tolerance` 以内だった。
///
/// 探索時と再検証時では姿勢の粗さが違い得るため、探索時の `best_gaps` だけでは
/// 完成扱いしない。状態数・深さで打ち切った結果や、再検証で落ちた結果は、
/// たとえ途中までの手が安全でも必ず [`VerifiedPlan::Partial`] になる。
/// watchdog/cancelはcontrolled探索の`Err`であり、この関数が受け取る
/// [`SearchOutcome`] 自体が無いので、どちらのvariantにも到達しない。
#[must_use]
pub fn verify_search_completion(
    session: &FoldSession,
    outcome: &SearchOutcome,
    goal: &FoldGoal,
    weights: GapWeights,
    scan: PoseScan,
    tolerance: CompletionTolerance,
) -> VerifiedPlan {
    let report = verify_search_outcome(session, outcome, goal, weights, scan);
    if completion_confirmed(outcome.stop, &report, tolerance) {
        VerifiedPlan::CheckedToFinish(CheckedToFinish { report })
    } else {
        VerifiedPlan::Partial(PartialPlan {
            report,
            stop: outcome.stop,
        })
    }
}

fn completion_confirmed(
    stop: SearchStop,
    report: &VerifyReport,
    tolerance: CompletionTolerance,
) -> bool {
    stop == SearchStop::GoalReached && report.passed() && tolerance.contains(&report.final_gaps)
}

/// ここまで折った形の、紙の重なり順の食い違いの件数。
///
/// 求め直せなかった場合は [`None`]。折る途中の姿勢([`replay`])とは別の道
/// ([`flat_state_at`])で重なり順を求め直すので、姿勢の確認とは別の確認になる。
fn layer_warnings(session: &FoldSession) -> Option<usize> {
    let document = session.document();
    let faces = extract_faces(&document.cp);
    flat_state_at(document, &faces, document.sequence.len())
        .ok()
        .map(|(_, warnings)| warnings.len())
}

/// 折り上がり(`t = 1`)の形を測る。
///
/// 折っている途中の姿勢は各手の確認がすでに見ているので、ここは
/// **手順を全部折り終えた形**の1点だけを見る。
fn check_final_form(session: &FoldSession) -> FinalCheck {
    let document = session.document();
    let faces = extract_faces(&document.cp);
    let replayed = replay(document, document.sequence.len(), 1.0);
    let frame = replayed.frame;
    let finite = frame
        .faces
        .iter()
        .all(|f| f.polygon.iter().flatten().all(|v| v.is_finite()));
    let gap = if finite {
        max_seam_gap(&document.cp, &faces, &frame)
    } else {
        f64::INFINITY
    };
    FinalCheck {
        faces: frame.faces.len(),
        expected_faces: faces.len(),
        finite: finite && gap.is_finite(),
        skipped: replayed.skipped.len(),
        warnings: replayed.warnings.len(),
        max_seam_gap: gap,
        penetrations: self_intersection_pairs(&frame).len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_report() -> VerifyReport {
        VerifyReport {
            requested: 0,
            steps: Vec::new(),
            failure: None,
            max_seam_gap: 0.0,
            penetrations: 0,
            poses_checked: 1,
            final_check: FinalCheck {
                faces: 1,
                expected_faces: 1,
                finite: true,
                skipped: 0,
                warnings: 0,
                max_seam_gap: 0.0,
                penetrations: 0,
            },
            start_gaps: FinishGaps::BEST,
            start_score: 0.0,
            final_gaps: FinishGaps::BEST,
            final_score: 0.0,
        }
    }

    #[test]
    fn only_a_reached_reverified_shape_can_be_checked_to_finish() {
        let tolerance = CompletionTolerance::DEFAULT;
        let passed = passing_report();
        assert!(completion_confirmed(
            SearchStop::GoalReached,
            &passed,
            tolerance
        ));

        for stop in [
            SearchStop::Exhausted,
            SearchStop::StateCap,
            SearchStop::DepthCap,
        ] {
            assert!(
                !completion_confirmed(stop, &passed, tolerance),
                "打ち切り理由{stop:?}が完成扱いになった"
            );
        }

        let mut unsafe_report = passed.clone();
        unsafe_report.final_check.warnings = 1;
        assert!(!completion_confirmed(
            SearchStop::GoalReached,
            &unsafe_report,
            tolerance
        ));

        let mut outside_tolerance = passed;
        outside_tolerance.final_gaps.length = tolerance.length * 2.0;
        assert!(!completion_confirmed(
            SearchStop::GoalReached,
            &outside_tolerance,
            tolerance
        ));
    }
}
