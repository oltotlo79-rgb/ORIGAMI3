//! Task 1-10: M1受け入れテスト — やっこさん(座布団折り2回相当)。
//!
//! 展開図はUIの線描画操作列に相当する `insert_segment` の呼び出し列
//! (スナップなしの生座標)で構築し、全折り線を±180°でsolveすると
//! 全面が同一平面(|z|<1e-6)に畳まれることを検証する。
//!
//! # 展開図の設計メモ
//!
//! - 座布団折り1回目: 辺中点 M1(0.5,0)・M2(1,0.5)・M3(0.5,1)・M4(0,0.5) を結ぶ
//!   谷折り4本(表側から見て4隅が手前に上がる=規約では負角)。内部頂点なし。
//! - 座布団折り2回目: 折った状態では「M1..M4を隅とする正方形の辺中点
//!   N1(0.75,0.25)・N2(0.75,0.75)・N3(0.25,0.75)・N4(0.25,0.25) を結ぶ谷折り」。
//!   これを展開すると、基層(回転正方形の内側)では内側の正方形 N1..N4(谷)、
//!   1回目で裏返った隅の層では同じ直線の延長が紙の縁まで届く区間(裏返りで
//!   山谷が反転して山)になる。つまり x=0.25, x=0.75, y=0.25, y=0.75 の4直線が
//!   紙を貫く。内部頂点 N1..N4 は次数6で、扇形は 45°/45°/90°/45°/45°/90°
//!   (川崎の定理: 交互和 45+90+45 = 45+45+90 = 180°)、山2本・谷4本
//!   (前川の定理: |山−谷|=2)となり平坦折り可能。
//! - 「折った状態の折り線(内側の正方形)だけ」を重ねた素朴な展開図は、
//!   Nの扇形が 90°/45°/180°/45° となり川崎の定理を満たさず平坦折り不能。
//!   ソルバーがこれを converged=false として検出することも負のテストで確認する。
//!
//! M1のソルバーは層順序(重なり順)を管理しないため、ここでは「全ヒンジ±180°で
//! 各面がどの平面位置に来るか」を検証する。±180°の回転は符号に依らず同一の
//! 回転のため、位置の検証は山谷の符号付けに影響されない。

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces, insert_segment, validate};
use ori3_model::{CreasePattern, Document, Driver, EdgeKind, Frame3D, Paper, VertexId};
use ori3_rigid::{
    MotionContactOptions, max_seam_gap, self_intersection_pairs, solve,
    solve_motion_with_contact_options,
};

/// UIの1回の線描画操作: 始点・終点・線種。
type Stroke = ([f64; 2], [f64; 2], EdgeKind);

/// 正方形の紙で新規作成した直後のCP(単位正方形、輪郭4辺のみ)。
fn new_square_cp() -> CreasePattern {
    Document::new(Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    })
    .cp
}

/// UIの線描画操作列に相当する `insert_segment` の呼び出し列。
/// 既存頂点への吸着・既存辺との交点での自動分割はinsert_segmentが行う。
fn draw(cp: &mut CreasePattern, strokes: &[Stroke]) {
    for &(a, b, kind) in strokes {
        insert_segment(cp, a, b, kind);
    }
}

/// 座布団折り1回目: 辺中点を結ぶ谷折り4本(4隅→中心)。
fn blintz_strokes() -> Vec<Stroke> {
    let (m1, m2, m3, m4) = ([0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]);
    vec![
        (m1, m2, EdgeKind::Valley),
        (m2, m3, EdgeKind::Valley),
        (m3, m4, EdgeKind::Valley),
        (m4, m1, EdgeKind::Valley),
    ]
}

/// 座布団折り2回目に相当する折り線: 1/4位置の縦横4直線。
/// 基層を通る中央区間は谷、1回目で裏返った隅の層を通る外側区間は山。
fn second_blintz_strokes() -> Vec<Stroke> {
    let mut s = Vec::new();
    for t in [0.25, 0.75] {
        // 縦線 x=t
        s.push(([t, 0.0], [t, 0.25], EdgeKind::Mountain));
        s.push(([t, 0.25], [t, 0.75], EdgeKind::Valley));
        s.push(([t, 0.75], [t, 1.0], EdgeKind::Mountain));
        // 横線 y=t
        s.push(([0.0, t], [0.25, t], EdgeKind::Mountain));
        s.push(([0.25, t], [0.75, t], EdgeKind::Valley));
        s.push(([0.75, t], [1.0, t], EdgeKind::Mountain));
    }
    s
}

