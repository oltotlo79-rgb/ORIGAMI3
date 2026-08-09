//! Task 1-8: ループ閉包ソルバーのテスト。
//!
//! テストCP: 正方形の中心に折り線4本が集まる次数4の内部頂点1個
//! (扇形の角度 50°/60°/130°/120°、川崎の定理を満たす平坦折り可能な配置)。
//! 面隣接グラフが4面のループになるため、非木辺1本の閉包拘束が生じる。

use std::collections::HashMap;

use glam::DVec3;
use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeKind, Vertex};
use ori3_rigid::{solve, solve_near};

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

/// 次数4の内部頂点1個のCP。中心(0.5,0.5)から方位角0°/50°/110°/240°へ
/// 折り線が伸びる(扇形は50°,60°,130°,120°で交互和がともに180°=川崎の定理)。
/// 山谷は前川の定理(山-谷=±2)を満たす M,V,M,M(最小扇形50°を挟む2本が逆)。
/// 折り線の辺ID: 8=方位角0°(山) 9=50°(谷) 10=110°(山) 11=240°(山)。
fn degree4_cp() -> CreasePattern {
    let p1x = 0.5 + 0.5 * 50f64.to_radians().cos() / 50f64.to_radians().sin();
    let p2x = 0.5 + 0.5 * 110f64.to_radians().cos() / 110f64.to_radians().sin();
    let p3x = 0.5 + 0.5 / 240f64.to_radians().sin().abs() * 240f64.to_radians().cos();
    CreasePattern {
        vertices: vec![
            v(0, 0.0, 0.0),
            v(1, p3x, 0.0),
            v(2, 1.0, 0.0),
            v(3, 1.0, 0.5),
            v(4, 1.0, 1.0),
            v(5, p1x, 1.0),
            v(6, p2x, 1.0),
            v(7, 0.0, 1.0),
            v(8, 0.5, 0.5),
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
            e(8, 8, 3, EdgeKind::Mountain),
            e(9, 8, 5, EdgeKind::Valley),
            e(10, 8, 6, EdgeKind::Mountain),
            e(11, 8, 1, EdgeKind::Mountain),
        ],
        next_vertex_id: 9,
        next_edge_id: 12,
    }
}

/// 折られた形の物理的な整合性: 2面が共有する各折り線について、両側の面から
/// 計算した端点の3D位置が一致することを確認する(ループ閉包の実体検証)。
fn assert_edges_closed(cp: &CreasePattern, frame: &ori3_model::Frame3D, tol: f64) {
    let faces = extract_faces(cp);
    // 頂点ID→(面ごとの3D位置)を集める
    let mut positions: HashMap<u32, Vec<DVec3>> = HashMap::new();
    for f3d in &frame.faces {
        let face = &faces[f3d.face as usize];
        for (vid, p) in face.vertices.iter().zip(&f3d.polygon) {
            positions.entry(*vid).or_default().push(DVec3::from(*p));
        }
    }
    for (vid, list) in positions {
        for p in &list {
            assert!(
                (*p - list[0]).length() < tol,
                "頂点{vid}の3D位置が面間で不一致: {list:?}"
            );
        }
    }
}

/// Newellの方法による面法線。
fn normal_of(poly: &[[f64; 3]]) -> DVec3 {
    let mut n = DVec3::ZERO;
    for i in 0..poly.len() {
        let p = DVec3::from(poly[i]);
        let q = DVec3::from(poly[(i + 1) % poly.len()]);
        n += p.cross(q);
    }
    n.normalize()
}

#[test]
fn driver_90_satisfies_loop_closure() {
    let cp = degree4_cp();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 4);

    let result = solve(&cp, &faces, &[d(8, 90.0)], None);
    assert!(result.converged, "angles={:?}", result.angles);
    // driver角は指定どおり
    assert!((result.angles[&8] - 90.0).abs() < 1e-6);
    // 残り3ヒンジも動いている(平らなままではループが閉じない)
    for id in [9u32, 10, 11] {
        assert!(
            result.angles[&id].abs() > 1.0,
            "ヒンジ{id}が動いていない: {:?}",
            result.angles
        );
    }
    // 全折り線の両側で端点の3D位置が一致(閉包条件の実体、許容1e-6)
    assert_edges_closed(&cp, &result.frame, 1e-6);

    // driverヒンジの二面角が90°(隣接2面の法線の内積が0)
    let adj: Vec<&ori3_model::Face3D> = frame_faces_adjacent_to(&cp, &result.frame, 8);
    assert_eq!(adj.len(), 2);
    let dot = normal_of(&adj[0].polygon).dot(normal_of(&adj[1].polygon));
    assert!(dot.abs() < 1e-6, "法線の内積={dot}");
}

