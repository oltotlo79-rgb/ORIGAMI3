//! 「利用者が固定した折り角度は、ほかの折り目を動かしても変わらない」ことの検査。
//!
//! 画面側の固定機能は、固定した折り目を追従計算の**動かさない側**(`drivers`)へ
//! 入れることで実現している。この検査は、その土台となる性質だけを見る。
//!
//! # 検査の書き方について
//!
//! - 角度と固定の有無は**すべて引数で明示して与える**。計算結果から取り出した値を
//!   期待値にしない(計算機が変わると最下位の桁が変わり得るため)。
//! - 小数は許容差つきで比べる。許容差は下の `PIN_TOLERANCE_DEG` の根拠を参照。
//! - 収束したかどうかのような「計算の内部事情」に期待値を結び付けない。
//!   成り立たない指定の検査でも、**複数の知らせのどれかが立つこと**だけを見る。

use std::collections::HashMap;

use glam::DVec3;
use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeId, EdgeKind, Vertex};
use ori3_rigid::{max_seam_gap, self_intersection_pairs, solve_near};

/// 固定した折り目が動いてよい量(度)。
///
/// 画面側の受け入れ条件が 1e-9 度なので、その値をそのまま使う。
/// 実測では、動かさない側へ入れた折り目の誤差は
/// **2.842e-14 度**(17本を固定した実機の姿勢。
/// `crates/ori3-soft/tests/live2_pose_diagnosis.rs`)で、
/// 1e-9 はその **約35,000倍の余裕**がある。実測値そのものを境目にはしない。
const PIN_TOLERANCE_DEG: f64 = 1e-9;

/// 成り立たない指定を「紙が裂けた」と判断する開き。
///
/// `crates/ori3-rigid/src/motion.rs` の `SEAM_TEAR_TOLERANCE` と同じ 1e-6。
const SEAM_TOLERANCE: f64 = 1e-6;

/// 報告のために「動いた」と数える最小のずれ(度)。画面が知らせる下限と同じ。
const PIN_RELEASE_REPORT_EPS_DEG: f64 = 0.1;

/// この検査の形で、紙がつながっているとみなす縫い目の開き。
///
/// この検査は「固定していない折り目は0度のままでいてほしい」という希望を
/// わざと強く与えるので、希望と紙のつながりの釣り合いの分だけ開きが残る。
/// 実測(この検査の5通り。2026-08-17 開発機 Windows 11・debugビルド)は
/// 3.022e-14 / 9.761e-7 / 1.243e-6 / 8.823e-14 / 2.055e-9 で、最悪は 1.243e-6 だった。
/// **実測値をそのまま境目にしない**ため、その約80倍の 1e-4 を上限にする。紙の長辺を1.0としているので、
/// 1e-4 は紙の1万分の1で、画面では見えない大きさである。
/// 成り立たない指定を見分ける検査には、上の 1e-6 のほうを使う。
const CONNECTED_SEAM_LIMIT: f64 = 1e-4;

