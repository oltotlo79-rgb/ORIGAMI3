//! 完成形の先端位置(PRO-006 / PRO-007)の検査。
//!
//! 作業5で「位置を運ぶ形」を作り、作業10で**その位置を紙の上の配置へ反映**した。
//! したがってここで主張するのは次の2つで、両方が同時に成り立たなければならない。
//!
//! - **位置を指定しなければ、今までと完全に一致する。**
//! - **位置を指定すれば、展開図が変わり、指定した並びへ近づく。**

use ori3_propose::finish::{FinishTarget, FinishedForm};
use ori3_propose::skeleton::{Skeleton, SkeletonNode, TIP_POS_MAX, TIP_POS_MIN, TipPos2d};
use ori3_propose::{Packing, body_on_paper, generate, pack, position_gap, tip_targets};

mod support;

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
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
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
    nodes.push(SkeletonNode::new(
        next_id,
        Some(0),
        0.2 + rng.next_unit() * 2.8,
    ));
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
    assert_eq!(
        s.node(0).unwrap().tip_pos_2d,
        None,
        "根には位置を書いていない"
    );
    assert_eq!(
        s.node(1).unwrap().tip_pos_2d,
        None,
        "胴には位置を書いていない"
    );

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
            assert!(
                a.x.is_finite() && a.y.is_finite(),
                "{i}件目の先端{id}が有限"
            );
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
    assert_eq!(
        s,
        bird_base(),
        "位置欄の無い今までのJSONが今までどおり読める"
    );
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

/// 記録(`fixtures/cp-baseline-1-12.json`)と同じ、根に葉を`n`本ぶら下げた星形。
/// `leaf_site.rs` の `star` と同じ骨格・同じ条件(紙1.0×1.0、seed 2026、やり直し8回)。
fn star(n: u32) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for i in 1..=n {
        nodes.push(SkeletonNode::new(i, Some(0), 1.0));
    }
    Skeleton { nodes }
}

/// 検査に使う、はっきり離れた位置の指定。
///
/// 先端を胴のまわりへ等しい角度で並べ、3本ごとに胴からの遠さを変える。
/// 実際の指定(頭は上、足は下)に近い形でありながら、2本が同じ場所へ来ない。
/// 同じ場所へ来る指定は `impossible_positions_still_return_a_proposal` で別に見る。
fn spread_positions(n: usize) -> Vec<TipPos2d> {
    (0..n)
        .map(|i| {
            let angle = std::f64::consts::TAU * (i as f64 + 0.25) / n as f64;
            let far = 0.55 + 0.45 * ((i % 3) as f64 / 2.0);
            TipPos2d::new(far * angle.cos(), far * angle.sin())
        })
        .collect()
}

/// 星形の全ての先端へ [`spread_positions`] の位置を付けた骨格。
fn star_with_positions(n: u32) -> Skeleton {
    let mut s = star(n);
    let want = spread_positions(n as usize);
    for (i, id) in s.leaves().into_iter().enumerate() {
        if let Some(node) = s.nodes.iter_mut().find(|node| node.id == id) {
            node.tip_pos_2d = Some(want[i]);
        }
    }
    s
}

/// 紙の上の配置から、完成形での先端の並びを測る。
///
/// 一軸基本形では紙が骨格の木の上へ畳み込まれ、円中心はその先端の節点へ移る。
/// だから**紙の上に並んだ円中心の並び方が、完成形を正面から見た先端の並び方**に
/// なる(`ori3_propose::TipTargets` のdocコメント)。ここではそれをそのまま測る。
/// 大きさのそろえ方は `FinishedForm::with_tip_points` に任せる。
///
/// `target` は「利用者が何を指定したか」なので、指定なしの配置を測るときも
/// **指定ありの目標**を渡す。そうしないと同じ物差しで比べられない。
fn measure_layout(
    skeleton: &Skeleton,
    packing: &Packing,
    target: &FinishTarget,
    body: [f64; 2],
    paper: (f64, f64),
) -> FinishedForm {
    let result = generate(skeleton, packing, paper.0, paper.1)
        .unwrap_or_else(|e| panic!("展開図を作れない: {e}"));
    FinishedForm::from_proposal(skeleton, packing, &result).with_tip_points(
        target,
        body,
        &packing.centers,
    )
}

