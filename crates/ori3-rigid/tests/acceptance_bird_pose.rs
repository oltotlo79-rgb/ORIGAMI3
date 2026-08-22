//! 利用者の画面で見つかった不具合の再発防止。
//!
//! 症状(2026-08-13): 鳥の基本形を作る途中の展開図で、8本を180°・2本を0°に
//! 折った状態から、別の2本を選んで−180°へ向けて折っていくと、**既に折ってある
//! 折り目が最大179.3°ほどけ**、紙どうしが食い込んだ。実際の紙では、折ってある
//! ところはそのままで折り進められる。
//!
//! 原因は、希望角(中優先)が「譲る必要が無い場面でも譲っていた」こと。
//! 同じ姿勢を12本すべて固定して解くと、−10°でも−90°でも−180°でも
//! 閉包1e-14以下・自己交差0組で解けることを実測して確かめた。

use std::collections::HashMap;

use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeKind, Vertex};
use ori3_rigid::motion::{MotionContactOptions, solve_motion_with_contact_options};
use ori3_rigid::self_intersection_pairs;

/// 利用者の画面から取り出した展開図(2026-08-13)。鳥の基本形を作る途中の形。
fn user_bird_cp() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            Vertex {
                id: 0,
                pos: [0.0, 0.0],
            },
            Vertex {
                id: 1,
                pos: [1.0, 0.0],
            },
            Vertex {
                id: 2,
                pos: [1.0, 1.0],
            },
            Vertex {
                id: 3,
                pos: [0.0, 1.0],
            },
            Vertex {
                id: 4,
                pos: [0.0, 0.5],
            },
            Vertex {
                id: 5,
                pos: [1.0, 0.5],
            },
            Vertex {
                id: 6,
                pos: [0.5, 0.0],
            },
            Vertex {
                id: 7,
                pos: [0.5, 1.0],
            },
            Vertex {
                id: 8,
                pos: [0.5, 0.5],
            },
            Vertex {
                id: 9,
                pos: [0.5, 0.7928932188134525],
            },
            Vertex {
                id: 10,
                pos: [0.7928932188134525, 0.5],
            },
            Vertex {
                id: 11,
                pos: [0.5, 0.20710678118654752],
            },
            Vertex {
                id: 12,
                pos: [0.20710678118654752, 0.5],
            },
            Vertex {
                id: 13,
                pos: [0.6464466094067263, 0.35355339059327373],
            },
            Vertex {
                id: 14,
                pos: [0.2391228982492633, 0.5],
            },
            Vertex {
                id: 15,
                pos: [0.3535533905932737, 0.6464466094067263],
            },
        ],
        edges: vec![
            Edge {
                id: 4,
                v0: 3,
                v1: 4,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 5,
                v0: 4,
                v1: 0,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 6,
                v0: 1,
                v1: 5,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 7,
                v0: 5,
                v1: 2,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 9,
                v0: 0,
                v1: 6,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 10,
                v0: 6,
                v1: 1,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 11,
                v0: 2,
                v1: 7,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 12,
                v0: 7,
                v1: 3,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 17,
                v0: 0,
                v1: 8,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 18,
                v0: 8,
                v1: 2,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 21,
                v0: 8,
                v1: 9,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 22,
                v0: 9,
                v1: 7,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 23,
                v0: 3,
                v1: 9,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 24,
                v0: 9,
                v1: 2,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 25,
                v0: 8,
                v1: 10,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 26,
                v0: 10,
                v1: 5,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 27,
                v0: 2,
                v1: 10,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 28,
                v0: 10,
                v1: 1,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 29,
                v0: 6,
                v1: 11,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 30,
                v0: 11,
                v1: 8,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 31,
                v0: 1,
                v1: 11,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 32,
                v0: 0,
                v1: 11,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 33,
                v0: 4,
                v1: 12,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 35,
                v0: 0,
                v1: 12,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 36,
                v0: 12,
                v1: 3,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 37,
                v0: 8,
                v1: 13,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 38,
                v0: 13,
                v1: 1,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 39,
                v0: 11,
                v1: 13,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 40,
                v0: 13,
                v1: 10,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 41,
                v0: 12,
                v1: 14,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 42,
                v0: 14,
                v1: 8,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 43,
                v0: 3,
                v1: 15,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 44,
                v0: 15,
                v1: 8,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 45,
                v0: 9,
                v1: 15,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 46,
                v0: 15,
                v1: 12,
                kind: EdgeKind::Valley,
            },
        ],
        next_vertex_id: 16,
        next_edge_id: 47,
    }
}

/// 利用者が指定していた角度。8本を180°、2本を0°。
fn user_targets() -> HashMap<u32, f64> {
    HashMap::from([
        (17, 0.0),
        (18, 0.0),
        (23, 180.0),
        (24, 180.0),
        (27, 180.0),
        (28, 180.0),
        (31, 180.0),
        (32, 180.0),
        (35, 180.0),
        (36, 180.0),
    ])
}

