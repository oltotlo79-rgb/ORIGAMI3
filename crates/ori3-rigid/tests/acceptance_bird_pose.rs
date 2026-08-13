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
use ori3_rigid::motion::solve_motion;
use ori3_rigid::self_intersection_pairs;

/// 利用者の画面から取り出した展開図(2026-08-13)。鳥の基本形を作る途中の形。
fn user_bird_cp() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            Vertex { id: 0, pos: [0.0, 0.0] },
            Vertex { id: 1, pos: [1.0, 0.0] },
            Vertex { id: 2, pos: [1.0, 1.0] },
            Vertex { id: 3, pos: [0.0, 1.0] },
            Vertex { id: 4, pos: [0.0, 0.5] },
            Vertex { id: 5, pos: [1.0, 0.5] },
            Vertex { id: 6, pos: [0.5, 0.0] },
            Vertex { id: 7, pos: [0.5, 1.0] },
            Vertex { id: 8, pos: [0.5, 0.5] },
            Vertex { id: 9, pos: [0.5, 0.7928932188134525] },
            Vertex { id: 10, pos: [0.7928932188134525, 0.5] },
            Vertex { id: 11, pos: [0.5, 0.20710678118654752] },
            Vertex { id: 12, pos: [0.20710678118654752, 0.5] },
            Vertex { id: 13, pos: [0.6464466094067263, 0.35355339059327373] },
            Vertex { id: 14, pos: [0.2391228982492633, 0.5] },
            Vertex { id: 15, pos: [0.3535533905932737, 0.6464466094067263] },
        ],
        edges: vec![
            Edge { id: 4, v0: 3, v1: 4, kind: EdgeKind::Border },
            Edge { id: 5, v0: 4, v1: 0, kind: EdgeKind::Border },
            Edge { id: 6, v0: 1, v1: 5, kind: EdgeKind::Border },
            Edge { id: 7, v0: 5, v1: 2, kind: EdgeKind::Border },
            Edge { id: 9, v0: 0, v1: 6, kind: EdgeKind::Border },
            Edge { id: 10, v0: 6, v1: 1, kind: EdgeKind::Border },
            Edge { id: 11, v0: 2, v1: 7, kind: EdgeKind::Border },
            Edge { id: 12, v0: 7, v1: 3, kind: EdgeKind::Border },
            Edge { id: 17, v0: 0, v1: 8, kind: EdgeKind::Valley },
            Edge { id: 18, v0: 8, v1: 2, kind: EdgeKind::Valley },
            Edge { id: 21, v0: 8, v1: 9, kind: EdgeKind::Mountain },
            Edge { id: 22, v0: 9, v1: 7, kind: EdgeKind::Mountain },
            Edge { id: 23, v0: 3, v1: 9, kind: EdgeKind::Mountain },
            Edge { id: 24, v0: 9, v1: 2, kind: EdgeKind::Mountain },
            Edge { id: 25, v0: 8, v1: 10, kind: EdgeKind::Mountain },
            Edge { id: 26, v0: 10, v1: 5, kind: EdgeKind::Mountain },
            Edge { id: 27, v0: 2, v1: 10, kind: EdgeKind::Mountain },
            Edge { id: 28, v0: 10, v1: 1, kind: EdgeKind::Mountain },
            Edge { id: 29, v0: 6, v1: 11, kind: EdgeKind::Mountain },
            Edge { id: 30, v0: 11, v1: 8, kind: EdgeKind::Mountain },
            Edge { id: 31, v0: 1, v1: 11, kind: EdgeKind::Mountain },
            Edge { id: 32, v0: 0, v1: 11, kind: EdgeKind::Mountain },
            Edge { id: 33, v0: 4, v1: 12, kind: EdgeKind::Mountain },
            Edge { id: 35, v0: 0, v1: 12, kind: EdgeKind::Mountain },
            Edge { id: 36, v0: 12, v1: 3, kind: EdgeKind::Mountain },
            Edge { id: 37, v0: 8, v1: 13, kind: EdgeKind::Valley },
            Edge { id: 38, v0: 13, v1: 1, kind: EdgeKind::Valley },
            Edge { id: 39, v0: 11, v1: 13, kind: EdgeKind::Valley },
            Edge { id: 40, v0: 13, v1: 10, kind: EdgeKind::Valley },
            Edge { id: 41, v0: 12, v1: 14, kind: EdgeKind::Mountain },
            Edge { id: 42, v0: 14, v1: 8, kind: EdgeKind::Mountain },
            Edge { id: 43, v0: 3, v1: 15, kind: EdgeKind::Valley },
            Edge { id: 44, v0: 15, v1: 8, kind: EdgeKind::Valley },
            Edge { id: 45, v0: 9, v1: 15, kind: EdgeKind::Valley },
            Edge { id: 46, v0: 15, v1: 12, kind: EdgeKind::Valley },
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
    let start = solve_motion(&cp, &faces, &start_drivers, Some(&targets), Some(&flat), true).result;
    assert!(start.converged, "出発の姿勢が閉じない");
    assert!(
        self_intersection_pairs(&start.frame).is_empty(),
        "出発の姿勢で紙が食い込んでいる"
    );

    let mut warm = start.angles.clone();
    let mut worst_rms = 0.0_f64;
    for step in 1..=36u32 {
        let angle = -5.0 * f64::from(step);
        let drivers = vec![
            Driver {
                hinge: 40,
                target_angle_deg: angle,
            },
            Driver {
                hinge: 45,
                target_angle_deg: angle,
            },
        ];
        let solved = solve_motion(&cp, &faces, &drivers, Some(&targets), Some(&warm), true).result;

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
        assert!(
            worst < 1e-6,
            "{angle}°で折ってある折り目がほどけた(最大 {worst:.1}°、{}本)",
            solved.relaxations.len()
        );
        worst_rms = worst_rms.max(solved.closure_rms);
        warm = solved.angles.clone();
    }
    assert!(
        worst_rms < 1e-12,
        "36段すべて閉じるが、最悪の閉包RMSが大きすぎる: {worst_rms:.3e}"
    );
}