/// やっこさんの展開図(座布団折り2回相当)。
fn yakko_cp() -> CreasePattern {
    let mut cp = new_square_cp();
    draw(&mut cp, &blintz_strokes());
    draw(&mut cp, &second_blintz_strokes());
    cp
}

/// 全折り線に完全折りのdriverを与える(山=+180°、谷=−180°)。
fn full_fold_drivers(cp: &CreasePattern) -> Vec<Driver> {
    cp.edges
        .iter()
        .filter_map(|e| {
            let deg = match e.kind {
                EdgeKind::Mountain => 180.0,
                EdgeKind::Valley => -180.0,
                EdgeKind::Border | EdgeKind::Aux => return None,
            };
            Some(Driver {
                hinge: e.id,
                target_angle_deg: deg,
            })
        })
        .collect()
}

/// 代表ヒンジを指定系列で動かし、接触回避後もhard・紙の接続・有限性を守ることを検査する。
///
/// # 速さの上限をここで測らない理由
///
/// 以前はこの中で `solve_motion` と `self_intersection_pairs` の実時間を測り、
/// 330ms・500msの上限と比べていた。しかしこのテストは最適化なしのビルドでも走る
/// (`cargo test --workspace`)ため、上限との差が計算機の混み具合でそのまま合否に
/// 出てしまい、420回の試行で65回落ちた(全件が同じ実時間の行。折り角・裂け・
/// 自己交差など数値の主張は1件も落ちていない)。実測は330.97ms〜974.82msで、
/// 何もしていない計算機でも30回中5回落ちた。
/// 速さの上限は最適化ありで測るものとして `tests/perf_yakko.rs` へ移した。
/// 上限値は緩めていない。ここには数値の主張だけを残す。
fn assert_contact_free_sweep(
    cp: &CreasePattern,
    faces: &[Face],
    (hinge, final_angle_deg): (u32, f64),
    magnitudes: impl IntoIterator<Item = u32>,
    medium: &HashMap<u32, f64>,
    mut warm: HashMap<u32, f64>,
    direction: &str,
) {
    let sign = final_angle_deg.signum();
    assert_ne!(sign, 0.0, "代表ヒンジの完成角には向きが必要");
    for magnitude in magnitudes {
        let requested = sign * f64::from(magnitude);
        let motion = solve_motion_with_contact_options(
            cp,
            faces,
            &[Driver {
                hinge,
                target_angle_deg: requested,
            }],
            Some(medium),
            Some(&warm),
            MotionContactOptions {
                detect: true,
                prevent: true,
            },
        );
        let result = &motion.result;
        assert!(
            !motion.contact_stopped,
            "{direction} {magnitude}°: 接触で操作が停止した"
        );
        let hard_error = (result.angles[&hinge] - requested).abs();
        assert!(
            hard_error < 1e-9,
            "{direction} {magnitude}°: hard誤差={hard_error:e} angles={:?}",
            result.angles
        );
        assert!(
            result.closure_rms.is_finite()
                && result.angles.values().all(|angle| angle.is_finite())
                && result.relaxations.iter().all(|relaxation| {
                    relaxation.target_angle_deg.is_finite()
                        && relaxation.actual_angle_deg.is_finite()
                        && relaxation.delta_deg.is_finite()
                })
                && result.frame.faces.len() == faces.len()
                && result.frame.faces.iter().all(|face| {
                    face.polygon
                        .iter()
                        .flatten()
                        .all(|coordinate| coordinate.is_finite())
                }),
            "{direction} {magnitude}°: finiteな全面形ではない"
        );
        let gap = max_seam_gap(cp, faces, &result.frame);
        assert!(
            gap < 1e-6,
            "{direction} {magnitude}°: seam={gap:e} rms={:e}",
            result.closure_rms
        );
        let intersections = self_intersection_pairs(&result.frame);
        assert!(
            intersections.is_empty(),
            "{direction} {magnitude}° (要求{requested}°): 交差={intersections:?} relaxations={:?}",
            result.relaxations
        );
        warm = result.angles.clone();
    }
}

