//! 完成形の先端位置(PRO-006 / PRO-007)を運ぶデータの形の検査。
//!
//! この作業で確かめるのは「位置を運ぶ形」だけで、位置を配置計算へ効かせるのは別作業。
//! したがってここでは「位置を足しても今までの提案結果が1文字も変わらない」ことを検査する。

use ori3_propose::skeleton::{Skeleton, SkeletonNode, TIP_POS_MAX, TIP_POS_MIN, TipPos2d};
use ori3_propose::{generate, pack};

/// 固定JSONの置き場所。`.gitignore` 対象の場所は読まない(CLAUDE.md §10.1)。
fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}が読めません: {e}", path.display()))
}

/// `tests/fixtures/position-skeleton.json` に書いてある値(葉ID昇順)。
/// TypeScript側 `apps/desktop/src/lib/skeleton.test.ts` の期待値と同じ数字を並べる。
const EXPECTED: [(u32, f64, f64); 6] = [
    (2, 0.123456789012345, 0.987654321098765),
    (3, -0.876543210987654, -0.234567890123456),
    (4, 0.5, -0.75),
    (5, -0.5, -0.75),
    (6, 1.0, -1.0),
    (7, -1.0, 1.0),
];

/// 頭1・尾1・足4 + 胴。`tests/fixtures/legacy-skeleton.json` と同じ形を組み立てる。
fn bird_base() -> Skeleton {
    Skeleton {
        nodes: vec![
            SkeletonNode::new(0, None, 0.0),
            SkeletonNode::new(1, Some(0), 0.4),
            SkeletonNode::new(2, Some(0), 1.0),
            SkeletonNode::new(3, Some(1), 1.0),
            SkeletonNode::new(4, Some(0), 0.7),
            SkeletonNode::new(5, Some(0), 0.7),
            SkeletonNode::new(6, Some(1), 0.7),
            SkeletonNode::new(7, Some(1), 0.7),
        ],
    }
}

/// 検査用の決まった並びの乱数(線形合同法)。実行するたびに同じ値が出る。
struct Lcg(u64);

impl Lcg {
    fn next_unit(&mut self) -> f64 {
        // Numerical Recipes の定数。0.0以上1.0未満を返す。
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }

    fn next_below(&mut self, n: usize) -> usize {
        (self.next_unit() * n as f64) as usize % n.max(1)
    }
}

/// 葉がちょうど `leaves` 本の骨格を作り、全ての葉へ位置を付ける。
fn random_skeleton(rng: &mut Lcg, leaves: usize) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    let mut next_id = 1u32;
    // まず1本目の先端を胴から出す。
    nodes.push(SkeletonNode::new(next_id, Some(0), 0.2 + rng.next_unit() * 2.8));
    next_id += 1;

    let leaf_ids = |nodes: &Vec<SkeletonNode>| -> Vec<u32> {
        nodes
            .iter()
            .filter(|n| n.parent.is_some() && !nodes.iter().any(|c| c.parent == Some(n.id)))
            .map(|n| n.id)
            .collect()
    };

    let mut guard = 0;
    while leaf_ids(&nodes).len() < leaves && guard < 1_000 {
        guard += 1;
        let current = leaf_ids(&nodes);
        // 先端へ足すと本数は変わらず深さだけ増える。先端でない節点へ足すと1本増える。
        let extend = rng.next_unit() < 0.3;
        let parent = if extend {
            current[rng.next_below(current.len())]
        } else {
            let inner: Vec<u32> = nodes
                .iter()
                .map(|n| n.id)
                .filter(|id| !current.contains(id))
                .collect();
            inner[rng.next_below(inner.len())]
        };
        nodes.push(SkeletonNode::new(
            next_id,
            Some(parent),
            0.2 + rng.next_unit() * 2.8,
        ));
        next_id += 1;
    }

    for n in nodes.iter_mut() {
        n.width_factor = 0.3 + rng.next_unit() * 1.7;
    }
    let current = leaf_ids(&nodes);
    for id in current {
        let x = TIP_POS_MIN + rng.next_unit() * (TIP_POS_MAX - TIP_POS_MIN);
        let y = TIP_POS_MIN + rng.next_unit() * (TIP_POS_MAX - TIP_POS_MIN);
        if let Some(n) = nodes.iter_mut().find(|n| n.id == id) {
            n.tip_pos_2d = Some(TipPos2d::new(x, y));
        }
    }
    Skeleton { nodes }
}

