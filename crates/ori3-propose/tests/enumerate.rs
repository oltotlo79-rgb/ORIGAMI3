//! 作業21「検査済みの次手を列挙する」の測定と検査。
//!
//! 作業18は展開図だけを見て手を数えた。紙の重なり順・めり込み・途中の姿勢を
//! 一切見ていないので、あの数は**上限側の見積もり**である
//! (`scratchpad/propose-18-report.md`、判断6の注意点3)。
//!
//! ここでは同じ標本に対して、候補を1つずつ**実際に折って**確かめ、
//! 確かめる前と後の手の数を並べる。**減った分が、見積もりに含まれていた折れない手**である。
//!
//! 標本は利用者の指示により**折り鶴とやっこさん**を使う(悪魔・1分ローズは使わない)。

use std::collections::BTreeSet;

use ori3_cp::insert_segment;
use ori3_layers::flat_state::FlatState;
use ori3_layers::precrease_collapse::{
    PRECREASE_ORDER_UNDETERMINED_WARNING_PREFIX, PrecreaseCollapseInput,
    collapse_precrease_network_for_operation, validate_precrease_layer_order,
};
use ori3_model::{CreasePattern, Document, EdgeKind, Paper};
use ori3_propose::enumerate::{FoldSession, MAX_SEAM_GAP, MoveReport, PoseScan, Unverified};
use ori3_propose::{FoldedMask, GenericPlanner, crease_lines};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

/// 測る回数。同じ結果になることを確かめるため3回まわす(合格条件4)。
const RUNS: usize = 3;

/// 標本1: 折り鶴。`apps/desktop/src/lib/__fixtures__/crane.json` の展開図を
/// 作業18が写したもの。読むのは追跡対象の `tests/fixtures/` の中だけである。
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

