//! たわみ計算の性能(NFR-002b: 面400の規模で1フレーム16ms以内)。
//!
//! 20×20のミウラ折り(面400・頂点441・辺840)を剛体折りで少し折った姿勢を
//! 基準にして、`relax` 1回(反復20回)の時間を測る。層の貫通防止の近傍探索が
//! 効く場合(層16枚)と効かない場合(層1枚)の両方を測る。
//!
//! # 実測(2026-08-06, 開発機 Windows 11 / release)
//!
//! | 網の細かさ | 三角形 | 層1枚 | 層16枚 |
//! |---|---|---|---|
//! | 細分2(自動調整なし・参考値) | 12,800 | 約12ms | 約21ms |
//! | 細分1(自動調整後=既定の動き) | 3,200 | 約3.1ms | 約6.3ms |
//!
//! 中間フレームの接触専用経路は、同じ開発機・release・面400・細分なし・4反復で
//! **7.52ms/フレーム**(2026-08-08実測)。16msの1フレーム枠内に収まる。
//!
//! 面400・細分2は16msに収まらないため、`ori3-soft` は三角形数の上限
//! (`MAX_TRIANGLES`)で細分を自動的に1へ落とす(NFR-002bの「大きな展開図では
//! 分割の細かさを自動で落として目標を保つ」)。このテストが測るのはその
//! 既定の動きで、目標16msに対しておよそ3倍の余裕がある。
//!
//! debugビルドは層16枚で約28ms(release比およそ5倍)。通常のcargo testでは
//! その数倍を上限にした緩い枠で「大きな性能後退」だけを検出する。release実測は
//!   cargo test --release -p ori3-soft --test perf_soft -- --nocapture
//! で確認する。

use std::time::{Duration, Instant};

use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Driver, Edge, EdgeKind, Vertex};
use ori3_rigid::solve;
use ori3_soft::{OverlapSettings, SoftSettings, prevent_overlap, relax};

/// debugビルドでの`relax` 1回あたりの上限(モジュール冒頭の計測記録を参照)。
const DEBUG_BUDGET: Duration = Duration::from_millis(200);
/// 接触専用経路のdebug実測(約129ms)に対する、CIばらつき込みの退行検知枠。
const OVERLAP_DEBUG_BUDGET: Duration = Duration::from_millis(300);

/// nc×nr面のミウラ折りCP(`ori3-rigid/tests/perf_miura.rs` と同じ作り方)。
fn miura_cp(nc: usize, nr: usize) -> CreasePattern {
    let s = 0.35;
    let (dx, dy) = (1.0 / nc as f64, 1.0 / (nr as f64 + s));
    let vid = |i: usize, j: usize| (j * (nc + 1) + i) as u32;
    let mut vertices = Vec::new();
    for j in 0..=nr {
        for i in 0..=nc {
            let y = (j as f64 + if i % 2 == 1 { s } else { 0.0 }) * dy;
            vertices.push(Vertex {
                id: vid(i, j),
                pos: [i as f64 * dx, y],
            });
        }
    }
    let mut edges: Vec<Edge> = Vec::new();
    for j in 0..nr {
        for i in 0..=nc {
            let kind = match () {
                () if i == 0 || i == nc => EdgeKind::Border,
                () if (i + j) % 2 == 0 => EdgeKind::Mountain,
                () => EdgeKind::Valley,
            };
            edges.push(Edge {
                id: edges.len() as u32,
                v0: vid(i, j),
                v1: vid(i, j + 1),
                kind,
            });
        }
    }
    for j in 0..=nr {
        for i in 0..nc {
            let kind = match () {
                () if j == 0 || j == nr => EdgeKind::Border,
                () if j % 2 == 1 => EdgeKind::Mountain,
                () => EdgeKind::Valley,
            };
            edges.push(Edge {
                id: edges.len() as u32,
                v0: vid(i, j),
                v1: vid(i + 1, j),
                kind,
            });
        }
    }
    let (nv, ne) = (vertices.len() as u32, edges.len() as u32);
    CreasePattern {
        vertices,
        edges,
        next_vertex_id: nv,
        next_edge_id: ne,
    }
}

