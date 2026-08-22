//! 中心を立ち上げ、外周へ滑らかにつなぐ回転対称カップ変形のテスト。

use ori3_soft::{RadialCupError, RadialCupSettings, radial_cup_vertices};

// 統合受入検査と同じ、依存追加なしの単独読み込みもコンパイルで保証する。
#[path = "../src/cup.rs"]
mod standalone_cup;

fn settings(height: f64) -> RadialCupSettings {
    RadialCupSettings {
        center: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 4.0],
        inner_radius: 0.25,
        outer_radius: 1.0,
        height,
    }
}

fn symmetric_points() -> Vec<[f64; 3]> {
    vec![
        [0.0, 0.0, 0.0],
        [0.25, 0.0, 0.0],
        [0.0, 0.25, 0.0],
        [-0.25, 0.0, 0.0],
        [0.0, -0.25, 0.0],
        [0.625, 0.0, 0.0],
        [0.0, 0.625, 0.0],
        [-0.625, 0.0, 0.0],
        [0.0, -0.625, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [1.25, 0.0, 0.0],
        [9.0, 9.0, 9.0], // 非選択頂点
    ]
}

#[test]
fn center_rises_outer_ring_is_fixed_and_fourfold_symmetry_is_kept() {
    let mut positions = symmetric_points();
    let before = positions.clone();
    let selected: Vec<u32> = (0..14).collect();
    let report = radial_cup_vertices(&mut positions, &selected, &settings(0.6)).unwrap();

    assert_eq!(report.selected_vertices, 14);
    assert_eq!(report.moved_vertices, 9, "中心・内周・中間周だけを動かす");
    assert!((report.max_displacement - 0.6).abs() < 1e-12);
    for &vertex in &[0, 1, 2, 3, 4] {
        assert_eq!(positions[vertex][2], 0.6, "中心面は同じ高さ");
    }
    for group in [[5, 6, 7, 8], [9, 10, 11, 12]] {
        let heights: Vec<f64> = group.iter().map(|&vertex| positions[vertex][2]).collect();
        assert!(
            heights.windows(2).all(|pair| pair[0] == pair[1]),
            "4回回転対称: {heights:?}"
        );
    }
    assert!((positions[5][2] - 0.3).abs() < 1e-12, "壁の中点は高さ半分");
    for &vertex in &[9, 10, 11, 12, 13] {
        assert_eq!(positions[vertex], before[vertex], "外周と外側を厳密固定");
    }
    assert_eq!(positions[14], before[14], "非選択頂点は不変");
    assert!(positions.iter().flatten().all(|value| value.is_finite()));
}

#[test]
fn existing_axial_shape_is_preserved_while_lift_is_added() {
    let mut positions = vec![[0.0, 0.0, 0.2], [0.625, 0.0, -0.1], [1.0, 0.0, 0.3]];
    radial_cup_vertices(&mut positions, &[0, 1, 2], &settings(-0.4)).unwrap();
    assert!((positions[0][2] - (-0.2)).abs() < 1e-12);
    assert!((positions[1][2] - (-0.3)).abs() < 1e-12);
    assert_eq!(positions[2], [1.0, 0.0, 0.3]);
}

#[test]
fn selection_is_deterministic_and_duplicate_safe() {
    let mut ordered = symmetric_points();
    let mut shuffled = symmetric_points();
    let a = radial_cup_vertices(&mut ordered, &[0, 1, 5, 9], &settings(0.5)).unwrap();
    let b = radial_cup_vertices(&mut shuffled, &[9, 5, 1, 5, 0, 1], &settings(0.5)).unwrap();
    assert_eq!(ordered, shuffled);
    assert_eq!(a, b);
}

#[test]
fn invalid_input_is_an_atomic_error() {
    let check = |vertices: &[u32], bad: RadialCupSettings, expected: RadialCupError| {
        let mut positions = symmetric_points();
        let before = positions.clone();
        assert_eq!(
            radial_cup_vertices(&mut positions, vertices, &bad),
            Err(expected)
        );
        assert_eq!(positions, before, "エラー時は1点も動かさない");
    };

    let mut bad = settings(0.5);
    bad.inner_radius = -0.1;
    check(&[0], bad, RadialCupError::InvalidRadii);
    bad = settings(0.5);
    bad.outer_radius = bad.inner_radius;
    check(&[0], bad, RadialCupError::InvalidRadii);
    bad = settings(0.5);
    bad.normal = [0.0; 3];
    check(&[0], bad, RadialCupError::DegenerateNormal);
    bad = settings(f64::NAN);
    check(&[0], bad, RadialCupError::NonFiniteSettings);
    check(
        &[99],
        settings(0.5),
        RadialCupError::VertexOutOfBounds {
            vertex: 99,
            vertex_count: symmetric_points().len(),
        },
    );

    let mut positions = symmetric_points();
    positions[5][0] = f64::INFINITY;
    let before = positions.clone();
    assert_eq!(
        radial_cup_vertices(&mut positions, &[0, 5], &settings(0.5)),
        Err(RadialCupError::NonFiniteVertex { vertex: 5 })
    );
    assert_eq!(positions, before);
}

#[test]
fn zero_height_is_a_bitwise_passthrough() {
    let mut positions = symmetric_points();
    let before = positions.clone();
    let report = radial_cup_vertices(&mut positions, &[0, 5, 9], &settings(0.0)).unwrap();
    assert_eq!(report.selected_vertices, 3);
    assert_eq!(report.moved_vertices, 0);
    assert_eq!(report.max_displacement, 0.0);
    assert_eq!(positions, before);
}

#[test]
fn source_file_can_be_used_as_a_standalone_module() {
    let mut positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
    let report = standalone_cup::radial_cup_vertices(
        &mut positions,
        &[0, 1],
        &standalone_cup::RadialCupSettings {
            center: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            inner_radius: 0.2,
            outer_radius: 1.0,
            height: 0.5,
        },
    )
    .unwrap();
    assert_eq!(report.selected_vertices, 2);
    assert_eq!(report.moved_vertices, 1);
    assert_eq!(positions[0], [0.0, 0.0, 0.5]);
    assert_eq!(positions[1], [1.0, 0.0, 0.0]);
}
