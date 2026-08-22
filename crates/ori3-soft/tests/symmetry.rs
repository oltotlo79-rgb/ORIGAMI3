//! 共有頂点を半回転対称な最近接位置へ平均する後処理のテスト。

use ori3_soft::{HalfTurnSymmetryError, HalfTurnSymmetrySettings, enforce_half_turn_symmetry};

// 統合受入検査から依存追加なしで単独利用できることもコンパイルで保証する。
#[path = "../src/symmetry.rs"]
mod standalone_symmetry;

fn settings() -> HalfTurnSymmetrySettings {
    HalfTurnSymmetrySettings {
        center: [1.0, 2.0, 3.0],
        axis: [0.0, 0.0, 2.0],
    }
}

fn close(actual: [f64; 3], expected: [f64; 3]) {
    for index in 0..3 {
        assert!(
            (actual[index] - expected[index]).abs() < 1e-12,
            "座標{index}: {} != {} ({actual:?})",
            actual[index],
            expected[index]
        );
    }
}

#[test]
fn pair_moves_to_the_least_squares_half_turn_positions() {
    let mut positions = vec![[3.0, 2.0, 4.0], [0.0, 1.0, 4.0], [9.0, 8.0, 7.0]];
    let untouched = positions[2];
    let report = enforce_half_turn_symmetry(&mut positions, &[[0, 1]], &settings()).unwrap();

    close(positions[0], [2.5, 2.5, 4.0]);
    close(positions[1], [-0.5, 1.5, 4.0]);
    assert_eq!(positions[2], untouched, "非選択頂点は不変");
    assert_eq!(report.pairs, 1);
    assert_eq!(report.selected_vertices, 2);
    assert_eq!(report.moved_vertices, 2);
    assert!((report.max_displacement - 0.5_f64.sqrt()).abs() < 1e-12);
}

#[test]
fn arbitrary_axis_pair_order_and_pair_direction_are_deterministic() {
    let symmetry = HalfTurnSymmetrySettings {
        center: [0.0; 3],
        axis: [1.0, 1.0, 0.0],
    };
    let source = vec![
        [1.0, 4.0, 2.0],
        [5.0, -1.0, 8.0],
        [2.0, 3.0, 1.0],
        [-4.0, 6.0, -2.0],
        [7.0, 7.0, 7.0],
    ];
    let mut forward = source.clone();
    let mut reversed = source;
    let a = enforce_half_turn_symmetry(&mut forward, &[[0, 1], [2, 3]], &symmetry).unwrap();
    let b = enforce_half_turn_symmetry(&mut reversed, &[[3, 2], [1, 0]], &symmetry).unwrap();

    assert_eq!(forward, reversed, "対の向き・順序に依らず同じビット列");
    assert_eq!(a, b);
    close(forward[0], [0.0, 4.5, -3.0]);
    close(forward[1], [4.5, 0.0, 3.0]);
    assert_eq!(forward[4], [7.0, 7.0, 7.0]);
    assert!(forward.iter().flatten().all(|value| value.is_finite()));
}

#[test]
fn an_already_symmetric_axis_aligned_pair_is_unchanged() {
    let mut positions = vec![[3.0, 2.0, 4.0], [-1.0, 2.0, 4.0]];
    let before = positions.clone();
    let report = enforce_half_turn_symmetry(&mut positions, &[[0, 1]], &settings()).unwrap();
    assert_eq!(positions, before);
    assert_eq!(report.moved_vertices, 0);
    assert_eq!(report.max_displacement, 0.0);
}

#[test]
fn invalid_input_is_an_atomic_error() {
    let source = vec![[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let check =
        |pairs: &[[u32; 2]], bad: HalfTurnSymmetrySettings, expected: HalfTurnSymmetryError| {
            let mut positions = source.clone();
            let before = positions.clone();
            assert_eq!(
                enforce_half_turn_symmetry(&mut positions, pairs, &bad),
                Err(expected)
            );
            assert_eq!(positions, before, "エラー時は1点も動かさない");
        };

    let mut bad = settings();
    bad.axis = [0.0; 3];
    check(&[[0, 1]], bad, HalfTurnSymmetryError::DegenerateAxis);
    bad = settings();
    bad.center[0] = f64::NAN;
    check(&[[0, 1]], bad, HalfTurnSymmetryError::NonFiniteSettings);
    check(
        &[[0, 0]],
        settings(),
        HalfTurnSymmetryError::RepeatedVertex { vertex: 0 },
    );
    check(
        &[[0, 1], [1, 2]],
        settings(),
        HalfTurnSymmetryError::RepeatedVertex { vertex: 1 },
    );
    check(
        &[[0, 99]],
        settings(),
        HalfTurnSymmetryError::VertexOutOfBounds {
            vertex: 99,
            vertex_count: source.len(),
        },
    );

    let mut positions = source;
    positions[1][2] = f64::INFINITY;
    let before = positions.clone();
    assert_eq!(
        enforce_half_turn_symmetry(&mut positions, &[[0, 1]], &settings()),
        Err(HalfTurnSymmetryError::NonFiniteVertex { vertex: 1 })
    );
    assert_eq!(positions, before);
}

#[test]
fn empty_pairs_leave_every_position_untouched() {
    let mut positions = vec![[1.0, 2.0, 3.0]];
    let before = positions.clone();
    let report = enforce_half_turn_symmetry(&mut positions, &[], &settings()).unwrap();
    assert_eq!(report.pairs, 0);
    assert_eq!(report.selected_vertices, 0);
    assert_eq!(report.moved_vertices, 0);
    assert_eq!(positions, before);
}

#[test]
fn source_file_can_be_used_as_a_standalone_module() {
    let mut positions = vec![[1.0, 0.0, 0.0], [-0.8, 0.2, 0.0]];
    let report = standalone_symmetry::enforce_half_turn_symmetry(
        &mut positions,
        &[[0, 1]],
        &standalone_symmetry::HalfTurnSymmetrySettings {
            center: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
        },
    )
    .unwrap();
    assert_eq!(report.pairs, 1);
    close(positions[0], [0.9, -0.1, 0.0]);
    close(positions[1], [-0.9, 0.1, 0.0]);
}
