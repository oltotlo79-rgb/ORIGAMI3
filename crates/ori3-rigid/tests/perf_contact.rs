//! 接触診断まわりの速さ(NFR-002)。
//!
//! # このファイルができた経緯
//!
//! `crates/ori3-rigid/tests/motion.rs` と `crates/ori3-rigid/tests/intersect.rs` は
//! 動きや交差の**正しさ**を見る検査でありながら、その中で実時間の上限
//! (330ms・500ms)も判定していた。これらは最適化なしのビルド
//! (`cargo test --workspace`)でも走るため、計算機の混み具合がそのまま合否に
//! 出てしまう。同じ形をしていた `acceptance_yakko.rs` の実時間の主張は、
//! 420回の試行で65回落ちることが実測されている
//! (詳しくは `crates/ori3-rigid/tests/perf_yakko.rs` の冒頭)。
//!
//! そこで **上限値は緩めずに、判定する場所だけをここへ移した**。
//! 元のファイルには数値の主張(折り角・裂け・交差の有無・停止しないこと)を
//! 全て残してある。ここでは**最適化ありのビルドのときだけ**実時間を判定する。
//!
//! # 上限値の根拠(実測)
//!
//! 開発機 Windows 11 / `cargo test --release -p ori3-rigid --test perf_contact`
//! を20回連続実行した実測値(2026-08-16、20回とも合格・他の作業なし)。
//! 掲載しているのは20回それぞれの判定値の、最大・中央・最小。
//!
//! | 測る対象 | 最大 | 中央 | 最小 | 上限 | 最大÷上限 |
//! |---|---:|---:|---:|---:|---:|
//! | 短冊3枚 solve_motion 1回 | 536.9µs | 170.6µs | 154.6µs | 60ms | 0.0089 |
//! | 短冊3枚 自己交差の走査 1回 | 0.6µs | 0.5µs | 0.5µs | 16ms | 0.000038 |
//! | 面400枚の交差判定 1回 | 203.0µs | 181.2µs | 166.6µs | 16ms | 0.0127 |
//!
//! 自動検査の計算機は開発機より**約3.6倍遅い**実測があるため、上限は
//! 「手元の実測の最大値が上限の1/3以下」になるように取っている
//! (`CLAUDE.md` §10.6)。上の最大値を3.6倍しても 1.93ms・2.2µs・731µs で、
//! それぞれ上限の1/31・1/7300・1/22に収まる。
//!
//! 16本のCPU負荷を並走させた状態でも同じく20回連続で合格した。
//!
//! 上限をµs単位まで詰めないのは、実時間の計測が処理そのものの速さだけでなく
//! OSがそのスレッドを一時的に止める時間も拾うためである。実際、短冊3枚の
//! solve_motionは同じ計算を繰り返しても 0.222ms〜4.845ms と20倍以上ばらついた。
//! ここを詰めると今回直したのと同じ種類の不安定さを作り込むことになる。
//! 16msは「1コマ(60コマ/秒)に収まること」から取った。
//!
//! # 同じ計測を繰り返して一番良かった回を採る
//!
//! 上のばらつきは処理が遅くなったのではなく、OSがそのスレッドを止めた時間が
//! 混ざったものである。そこで同じ計測を複数回行い、**一番良かった回**で
//! 判定する。一番良かった回はOSに邪魔されなかった回なので、処理そのものの
//! 速さに近い。処理が本当に遅くなったなら全ての回が遅くなるので、
//! 性能後退は変わらず捕まる。`crates/ori3-soft/tests/perf_soft.rs` も
//! 同じ考え方で最小値を採っている。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeKind, Face3D, Frame3D, Vertex};
use ori3_rigid::{self_intersection_pairs, self_intersects, solve, solve_motion};

/// 折り操作1回(`solve_motion`)の上限。モジュール冒頭の計測記録を参照。
const SOLVE_BUDGET: Duration = Duration::from_millis(60);
/// 自己交差の走査1回の上限。モジュール冒頭の計測記録を参照。
const CONTACT_BUDGET: Duration = Duration::from_millis(16);
/// 面400枚の交差判定1回の上限。モジュール冒頭の計測記録を参照。
const LARGE_SCAN_BUDGET: Duration = Duration::from_millis(16);

/// 速さの上限は、最適化ありのビルドのときだけ判定する。
///
/// 最適化なしのビルドでは同じ処理が十数倍遅くなるため、同じ数値と比べても
/// 意味がなく、計算機の混み具合がそのまま合否になってしまう。
/// 上限値そのものは緩めていない。
fn assert_within_budget(elapsed: Duration, budget: Duration, label: &str) {
    if cfg!(debug_assertions) {
        return;
    }
    assert!(
        elapsed < budget,
        "{label}: {elapsed:?}(上限 {budget:?}。モジュール冒頭の計測記録を参照)"
    );
}

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

/// 正方形を縦3短冊へ分けた木構造(`motion.rs` の `three_strips` と同じ作り)。
fn three_strips() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            v(0, 0.0, 0.0),
            v(1, 1.0 / 3.0, 0.0),
            v(2, 2.0 / 3.0, 0.0),
            v(3, 1.0, 0.0),
            v(4, 1.0, 1.0),
            v(5, 2.0 / 3.0, 1.0),
            v(6, 1.0 / 3.0, 1.0),
            v(7, 0.0, 1.0),
        ],
        edges: vec![
            e(0, 0, 1, EdgeKind::Border),
            e(1, 1, 2, EdgeKind::Border),
            e(2, 2, 3, EdgeKind::Border),
            e(3, 3, 4, EdgeKind::Border),
            e(4, 4, 5, EdgeKind::Border),
            e(5, 5, 6, EdgeKind::Border),
            e(6, 6, 7, EdgeKind::Border),
            e(7, 7, 0, EdgeKind::Border),
            e(8, 1, 6, EdgeKind::Mountain),
            e(9, 2, 5, EdgeKind::Mountain),
        ],
        next_vertex_id: 8,
        next_edge_id: 10,
    }
}