#[test]
fn fixture_json_is_read_with_the_expected_positions() {
    let s: Skeleton = serde_json::from_str(&fixture("position-skeleton.json")).unwrap();
    assert_eq!(s.validate(), Ok(()));

    // 位置を書いていない節点は「指定なし」のまま。
    assert_eq!(s.node(0).unwrap().tip_pos_2d, None, "根には位置を書いていない");
    assert_eq!(s.node(1).unwrap().tip_pos_2d, None, "胴には位置を書いていない");

    let got = s.leaf_tip_positions();
    assert_eq!(got.len(), EXPECTED.len(), "位置つきの先端は6本");
    for ((id, x, y), (got_id, got_pos)) in EXPECTED.iter().zip(got.iter()) {
        assert_eq!(*id, *got_id);
        assert_eq!(*x, got_pos.x, "先端{id}の横の値");
        assert_eq!(*y, got_pos.y, "先端{id}の縦の値");
    }
}

#[test]
fn json_roundtrip_keeps_positions_within_1e_12() {
    let s: Skeleton = serde_json::from_str(&fixture("position-skeleton.json")).unwrap();
    let back: Skeleton = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(s, back, "書いて読み直した骨格が元と一致する");

    let mut max_err: f64 = 0.0;
    for ((_, a), (_, b)) in s.leaf_tip_positions().iter().zip(back.leaf_tip_positions()) {
        max_err = max_err.max((a.x - b.x).abs()).max((a.y - b.y).abs());
    }
    // 実測: 0.0(2026-08-16、固定JSON6本12値)。合格条件は要件PRO-006の 1e-12。
    assert!(max_err <= 1e-12, "往復の絶対誤差 {max_err} が大きすぎる");
    assert_eq!(max_err, 0.0, "実測の往復誤差は0.0(桁落ちが起きていない)");
}

#[test]
fn one_to_twelve_leaves_stay_finite_over_1000_examples() {
    let mut rng = Lcg(2026_0816);
    let mut max_err: f64 = 0.0;
    let mut checked_positions = 0usize;
    let mut leaf_counts = [0usize; 13];

    for i in 0..1_000usize {
        let leaves = i % 12 + 1; // 1〜12本を均等に回す
        let s = random_skeleton(&mut rng, leaves);
        assert_eq!(s.leaves().len(), leaves, "{i}件目の先端の本数");
        leaf_counts[leaves] += 1;
        assert_eq!(s.validate(), Ok(()), "{i}件目の骨格");

        let positions = s.leaf_tip_positions();
        assert_eq!(positions.len(), leaves, "{i}件目は全先端に位置がある");
        let back: Skeleton = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        let back_positions = back.leaf_tip_positions();
        assert_eq!(positions.len(), back_positions.len());
        for ((id, a), (back_id, b)) in positions.iter().zip(back_positions.iter()) {
            assert_eq!(id, back_id);
            assert!(a.x.is_finite() && a.y.is_finite(), "{i}件目の先端{id}が有限");
            assert!(a.is_valid(), "{i}件目の先端{id}が範囲内");
            max_err = max_err.max((a.x - b.x).abs()).max((a.y - b.y).abs());
            checked_positions += 1;
        }
    }

    // 1〜12本を順に回すので、1,000件の先端の総数は 83*(1+..+12) + (1+2+3+4) = 6,484本。
    assert_eq!(checked_positions, 6_484, "1,000件で調べた先端の数");
    for (leaves, count) in leaf_counts.iter().enumerate().skip(1) {
        assert!(*count > 0, "先端{leaves}本の例が0件");
    }
    // 実測: 最大 1.1102230246251565e-16(2026-08-16、1,000件・先端6,484本・12,968値)。
    // 桁の多い値をJSONの文字から数値へ戻すときの最小単位1つ分で、合格条件 1e-12 の1万分の1以下。
    assert!(max_err <= 1e-12, "1,000件の往復の最大絶対誤差 {max_err}");
}