fn v(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn e(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

fn d(hinge: u32, deg: f64) -> Driver {
    Driver {
        hinge,
        target_angle_deg: deg,
    }
}

/// 中心から折り線が伸びる展開図を作る。
///
/// 紙の輪郭は、中心から見て `azimuths_deg` の向きにある点を結んだ多角形で、
/// その頂点それぞれへ中心から折り線を引く。内部頂点1個・次数nなので、
/// 動かせる自由度は n-3 になる(n=6なら3、n=8なら5)。
/// 1本を固定して1本を動かしても自由度が残るので、
/// 「固定したまま別の折り目を動かす」ことができる。
///
/// 折り線の辺IDは n..2n(輪郭の辺IDが 0..n)。
/// 山谷は交互に付けるが、角度は呼び出し側が明示的に与えるので折り方は決め打ちしない。
fn star_cp_at(azimuths_deg: &[f64], radii: &[f64]) -> CreasePattern {
    let spokes = azimuths_deg.len();
    assert!(spokes >= 4);
    let mut vertices = Vec::new();
    let mut edges = Vec::new();
    for (i, azimuth) in azimuths_deg.iter().enumerate() {
        let angle = azimuth.to_radians();
        let radius = radii[i % radii.len()];
        vertices.push(v(
            i as u32,
            0.5 + 0.5 * radius * angle.cos(),
            0.5 + 0.5 * radius * angle.sin(),
        ));
    }
    let center = spokes as u32;
    vertices.push(v(center, 0.5, 0.5));
    for i in 0..spokes {
        edges.push(e(
            i as u32,
            i as u32,
            ((i + 1) % spokes) as u32,
            EdgeKind::Border,
        ));
    }
    for i in 0..spokes {
        edges.push(e(
            (spokes + i) as u32,
            center,
            i as u32,
            if i % 2 == 0 {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            },
        ));
    }
    CreasePattern {
        vertices,
        edges,
        next_vertex_id: spokes as u32 + 1,
        next_edge_id: 2 * spokes as u32,
    }
}

/// 折り線を等間隔に並べた展開図。
fn star_cp(spokes: usize, radii: &[f64]) -> CreasePattern {
    let azimuths: Vec<f64> = (0..spokes)
        .map(|i| 360.0 * (i as f64) / (spokes as f64))
        .collect();
    star_cp_at(&azimuths, radii)
}

/// 1つの形について「固定した折り目が動かないこと」を確かめる。
///
/// - `pinned`: 固定する折り目と、その角度(**引数で明示する**)
/// - `driven`: 動かす折り目と、順に指定していく角度(**引数で明示する**)
/// - そのほかの折り線は 0° を希望として渡す(画面側と同じ渡し方)。
///
/// 返すのは「固定した折り目が動いた最大量(度)」と「縫い目の最大の開き」。
fn drive_with_pins(
    cp: &CreasePattern,
    pinned: &[(EdgeId, f64)],
    driven: EdgeId,
    steps: &[f64],
) -> (f64, f64) {
    let faces = extract_faces(cp);
    let hinges: Vec<EdgeId> = cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|edge| edge.id)
        .collect();
    let pinned_ids: Vec<EdgeId> = pinned.iter().map(|(hinge, _)| *hinge).collect();
    // 固定していない折り目は「なるべく0度のままでいてほしい」希望として渡す。
    let targets: HashMap<EdgeId, f64> = hinges
        .iter()
        .filter(|hinge| **hinge != driven && !pinned_ids.contains(hinge))
        .map(|hinge| (*hinge, 0.0))
        .collect();

    let mut warm: Option<HashMap<EdgeId, f64>> = None;
    let mut worst_pin_error = 0.0f64;
    let mut worst_gap = 0.0f64;
    for step in steps {
        let mut drivers: Vec<Driver> = pinned.iter().map(|(h, deg)| d(*h, *deg)).collect();
        drivers.push(d(driven, *step));
        let result = solve_near(cp, &faces, &drivers, &targets, warm.as_ref());
        for (hinge, deg) in pinned {
            let actual = result.angles[hinge];
            assert!(
                actual.is_finite(),
                "固定した折り目 {hinge} の角度が有限でない"
            );
            worst_pin_error = worst_pin_error.max((actual - deg).abs());
        }
        worst_gap = worst_gap.max(max_seam_gap(cp, &faces, &result.frame));
        warm = Some(result.angles);
    }
    (worst_pin_error, worst_gap)
}

/// 検査する1通りぶんの条件。角度も固定の有無もここに書いた値だけを使う。
struct PinCase {
    name: &'static str,
    cp: CreasePattern,
    /// 固定する折り目と、その角度(度)。
    pinned: Vec<(EdgeId, f64)>,
    /// 動かす折り目。
    driven: EdgeId,
    /// 動かす折り目へ順に指定する角度(度)。
    steps: Vec<f64>,
}

/// 5通りの形で、固定した折り目がほかの折り目の操作で動かないこと。
///
/// 形を変えても、固定した角度・動かす角度は**すべてここで明示している**。
#[test]
fn pinned_folds_do_not_move_while_another_fold_is_driven() {
    let shapes: Vec<PinCase> = vec![
        PinCase {
            name: "折り線5本",
            cp: star_cp(5, &[1.0]),
            pinned: vec![(5, -20.0)],
            driven: 6,
            steps: vec![-10.0, -20.0, -30.0],
        },
        PinCase {
            name: "折り線6本",
            cp: star_cp(6, &[1.0]),
            pinned: vec![(6, 25.0)],
            driven: 8,
            steps: vec![-15.0, -30.0, -45.0],
        },
        PinCase {
            name: "折り線6本(いびつな輪郭)",
            cp: star_cp(6, &[1.0, 0.75, 0.9]),
            pinned: vec![(6, -35.0)],
            driven: 9,
            steps: vec![10.0, 20.0, 35.0],
        },
        PinCase {
            name: "折り線8本(2本を固定)",
            cp: star_cp(8, &[1.0]),
            pinned: vec![(8, 30.0), (10, -40.0)],
            driven: 12,
            steps: vec![-20.0, -40.0, -60.0],
        },
        PinCase {
            name: "折り線8本(いびつな輪郭・3本を固定)",
            cp: star_cp(8, &[1.0, 0.8]),
            pinned: vec![(8, -12.5), (11, 47.5), (13, -33.0)],
            driven: 9,
            steps: vec![15.0, 30.0, 50.0],
        },
    ];

    for case in shapes {
        let name = case.name;
        let (pin_error, gap) = drive_with_pins(&case.cp, &case.pinned, case.driven, &case.steps);
        println!("{name}: 固定した折り目のずれ={pin_error:e}度 / 縫い目の開き={gap:e}");
        assert!(
            pin_error < PIN_TOLERANCE_DEG,
            "{name}: 固定した折り目が動いた: {pin_error}度(許容 {PIN_TOLERANCE_DEG}度)"
        );
        assert!(
            gap < CONNECTED_SEAM_LIMIT,
            "{name}: 紙が裂けた: {gap}(許容 {CONNECTED_SEAM_LIMIT})"
        );
    }
}

/// 固定を外すと、その折り目は追従して動けるようになる。
///
/// 「動かせる」ことだけを見て、動く量そのものを期待値にしない
/// (量は形と計算機で変わり得るため)。
#[test]
fn released_folds_follow_again() {
    let cp = star_cp(6, &[1.0]);
    let faces = extract_faces(&cp);
    let hinges: Vec<EdgeId> = (6..12).collect();

    // 折り目8を固定して、折り目6を-60度まで動かす。
    let pinned_result = {
        let targets: HashMap<EdgeId, f64> = hinges
            .iter()
            .filter(|h| **h != 6 && **h != 8)
            .map(|h| (*h, 0.0))
            .collect();
        solve_near(&cp, &faces, &[d(6, -60.0), d(8, 0.0)], &targets, None)
    };
    // 同じ操作で、折り目8の固定だけを外す(0度は希望として渡す)。
    let released_result = {
        let targets: HashMap<EdgeId, f64> = hinges
            .iter()
            .filter(|h| **h != 6)
            .map(|h| (*h, 0.0))
            .collect();
        solve_near(&cp, &faces, &[d(6, -60.0)], &targets, None)
    };

    let pinned_angle = pinned_result.angles[&8];
    let released_angle = released_result.angles[&8];
    println!("固定したとき={pinned_angle}度 / 固定を外したとき={released_angle}度");
    assert!(
        (pinned_angle - 0.0).abs() < PIN_TOLERANCE_DEG,
        "固定した折り目が動いた: {pinned_angle}度"
    );
    assert!(
        released_angle.is_finite(),
        "固定を外した折り目の角度が有限でない"
    );
    // 外した側は「動いてよい」ことだけを見る。動く量は形と計算機で変わり得るので、
    // 固定した側の許容差(1e-9度)より大きく動いたことだけを主張する。
    assert!(
        (released_angle - 0.0).abs() > PIN_TOLERANCE_DEG,
        "固定を外しても追従しなかった: {released_angle}度"
    );
}

/// 成り立たない固定を与えても、計算は止まらず有限の形を返す。
///
/// 次数4の内部頂点1個の展開図は動かせる自由度が1しかないため、
/// 2本の折り目を別々の角度で固定することはできない。
/// それでも「結果が返る」ことと「成り立たなかったと分かる知らせが立つ」ことを見る。
#[test]
fn impossible_pins_still_return_a_finite_shape() {
    // 折り線が一直線にならない向き(0°/50°/110°/240°)にする。
    // 向かい合う2本が一直線だと、それはただの1本の折り目なので
    // 同じ角度で固定でき、成り立たない例にならない。
    let cp = star_cp_at(&[0.0, 50.0, 110.0, 240.0], &[1.0]);
    let faces = extract_faces(&cp);
    let targets: HashMap<EdgeId, f64> = HashMap::new();

    // 自由度1の形で、3通りの成り立たない組み合わせを与える。
    for (a, b) in [(-30.0, 40.0), (85.0, -20.0), (-120.0, 15.0)] {
        let result = solve_near(&cp, &faces, &[d(4, a), d(5, b)], &targets, None);
        for (hinge, angle) in &result.angles {
            assert!(
                angle.is_finite(),
                "成り立たない指定で折り目 {hinge} の角度が有限でなくなった"
            );
        }
        for face in &result.frame.faces {
            for point in &face.polygon {
                for value in point {
                    assert!(
                        value.is_finite(),
                        "成り立たない指定で頂点が有限でなくなった"
                    );
                }
            }
        }
        // 収束の報告だけに期待値を結び付けない。
        // 「収束しなかった」「紙が裂けた」のどちらかが立つことだけを見る。
        let gap = max_seam_gap(&cp, &faces, &result.frame);
        println!(
            "固定 {a}度と{b}度: 収束={} / 縫い目の開き={gap:e}",
            result.converged
        );
        assert!(
            !result.converged || gap >= SEAM_TOLERANCE,
            "成り立たない指定なのに、成り立ったと報告された(開き {gap})"
        );
    }
}

/// 同じ入力を繰り返しても、固定した折り目の扱いが変わらない。
#[test]
fn pinned_solve_is_repeatable() {
    let cp = star_cp(6, &[1.0]);
    let faces = extract_faces(&cp);
    let targets: HashMap<EdgeId, f64> = (7..12).filter(|h| *h != 8).map(|h| (h, 0.0)).collect();
    let drivers = [d(6, -45.0), d(8, 20.0)];

    let first = solve_near(&cp, &faces, &drivers, &targets, None);
    for _ in 0..9 {
        let again = solve_near(&cp, &faces, &drivers, &targets, None);
        assert_eq!(again.angles, first.angles, "同じ入力で結果が変わった");
    }
}

// ---------------------------------------------------------------------------
// 固定を使った形で、紙の重なりが壊れないこと。
//
// 展開図と折り角は、実機の `desktop.exe` から読み取った値をそのまま検査へ
// 書き込んでいる(`.gitignore` 対象の `verification/` を読まないため。§10.1)。
// `crates/ori3-rigid/tests/surface_order.rs` にある同じ姿勢を写したもの。

/// 利用者の画面で紙の裏が見えていた、実機の展開図(頂点13・辺28)。
fn live_frame_square() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            v(0, 0.0, 0.0),
            v(1, 1.0, 0.0),
            v(2, 1.0, 1.0),
            v(3, 0.0, 1.0),
            v(4, 1.0, 0.5),
            v(5, 0.0, 0.5),
            v(6, 0.5, 1.0),
            v(7, 0.5, 0.0),
            v(8, 0.5, 0.5),
            v(9, 0.5, 0.792_893_218_813_452_5),
            v(10, 0.792_893_218_813_452_5, 0.5),
            v(11, 0.5, 0.207_106_781_186_547_52),
            v(12, 0.207_106_781_186_547_52, 0.5),
        ],
        edges: vec![
            e(4, 1, 4, EdgeKind::Border),
            e(5, 4, 2, EdgeKind::Border),
            e(6, 3, 5, EdgeKind::Border),
            e(7, 5, 0, EdgeKind::Border),
            e(9, 2, 6, EdgeKind::Border),
            e(10, 6, 3, EdgeKind::Border),
            e(11, 0, 7, EdgeKind::Border),
            e(12, 7, 1, EdgeKind::Border),
            e(17, 0, 8, EdgeKind::Valley),
            e(18, 8, 2, EdgeKind::Valley),
            e(19, 6, 9, EdgeKind::Mountain),
            e(20, 9, 8, EdgeKind::Mountain),
            e(21, 2, 9, EdgeKind::Mountain),
            e(22, 4, 10, EdgeKind::Mountain),
            e(23, 10, 8, EdgeKind::Mountain),
            e(24, 2, 10, EdgeKind::Mountain),
            e(25, 10, 1, EdgeKind::Mountain),
            e(26, 8, 11, EdgeKind::Mountain),
            e(27, 11, 7, EdgeKind::Mountain),
            e(28, 0, 11, EdgeKind::Mountain),
            e(29, 11, 1, EdgeKind::Mountain),
            e(30, 8, 12, EdgeKind::Mountain),
            e(31, 12, 5, EdgeKind::Mountain),
            e(32, 0, 12, EdgeKind::Mountain),
            e(33, 12, 3, EdgeKind::Mountain),
            e(34, 3, 9, EdgeKind::Mountain),
            e(35, 12, 9, EdgeKind::Valley),
            e(36, 10, 11, EdgeKind::Valley),
        ],
        next_vertex_id: 13,
        next_edge_id: 37,
    }
}