fn assert_yakko_hinge_20_round_trip(magnitudes: &[u32], label: &str) {
    const HINGE: u32 = 20;

    let cp = yakko_cp();
    let faces = extract_faces(&cp);
    let completed_drivers = full_fold_drivers(&cp);
    let final_angle = completed_drivers
        .iter()
        .find(|driver| driver.hinge == HINGE)
        .unwrap_or_else(|| panic!("代表ヒンジ#{HINGE}が展開図にない"))
        .target_angle_deg;

    let flat = solve(&cp, &faces, &[], None);
    assert!(
        flat.converged,
        "平坦開始形: warnings={:?}",
        flat.frame.warnings
    );
    assert!(self_intersection_pairs(&flat.frame).is_empty());
    let ascending_medium: HashMap<u32, f64> = completed_drivers
        .iter()
        .filter(|driver| driver.hinge != HINGE)
        .map(|driver| (driver.hinge, 0.0))
        .collect();
    assert_contact_free_sweep(
        &cp,
        &faces,
        (HINGE, final_angle),
        magnitudes.iter().copied(),
        &ascending_medium,
        flat.angles,
        &format!("{label} 0→180"),
    );

    let completed = solve(&cp, &faces, &completed_drivers, None);
    assert!(
        completed.converged,
        "完成開始形: warnings={:?}",
        completed.frame.warnings
    );
    assert!(self_intersection_pairs(&completed.frame).is_empty());
    let descending_medium: HashMap<u32, f64> = completed
        .angles
        .iter()
        .filter(|(hinge, _)| **hinge != HINGE)
        .map(|(&hinge, &angle)| (hinge, angle))
        .collect();
    assert_contact_free_sweep(
        &cp,
        &faces,
        (HINGE, final_angle),
        magnitudes.iter().rev().copied(),
        &descending_medium,
        completed.angles,
        &format!("{label} 180→0"),
    );
}

/// 種類ごとの辺の本数 (輪郭, 山, 谷)。
fn kind_counts(cp: &CreasePattern) -> (usize, usize, usize) {
    let count = |k: EdgeKind| cp.edges.iter().filter(|e| e.kind == k).count();
    (
        count(EdgeKind::Border),
        count(EdgeKind::Mountain),
        count(EdgeKind::Valley),
    )
}

/// 指定座標(誤差1e-9以内)にある頂点のID。
fn vid_at(cp: &CreasePattern, p: [f64; 2]) -> VertexId {
    cp.vertices
        .iter()
        .find(|v| (DVec2::from(v.pos) - DVec2::from(p)).length() < 1e-9)
        .unwrap_or_else(|| panic!("座標{p:?}に頂点がない"))
        .id
}

/// 頂点ID→折り後の3D位置。同じ頂点を共有する全面で位置が一致することも
/// 確認する(折り線の両側で紙が裂けていないことの実体検証)。
fn folded_positions(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
    tol: f64,
) -> HashMap<VertexId, DVec3> {
    let mut out: HashMap<VertexId, DVec3> = HashMap::new();
    for f3d in &frame.faces {
        let face = &faces[f3d.face as usize];
        for (vid, p) in face.vertices.iter().zip(&f3d.polygon) {
            let p = DVec3::from(*p);
            let q = *out.entry(*vid).or_insert(p);
            assert!(
                (p - q).length() < tol,
                "頂点{vid}の3D位置が面間で不一致: {p} vs {q}"
            );
        }
    }
    // 面抽出対象の全頂点を網羅している(CPの頂点は全て折り辺の端点)
    assert_eq!(out.len(), cp.vertices.len());
    out
}

/// 全頂点のz座標の最大絶対値(平坦性の検証用)。
fn max_abs_z(frame: &Frame3D) -> f64 {
    frame
        .faces
        .iter()
        .flat_map(|f| &f.polygon)
        .map(|p| p[2].abs())
        .fold(0.0, f64::max)
}

