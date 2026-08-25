//! 作業30「端から端までの受け入れ」の恒久検査。
//!
//! 作業24で固定した4標本を、検査内で組み立てて提案候補まで通す。完成確認は
//! 要件の名前付き標本「頭1・尾1・足4」を、製品と同じ目標・材料点対応で行う。

use ori3_model::{AlignmentTarget, CreasePattern, Document, FoldStep, Paper};
use ori3_propose::{
    CompletionTolerance, FinishGaps, FinishTarget, FoldGoal, FoldSession, GapWeights, Packing,
    PoseScan, ProposalResult, SearchBudget, SearchOutcome, SearchStop, Skeleton, SkeletonNode,
    TipSite, VerifiedPlan, VerifyReport, body_on_paper, generate, pack, search_to_completion,
    verify_search_completion,
};

const PAPER: Paper = Paper {
    width_mm: 100.0,
    height_mm: 100.0,
};
const RUNS: usize = 10;

/// `commands.rs::plan_folds` が画面の4候補に使う製品上限。
///
/// 4〜7葉×4候補の実測で、`2/2`は`1/3`より手を残し、`4/3`の半分以下の時間で
/// 計算できたため採用されている。完成の許容値や走査点は変更しない。
/// この複製はcoreの数値を測るためのもの。production配線の権威は、実commandを
/// 呼ぶstore側の受け入れ検査であり、この定数だけで配線済みとは判定しない。
const PRODUCT_PLAN_BUDGET: SearchBudget = SearchBudget {
    max_states: 2,
    branch: 2,
    max_depth: SearchBudget::DEFAULT.max_depth,
    rank_scan: SearchBudget::DEFAULT.rank_scan,
    scan: SearchBudget::DEFAULT.scan,
};

/// 決定性比較の小数許容差。
///
/// 既存3標本の記録照合許容`MEASUREMENT_TOL = 1e-9`と同じ値を使う。同検査が
/// 根拠にした許容境界の最小余裕`0.062132...`より7桁以上細い。名前付き候補一般の
/// 最小差や反復揺れとは解釈せず、実測小数そのものを境目にしない。
/// ID・種類・件数・順序は完全一致で比べる。
const FLOAT_TOL: f64 = 1e-9;

fn star(leaves: u32) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    nodes.extend((1..=leaves).map(|id| SkeletonNode::new(id, Some(0), 1.0)));
    Skeleton { nodes }
}

/// 要件の名前付き標本。0=胴、1=頭、2=尾、3..=6=足。
fn head_tail_four_legs() -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    nodes.push(SkeletonNode::new(1, Some(0), 1.0));
    nodes.push(SkeletonNode::new(2, Some(0), 1.0));
    for id in 3..=6 {
        nodes.push(SkeletonNode::new(id, Some(0), 0.7));
    }
    Skeleton { nodes }
}

/// 0=胴、1=途中、その先に3・6・7が分岐する作業24の深さ3分岐標本。
fn depth_three_branch() -> Skeleton {
    Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(1, Some(0), 0.3),
            SkeletonNode::new(2, Some(0), 1.0),
            SkeletonNode::new(3, Some(1), 1.0),
            SkeletonNode::new(4, Some(0), 0.6),
            SkeletonNode::new(5, Some(0), 0.6),
            SkeletonNode::new(6, Some(1), 0.6),
            SkeletonNode::new(7, Some(1), 0.6),
        ],
    }
}

fn four_corners_packing() -> Packing {
    Packing {
        scale: 0.5,
        centers: vec![
            (1, [0.0, 0.0]),
            (2, [1.0, 0.0]),
            (3, [1.0, 1.0]),
            (4, [0.0, 1.0]),
        ],
        violation: 0.0,
        circles: Vec::new(),
    }
}

