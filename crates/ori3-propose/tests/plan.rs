//! 作業18「折り順の探し方を、同じ標本で比べる」の測定。
//!
//! 2つの探し方(生成履歴を使う専用の計画 / 任意の展開図の汎用探索)を、
//! **同じ4つの展開図**へ掛けて、たどった状態の数・枝分かれの最大数・時間を測る。
//! **どちらを採るかはここでは決めない。** 決めるのは作業19のClaudeである。
//!
//! 標本4つの内訳と選んだ理由、測った値の読み方は
//! `scratchpad/propose-18-report.md` に書いた。
//!
//! 標本4は2026-08-16の利用者指示で、読み込んだ作品から「やっこさん」へ差し替えた。
//! このファイルの実測値は差し替え後のもので、判断6の根拠には折り鶴の値を使う。

use ori3_cp::local_violations;
use ori3_model::{CreasePattern, EdgeKind};
use ori3_propose::skeleton::{Skeleton, SkeletonNode};
use ori3_propose::{
    FoldPlanTrace, GenericPlanner, HistoryPlanner, MAX_LINES, Packing, ProposalResult,
    SearchLimits, SearchStats, StopReason, crease_lines, generate, pack,
};

/// 探索の打ち切り条件。どちらの方式にも同じ値を使う。
///
/// 実測(このファイルの測定を手元で実行、debugビルド): 折り順を1つ見つけるまでの
/// 探索は8通り(方式2×標本4)すべて 36状態以下・0.3ms以下で終わり、この上限には
/// 一度も掛かっていない。上限は、行き詰まったときに有限時間で止めるための歯止め。
const LIMITS: SearchLimits = SearchLimits {
    max_states: 200_000,
    max_millis: 20_000,
};

/// 枝分かれの広さを測るときの深さ。
///
/// 実測(debugビルド): 深さ4なら8通りすべてが上限に掛からず最後まで数え切れる
/// (最大は汎用探索・頭1尾1足4の 13,554 状態 / 47ms)。深さ5にすると同じ組が
/// 67,922 状態 / 314ms へ5.0倍に増え、測るたびの時間が桁で変わって比べにくく
/// なるため4にした。
const BREADTH_DEPTH: usize = 4;

/// 測る回数。同じ結果になることを確かめるため3回まわす(合格条件4)。
const RUNS: usize = 3;

/// 根に葉を`n`本ぶら下げた星形の骨格。
fn star(n: u32) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        nodes.push(SkeletonNode::new(i, Some(0), 1.0));
    }
    Skeleton { nodes }
}

/// 標本1: 紙の四隅に先端を1本ずつ(ロードマップの「四隅4葉」)。
/// 平らに折りにくい点0か所・表裏の食い違い0件という、いちばん素直な提案結果。
fn four_corners() -> ProposalResult {
    let s = star(4);
    let packing = Packing {
        scale: 0.5,
        centers: vec![
            (1, [0.0, 0.0]),
            (2, [1.0, 0.0]),
            (3, [1.0, 1.0]),
            (4, [0.0, 1.0]),
        ],
        violation: 0.0,
        circles: Vec::new(),
    };
    generate(&s, &packing, 1.0, 1.0).expect("四隅4葉の生成に失敗した")
}

/// 標本2: 「頭1・尾1・足4」(紙1×1、seed 2026、やり直し8回、先頭候補)。
/// ロードマップが受け入れの必須標本にしている形。
fn head_tail_four_legs() -> ProposalResult {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    nodes.push(SkeletonNode::new(1, Some(0), 1.0));
    nodes.push(SkeletonNode::new(2, Some(0), 1.0));
    for i in 3..=6 {
        nodes.push(SkeletonNode::new(i, Some(0), 0.7));
    }
    let s = Skeleton { nodes };
    let ps = pack(&s, 1.0, 1.0, 2026, 8);
    assert!(!ps.is_empty(), "頭1・尾1・足4の配置に失敗した");
    generate(&s, &ps[0], 1.0, 1.0).expect("頭1・尾1・足4の生成に失敗した")
}

