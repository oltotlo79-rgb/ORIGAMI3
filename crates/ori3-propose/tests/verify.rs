//! 作業23「全手順と完成形を検証する仕組み」の測定と検査。
//!
//! 作業21は「その手が1つ折れるか」、作業22は「完成形へ近づく順に手を選べるか」を
//! 見た。ここでは**返した手順を最初から最後まで通したときに、本当にその形になるか**を
//! 確かめ、落ちたときは**何手目の、どの確認で**落ちたかが分かることを見る。
//!
//! 標本は利用者の指示により**折り鶴とやっこさん**を使う(悪魔・1分ローズは使わない)。

use ori3_model::{CreasePattern, Document, EdgeKind, Paper};
use ori3_propose::enumerate::{FoldSession, MAX_SEAM_GAP, PoseScan, Unverified};
use ori3_propose::finish::{FinishTarget, TargetTip};
use ori3_propose::search::{FoldGoal, GapWeights, SearchBudget, TipSite, search_to_finish};
use ori3_propose::verify::{StepFailure, VerifyReport, verify_fold_order, verify_search_outcome};

#[path = "support/fixed_order.rs"]
mod fixed_order;

use fixed_order::folded_along;

/// 決定性を見るために同じ入力を回す回数(合格条件3)。
const RUNS: usize = 3;

const CRANE_STRICT_ORDER: [usize; 2] = [3, 16]; // 2026-08-28: `[16,3]`→`[3,16]`; 旧16は入力CPで一般制約2/37違反・破棄5、strict有効手は1/27。
const YAKKO_STRICT_ORDER: [usize; 2] = [2, 1]; // 2026-08-28: `[0,7,3]`→`[2,1]`; 旧0は入力CPで一般制約1/9違反・破棄5、strict有効手は4/8。
const YAKKO_EQUIVALENT_ORDER: [usize; 2] = [1, 2]; // 2026-08-28: `[0,3,7]`→`[1,2]`; 旧0は入力CPで一般制約1/9違反・破棄5、strict有効手は4/8。
const YAKKO_SECOND_REORDER_PAIR: [[usize; 2]; 2] = [[2, 6], [6, 2]]; // 2026-08-28: 折り鶴の旧`[16,3]↔[3,16]`→やっこ`[2,6]↔[6,2]`; 旧16は2/37違反・破棄5、strict有効手は折り鶴1/27・やっこ4/8。
const YAKKO_CUT_SHORT_ORDER: [usize; 2] = [1, 2]; // 2026-08-28: 打ち切り入力`[0,7,3]`→`[1,2]`; 旧0は1/9違反・破棄5、strict有効手は4/8。
const YAKKO_BAD_AFTER_FIRST: usize = 0; // 2026-08-28: 旧bad 2→0（新prefix 2後）; 旧固定prefix 0は1/9違反・破棄5、strict有効手は4/8。

/// 標本1: 折り鶴。作業18が写した展開図を、追跡対象の `tests/fixtures/` から読む。
fn crane() -> Document {
    let path = format!(
        "{}/tests/fixtures/cp-crane.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path} を読めない: {e}"));
    let cp: CreasePattern =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path} を読み解けない: {e}"));
    let mut doc = square_document();
    doc.cp = cp;
    doc
}

/// 標本2: やっこさん(座布団折り2回)。`tests/search.rs` と同じ線を同じ順に引く。
fn yakko() -> Document {
    let mut doc = square_document();
    let cp = &mut doc.cp;
    let (m1, m2, m3, m4) = ([0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]);
    for (a, b) in [(m1, m2), (m2, m3), (m3, m4), (m4, m1)] {
        ori3_cp::insert_segment(cp, a, b, EdgeKind::Valley);
    }
    for t in [0.25, 0.75] {
        for (a, b, kind) in [
            ([t, 0.0], [t, 0.25], EdgeKind::Mountain),
            ([t, 0.25], [t, 0.75], EdgeKind::Valley),
            ([t, 0.75], [t, 1.0], EdgeKind::Mountain),
            ([0.0, t], [0.25, t], EdgeKind::Mountain),
            ([0.25, t], [0.75, t], EdgeKind::Valley),
            ([0.75, t], [1.0, t], EdgeKind::Mountain),
        ] {
            ori3_cp::insert_segment(cp, a, b, kind);
        }
    }
    doc
}