/// 辺IDを境界に含む面のFrame3D要素を返す。
fn frame_faces_adjacent_to<'a>(
    cp: &CreasePattern,
    frame: &'a ori3_model::Frame3D,
    edge: u32,
) -> Vec<&'a ori3_model::Face3D> {
    let faces = extract_faces(cp);
    frame
        .faces
        .iter()
        .filter(|f3d| faces[f3d.face as usize].edges.contains(&edge))
        .collect()
}

#[test]
fn driver_180_reaches_flat_folded_state() {
    let cp = degree4_cp();
    let faces = extract_faces(&cp);

    for sign in [1.0f64, -1.0] {
        let result = solve(&cp, &faces, &[d(8, sign * 180.0)], None);
        assert!(result.converged, "sign={sign} angles={:?}", result.angles);
        // 全ヒンジが±180°に達する
        for id in [8u32, 9, 10, 11] {
            assert!(
                (result.angles[&id].abs() - 180.0).abs() < 1e-3,
                "sign={sign} ヒンジ{id}: {:?}",
                result.angles
            );
        }
        // 平坦: 全頂点のzが0
        for f3d in &result.frame.faces {
            for p in &f3d.polygon {
                assert!(p[2].abs() < 1e-6, "sign={sign} z={}", p[2]);
            }
        }
        assert_edges_closed(&cp, &result.frame, 1e-6);
    }
}

#[test]
fn contradictory_drivers_return_best_effort_without_panic() {
    let cp = degree4_cp();
    let faces = extract_faces(&cp);

    // 辺8を完全に折ると全ヒンジが±180°になるしかないため、辺9=-90°とは両立しない
    let result = solve(&cp, &faces, &[d(8, 180.0), d(9, -90.0)], None);
    assert!(!result.converged);
    assert!(
        result
            .frame
            .warnings
            .iter()
            .any(|w| w.contains("収束していません")),
        "warnings={:?}",
        result.frame.warnings
    );
    // 直前の最良解でフレームは返る(全面が出力される)
    assert_eq!(result.frame.faces.len(), 4);
    // driver角は固定されたまま
    assert!((result.angles[&8] - 180.0).abs() < 1e-9);
    assert!((result.angles[&9] + 90.0).abs() < 1e-9);
}

#[test]
fn warm_start_converges_faster_to_same_solution() {
    let cp = degree4_cp();
    let faces = extract_faces(&cp);

    let cold = solve(&cp, &faces, &[d(8, 90.0)], None);
    assert!(cold.converged);
    assert!(cold.iterations > 0, "冷間開始は反復が必要なはず");

    let warm = solve(&cp, &faces, &[d(8, 90.0)], Some(&cold.angles));
    assert!(warm.converged);
    // 同じ解に、より少ない反復で収束する
    assert!(
        warm.iterations < cold.iterations,
        "warm={} cold={}",
        warm.iterations,
        cold.iterations
    );
    for id in [8u32, 9, 10, 11] {
        assert!(
            (warm.angles[&id] - cold.angles[&id]).abs() < 1e-6,
            "ヒンジ{id}: warm={} cold={}",
            warm.angles[&id],
            cold.angles[&id]
        );
    }
}

/// ループのないCP(正方形+中央縦1本、辺ID6が折り線)。
fn loop_free_cp() -> CreasePattern {
    CreasePattern {
        vertices: vec![
            v(0, 0.0, 0.0),
            v(1, 0.5, 0.0),
            v(2, 1.0, 0.0),
            v(3, 1.0, 1.0),
            v(4, 0.5, 1.0),
            v(5, 0.0, 1.0),
        ],
        edges: vec![
            e(0, 0, 1, EdgeKind::Border),
            e(1, 1, 2, EdgeKind::Border),
            e(2, 2, 3, EdgeKind::Border),
            e(3, 3, 4, EdgeKind::Border),
            e(4, 4, 5, EdgeKind::Border),
            e(5, 5, 0, EdgeKind::Border),
            e(6, 1, 4, EdgeKind::Mountain),
        ],
        next_vertex_id: 6,
        next_edge_id: 7,
    }
}

#[test]
fn loop_free_cp_converges_immediately() {
    // ループのないCPはそのまま伝播して即収束する
    let cp = loop_free_cp();
    let faces = extract_faces(&cp);
    let result = solve(&cp, &faces, &[d(6, 90.0)], None);
    assert!(result.converged);
    assert_eq!(result.iterations, 0);
    assert!((result.angles[&6] - 90.0).abs() < 1e-9);
    assert_edges_closed(&cp, &result.frame, 1e-9);
}

