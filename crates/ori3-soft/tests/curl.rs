//! 折り目を付けず、花びらなどの端だけを外へカールさせる後処理のテスト。

use ori3_soft::{CurlError, CurlSettings, SoftMesh, curl_vertices};

// 統合受入検査も依存追加なしで同じ実装を読む。この形で常にコンパイル
// できることを、通常の公開APIを使うテストとは別に確認する。
#[path = "../src/curl.rs"]
mod standalone_curl;

fn strip() -> SoftMesh {
    // y軸上が動かない付け根。x方向へ、90度の円弧長π/2と、その先0.5まで伸びる。
    let tip = std::f64::consts::FRAC_PI_2;
    SoftMesh {
        positions: vec![
            [0.0, -0.5, 0.0],
            [0.0, 0.5, 0.0],
            [tip * 0.5, -0.5, 0.0],
            [tip * 0.5, 0.5, 0.0],
            [tip, -0.5, 0.0],
            [tip, 0.5, 0.0],
            [tip + 0.5, -0.5, 0.0],
            [tip + 0.5, 0.5, 0.0],
            // 同じ網だが、選択しない離れた紙片。
            [4.0, 4.0, 4.0],
        ],
        triangles: vec![
            [0, 2, 1],
            [1, 2, 3],
            [2, 4, 3],
            [3, 4, 5],
            [4, 6, 5],
            [5, 6, 7],
        ],
        triangle_faces: vec![10, 10, 10, 10, 10, 10],
        triangle_layers: vec![3, 3, 3, 3, 3, 3],
        warnings: vec!["既存の警告".to_string()],
    }
}

fn settings(angle_deg: f64) -> CurlSettings {
    CurlSettings {
        axis_origin: [0.0, 0.0, 0.0],
        axis_direction: [0.0, 2.0, 0.0],
        toward_tip: [3.0, 0.2, 0.0], // 軸方向の混入は内部で除かれる。
        radius: 1.0,
        angle_deg,
    }
}

fn close(actual: [f64; 3], expected: [f64; 3]) {
    for k in 0..3 {
        assert!(
            (actual[k] - expected[k]).abs() < 1e-12,
            "座標{k}: {} != {} ({actual:?})",
            actual[k],
            expected[k]
        );
    }
}

#[test]
fn circular_curl_keeps_the_axis_connected_and_continues_at_the_tangent() {
    let mut mesh = strip();
    let topology = mesh.triangles.clone();
    let faces = mesh.triangle_faces.clone();
    let layers = mesh.triangle_layers.clone();
    let warnings = mesh.warnings.clone();
    let report = curl_vertices(
        &mut mesh.positions,
        &[0, 1, 2, 3, 4, 5, 6, 7],
        &settings(90.0),
    )
    .expect("有効なカール");

    assert_eq!(report.selected_vertices, 8);
    assert_eq!(report.moved_vertices, 6, "軸上の2点だけは動かない");
    assert!(report.max_displacement.is_finite());
    assert_eq!(mesh.positions[0], [0.0, -0.5, 0.0], "付け根を厳密に固定");
    assert_eq!(mesh.positions[1], [0.0, 0.5, 0.0], "共有境界を厳密に固定");
    close(mesh.positions[4], [1.0, -0.5, -1.0]);
    close(mesh.positions[5], [1.0, 0.5, -1.0]);
    close(mesh.positions[6], [1.0, -0.5, -1.5]);
    close(mesh.positions[7], [1.0, 0.5, -1.5]);
    assert_eq!(mesh.positions[8], [4.0, 4.0, 4.0], "非対象頂点は不変");
    assert_eq!(mesh.triangles, topology);
    assert_eq!(mesh.triangle_faces, faces);
    assert_eq!(mesh.triangle_layers, layers);
    assert_eq!(mesh.warnings, warnings);
    assert!(
        mesh.positions
            .iter()
            .flatten()
            .all(|value| value.is_finite())
    );
}

