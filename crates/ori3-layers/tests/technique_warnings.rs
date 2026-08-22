//! 技法の警告を「折れなかった」と「折れた上での注意」に分ける規則の検査。
//!
//! 「止めずに警告する」(`docs/requirements-definition.md` §2 / `CLAUDE.md` §8)ので、
//! 警告が出たことを理由に折る動きを止めてはいけない。一方で
//! **折り上がりが指定と違ってしまう**警告まで通すと、折れない手順を返してしまう。
//! その線引きが [`warning_means_the_fold_was_not_as_requested`] である。

use ori3_cp::extract_faces;
use ori3_layers::flat_state::FlatState;
use ori3_layers::fold_through::{
    FoldDirection, FoldThroughInput, fold_through, warning_means_the_fold_was_not_as_requested,
};
use ori3_model::{Document, Paper};

/// 折り上がりが指定と違ってしまう警告だけを、折れなかった側に分ける。
///
/// 文言はすべて `crates/ori3-layers/src/` の実際の警告文から取っている。
/// 文言を書き換えたらこの検査が落ちるので、
/// `NOT_AS_REQUESTED_MARKS` の直し忘れに気づける。
#[test]
fn warnings_that_change_the_result_are_told_apart_from_advice() {
    // 折れなかった側(捨てる)
    for warning in [
        // techniques.rs: 山谷と重なり順が食い違い、手順から折り直すと形が変わる
        "この花弁折りでは、折り目2本の山谷と紙の重なり順が食い違います。このままでは展開図から折り直したときに形が変わります(指定のまま続行します)",
        // techniques.rs: 動きそのものを作れなかった
        "花弁折りの支点が決められません。中心線を引き直してください",
        // flat_motion.rs: 新しい面を動かせず置き去りにした
        "新しい面 7 の親面が特定できないため、動かさず元の配置のままにします",
        // flat_motion.rs / fold_through.rs: 指定した層が実行から外された
        "対象層 3 は現在の面に存在しないため除外しました",
        "対象層 3 は折り線の可動側に掛かっていないため除外しました",
    ] {
        assert!(
            warning_means_the_fold_was_not_as_requested(warning),
            "折り上がりが指定と違う警告は捨てる側: {warning}"
        );
    }

    // 折れた上での注意(通す)
    for warning in [
        // fold_through.rs: 折り筋を先に引いた紙では必ず出るが、実際の紙では折れる
        "折り線の一部に反対向きの折り線(山/谷)が既にあります(辺ID 12)。折り上がりは同じですが、そのままでは折り途中の形が正しく表示されません",
        // techniques.rs: 平らにならないことが「ある」という注意。実際に平らかは形で測る
        "この花弁折りでは、先端の紙が中心線の横まで広がっています。折り上がりが平らにならないことがあります(指定のまま続行します)",
        "この開いてつぶす折りでは、中央多角形が凹んでいます。折り上がりが平らにならないことがあります(指定のまま続行します)",
        "この開いてつぶす折りでは、層のつながりが輪になっていて開く側を決めきれません(指定のまま続行します)",
        "この花弁折りでは、中心線の上に開ける折り目が見つかりません。中心線がフラップの背に重なっているか確かめてください(指定のまま続行します)",
        "この花弁折りでは、斜めの折り目がフラップの外へ出る点を読めませんでした。ちょうつがいの位置を縁の長さから見積もっています(指定のまま続行します)",
        "この花弁折りでは、指定した層 3 が折り線の手前側に掛かっていないため動きません(指定のまま続行します)",
        // flat_motion.rs: 技法の途中では必ず出るので、技法が自分で選り分けている
        "折り目(辺ID 12)の両側の紙が離れているため、このままでは紙が裂けます(指定のまま続行します)",
    ] {
        assert!(
            !warning_means_the_fold_was_not_as_requested(warning),
            "折れた上での注意は通す側: {warning}"
        );
    }
}

/// 実際に出た警告でも同じ分け方になる(文言を組み立て直していないことの確認)。
///
/// 実在する層と、存在しない層を混ぜて指定すると、`fold_through` は実在する層だけで
/// 折り進め、「対象層 … は現在の面に存在しないため除外しました」を出す。
/// **頼んだ手と違う手になった**ので、捨てる側でなければならない。
#[test]
fn a_target_layer_that_does_not_exist_is_reported_as_not_as_requested() {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });

    // 1回折って層を2枚にする(1枚しかないと、全部の層が無効になり折り自体が断られる)
    let faces = extract_faces(&document.cp);
    let state = FlatState::initial(&document.cp, &faces);
    let first = fold_through(
        &mut document.cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.0, 0.5], [1.0, 0.5]],
            keep_side_point: [0.5, 0.25],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("半分に折れる");
    let mut step = first.step;
    step.id = 0;
    document.sequence.push(step);

    let faces = extract_faces(&document.cp);
    let state = first.state;
    let existing = *state.order.last().expect("最前面");
    let missing = faces.iter().map(|face| face.id).max().unwrap_or(0) + 99;

    let result = fold_through(
        &mut document.cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.25, 0.0], [0.25, 1.0]],
            keep_side_point: [0.1, 0.25],
            target_layers: Some(vec![existing, missing]),
            direction: FoldDirection::Up,
        },
    )
    .expect("実在する層があるので、止めずに続行する");

    let excluded: Vec<&String> = result
        .warnings
        .iter()
        .filter(|warning| warning_means_the_fold_was_not_as_requested(warning))
        .collect();
    assert_eq!(
        excluded.len(),
        1,
        "無い層を指定した警告がちょうど1件、捨てる側に分かれる(実際の警告 {:?})",
        result.warnings
    );
}