#[test]
fn free_hinge_without_driver_clamps_out_of_range_warm_start() {
    // driverを外した自由ヒンジにも物理限界は適用する。古いwarm startに
    // 範囲外の値が残っていても、同値角へ通り抜けず境界で止まる。
    let cp = loop_free_cp();
    let faces = extract_faces(&cp);
    let warm = HashMap::from([(6u32, 350.0f64)]);
    let result = solve(&cp, &faces, &[], Some(&warm));
    assert!(result.converged);
    assert_eq!(result.iterations, 0);
    assert_eq!(result.angles[&6], 180.0, "angles={:?}", result.angles);
}

#[test]
fn solve_near_dependent_target_stops_exactly_at_both_limits() {
    // 閉路のない自由ヒンジはsolve_nearの目標へそのまま動くため、射影境界を
    // 厳密に検査できる。±180°を越えた目標でも返り値は境界ちょうどになる。
    let cp = loop_free_cp();
    let faces = extract_faces(&cp);
    for (target, expected) in [(240.0, 180.0), (-240.0, -180.0)] {
        let targets = HashMap::from([(6u32, target)]);
        let result = solve_near(&cp, &faces, &[], &targets, None);
        assert!(result.converged);
        assert_eq!(result.angles[&6], expected, "target={target}");
    }
}

#[test]
fn all_dependent_angles_stay_within_physical_range() {
    let cp = degree4_cp();
    let faces = extract_faces(&cp);
    let warm = HashMap::from([(9u32, 540.0), (10u32, -540.0), (11u32, 270.0)]);
    let result = solve(&cp, &faces, &[d(8, 90.0)], Some(&warm));
    for id in [9u32, 10, 11] {
        assert!(
            (-180.0..=180.0).contains(&result.angles[&id]),
            "ヒンジ{id}: {:?}",
            result.angles
        );
    }
}

#[test]
fn pendant_hinge_off_loop_stays_flat() {
    // ループを含むCPに、ループへ関与しないペンダント折り線(左下隅を切る1本、
    // 辺ID16)を足す。driverで他を折っても、この辺には閉包拘束がない=解の途中で
    // 動く理由がないため、正確に0度のまま残らなければならない
    // (回帰テスト: かつて初期値バイアスが成分単位で掛かり、触っていない辺が
    // 45度のままconverged:true・警告なしで出力されるバグがあった)。
    let mut cp = degree4_cp();
    cp.vertices.push(v(9, 0.0, 0.3));
    cp.vertices.push(v(10, 0.15, 0.0));
    // 左辺 e7(v7→v0) を v9 で、下辺 e0(v0→v1) を v10 で分割し、v9-v10 を折り線に
    cp.edges.retain(|edge| edge.id != 0 && edge.id != 7);
    cp.edges.push(e(12, 7, 9, EdgeKind::Border));
    cp.edges.push(e(13, 9, 0, EdgeKind::Border));
    cp.edges.push(e(14, 0, 10, EdgeKind::Border));
    cp.edges.push(e(15, 10, 1, EdgeKind::Border));
    cp.edges.push(e(16, 9, 10, EdgeKind::Mountain));
    cp.next_vertex_id = 11;
    cp.next_edge_id = 17;

    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 5, "4扇形のうち1つが隅の三角形と分かれて5面");

    let result = solve(&cp, &faces, &[d(8, 90.0)], None);
    assert!(result.converged, "angles={:?}", result.angles);
    assert_eq!(
        result.angles[&16], 0.0,
        "触っていないペンダント辺が動いた: {:?}",
        result.angles
    );
    // ループ側は従来どおり解けている
    for id in [9u32, 10, 11] {
        assert!(result.angles[&id].abs() > 1.0, "angles={:?}", result.angles);
    }
    assert_edges_closed(&cp, &result.frame, 1e-6);
}

#[test]
fn driver_on_non_hinge_edge_is_ignored_with_warning() {
    let cp = degree4_cp();
    let faces = extract_faces(&cp);
    // 辺0は輪郭(1面のみ)、辺99は存在しない → どちらも無視して警告
    let result = solve(&cp, &faces, &[d(0, 90.0), d(99, 45.0)], None);
    assert!(result.converged);
    assert!(
        result.frame.warnings.iter().any(|w| w.contains("無視")),
        "warnings={:?}",
        result.frame.warnings
    );
    assert!(!result.angles.contains_key(&0));
    assert!(!result.angles.contains_key(&99));
}