fn square_document() -> Document {
    Document::new(Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    })
}

/// 紙の4隅を4本の角の材料にする。胴の中心は紙の中心(`tests/search.rs` と同じ)。
fn corner_sites() -> Vec<TipSite> {
    let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    corners
        .iter()
        .enumerate()
        .map(|(i, &material)| TipSite {
            leaf_id: i as u32 + 1,
            material,
        })
        .collect()
}

/// **その手順で実際に折り上がる形**を目標にする(`tests/search.rs` と同じ作り方)。
///
/// 利用者の指定が紙で実現できるとは限らないので、検査では「この手順で折ると
/// こうなる」という到達できる形を目標に与える。こうすると
/// 「その手順を通したら本当にその形になるか」を、答えの分かっている問題として測れる。
fn goal_of_state(doc: &Document, ids: &[usize]) -> FoldGoal {
    let draft = FoldGoal {
        target: FinishTarget {
            tips: (0..4)
                .map(|i| TargetTip {
                    leaf_id: i as u32 + 1,
                    length: 1.0,
                    width: 1.0,
                    pos: None,
                })
                .collect(),
        },
        body: [0.5, 0.5],
        sites: corner_sites(),
        layer_target: None,
    };
    let session = folded_along(doc, ids);
    let form = draft.measure(session.document());
    // 折り上げたときに出ていない角は、その形の指定に入れない。
    let present: Vec<_> = form.tips.iter().filter(|t| t.length > 0.0).collect();
    assert!(
        !present.is_empty(),
        "目標にする形に角が1本も出ていない: {:?}",
        form.tips
    );
    FoldGoal {
        target: FinishTarget {
            tips: present
                .iter()
                .map(|t| TargetTip {
                    leaf_id: t.leaf_id,
                    length: t.length,
                    width: t.width,
                    pos: t.pos,
                })
                .collect(),
        },
        sites: draft
            .sites
            .iter()
            .filter(|s| present.iter().any(|t| t.leaf_id == s.leaf_id))
            .copied()
            .collect(),
        ..draft
    }
    .with_layer_target_from(session.document())
}

/// 標本と、その標本で**最後まで通る手順**と、その手順で折り上がる形の目標。
///
/// 手順は実測で選んだ(`tests/verify.rs` を書くための測定、debugビルド)。
///
/// | 標本 | 段階0で折れる手 | 選んだ手順 | 選んだ理由 |
/// |---|---|---|---|
/// | 折り鶴 | `[3]` | `[3, 16]` | 唯一strict有効な初手3から2手を通した形を目標にする |
/// | やっこさん | `[1, 2, 5, 6]` | `[2, 1]` | strict有効な2手を通した形を目標にする |
fn samples() -> Vec<(&'static str, Document, Vec<usize>)> {
    vec![
        ("折り鶴", crane(), CRANE_STRICT_ORDER.to_vec()),
        ("やっこさん", yakko(), YAKKO_STRICT_ORDER.to_vec()),
    ]
}

fn check(doc: &Document, order: &[usize], goal: &FoldGoal) -> VerifyReport {
    let session = FoldSession::new(doc).expect("折り始められない");
    verify_fold_order(
        &session,
        order,
        goal,
        GapWeights::DEFAULT,
        PoseScan::DEFAULT,
    )
}

fn gaps_line(r: &VerifyReport) -> String {
    format!(
        "数{:.6} 長さ{:.6} 太さ{:.6} 位置{:.6} 点{:.6}",
        r.final_gaps.count,
        r.final_gaps.length,
        r.final_gaps.width,
        r.final_gaps.position,
        r.final_score
    )
}

/// 手順そのものが健全で、最後の4つの物差しも既存の表示精度内に収まったか。
fn reaches_goal(r: &VerifyReport) -> bool {
    r.passed()
        && [
            r.final_gaps.count,
            r.final_gaps.length,
            r.final_gaps.width,
            r.final_gaps.position,
        ]
        .into_iter()
        .all(|gap| gap < 1e-6)
}