/// xy投影のバウンディングボックス (min, max)。
fn bbox_xy(frame: &Frame3D) -> ([f64; 2], [f64; 2]) {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for f3d in &frame.faces {
        for p in &f3d.polygon {
            for k in 0..2 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
    }
    (min, max)
}

fn signed_area_xy(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|index| {
            let a = points[index];
            let b = points[(index + 1) % points.len()];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
        * 0.5
}

/// CPを描画順に依らない正規形へ: 辺を(整列した端点座標の組, 種類)の
/// 集合として取り出して整列する。座標は1e-6格子に丸める。
fn canonical_edges(cp: &CreasePattern) -> Vec<([i64; 2], [i64; 2], u8)> {
    let pos = |id: VertexId| {
        let v = cp
            .vertices
            .iter()
            .find(|v| v.id == id)
            .unwrap_or_else(|| panic!("頂点ID {id} が存在しない"));
        [
            (v.pos[0] * 1e6).round() as i64,
            (v.pos[1] * 1e6).round() as i64,
        ]
    };
    let mut out: Vec<([i64; 2], [i64; 2], u8)> = cp
        .edges
        .iter()
        .map(|e| {
            let (a, b) = (pos(e.v0), pos(e.v1));
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            let k = match e.kind {
                EdgeKind::Border => 0u8,
                EdgeKind::Mountain => 1,
                EdgeKind::Valley => 2,
                EdgeKind::Aux => 3,
            };
            (a, b, k)
        })
        .collect();
    out.sort_unstable();
    out
}

/// 段階(a): 座布団折り1回。谷折り4本を−180°で畳むと、外形が1/√2角の
/// 回転正方形(バウンディングボックスは1×1)になり、4隅は中心の1点に重なる。
#[test]
fn blintz_once_folds_flat_to_rotated_square() {
    let mut cp = new_square_cp();
    draw(&mut cp, &blintz_strokes());

    // 構造: 頂点8(隅4+辺中点4)、辺12(輪郭8+谷4)、面5(中央+隅の三角形4)
    assert_eq!(cp.vertices.len(), 8);
    assert_eq!(kind_counts(&cp), (8, 0, 4));
    assert_eq!(validate(&cp), Vec::<String>::new());
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 5);

    let drivers = full_fold_drivers(&cp);
    assert_eq!(drivers.len(), 4);
    let result = solve(&cp, &faces, &drivers, None);
    assert!(result.converged, "angles={:?}", result.angles);
    // 内部頂点がなく面隣接グラフは木 → 反復なしで即収束する
    assert_eq!(result.iterations, 0);

    // 平坦: 全面の|z|<1e-6
    let z = max_abs_z(&result.frame);
    assert!(z < 1e-6, "max|z|={z}");
    let vertex_positions = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<HashMap<_, _>>();
    let faces_by_id = faces
        .iter()
        .map(|face| (face.id, face))
        .collect::<HashMap<_, _>>();
    for face3d in &result.frame.faces {
        let face = faces_by_id[&face3d.face];
        let source = face
            .vertices
            .iter()
            .map(|vertex| vertex_positions[vertex])
            .collect::<Vec<_>>();
        let folded = face3d
            .polygon
            .iter()
            .map(|point| [point[0], point[1]])
            .collect::<Vec<_>>();
        let source_area = signed_area_xy(&source);
        let folded_area = signed_area_xy(&folded);
        assert!(
            source_area.abs() > 1e-12 && folded_area.abs() > 1e-12,
            "面{}の符号付き面積: source={source_area}, folded={folded_area}",
            face3d.face
        );
        assert_eq!(
            face3d.mirrored,
            source_area * folded_area < 0.0,
            "完成やっこさんの面{}の材質座標向き",
            face3d.face
        );
    }
    assert!(
        result.frame.faces.iter().any(|face| face.mirrored)
            && result.frame.faces.iter().any(|face| !face.mirrored),
        "完成やっこさんは表向き面と裏返った面の双方を含む"
    );
    let pos = folded_positions(&cp, &faces, &result.frame, 1e-6);

    // 外形: バウンディングボックスは1×1(回転正方形が紙の四辺に接する)
    let (min, max) = bbox_xy(&result.frame);
    assert!(
        (max[0] - min[0] - 1.0).abs() < 1e-6,
        "bbox={min:?}..{max:?}"
    );
    assert!(
        (max[1] - min[1] - 1.0).abs() < 1e-6,
        "bbox={min:?}..{max:?}"
    );
    let c = DVec3::new((min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, 0.0);

    // 全ての点が中心まわりの1/√2角の回転正方形(|dx|+|dy|≤0.5)の中にある
    for f3d in &result.frame.faces {
        for p in &f3d.polygon {
            let l1 = (p[0] - c.x).abs() + (p[1] - c.y).abs();
            assert!(l1 <= 0.5 + 1e-6, "面{}の点{p:?}が回転正方形の外", f3d.face);
        }
    }
    // 4隅は全て中心の1点に重なる
    for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
        let p = pos[&vid_at(&cp, corner)];
        assert!((p - c).length() < 1e-6, "隅{corner:?} → {p} ≠ 中心{c}");
    }
}

/// 段階(b): やっこさん(座布団折り2回相当)の受け入れテスト本体。
/// 全折り線(山8+谷12)を±180°で畳むと全面が同一平面に畳まれ、外形は
/// 1/2角の正方形。4隅と辺中点M1..M4は中心の1点に、内部頂点N1..N4は
/// 畳んだ正方形の4隅に重なる。
#[test]
fn yakko_double_blintz_folds_flat_to_half_square() {
    let cp = yakko_cp();

    // 構造: 頂点20(隅4+辺中点4+縁の1/4点8+内部N4)、辺36(輪郭16+山8+谷12)、
    // 面17(中央1+基層の三角形4+隅の小片3×4)。オイラーの式 V−E+F=1 を満たす
    assert_eq!(cp.vertices.len(), 20);
    assert_eq!(kind_counts(&cp), (16, 8, 12));
    assert_eq!(validate(&cp), Vec::<String>::new());
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 17);

    let drivers = full_fold_drivers(&cp);
    assert_eq!(drivers.len(), 20);
    let result = solve(&cp, &faces, &drivers, None);
    // 全ヒンジがdriverなので自由変数はなく、ループ閉包残差がそのまま
    // 「この展開図が±180°で平坦に畳めるか」の判定になる
    assert!(
        result.converged,
        "warnings={:?} angles={:?}",
        result.frame.warnings, result.angles
    );
    let gap = max_seam_gap(&cp, &faces, &result.frame);
    assert!(gap < 1e-6, "やっこさん: 面が {gap:.9} 離れている");
    for d in &drivers {
        let got = result.angles[&d.hinge];
        assert!(
            (got - d.target_angle_deg).abs() < 1e-9,
            "ヒンジ{}: {got} ≠ {}",
            d.hinge,
            d.target_angle_deg
        );
    }

    // 平坦: 全面の|z|<1e-6
    let z = max_abs_z(&result.frame);
    assert!(z < 1e-6, "max|z|={z}");
    let pos = folded_positions(&cp, &faces, &result.frame, 1e-6);

    // 外形: 1/2角の正方形(理論値と1e-6で一致)
    let (min, max) = bbox_xy(&result.frame);
    assert!(
        (max[0] - min[0] - 0.5).abs() < 1e-6,
        "bbox={min:?}..{max:?}"
    );
    assert!(
        (max[1] - min[1] - 0.5).abs() < 1e-6,
        "bbox={min:?}..{max:?}"
    );
    let c = DVec3::new((min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, 0.0);

    // 4隅と辺中点M1..M4は全て中心の1点に重なる(座布団折り2回の性質)
    for q in [
        [0.0, 0.0],
        [1.0, 0.0],
        [1.0, 1.0],
        [0.0, 1.0],
        [0.5, 0.0],
        [1.0, 0.5],
        [0.5, 1.0],
        [0.0, 0.5],
    ] {
        let p = pos[&vid_at(&cp, q)];
        assert!((p - c).length() < 1e-6, "{q:?} → {p} ≠ 中心{c}");
    }

    // 内部頂点N1..N4は畳んだ正方形の4隅へ1対1で写る
    let bbox_corners = [
        DVec3::new(min[0], min[1], 0.0),
        DVec3::new(max[0], min[1], 0.0),
        DVec3::new(max[0], max[1], 0.0),
        DVec3::new(min[0], max[1], 0.0),
    ];
    let n_points = [[0.75, 0.25], [0.75, 0.75], [0.25, 0.75], [0.25, 0.25]];
    for corner in bbox_corners {
        let hits = n_points
            .iter()
            .filter(|&&n| (pos[&vid_at(&cp, n)] - corner).length() < 1e-6)
            .count();
        assert_eq!(hits, 1, "畳んだ正方形の隅{corner}に写るNが{hits}個");
    }
}

/// 診断で代表にしたヒンジ#20を有効な平坦形から往復させても、他の折り目が譲って
/// 紙の交差を作らない。上昇は以前の指定を0°、下降は完成角へ保つmediumとする。
#[test]
fn yakko_hinge_20_round_trip_stays_contact_free() {
    let magnitudes: Vec<u32> = (0..=180).collect();
    assert_yakko_hinge_20_round_trip(&magnitudes, "1°刻み");
}

#[test]
fn yakko_hinge_20_sixteen_degree_jumps_stay_contact_free() {
    let mut magnitudes: Vec<u32> = (0..=180).step_by(16).collect();
    if magnitudes.last() != Some(&180) {
        magnitudes.push(180);
    }
    assert_yakko_hinge_20_round_trip(&magnitudes, "16°飛び");
}

/// 決定性: 同じ入力で2回solveすると、角度も3D座標も完全に同一になる。
#[test]
fn yakko_solve_twice_gives_identical_results() {
    let cp = yakko_cp();
    let faces = extract_faces(&cp);
    let drivers = full_fold_drivers(&cp);

    let r1 = solve(&cp, &faces, &drivers, None);
    let r2 = solve(&cp, &faces, &drivers, None);
    assert!(r1.converged && r2.converged);
    assert_eq!(r1.iterations, r2.iterations);
    assert_eq!(r1.angles, r2.angles);
    assert_eq!(r1.frame.faces.len(), r2.frame.faces.len());
    for (f1, f2) in r1.frame.faces.iter().zip(&r2.frame.faces) {
        assert_eq!(f1.face, f2.face);
        assert_eq!(f1.mirrored, f2.mirrored, "面{}の鏡映偶奇が不一致", f1.face);
        assert_eq!(f1.polygon, f2.polygon, "面{}の座標が不一致", f1.face);
    }
}

/// 手動確認の代替(UIの操作フロー検証): 線を描く順序を変えても
/// (交点の自動分割が逆向きに働いても)同じ展開図になり、面数も一致する。
#[test]
fn drawing_order_does_not_change_the_cp() {
    // 操作列1: 1回目の折り線 → 2回目の折り線(通常の手順)
    let cp1 = yakko_cp();
    // 操作列2: 2回目の折り線を先に描き、1回目の折り線を後から重ねる
    let mut cp2 = new_square_cp();
    draw(&mut cp2, &second_blintz_strokes());
    draw(&mut cp2, &blintz_strokes());

    assert_eq!(canonical_edges(&cp1), canonical_edges(&cp2));
    assert_eq!(extract_faces(&cp2).len(), 17);
    assert_eq!(validate(&cp2), Vec::<String>::new());
}

/// 単一hard折りを平坦側から追った経路と、利用者へ返すpolygonが別の閉包枝に
/// 着地しても、表示用の重なり順位は返却polygon自身の上下と一致しなければならない。
/// 終点だけでは `+180°` と `-180°` の途中の姿勢を区別できないため、179.99°の
/// 実深度も固定し、179.999°では面法線から独立に期待上下を決める。
#[test]
fn yakko_single_hard_surface_rank_matches_returned_branch() {
    const CHECKPOINTS: [f64; 22] = [
        9.0, 19.0, 29.0, 39.0, 49.0, 59.0, 69.0, 79.0, 90.0, 101.0, 111.0, 121.0, 131.0, 141.0,
        151.0, 161.0, 171.0, 179.0, 179.5, 179.9, 179.99, 179.999,
    ];
    // 179.99°の実測最小深度2.0568902387e-5に対して約8割の余裕付き境目。
    const PATH_DEPTH_MIN: f64 = 1.6e-5;
    // 179.999°の返却frame実測最小深度2.0568902491e-6に対して約8割の境目。
    const RETURNED_DEPTH_MIN: f64 = 1.6e-6;

    fn polygon(frame: &Frame3D, face_id: u32) -> &[[f64; 3]] {
        &frame
            .faces
            .iter()
            .find(|face| face.face == face_id)
            .unwrap_or_else(|| panic!("面{face_id}が返却frameにない"))
            .polygon
    }

    fn face_normal(points: &[[f64; 3]]) -> DVec3 {
        let mut normal = DVec3::ZERO;
        for index in 0..points.len() {
            normal +=
                DVec3::from(points[index]).cross(DVec3::from(points[(index + 1) % points.len()]));
        }
        normal.normalize()
    }

    fn canonical_up(normal: DVec3) -> DVec3 {
        let absolute = normal.abs();
        let component = if absolute.x >= absolute.y && absolute.x >= absolute.z {
            normal.x
        } else if absolute.y >= absolute.z {
            normal.y
        } else {
            normal.z
        };
        if component < 0.0 { -normal } else { normal }
    }

    fn centroid(points: &[[f64; 3]]) -> DVec3 {
        points
            .iter()
            .map(|point| DVec3::from(*point))
            .sum::<DVec3>()
            / points.len() as f64
    }

    fn pair_depth(frame: &Frame3D, reference: u32, other: u32) -> (f64, DVec3, DVec3) {
        let reference_points = polygon(frame, reference);
        let other_points = polygon(frame, other);
        let normal = face_normal(reference_points);
        let up = canonical_up(normal);
        (
            (centroid(other_points) - centroid(reference_points)).dot(up),
            normal,
            up,
        )
    }

    fn max_vertex_delta(left: &Frame3D, right: &Frame3D) -> f64 {
        left.faces.iter().fold(0.0_f64, |maximum, left_face| {
            let right_points = polygon(right, left_face.face);
            assert_eq!(left_face.polygon.len(), right_points.len());
            left_face.polygon.iter().zip(right_points).fold(
                maximum,
                |maximum, (left_point, right_point)| {
                    maximum.max((DVec3::from(*left_point) - DVec3::from(*right_point)).length())
                },
            )
        })
    }

    let cp = yakko_cp();
    let faces = extract_faces(&cp);
    let mut rank_violations = Vec::new();

    // (ヒンジ, BFS基準面, 相手面, 平坦側179.99°で相手が上か)
    for (hinge, reference, other, path_other_is_above) in [(19, 1, 4, true), (51, 14, 16, false)] {
        let owners = faces
            .iter()
            .filter(|face| face.edges.contains(&hinge))
            .map(|face| face.id)
            .collect::<Vec<_>>();
        assert_eq!(owners, vec![reference, other], "ヒンジ{hinge}の所有面");

        let mut warm = None;
        let mut before_flat = None;
        let mut flat_path_endpoint = None;
        for checkpoint in CHECKPOINTS {
            let solved = solve(
                &cp,
                &faces,
                &[Driver {
                    hinge,
                    target_angle_deg: checkpoint,
                }],
                warm.as_ref(),
            );
            assert!(solved.converged, "ヒンジ{hinge} {checkpoint}°が未収束");
            assert!(
                (solved.angles[&hinge] - checkpoint).abs() < 1e-9,
                "ヒンジ{hinge} {checkpoint}°のhard誤差={:e}",
                solved.angles[&hinge] - checkpoint
            );
            assert!(
                self_intersection_pairs(&solved.frame).is_empty(),
                "ヒンジ{hinge} {checkpoint}°のflat-pathが自己交差"
            );
            if checkpoint == 179.99 {
                before_flat = Some(solved.frame.clone());
            }
            if checkpoint == 179.999 {
                flat_path_endpoint = Some(solved.frame.clone());
            }
            warm = Some(solved.angles);
        }
        let before_flat = before_flat.expect("179.99°の経路frame");
        let flat_path_endpoint = flat_path_endpoint.expect("179.999°の経路frame");
        let (path_depth, _, _) = pair_depth(&before_flat, reference, other);
        assert!(
            path_depth.abs() > PATH_DEPTH_MIN && (path_depth > 0.0) == path_other_is_above,
            "ヒンジ{hinge} 179.99°の途中深度={path_depth:.17e}"
        );

        let motion = solve_motion_with_contact_options(
            &cp,
            &faces,
            &[Driver {
                hinge,
                target_angle_deg: 179.999,
            }],
            None,
            None,
            MotionContactOptions {
                detect: true,
                prevent: false,
            },
        );
        let returned = &motion.result;
        assert!(
            motion.surface_order_authoritative,
            "ヒンジ{hinge}: {:#?}",
            motion.surface_order
        );
        assert!(
            self_intersection_pairs(&returned.frame).is_empty(),
            "ヒンジ{hinge}: 返却frameが自己交差"
        );

        let actual = returned.angles[&hinge];
        // 返却解をwarmに1段だけ戻し、返却枝そのものの途中姿勢でも上下を測る。
        let returned_branch_before = solve(
            &cp,
            &faces,
            &[Driver {
                hinge,
                target_angle_deg: 179.99,
            }],
            Some(&returned.angles),
        );
        assert!(
            returned_branch_before.converged
                && (returned_branch_before.angles[&hinge] - 179.99).abs() < 1e-9,
            "ヒンジ{hinge}: 返却枝を179.99°へ戻せない"
        );
        assert!(
            self_intersection_pairs(&returned_branch_before.frame).is_empty(),
            "ヒンジ{hinge}: 返却枝179.99°が自己交差"
        );
        let (returned_branch_depth, returned_branch_normal, returned_branch_up) =
            pair_depth(&returned_branch_before.frame, reference, other);
        let returned_branch_other_is_above = returned_branch_normal.dot(returned_branch_up) < 0.0;

        let (returned_depth, returned_normal, returned_up) =
            pair_depth(&returned.frame, reference, other);
        let oracle_other_is_above = (actual < 0.0) == (returned_normal.dot(returned_up) > 0.0);
        assert!(
            returned_depth.abs() > RETURNED_DEPTH_MIN
                && (returned_depth > 0.0) == oracle_other_is_above,
            "ヒンジ{hinge}: 独立法線oracle={oracle_other_is_above} 返却実深度={returned_depth:.17e}"
        );
        assert!(
            returned_branch_depth.abs() > PATH_DEPTH_MIN
                && (returned_branch_depth > 0.0) == returned_branch_other_is_above
                && returned_branch_other_is_above == oracle_other_is_above,
            "ヒンジ{hinge}: 返却枝179.99° depth={returned_branch_depth:.17e} \
             other_above={returned_branch_other_is_above} final_oracle={oracle_other_is_above}"
        );

        let reference_rank = returned
            .frame
            .faces
            .iter()
            .find(|face| face.face == reference)
            .expect("基準面")
            .surface_rank;
        let other_rank = returned
            .frame
            .faces
            .iter()
            .find(|face| face.face == other)
            .expect("相手面")
            .surface_rank;
        let ranked_other_is_above = other_rank > reference_rank;
        let branch_delta = max_vertex_delta(&flat_path_endpoint, &returned.frame);
        // 修正前の枝差は0.022885/0.034523。正しい修正が枝自体を一致させる場合も
        // あるため合格条件にはせず、rank不一致時の原因診断値としてだけ残す。
        if ranked_other_is_above != oracle_other_is_above {
            rank_violations.push(format!(
                "hinge={hinge} owners=({reference},{other}) actual={actual:.17} \
                 ranks=({reference_rank},{other_rank}) oracle_other_above={oracle_other_is_above} \
                 returned_depth={returned_depth:.17e} path179.99_depth={path_depth:.17e} \
                 returned_branch179.99_depth={returned_branch_depth:.17e} \
                 flat_path_returned_delta={branch_delta:.17e} diagnostics={:#?}",
                motion.surface_order
            ));
        }
    }

    assert!(
        rank_violations.is_empty(),
        "返却polygonと表示用surface_rankの上下が不一致:\n{}",
        rank_violations.join("\n")
    );
}

/// 負のテスト: 「折った状態の折り線(内側の正方形)だけ」を展開図に重ねた
/// 素朴なCPは、1回目で裏返った隅の層を貫く延長区間が欠けるため、内部頂点Nの
/// 扇形が90°/45°/180°/45°となり川崎の定理を満たさない=平坦折り不能。
/// ソルバーはこれを converged=false として正しく検出する
/// (受け入れテストのconvergedが自明に真ではないことの裏付け)。
#[test]
fn naive_inner_square_overlay_is_not_flat_foldable() {
    let mut cp = new_square_cp();
    draw(&mut cp, &blintz_strokes());
    let (n1, n2, n3, n4) = ([0.75, 0.25], [0.75, 0.75], [0.25, 0.75], [0.25, 0.25]);
    draw(
        &mut cp,
        &[
            (n1, n2, EdgeKind::Valley),
            (n2, n3, EdgeKind::Valley),
            (n3, n4, EdgeKind::Valley),
            (n4, n1, EdgeKind::Valley),
        ],
    );

    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 9);
    let drivers = full_fold_drivers(&cp);
    assert_eq!(drivers.len(), 12);
    let result = solve(&cp, &faces, &drivers, None);
    assert!(!result.converged, "平坦折り不能なCPで収束してしまった");
    assert!(
        result
            .frame
            .warnings
            .iter()
            .any(|w| w.contains("収束していません")),
        "warnings={:?}",
        result.frame.warnings
    );
}
