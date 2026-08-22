//! 「紙の本立て」の受け入れテスト。**平らに畳まず、立体のまま自立する**作品。
//!
//! # なぜこの作品か
//!
//! リポジトリにある他の作品(折り鶴・カエル・やっこさん・鳥の基本形)は、
//! すべて**平らに畳んだ状態**を合格条件にしている。そのため次の4つが、
//! どの作品でも一度も確かめられていなかった。
//!
//! - D1 立ち姿の三点支持(`three_point_support`)。**複数の面**があり、
//!   **支持面が座標軸に平行でなく傾いていて**、**重心の高さが0でない**場合。
//! - D2 左右対称の拘束で解いたとき、**対称化の射影が実際に回る**
//!   (`projection_iterations > 0`)。warm start が `None` でない場合。
//! - D3 折り角を線として書き出し、読み直して解き直したとき、**反復0で座標まで一致**すること。
//! - D4 剛体で解いた結果が**本当に立体になる**こと(`z_span > 0.1`)。
//!
//! 選んだ形は、帯状の紙に**平行な折り線を4本**引いて左右対称に折り、
//! 底・立ち上がり・支えの順に曲げて自立させる台(本立て・写真立ての足)である。
//! 選んだ理由は次の3つ。
//!
//! 1. **簡単**: 折り線は4本、面は5つ。折り線どうしが交わらないので内部頂点が無く、
//!    剛体で解いたとき必ず解ける(難しい作品にしない)。
//! 2. **左右対称**: 帯の中央線 `y = 0.5` について、頂点も辺も線種も鏡像になる。
//! 3. **立体で自立する**: 平らに畳まない。折り上がりは両端で床に触れて立つ。
//!
//! # 実測(このテストを手元で実行した値)
//!
//! | 量 | 値 |
//! |---|---|
//! | 面 / ヒンジ | 5 / 4 |
//! | 対称化の射影 | 1回 |
//! | 鏡像ヒンジの角度差 | 0.000e0 |
//! | `z_span` | 0.518310 |
//! | `max_seam_gap` | 1.6e-16 |
//! | 支持面の法線 | `[0, 0.9063, 0.4226]`(座標軸に平行でない) |
//! | 重心の高さ | 0.173536 |
//! | 支持三角形の面積 | 0.201423 |
//! | 書き出して解き直した反復 | 0 |

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces, insert_segment, local_violations, validate};
use ori3_model::{
    CreasePattern, Document, Driver, DriverLine, EPS, EdgeId, EdgeKind, Frame3D, Paper, VertexId,
};
use ori3_rigid::{
    max_seam_gap, solve, solve_near_with_reflection_symmetry, three_point_support,
    three_point_support_with_tolerance,
};

/// 折り線を引く位置。中央線 `y = 0.5` について鏡像になっている。
const CREASE_Y: [f64; 4] = [0.2, 0.35, 0.65, 0.8];
/// 左右対称の鏡映軸(帯の中央線)。
const MIRROR_AXIS: [[f64; 2]; 2] = [[0.0, 0.5], [1.0, 0.5]];
/// 底と支えを立ち上げる角度。両端で同じ値を使う(鏡像の対)。
const STAND_ANGLE_DEG: f64 = -90.0;
/// 表示幾何の許容差(正規化紙長あたり)。
const SUPPORT_TOL: f64 = 1e-6;

fn paper_stand_cp() -> CreasePattern {
    let mut document = Document::new(Paper {
        width_mm: 200.0,
        height_mm: 200.0,
    });
    for y in CREASE_Y {
        insert_segment(&mut document.cp, [0.0, y], [1.0, y], EdgeKind::Valley);
    }
    document.cp
}

/// `y` の高さにある折り目の辺IDを返す。折り線どうしが交わらないので1本に決まる。
fn crease_at(cp: &CreasePattern, y: f64) -> EdgeId {
    let positions: HashMap<VertexId, [f64; 2]> =
        cp.vertices.iter().map(|v| (v.id, v.pos)).collect();
    let mut found: Vec<EdgeId> = cp
        .edges
        .iter()
        .filter(|edge| {
            edge.kind == EdgeKind::Valley
                && (positions[&edge.v0][1] - y).abs() < 1e-9
                && (positions[&edge.v1][1] - y).abs() < 1e-9
        })
        .map(|edge| edge.id)
        .collect();
    found.sort_unstable();
    assert_eq!(found.len(), 1, "y={y} の折り目は1本に決まる: {found:?}");
    found[0]
}