/// 合格条件1: 折り鶴とやっこさんで、手順の全体を通して検証できること。
///
/// 通った手数・裂けの最大・すり抜けの件数・最後の4つの物差しを報告し、
/// 最後の形が指定した完成形と一致することまで見る。
#[test]
fn a_whole_fold_order_is_checked_from_the_first_move_to_the_finished_shape() {
    for (name, doc, order) in samples() {
        let goal = goal_of_state(&doc, &order);
        let report = check(&doc, &order, &goal);
        println!("== {name} 手順{order:?}: {}", report.describe());
        println!("   最後の形: {:?}", report.final_check);
        println!("   4つの物差し: {}", gaps_line(&report));
        for s in &report.steps {
            println!(
                "   {}手目(折り線{}): 裂け{:.3e} / すり抜け{} / 姿勢{}点 / 重なり順の食い違い{}",
                s.index + 1,
                s.id,
                s.max_seam_gap,
                s.penetrations,
                s.poses_checked,
                s.layer_warnings
            );
        }

        assert!(report.passed(), "{name}: 手順が通らなかった");
        assert_eq!(
            report.cleared(),
            order.len(),
            "{name}: 通った手数が足りない"
        );
        assert_eq!(report.failure, None, "{name}: 落ちた場所が入っている");
        // 実測(debugビルド): 折り鶴 1.188e-13 / やっこさん 1.927e-14。
        // 上限 1e-6 に対して7桁の余裕がある。
        assert!(
            report.max_seam_gap < MAX_SEAM_GAP,
            "{name}: 裂け {} が上限 {MAX_SEAM_GAP} 以上",
            report.max_seam_gap
        );
        assert_eq!(report.penetrations, 0, "{name}: すり抜けが0件でない");
        // 各手21点 + 最後の形1点。
        assert_eq!(
            report.poses_checked,
            order.len() * PoseScan::DEFAULT.points() + 1,
            "{name}: 見た姿勢の数が合わない"
        );
        for s in &report.steps {
            assert_eq!(s.poses_checked, PoseScan::DEFAULT.points());
            assert_eq!(s.penetrations, 0, "{name}: {}手目のすり抜け", s.index + 1);
            assert_eq!(
                s.layer_warnings,
                0,
                "{name}: {}手目で紙の重なり順が食い違った",
                s.index + 1
            );
        }
        // 1手目に閉じた直線が、その番号の折り線の端から端と一致すること。
        let start = FoldSession::new(&doc).expect("折り始められない");
        let first = start
            .fold_lines()
            .iter()
            .find(|l| l.id == order[0])
            .unwrap_or_else(|| panic!("{name}: 折り線 {} が無い", order[0]));
        assert_eq!(
            report.steps[0].line,
            [first.a, first.b],
            "{name}: 1手目に閉じた直線が折り線と食い違う"
        );

        assert!(
            report.final_check.is_sound(),
            "{name}: 最後の形に問題がある"
        );
        assert_eq!(report.final_check.skipped, 0);
        assert_eq!(report.final_check.warnings, 0);
        assert_eq!(report.final_check.faces, report.final_check.expected_faces);
        assert!(report.final_check.finite);
        // 実測(debugビルド): 折り鶴 1.225e-16 / やっこさん 4.441e-16。
        assert!(
            report.final_check.max_seam_gap < MAX_SEAM_GAP,
            "{name}: 最後の形の裂け {} が上限以上",
            report.final_check.max_seam_gap
        );
        assert_eq!(
            report.final_check.penetrations, 0,
            "{name}: 最後の形ですり抜けている"
        );

        // 目標はこの手順で実際に折り上がる形なので、最後は4つとも最良になる。
        // 上限 1e-6 は、姿勢の表示精度として使われている値と同じにした。
        // 実測の隔たりは4つとも 0.0(完全一致)だった。
        for (label, v) in [
            ("数", report.final_gaps.count),
            ("長さ", report.final_gaps.length),
            ("太さ", report.final_gaps.width),
            ("位置", report.final_gaps.position),
        ] {
            assert!(
                v < 1e-6,
                "{name}: 最後の形の{label}の隔たり {v} が 1e-6 以上。\
                 手順を通しても目標の形になっていない"
            );
        }
        assert!(
            report.final_score <= report.start_score,
            "{name}: 折った後の点数 {} が折る前 {} より悪い",
            report.final_score,
            report.start_score
        );
    }
}

