//! 検査どうしで共有する、展開図の記録(`fixtures/cp-baseline-1-12.json`)の
//! 読み取りと突き合わせ。
//!
//! `leaf_site.rs`(作業9 合格条件5)と `fold_plan_trace.rs`(作業17 合格条件2)が
//! 同じ記録を使い、同じ「変わっていない」を主張する。比べ方が2か所で食い違わない
//! よう、突き合わせはここに1つだけ置く。

use ori3_model::CreasePattern;
use serde::{Deserialize, Serialize};

/// 記録との突き合わせで、**座標にだけ**許す差(紙の長辺を1.0とした系)。
///
/// **なぜ座標を完全一致で比べないか。** 頂点の座標は三角関数・平方根・除算を
/// 通した計算結果なので、いちばん下の桁は計算機ごとに変わり得る
/// (`CLAUDE.md` §10.7.7)。実際、この記録を**書き出した文字列として完全一致**で
/// 比べていたとき、手元では通るのにCI(GitHub Actions)の計算機で葉12本が落ちた。
/// 違っていたのは最下位1桁だけで、頂点と辺の個数、辺のつながり、山谷の種類、
/// 番号はすべて一致していた。それは「展開図が変わった」ではない。
///
/// **実測**(落ちたCIの出力と、手元での計測):
///
/// | 測ったもの | 値 |
/// |---|---|
/// | CIで実際に出たずれ(頂点9・10・27・28の5例) | 最大 **1.11e-16**(1〜2 ULP。16桁目) |
/// | 手元の計算機での、記録との差(のべ240頂点×2軸) | **0.0**(1ビットも違わない) |
/// | 記録240頂点で、別々の頂点どうしの最短距離 | **1.29e-3** |
/// | 展開図の組み立てが「同じ点」とみなす距離(`ori3_model::EPS`) | **1e-9** |
///
/// 許容差 **1e-12** の根拠は次の2つで、どちらも上の実測から出している。
/// 「落ちない値」から逆算したものではない。
///
/// - 実測のずれ 1.11e-16 に対して **約9,000倍**の余裕がある。計算機差では跨がない。
/// - 展開図の組み立てが同じ点とみなす 1e-9 より **1,000倍細かい**。頂点が別の場所へ
///   動いたと言える変化(1e-9以上)は必ず捕まる。実測の最短の頂点間隔 1.29e-3 に
///   対しては10億分の1以下である。
///
/// リポジトリの他の許容差(裂けの量 1e-6、往復の誤差 1e-12)とも同じ桁にそろえた。
///
/// わざと壊して落ちることを確かめた記録: 座標を1つだけ **+1e-11** 動かすと
/// 「頂点0のx座標が動いた: 差 1.000e-11 > 許容 1e-12」で落ちる。
/// CIと同じ大きさの **+1e-16** では落ちない。辺の山谷を1本変えると
/// 「辺の並び・つながり・山谷の種類が変わった」で、頂点を1つ減らすと
/// 「頂点の個数が変わった」で落ちる。
pub const CP_POS_TOL: f64 = 1e-12;

/// 展開図の記録1件。
///
/// 展開図は**書き出した文字そのもの**(`cp_json`)で持つ。数値の配列で持たない
/// のは、記録を作った当時の `serde_json` が17桁の小数を読み戻すときに最後の1桁を
/// 落としたため(当時の実測: `0.20710678118654754` を書いて読み戻すと
/// `0.20710678118654752`。ビット表現 `3fca827999fcef33` → `3fca827999fcef32`)。
///
/// 読み戻しの1桁ぶんのずれも、上の [`CP_POS_TOL`] の内側に入る
/// (手元の実測では、いまの `serde_json` は240頂点すべてを1ビットも変えずに
/// 読み戻している)。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CpBaseline {
    pub leaves: u32,
    pub cp_json: String,
}

/// 記録の保存先。検査は読むだけで、書き込むのは `#[ignore]` の再生成専用テスト
/// だけにする(CLAUDE.md §10.7.6)。
pub fn baseline_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("cp-baseline-1-12.json")
}

pub fn read_baseline() -> Vec<CpBaseline> {
    let path = baseline_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("記録を読めない({}): {e}", path.display()));
    serde_json::from_str(&text).expect("記録の形が壊れている")
}

/// 記録1件と、いま作った展開図を突き合わせる。
///
/// **頂点と辺の個数・番号・並び・つながり・山谷の種類は完全一致**を求める。
/// **座標だけ** [`CP_POS_TOL`] の許容差で比べる(理由と実測は同定数のコメント)。
///
/// 戻り値は `(見た頂点の数, 見た辺の数, 座標の差の最大)`。
pub fn assert_cp_matches_baseline(entry: &CpBaseline, now: &CreasePattern) -> (usize, usize, f64) {
    let n = entry.leaves;
    let want: CreasePattern = serde_json::from_str(&entry.cp_json)
        .unwrap_or_else(|e| panic!("葉{n}本の記録の展開図を読めない: {e}"));

    // --- ここから完全一致を求める(座標以外のすべて) ---
    assert_eq!(
        now.vertices.len(),
        want.vertices.len(),
        "葉{n}本で頂点の個数が変わった"
    );
    assert_eq!(
        now.edges.len(),
        want.edges.len(),
        "葉{n}本で辺の本数が変わった"
    );
    assert_eq!(
        now.next_vertex_id, want.next_vertex_id,
        "葉{n}本で次に配る頂点番号が変わった"
    );
    assert_eq!(
        now.next_edge_id, want.next_edge_id,
        "葉{n}本で次に配る辺番号が変わった"
    );
    // 辺は番号・両端の頂点・山谷だけでできており、計算した小数を1つも含まない。
    // 並びも含めてそのまま完全一致で比べる。
    assert_eq!(
        now.edges, want.edges,
        "葉{n}本で辺の並び・つながり・山谷の種類が変わった"
    );
    let now_ids: Vec<u32> = now.vertices.iter().map(|v| v.id).collect();
    let want_ids: Vec<u32> = want.vertices.iter().map(|v| v.id).collect();
    assert_eq!(now_ids, want_ids, "葉{n}本で頂点の番号・並びが変わった");

    // --- 座標だけ許容差つきで比べる ---
    let mut worst = 0.0f64;
    for (got, expected) in now.vertices.iter().zip(want.vertices.iter()) {
        for axis in 0..2 {
            let d = (got.pos[axis] - expected.pos[axis]).abs();
            worst = worst.max(d);
            assert!(
                d <= CP_POS_TOL,
                "葉{n}本: 頂点{}の{}座標が動いた: 記録 {} → いま {} (差 {d:.3e} > 許容 {CP_POS_TOL:.0e})",
                expected.id,
                if axis == 0 { "x" } else { "y" },
                expected.pos[axis],
                got.pos[axis],
            );
        }
    }
    (now.vertices.len(), now.edges.len(), worst)
}