/// 円中心の重心。位置の指定が無いときに胴の場所を見積もる唯一の手がかり。
fn centroid(packing: &Packing) -> [f64; 2] {
    let n = packing.centers.len() as f64;
    let mut sum = [0.0f64; 2];
    for &(_, c) in &packing.centers {
        sum[0] += c[0];
        sum[1] += c[1];
    }
    [sum[0] / n, sum[1] / n]
}

/// 作業10の中心となる検査。**2つのことを同時に主張する。**
///
/// 1. **位置を指定しなければ、今までと完全に一致する**(合格条件3)。
///    作業9で残した記録 `fixtures/cp-baseline-1-12.json` と、葉1〜12本の
///    12通りで突き合わせる。頂点と辺の個数・番号・並び・つながり・山谷は
///    完全一致、座標だけ `support::CP_POS_TOL` の許容差で比べる。
/// 2. **位置を指定すれば、展開図が変わる**(合格条件1)。同じ12通りで、
///    指定を付けた骨格の展開図が記録と違うことを確かめる。
///
/// 作業5の時点でこの検査は「位置の有無で結果が完全一致」を主張していた。
/// 位置が配置へ効くようになった以上、その主張は**逆に間違い**になるので、
/// 消さずに「指定なし＝不変」と「指定あり＝変わる」の2つへ書き直した。
#[test]
fn tip_pos_changes_the_proposal_only_when_it_is_given() {
    const PAPER: (f64, f64) = (1.0, 1.0);
    let baseline = support::read_baseline();
    assert_eq!(baseline.len(), 12, "記録が12件ない");

    let mut unchanged = 0usize;
    let mut changed = 0usize;
    for entry in &baseline {
        let n = entry.leaves;

        // 1. 位置を指定しない → 記録と一致する。
        let plain = star(n);
        let free = pack(&plain, PAPER.0, PAPER.1, 2026, 8);
        assert!(!free.is_empty(), "葉{n}本: 指定なしの配置に失敗した");
        let from_plain = generate(&plain, &free[0], PAPER.0, PAPER.1).unwrap();
        support::assert_cp_matches_baseline(entry, &from_plain.cp);
        assert!(
            from_plain
                .warnings
                .iter()
                .all(|w| !w.contains("出したい場所")),
            "葉{n}本: 位置を指定していないのに位置の知らせが出た: {:?}",
            from_plain.warnings
        );
        unchanged += 1;

        // 2. 位置を指定する → 展開図が変わる。
        let posed = star_with_positions(n);
        let guided = pack(&posed, PAPER.0, PAPER.1, 2026, 8);
        assert!(!guided.is_empty(), "葉{n}本: 指定ありの配置に失敗した");
        let from_posed = generate(&posed, &guided[0], PAPER.0, PAPER.1).unwrap();

        // 円の中心がどれだけ動いたかを、先端IDで突き合わせて数で見る。
        let moved = free[0]
            .centers
            .iter()
            .map(|&(leaf_id, a)| {
                let b = guided[0]
                    .centers
                    .iter()
                    .find(|(id, _)| *id == leaf_id)
                    .map(|&(_, c)| c)
                    .unwrap_or_else(|| panic!("葉{n}本: 先端{leaf_id}が指定ありの配置に無い"));
                (a[0] - b[0]).hypot(a[1] - b[1])
            })
            .fold(0.0f64, f64::max);
        assert!(
            moved > 1e-3,
            "葉{n}本: 位置を指定しても円の中心がほとんど動かない(最大 {moved:e})"
        );
        let same_cp = serde_json::to_string(&from_plain.cp).unwrap()
            == serde_json::to_string(&from_posed.cp).unwrap();
        assert!(!same_cp, "葉{n}本: 位置を指定しても展開図が変わらなかった");
        changed += 1;
    }
    assert_eq!(unchanged, 12, "指定なしで記録と一致した件数");
    assert_eq!(changed, 12, "指定ありで展開図が変わった件数");
}