/// 合格条件2の前提: 幾何的に有効な手の入れ替えを、壊れた手順として拒否しないこと。
#[test]
fn a_geometrically_valid_reordered_fold_order_is_not_rejected() {
    // 重なり順を幾何から求める前は、どちらも入れ替えた2手目が
    // 「紙がすり抜ける」で落ちていた。しかし、そのすり抜けは折り返した紙を
    // 相手の上へ回せなかったための偽陽性だった。現在は両順とも最後まで折れる。
    //
    // | 標本 | 基準の手順 | 入れ替えた手順 |
    // |---|---|---|
    // | やっこさんA | `[1, 2]` | `[2, 1]` |
    // | やっこさんB | `[2, 6]` | `[6, 2]` |
    // 2026-08-28のstrict実測では、どちらの組も両順が通り、完成目標の4 gapは0だった。
    let cases: Vec<(&str, Document, Vec<usize>, Vec<usize>)> = vec![
        (
            "やっこさんA",
            yakko(),
            YAKKO_EQUIVALENT_ORDER.to_vec(),
            YAKKO_STRICT_ORDER.to_vec(),
        ),
        (
            "やっこさんB",
            yakko(),
            YAKKO_SECOND_REORDER_PAIR[0].to_vec(),
            YAKKO_SECOND_REORDER_PAIR[1].to_vec(),
        ),
    ];
    for (name, doc, good, swapped) in cases {
        let goal = goal_of_state(&doc, &good);
        let _ = folded_along(&doc, &swapped);
        let good_report = check(&doc, &good, &goal);
        assert!(good_report.passed(), "{name}: 正しい手順のほうが通らない");

        let report = check(&doc, &swapped, &goal);
        println!("{name} 入れ替え{swapped:?}: {}", report.describe());
        assert!(
            report.passed(),
            "{name}: 有効になった入れ替え手順 {swapped:?} を拒否した: {:?}",
            report.failure
        );
        assert_eq!(report.failure, None, "{name}: 落ちた場所が入っている");
        assert_eq!(
            report.cleared(),
            swapped.len(),
            "{name}: 通った手数が足りない"
        );
        assert_eq!(report.requested, swapped.len());
        assert!(report.max_seam_gap < MAX_SEAM_GAP);
        assert_eq!(report.penetrations, 0);
        assert!(report.final_check.is_sound());
        assert!(
            reaches_goal(&report),
            "{name}: 入れ替え手順は折れるが、基準手順と同じ完成目標へ届いていない"
        );
        assert_eq!(
            report.poses_checked,
            swapped.len() * PoseScan::DEFAULT.points() + 1,
            "{name}: 見た姿勢の数が合わない"
        );
    }
}