fn first_generated_candidate(name: &str, skeleton: &Skeleton) -> (Packing, ProposalResult) {
    let packings = pack(skeleton, 1.0, 1.0, 2026, 8);
    assert!(!packings.is_empty(), "{name}: 配置候補が0件");
    packings
        .into_iter()
        .find_map(|packing| {
            generate(skeleton, &packing, 1.0, 1.0)
                .ok()
                .map(|proposal| (packing, proposal))
        })
        .unwrap_or_else(|| panic!("{name}: 展開図候補を1件も生成できない"))
}

fn assert_proposal_candidate(name: &str, skeleton: &Skeleton, proposal: &ProposalResult) {
    assert!(proposal.cp.edges.len() > 4, "{name}: 輪郭以外の辺がない");
    assert_eq!(
        proposal.sites.len(),
        skeleton.leaves().len(),
        "{name}: 先端と材料点の対応が欠けた"
    );
    assert!(
        proposal.sites.iter().all(|site| site.vertex.is_some()),
        "{name}: 材料点を持たない先端がある"
    );
}

/// 作業24から引き継いだ4標本すべてで、配置から展開図候補まで生成する。
#[test]
fn all_four_fixed_samples_generate_a_candidate() {
    let two = star(2);
    let four = star(4);
    let named = head_tail_four_legs();
    let deep = depth_three_branch();
    let mut generated = 0usize;

    for (name, skeleton) in [
        ("2葉", &two),
        ("頭1・尾1・足4", &named),
        ("深さ3分岐", &deep),
    ] {
        skeleton
            .validate()
            .unwrap_or_else(|error| panic!("{name}: 骨格が不正: {error}"));
        let (_, proposal) = first_generated_candidate(name, skeleton);
        assert_proposal_candidate(name, skeleton, &proposal);
        generated += 1;
    }

    four.validate().expect("四隅4葉の骨格が不正");
    let proposal = generate(&four, &four_corners_packing(), 1.0, 1.0)
        .expect("四隅4葉の展開図候補を生成できない");
    assert_proposal_candidate("四隅4葉", &four, &proposal);
    generated += 1;

    assert_eq!(generated, 4, "4標本すべてを通していない");
    println!("WORK30_CANDIDATES generated={generated}/4");
}