/// 合格条件2: 指定した位置に近づく。葉1〜12本の12通りで、指定なしの配置より
/// 指定ありの配置のほうが [`position_gap`] が小さい。
///
/// 胴の場所は、それぞれで**得られる限りの手がかり**から決める。指定があるときは
/// 指定の枠の原点([`tip_targets`] の `body`)、無いときは円中心の重心。
/// どちらの見積もりでも結果が変わらないことを確かめるため、
/// **重心にそろえた測り方でも**同じ向きの差が出ることを併せて見る。
///
/// 実測値はテストの出力(`-- --nocapture`)に12通りぶん並べる。
#[test]
fn the_given_positions_bring_the_finished_form_closer() {
    const PAPER: (f64, f64) = (1.0, 1.0);
    // 指定ありの隔たりの上限。指定した先端をすべて目標点へ固定できる骨格では、
    // 測り直した位置は指定そのものに戻るので、残るのは小数の丸めだけになる。
    // 実測は下の出力のとおり(2026-08-17、葉1〜12本の12通りで最大
    // **4.531712712300976e-17**、葉4本のとき)。上限はその約22,000倍にあたる
    // 1e-12 で、記録の突き合わせ(`support::CP_POS_TOL`)と同じ桁にそろえてある。
    // 実測をそのまま境目にはしていない(`CLAUDE.md` §10.7.9)。
    const GUIDED_LIMIT: f64 = 1e-12;

    let mut better = 0usize;
    let mut better_by_centroid = 0usize;
    let mut worst_guided = 0.0f64;
    for n in 1..=12u32 {
        let plain = star(n);
        let posed = star_with_positions(n);
        // 物差しは利用者の指定そのもの。どちらの配置も同じ目標で測る。
        let target = FinishTarget::from_skeleton_on_paper(&posed);

        let free = pack(&plain, PAPER.0, PAPER.1, 2026, 8);
        let guided = pack(&posed, PAPER.0, PAPER.1, 2026, 8);
        assert!(
            !free.is_empty() && !guided.is_empty(),
            "葉{n}本の配置に失敗"
        );

        let free_body = body_on_paper(&plain, &free[0], PAPER.0, PAPER.1);
        let guided_body = body_on_paper(&posed, &guided[0], PAPER.0, PAPER.1);
        let free_gap = position_gap(
            &target,
            &measure_layout(&plain, &free[0], &target, free_body, PAPER),
        );
        let guided_gap = position_gap(
            &target,
            &measure_layout(&posed, &guided[0], &target, guided_body, PAPER),
        );

        // 胴の場所をどちらも重心にそろえた、同じ条件どうしの比べ方。
        let free_c = position_gap(
            &target,
            &measure_layout(&plain, &free[0], &target, centroid(&free[0]), PAPER),
        );
        let guided_c = position_gap(
            &target,
            &measure_layout(&posed, &guided[0], &target, centroid(&guided[0]), PAPER),
        );

        println!(
            "葉{n:2}本: 指定なし {free_gap:.6} → 指定あり {guided_gap:.3e} \
             (重心でそろえた場合 {free_c:.6} → {guided_c:.6})"
        );
        assert!(
            guided_gap < free_gap,
            "葉{n}本: 位置を指定したほうが遠い(指定なし {free_gap}, 指定あり {guided_gap})"
        );
        assert!(
            guided_gap <= GUIDED_LIMIT,
            "葉{n}本: 指定した位置に戻っていない({guided_gap:e} > {GUIDED_LIMIT:e})"
        );
        if n == 1 {
            // 先端が1本しかないと、円中心の重心はその先端そのものになる。
            // 胴からの離れ方が0になるので、どこへ置いても同じ値にしかならない。
            // 重心でそろえた測り方が使えるのは先端が2本以上のときだけである。
            assert_eq!(
                free_c, guided_c,
                "葉1本では重心が先端と重なるので、同じ値になるはず"
            );
        } else {
            assert!(
                guided_c < free_c,
                "葉{n}本: 重心でそろえても近づいていない(指定なし {free_c}, 指定あり {guided_c})"
            );
            better_by_centroid += 1;
        }
        worst_guided = worst_guided.max(guided_gap);
        better += 1;
    }
    println!("指定ありの隔たりの最大 = {worst_guided:e}(上限 {GUIDED_LIMIT:e})");
    assert_eq!(better, 12, "近づいた件数");
    assert_eq!(
        better_by_centroid, 11,
        "重心でそろえても近づいた件数(先端2〜12本)"
    );
}