#[test]
fn selection_order_and_duplicates_do_not_change_the_result() {
    let mut ordered = strip();
    let mut shuffled = strip();
    let one = curl_vertices(
        &mut ordered.positions,
        &[0, 1, 2, 3, 4, 5],
        &settings(-75.0),
    )
    .unwrap();
    let two = curl_vertices(
        &mut shuffled.positions,
        &[5, 3, 1, 5, 0, 4, 2, 3],
        &settings(-75.0),
    )
    .unwrap();
    assert_eq!(ordered.positions, shuffled.positions, "頂点番号順で決定的");
    assert_eq!(one, two, "重複を1回だけ数える");

    let once = ordered.clone();
    let mut repeat = strip();
    curl_vertices(&mut repeat.positions, &[0, 1, 2, 3, 4, 5], &settings(-75.0)).unwrap();
    assert_eq!(once.positions, repeat.positions, "同じ入力から同じビット列");
}

#[test]
fn only_selected_vertices_ahead_of_the_axis_move() {
    let mut mesh = strip();
    mesh.positions.push([-0.25, 0.0, 0.0]);
    let before = mesh.positions.clone();
    let report = curl_vertices(&mut mesh.positions, &[0, 2, 4, 9], &settings(45.0)).unwrap();
    assert_eq!(report.selected_vertices, 4);
    assert_eq!(report.moved_vertices, 2);
    assert_eq!(mesh.positions[0], before[0], "軸上の選択頂点は固定");
    assert_eq!(mesh.positions[9], before[9], "付け根側の選択頂点は固定");
    assert_ne!(mesh.positions[2], before[2]);
    assert_ne!(mesh.positions[4], before[4]);
    for &vertex in &[1, 3, 5, 6, 7, 8] {
        assert_eq!(mesh.positions[vertex], before[vertex], "非対象頂点{vertex}");
    }
}

#[test]
fn invalid_input_is_an_atomic_error() {
    let check = |vertices: &[u32], bad: CurlSettings, expected: CurlError| {
        let mut mesh = strip();
        let before = mesh.positions.clone();
        assert_eq!(
            curl_vertices(&mut mesh.positions, vertices, &bad),
            Err(expected)
        );
        assert_eq!(mesh.positions, before, "エラー時は1点も動かさない");
    };

    let mut bad = settings(45.0);
    bad.radius = 0.0;
    check(&[2], bad, CurlError::InvalidRadius);
    bad = settings(45.0);
    bad.angle_deg = f64::NAN;
    check(&[2], bad, CurlError::NonFiniteSettings);
    bad = settings(45.0);
    bad.axis_direction = [0.0; 3];
    check(&[2], bad, CurlError::DegenerateAxis);
    bad = settings(45.0);
    bad.toward_tip = [0.0, 1.0, 0.0];
    check(&[2], bad, CurlError::DegenerateTipDirection);
    check(
        &[99],
        settings(45.0),
        CurlError::VertexOutOfBounds {
            vertex: 99,
            vertex_count: strip().positions.len(),
        },
    );

    let mut mesh = strip();
    mesh.positions[4][2] = f64::INFINITY;
    let before = mesh.positions.clone();
    assert_eq!(
        curl_vertices(&mut mesh.positions, &[2, 4], &settings(45.0)),
        Err(CurlError::NonFiniteVertex { vertex: 4 })
    );
    assert_eq!(mesh.positions, before);
}

#[test]
fn zero_angle_is_a_finite_bitwise_passthrough() {
    let mut mesh = strip();
    let before = mesh.positions.clone();
    let report = curl_vertices(&mut mesh.positions, &[0, 2, 4, 6], &settings(0.0)).unwrap();
    assert_eq!(report.selected_vertices, 4);
    assert_eq!(report.moved_vertices, 0);
    assert_eq!(report.max_displacement, 0.0);
    assert_eq!(mesh.positions, before);
}

#[test]
fn source_file_can_be_used_as_a_standalone_module() {
    let mut positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
    let report = standalone_curl::curl_vertices(
        &mut positions,
        &[0, 1],
        &standalone_curl::CurlSettings {
            axis_origin: [0.0; 3],
            axis_direction: [0.0, 1.0, 0.0],
            toward_tip: [1.0, 0.0, 0.0],
            radius: 1.0,
            angle_deg: 30.0,
        },
    )
    .unwrap();
    assert_eq!(report.selected_vertices, 2);
    assert_eq!(report.moved_vertices, 1);
    assert!(positions.iter().flatten().all(|value| value.is_finite()));
}