#[test]
fn legacy_json_without_tip_pos_loads_as_none() {
    let s: Skeleton = serde_json::from_str(&fixture("legacy-skeleton.json")).unwrap();
    assert_eq!(s, bird_base(), "位置欄の無い今までのJSONが今までどおり読める");
    assert_eq!(s.validate(), Ok(()));
    assert!(s.nodes.iter().all(|n| n.tip_pos_2d.is_none()));
    assert_eq!(s.leaf_tip_positions().len(), 0, "位置の指定は0件");
}

#[test]
fn skeleton_without_tip_pos_serializes_without_the_field() {
    let json = serde_json::to_string(&bird_base()).unwrap();
    assert!(
        !json.contains("tip_pos_2d"),
        "位置を指定しない骨格の書き出しに位置の欄が出ている: {json}"
    );
}

#[test]
fn tip_pos_does_not_change_the_existing_proposal() {
    let paper = (1.0, 1.0);
    let mut rng = Lcg(20_260_816);
    let mut compared = 0usize;

    for leaves in 1..=12usize {
        let plain = {
            let mut s = random_skeleton(&mut Lcg(1_000 + leaves as u64), leaves);
            for n in s.nodes.iter_mut() {
                n.tip_pos_2d = None;
            }
            s
        };
        let mut with_pos = plain.clone();
        for id in with_pos.leaves() {
            let x = TIP_POS_MIN + rng.next_unit() * (TIP_POS_MAX - TIP_POS_MIN);
            let y = TIP_POS_MIN + rng.next_unit() * (TIP_POS_MAX - TIP_POS_MIN);
            if let Some(n) = with_pos.nodes.iter_mut().find(|n| n.id == id) {
                n.tip_pos_2d = Some(TipPos2d::new(x, y));
            }
        }

        let a = pack(&plain, paper.0, paper.1, 7, 4);
        let b = pack(&with_pos, paper.0, paper.1, 7, 4);
        assert_eq!(a.len(), b.len(), "先端{leaves}本の候補数");
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
            "先端{leaves}本: 位置を足すと配置が変わってしまった"
        );
        compared += 1;
    }
    assert_eq!(compared, 12, "比べた骨格の数");

    // 展開図の生成まで含めて、位置の有無で結果が変わらないことを見る。
    let plain = bird_base();
    let mut with_pos = plain.clone();
    for (i, id) in with_pos.leaves().into_iter().enumerate() {
        if let Some(n) = with_pos.nodes.iter_mut().find(|n| n.id == id) {
            n.tip_pos_2d = Some(TipPos2d::new(0.1 * i as f64 - 0.3, 0.2 - 0.05 * i as f64));
        }
    }
    let packing = pack(&plain, paper.0, paper.1, 2026, 4);
    let from_plain = generate(&plain, &packing[0], paper.0, paper.1).unwrap();
    let from_pos = generate(&with_pos, &packing[0], paper.0, paper.1).unwrap();
    assert_eq!(
        serde_json::to_string(&from_plain).unwrap(),
        serde_json::to_string(&from_pos).unwrap(),
        "位置を足すと提案した展開図が変わってしまった"
    );
}