/// 実機が表示していた20本の折り角(そのまま写した値)。
fn live_frame_angles() -> Vec<(EdgeId, f64)> {
    vec![
        (17, -180.0),
        (18, -180.0),
        (19, -178.265_130_385_534_97),
        (20, 180.0),
        (21, 180.0),
        (22, -3.062_204_584_590_538_5e-15),
        (23, 180.0),
        (24, 180.0),
        (25, 180.0),
        (26, 180.0),
        (27, -5.233_885_113_024_099e-15),
        (28, 180.0),
        (29, 180.0),
        (30, 179.999_999_999_999_97),
        (31, -178.265_130_385_534_97),
        (32, 180.0),
        (33, 180.0),
        (34, 180.0),
        (35, -1.734_869_614_465_027),
        (36, -180.0),
    ]
}

/// 重なり順(`surface_rank`)を下から順の面IDに直す。
fn rank_order(frame: &ori3_model::Frame3D) -> Vec<u32> {
    let mut ranked: Vec<(u32, u32)> = frame
        .faces
        .iter()
        .map(|face| (face.surface_rank, face.face))
        .collect();
    ranked.sort_unstable();
    ranked.into_iter().map(|(_, face)| face).collect()
}

/// 固定を使った形で、紙の重なりが壊れないこと。
///
/// 実機で利用者が「折り目35を -35 度にしたい」と操作したときの姿勢を使う。
/// 折り切っている17本と折り目35を固定し(角度は下で明示している)、
/// 残る2本(19・31)だけを譲れる側にする。
#[test]
fn pinned_pose_keeps_the_paper_stack_sound() {
    let cp = live_frame_square();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 16, "この姿勢は16面のはず");

    // 固定する折り目と角度を、ここで明示して与える(計算結果から取らない)。
    let mut pinned: Vec<Driver> = live_frame_angles()
        .into_iter()
        .filter(|(hinge, _)| *hinge != 19 && *hinge != 31 && *hinge != 35)
        .map(|(hinge, deg)| d(hinge, deg))
        .collect();
    pinned.sort_by_key(|driver| driver.hinge);
    assert_eq!(pinned.len(), 17, "折り切っている折り目は17本のはず");
    // 利用者が動かしている折り目。要求どおり -35 度で固定する。
    let requested = d(35, -35.0);
    // 譲れる側(なるべく守るだけ)。
    let targets: HashMap<EdgeId, f64> =
        HashMap::from([(19, -178.265_130_385_534_97), (31, -178.265_130_385_534_97)]);

    let mut drivers = pinned.clone();
    drivers.push(requested.clone());
    let result = solve_near(&cp, &faces, &drivers, &targets, None);

    // 1. 固定した折り目は動いていない。
    let mut worst = 0.0f64;
    for driver in &pinned {
        let actual = result.angles[&driver.hinge];
        worst = worst.max((actual - driver.target_angle_deg).abs());
    }
    let requested_error = (result.angles[&35] - requested.target_angle_deg).abs();
    println!("固定17本のずれ={worst:e}度 / 折り目35のずれ={requested_error:e}度");
    assert!(
        worst < PIN_TOLERANCE_DEG,
        "固定した折り目が動いた: {worst}度"
    );
    assert!(
        requested_error < PIN_TOLERANCE_DEG,
        "利用者の要求どおりにならなかった: {requested_error}度"
    );

    // 2. 紙が裂けていない。
    let gap = max_seam_gap(&cp, &faces, &result.frame);
    println!("縫い目の開き={gap:e}");
    assert!(gap < SEAM_TOLERANCE, "紙が裂けた: {gap}");

    // 3. 面どうしが突き抜けていない(見えないはずの紙の裏が出る原因)。
    let intersections = self_intersection_pairs(&result.frame);
    assert!(
        intersections.is_empty(),
        "面が突き抜けている: {intersections:?}"
    );

    // 4. 重なり順が面の番号順になっていない。
    //
    // 重なりが決まらなくなると、順序はほぼ面の番号順(0,1,2,…)へ落ちる。
    // 実測では、束が 2.473e-3 ばらけた壊れた姿勢の順序が
    // face0,1,3,4,6,7,8,9,11,12,… と番号順に近くなり、
    // 正しい姿勢では face3 が一番下だった(scratchpad/layer-rank-fix-report.md)。
    // ここは「番号順ではない」という質の違いを見るので、計算機が変わっても成り立つ。
    let order = rank_order(&result.frame);
    let numbered: Vec<u32> = {
        let mut ids: Vec<u32> = faces.iter().map(|face| face.id).collect();
        ids.sort_unstable();
        ids
    };
    println!("重なり順(下から)={order:?}");
    assert_eq!(order.len(), faces.len(), "全ての面に重なり順が付くはず");
    assert_ne!(
        order, numbered,
        "重なり順が面の番号順のまま(重なりが決まっていない)"
    );

    // 5. 平らに畳んだ束の高さがばらけていない。
    //
    //    この姿勢は全体が平らではない(折り目35を -35 度に開いているので、
    //    その先の羽根は傾いている)。ばらけを測るのは、**水平に重なっている面
    //    だけの束**である。傾いた羽根まで混ぜて全体の高さを測ると 2.868e-1 に
    //    なるが、それは折り目35を開いた分であって、重なりの乱れではない。
    //
    //    実測では、この束のばらけは固定しないと 2.473e-3、固定すると 8.33e-16
    //    に収まった(scratchpad/layer-rank-fix-report.md)。上限は
    //    「3D表示が同じ高さとみなせる幅 2.78e-5」とし、実測値そのものは境目にしない。
    let spread = flat_stack_spread(&result.frame);
    println!("水平に重なった束のばらけ={spread:e}");
    assert!(
        spread < 2.78e-5,
        "平らに畳んだはずの束がばらけている: {spread}"
    );
}