struct NamedRun {
    packing: Packing,
    proposal: ProposalResult,
    outcome: SearchOutcome,
    report: VerifyReport,
    status: ProductPlanStatus,
    rebuilt_steps: usize,
    rebuilt_cp: CreasePattern,
    rebuilt_sequence: Vec<FoldStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProductPlanStatus {
    CheckedToFinish,
    Partial,
    NoPlan,
}

fn run_named_candidate(
    skeleton: &Skeleton,
    packing: Packing,
    proposal: ProposalResult,
) -> NamedRun {
    let mut document = Document::new(PAPER);
    document.cp = proposal.cp.clone();
    let session = FoldSession::new(&document).expect("生成候補から折り始められない");
    let goal = FoldGoal {
        target: FinishTarget::from_skeleton(skeleton),
        body: body_on_paper(skeleton, &packing, 1.0, 1.0),
        sites: proposal
            .sites
            .iter()
            .map(|site| TipSite {
                leaf_id: site.circle.leaf_id,
                material: site.vertex.map_or(site.circle.center, |vertex| vertex.pos),
            })
            .collect(),
    };
    let outcome = search_to_completion(
        &session,
        &goal,
        GapWeights::DEFAULT,
        PRODUCT_PLAN_BUDGET,
        CompletionTolerance::DEFAULT,
    );
    let verified = verify_search_completion(
        &session,
        &outcome,
        &goal,
        GapWeights::DEFAULT,
        PoseScan::DEFAULT,
        CompletionTolerance::DEFAULT,
    );
    let verified_to_finish = matches!(&verified, VerifiedPlan::CheckedToFinish(_));
    let report = verified.report().clone();
    // `commands.rs::plan_folds` と同じく、21姿勢で通った手を1点でたどり直し、
    // 実際に作品へ入る`FoldStep`を作る。0手なら製品境界は`fold_plan: None`を返す。
    let mut walk = session.clone();
    for step in &report.steps {
        let Some(Ok(mv)) = walk.check_move(step.id, PoseScan { steps: 0 }) else {
            break;
        };
        if walk.apply(&mv).is_err() {
            break;
        }
    }
    let folded = walk.document();
    let rebuilt_steps = folded.sequence.len();
    let rebuilt_cp = folded.cp.clone();
    let rebuilt_sequence = folded.sequence.clone();
    let status = if rebuilt_steps == 0 {
        ProductPlanStatus::NoPlan
    } else if verified_to_finish
        && rebuilt_steps == report.requested
        && rebuilt_steps == report.cleared()
    {
        ProductPlanStatus::CheckedToFinish
    } else {
        ProductPlanStatus::Partial
    };
    NamedRun {
        packing,
        proposal,
        outcome,
        report,
        status,
        rebuilt_steps,
        rebuilt_cp,
        rebuilt_sequence,
    }
}

fn run_named_candidates() -> Vec<NamedRun> {
    let skeleton = head_tail_four_legs();
    let packings = pack(&skeleton, 1.0, 1.0, 2026, 8);
    assert!(!packings.is_empty(), "頭1・尾1・足4: 配置候補が0件");
    let runs: Vec<_> = packings
        .into_iter()
        .filter_map(|packing| {
            generate(&skeleton, &packing, 1.0, 1.0)
                .ok()
                .map(|proposal| run_named_candidate(&skeleton, packing, proposal))
        })
        .collect();
    assert!(!runs.is_empty(), "頭1・尾1・足4: 展開図候補が0件");
    runs
}

fn assert_near(label: &str, got: f64, want: f64) -> f64 {
    let delta = (got - want).abs();
    assert!(
        delta <= FLOAT_TOL,
        "{label}: |{got} - {want}|={delta} が許容差{FLOAT_TOL}を超えた"
    );
    delta
}

fn assert_gaps_near(label: &str, got: FinishGaps, want: FinishGaps) -> f64 {
    let mut max_delta = 0.0_f64;
    for (measure, got, want) in [
        ("数", got.count, want.count),
        ("長さ", got.length, want.length),
        ("太さ", got.width, want.width),
        ("位置", got.position, want.position),
    ] {
        max_delta = max_delta.max(assert_near(&format!("{label}/{measure}"), got, want));
    }
    max_delta
}

fn assert_cp_near(label: &str, got: &CreasePattern, want: &CreasePattern) -> f64 {
    assert_eq!(got.next_vertex_id, want.next_vertex_id);
    assert_eq!(got.next_edge_id, want.next_edge_id);
    assert_eq!(got.vertices.len(), want.vertices.len());
    assert_eq!(got.edges, want.edges, "{label}: 辺が一致しない");
    let mut max_delta = 0.0_f64;
    for (got, want) in got.vertices.iter().zip(&want.vertices) {
        assert_eq!(got.id, want.id, "{label}: 頂点IDが一致しない");
        for axis in 0..2 {
            max_delta = max_delta.max(assert_near(
                &format!("{label}/頂点{}[{axis}]", got.id),
                got.pos[axis],
                want.pos[axis],
            ));
        }
    }
    max_delta
}

fn assert_fold_steps_near(label: &str, got: &[FoldStep], want: &[FoldStep]) -> f64 {
    assert_eq!(got.len(), want.len(), "{label}: 手数が一致しない");
    let mut max_delta = 0.0_f64;
    for (index, (got, want)) in got.iter().zip(want).enumerate() {
        assert_eq!(got.id, want.id, "{label}: {}手目のIDが違う", index + 1);
        assert_eq!(
            got.kind,
            want.kind,
            "{label}: {}手目の技法が違う",
            index + 1
        );
        assert_eq!(
            got.note,
            want.note,
            "{label}: {}手目の注記が違う",
            index + 1
        );
        assert_eq!(got.drivers.len(), want.drivers.len());
        for (driver_index, (got, want)) in got.drivers.iter().zip(&want.drivers).enumerate() {
            for point in [("a", got.a, want.a), ("b", got.b, want.b)] {
                for axis in 0..2 {
                    max_delta = max_delta.max(assert_near(
                        &format!(
                            "{label}/{}手目/driver{driver_index}/{}[{axis}]",
                            index + 1,
                            point.0
                        ),
                        point.1[axis],
                        point.2[axis],
                    ));
                }
            }
            max_delta = max_delta.max(assert_near(
                &format!("{label}/{}手目/driver{driver_index}/角度", index + 1),
                got.target_angle_deg,
                want.target_angle_deg,
            ));
        }

        match (&got.layer_order, &want.layer_order) {
            (Some(got), Some(want)) => {
                assert_eq!(got.len(), want.len(), "{label}: 層順の面数が違う");
                for (face_index, (got, want)) in got.iter().zip(want).enumerate() {
                    for axis in 0..2 {
                        max_delta = max_delta.max(assert_near(
                            &format!("{label}/{}手目/層順{face_index}[{axis}]", index + 1),
                            got[axis],
                            want[axis],
                        ));
                    }
                }
            }
            (None, None) => {}
            _ => panic!("{label}: {}手目の層順有無が違う", index + 1),
        }

        match (&got.alignment, &want.alignment) {
            (Some(got), Some(want)) => {
                assert_eq!(got.mode, want.mode, "{label}: 合わせ方が違う");
                assert_eq!(got.picks.len(), want.picks.len());
                for (pick_index, (got, want)) in got.picks.iter().zip(&want.picks).enumerate() {
                    match (got, want) {
                        (AlignmentTarget::Point { p: got }, AlignmentTarget::Point { p: want }) => {
                            for axis in 0..2 {
                                max_delta = max_delta.max(assert_near(
                                    &format!(
                                        "{label}/{}手目/合わせ点{pick_index}[{axis}]",
                                        index + 1
                                    ),
                                    got[axis],
                                    want[axis],
                                ));
                            }
                        }
                        (
                            AlignmentTarget::Line { a: got_a, b: got_b },
                            AlignmentTarget::Line {
                                a: want_a,
                                b: want_b,
                            },
                        ) => {
                            for (point, got, want) in [("a", got_a, want_a), ("b", got_b, want_b)] {
                                for axis in 0..2 {
                                    max_delta = max_delta.max(assert_near(
                                        &format!(
                                            "{label}/{}手目/合わせ線{pick_index}/{point}[{axis}]",
                                            index + 1
                                        ),
                                        got[axis],
                                        want[axis],
                                    ));
                                }
                            }
                        }
                        _ => panic!("{label}: 合わせ先{pick_index}の種類が違う"),
                    }
                }
            }
            (None, None) => {}
            _ => panic!("{label}: {}手目の合わせ折り有無が違う", index + 1),
        }

        match (got.finish_soft, want.finish_soft) {
            (Some(got), Some(want)) => {
                assert_eq!(got.enabled, want.enabled, "{label}: たわみ有無が違う");
                max_delta = max_delta.max(assert_near(
                    &format!("{label}/{}手目/硬さ", index + 1),
                    got.stiffness,
                    want.stiffness,
                ));
                max_delta = max_delta.max(assert_near(
                    &format!("{label}/{}手目/圧力", index + 1),
                    got.pressure,
                    want.pressure,
                ));
            }
            (None, None) => {}
            _ => panic!("{label}: {}手目のたわみ有無が違う", index + 1),
        }
    }
    max_delta
}

fn assert_candidate_near(got: &NamedRun, want: &NamedRun) -> f64 {
    let mut max_delta = assert_near("配置縮尺", got.packing.scale, want.packing.scale);
    max_delta = max_delta.max(assert_near(
        "配置違反",
        got.packing.violation,
        want.packing.violation,
    ));
    assert_eq!(got.packing.centers.len(), want.packing.centers.len());
    for (index, (got, want)) in got
        .packing
        .centers
        .iter()
        .zip(&want.packing.centers)
        .enumerate()
    {
        assert_eq!(got.0, want.0, "配置{index}の葉IDが違う");
        for axis in 0..2 {
            max_delta = max_delta.max(assert_near(
                &format!("配置{index}[{axis}]"),
                got.1[axis],
                want.1[axis],
            ));
        }
    }

    assert_eq!(got.packing.circles.len(), want.packing.circles.len());
    for (index, (got, want)) in got
        .packing
        .circles
        .iter()
        .zip(&want.packing.circles)
        .enumerate()
    {
        assert_eq!(got.leaf_id, want.leaf_id, "円{index}の葉IDが違う");
        assert_eq!(got.circle_index, want.circle_index, "円{index}の番号が違う");
        for axis in 0..2 {
            max_delta = max_delta.max(assert_near(
                &format!("円{index}の中心[{axis}]"),
                got.center[axis],
                want.center[axis],
            ));
        }
        max_delta = max_delta.max(assert_near(
            &format!("円{index}の半径"),
            got.radius,
            want.radius,
        ));
    }

    assert_eq!(got.proposal.violations, want.proposal.violations);
    assert_eq!(got.proposal.warnings, want.proposal.warnings);
    assert_eq!(got.proposal.sites.len(), want.proposal.sites.len());
    for (index, (got, want)) in got
        .proposal
        .sites
        .iter()
        .zip(&want.proposal.sites)
        .enumerate()
    {
        assert_eq!(got.circle.leaf_id, want.circle.leaf_id);
        assert_eq!(got.circle.circle_index, want.circle.circle_index);
        assert_eq!(got.molecules, want.molecules, "対応{index}の分子順が違う");
        for axis in 0..2 {
            max_delta = max_delta.max(assert_near(
                &format!("対応{index}の円中心[{axis}]"),
                got.circle.center[axis],
                want.circle.center[axis],
            ));
        }
        max_delta = max_delta.max(assert_near(
            &format!("対応{index}の円半径"),
            got.circle.radius,
            want.circle.radius,
        ));
        match (got.vertex, want.vertex) {
            (Some(got), Some(want)) => {
                assert_eq!(got.id, want.id, "対応{index}の材料点IDが違う");
                max_delta = max_delta.max(assert_near(
                    &format!("対応{index}の材料点差"),
                    got.gap,
                    want.gap,
                ));
                for axis in 0..2 {
                    max_delta = max_delta.max(assert_near(
                        &format!("対応{index}の材料点[{axis}]"),
                        got.pos[axis],
                        want.pos[axis],
                    ));
                }
            }
            (None, None) => {}
            _ => panic!("対応{index}の材料点有無が違う"),
        }
    }

    max_delta = max_delta.max(assert_cp_near(
        "提案展開図",
        &got.proposal.cp,
        &want.proposal.cp,
    ));
    max_delta
}

fn assert_outcome_near(got: &SearchOutcome, want: &SearchOutcome) -> f64 {
    assert_eq!(got.stop, want.stop, "停止理由が一致しない");
    assert_eq!(got.states_expanded, want.states_expanded);
    assert_eq!(got.states_generated, want.states_generated);
    assert_eq!(got.max_branching, want.max_branching);
    assert_eq!(got.depth_capped, want.depth_capped);
    assert_eq!(got.steps.len(), want.steps.len());
    let mut max_delta = assert_gaps_near("開始", got.start_gaps, want.start_gaps);
    max_delta = max_delta.max(assert_gaps_near("終点", got.best_gaps, want.best_gaps));
    max_delta = max_delta.max(assert_near("開始点数", got.start_score, want.start_score));
    max_delta = max_delta.max(assert_near("終点点数", got.best_score, want.best_score));
    for (index, (got, want)) in got.steps.iter().zip(&want.steps).enumerate() {
        assert_eq!(got.mv.id, want.mv.id, "{}手目のIDが違う", index + 1);
        assert_eq!(got.mv.closes, want.mv.closes);
        assert_eq!(got.mv.mask, want.mv.mask);
        assert_eq!(got.mv.penetrations, want.mv.penetrations);
        assert_eq!(got.mv.poses_checked, want.mv.poses_checked);
        for point in 0..2 {
            for axis in 0..2 {
                max_delta = max_delta.max(assert_near(
                    &format!("{}手目の線[{point}][{axis}]", index + 1),
                    got.mv.line[point][axis],
                    want.mv.line[point][axis],
                ));
            }
        }
        max_delta = max_delta.max(assert_near(
            &format!("{}手目の裂け", index + 1),
            got.mv.max_seam_gap,
            want.mv.max_seam_gap,
        ));
        max_delta = max_delta.max(assert_gaps_near(
            &format!("{}手目", index + 1),
            got.gaps,
            want.gaps,
        ));
        max_delta = max_delta.max(assert_near(
            &format!("{}手目の点数", index + 1),
            got.score,
            want.score,
        ));
    }
    max_delta
}

fn assert_report_near(got: &VerifyReport, want: &VerifyReport) -> f64 {
    assert_eq!(got.requested, want.requested);
    assert_eq!(got.cleared(), want.cleared());
    assert_eq!(got.failure, want.failure);
    assert_eq!(got.penetrations, want.penetrations);
    assert_eq!(got.poses_checked, want.poses_checked);
    assert_eq!(got.final_check.faces, want.final_check.faces);
    assert_eq!(
        got.final_check.expected_faces,
        want.final_check.expected_faces
    );
    assert_eq!(got.final_check.finite, want.final_check.finite);
    assert_eq!(got.final_check.skipped, want.final_check.skipped);
    assert_eq!(got.final_check.warnings, want.final_check.warnings);
    assert_eq!(got.final_check.penetrations, want.final_check.penetrations);
    let mut max_delta = assert_near("再検証の最大裂け", got.max_seam_gap, want.max_seam_gap);
    max_delta = max_delta.max(assert_near(
        "再検証終点の裂け",
        got.final_check.max_seam_gap,
        want.final_check.max_seam_gap,
    ));
    max_delta = max_delta.max(assert_gaps_near(
        "再検証開始",
        got.start_gaps,
        want.start_gaps,
    ));
    max_delta = max_delta.max(assert_gaps_near(
        "再検証終点",
        got.final_gaps,
        want.final_gaps,
    ));
    max_delta = max_delta.max(assert_near(
        "再検証開始点数",
        got.start_score,
        want.start_score,
    ));
    max_delta = max_delta.max(assert_near(
        "再検証終点点数",
        got.final_score,
        want.final_score,
    ));
    for (got, want) in got.steps.iter().zip(&want.steps) {
        assert_eq!(got.index, want.index);
        assert_eq!(got.id, want.id);
        assert_eq!(got.penetrations, want.penetrations);
        assert_eq!(got.poses_checked, want.poses_checked);
        assert_eq!(got.layer_warnings, want.layer_warnings);
        max_delta = max_delta.max(assert_near(
            &format!("再検証{}手目の裂け", got.index + 1),
            got.max_seam_gap,
            want.max_seam_gap,
        ));
        for point in 0..2 {
            for axis in 0..2 {
                max_delta = max_delta.max(assert_near(
                    &format!("再検証{}手目の線[{point}][{axis}]", got.index + 1),
                    got.line[point][axis],
                    want.line[point][axis],
                ));
            }
        }
    }
    max_delta
}

fn assert_named_completion(run: &NamedRun) {
    assert_eq!(run.outcome.stop, SearchStop::GoalReached);
    assert!(!run.outcome.steps.is_empty(), "名前付き標本の手数が0");
    assert!(
        run.report.passed(),
        "全手順の再検証に失敗: {:?}",
        run.report
    );
    assert_eq!(
        run.report.cleared(),
        run.report.requested,
        "手順消化率が100%でない"
    );
    assert_eq!(run.report.final_check.skipped, 0, "飛ばした手順がある");
    assert_eq!(
        run.rebuilt_steps, run.report.requested,
        "製品境界で手順を全件組み直せなかった"
    );

    let tolerance = CompletionTolerance::DEFAULT;
    let gaps = run.report.final_gaps;
    let within = [
        gaps.count <= tolerance.count,
        gaps.length <= tolerance.length,
        gaps.width <= tolerance.width,
        gaps.position <= tolerance.position,
    ];
    assert_eq!(
        within.into_iter().filter(|inside| *inside).count(),
        4,
        "4つの完成誤差が許容内でない: {gaps:?} / {tolerance:?}"
    );
}

/// 名前付き標本を入力から完成確認まで10回独立に計算する。
#[test]
fn named_sample_completes_end_to_end_and_is_deterministic_ten_out_of_ten() {
    let runs: Vec<_> = (0..RUNS).map(|_| run_named_candidates()).collect();
    let candidate_count = runs[0].len();
    assert!(candidate_count >= 1, "名前付き標本の候補が0件");
    assert!(
        runs.iter().all(|run| run.len() == candidate_count),
        "10回で候補数が変わった"
    );
    let completion_targets = runs[0]
        .iter()
        .filter(|candidate| candidate.status == ProductPlanStatus::CheckedToFinish)
        .count();
    assert!(completion_targets >= 1, "完成確認の対象が0件");
    let mut max_delta = 0.0_f64;
    for run in &runs {
        // 完成を名乗った候補は例外なく同じ全条件を満たす。
        for candidate in run
            .iter()
            .filter(|candidate| candidate.status == ProductPlanStatus::CheckedToFinish)
        {
            assert_named_completion(candidate);
        }
    }
    for run in runs.iter().skip(1) {
        for (candidate, first) in run.iter().zip(&runs[0]) {
            assert_eq!(candidate.status, first.status);
            assert_eq!(candidate.rebuilt_steps, first.rebuilt_steps);
            max_delta = max_delta.max(assert_candidate_near(candidate, first));
            max_delta = max_delta.max(assert_outcome_near(&candidate.outcome, &first.outcome));
            max_delta = max_delta.max(assert_report_near(&candidate.report, &first.report));
            max_delta = max_delta.max(assert_cp_near(
                "再構築後の展開図",
                &candidate.rebuilt_cp,
                &first.rebuilt_cp,
            ));
            max_delta = max_delta.max(assert_fold_steps_near(
                "再構築後の手順",
                &candidate.rebuilt_sequence,
                &first.rebuilt_sequence,
            ));
        }
    }

    let first = &runs[0][0];
    println!(
        "WORK30_NAMED candidates={candidate_count} completed={completion_targets}/{completion_targets} runs={RUNS}/{RUNS} first_stop={:?} first_steps={} first_consumed={}/{} first_skipped={} first_gaps={:?} tolerance={:?} first_states={}/{} first_branch={} float_max_delta={max_delta:.3e}",
        first.outcome.stop,
        first.outcome.steps.len(),
        first.report.cleared(),
        first.report.requested,
        first.report.final_check.skipped,
        first.report.final_gaps,
        CompletionTolerance::DEFAULT,
        first.outcome.states_expanded,
        first.outcome.states_generated,
        first.outcome.max_branching,
    );
    for (index, candidate) in runs[0].iter().enumerate() {
        println!(
            "WORK30_CANDIDATE index={} status={:?} stop={:?} steps={} rebuilt={} consumed={}/{} skipped={} gaps={:?} states={}/{} branch={}",
            index + 1,
            candidate.status,
            candidate.outcome.stop,
            candidate.outcome.steps.len(),
            candidate.rebuilt_steps,
            candidate.report.cleared(),
            candidate.report.requested,
            candidate.report.final_check.skipped,
            candidate.report.final_gaps,
            candidate.outcome.states_expanded,
            candidate.outcome.states_generated,
            candidate.outcome.max_branching,
        );
    }
}