fn z_span(frame: &Frame3D) -> f64 {
    let (lo, hi) = frame
        .faces
        .iter()
        .flat_map(|face| &face.polygon)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), point| {
            (lo.min(point[2]), hi.max(point[2]))
        });
    hi - lo
}

/// 折り上がりの頂点座標。共有頂点が裂けていないことも同時に確かめる。
fn folded_vertices(faces: &[Face], frame: &Frame3D) -> HashMap<VertexId, DVec3> {
    let by_id: HashMap<u32, &Face> = faces.iter().map(|face| (face.id, face)).collect();
    let mut positions = HashMap::<VertexId, DVec3>::new();
    for face3d in &frame.faces {
        let face = by_id[&face3d.face];
        assert_eq!(face.vertices.len(), face3d.polygon.len());
        for (&vertex, &point) in face.vertices.iter().zip(&face3d.polygon) {
            let point = DVec3::from(point);
            if let Some(previous) = positions.insert(vertex, point) {
                assert!(
                    (previous - point).length() < 1e-9,
                    "共有頂点{vertex}が裂けた: {previous} vs {point}"
                );
            }
        }
    }
    positions
}

/// 鏡像の頂点対が、1枚の平面について実際に鏡像になっているかの最大誤差。
fn reflection_error(
    positions: &HashMap<VertexId, DVec3>,
    mirrors: &HashMap<VertexId, VertexId>,
) -> f64 {
    let pairs: Vec<(DVec3, DVec3)> = mirrors
        .iter()
        .map(|(&vertex, &mirror)| (positions[&vertex], positions[&mirror]))
        .collect();
    assert!(
        pairs.len() >= 8,
        "対称性を見る点が少なすぎる: {}",
        pairs.len()
    );
    let &(p0, q0) = pairs
        .iter()
        .max_by(|(a0, b0), (a1, b1)| {
            (*a0 - *b0)
                .length_squared()
                .total_cmp(&(*a1 - *b1).length_squared())
        })
        .expect("鏡像の対");
    let normal = (p0 - q0).normalize();
    let offset = normal.dot((p0 + q0) * 0.5);
    pairs
        .into_iter()
        .map(|(p, q)| {
            let reflected = p - 2.0 * normal * (normal.dot(p) - offset);
            (reflected - q).length()
        })
        .fold(0.0, f64::max)
}

/// 解いた角度を、頂点座標から作った線として書き出す(保存の再現)。
fn record_driver_lines(cp: &CreasePattern, angles: &HashMap<EdgeId, f64>) -> Vec<DriverLine> {
    let positions: HashMap<VertexId, [f64; 2]> =
        cp.vertices.iter().map(|v| (v.id, v.pos)).collect();
    let mut edges: Vec<_> = cp
        .edges
        .iter()
        .filter(|edge| angles.contains_key(&edge.id))
        .collect();
    edges.sort_unstable_by_key(|edge| edge.id);
    edges
        .into_iter()
        .map(|edge| DriverLine {
            a: positions[&edge.v0],
            b: positions[&edge.v1],
            target_angle_deg: angles[&edge.id],
        })
        .collect()
}