/// 標本4: やっこさん(座布団折り2回)。既存の作品で、提案が作ったものではない。
///
/// 展開図は `crates/ori3-rigid/tests/acceptance_yakko.rs` の `yakko_cp()` と同じ
/// 線を同じ順に引いて組み立てる。折り線の引き方が決まっているので、読み込む
/// ファイルを持たずに毎回同じ展開図になる。
fn yakko() -> CreasePattern {
    let mut cp = ori3_model::Document::new(ori3_model::Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    })
    .cp;
    // 座布団折り1回目: 辺の中点を結ぶ谷折り4本。
    let (m1, m2, m3, m4) = ([0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]);
    for (a, b) in [(m1, m2), (m2, m3), (m3, m4), (m4, m1)] {
        ori3_cp::insert_segment(&mut cp, a, b, EdgeKind::Valley);
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
            ori3_cp::insert_segment(&mut cp, a, b, kind);
        }
    }
    cp
}

/// 記録した展開図を読む(標本3)。
fn load_cp(name: &str) -> CreasePattern {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path} を読めない: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path} を読み解けない: {e}"))
}

/// 標本の1件ぶん。
struct Sample {
    name: &'static str,
    cp: CreasePattern,
    /// 提案が作った展開図だけが持つ追跡情報。作品を読み込んだ展開図では `None`。
    trace: Option<FoldPlanTrace>,
}

fn samples() -> Vec<Sample> {
    let a = four_corners();
    let b = head_tail_four_legs();
    vec![
        Sample {
            name: "提案・四隅4葉",
            cp: a.cp,
            trace: Some(a.trace),
        },
        Sample {
            name: "提案・頭1尾1足4",
            cp: b.cp,
            trace: Some(b.trace),
        },
        // 既存の作品。`apps/desktop/src/lib/__fixtures__/crane.json` の展開図を写した。
        Sample {
            name: "作品・折り鶴",
            cp: load_cp("cp-crane.json"),
            trace: None,
        },
        // 既存の作品。座布団折り2回で、提案が作ったものではない。
        Sample {
            name: "作品・やっこさん",
            cp: yakko(),
            trace: None,
        },
    ]
}

fn creases_of(cp: &CreasePattern) -> usize {
    cp.edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .count()
}

fn stop_text(stop: StopReason) -> &'static str {
    match stop {
        StopReason::Completed => "最後まで到達",
        StopReason::NoFirstMove => "最初の手が0",
        StopReason::Exhausted => "行ける先を全部見た",
        StopReason::StateCap => "状態数で打ち切り",
        StopReason::TimeCap => "時間で打ち切り",
    }
}

/// 追跡情報が無い展開図でも同じ道具で測れるよう、空の追跡情報を渡す。
fn history_of(s: &Sample) -> HistoryPlanner {
    match &s.trace {
        Some(t) => HistoryPlanner::new(&s.cp, t),
        None => HistoryPlanner::new(&s.cp, &FoldPlanTrace::default()),
    }
}

/// 3回まわして、時間以外が毎回同じであることを確かめてから返す。
/// 返す時間は3回の中央値。
fn measure_thrice(name: &str, method: &str, mut run: impl FnMut() -> SearchStats) -> SearchStats {
    let all: Vec<SearchStats> = (0..RUNS).map(|_| run()).collect();
    for (i, s) in all.iter().enumerate().skip(1) {
        assert!(
            all[0].same_result(s),
            "{name} / {method}: {}回目の結果が1回目と違う\n1回目: {:?}\n{}回目: {s:?}",
            i + 1,
            all[0],
            i + 1
        );
    }
    let mut times: Vec<f64> = all.iter().map(|s| s.millis).collect();
    times.sort_by(f64::total_cmp);
    let median = times[RUNS / 2];
    let mut out = all[0];
    out.millis = median;
    out
}

// ---------------------------------------------------------------------------
// 合格条件2・3・4: 方式2×展開図4×3項目 = 24個の値
// ---------------------------------------------------------------------------