/// 位置を指定した先端が、指定どおりの並びで紙の上に置かれていること。
///
/// 紙の上の目標点([`tip_targets`])と、実際に置かれた円の中心が同じ点になる。
/// 目標点そのものは「胴 + 共通の倍率 × 指定」なので、これが一致すれば
/// **胴から見た向きと、遠さの割合の両方**が指定どおりになっている。
#[test]
fn positioned_tips_sit_on_the_place_the_given_layout_asks_for() {
    const PAPER: (f64, f64) = (1.0, 1.0);
    // 円中心の一致に許す差。展開図が「同じ点」とみなす距離(1e-7)より4桁細かい。
    // 実測は下の出力のとおり(2026-08-17、のべ78本で最大 0.0)。
    const LIMIT: f64 = 1e-11;
    let mut checked = 0usize;
    let mut worst = 0.0f64;
    for n in 1..=12u32 {
        let posed = star_with_positions(n);
        let guide = tip_targets(&posed, PAPER.0, PAPER.1).expect("目標点が作れない");
        assert!(!guide.conflicting, "葉{n}本: 指定が重なってしまった");
        assert_eq!(
            guide.notices.len(),
            0,
            "葉{n}本: 無理のない指定で知らせが出た"
        );
        assert_eq!(guide.points.len(), n as usize, "葉{n}本の目標点の数");

        let packed = pack(&posed, PAPER.0, PAPER.1, 2026, 8);
        assert!(!packed.is_empty(), "葉{n}本の配置に失敗");
        for &(leaf_id, want) in &guide.points {
            let got = packed[0]
                .centers
                .iter()
                .find(|(id, _)| *id == leaf_id)
                .map(|&(_, c)| c)
                .unwrap_or_else(|| panic!("葉{n}本: 先端{leaf_id}の中心が無い"));
            let gap = (got[0] - want[0]).hypot(got[1] - want[1]);
            assert!(
                gap <= LIMIT,
                "葉{n}本: 先端{leaf_id}が目標点から{gap:e}離れている"
            );
            worst = worst.max(gap);
            checked += 1;
        }
        // 胴も紙の中に無ければならない。
        assert!(
            (0.0..=PAPER.0).contains(&guide.body[0]) && (0.0..=PAPER.1).contains(&guide.body[1]),
            "葉{n}本: 胴の場所{:?}が紙の外",
            guide.body
        );
    }
    println!("目標点との差: のべ{checked}本、最大 {worst:e}(上限 {LIMIT:e})");
    assert_eq!(checked, 78, "1+2+...+12=78本を見ていない");
}