/// 合格条件2の2つ目: **折れない手を混ぜる**と、何手目で落ちたかが分かること。
#[test]
fn an_unfoldable_move_mixed_into_the_order_fails_at_that_move() {
    // 混ぜる手は、その時点で**実際に折れないと確かめた手**から選ぶ(実測、debugビルド)。
    //
    // | 標本 | 正しい手順 | 混ぜた手順 | 落ちる場所 |
    // |---|---|---|---|
    // | 折り鶴 | `[3, 16]` | `[3, 0, 16]` | 2手目・手0 が **平らに畳めない** |
    // | やっこさん | `[2, 1]` | `[2, 0, 1]` | 2手目・手0 が **平らに畳めない**(手2を折ると手0は畳めなくなる) |
    let cases: Vec<(&str, Document, Vec<usize>, usize)> = vec![
        ("折り鶴", crane(), CRANE_STRICT_ORDER.to_vec(), 0),
        (
            "やっこさん",
            yakko(),
            YAKKO_STRICT_ORDER.to_vec(),
            YAKKO_BAD_AFTER_FIRST,
        ),
    ];
    for (name, doc, good, bad) in cases {
        let goal = goal_of_state(&doc, &good);
        let mut mixed = good.clone();
        mixed.insert(1, bad);

        // 混ぜる手が、その時点で本当に折れないことを先に確かめる。
        let after_first = folded_along(&doc, &good[..1]);
        assert!(
            after_first.has_fold_line(bad),
            "{name}: 混ぜる手 {bad} がそもそも展開図に無い。別の壊し方になっている"
        );
        assert!(
            matches!(after_first.check_move(bad, PoseScan::DEFAULT), Some(Err(_))),
            "{name}: 混ぜる手 {bad} は折れてしまう。折れない手を混ぜたことにならない"
        );

        let report = check(&doc, &mixed, &goal);
        let failure = report
            .failure
            .unwrap_or_else(|| panic!("{name}: 折れない手を混ぜた手順 {mixed:?} が落ちなかった"));
        println!("{name} 折れない手混ぜ{mixed:?}: {}", report.describe());
        assert_eq!(failure.index, 1, "{name}: 落ちた手数が2手目でない");
        assert_eq!(failure.id, bad, "{name}: 落ちた折り線の番号が違う");
        assert_eq!(
            failure.cause,
            StepFailure::NotFoldable(Unverified::CannotCollapse),
            "{name}: 落ちた理由が「平らに畳めない」でない"
        );
        assert_eq!(report.cleared(), 1, "{name}: 通った手数が1手でない");
        assert!(!report.passed());
    }
}

/// 合格条件2の3つ目: 手順を完成前で打ち切ると、目標へ届いていないと分かること。
#[test]
fn a_fold_order_cut_short_is_detected_by_the_unfinished_shape() {
    // 重なり順を幾何から求めるようになり、以前「紙がすり抜ける」で落ちた
    // `[3, 16]` と `[1, 2]` は本当に折れると分かった。そのため、手の成否ではなく、
    // 健全な1手だけで打ち切った形が完成目標へ届かないことを4つの物差しで見る。
    //
    // 2026-08-28のstrict実測: 折り鶴は点0.603553→0.353553で主張を満たす。
    // やっこさんは深さ2の12列と深さ3の24列を全測定したが、「前進し、かつ未完成」は
    // 0/36だった。`[1]→[1,2]` は0.5→0.5で未完成だが前進せず、下のassertを
    // 緩めずに統括判断のため失敗を露出させている。
    let cases: Vec<(&str, Document, Vec<usize>, Vec<usize>)> = vec![
        (
            "折り鶴",
            crane(),
            CRANE_STRICT_ORDER.to_vec(),
            vec![CRANE_STRICT_ORDER[0]],
        ),
        (
            "やっこさん",
            yakko(),
            YAKKO_CUT_SHORT_ORDER.to_vec(),
            vec![YAKKO_CUT_SHORT_ORDER[0]],
        ),
    ];
    for (name, doc, good, incomplete) in cases {
        let goal = goal_of_state(&doc, &good);
        let good_report = check(&doc, &good, &goal);
        assert!(
            reaches_goal(&good_report),
            "{name}: 正しい手順が完成目標へ届かない"
        );

        let report = check(&doc, &incomplete, &goal);
        println!("{name} 完成前打ち切り{incomplete:?}: {}", report.describe());
        println!("   そこまでの4つの物差し: {}", gaps_line(&report));
        assert!(
            report.passed(),
            "{name}: 完成前の手順そのものが壊れている: {:?}",
            report.failure
        );
        assert_eq!(report.failure, None, "{name}: 落ちた場所が入っている");
        assert_eq!(
            report.cleared(),
            incomplete.len(),
            "{name}: 通った手数が足りない"
        );
        assert_eq!(report.requested, incomplete.len());
        assert!(report.final_check.is_sound());
        assert!(report.max_seam_gap < MAX_SEAM_GAP);
        assert_eq!(report.penetrations, 0);
        assert_eq!(
            report.poses_checked,
            incomplete.len() * PoseScan::DEFAULT.points() + 1,
            "{name}: 見た姿勢の数が合わない"
        );
        assert!(
            !reaches_goal(&report),
            "{name}: 完成前で打ち切っても目標の形になってしまっている"
        );
        assert!(
            report.final_score > 1e-6,
            "{name}: 完成前で打ち切った点 {} が完成扱いになっている",
            report.final_score
        );
        assert!(
            report.final_score < report.start_score,
            "{name}: 1手折っても目標へ近づいていない({} → {})",
            report.start_score,
            report.final_score
        );
        let start_session = FoldSession::new(&doc).expect("折り始められない");
        let prefix_session = folded_along(&doc, &incomplete);
        let completed_session = folded_along(&doc, &good);
        let start_layer_gap = goal.layer_gap(start_session.document());
        let prefix_layer_gap = goal.layer_gap(prefix_session.document());
        let completed_layer_gap = goal.layer_gap(completed_session.document());
        println!(
            "{name} 材料層構造の隔たり: 開始{start_layer_gap:.6} / 打切り{prefix_layer_gap:.6} / 完成{completed_layer_gap:.6}"
        );
        assert_eq!(
            start_layer_gap, 1.0,
            "{name}: 折る前に目標の材料層順を共有している"
        );
        assert_eq!(
            prefix_layer_gap, 0.5,
            "{name}: 1段分の材料層順を進捗として測れていない"
        );
        assert_eq!(
            completed_layer_gap, 0.0,
            "{name}: 完成形の材料層順が目標と一致していない"
        );
    }
}