/// 2つの探し方を同じ4標本で測る。数え方は次のとおり。
///
/// - たどった状態の数: 折り順を1つ見つけるまでに入った、異なる状態の数(最初を含む)。
/// - 枝分かれの最大数: 1つの状態から列挙できた手の数の最大。
/// - 時間: 3回まわした中央値(debugビルド)。
///
/// この検査は**採否を決めない**。数えて表に出すだけである。
#[test]
fn compare_two_ways_of_searching_the_fold_order() {
    println!("\n### 表A: 折り順を1つ見つけるまで(24個の値)\n");
    println!(
        "| 標本 | 折り目 | まとまり | 方式 | 最初の手 | たどった状態 | 枝分かれ最大 | 時間ms | 手数 | 終わり方 |"
    );
    println!("|---|---:|---:|---|---:|---:|---:|---:|---:|---|");

    let all = samples();
    let mut breadth_rows: Vec<String> = Vec::new();
    let mut context_rows: Vec<String> = Vec::new();
    let mut generic_first_zero: Vec<&str> = Vec::new();
    let mut history_first_zero: Vec<&str> = Vec::new();

    for s in &all {
        let creases = creases_of(&s.cp);
        let lines = crease_lines(&s.cp);
        assert!(
            lines.len() <= MAX_LINES,
            "{}: 折り線のまとまりが{}本で上限{MAX_LINES}を超えた",
            s.name,
            lines.len()
        );

        let generic = GenericPlanner::new(&s.cp);
        let history = history_of(s);

        let h = measure_thrice(s.name, "生成履歴", || history.measure(LIMITS));
        let g = measure_thrice(s.name, "汎用探索", || generic.measure(LIMITS));
        if h.first_moves == 0 {
            history_first_zero.push(s.name);
        }
        if g.first_moves == 0 {
            generic_first_zero.push(s.name);
        }

        for (method, st) in [("生成履歴", h), ("汎用探索", g)] {
            println!(
                "| {} | {creases} | {} | {method} | {} | {} | {} | {:.2} | {} | {} |",
                s.name,
                st.lines,
                st.first_moves,
                st.states,
                st.max_branching,
                st.millis,
                st.plan_len
                    .map_or_else(|| "-".to_string(), |n| n.to_string()),
                stop_text(st.stop),
            );
        }

        let hb = measure_thrice(s.name, "生成履歴(広さ)", || {
            history.measure_breadth(BREADTH_DEPTH, LIMITS)
        });
        let gb = measure_thrice(s.name, "汎用探索(広さ)", || {
            generic.measure_breadth(BREADTH_DEPTH, LIMITS)
        });
        for (method, st) in [("生成履歴", hb), ("汎用探索", gb)] {
            breadth_rows.push(format!(
                "| {} | {method} | {} | {} | {:.2} | {} |",
                s.name,
                st.states,
                st.max_branching,
                st.millis,
                stop_text(st.stop),
            ));
        }

        context_rows.push(format!(
            "| {} | {} | {} | {} | {} | {} | {} |",
            s.name,
            generic.vertex_count(),
            generic.relaxed_vertices(0),
            local_violations(&s.cp).len(),
            history.lines_without_role(),
            history.lines_with_many_roles(),
            s.trace.as_ref().map_or(0, |t| t.molecules.len()),
        ));
    }

    println!("\n### 表B: 最初の{BREADTH_DEPTH}手までに行ける状態を全部数える(24個の値)\n");
    println!("| 標本 | 方式 | たどった状態 | 枝分かれ最大 | 時間ms | 終わり方 |");
    println!("|---|---|---:|---:|---:|---|");
    for row in &breadth_rows {
        println!("{row}");
    }

    println!("\n### 表C: 判断の材料になる内訳\n");
    println!(
        "| 標本 | 条件を課した点 | うち規則をゆるめた点(初期) | 平らに折りにくい点 | 役目なしのまとまり | 役目を兼ねるまとまり | 分子 |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for row in &context_rows {
        println!("{row}");
    }

    println!("\n汎用探索で最初の手が0だった標本: {generic_first_zero:?}");
    println!("生成履歴で最初の手が0だった標本: {history_first_zero:?}");
}

