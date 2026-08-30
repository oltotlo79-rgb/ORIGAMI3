//! 検査が固定した手順を、入力CPの一般制約を使うstrict経路で進める共通入口。
//!
//! 固定IDが無効になったときだけ同じ状態の全[`FoldLine`](ori3_propose::FoldLine)を
//! 再測定する。通常時の有効件数を期待値や境目にはせず、失敗の原因を診断するための
//! 実測値としてだけpanicへ載せる。

use ori3_model::Document;
use ori3_propose::enumerate::{FoldSession, PoseScan, VerifiedMove};

/// `ids`を順にstrict検証してから適用した状態を返す。
///
/// [`FoldSession::verify_move`]は入力CPのM/Vを一般制約の根拠にする提案・手順検証の
/// 経路である。固定IDが無効なら、同じAPIでその時点の全FoldLineを測り直し、固定入力の
/// 破損と、単に21姿勢で折れないことを区別できる診断を出す。
pub(crate) fn folded_along(doc: &Document, ids: &[usize]) -> FoldSession {
    let mut session = FoldSession::new(doc).expect("折り始められない");
    for &id in ids {
        let mv = verify_fixed_move(&session, id);
        session
            .apply(&mv)
            .unwrap_or_else(|error| panic!("固定した手{id}を適用できない: {error}"));
    }
    session
}

fn verify_fixed_move(session: &FoldSession, id: usize) -> VerifiedMove {
    if let Some(mv) = session.verify_move(id, PoseScan::DEFAULT) {
        return mv;
    }

    // 追加の全数走査は失敗時だけ行う。正常時の実行時間を増やさず、有効件数を
    // 合否の境目へ固定しないためである。
    let total = session.fold_lines().len();
    let valid = session
        .fold_lines()
        .iter()
        .filter(|line| session.verify_move(line.id, PoseScan::DEFAULT).is_some())
        .count();
    panic!("固定している手が無効になった。固定ID {id}。strict 有効手の実測は {valid}/{total}");
}