/// 選べない手を渡したときも、何手目で落ちたかが分かること。
///
/// - **その番号の折り線が無い**: 展開図に無い番号を渡した場合。
/// - **もう折り終えている**: 同じ手を2回渡した場合。
#[test]
fn moves_that_cannot_be_chosen_are_reported_with_their_place_in_the_order() {
    let doc = yakko();
    let goal = goal_of_state(&doc, &YAKKO_STRICT_ORDER);
    let prefix = YAKKO_STRICT_ORDER[0];

    // やっこさんの折り線は8本(番号0〜7)なので、9999番は存在しない。
    let session = FoldSession::new(&doc).expect("折り始められない");
    assert_eq!(session.fold_lines().len(), 8, "折り線の本数が変わっている");
    assert!(!session.has_fold_line(9999));
    let report = check(&doc, &[prefix, 9999], &goal);
    let failure = report.failure.expect("無い番号なのに落ちなかった");
    println!("やっこさん 無い番号: {}", report.describe());
    assert_eq!(failure.index, 1);
    assert_eq!(failure.id, 9999);
    assert_eq!(failure.cause, StepFailure::NoSuchFoldLine);
    assert_eq!(failure.cause.label(), "その折り線が無い");
    assert_eq!(report.cleared(), 1);

    // 同じ手を2回。1回目で折り終えているので、2回目は選べない。
    let report = check(&doc, &[prefix, prefix], &goal);
    let failure = report.failure.expect("同じ手を2回渡したのに落ちなかった");
    println!("やっこさん 同じ手を2回: {}", report.describe());
    assert_eq!(failure.index, 1);
    assert_eq!(failure.id, prefix);
    assert_eq!(failure.cause, StepFailure::AlreadyFolded);
    assert_eq!(
        failure.describe(),
        format!("2手目(折り線{prefix}): もう折り終えている")
    );
    assert_eq!(report.cleared(), 1);
}