/// 折ってある折り目を保ったまま、別の2本を−180°まで折り進められること。
///
/// 上限値の根拠(この修正後の実測、36段すべて):
/// 閉包RMSの最悪 4.643e-15、自己交差 0組、希望角からの譲り 0本。
/// 指定した折り目を1本ずつ順に送り、毎段で「閉じる・食い込まない・ほどけない」ことを確かめる。
fn sweep(
    cp: &CreasePattern,
    faces: &[ori3_cp::Face],
    targets: &HashMap<u32, f64>,
    driven: &[u32],
    sign: f64,
    start: &HashMap<u32, f64>,
    label: &str,
) {
    let mut warm = start.clone();
    for step in 1..=36u32 {
        let angle = sign * 5.0 * f64::from(step);
        let drivers: Vec<Driver> = driven
            .iter()
            .map(|&hinge| Driver {
                hinge,
                target_angle_deg: angle,
            })
            .collect();
        let solved = solve_motion_with_contact_options(
            cp,
            faces,
            &drivers,
            Some(targets),
            Some(&warm),
            MotionContactOptions {
                detect: true,
                prevent: true,
            },
        )
        .result;

        assert!(
            solved.converged,
            "{label} {angle}°で紙が閉じない(閉包RMS {:.3e})",
            solved.closure_rms
        );
        let pairs = self_intersection_pairs(&solved.frame);
        assert!(
            pairs.is_empty(),
            "{label} {angle}°で紙が食い込んでいる({}組)",
            pairs.len()
        );
        let worst = solved
            .relaxations
            .iter()
            .map(|relaxation| relaxation.delta_deg.abs())
            .fold(0.0_f64, f64::max);
        assert!(
            worst < 1e-6,
            "{label} {angle}°で折ってある折り目がほどけた(最大 {worst:.1}°、{}本)",
            solved.relaxations.len()
        );
        warm = solved.angles.clone();
    }
}

/// 指定した折り目をまとめて山折りしていっても、紙が食い込まないこと。
///
/// 利用者の報告(2026-08-13): 「指定した線を同時に山折りしていったら
/// 鶴の花弁折りした状態になるが貫通になる」。実際の紙ではそうならない。
#[test]
fn folding_the_selected_creases_together_never_penetrates() {
    let cp = user_bird_cp();
    let faces = extract_faces(&cp);
    let flat: HashMap<u32, f64> = cp
        .edges
        .iter()
        .filter(|edge| edge.kind != EdgeKind::Border)
        .map(|edge| (edge.id, 0.0))
        .collect();
    let keep_flat: HashMap<u32, f64> = HashMap::from([(17, 0.0), (18, 0.0)]);
    sweep(
        &cp,
        &faces,
        &keep_flat,
        &[23, 24, 27, 28, 31, 32, 35, 36],
        1.0,
        &flat,
        "まとめて山折り",
    );
}

#[test]
fn folding_two_more_creases_keeps_the_folded_ones() {
    let cp = user_bird_cp();
    let faces = extract_faces(&cp);
    let targets = user_targets();

    let flat: HashMap<u32, f64> = cp
        .edges
        .iter()
        .filter(|edge| edge.kind != EdgeKind::Border)
        .map(|edge| (edge.id, 0.0))
        .collect();
    let start_drivers: Vec<Driver> = targets
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect();
    let start = solve_motion_with_contact_options(
        &cp,
        &faces,
        &start_drivers,
        Some(&targets),
        Some(&flat),
        MotionContactOptions {
            detect: true,
            prevent: true,
        },
    )
    .result;
    assert!(start.converged, "出発の姿勢が閉じない");
    assert!(
        self_intersection_pairs(&start.frame).is_empty(),
        "出発の姿勢で紙が食い込んでいる"
    );

    // 谷折り(利用者が報告した向き)と山折りの両方を確かめる
    sweep(
        &cp,
        &faces,
        &targets,
        &[40, 45],
        -1.0,
        &start.angles,
        "谷折り",
    );
    sweep(
        &cp,
        &faces,
        &targets,
        &[40, 45],
        1.0,
        &start.angles,
        "山折り",
    );
}