/// 位置を指定していない先端は、今までどおり自動で置かれること。
///
/// 一部だけ指定した骨格で、指定した先端は目標点へ、指定していない先端は
/// そこから離れた場所へ置かれる。指定していない先端が目標点を持たないことも見る。
#[test]
fn tips_without_a_given_position_are_still_placed_automatically() {
    const PAPER: (f64, f64) = (1.0, 1.0);
    let mut cases = 0usize;
    for n in 2..=12u32 {
        let all = star_with_positions(n);
        let want = spread_positions(n as usize);
        // 先端IDが奇数のものだけ位置を指定する。
        let mut some = star(n);
        let mut given: Vec<u32> = Vec::new();
        for (i, id) in some.leaves().into_iter().enumerate() {
            if id % 2 == 1 {
                given.push(id);
                if let Some(node) = some.nodes.iter_mut().find(|node| node.id == id) {
                    node.tip_pos_2d = Some(want[i]);
                }
            }
        }
        assert!(!given.is_empty(), "葉{n}本: 指定した先端が0本");

        let guide = tip_targets(&some, PAPER.0, PAPER.1).expect("目標点が作れない");
        assert_eq!(guide.points.len(), given.len(), "葉{n}本の目標点の数");
        let packed = pack(&some, PAPER.0, PAPER.1, 2026, 8);
        assert!(!packed.is_empty(), "葉{n}本の配置に失敗");
        assert_eq!(
            packed[0].centers.len(),
            n as usize,
            "葉{n}本: 置かれた先端の数が足りない"
        );
        for &(leaf_id, target) in &guide.points {
            let got = packed[0]
                .centers
                .iter()
                .find(|(id, _)| *id == leaf_id)
                .map(|&(_, c)| c)
                .expect("中心が無い");
            let gap = (got[0] - target[0]).hypot(got[1] - target[1]);
            assert!(
                gap <= 1e-11,
                "葉{n}本: 指定した先端{leaf_id}がずれた({gap:e})"
            );
        }
        // 指定していない先端は紙の中のどこかに置かれ、指定した先端と重ならない。
        for &(leaf_id, c) in &packed[0].centers {
            if given.contains(&leaf_id) {
                continue;
            }
            assert!(
                (0.0..=PAPER.0).contains(&c[0]) && (0.0..=PAPER.1).contains(&c[1]),
                "葉{n}本: 指定していない先端{leaf_id}が紙の外{c:?}"
            );
        }
        // 全部指定した場合とは違う配置になる(指定していない分は自動で決まる)。
        let all_packed = pack(&all, PAPER.0, PAPER.1, 2026, 8);
        assert_ne!(
            packed[0].centers, all_packed[0].centers,
            "葉{n}本: 一部だけ指定した配置が、全部指定した配置と同じになった"
        );
        cases += 1;
    }
    assert_eq!(cases, 11, "先端2〜12本の11通りを見ていない");
}