/// 合格条件3: 同じ手順を3回検証して、結果が完全に一致すること。
///
/// 通る基準手順と有効な入れ替え、折れない手を混ぜた手順、完成前で
/// 打ち切った手順のいずれでも一致することを見る。
/// 落ちた場所や最後の4つの物差しまで毎回同じでなければ、報告として使えない。
#[test]
fn the_same_order_gives_the_same_report_three_times() {
    let crane_doc = crane();
    let crane_goal = goal_of_state(&crane_doc, &CRANE_STRICT_ORDER);
    let yakko_doc = yakko();
    let yakko_goal = goal_of_state(&yakko_doc, &YAKKO_STRICT_ORDER);
    let _ = folded_along(&yakko_doc, &YAKKO_EQUIVALENT_ORDER);
    let cases: Vec<(&str, &Document, Vec<usize>, &FoldGoal)> = vec![
        (
            "折り鶴 そのまま",
            &crane_doc,
            CRANE_STRICT_ORDER.to_vec(),
            &crane_goal,
        ),
        (
            "やっこさん 入れ替え",
            &yakko_doc,
            YAKKO_EQUIVALENT_ORDER.to_vec(),
            &yakko_goal,
        ),
        (
            "やっこさん そのまま",
            &yakko_doc,
            YAKKO_STRICT_ORDER.to_vec(),
            &yakko_goal,
        ),
        (
            "やっこさん 折れない手混ぜ",
            &yakko_doc,
            vec![
                YAKKO_STRICT_ORDER[0],
                YAKKO_BAD_AFTER_FIRST,
                YAKKO_STRICT_ORDER[1],
            ],
            &yakko_goal,
        ),
        (
            "やっこさん 完成前打ち切り",
            &yakko_doc,
            vec![YAKKO_STRICT_ORDER[0]],
            &yakko_goal,
        ),
    ];
    for (name, doc, order, goal) in cases {
        let runs: Vec<VerifyReport> = (0..RUNS).map(|_| check(doc, &order, goal)).collect();
        for (i, r) in runs.iter().enumerate().skip(1) {
            assert_eq!(&runs[0], r, "{name}: {}回目の結果が1回目と違う", i + 1);
        }
        println!("{name}: {RUNS}回とも同じ結果 / {}", runs[0].describe());
    }
}

/// 作業22の探索が返した手順が、そのまま全体の検証を通ること。
///
/// 探索は速さのために粗い確認で候補を集める。ここでは返ってきた手順を
/// **改めて最初から姿勢21点で**折り直し、最後の形まで確かめる。
#[test]
fn the_fold_order_returned_by_the_search_passes_the_whole_check() {
    for (name, doc, order) in samples() {
        let goal = goal_of_state(&doc, &order);
        let session = FoldSession::new(&doc).expect("折り始められない");
        let outcome = search_to_finish(&session, &goal, GapWeights::DEFAULT, SearchBudget::DEFAULT);
        let ids: Vec<usize> = outcome.steps.iter().map(|s| s.mv.id).collect();
        let report = verify_search_outcome(
            &session,
            &outcome,
            &goal,
            GapWeights::DEFAULT,
            PoseScan::DEFAULT,
        );
        println!("{name} 探索の手順{ids:?}: {}", report.describe());
        println!("   4つの物差し: {}", gaps_line(&report));
        assert!(!ids.is_empty(), "{name}: 探索が手順を返さなかった");
        assert!(
            report.passed(),
            "{name}: 探索が返した手順が全体の検証を通らなかった: {:?}",
            report.failure
        );
        assert_eq!(report.cleared(), ids.len());
        assert_eq!(report.penetrations, 0);
        assert!(report.max_seam_gap < MAX_SEAM_GAP);
        // 探索が測った点数と、検証で測り直した点数が食い違わないこと。
        assert!(
            (report.final_score - outcome.best_score).abs() < 1e-6,
            "{name}: 探索の点数 {} と検証の点数 {} が食い違う",
            outcome.best_score,
            report.final_score
        );
    }
}

/// 手順が空のときも止まらず、平らな紙をそのまま測ること。
///
/// 「どの手を折っても目標から遠ざかる」展開図では、作業22の探索は手順0件を返す
/// (異常ではない)。その手順を検証しても落ちないことを見る。
#[test]
fn an_empty_order_is_checked_as_the_flat_paper() {
    let doc = yakko();
    let goal = goal_of_state(&doc, &YAKKO_STRICT_ORDER);
    let report = check(&doc, &[], &goal);
    println!("やっこさん 手順なし: {}", report.describe());
    assert!(
        report.passed(),
        "手順が空なのに落ちた: {:?}",
        report.failure
    );
    assert_eq!(report.requested, 0);
    assert_eq!(report.cleared(), 0);
    assert_eq!(report.poses_checked, 1, "最後の形の1点だけを見るはず");
    assert!(report.final_check.is_sound());
    // 折る前と後が同じ形なので、4つの物差しも同じ値になる。
    assert_eq!(report.start_gaps, report.final_gaps);
    assert_eq!(report.start_score, report.final_score);
}