/// 固定を全部外した場合と比べて、動いた折り目の本数と最大のずれを測る。
///
/// 合否は付けず、報告のために数値を出す(どちらの形も計算は止まらない)。
#[test]
fn pinned_versus_unpinned_movement_report() {
    let cp = live_frame_square();
    let faces = extract_faces(&cp);
    let live = live_frame_angles();
    let pinned: Vec<Driver> = live
        .iter()
        .filter(|(hinge, _)| *hinge != 19 && *hinge != 31 && *hinge != 35)
        .map(|(hinge, deg)| d(*hinge, *deg))
        .collect();
    let requested = d(35, -35.0);

    // 固定あり: 17本+要求の1本を動かさない側へ。
    let mut with_pins = pinned.clone();
    with_pins.push(requested.clone());
    let targets_with: HashMap<EdgeId, f64> =
        HashMap::from([(19, -178.265_130_385_534_97), (31, -178.265_130_385_534_97)]);
    let held = solve_near(&cp, &faces, &with_pins, &targets_with, None);

    // 固定なし: 要求の1本だけを動かさない側へ(いままでの動き)。
    let targets_without: HashMap<EdgeId, f64> = live
        .iter()
        .filter(|(hinge, _)| *hinge != 35)
        .map(|(hinge, deg)| (*hinge, *deg))
        .collect();
    let free = solve_near(&cp, &faces, &[requested], &targets_without, None);

    let count_moved = |angles: &HashMap<EdgeId, f64>| {
        live.iter()
            .filter(|(hinge, deg)| {
                *hinge != 35
                    && canonical_delta_deg(angles[hinge], *deg).abs() > PIN_RELEASE_REPORT_EPS_DEG
            })
            .count()
    };
    let worst_moved = |angles: &HashMap<EdgeId, f64>| {
        live.iter()
            .filter(|(hinge, _)| *hinge != 35)
            .map(|(hinge, deg)| canonical_delta_deg(angles[hinge], *deg).abs())
            .fold(0.0f64, f64::max)
    };

    println!(
        "固定あり: 動いた折り目={}本 / 最大のずれ={:.3}度",
        count_moved(&held.angles),
        worst_moved(&held.angles)
    );
    println!(
        "固定なし: 動いた折り目={}本 / 最大のずれ={:.3}度",
        count_moved(&free.angles),
        worst_moved(&free.angles)
    );
    let held_spread = flat_stack_spread(&held.frame);
    let free_spread = flat_stack_spread(&free.frame);
    println!("束のばらけ: 固定あり={held_spread:e} / 固定なし={free_spread:e}");
    println!(
        "折り目35: 固定あり={:.6}度 / 固定なし={:.6}度(要求 -35度)",
        held.angles[&35], free.angles[&35]
    );
}