/// 利用者の画面から取り出した展開図(2026-08-13)。鶴の基本形を作る途中。
fn user_cp2() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            Vertex {
                id: 0,
                pos: [0.0, 0.0],
            },
            Vertex {
                id: 1,
                pos: [1.0, 0.0],
            },
            Vertex {
                id: 2,
                pos: [1.0, 1.0],
            },
            Vertex {
                id: 3,
                pos: [0.0, 1.0],
            },
            Vertex {
                id: 4,
                pos: [0.0, 0.5],
            },
            Vertex {
                id: 5,
                pos: [1.0, 0.5],
            },
            Vertex {
                id: 6,
                pos: [0.5, 1.0],
            },
            Vertex {
                id: 7,
                pos: [0.5, 0.0],
            },
            Vertex {
                id: 8,
                pos: [0.5, 0.5],
            },
            Vertex {
                id: 9,
                pos: [0.7928932188134525, 0.5],
            },
            Vertex {
                id: 10,
                pos: [0.5, 0.20710678118654752],
            },
            Vertex {
                id: 11,
                pos: [0.25, 0.5],
            },
            Vertex {
                id: 12,
                pos: [0.5, 0.7928932188134525],
            },
        ],
        edges: vec![
            Edge {
                id: 4,
                v0: 3,
                v1: 4,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 5,
                v0: 4,
                v1: 0,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 6,
                v0: 1,
                v1: 5,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 7,
                v0: 5,
                v1: 2,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 9,
                v0: 2,
                v1: 6,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 10,
                v0: 6,
                v1: 3,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 11,
                v0: 0,
                v1: 7,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 12,
                v0: 7,
                v1: 1,
                kind: EdgeKind::Border,
            },
            Edge {
                id: 17,
                v0: 0,
                v1: 8,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 18,
                v0: 8,
                v1: 2,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 19,
                v0: 8,
                v1: 9,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 20,
                v0: 9,
                v1: 5,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 21,
                v0: 2,
                v1: 9,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 22,
                v0: 9,
                v1: 1,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 23,
                v0: 8,
                v1: 10,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 24,
                v0: 10,
                v1: 7,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 25,
                v0: 0,
                v1: 10,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 26,
                v0: 10,
                v1: 1,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 27,
                v0: 4,
                v1: 11,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 28,
                v0: 11,
                v1: 8,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 29,
                v0: 0,
                v1: 11,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 30,
                v0: 11,
                v1: 3,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 31,
                v0: 6,
                v1: 12,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 32,
                v0: 12,
                v1: 8,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 33,
                v0: 2,
                v1: 12,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 34,
                v0: 12,
                v1: 3,
                kind: EdgeKind::Mountain,
            },
            Edge {
                id: 35,
                v0: 11,
                v1: 12,
                kind: EdgeKind::Valley,
            },
            Edge {
                id: 36,
                v0: 10,
                v1: 9,
                kind: EdgeKind::Valley,
            },
        ],
        next_vertex_id: 13,
        next_edge_id: 37,
    }
}

/// 選んだ折り目を「まとめて動かす」でそろえて折っても、紙が閉じ続けること。
///
/// 利用者の報告(2026-08-13): 「指定した線を同時に山折りしていったら鶴の
/// 花弁折りした状態になるが貫通になる」。
///
/// 原因は、選んだ折り目を**全て同じ角度ちょうどで固定**していたこと。
/// 実際の紙では折り目どうしがわずかに違う角度を取る必要がある。8本を147°で
/// 固定すると紙が閉じない(閉包RMS 8.714e-3)が、代表1本だけを固定して残りを
/// 希望角にすると、45°・90°・147°のいずれでも閉包1e-15以下・食い込み0組で解け、
/// 8本の実際の角度は要求から1.6°以内に収まる(実測)。
///
/// 画面側は `setDriverAngles` で代表1本だけを固定して送るようになっている。
#[test]
fn moving_the_selected_creases_together_keeps_the_paper_closed() {
    let cp = user_cp2();
    let faces = extract_faces(&cp);
    let group = [21u32, 22, 25, 26, 29, 30, 33, 34];

    let mut warm: HashMap<u32, f64> = cp
        .edges
        .iter()
        .filter(|edge| edge.kind != EdgeKind::Border)
        .map(|edge| (edge.id, 0.0))
        .collect();
    // 5°から180°まで36段すべてを確かめる。以前は170°〜179°で別の形へ飛び、
    // 食い込みが最大1.053e-1(紙の一辺の10%)になっていた。前の姿勢から追って
    // 食い込みが残るときは平らから1回だけ追い直すようにして、0組になった。
    for step in 1..=36u32 {
        let angle = 5.0 * f64::from(step);
        let drivers = vec![Driver {
            hinge: group[0],
            target_angle_deg: angle,
        }];
        let targets: HashMap<u32, f64> =
            group.iter().skip(1).map(|&hinge| (hinge, angle)).collect();
        let solved = solve_motion_with_contact_options(
            &cp,
            &faces,
            &drivers,
            Some(&targets),
            Some(&warm),
            MotionContactOptions {
                detect: true,
                prevent: true,
            },
        )
        .result;

        assert!(
            solved.converged,
            "{angle}°で紙が閉じない(閉包RMS {:.3e})",
            solved.closure_rms
        );
        let pairs = self_intersection_pairs(&solved.frame);
        assert!(
            pairs.is_empty(),
            "{angle}°で紙が食い込んでいる({}組)",
            pairs.len()
        );
        let worst = solved
            .relaxations
            .iter()
            .map(|relaxation| relaxation.delta_deg.abs())
            .fold(0.0_f64, f64::max);
        // 指定からのずれ。実測では145°まで最大1.7°に収まる。
        //
        // 148.5°から149°の間で解の枝が変わり、そこから先は最大33.4°ずれる。
        // これは計算の失敗ではなく、この展開図で8本をほぼ同じ角度に保てる形が
        // その角度で終わっているため(どの初期値から解いても同じ値になることを実測)。
        // 実際の紙でも花弁折りはここで一気に入る。よって上限は145°までに掛ける。
        if angle <= 145.0 {
            assert!(
                worst < 2.0,
                "{angle}°で指定から離れすぎた(最大 {worst:.1}°)"
            );
        }
        warm = solved.angles.clone();
    }
}