/// 書き出した線を、辺IDを一切使わずに座標だけで辺へ解決し直す(読み直しの再現)。
fn resolve_driver_lines(cp: &CreasePattern, lines: &[DriverLine]) -> Vec<Driver> {
    let positions: HashMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();
    let mut by_edge = HashMap::<EdgeId, f64>::new();
    for line in lines {
        let a = DVec2::from(line.a);
        let b = DVec2::from(line.b);
        let mut resolved: Vec<EdgeId> = cp
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .filter(|edge| {
                let (p, q) = (positions[&edge.v0], positions[&edge.v1]);
                ((p - a).length() <= EPS && (q - b).length() <= EPS)
                    || ((p - b).length() <= EPS && (q - a).length() <= EPS)
            })
            .map(|edge| edge.id)
            .collect();
        resolved.sort_unstable();
        assert_eq!(
            resolved.len(),
            1,
            "書き出した線が辺へ解決できない: {line:?} -> {resolved:?}"
        );
        assert!(
            by_edge.insert(resolved[0], line.target_angle_deg).is_none(),
            "同じ辺へ2本の線が解決された"
        );
    }
    let mut drivers: Vec<Driver> = by_edge
        .into_iter()
        .map(|(hinge, target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect();
    drivers.sort_unstable_by_key(|driver| driver.hinge);
    drivers
}

/// 左右対称の拘束で、本立ての形まで解く。
///
/// 内側の2本には**わざと左右で違う角度**(-15° と -35°)を要望として渡す。
/// 要望のままでは左右が食い違うので、対称化の射影が回って平均の -25° に落ち着く。
/// これが `projection_iterations > 0` を通す道筋である。
type SolvedStand = (
    CreasePattern,
    Vec<Face>,
    ori3_rigid::ReflectionSymmetrySolveResult,
);

fn solve_paper_stand() -> SolvedStand {
    let cp = paper_stand_cp();
    let faces = extract_faces(&cp);
    let outer = [crease_at(&cp, CREASE_Y[0]), crease_at(&cp, CREASE_Y[3])];
    let inner = [crease_at(&cp, CREASE_Y[1]), crease_at(&cp, CREASE_Y[2])];

    let hard: Vec<Driver> = outer
        .into_iter()
        .map(|hinge| Driver {
            hinge,
            target_angle_deg: STAND_ANGLE_DEG,
        })
        .collect();
    let targets: HashMap<EdgeId, f64> = [
        (outer[0], STAND_ANGLE_DEG),
        (outer[1], STAND_ANGLE_DEG),
        (inner[0], -15.0),
        (inner[1], -35.0),
    ]
    .into_iter()
    .collect();
    // warm start は `None` ではなく、平らでもない途中の姿勢を明示して渡す。
    let warm: HashMap<EdgeId, f64> = [
        (outer[0], STAND_ANGLE_DEG * 0.5),
        (outer[1], STAND_ANGLE_DEG * 0.5),
        (inner[0], -5.0),
        (inner[1], -20.0),
    ]
    .into_iter()
    .collect();

    let symmetric =
        solve_near_with_reflection_symmetry(&cp, &faces, &hard, &targets, Some(&warm), MIRROR_AXIS)
            .expect("本立ては左右対称の拘束で解ける");
    (cp, faces, symmetric)
}

#[test]
fn the_paper_stand_crease_pattern_is_simple_and_mirror_symmetric() {
    let cp = paper_stand_cp();

    // 実測: 折り線4本 -> 頂点12 / 辺16 / 面5。折り線どうしが交わらないので内部頂点は無い。
    assert_eq!(cp.vertices.len(), 12);
    assert_eq!(cp.edges.len(), 16);
    assert_eq!(extract_faces(&cp).len(), 5);
    assert!(validate(&cp).is_empty(), "CP検証警告={:?}", validate(&cp));
    assert!(local_violations(&cp).is_empty());

    // 中央線について、すべての頂点に鏡像の相手がいる。
    for vertex in &cp.vertices {
        let mirrored = [vertex.pos[0], 1.0 - vertex.pos[1]];
        assert!(
            cp.vertices
                .iter()
                .any(|other| { (DVec2::from(other.pos) - DVec2::from(mirrored)).length() <= EPS }),
            "頂点{}({:?})の鏡像が無い",
            vertex.id,
            vertex.pos
        );
    }
}

/// D2 + D4: 対称化の射影が実際に回り、解いた結果が立体になる。
#[test]
fn solving_with_reflection_symmetry_runs_the_projection_and_stands_up_in_three_dimensions() {
    let (cp, faces, symmetric) = solve_paper_stand();
    let solved = &symmetric.result;

    assert!(solved.converged, "warnings={:?}", solved.frame.warnings);
    assert!(solved.frame.warnings.is_empty());
    assert_eq!(solved.frame.faces.len(), faces.len(), "面が失われた");
    assert_eq!(solved.angles.len(), 4, "ヒンジは4本");

    // D2: 左右で違う角度を要望したので、対称化の射影が必ず1回以上回る。
    // 単体検査(`symmetry.rs`)は射影が0回の場合しか通していない。
    assert!(
        symmetric.projection_iterations >= 1,
        "対称化の射影が回らなかった: {}",
        symmetric.projection_iterations
    );
    assert!(
        symmetric.max_mirrored_angle_error_deg < 1e-9,
        "鏡像ヒンジの角度差={:.3e}",
        symmetric.max_mirrored_angle_error_deg
    );
    // 食い違った要望(-15° と -35°)は、その平均(-25°)へそろえられる。
    let inner = [crease_at(&cp, CREASE_Y[1]), crease_at(&cp, CREASE_Y[2])];
    for hinge in inner {
        assert!(
            (solved.angles[&hinge] + 25.0).abs() < 1e-9,
            "内側のヒンジ{hinge}={}",
            solved.angles[&hinge]
        );
    }
    // 指定した外側の2本は要望どおりに保たれる。
    for hinge in [crease_at(&cp, CREASE_Y[0]), crease_at(&cp, CREASE_Y[3])] {
        assert!((solved.angles[&hinge] - STAND_ANGLE_DEG).abs() < 1e-9);
    }

    // 裂けていない。
    let gap = max_seam_gap(&cp, &faces, &solved.frame);
    assert!(gap < 1e-6, "max_seam_gap={gap:.3e}");

    // D4: 平らではなく立体である。他の作品は逆に「平坦」を主張しているので、
    // 剛体で解いた結果が本当に平面から立ち上がることは、ここでしか見ていない。
    let span = z_span(&solved.frame);
    assert!(span > 0.1, "平らなまま: z_span={span:.6}");

    // 折り上がりも1枚の平面について鏡像になっている(角度だけでなく形も対称)。
    let positions = folded_vertices(&faces, &solved.frame);
    let symmetry_error = reflection_error(&positions, &symmetric.mirrored_vertices);
    assert!(
        symmetry_error < 1e-9,
        "折り上がりの左右対称誤差={symmetry_error:.3e}"
    );

    println!(
        "本立て: 面{} ヒンジ{} 射影{}回 角度差{:.3e} z_span={span:.6} gap={gap:.3e} 対称誤差{symmetry_error:.3e}",
        faces.len(),
        solved.angles.len(),
        symmetric.projection_iterations,
        symmetric.max_mirrored_angle_error_deg,
    );
}

/// D1: 立ち姿の三点支持。複数面・傾いた支持面・重心の高さが0でない場合を通す。
#[test]
fn the_folded_stand_is_supported_by_three_points_on_a_tilted_plane() {
    let (_cp, faces, symmetric) = solve_paper_stand();
    let frame = &symmetric.result.frame;
    assert!(faces.len() >= 3, "支持の評価は複数の面にまたがる");

    // 帯の両端の3隅で床を作る。
    let support = three_point_support_with_tolerance(&faces, frame, [0, 1, 2], SUPPORT_TOL)
        .expect("両端の3隅は支持面を作る");

    assert!(support.one_sided, "床より下へ出た点がある: {support:?}");
    assert!(
        support.centroid_projection_inside,
        "重心が支持三角形の外にある: {support:?}"
    );
    assert!(support.stable, "自立していない: {support:?}");

    // 重心の高さが0でない。単体検査(`support.rs`)は高さ0の平らな三角形1枚しか通していない。
    // 実測 0.173536。
    assert!(
        support.centroid_height > 0.1,
        "重心の高さ={:.6}",
        support.centroid_height
    );
    // 実測 0.201423。
    assert!(
        support.support_area > 0.1,
        "支持三角形の面積={:.6}",
        support.support_area
    );

    // 支持面が座標軸のどれにも平行でない(傾いている)。実測 [0, 0.9063, 0.4226]。
    let normal = DVec3::from(support.plane.normal);
    assert!((normal.length() - 1.0).abs() < 1e-9);
    for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
        assert!(
            normal.dot(axis).abs() < 0.99,
            "支持面が座標軸に平行: normal={normal:?}"
        );
    }
    // 重心も支持面も、有限で意味のある値になっている。
    assert!(support.surface_area > 0.0);
    assert!(
        support
            .centroid_barycentric
            .iter()
            .all(|weight| weight.is_finite())
    );

    println!(
        "支持: stable={} 高さ={:.6} 面積={:.6} 法線={:?} 重心={:?}",
        support.stable,
        support.centroid_height,
        support.support_area,
        support.plane.normal,
        support.surface_centroid
    );
}

/// D1(続き): 支えにならない3点を選ぶと、ちゃんと「立たない」と判定される。
///
/// 単体検査は `stable == true` の場合しか通していない。
#[test]
fn three_points_that_do_not_support_the_stand_are_reported_as_unstable() {
    let (_cp, faces, symmetric) = solve_paper_stand();
    let frame = &symmetric.result.frame;

    // 底面の3点。床より下へは出ないが、重心がこの三角形の外に落ちる。
    let tipping = three_point_support_with_tolerance(&faces, frame, [4, 5, 6], SUPPORT_TOL)
        .expect("底面の3点でも支持面は作れる");
    assert!(tipping.one_sided, "{tipping:?}");
    assert!(!tipping.centroid_projection_inside, "{tipping:?}");
    assert!(!tipping.stable, "重心が外なら自立と判定してはいけない");

    // 紙の途中の3点。こちらは床の下へ他の点が出る。
    let cutting = three_point_support_with_tolerance(&faces, frame, [8, 9, 4], SUPPORT_TOL)
        .expect("途中の3点でも支持面は作れる");
    assert!(!cutting.one_sided, "床を突き抜けているのに検出できない");
    assert!(!cutting.stable, "{cutting:?}");

    // 既定の許容差を使う入口でも同じ判定になる。
    let default_tolerance =
        three_point_support(&faces, frame, [4, 5, 6]).expect("既定の許容差でも評価できる");
    assert_eq!(default_tolerance.stable, tipping.stable);
}

/// D3: 折り角を線として書き出し、辺IDを使わずに読み直して解き直しても、
/// 反復0で同じ形へ戻る。
#[test]
fn recorded_driver_lines_replay_the_same_shape_without_any_iteration() {
    let (cp, faces, symmetric) = solve_paper_stand();
    let solved = &symmetric.result;

    let lines = record_driver_lines(&cp, &solved.angles);
    assert_eq!(lines.len(), solved.angles.len());
    // 書き出した線は座標だけを持ち、辺IDを持たない。
    let replayed_drivers = resolve_driver_lines(&cp, &lines);
    assert_eq!(replayed_drivers.len(), solved.angles.len());

    let replayed = solve(&cp, &faces, &replayed_drivers, None);
    assert!(replayed.converged, "warnings={:?}", replayed.frame.warnings);
    assert!(replayed.frame.warnings.is_empty());
    // 記録した角度がそのまま解になっているので、1回も動かす必要がない。
    assert_eq!(replayed.iterations, 0, "読み直しで解き直しが必要になった");

    for driver in &replayed_drivers {
        assert!(
            (replayed.angles[&driver.hinge] - solved.angles[&driver.hinge]).abs() < 1e-9,
            "ヒンジ{}の角度が一致しない",
            driver.hinge
        );
    }
    assert_eq!(replayed.frame.faces.len(), solved.frame.faces.len());
    for (before, after) in solved.frame.faces.iter().zip(&replayed.frame.faces) {
        assert_eq!(before.face, after.face);
        assert_eq!(before.mirrored, after.mirrored);
        for (&p, &q) in before.polygon.iter().zip(&after.polygon) {
            assert!(
                (DVec3::from(p) - DVec3::from(q)).length() < 1e-9,
                "面{}の座標が一致しない: {p:?} / {q:?}",
                before.face
            );
        }
    }
    let replay_gap = max_seam_gap(&cp, &faces, &replayed.frame);
    assert!(
        replay_gap < 1e-6,
        "読み直しの max_seam_gap={replay_gap:.3e}"
    );

    println!(
        "書き出し{}本 -> 読み直しの反復{} / gap={replay_gap:.3e}",
        lines.len(),
        replayed.iterations
    );
}