#[test]
fn relax_of_400_faces_fits_in_one_frame() {
    let cp = miura_cp(20, 20);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 400, "面400の規模");
    // 少しだけ折った姿勢を基準の形にする(平らのままだと網も平面になる)
    let drivers = vec![Driver {
        hinge: 21,
        target_angle_deg: 20.0,
    }];
    let frame = solve(&cp, &faces, &drivers, None).frame;

    let settings = SoftSettings {
        enabled: true,
        subdivision: 2,
        stiffness: 0.5,
        pressure: 0.5,
        iterations: 20,
    };
    let warm = relax(&cp, &faces, &frame, &settings);
    assert!(
        warm.warnings.iter().any(|w| w.contains("自動で落とし")),
        "面400・細分2は上限を超えるので自動で落ちる: {:?}",
        warm.warnings
    );
    assert_eq!(warm.triangles.len(), 3_200, "細分1に落ちた網");

    let mut best = Duration::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        let mesh = relax(&cp, &faces, &frame, &settings);
        best = best.min(t.elapsed());
        assert_eq!(mesh.triangles.len(), warm.triangles.len());
    }
    // 層が何枚もある場合(貫通防止の近傍探索が働く)。剛体折りは全面 layer=0 を
    // 返すので、ここでは層番号を振り直して探索経路を通す。
    let mut layered = frame.clone();
    for (i, f) in layered.faces.iter_mut().enumerate() {
        f.layer = (i % 16) as u32;
    }
    let mut best_layered = Duration::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        let mesh = relax(&cp, &faces, &layered, &settings);
        best_layered = best_layered.min(t.elapsed());
        assert_eq!(mesh.triangles.len(), warm.triangles.len());
    }
    println!(
        "relax: 面{} 三角形{} 頂点{} → 1層 {best:?}/回、16層 {best_layered:?}/回",
        faces.len(),
        warm.triangles.len(),
        warm.positions.len()
    );
    assert!(
        best <= DEBUG_BUDGET,
        "1回あたり {best:?}(上限 {DEBUG_BUDGET:?})"
    );
    assert!(
        best_layered <= DEBUG_BUDGET,
        "16層で1回あたり {best_layered:?}(上限 {DEBUG_BUDGET:?})"
    );
}

#[test]
fn overlap_correction_of_400_faces_fits_in_one_frame() {
    let cp = miura_cp(20, 20);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 400, "面400の規模");
    let frame = solve(
        &cp,
        &faces,
        &[Driver {
            hinge: 21,
            target_angle_deg: 20.0,
        }],
        None,
    )
    .frame;
    let mut order: Vec<u32> = faces.iter().map(|face| face.id).collect();
    order.sort_unstable();
    let settings = OverlapSettings::default();

    let mut warm = frame.clone();
    let report = prevent_overlap(&cp, &faces, &mut warm, &order, &order, 0.5, &settings);
    assert!(report.applied);
    let mut best = Duration::MAX;
    for _ in 0..5 {
        let mut corrected = frame.clone();
        let t = Instant::now();
        let report = prevent_overlap(&cp, &faces, &mut corrected, &order, &order, 0.5, &settings);
        best = best.min(t.elapsed());
        assert!(report.applied);
    }
    println!(
        "prevent_overlap: 面{}・反復{} → {best:?}/フレーム",
        faces.len(),
        settings.iterations
    );
    let budget = if cfg!(debug_assertions) {
        OVERLAP_DEBUG_BUDGET
    } else {
        Duration::from_millis(16)
    };
    assert!(best <= budget, "1フレーム {best:?}(上限 {budget:?})");
}