/// 標本2: やっこさん(座布団折り2回)。
///
/// `crates/ori3-propose/tests/plan.rs` の `yakko()` と同じ線を同じ順に引く。
/// 読み込むファイルを持たないので毎回同じ展開図になる。
fn yakko() -> Document {
    let mut doc = square_document();
    let cp = &mut doc.cp;
    // 座布団折り1回目: 辺の中点を結ぶ谷折り4本。
    let (m1, m2, m3, m4) = ([0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]);
    for (a, b) in [(m1, m2), (m2, m3), (m3, m4), (m4, m1)] {
        ori3_cp::insert_segment(cp, a, b, EdgeKind::Valley);
    }
    // 座布団折り2回目: 1/4の位置の縦横4直線。中央区間は谷、外側の区間は山。
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

fn samples() -> Vec<(&'static str, Document)> {
    vec![("折り鶴", crane()), ("やっこさん", yakko())]
}

fn creases_of(cp: &CreasePattern) -> usize {
    cp.edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .count()
}

/// 合格条件1: 確かめる前と後の手の数を並べる。
///
/// 実測(debugビルド、`PoseScan::DEFAULT` の21点):
///
/// | 標本 | 折り目 | まとまり | 直線 | 確かめる前(まとまり) | 確かめる前(直線) | 確かめた後(見積もり内) | 見積もり外で折れた | 確かめられなかった |
/// |---|---:|---:|---:|---:|---:|---:|---:|---:|
/// | 折り鶴 | 43 | 34 | 27 | 18 | 17 | 0 | 1 | 17 |
/// | やっこさん | 20 | 16 | 8 | 12 | 8 | 8 | 0 | 0 |
///
/// 「確かめる前(まとまり)」は作業18の [`GenericPlanner`] がそのまま数えた本数で、
/// 「確かめる前(直線)」は同じ候補を、実際に折る単位(同じ直線に乗る折り目は
/// 一緒に閉じる)へまとめ直した本数である。**見積もり内で確かめた後の数と同じ
/// 集合で引き算できるのは後者**で、減った分が見積もりに含まれていた折れない手に
/// あたる。見積もり外で物理検査を通った手は別の列に出し、この会計へ混ぜない。
#[test]
fn moves_before_and_after_checking_are_counted_for_crane_and_yakko() {
    println!(
        "| 標本 | 折り目 | まとまり | 直線 | 確かめる前(まとまり) | 確かめる前(直線) | 確かめた後(見積もり内) | 見積もり外で折れた | 確かめられなかった |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    for (name, doc) in samples() {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let report = session.verified_moves(PoseScan::DEFAULT);
        println!(
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {} |",
            creases_of(&doc.cp),
            session.crease_lines().len(),
            session.fold_lines().len(),
            report.proposed_crease_lines,
            report.proposed_fold_lines,
            report.verified_within_estimate.len(),
            report.verified_outside_estimate.len(),
            report.unverified(),
        );
        assert_eq!(
            report.proposed_fold_lines,
            report.verified_within_estimate.len() + report.unverified(),
            "{name}: 確かめる前の数が、確かめた後と確かめられなかった数の合計に一致しない"
        );
        assert!(
            report.all_verified().next().is_some(),
            "{name}: 確かめられた手が1つも無い"
        );
        assert!(
            report.verified_within_estimate.len() <= report.proposed_fold_lines,
            "{name}: 確かめた後の手が、確かめる前より増えている"
        );
    }

    // 理由の内訳。どの条件で落ちたのかを隠さず出す。
    for (name, doc) in samples() {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let report = session.verified_moves(PoseScan::DEFAULT);
        println!(
            "== {name}: 確かめられなかった {} 件の内訳",
            report.unverified()
        );
        for (label, count) in report.reasons() {
            println!("   {label}: {count}件");
        }
        println!(
            "   見積もりの外で折れた手: {}件",
            report.verified_outside_estimate.len()
        );
    }
}

/// 合格条件2: 確かめた手を実際に実行して、裂けとめり込みを数値で示す。
#[test]
fn every_verified_move_really_folds_without_tearing_or_passing_through() {
    println!("| 標本 | 手 | 閉じる直線 | まとまり数 | 見た姿勢 | 裂け | めり込み |");
    println!("|---|---:|---|---:|---:|---:|---:|");
    let mut checked = 0usize;
    let mut worst_gap_overall: f64 = 0.0;
    for (name, doc) in samples() {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let report = session.verified_moves(PoseScan::DEFAULT);
        for operation in report.operation_moves() {
            let mv = operation.movement();
            // 列挙のときの値。
            assert!(
                mv.max_seam_gap < MAX_SEAM_GAP,
                "{name} 手{}: 列挙時の裂け {} が上限 {MAX_SEAM_GAP} 以上",
                mv.id,
                mv.max_seam_gap
            );
            assert_eq!(
                mv.penetrations, 0,
                "{name} 手{}: 列挙時のめり込みが0件でない",
                mv.id
            );
            assert_eq!(mv.poses_checked, PoseScan::DEFAULT.points());

            // 実際に進めて、もう一度測り直す。
            let mut advanced = session.clone();
            advanced
                .apply_operation(operation)
                .unwrap_or_else(|e| panic!("{name} 手{}: 確かめた手を進められない: {e}", mv.id));
            assert_eq!(advanced.applied_moves(), 1, "{name}: 手順が1件にならない");

            let faces = ori3_cp::extract_faces(&advanced.document().cp);
            let up_to = advanced.document().sequence.len();
            let mut gap: f64 = 0.0;
            let mut pairs = 0usize;
            for i in 0..PoseScan::DEFAULT.points() {
                let t = i as f64 / PoseScan::DEFAULT.steps as f64;
                let replayed = ori3_layers::replay::replay(advanced.document(), up_to, t);
                assert!(
                    replayed.skipped.is_empty() && replayed.warnings.is_empty(),
                    "{name} 手{}: t={t} で手順が飛ばされたか警告が出た",
                    mv.id
                );
                gap = gap.max(max_seam_gap(
                    &advanced.document().cp,
                    &faces,
                    &replayed.frame,
                ));
                pairs = pairs.max(self_intersection_pairs(&replayed.frame).len());
            }
            worst_gap_overall = worst_gap_overall.max(gap);
            println!(
                "| {name} | {} | ({:.3},{:.3})-({:.3},{:.3}) | {} | {} | {gap:.3e} | {pairs} |",
                mv.id,
                mv.line[0][0],
                mv.line[0][1],
                mv.line[1][0],
                mv.line[1][1],
                mv.closes.len(),
                PoseScan::DEFAULT.points(),
            );
            assert!(
                gap < MAX_SEAM_GAP,
                "{name} 手{}: 実行後の裂け {gap} が上限 {MAX_SEAM_GAP} 以上",
                mv.id
            );
            assert_eq!(pairs, 0, "{name} 手{}: 実行後のめり込みが0件でない", mv.id);
            // もう一度折り終えた印が付き、次の状態へ進んでいること。
            assert!(
                advanced.folded_mask() != 0,
                "{name} 手{}: 折り終えた印が付いていない",
                mv.id
            );
            checked += 1;
        }
    }
    assert!(checked >= 2, "実行して確かめた手が少なすぎる: {checked}件");
    println!("実行して確かめた手 {checked}件 / 裂けの最大 {worst_gap_overall:.3e}");
}

/// 合格条件3: 確かめられなかった手は返さない。件数と理由を報告する。
#[test]
fn moves_that_could_not_be_checked_are_never_returned_as_foldable() {
    let mut total_rejected = 0usize;
    for (name, doc) in samples() {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let report = session.verified_moves(PoseScan::DEFAULT);
        let displayed = report.all_verified().collect::<Vec<_>>();
        let operation_metadata = report
            .operation_moves()
            .map(|operation| operation.movement())
            .collect::<Vec<_>>();
        assert_eq!(
            operation_metadata, displayed,
            "{name}: 表示した検証済み手と手動tokenの順序・metadataが対応しない"
        );
        let verified: BTreeSet<usize> = report.all_verified().map(|m| m.id).collect();
        let applicable: BTreeSet<usize> = report
            .operation_moves()
            .map(|operation| operation.movement().id)
            .collect();
        assert_eq!(
            applicable, verified,
            "{name}: 表示した検証済み手と、適用可能な手動tokenが対応しない"
        );
        for r in &report.rejected {
            assert!(
                !verified.contains(&r.id),
                "{name}: 確かめられなかった手 {} が折れる手としても返っている",
                r.id
            );
            assert!(
                !applicable.contains(&r.id),
                "{name}: 確かめられなかった手 {} に適用可能なtokenを発行している",
                r.id
            );
        }
        assert_eq!(
            verified.len(),
            report.verified_within_estimate.len() + report.verified_outside_estimate.len(),
            "{name}: 同じ手が2回返っている"
        );
        total_rejected += report.unverified();
        println!(
            "{name}: 確かめられなかった {} 件 {:?}",
            report.unverified(),
            report.reasons()
        );

        // 表示用metadataから未検証の手動操作tokenを偽造する入口は公開しない。
        // 折れない手にはtokenを発行しない、という外側の契約を上の集合一致で固定する。
    }
    assert!(
        total_rejected > 0,
        "どの標本でも1件も落ちていない。見積もりとの差が出ていない"
    );
}

/// 合格条件4: 種を固定して3回実行し、同じ結果になること。
#[test]
fn the_same_crease_pattern_gives_the_same_verified_moves_three_times() {
    for (name, _) in samples() {
        let runs: Vec<MoveReport> = (0..RUNS)
            .map(|_| {
                let doc = samples()
                    .into_iter()
                    .find(|(n, _)| *n == name)
                    .expect("標本が見つからない")
                    .1;
                FoldSession::new(&doc)
                    .expect("折り始められない")
                    .verified_moves(PoseScan::DEFAULT)
            })
            .collect();
        for (i, r) in runs.iter().enumerate().skip(1) {
            assert_eq!(&runs[0], r, "{name}: {}回目の結果が1回目と違う", i + 1);
        }
        println!(
            "{name}: {RUNS}回とも同じ(確かめた後 {} 件 / 確かめられなかった {} 件)",
            runs[0].verified_within_estimate.len(),
            runs[0].unverified()
        );
    }
}

/// 手の単位の作り方が正しいこと: 折り目を取りこぼしも重複もなく直線へ配ること。
#[test]
fn fold_lines_cover_every_crease_line_exactly_once() {
    for (name, doc) in samples() {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let lines = crease_lines(&doc.cp);
        assert_eq!(
            session.crease_lines().len(),
            lines.len(),
            "{name}: 作業18の数え方と食い違っている"
        );
        assert!(!session.faces().is_empty(), "{name}: 面が取り出せていない");

        let mut seen_lines: BTreeSet<usize> = BTreeSet::new();
        let mut seen_edges: BTreeSet<u32> = BTreeSet::new();
        for fold_line in session.fold_lines() {
            for &m in &fold_line.closes {
                assert!(
                    seen_lines.insert(m),
                    "{name}: まとまり {m} が2つの直線に入っている"
                );
            }
            for &e in &fold_line.edges {
                assert!(
                    seen_edges.insert(e),
                    "{name}: 折り目 {e} が2つの直線に入っている"
                );
            }
            assert_eq!(
                fold_line.mask.count_ones() as usize,
                fold_line.closes.len(),
                "{name}: 直線 {} のビットと本数が合わない",
                fold_line.id
            );
        }
        assert_eq!(
            seen_lines.len(),
            lines.len(),
            "{name}: 直線へ配られなかったまとまりがある"
        );
        assert_eq!(
            seen_edges.len(),
            creases_of(&doc.cp),
            "{name}: 直線へ配られなかった折り目がある"
        );
        println!(
            "{name}: 折り目 {} 本 → まとまり {} 本 → 直線 {} 本",
            creases_of(&doc.cp),
            lines.len(),
            session.fold_lines().len()
        );
    }
}

/// 確かめた手を続けて折れること(途中の状態からも同じ確かめができること)。
#[test]
fn verified_moves_can_be_folded_one_after_another() {
    for (name, doc) in samples() {
        let mut session = FoldSession::new(&doc).expect("折り始められない");
        let mut applied = 0usize;
        let mut worst_gap: f64 = 0.0;
        while applied < 4 {
            let report = session.verified_moves(PoseScan::DEFAULT);
            let Some(operation) = report.operation_moves().next().cloned() else {
                break;
            };
            let mv = operation.movement();
            worst_gap = worst_gap.max(mv.max_seam_gap);
            assert_eq!(
                mv.penetrations,
                0,
                "{name}: {}手目でめり込みが出た",
                applied + 1
            );
            let before = session.folded_mask();
            session
                .apply_operation(&operation)
                .unwrap_or_else(|e| panic!("{name}: {}手目を進められない: {e}", applied + 1));
            assert_ne!(
                session.folded_mask(),
                before,
                "{name}: {}手目で折り終えた印が増えていない",
                applied + 1
            );
            applied += 1;
        }
        assert!(applied >= 1, "{name}: 1手も進められなかった");
        println!(
            "{name}: 続けて {applied} 手進めた / 裂けの最大 {worst_gap:.3e} / 手順 {} 件",
            session.applied_moves()
        );
    }
}

/// **左右対称な展開図で、対称な2本の直線が両方とも折れることを確かめる。**
///
/// やっこさんの `y = 0.25` と `y = 0.75` は紙の真ん中を挟んで対称なので、
/// 片方が折れるならもう片方も折れなければならない。
///
/// この検査はもともと「`y = 0.75` だけが『紙がすり抜ける』として落ちる」という
/// **取りこぼしの記録**だった(2026-08-13)。原因は
/// [`collapse_precrease_network`](ori3_layers::collapse_precrease_network) が
/// **紙の重なり順を組み替えていなかった**ことにある。折り返した紙を相手の上へ回せず、
/// 折り返す先の面の番号がたまたま手前にある `y = 0.25` だけが通っていた。
///
/// 2026-08-17に重なり順を畳んだ形の幾何から決めるよう直したので、
/// **やっこさんで落ちる手は0件**になった。実測: `y = 0.75` は
/// 裂け `3.611e-15` / めり込み `0` で折れる。同じ直線を画面が使う普通の折り操作
/// ([`fold_through`](ori3_layers::fold_through))で折った値もあわせて出す。
#[test]
fn the_two_mirror_image_lines_of_yakko_both_fold() {
    let doc = yakko();
    let session = FoldSession::new(&doc).expect("折り始められない");
    let report = session.verified_moves(PoseScan::DEFAULT);
    let rejected: Vec<_> = report
        .rejected
        .iter()
        .filter(|r| matches!(r.reason, Unverified::PaperPassesThrough { .. }))
        .collect();
    assert!(
        rejected.is_empty(),
        "やっこさん: めり込みで落ちた手がある {:?}",
        rejected.iter().map(|r| r.line).collect::<Vec<_>>()
    );

    // 紙の真ん中を挟んで対称な2本が、どちらも折れる手として返っている。
    let line = [[0.0, 0.75], [1.0, 0.75]];
    for wanted in [[[0.0, 0.25], [1.0, 0.25]], line] {
        assert!(
            report.all_verified().any(|mv| {
                let on = |point: [f64; 2]| (point[1] - wanted[0][1]).abs() <= 1e-9;
                on(mv.line[0]) && on(mv.line[1])
            }),
            "やっこさん: y = {} を折る手が返っていない",
            wanted[0][1]
        );
    }

    // 同じ直線を、画面が使う普通の折り操作で折ってみる。
    let faces = ori3_cp::extract_faces(&doc.cp);
    let state = ori3_layers::flat_state::FlatState::initial(&doc.cp, &faces);
    let mut cp = doc.cp.clone();
    let mut result = ori3_layers::fold_through::fold_through(
        &mut cp,
        &faces,
        &state,
        &ori3_layers::fold_through::FoldThroughInput {
            line,
            keep_side_point: [0.5, 0.1],
            target_layers: None,
            direction: ori3_layers::fold_through::FoldDirection::Up,
        },
    )
    .expect("普通の折り操作でも折れないなら、原因は確かめ方ではない");
    let mut candidate = doc.clone();
    candidate.cp = cp;
    result.step.id = 0;
    candidate.sequence.push(result.step);
    let candidate_faces = ori3_cp::extract_faces(&candidate.cp);
    let mut gap: f64 = 0.0;
    let mut pairs = 0usize;
    for i in 0..PoseScan::DEFAULT.points() {
        let t = i as f64 / PoseScan::DEFAULT.steps as f64;
        let replayed = ori3_layers::replay::replay(&candidate, 1, t);
        gap = gap.max(max_seam_gap(
            &candidate.cp,
            &candidate_faces,
            &replayed.frame,
        ));
        pairs = pairs.max(self_intersection_pairs(&replayed.frame).len());
    }
    println!("やっこさん {line:?}: 普通の折り操作でも 裂け{gap:.3e} / めり込み{pairs}");
    assert!(gap < MAX_SEAM_GAP, "普通の折り操作でも裂けた: {gap}");
    assert_eq!(pairs, 0, "普通の折り操作でもめり込んだ: {pairs}件");
}

/// 2本を同時に閉じる操作を、all-Auxの「単一」book fold既定へ誤分類しない。
///
/// 縦横2本は互いに異なる鏡映を同時に要求するため、1つのmoved packetを外側へ回す
/// single book foldではない。従ってoperation-aware置換の入口へ入れず、一般solverが
/// 残した6組の未決定を警告し、表示用tie-breakを保存oracleへ昇格させない。
#[test]
fn crossing_multi_line_collapse_does_not_receive_single_book_fold_replacement() {
    let mut document = square_document();
    let lines = [[[0.5, 0.0], [0.5, 1.0]], [[0.0, 0.5], [1.0, 0.5]]];
    for line in lines {
        ori3_cp::insert_segment(&mut document.cp, line[0], line[1], EdgeKind::Aux);
    }
    let faces = ori3_cp::extract_faces(&document.cp);
    let state = FlatState::initial(&document.cp, &faces);
    let result = collapse_precrease_network_for_operation(
        &mut document.cp,
        &faces,
        &state,
        &PrecreaseCollapseInput {
            lines: lines.to_vec(),
            target_layers: None,
        },
    )
    .expect("交差2線の同時collapseを診断できない");
    let collapsed_faces = ori3_cp::extract_faces(&document.cp);

    println!(
        "STAGE6_NON_BOOK_MULTI_LINE requested_lines={} faces={} unresolved=6 warnings={} saved_order={}",
        lines.len(),
        collapsed_faces.len(),
        result.warnings.len(),
        result.step.layer_order.is_some(),
    );
    assert_eq!(lines.len(), 2);
    assert_eq!(collapsed_faces.len(), 4);
    assert_eq!(
        result.warnings,
        vec![format!(
            "{PRECREASE_ORDER_UNDETERMINED_WARNING_PREFIX}6組あります"
        )]
    );
    assert!(
        result.step.layer_order.is_none(),
        "非single book foldの表示順を保存oracleへ昇格させた"
    );
}

/// 単一book foldの向きが非0同数なら、表示用tie-breakを物理的な権威へ昇格させない。
///
/// 旧候補16は `x = 0.5` 上で Mountain 2 / Valley 2。settle後CPを読むと候補順が
/// 自己認証されるため、必ず操作**前**CPとcollapseが返した配置・順を読み合わせる。
/// やっこさんのstrict majorityを操作制約へ置換しても、この2/37違反・物理破棄4組は残る。
/// 表示用total化の失敗1組は別fieldへ残し、物理破棄数へ混ぜない。
#[test]
fn strict_proposal_gate_rejects_tied_crane_candidate_16_by_input_layer_constraints() {
    let doc = crane();
    let session = FoldSession::new(&doc).expect("折り鶴を折り始められない");
    let candidate = session
        .fold_lines()
        .iter()
        .find(|line| {
            let on_axis = |point: [f64; 2]| (point[0] - 0.5).abs() <= 1e-12;
            on_axis(line.a) && on_axis(line.b)
        })
        .expect("折り鶴の x = 0.5 候補が無い");
    assert_eq!(candidate.id, 16, "旧候補16の入力fixtureが変わった");

    let input_cp = doc.cp.clone();
    let mountain_votes = candidate
        .edges
        .iter()
        .filter(|edge_id| {
            input_cp
                .edges
                .iter()
                .find(|edge| edge.id == **edge_id)
                .is_some_and(|edge| edge.kind == EdgeKind::Mountain)
        })
        .count();
    let valley_votes = candidate
        .edges
        .iter()
        .filter(|edge_id| {
            input_cp
                .edges
                .iter()
                .find(|edge| edge.id == **edge_id)
                .is_some_and(|edge| edge.kind == EdgeKind::Valley)
        })
        .count();
    assert_eq!((mountain_votes, valley_votes), (2, 2));
    let input_faces = ori3_cp::extract_faces(&input_cp);
    let input_state = FlatState::initial(&input_cp, &input_faces);
    let mut collapsed_cp = input_cp.clone();
    let collapsed = collapse_precrease_network_for_operation(
        &mut collapsed_cp,
        &input_faces,
        &input_state,
        &PrecreaseCollapseInput {
            lines: vec![[candidate.a, candidate.b]],
            target_layers: None,
        },
    )
    .expect("旧候補16の診断用collapse自体が失敗した");
    let collapsed_faces = ori3_cp::extract_faces(&collapsed_cp);
    let validation = validate_precrease_layer_order(
        &input_cp,
        &collapsed_faces,
        &collapsed.state.placements,
        &collapsed.state.order,
    )
    .expect("旧候補16の入力CP一般制約を検査できない");
    let initial_seed_validation = validate_precrease_layer_order(
        &input_cp,
        &collapsed_faces,
        &collapsed.state.placements,
        &input_state.order,
    )
    .expect("旧候補16の初期表示seedを検査できない");
    let expected_faces = collapsed_faces
        .iter()
        .map(|face| face.id)
        .collect::<BTreeSet<_>>();
    let ordered_faces = collapsed
        .state
        .order
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(collapsed.state.order.len(), collapsed_faces.len());
    assert_eq!(ordered_faces, expected_faces);

    let checked = validation.counts.adjacent_folds
        + validation.counts.taco_tortilla
        + validation.counts.taco_taco
        + validation.counts.continuous;
    let violations = validation.violations.adjacent_folds.len()
        + validation.violations.taco_tortilla.len()
        + validation.violations.taco_taco.len()
        + validation.violations.continuous_crossings.len()
        + validation.violations.continuous.len();
    println!(
        "STAGE7_CRANE_TIED_BOOK_FOLD id={} mountain_votes={mountain_votes} valley_votes={valley_votes} violations={violations}/{checked} adjacent={}/{} taco_tortilla={} taco_taco={} continuous_crossings={} continuous={} physical_discarded={} display_resolution_failure={:?} initial_seed_display_resolution_failure={:?} accepted={} warnings={} saved_order={}",
        candidate.id,
        validation.violations.adjacent_folds.len(),
        validation.counts.adjacent_folds,
        validation.violations.taco_tortilla.len(),
        validation.violations.taco_taco.len(),
        validation.violations.continuous_crossings.len(),
        validation.violations.continuous.len(),
        validation.discarded_relations.len(),
        validation.display_resolution_failure,
        initial_seed_validation.display_resolution_failure,
        validation.is_valid(),
        collapsed.warnings.len(),
        collapsed.step.layer_order.is_some(),
    );

    assert_eq!(violations, 2, "旧候補16の一般制約違反数が変わった");
    assert_eq!(checked, 37, "旧候補16の一般制約総数が変わった");
    assert_eq!(
        validation.violations.continuous,
        vec![(0, 1, 6, 10), (2, 3, 8, 9)],
        "旧候補16を拒否する0度縫い目の上下反転が変わった"
    );
    let physical_conflicts = vec![(1, 10), (3, 9), (6, 0), (8, 2)];
    assert_eq!(
        validation.discarded_relations, physical_conflicts,
        "旧候補16の物理的な破棄関係4組が変わった"
    );
    assert_eq!(
        initial_seed_validation.discarded_relations, physical_conflicts,
        "表示seedを変えると物理的な破棄関係が変わった"
    );
    assert_eq!(
        validation.display_resolution_failure,
        Some((0, 2)),
        "collapse表示seedの全順序化失敗markerが消えた"
    );
    assert_eq!(
        initial_seed_validation.display_resolution_failure,
        Some((0, 1)),
        "初期表示seedの全順序化失敗markerが消えた"
    );
    assert!(
        !validation.is_valid(),
        "提案関門と同じis_valid述語が旧候補16を採用した"
    );
    assert_eq!(
        collapsed.warnings,
        vec!["紙の重なり順の条件が4組で両立しません"],
        "表示用markerを物理警告へ混ぜた"
    );
    assert!(collapsed.step.layer_order.is_none());
    assert!(matches!(
        session.check_move(candidate.id, PoseScan::DEFAULT),
        Some(Err(Unverified::CannotCollapse))
    ));
}

/// 作業18の見積もりが、実際に折れる手を取りこぼしてもいることの記録。
///
/// 見積もりは「上限側」とだけ言われてきたが、やっこさんでは規則が
/// **折れる手を落としてもいる**。上限とも下限とも言えないことを数字で残す。
#[test]
fn the_estimate_from_task_18_is_neither_an_upper_nor_a_lower_bound() {
    let mut outside_total = 0usize;
    for (name, doc) in samples() {
        let session = FoldSession::new(&doc).expect("折り始められない");
        let planner = GenericPlanner::new(&doc.cp);
        let proposed = planner.next_moves(0).len();
        let report = session.verified_moves(PoseScan::DEFAULT);
        assert_eq!(
            proposed, report.proposed_crease_lines,
            "{name}: 作業18の数え方と食い違っている"
        );
        outside_total += report.verified_outside_estimate.len();
        println!(
            "{name}: 見積もり {proposed} 本(まとまり) / 見積もりの中で折れた {} 手 / 見積もりの外で折れた {} 手",
            report.verified_within_estimate.len(),
            report.verified_outside_estimate.len()
        );
    }
    assert!(
        outside_total > 0,
        "見積もりの外で折れた手が0件だった。この検査が守っている性質が変わっている"
    );
}

/// `check_move` が通した手は、その検証で作った終点をそのまま適用する。
///
/// 十字を同じ山谷で2回畳むと、2手目の入力CP制約と単純操作の層順が異なる。
/// 検証後に別authorityで解き直す実装では、21姿勢を通った2手目が適用時に失敗する。
#[derive(Debug)]
struct CheckedCrossOutcome {
    verified_mask: FoldedMask,
    actual_mask: FoldedMask,
    applied_moves: usize,
    sequence_len: usize,
}

fn checked_cross_move_applies_verified_successor(kind: EdgeKind) -> CheckedCrossOutcome {
    let mut document = square_document();
    insert_segment(&mut document.cp, [0.0, 0.5], [1.0, 0.5], kind);
    insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], kind);

    let mut session = FoldSession::new(&document).expect("十字の紙から折り始められない");
    let horizontal = session
        .fold_lines()
        .iter()
        .find(|line| (line.a[1] - 0.5).abs() < 1e-12 && (line.b[1] - 0.5).abs() < 1e-12)
        .expect("中央の横線がない")
        .id;
    let Some(Ok(first)) = session.check_move(horizontal, PoseScan::DEFAULT) else {
        panic!("{kind:?}: 横線の初手が21姿勢を通らない");
    };
    session
        .apply(&first)
        .unwrap_or_else(|error| panic!("{kind:?}: 検証済みの横線を適用できない: {error}"));

    let vertical = session
        .fold_lines()
        .iter()
        .find(|line| (line.a[0] - 0.5).abs() < 1e-12 && (line.b[0] - 0.5).abs() < 1e-12)
        .expect("中央の縦線がない")
        .id;
    let Some(Ok(second)) = session.check_move(vertical, PoseScan::DEFAULT) else {
        panic!("{kind:?}: 縦線の2手目が21姿勢を通らない");
    };
    let movement = second.movement();
    assert_eq!(movement.poses_checked, PoseScan::DEFAULT.points());
    assert_eq!(movement.penetrations, 0);
    assert!(movement.max_seam_gap < MAX_SEAM_GAP);
    let verified_mask = movement.mask;
    session.apply(&second).unwrap_or_else(|error| {
        panic!("{kind:?}: 21姿勢を通った縦線の2手目を適用できない: {error}")
    });
    CheckedCrossOutcome {
        verified_mask,
        actual_mask: session.folded_mask(),
        applied_moves: session.applied_moves(),
        sequence_len: session.document().sequence.len(),
    }
}