#[test]
fn broken_position_input_is_an_error_and_never_panics() {
    // (説明, JSON) の並び。すべて読み込みか検査のエラーになり、panicは0件。
    let broken_json = [
        ("横が無い", r#"{"y":0.5}"#),
        ("縦が無い", r#"{"x":0.5}"#),
        ("型違い(文字列)", r#"{"x":"0.5","y":0.5}"#),
        ("型違い(真偽値)", r#"{"x":true,"y":0.5}"#),
        ("奥行きを紛れ込ませた", r#"{"x":0.5,"y":0.5,"z":0.5}"#),
        ("並びで書いた", r#"[0.5,0.5]"#),
        ("奥行きまで並びで書いた", r#"[0.5,0.5,0.5]"#),
        ("空", r#"{}"#),
        ("横が重複", r#"{"x":0.1,"x":0.2,"y":0.5}"#),
    ];
    let mut rejected = 0usize;
    for (why, body) in broken_json {
        let json = format!(
            r#"{{"nodes":[{{"id":0,"parent":null,"length":0.0,"width_factor":1.0}},
                {{"id":1,"parent":0,"length":1.0,"width_factor":1.0,"tip_pos_2d":{body}}}]}}"#
        );
        let parsed: Result<Skeleton, _> = serde_json::from_str(&json);
        assert!(parsed.is_err(), "{why}: 読み込めてしまった");
        rejected += 1;
    }
    assert_eq!(rejected, 9, "読み込みで断った不正な入力の数");

    // 範囲外・数値にならない値は、読み込めても `validate` が日本語のエラーで断る。
    let out_of_range = [
        ("横が上限超え", 1.000_000_1, 0.0),
        ("横が下限未満", -1.000_000_1, 0.0),
        ("縦が上限超え", 0.0, 1.5),
        ("縦が下限未満", 0.0, -1.5),
        ("数値にならない値", f64::NAN, 0.0),
        ("無限大", 0.0, f64::INFINITY),
        ("負の無限大", f64::NEG_INFINITY, 0.0),
    ];
    let mut caught = 0usize;
    for (why, x, y) in out_of_range {
        let mut s = bird_base();
        s.nodes[2].tip_pos_2d = Some(TipPos2d::new(x, y));
        let err = s.validate().unwrap_err();
        assert!(err.contains("位置"), "{why}: {err}");
        caught += 1;
    }
    assert_eq!(caught, 7, "検査で断った範囲外の入力の数");

    // 欄が無い・nullは「指定なし」であってエラーではない(今までの入力が動く)。
    for body in [r#""tip_pos_2d":null,"#, ""] {
        let json = format!(
            r#"{{"nodes":[{{"id":0,"parent":null,"length":0.0,"width_factor":1.0}},
                {{"id":1,"parent":0,{body}"length":1.0,"width_factor":1.0}}]}}"#
        );
        let s: Skeleton = serde_json::from_str(&json).unwrap();
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(s.leaf_tip_positions().len(), 0);
    }

    // 境界そのものは受け付ける。
    let mut edge = bird_base();
    edge.nodes[2].tip_pos_2d = Some(TipPos2d::new(TIP_POS_MIN, TIP_POS_MAX));
    assert_eq!(edge.validate(), Ok(()));
}

#[test]
fn clamped_fits_into_the_range() {
    assert_eq!(TipPos2d::new(1.5, -2.0).clamped(), TipPos2d::new(1.0, -1.0));
    assert_eq!(TipPos2d::new(0.25, -0.5).clamped(), TipPos2d::new(0.25, -0.5));
    assert_eq!(
        TipPos2d::new(f64::INFINITY, f64::NEG_INFINITY).clamped(),
        TipPos2d::new(1.0, -1.0)
    );
    // 数値にならない値は原点の成分へ寄せる。
    assert_eq!(TipPos2d::new(f64::NAN, f64::NAN).clamped(), TipPos2d::new(0.0, 0.0));
    assert!(TipPos2d::new(f64::NAN, 12.0).clamped().is_valid());
}

#[test]
fn each_leaf_appears_at_most_once_and_stale_positions_are_ignored() {
    let mut s = bird_base();
    for (i, id) in s.leaves().into_iter().enumerate() {
        if let Some(n) = s.nodes.iter_mut().find(|n| n.id == id) {
            n.tip_pos_2d = Some(TipPos2d::new(0.1 * i as f64, -0.1 * i as f64));
        }
    }
    let ids: Vec<u32> = s.leaf_tip_positions().iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![2, 3, 4, 5, 6, 7], "葉ID昇順でちょうど1回ずつ");

    // 位置を付けた節点をまとめて作る近道も、同じ値になる。
    let made = SkeletonNode::new(9, Some(0), 1.0).with_tip_pos(TipPos2d::new(0.5, -0.5));
    assert_eq!(made.tip_pos_2d, Some(TipPos2d::new(0.5, -0.5)));
    assert_eq!(SkeletonNode::new(9, Some(0), 1.0).tip_pos_2d, None);

    // 先端2の先へ枝を足すと、2は先端でなくなる。古い位置は使わない。
    s.nodes.push(SkeletonNode::new(8, Some(2), 0.5));
    let after: Vec<u32> = s.leaf_tip_positions().iter().map(|(id, _)| *id).collect();
    assert_eq!(after, vec![3, 4, 5, 6, 7], "先端でなくなった節点の位置は出ない");
    assert_eq!(s.validate(), Ok(()), "古い位置が残っていても骨格は正しい");
}
