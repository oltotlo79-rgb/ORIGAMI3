//! 作業22「4つの目標で順位付けする決定的な探索」の測定と検査。
//!
//! 作業21は「その手が実際に折れるか」だけを見た。ここでは折れる手を
//! **完成形へ近づく順**に選び、決定的な手順として返せることを確かめる。
//!
//! 標本は利用者の指示により**折り鶴とやっこさん**を使う(悪魔・1分ローズは使わない)。

use ori3_model::{CreasePattern, Document, EdgeKind, Paper};
use ori3_propose::enumerate::{FoldSession, MAX_SEAM_GAP, PoseScan};
use ori3_propose::finish::{FinishGaps, FinishTarget, TargetTip, finish_gaps};
use ori3_propose::search::{
    FoldGoal, GapWeights, SCORE_QUANTUM, SearchBudget, SearchOutcome, SearchStop, TipSite,
    search_to_finish,
};
use ori3_propose::skeleton::TipPos2d;
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

#[path = "support/fixed_order.rs"]
mod fixed_order;

use fixed_order::folded_along;

/// 決定性を見るために同じ入力を回す回数(合格条件2)。
const RUNS: usize = 3;

const CRANE_STRICT_GOAL_ORDER: [usize; 2] = [3, 16]; // 2026-08-28: `[16,3]`→`[3,16]`; 旧16は入力CPで一般制約2/37違反・物理破棄4＋表示marker 1、strict有効手は1/27。
const YAKKO_STRICT_GOAL_ORDER: [usize; 1] = [2]; // 2026-08-28: `[0,7]`→`[2]`; 旧0は入力CPで一般制約1/9違反・破棄5、strict有効手は4/8。

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

/// 標本2: やっこさん(座布団折り2回)。`tests/enumerate.rs` と同じ線を同じ順に引く。
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

/// 紙の4隅を4本の角の材料にする。胴の中心は紙の中心。
///
/// 提案が作った展開図なら、この対応は作業9が残す [`ori3_propose::LeafSite`] から
/// そのまま作れる。作品の展開図には対応が無いので、ここでは「紙のこの角が
/// この角になる」という利用者の指定として与える。
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

/// 4本の角の指定(長さ・太さ・位置)から目標を作る。
fn corner_goal(lengths: [f64; 4], widths: [f64; 4], pos: [[f64; 2]; 4]) -> FoldGoal {
    FoldGoal {
        target: FinishTarget {
            tips: (0..4)
                .map(|i| TargetTip {
                    leaf_id: i as u32 + 1,
                    length: lengths[i],
                    width: widths[i],
                    pos: Some(TipPos2d::new(pos[i][0], pos[i][1])),
                })
                .collect(),
        },
        body: [0.5, 0.5],
        sites: corner_sites(),
        layer_target: None,
    }
}