/// 系列を一通り送って、その中で最も重かった1手の所要時間を返す。
fn worst_step_of_one_pass() -> (Duration, Duration) {
    let cp = three_strips();
    let faces = extract_faces(&cp);
    let targets = HashMap::from([(8, 150.0)]);
    let mut warm = solve(&cp, &faces, &[d(8, 150.0), d(9, 0.0)], None).angles;

    let mut upward: Vec<u32> = (0..=180).step_by(16).collect();
    if upward.last() != Some(&180) {
        upward.push(180);
    }
    let downward: Vec<u32> = upward.iter().copied().rev().collect();
    let fine_downward: Vec<u32> = (170..=180).rev().collect();

    let mut worst_solve = Duration::ZERO;
    let mut worst_contact = Duration::ZERO;
    for (label, angles) in [
        ("16°飛び 0→180", upward),
        ("16°飛び 180→0", downward),
        ("1°刻み 180→170", fine_downward),
    ] {
        for angle in angles {
            let started = Instant::now();
            let motion = solve_motion(
                &cp,
                &faces,
                &[d(9, f64::from(angle))],
                Some(&targets),
                Some(&warm),
                true,
            );
            let solve_time = started.elapsed();
            let started = Instant::now();
            let pairs = self_intersection_pairs(&motion.result.frame);
            let contact_time = started.elapsed();
            // 空回りを測っていないことだけ確かめる(正しさの判定は motion.rs)。
            assert!(
                (motion.result.angles[&9] - f64::from(angle)).abs() < 1e-9,
                "{label} {angle}°: 要求角に届いていない"
            );
            if !pairs.is_empty() {
                println!("{label} {angle}°: 交差={}組", pairs.len());
            }
            worst_solve = worst_solve.max(solve_time);
            worst_contact = worst_contact.max(contact_time);
            warm = motion.result.angles;
        }
    }
    (worst_solve, worst_contact)
}

/// 短冊3枚のヒンジ#9を送るときの、1手あたりの速さ。
///
/// 送る系列は16°飛びの往復と、畳んだ側から1°刻みで戻す最初の11手。
/// `motion.rs` は同じ動きを1°刻みの全180手で送って**交差しないこと**を確かめる。
/// ここでは**速さだけ**を見るので、重くなる区間に絞って測る。
///
/// 3回送って一番良かった回で判定する理由はモジュール冒頭に書いた。
#[test]
fn three_strips_sweep_stays_within_frame_budget() {
    let mut best_solve = Duration::MAX;
    let mut best_contact = Duration::MAX;
    for pass in 1..=3 {
        let (solve_time, contact_time) = worst_step_of_one_pass();
        println!("{pass}回目: solve最悪={solve_time:?} 自己交差の走査最悪={contact_time:?}");
        best_solve = best_solve.min(solve_time);
        best_contact = best_contact.min(contact_time);
    }
    println!(
        "短冊3枚・折り操作1回: solve最悪={best_solve:?}(上限 {SOLVE_BUDGET:?}) \
         自己交差の走査最悪={best_contact:?}(上限 {CONTACT_BUDGET:?})"
    );
    assert_within_budget(best_solve, SOLVE_BUDGET, "短冊3枚の折り操作1回");
    assert_within_budget(best_contact, CONTACT_BUDGET, "短冊3枚の自己交差の走査1回");
}

fn frame(faces: Vec<Face3D>) -> Frame3D {
    Frame3D {
        faces,
        warnings: Vec::new(),
    }
}

fn face(id: u32, polygon: &[[f64; 3]]) -> Face3D {
    Face3D {
        face: id,
        polygon: polygon.to_vec(),
        layer: 0,
        surface_rank: 0,
        mirrored: false,
    }
}

/// 面400枚(NFR-002の想定規模)を折り途中のように重ねたときの交差判定の速さ。
/// 判定は編集のたびに走るので、遅いと画面が引っかかる。
/// 「交差していないと正しく判定すること」自体は
/// `intersect.rs` の `checks_400_faces_quickly` が確かめる。
#[test]
fn four_hundred_face_scan_stays_within_frame_budget() {
    let faces: Vec<Face3D> = (0..400)
        .map(|k| {
            let z = f64::from(k) * 0.002;
            face(
                u32::try_from(k).unwrap(),
                &[
                    [0.0, 0.0, z],
                    [1.0, 0.0, z],
                    [1.0, 1.0, z + 0.001],
                    [0.0, 1.0, z + 0.001],
                ],
            )
        })
        .collect();
    let frame = frame(faces);
    // 5回測って一番良かった回で判定する(理由はモジュール冒頭)。
    let mut best = Duration::MAX;
    for _ in 0..5 {
        let started = Instant::now();
        let hit = self_intersects(&frame);
        best = best.min(started.elapsed());
        assert!(!hit, "この姿勢は交差していない");
    }
    println!("面400枚の交差判定: {best:?}(上限 {LARGE_SCAN_BUDGET:?})");
    assert_within_budget(best, LARGE_SCAN_BUDGET, "面400枚の交差判定1回");
}