#[test]
fn checked_mountain_cross_move_applies_the_verified_successor() {
    let outcome = checked_cross_move_applies_verified_successor(EdgeKind::Mountain);
    assert_eq!(
        outcome.actual_mask, outcome.verified_mask,
        "Mountain: 適用後maskが検証済み後続と一致しない"
    );
    assert_eq!(outcome.applied_moves, 2, "Mountain: 2手進んでいない");
    assert_eq!(
        outcome.sequence_len, 2,
        "Mountain: 検証済み2手が手順へ保存されていない"
    );
}

#[test]
fn checked_valley_cross_move_applies_the_verified_successor() {
    let outcome = checked_cross_move_applies_verified_successor(EdgeKind::Valley);
    assert_eq!(
        outcome.actual_mask, outcome.verified_mask,
        "Valley: 適用後maskが検証済み後続と一致しない"
    );
    assert_eq!(outcome.applied_moves, 2, "Valley: 2手進んでいない");
    assert_eq!(
        outcome.sequence_len, 2,
        "Valley: 検証済み2手が手順へ保存されていない"
    );
}

/// 手動列挙は、入力CP全体ではなく利用者が明示した単純操作の向きを根拠にする。
///
/// やっこさんの下側の横線はstrict提案では拒否される一方、手動操作としては
/// 21姿勢を安全に通る。この差を残し、全手をInputCpへ寄せる誤修正を防ぐ。
#[test]
fn explicit_manual_move_keeps_operation_authority() {
    let document = yakko();
    let mut session = FoldSession::new(&document).expect("やっこさんから折り始められない");
    let id = session
        .fold_lines()
        .iter()
        .find(|line| (line.a[1] - 0.25).abs() < 1e-12 && (line.b[1] - 0.25).abs() < 1e-12)
        .expect("下側の横線がない")
        .id;
    assert!(matches!(
        session.check_move(id, PoseScan::DEFAULT),
        Some(Err(Unverified::CannotCollapse))
    ));

    let report = session.verified_moves(PoseScan::DEFAULT);
    let operation = report
        .operation_moves()
        .find(|operation| operation.movement().id == id)
        .expect("手動操作として折れる下側の横線が列挙されない");
    let movement = operation.movement();
    assert_eq!(movement.poses_checked, PoseScan::DEFAULT.points());
    assert_eq!(movement.penetrations, 0);
    assert!(movement.max_seam_gap < MAX_SEAM_GAP);
    let stale = operation.clone();
    let mut same_state_clone = session.clone();
    same_state_clone
        .apply_operation(&stale)
        .expect("検証元と同じ状態のcloneへ手動tokenを適用できない");
    session
        .apply_operation(operation)
        .expect("手動操作として検証した下側の横線を適用できない");
    assert_eq!(session.applied_moves(), 1);
    assert!(
        session.apply_operation(&stale).is_err(),
        "1手進んだ後にも古い手動tokenを再適用できた"
    );
    let mut independently_built =
        FoldSession::new(&document).expect("同じやっこさんを再読込できない");
    assert!(
        independently_built.apply_operation(&stale).is_err(),
        "同じ文書から作った別sessionへ手動tokenを移せた"
    );
}