/// 重みを決めるときに使った目標(`GapWeights::DEFAULT` のコメントと同じもの)。
///
/// 角4本を、長さ `1.0 / 1.0 / 0.6 / 0.6`、太さ `0.3`、位置は紙の四隅に指定する。
fn weights_goal() -> FoldGoal {
    corner_goal(
        [1.0, 1.0, 0.6, 0.6],
        [0.3, 0.3, 0.3, 0.3],
        [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
    )
}

/// **実際に折り上げられる形**を目標にする。
///
/// 利用者の指定が紙で実現できるとは限らないので、検査では
/// 「この手順で折るとこうなる」という到達できる形を目標として与える。
/// こうすると「その形へ行き着く折り順を見つけられるか」を、
/// 答えが分かっている問題として確かめられる。
///
/// 作り方は2段構えである。長さも位置も「いちばん遠い先端に合わせてそろえる」
/// (作業20 §2.4)ので、目標が無いと目盛りが決まらない。そこでまず
/// 長さ1.0・太さ1.0・位置なしの下ごしらえで測り、その測り値をそのまま目標にする。
/// 下ごしらえの目盛り(いちばん長い先端が1.0)と、出来上がった目標の目盛りは
/// 同じなので、同じ形をもう一度測れば4つとも0になる
/// (検査 `a_goal_taken_from_a_folded_shape_scores_zero_on_that_shape` が照合する)。
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
    // 折り上げたときに出ていない角は、その形の指定に入れない
    // (「無い角の長さ」を目標にすると、目標そのものが成り立たない)。
    let present: Vec<_> = form.tips.iter().filter(|t| t.length > 0.0).collect();
    assert!(
        present.len() >= 3,
        "目標にする形で出ている角が {} 本しかない: {:?}",
        present.len(),
        form.tips
    );
    assert!(
        present.iter().all(|t| t.pos.is_some()),
        "出ているのに位置を測れていない角がある: {:?}",
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
}

/// 固定入力が無効になったとき、21姿勢の一般的な失敗ではなく固定IDの破損と分かること。
#[test]
fn an_invalid_fixed_move_is_reported_as_a_broken_fixed_input() {
    // 候補16は診断文を確かめるために意図的に渡す。目標fixtureには使わない。
    let panic = match std::panic::catch_unwind(|| folded_along(&crane(), &[16])) {
        Ok(_) => panic!("無効な固定手16を渡したのに共通入口が失敗しなかった"),
        Err(panic) => panic,
    };
    let message = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
        .unwrap_or_else(|| "文字列でないpanic".to_owned());
    println!("STAGE8_FIXED_ORDER_DIAGNOSTIC {message}");
    assert!(
        message.contains("固定している手が無効になった"),
        "固定入力の破損だと分からない診断だった: {message}"
    );
    assert!(
        message.contains("strict 有効手の実測は"),
        "失敗時の全数再測定が診断に入っていない: {message}"
    );
}

/// 標本と、その標本で**実際に折り上げられる形**の目標。
///
/// 固定手順は入口で入力CPの一般制約を使って検証する。折り鶴は唯一strict有効な
/// 初手3から完成する `[3, 16]`、やっこさんは初期のstrict有効4手のうち、平らな
/// 状態から点数を改善できる到達形 `[2]` を使う。
fn samples() -> Vec<(&'static str, Document, FoldGoal)> {
    let crane_doc = crane();
    let crane_goal = goal_of_state(&crane_doc, &CRANE_STRICT_GOAL_ORDER);
    let yakko_doc = yakko();
    let yakko_goal = goal_of_state(&yakko_doc, &YAKKO_STRICT_GOAL_ORDER);
    vec![
        ("折り鶴", crane_doc, crane_goal),
        ("やっこさん", yakko_doc, yakko_goal),
    ]
}

fn line(g: &FinishGaps, score: f64) -> String {
    format!(
        "数{:.6} 長さ{:.6} 太さ{:.6} 位置{:.6} 点{score:.6}",
        g.count, g.length, g.width, g.position
    )
}

/// 目標に選んだ形をもう一度測ると、4つとも最良(0)になること。
///
/// 目盛りのそろえ方(作業20 §2.4)が測るたびにぶれないことの確認でもある。
/// 上限 `1e-6` は、姿勢の表示精度として使われている値と同じにした。
/// 実測の隔たりは4つとも `0.0`(完全一致)だった。
#[test]
fn a_goal_taken_from_a_folded_shape_scores_zero_on_that_shape() {
    for (name, doc, ids) in [
        ("折り鶴", crane(), CRANE_STRICT_GOAL_ORDER.to_vec()),
        ("やっこさん", yakko(), YAKKO_STRICT_GOAL_ORDER.to_vec()),
    ] {
        let goal = goal_of_state(&doc, &ids);
        let session = folded_along(&doc, &ids);
        let gaps = finish_gaps(&goal.target, &goal.measure(session.document()));
        println!(
            "{name} 手順{ids:?} を目標にして測り直す: {}",
            line(&gaps, 0.0)
        );
        for (label, v) in [
            ("数", gaps.count),
            ("長さ", gaps.length),
            ("太さ", gaps.width),
            ("位置", gaps.position),
        ] {
            assert!(
                v < 1e-6,
                "{name}: {label}の隔たり {v} が 1e-6 以上。目盛りのそろえ方がぶれている"
            );
        }
    }
}

/// **重みを決めた根拠の測定**(段階1)。最初の手ごとに4つの物差しを並べる。
///
/// 順位を決めるのは値の大きさではなく**手による差**なので、振れ幅を見る。
/// 実測(debugビルド)は `GapWeights::DEFAULT` のコメントの表のとおりで、
/// **太さだけが他の3つの5倍以上広い**。単純な足し算にすると太さだけで順位が
/// 決まるので、太さの重みを下げた。
#[test]
fn measure_the_four_gaps_of_every_first_move() {
    let goal = weights_goal();
    for (name, doc) in [("折り鶴", crane()), ("やっこさん", yakko())] {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let start = finish_gaps(&goal.target, &goal.measure(session.document()));
        println!(
            "== {name} 平らなとき: {}",
            line(&start, GapWeights::DEFAULT.score(&start))
        );
        let report = session.verified_moves(SearchBudget::DEFAULT.scan);
        let mut rows: Vec<(usize, FinishGaps, f64)> = Vec::new();
        for operation in report.operation_moves() {
            let mv = operation.movement();
            let mut next = session.clone();
            if next.apply_operation(operation).is_err() {
                continue;
            }
            let g = finish_gaps(&goal.target, &goal.measure(next.document()));
            assert!(g.all_finite(), "{name} 手{}: 数にならない値が出た", mv.id);
            let plain = g.count + g.length + g.width + g.position;
            println!(
                "   手{:3}: {} / 重み全部1のときの点{plain:.6}",
                mv.id,
                line(&g, GapWeights::DEFAULT.score(&g))
            );
            rows.push((mv.id, g, plain));
        }
        assert!(!rows.is_empty(), "{name}: 折れる手が1つも無い");

        let pick = |g: &FinishGaps, k: usize| [g.count, g.length, g.width, g.position][k];
        let mut spread = [0.0_f64; 4];
        for (k, label) in ["数", "長さ", "太さ", "位置"].iter().enumerate() {
            let lo = rows
                .iter()
                .map(|r| pick(&r.1, k))
                .fold(f64::INFINITY, f64::min);
            let hi = rows
                .iter()
                .map(|r| pick(&r.1, k))
                .fold(f64::NEG_INFINITY, f64::max);
            spread[k] = hi - lo;
            println!("   {label}の振れ幅: {:.6}({lo:.6}〜{hi:.6})", spread[k]);
        }

        if name == "やっこさん" {
            // 実測: 数0.250 / 長さ0.299 / 太さ1.655 / 位置0.354。
            // 太さは他3つの平均(0.301)の5.5倍だった。倍率3.0で照合するのは、
            // 5.5に対して4割以上の余裕を取り、計算機の違いで揺れないようにするため。
            let others = (spread[0] + spread[1] + spread[3]) / 3.0;
            assert!(
                spread[2] > others * 3.0,
                "やっこさん: 太さの振れ幅 {} が他3つの平均 {others} の3倍に届かない。\
                 重みを決めた前提(太さだけが広い)が変わっている",
                spread[2]
            );
        }

        let mut order: Vec<(usize, f64)> = rows
            .iter()
            .map(|r| (r.0, GapWeights::DEFAULT.score(&r.1)))
            .collect();
        order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        println!(
            "   既定の重みでの順位: {:?}",
            order.iter().map(|(id, _)| *id).collect::<Vec<_>>()
        );

        // 点数の刻み(SCORE_QUANTUM)が意味のある差を潰していないこと。
        let mut scores: Vec<f64> = rows.iter().map(|r| r.2).collect();
        scores.sort_by(f64::total_cmp);
        let mut smallest = f64::INFINITY;
        for w in scores.windows(2) {
            let d = w[1] - w[0];
            if d > SCORE_QUANTUM {
                smallest = smallest.min(d);
            }
        }
        if smallest.is_finite() {
            println!("   点数の意味のある差のうち最小: {smallest:.3e}");
            assert!(
                smallest > SCORE_QUANTUM * 1000.0,
                "{name}: 点数の差 {smallest} が刻み {SCORE_QUANTUM} に近すぎる。\
                 丸めが意味のある差を潰しうる"
            );
        }
    }
}

/// 合格条件1: 折り鶴とやっこさんで手順を返し、4つの物差しが探索の前後で良くなること。
#[test]
fn the_search_returns_a_fold_order_that_gets_closer_to_the_goal() {
    for (name, doc, goal) in samples() {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let started = std::time::Instant::now();
        let out = search_to_finish(&session, &goal, GapWeights::DEFAULT, SearchBudget::DEFAULT);
        let elapsed = started.elapsed().as_secs_f64();
        let ids: Vec<usize> = out.steps.iter().map(|s| s.mv.id).collect();
        println!("== {name}: {}手 {ids:?} / {:?}", out.steps.len(), out.stop);
        println!(
            "   かかった時間 {elapsed:.1}秒(広げた状態1つあたり {:.2}秒)",
            elapsed / out.states_expanded.max(1) as f64
        );
        println!("   前: {}", line(&out.start_gaps, out.start_score));
        println!("   後: {}", line(&out.best_gaps, out.best_score));
        println!(
            "   広げた状態{} / 作った状態{} / 枝分かれ最大{} / 深さの打ち切り{}回",
            out.states_expanded, out.states_generated, out.max_branching, out.depth_capped
        );

        assert!(!out.steps.is_empty(), "{name}: 手順が1件も返らなかった");
        assert!(
            out.best_score < out.start_score,
            "{name}: 点数が良くなっていない({} → {})",
            out.start_score,
            out.best_score
        );
        for (label, before, after) in [
            ("数", out.start_gaps.count, out.best_gaps.count),
            ("長さ", out.start_gaps.length, out.best_gaps.length),
            ("太さ", out.start_gaps.width, out.best_gaps.width),
            ("位置", out.start_gaps.position, out.best_gaps.position),
        ] {
            assert!(
                after <= before + 1e-9,
                "{name}: {label}の隔たりが悪くなった({before} → {after})"
            );
        }
        // 目標は実際に折り上げられる形なので、そこへ行き着けること。
        assert!(
            out.best_score < 1e-6,
            "{name}: 目標の形へ行き着けなかった(点 {})",
            out.best_score
        );
    }
}

/// 合格条件3: 返した手順のすべての手が、途中の姿勢21点で本当に折れること。
///
/// 探索は速さのために姿勢5点で確かめている([`SearchBudget::DEFAULT`])。
/// ここでは返ってきた手順を**改めて21点で**折り直し、裂けとめり込みを測る。
#[test]
fn every_move_in_the_returned_order_really_folds() {
    let scan = PoseScan::DEFAULT;
    for (name, doc, goal) in samples() {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let out = search_to_finish(&session, &goal, GapWeights::DEFAULT, SearchBudget::DEFAULT);
        assert!(!out.steps.is_empty(), "{name}: 手順が1件も返らなかった");

        let mut replayed_session = FoldSession::new(&doc).expect("折り始められない");
        let mut worst_gap: f64 = 0.0;
        let mut worst_pairs = 0usize;
        for (i, step) in out.steps.iter().enumerate() {
            // その時点で、返ってきた手が姿勢21点でも折れると確かめられること。
            let found = replayed_session
                .verify_move(step.mv.id, scan)
                .unwrap_or_else(|| {
                    panic!(
                        "{name}: {}手目(手{})が、姿勢21点では折れない",
                        i + 1,
                        step.mv.id
                    )
                });
            let movement = found.movement();
            assert!(
                movement.max_seam_gap < MAX_SEAM_GAP,
                "{name}: {}手目の裂け {} が上限 {MAX_SEAM_GAP} 以上",
                i + 1,
                movement.max_seam_gap
            );
            assert_eq!(
                movement.penetrations,
                0,
                "{name}: {}手目のめり込みが0件でない",
                i + 1
            );
            assert_eq!(movement.poses_checked, scan.points());
            replayed_session
                .apply(&found)
                .unwrap_or_else(|e| panic!("{name}: {}手目を進められない: {e}", i + 1));

            // 進めたあとの姿勢を、もう一度21点で測り直す。
            let faces = ori3_cp::extract_faces(&replayed_session.document().cp);
            let up_to = replayed_session.document().sequence.len();
            for k in 0..scan.points() {
                let t = k as f64 / scan.steps as f64;
                let frame =
                    ori3_layers::replay::replay(replayed_session.document(), up_to, t).frame;
                worst_gap = worst_gap.max(max_seam_gap(
                    &replayed_session.document().cp,
                    &faces,
                    &frame,
                ));
                worst_pairs = worst_pairs.max(self_intersection_pairs(&frame).len());
            }
        }
        println!(
            "{name}: {}手すべてを姿勢{}点で確かめた / 裂けの最大 {worst_gap:.3e} / めり込み {worst_pairs}",
            out.steps.len(),
            scan.points()
        );
        assert!(
            worst_gap < MAX_SEAM_GAP,
            "{name}: 裂け {worst_gap} が上限 {MAX_SEAM_GAP} 以上"
        );
        assert_eq!(worst_pairs, 0, "{name}: めり込みが0件でない");
    }
}

/// 合格条件2: 同じ入力を3回実行して、返す手順が完全に一致すること。
#[test]
fn the_same_input_gives_the_same_fold_order_three_times() {
    for (name, doc, goal) in samples() {
        let runs: Vec<SearchOutcome> = (0..RUNS)
            .map(|_| {
                let session = FoldSession::new(&doc).expect("折り始められない");
                search_to_finish(&session, &goal, GapWeights::DEFAULT, SearchBudget::DEFAULT)
            })
            .collect();
        for (i, r) in runs.iter().enumerate().skip(1) {
            assert_eq!(&runs[0], r, "{name}: {}回目の結果が1回目と違う", i + 1);
        }
        let ids: Vec<usize> = runs[0].steps.iter().map(|s| s.mv.id).collect();
        println!(
            "{name}: {RUNS}回とも同じ手順 {ids:?}(点 {:.6})",
            runs[0].best_score
        );
    }
}

/// 合格条件4: 打ち切りに達しても、そこまでの最良の手順を返すこと。
///
/// 状態を1つしか広げられない予算にすると、最初の手を選んだところで打ち切られる。
/// それでも手順は空にならず、選ばれた手は点数のいちばん良いものになる。
#[test]
fn the_search_returns_its_best_so_far_when_the_budget_runs_out() {
    let tight = SearchBudget {
        max_states: 1,
        ..SearchBudget::DEFAULT
    };
    for (name, doc, goal) in samples() {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let out = search_to_finish(&session, &goal, GapWeights::DEFAULT, tight);
        let ids: Vec<usize> = out.steps.iter().map(|s| s.mv.id).collect();
        println!(
            "{name} 打ち切り: 手順{ids:?} / {:?} / 広げた状態{} / 作った状態{} / 点 {:.6}(打ち切り前 {:.6})",
            out.stop, out.states_expanded, out.states_generated, out.best_score, out.start_score
        );
        assert_eq!(
            out.stop,
            SearchStop::StateCap,
            "{name}: 打ち切りが起きていない"
        );
        assert_eq!(out.states_expanded, 1, "{name}: 広げた状態の数が予算と違う");
        assert!(
            !out.steps.is_empty(),
            "{name}: 打ち切りで手順が空になった。そこまでの最良を返していない"
        );
        assert!(
            out.best_score <= out.start_score,
            "{name}: 打ち切りで返した手順が、折らないときより悪い"
        );
        // 打ち切りで返した手は、最初の手の中でいちばん点数が良いものであること。
        //
        // ここで数え直す「最初の手」は、探索が候補にする手と**同じ集め方**で
        // なければならない。以前は `FoldSession::verified_moves` を使っていたが、
        // あれは作業18の見積もりに入った手だけを返す。その見積もりは
        // **上限とも下限とも言えない**ことが作業21で実測されている
        // (`tests/enumerate.rs` の
        // `the_estimate_from_task_18_is_neither_an_upper_nor_a_lower_bound`)。
        //
        // 正規再測定(2026-08-28、debugビルド): 折り鶴でstrictに折れる初手は
        // 27件中、見積もり外の手3(`y = 0.5`)だけだった。旧手16は入力CPで
        // 一般制約2/37違反・物理破棄4組（ほかに表示marker 1）のため拒否される。見積もり内だけで数え直すと
        // 有効な手を0件として取りこぼすので、ここでは全FoldLineを同じstrict経路で見る。
        //
        // **主張は緩めていない。** 「打ち切りで返した1手目は、最初に折れる手の
        // 中でいちばん点数が良い」という条件はそのままで、比べる相手の集め方だけを
        // 探索に合わせた。
        let first = FoldSession::new(&doc).expect("折り始められない");
        let mut best: Option<(f64, usize)> = None;
        let mut counted = 0usize;
        for fold_line in first.fold_lines() {
            let Some(mv) = first.verify_move(fold_line.id, tight.rank_scan) else {
                continue;
            };
            let mut next = first.clone();
            if next.apply(&mv).is_err() {
                continue;
            }
            counted += 1;
            let g = finish_gaps(&goal.target, &goal.measure(next.document()));
            let s = GapWeights::DEFAULT.score(&g);
            if best.is_none_or(|(b, _)| s < b) {
                best = Some((s, mv.movement().id));
            }
        }
        println!("   {name}: 最初に折れる手を {counted} 通り数え直した");
        let (best_score, best_id) = best.expect("折れる手が無い");
        assert_eq!(
            out.steps[0].mv.id, best_id,
            "{name}: 打ち切りで返した1手目が、最初の手の中でいちばん良いものではない"
        );
        assert!(
            (out.steps[0].score - best_score).abs() < SCORE_QUANTUM,
            "{name}: 打ち切りで返した点数が、測り直した点数と食い違う"
        );

        // 深さの上限でも同じように、そこまでの最良を返すこと。
        let shallow = SearchBudget {
            max_depth: 1,
            ..SearchBudget::DEFAULT
        };
        let out = search_to_finish(&session, &goal, GapWeights::DEFAULT, shallow);
        println!(
            "{name} 深さの打ち切り: 手順{:?} / {:?} / 深さの上限に当たった回数{} / 広げた状態{} / 作った状態{} / 点 {:.6}",
            out.steps.iter().map(|s| s.mv.id).collect::<Vec<_>>(),
            out.stop,
            out.depth_capped,
            out.states_expanded,
            out.states_generated,
            out.best_score
        );
        assert_eq!(
            out.stop,
            SearchStop::DepthCap,
            "{name}: 深さの打ち切りが起きていない"
        );
        assert!(
            out.depth_capped > 0,
            "{name}: 深さの上限に当たった回数が数えられていない"
        );
        assert_eq!(out.steps.len(), 1, "{name}: 深さ1の上限を超えて折っている");
    }
}

/// 合格条件5: 位置の指定を変えると、選ばれる手順が変わること。
///
/// 長さと太さの指定はそのままで、**位置だけ**を変えた2つの目標で探索する。
/// 変わらなければ「位置は効いていない」ことになるので、そのときは落ちる。
#[test]
fn changing_the_specified_position_changes_the_chosen_fold_order() {
    let doc = yakko();
    let a = goal_of_state(&doc, &YAKKO_STRICT_GOAL_ORDER);
    // 長さも太さも紙の場所の対応もそのままで、**位置だけ**を上下ひっくり返す。
    //
    // 2026-08-30の再測定では、通常の単線候補だけでは目標A `[2]` へは手順`[2]`、
    // 上下反転後は空手順だった。反転側は、残す側と上/下を明示した方向付き候補を
    // 使うと非空の手順を選べる。
    //
    // | 位置の変え方 | 返った手順 | 点 |
    // |---|---|---:|
    // | そのまま | `[2]` | 0.000000 |
    // | **上下反転** | **`[2, 62, 35, 48]`** | **0.666667** |
    //
    // 方向付き候補を通常候補へ無条件に混ぜると既存の非空探索を変えるため、実装は
    // 通常候補で空手順になった場合だけ同じ上限で方向付き候補を再探索する。
    let b = FoldGoal {
        target: FinishTarget {
            tips: a
                .target
                .tips
                .iter()
                .map(|t| TargetTip {
                    pos: t.pos.map(|p| TipPos2d::new(p.x, -p.y)),
                    ..*t
                })
                .collect(),
        },
        ..a.clone()
    };
    assert_eq!(
        a.sites, b.sites,
        "紙の場所の対応まで変えたら、位置だけの効き目を見たことにならない"
    );
    assert_eq!(a.body, b.body, "胴の中心まで変わっている");
    for (i, (ta, tb)) in a.target.tips.iter().zip(&b.target.tips).enumerate() {
        assert_eq!(ta.length, tb.length, "角{i}: 長さの指定まで変わっている");
        assert_eq!(ta.width, tb.width, "角{i}: 太さの指定まで変わっている");
        assert_ne!(ta.pos, tb.pos, "角{i}: 位置の指定が変わっていない");
    }

    let session = FoldSession::new(&doc).expect("折り始められない");
    let out_a = search_to_finish(&session, &a, GapWeights::DEFAULT, SearchBudget::DEFAULT);
    let out_b = search_to_finish(&session, &b, GapWeights::DEFAULT, SearchBudget::DEFAULT);
    let ids_a: Vec<usize> = out_a.steps.iter().map(|s| s.mv.id).collect();
    let ids_b: Vec<usize> = out_b.steps.iter().map(|s| s.mv.id).collect();
    println!(
        "やっこさん 位置そのまま: {ids_a:?}(点 {:.6}) / 位置を上下反転: {ids_b:?}(点 {:.6})",
        out_a.best_score, out_b.best_score
    );
    assert_ne!(
        ids_a, ids_b,
        "位置の指定を変えても手順が変わらなかった。位置が探索に効いていない"
    );
    assert!(
        !ids_b.is_empty(),
        "まわした側で手順が空になった。位置の効き目を見たことにならない"
    );
}

/// **太さの重みを下げた根拠**(段階1)。4つを同じ重みで足すと何が起きるかを固定する。
///
/// 太さの隔たりは、角が消えると `3.714 → 1.000` と大きく下がる
/// (`|0 − 0.3| ÷ 0.3 = 1.0`)。重みが同じだと、この下がり方が
/// 数・長さ・位置の悪化を上回り、**角を無くす手順のほうが良い点**になってしまう。
/// 既定の重み(太さ0.2)ではそうならない。
#[test]
fn weighting_all_four_gaps_equally_rewards_losing_the_tips() {
    let goal = weights_goal();
    let doc = yakko();
    let session = FoldSession::new(&doc).expect("折り始められない");
    let equal = GapWeights {
        count: 1.0,
        length: 1.0,
        width: 1.0,
        position: 1.0,
    };
    let out = search_to_finish(&session, &goal, equal, SearchBudget::DEFAULT);
    let ids: Vec<usize> = out.steps.iter().map(|s| s.mv.id).collect();
    println!(
        "やっこさん 重み全部1: 手順{ids:?} / 前 {} → 後 {}",
        line(&out.start_gaps, out.start_score),
        line(&out.best_gaps, out.best_score)
    );
    assert!(
        out.best_gaps.count > out.start_gaps.count,
        "重みを同じにしても角は減らなかった。太さの重みを下げた根拠が変わっている"
    );
    assert!(
        out.best_gaps.width < out.start_gaps.width,
        "角が減ったのに太さの隔たりが下がっていない"
    );

    // 既定の重みでは、同じ手順の点数が「折らない」より悪くなる。
    let losing = GapWeights::DEFAULT.score(&out.best_gaps);
    let flat = GapWeights::DEFAULT.score(&out.start_gaps);
    println!("   既定の重みで測り直すと {losing:.6}(折らないとき {flat:.6})");
    assert!(
        losing > flat,
        "既定の重みでも、角を無くす手順のほうが良い点になっている"
    );
}