/// 合格条件4: 紙に収まらない指定でも止まらない。**2通り**を確かめる。
///
/// 1. 先端を全部同じ場所へ指定する
/// 2. 先端を枠の外(紙の外)へ指定する
///
/// どちらも結果が返り、日本語の知らせが出る(`CLAUDE.md` §8「止めずに警告する」)。
#[test]
fn impossible_positions_still_return_a_proposal_with_a_notice() {
    const PAPER: (f64, f64) = (1.0, 1.0);
    let mut cases = 0usize;

    // 1. 全部同じ場所。
    let mut same_place = star(6);
    for id in same_place.leaves() {
        if let Some(node) = same_place.nodes.iter_mut().find(|node| node.id == id) {
            node.tip_pos_2d = Some(TipPos2d::new(0.5, 0.5));
        }
    }
    let guide = tip_targets(&same_place, PAPER.0, PAPER.1).expect("目標点が作れない");
    assert!(guide.conflicting, "同じ場所の指定を見つけられていない");
    let packed = pack(&same_place, PAPER.0, PAPER.1, 2026, 8);
    assert!(!packed.is_empty(), "同じ場所の指定で配置が返らなかった");
    assert_eq!(packed[0].centers.len(), 6, "6本すべてが置かれていない");
    let made = generate(&same_place, &packed[0], PAPER.0, PAPER.1).expect("展開図が返らなかった");
    assert!(
        made.warnings.iter().any(|w| w.contains("同じ場所")),
        "同じ場所の指定で知らせが出ていない: {:?}",
        made.warnings
    );
    // 重なったままにせず、離して置いている。
    let mut closest = f64::INFINITY;
    for a in 0..packed[0].centers.len() {
        for b in (a + 1)..packed[0].centers.len() {
            let (p, q) = (packed[0].centers[a].1, packed[0].centers[b].1);
            closest = closest.min((p[0] - q[0]).hypot(p[1] - q[1]));
        }
    }
    println!(
        "全部同じ場所: いちばん近い2本の距離 = {closest:.6}、縮尺 = {}",
        packed[0].scale
    );
    assert!(
        closest > 1e-3,
        "同じ場所の指定で先端が重なったまま({closest:e})"
    );
    cases += 1;

    // 2. 枠の外(紙の外)。位置ごとにばらばらの外側を指す。
    let outside = [
        TipPos2d::new(3.0, 0.0),
        TipPos2d::new(0.0, -4.5),
        TipPos2d::new(-2.5, 2.5),
        TipPos2d::new(1.5, -1.5),
    ];
    let mut far = star(4);
    for (i, id) in far.leaves().into_iter().enumerate() {
        if let Some(node) = far.nodes.iter_mut().find(|node| node.id == id) {
            node.tip_pos_2d = Some(outside[i]);
        }
    }
    // データを読み書きする側(保存・画面からの受け取り)は今までどおりエラーにする。
    assert!(far.validate().is_err(), "枠の外の指定が検査を素通りした");
    assert_eq!(far.validate_structure(), Ok(()), "骨格の形そのものは正しい");

    let guide = tip_targets(&far, PAPER.0, PAPER.1).expect("目標点が作れない");
    assert_eq!(guide.notices.len(), 4, "枠の外4本ぶんの知らせが出ていない");
    let packed = pack(&far, PAPER.0, PAPER.1, 2026, 8);
    assert!(!packed.is_empty(), "枠の外の指定で配置が返らなかった");
    assert_eq!(packed[0].centers.len(), 4, "4本すべてが置かれていない");
    for &(id, c) in &packed[0].centers {
        assert!(
            (0.0..=PAPER.0).contains(&c[0]) && (0.0..=PAPER.1).contains(&c[1]),
            "先端{id}が紙の外{c:?}へ置かれた"
        );
    }
    let made = generate(&far, &packed[0], PAPER.0, PAPER.1).expect("展開図が返らなかった");
    assert!(
        made.warnings
            .iter()
            .any(|w| w.contains("いちばん近いところへ寄せました")),
        "枠の外の指定で知らせが出ていない: {:?}",
        made.warnings
    );
    // 寄せた先は枠の角。いちばん近い置き方になっている。
    let corner = star_with_clamped(&outside);
    let want = tip_targets(&corner, PAPER.0, PAPER.1).expect("目標点が作れない");
    assert_eq!(
        guide.points.len(),
        want.points.len(),
        "寄せた指定と、はじめから縁にある指定で本数が違う"
    );
    for (a, b) in guide.points.iter().zip(want.points.iter()) {
        assert_eq!(a.0, b.0, "先端の並びが違う");
        let gap = (a.1[0] - b.1[0]).hypot(a.1[1] - b.1[1]);
        assert!(gap <= 1e-12, "先端{}の寄せ先が違う({gap:e})", a.0);
    }
    cases += 1;

    assert_eq!(cases, 2, "2通りとも確かめていない");
}

/// 上の検査で使う、位置を枠の中へ寄せた同じ骨格。
fn star_with_clamped(positions: &[TipPos2d]) -> Skeleton {
    let mut s = star(positions.len() as u32);
    for (i, id) in s.leaves().into_iter().enumerate() {
        if let Some(node) = s.nodes.iter_mut().find(|node| node.id == id) {
            node.tip_pos_2d = Some(positions[i].clamped());
        }
    }
    s
}

