//! 重なり順の証拠にしてよい「経路」の条件を固定する。
//!
//! `surface_order_from_angles` は、呼出し元が渡した現在手順の追従経路の深度差から
//! 完全に重なる面対の上下を決める。この読み方が成り立つのは、渡された並びが
//! **終点へ向かう途中の姿勢**であるときだけである。同じ姿勢を繰り返しているだけの
//! 並びを渡されても、その高さの差を終点の上下の証拠にしてはならない。

use std::collections::HashMap;

use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeId, EdgeKind, FaceId, Frame3D, Vertex};
use ori3_rigid::{solve, surface_order_from_angles};

fn vertex(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

/// 利用者の実機と同じ、16面・非木辺つきの折り目。完全に重なる面対を持つ。
fn live_frame_square() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, 1.0, 0.0),
            vertex(2, 1.0, 1.0),
            vertex(3, 0.0, 1.0),
            vertex(4, 1.0, 0.5),
            vertex(5, 0.0, 0.5),
            vertex(6, 0.5, 1.0),
            vertex(7, 0.5, 0.0),
            vertex(8, 0.5, 0.5),
            vertex(9, 0.5, 0.792_893_218_813_452_5),
            vertex(10, 0.792_893_218_813_452_5, 0.5),
            vertex(11, 0.5, 0.207_106_781_186_547_52),
            vertex(12, 0.207_106_781_186_547_52, 0.5),
        ],
        edges: vec![
            edge(4, 1, 4, EdgeKind::Border),
            edge(5, 4, 2, EdgeKind::Border),
            edge(6, 3, 5, EdgeKind::Border),
            edge(7, 5, 0, EdgeKind::Border),
            edge(9, 2, 6, EdgeKind::Border),
            edge(10, 6, 3, EdgeKind::Border),
            edge(11, 0, 7, EdgeKind::Border),
            edge(12, 7, 1, EdgeKind::Border),
            edge(17, 0, 8, EdgeKind::Valley),
            edge(18, 8, 2, EdgeKind::Valley),
            edge(19, 6, 9, EdgeKind::Mountain),
            edge(20, 9, 8, EdgeKind::Mountain),
            edge(21, 2, 9, EdgeKind::Mountain),
            edge(22, 4, 10, EdgeKind::Mountain),
            edge(23, 10, 8, EdgeKind::Mountain),
            edge(24, 2, 10, EdgeKind::Mountain),
            edge(25, 10, 1, EdgeKind::Mountain),
            edge(26, 8, 11, EdgeKind::Mountain),
            edge(27, 11, 7, EdgeKind::Mountain),
            edge(28, 0, 11, EdgeKind::Mountain),
            edge(29, 11, 1, EdgeKind::Mountain),
            edge(30, 8, 12, EdgeKind::Mountain),
            edge(31, 12, 5, EdgeKind::Mountain),
            edge(32, 0, 12, EdgeKind::Mountain),
            edge(33, 12, 3, EdgeKind::Mountain),
            edge(34, 3, 9, EdgeKind::Mountain),
            edge(35, 12, 9, EdgeKind::Valley),
            edge(36, 10, 11, EdgeKind::Valley),
        ],
        next_vertex_id: 13,
        next_edge_id: 37,
    }
}

/// 実機が表示していた20本の折り角。
fn live_frame_angles() -> HashMap<EdgeId, f64> {
    HashMap::from([
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
    ])
}

/// 全ての折り目を同じ割合まで送った姿勢を、実際に解いて返す。
fn pose_at(
    cp: &CreasePattern,
    faces: &[ori3_cp::Face],
    angles: &HashMap<EdgeId, f64>,
    share: f64,
) -> Frame3D {
    let mut drivers = angles
        .iter()
        .map(|(&hinge, &angle)| Driver {
            hinge,
            target_angle_deg: angle * share,
        })
        .collect::<Vec<Driver>>();
    drivers.sort_unstable_by_key(|driver| driver.hinge);
    solve(cp, faces, &drivers, None).frame
}

fn order_of(
    cp: &CreasePattern,
    faces: &[ori3_cp::Face],
    angles: &HashMap<EdgeId, f64>,
    path: &[Frame3D],
) -> Result<Vec<FaceId>, String> {
    surface_order_from_angles(cp, faces, angles, path, None).map(|(order, _)| order)
}

/// 1つの姿勢を繰り返すだけの経路は、終点の重なり順の証拠にならない。
///
/// **この検査が捕まえた不具合(2026-08-22)**: `folded-sample.ori3` の手順7で、
/// 手順の再生が渡す3姿勢の探り経路が**すべて同じ姿勢**で、終点から 0.5756
/// (紙の長辺=1.0)離れたまま1度も動いていなかった。それでも高さの差が採用され、
/// 完全重なり4組 (24,29)・(25,28)・(27,33)・(39,43) の上下が、実際に表示する動きと
/// 逆に決まっていた。手順8は角度を1本も変えないので、その誤りが最終形へ残っていた。
///
/// ここでは同じ形を最小の材料で作る。**同じ姿勢を3つ並べた経路**を渡し、
/// 結果が**経路を渡さなかったときと1文字も変わらない**ことを確かめる。
/// つまり、動かない経路は1本も制約を足していない。
#[test]
fn a_path_that_repeats_one_pose_away_from_the_endpoint_adds_no_evidence() {
    let cp = live_frame_square();
    let faces = extract_faces(&cp);
    let angles = live_frame_angles();

    let stalled_pose = pose_at(&cp, &faces, &angles, 0.5);
    let stalled_path = vec![
        stalled_pose.clone(),
        stalled_pose.clone(),
        stalled_pose.clone(),
    ];

    let without_path = order_of(&cp, &faces, &angles, &[]);
    let with_stalled_path = order_of(&cp, &faces, &angles, &stalled_path);
    assert_eq!(
        format!("{without_path:?}"),
        format!("{with_stalled_path:?}"),
        "動かない経路が重なり順を変えている"
    );
}

/// 終点との離れが変わる経路は、これまでどおり証拠として使う。
///
/// 上の検査だけだと「経路をいつも捨てる」実装でも通ってしまう。近づく経路が
/// **実際に効いている**(経路なしでは決まらない上下を決めている)ことを固定する。
#[test]
fn a_path_that_approaches_the_endpoint_is_still_used_as_evidence() {
    let cp = live_frame_square();
    let faces = extract_faces(&cp);
    let angles = live_frame_angles();

    let approaching_path = [0.5_f64, 0.9, 0.99]
        .into_iter()
        .map(|share| pose_at(&cp, &faces, &angles, share))
        .collect::<Vec<_>>();

    let without_path = order_of(&cp, &faces, &angles, &[]);
    let with_approaching_path = order_of(&cp, &faces, &angles, &approaching_path);
    assert!(
        without_path.is_err(),
        "この折り目は経路なしでは重なり順が決まらない前提が崩れた: {without_path:?}"
    );
    assert!(
        with_approaching_path.is_ok(),
        "終点へ近づく経路を渡しても重なり順が決まらない: {with_approaching_path:?}"
    );
}