/// 水平に重なっている面だけを取り出し、その高さの広がりを返す。
///
/// 傾いた面(この姿勢では開いた羽根)を混ぜると、重なりの乱れではなく
/// 折り目を開いた分を測ってしまうので、法線がほぼ真上・真下の面に限る。
fn flat_stack_spread(frame: &ori3_model::Frame3D) -> f64 {
    let mut lowest = f64::INFINITY;
    let mut highest = f64::NEG_INFINITY;
    for face in &frame.faces {
        if face.polygon.len() < 3 {
            continue;
        }
        let p0 = DVec3::from_array(face.polygon[0]);
        let p1 = DVec3::from_array(face.polygon[1]);
        let p2 = DVec3::from_array(face.polygon[2]);
        let normal = (p1 - p0).cross(p2 - p0);
        if normal.length() < 1e-12 {
            continue;
        }
        // 水平(法線が真上か真下)から 1e-6 以上ずれている面は束に入れない
        if (normal.normalize().z.abs() - 1.0).abs() > 1e-6 {
            continue;
        }
        for point in &face.polygon {
            lowest = lowest.min(point[2]);
            highest = highest.max(point[2]);
        }
    }
    if lowest > highest {
        0.0
    } else {
        highest - lowest
    }
}

/// 指定と実際の角度の差。±180°をまたぐ回り方は近いほうで測る
/// (山折り180°と谷折り-180°は同じ折り切りなので、359度動いたとは数えない)。
fn canonical_delta_deg(actual: f64, target: f64) -> f64 {
    let raw = actual - target;
    if (-180.0..=180.0).contains(&raw) {
        return raw;
    }
    let wrapped = (raw + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped == -180.0 && raw > 0.0 {
        180.0
    } else {
        wrapped
    }
}