/// 合格条件5: 同じ入力を10回計算して同じ結果。
#[test]
fn the_same_given_positions_give_the_same_proposal_ten_times() {
    const PAPER: (f64, f64) = (1.0, 1.0);
    for n in [1u32, 5, 12] {
        let posed = star_with_positions(n);
        let first = {
            let p = pack(&posed, PAPER.0, PAPER.1, 2026, 8);
            let r = generate(&posed, &p[0], PAPER.0, PAPER.1).unwrap();
            (
                serde_json::to_string(&p).unwrap(),
                serde_json::to_string(&r.cp).unwrap(),
            )
        };
        let mut same = 0usize;
        for round in 1..=10 {
            let p = pack(&posed, PAPER.0, PAPER.1, 2026, 8);
            let r = generate(&posed, &p[0], PAPER.0, PAPER.1).unwrap();
            assert_eq!(
                serde_json::to_string(&p).unwrap(),
                first.0,
                "葉{n}本の{round}回目で配置が変わった"
            );
            assert_eq!(
                serde_json::to_string(&r.cp).unwrap(),
                first.1,
                "葉{n}本の{round}回目で展開図が変わった"
            );
            same += 1;
        }
        assert_eq!(same, 10, "葉{n}本で10回そろっていない");
    }
}

/// 位置を1つも指定していない骨格では、位置の仕組みが1つも動かないこと。
///
/// 木の形も長さも太さもばらばらな骨格1,000通りで、目標点が作られないこと、
/// 胴の場所が円中心の重心になること、配置がすべて紙の中に収まることを見る。
/// 展開図が今までと1文字も変わらないことは
/// [`tip_pos_changes_the_proposal_only_when_it_is_given`] が記録と突き合わせる。
#[test]
fn nothing_happens_when_no_position_is_given() {
    const PAPER: (f64, f64) = (1.0, 1.0);
    let mut checked = 0usize;
    for leaves in 1..=12usize {
        let mut plain = random_skeleton(&mut Lcg(1_000 + leaves as u64), leaves);
        for n in plain.nodes.iter_mut() {
            n.tip_pos_2d = None;
        }
        assert!(
            tip_targets(&plain, PAPER.0, PAPER.1).is_none(),
            "先端{leaves}本: 指定が無いのに目標点ができた"
        );
        let packed = pack(&plain, PAPER.0, PAPER.1, 7, 4);
        assert!(!packed.is_empty(), "先端{leaves}本の配置に失敗");
        // 胴の場所は、紙の上の手がかりだけから見積もった円中心の重心になる。
        assert_eq!(
            body_on_paper(&plain, &packed[0], PAPER.0, PAPER.1),
            centroid(&packed[0]),
            "先端{leaves}本の胴の場所"
        );
        for &(id, c) in &packed[0].centers {
            assert!(
                c[0].is_finite() && c[1].is_finite(),
                "先端{leaves}本: 先端{id}の中心が数でない"
            );
            assert!(
                (0.0..=PAPER.0).contains(&c[0]) && (0.0..=PAPER.1).contains(&c[1]),
                "先端{leaves}本: 先端{id}が紙の外{c:?}"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 12, "先端1〜12本の12通りを見ていない");
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
    assert_eq!(
        TipPos2d::new(0.25, -0.5).clamped(),
        TipPos2d::new(0.25, -0.5)
    );
    assert_eq!(
        TipPos2d::new(f64::INFINITY, f64::NEG_INFINITY).clamped(),
        TipPos2d::new(1.0, -1.0)
    );
    // 数値にならない値は原点の成分へ寄せる。
    assert_eq!(
        TipPos2d::new(f64::NAN, f64::NAN).clamped(),
        TipPos2d::new(0.0, 0.0)
    );
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
    assert_eq!(
        after,
        vec![3, 4, 5, 6, 7],
        "先端でなくなった節点の位置は出ない"
    );
    assert_eq!(s.validate(), Ok(()), "古い位置が残っていても骨格は正しい");
}