// ---------------------------------------------------------------------------
// 合格条件3: 固定の展開図4つで、次の手を1件以上列挙できたか
// ---------------------------------------------------------------------------

/// 汎用探索は4標本すべてで次の手を列挙でき、折り順も最後まで見つかる。
///
/// 実測(debugビルド): 最初の手は 5 / 22 / 18 / 12 件、手数は 9 / 35 / 34 / 16。
#[test]
fn the_generic_search_finds_moves_on_all_four_samples() {
    for s in &samples() {
        let planner = GenericPlanner::new(&s.cp);
        let stats = planner.measure(LIMITS);
        assert!(
            stats.first_moves >= 1,
            "{}: 汎用探索が最初の手を1つも列挙できなかった",
            s.name
        );
        assert_eq!(
            stats.plan_len,
            Some(stats.lines),
            "{}: 汎用探索が全部の折り線を折り切る順番を見つけられなかった",
            s.name
        );
        assert_eq!(stats.stop, StopReason::Completed, "{}", s.name);
    }
}

/// 生成履歴を使う方式は、提案が作った展開図でしか手を列挙できない。
///
/// 利用者が自分で描いた展開図・既存の作品を読み込んだ展開図には追跡情報が無いため、
/// 折り線の役目が1つも決まらず、最初の一手も出せない。作業19の判断材料として
/// **この限界を検査として残す**(実測: 折り鶴34本・やっこさん16本すべてが役目なし)。
#[test]
fn the_history_plan_only_works_on_crease_patterns_the_proposal_made() {
    for s in &samples() {
        let planner = history_of(s);
        let stats = planner.measure(LIMITS);
        match &s.trace {
            Some(_) => {
                assert!(
                    stats.first_moves >= 1,
                    "{}: 提案が作った展開図なのに最初の手が0",
                    s.name
                );
                assert_eq!(
                    planner.lines_without_role(),
                    0,
                    "{}: 役目の付かない折り線が残った",
                    s.name
                );
                assert_eq!(
                    stats.plan_len,
                    Some(stats.lines),
                    "{}: 全部の折り線を折り切る順番を見つけられなかった",
                    s.name
                );
            }
            None => {
                assert_eq!(
                    stats.first_moves, 0,
                    "{}: 追跡情報が無いのに手が出た。この検査の前提が崩れている",
                    s.name
                );
                assert_eq!(stats.stop, StopReason::NoFirstMove, "{}", s.name);
                assert_eq!(
                    planner.lines_without_role(),
                    stats.lines,
                    "{}: 追跡情報が無いのに役目の付いた折り線がある",
                    s.name
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 土台の点検
// ---------------------------------------------------------------------------

/// 折り目を「一度に折るまとまり」へ分けたとき、取りこぼしも重複も無いこと。
#[test]
fn crease_lines_cover_every_crease_exactly_once() {
    for s in &samples() {
        let lines = crease_lines(&s.cp);
        let mut ids: Vec<u32> = lines.iter().flat_map(|l| l.edges.iter().copied()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(
            before,
            ids.len(),
            "{}: 同じ辺が2つのまとまりに入った",
            s.name
        );
        assert_eq!(
            ids.len(),
            creases_of(&s.cp),
            "{}: まとまりに入らなかった折り目がある",
            s.name
        );
        for l in &lines {
            assert!(
                matches!(l.kind, EdgeKind::Mountain | EdgeKind::Valley),
                "{}: 折らない線がまとまりに入った",
                s.name
            );
        }
    }
}

/// まとまりの番号の付き方が毎回同じであること(決定性の土台)。
#[test]
fn crease_lines_are_numbered_the_same_way_every_time() {
    for s in &samples() {
        let first = crease_lines(&s.cp);
        for _ in 0..RUNS {
            assert_eq!(
                first,
                crease_lines(&s.cp),
                "{}: 番号の付き方が変わった",
                s.name
            );
        }
    }
}
