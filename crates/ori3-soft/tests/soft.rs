//! たわみ計算(SIM-012〜015)の受け入れテスト。

use std::collections::BTreeMap;

use glam::DVec3;
use ori3_cp::{Face, extract_faces};
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{flat_state_at, replay, squash};
use ori3_model::{Document, Frame3D, Paper};
use ori3_soft::{SoftMesh, SoftSettings, relax};

/// 単位正方形の紙。
fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

/// 重なり全部をまとめて1手折る(`SeqOp::FoldThrough` と同じ手順)。
fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2], direction: FoldDirection) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line,
            keep_side_point: keep,
            target_layers: None,
            direction,
        },
    )
    .expect("折れる指定");
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
}

/// 開いてつぶす折りを1回適用する(`SeqOp::Technique` と同じ手順)。
fn squash_bottom(doc: &mut Document, line: [[f64; 2]; 2], reference_point: [f64; 2]) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = squash(
        &mut cp,
        &faces,
        &state,
        &TechniqueInput {
            flap: vec![state.order[0]],
            line,
            reference_point,
            open_to_back: None,
            polygon: None,
            center: None,
        },
    )
    .expect("折れる指定");
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
}

/// 予備基本形(折り鶴の第一段階。4層が輪につながった袋になる)。
/// `acceptance_crane.rs` の `preliminary_base` と同じ折り順。
fn preliminary_base() -> Document {
    let mut doc = square_doc();
    fold(&mut doc, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25], FoldDirection::Up);
    fold(&mut doc, [[0.5, 0.0], [0.5, 0.5]], [0.25, 0.25], FoldDirection::Up);
    squash_bottom(&mut doc, [[0.5, 0.0], [0.5, 1.0]], [0.5, 0.1]);
    squash_bottom(&mut doc, [[0.0, 0.5], [1.0, 0.5]], [0.1, 0.5]);
    doc
}

/// 手順を最後まで(`t`まで)再生した面とフレーム。
fn folded(doc: &Document, t: f64) -> (Vec<Face>, Frame3D) {
    let faces = extract_faces(&doc.cp);
    (faces, replay(doc, doc.sequence.len(), t).frame)
}

/// 半分に折った紙(2層)。
fn half_folded(t: f64) -> (Document, Vec<Face>, Frame3D) {
    let mut doc = square_doc();
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 1.0]],
        [0.25, 0.5],
        FoldDirection::Up,
    );
    let (faces, frame) = folded(&doc, t);
    (doc, faces, frame)
}

fn settings(enabled: bool, stiffness: f64, pressure: f64) -> SoftSettings {
    SoftSettings {
        enabled,
        subdivision: 2,
        stiffness,
        pressure,
        iterations: 20,
    }
}

/// 網の二面角: (折り目=面をまたぐ辺の角度の絶対値の合計, 面の中の辺の合計)。
fn bend_stats(mesh: &SoftMesh) -> (f64, f64) {
    let p = |i: u32| DVec3::from(mesh.positions[i as usize]);
    let normal = |t: [u32; 3]| (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).normalize_or_zero();
    let mut seen: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let (mut crease, mut inner) = (0.0, 0.0);
    for (ti, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            match seen.insert((a.min(b), a.max(b)), ti) {
                None => {}
                Some(t0) => {
                    let ang = normal(mesh.triangles[t0]).dot(normal(*t)).clamp(-1.0, 1.0).acos();
                    if mesh.triangle_faces[t0] == mesh.triangle_faces[ti] {
                        inner += ang;
                    } else {
                        crease += ang;
                    }
                }
            }
        }
    }
    (crease, inner)
}

/// 層ごとの三角形の重心の高さ(z)の平均。
fn mean_height(mesh: &SoftMesh, layer: u32) -> f64 {
    let zs: Vec<f64> = mesh
        .triangles
        .iter()
        .zip(&mesh.triangle_layers)
        .filter(|&(_, &l)| l == layer)
        .map(|(t, _)| t.iter().map(|&i| mesh.positions[i as usize][2]).sum::<f64>() / 3.0)
        .collect();
    zs.iter().sum::<f64>() / zs.len() as f64
}

#[test]
fn flat_paper_stays_flat_without_pressure() {
    let doc = square_doc();
    let (faces, frame) = folded(&doc, 1.0);
    let a = relax(&doc.cp, &faces, &frame, &settings(true, 0.5, 0.0));
    let mut long = settings(true, 0.5, 0.0);
    long.iterations = 200;
    let b = relax(&doc.cp, &faces, &frame, &long);
    assert_eq!(a.positions, b.positions, "反復しても1ビットも動かない");
    assert!(a.positions.iter().all(|p| p[2] == 0.0), "平らなまま");
    assert!(a.triangles.len() >= 32, "細分される: {}", a.triangles.len());
}

#[test]
fn disabled_returns_the_rigid_polygons_untouched() {
    let (doc, faces, frame) = half_folded(1.0);
    let m = relax(&doc.cp, &faces, &frame, &settings(false, 0.5, 0.9));
    let want: usize = faces.iter().map(|f| f.vertices.len() - 2).sum();
    assert_eq!(m.triangles.len(), want, "多角形を三角形にしただけ");
    let pts: Vec<[f64; 3]> = frame.faces.iter().flat_map(|f| f.polygon.clone()).collect();
    assert!(m.positions.iter().all(|p| pts.contains(p)), "位置はそのまま");
}

#[test]
fn same_input_gives_bitwise_same_result() {
    let (doc, faces, frame) = half_folded(1.0);
    let s = settings(true, 0.4, 0.7);
    let a = relax(&doc.cp, &faces, &frame, &s);
    let b = relax(&doc.cp, &faces, &frame, &s);
    assert_eq!(a.positions, b.positions, "決定的");
    assert_eq!(a.triangles, b.triangles);
}

#[test]
fn the_mesh_is_shared_across_the_crease() {
    // 途中まで折った形(2枚が離れている)なら、重なった点があってはならない。
    // 折り目の上の点が面ごとに二重にできていれば、ここで同じ位置の2点になる。
    let (doc, faces, frame) = half_folded(0.6);
    let m = relax(&doc.cp, &faces, &frame, &settings(true, 0.5, 0.0));
    let per_face_total: usize = faces.iter().map(|f| f.vertices.len()).sum();
    assert!(m.positions.len() > per_face_total, "細分された点がある");
    for (i, a) in m.positions.iter().enumerate() {
        for b in &m.positions[i + 1..] {
            let d = (DVec3::from(*a) - DVec3::from(*b)).length();
            assert!(d > 1e-9, "折り目の上の点が二重になっていない: {a:?} {b:?}");
        }
    }
}

#[test]
fn crease_angle_is_kept_while_the_paper_bends() {
    // 半分に折った紙(折り目の角度 = 0.6 × 180°)
    let (doc, faces, frame) = half_folded(0.6);
    let rigid = 0.6 * std::f64::consts::PI;
    let (c0, i0) = bend_stats(&relax(&doc.cp, &faces, &frame, &settings(true, 0.5, 0.0)));
    let creases = (c0 / rigid).round();
    assert!(creases >= 4.0, "折り目は細分されている: {creases}");
    assert!(
        (c0 - creases * rigid).abs() < 1e-6 && i0 < 1e-9,
        "膨らませなければ剛体折りの角度そのまま: {c0} (期待 {})",
        creases * rigid
    );
}

#[test]
fn the_crease_resists_much_more_than_the_paper_between_creases() {
    // 袋(予備基本形)を強く膨らませても、折り目の角度の変化はわずかで、
    // 変形は主に面の中のたわみとして現れる。
    let doc = preliminary_base();
    let (faces, frame) = folded(&doc, 1.0);
    let (c0, i0) = bend_stats(&relax(&doc.cp, &faces, &frame, &settings(true, 0.5, 0.0)));
    let (c1, i1) = bend_stats(&relax(&doc.cp, &faces, &frame, &settings(true, 0.5, 0.9)));
    assert!(
        (c1 - c0).abs() / c0 < 0.2,
        "折り目の角度はほぼ保たれる: {c0} → {c1}"
    );
    assert!(i1 > i0 * 10.0, "面の中は大きくたわむ: {i0} → {i1}");
}

#[test]
fn higher_stiffness_flattens_the_paper() {
    let doc = preliminary_base();
    let (faces, frame) = folded(&doc, 1.0);
    let (_, a) = bend_stats(&relax(&doc.cp, &faces, &frame, &settings(true, 0.05, 0.9)));
    let (_, b) = bend_stats(&relax(&doc.cp, &faces, &frame, &settings(true, 1.0, 0.9)));
    assert!(b < a * 0.5, "硬いほど曲がりの総量が減る: 柔={a} 硬={b}");
}

#[test]
fn higher_pressure_inflates_more() {
    let (doc, faces, frame) = half_folded(1.0);
    let base = relax(&doc.cp, &faces, &frame, &settings(true, 0.5, 0.0));
    let spread = |p: f64| {
        let m = relax(&doc.cp, &faces, &frame, &settings(true, 0.5, p));
        m.positions
            .iter()
            .zip(&base.positions)
            .map(|(a, b)| (DVec3::from(*a) - DVec3::from(*b)).length())
            .sum::<f64>()
    };
    let (lo, hi) = (spread(0.2), spread(0.8));
    assert!(hi > lo * 1.5, "強くするほど膨らむ: 弱={lo} 強={hi}");
}

#[test]
fn the_crane_base_bulges_when_inflated() {
    let doc = preliminary_base();
    let (faces, frame) = folded(&doc, 1.0);
    let mut layers: Vec<u32> = frame.faces.iter().map(|f| f.layer).collect();
    layers.sort_unstable();
    layers.dedup();
    assert_eq!(layers, vec![0, 1, 2, 3], "4層の袋になっている: {layers:?}");
    let extent = |m: &SoftMesh| {
        let zs: Vec<f64> = m.positions.iter().map(|p| p[2]).collect();
        zs.iter().copied().fold(f64::MIN, f64::max) - zs.iter().copied().fold(f64::MAX, f64::min)
    };
    let flat = relax(&doc.cp, &faces, &frame, &settings(true, 0.5, 0.0));
    let puffy = relax(&doc.cp, &faces, &frame, &settings(true, 0.5, 0.9));
    assert!(
        extent(&puffy) > extent(&flat) + 0.05,
        "膨らませると厚みが出る: {} → {}",
        extent(&flat),
        extent(&puffy)
    );
    let hs: Vec<f64> = layers.iter().map(|&l| mean_height(&puffy, l)).collect();
    assert!(hs.windows(2).all(|w| w[0] < w[1]), "層順どおり重なる: {hs:?}");
    assert!(puffy.warnings.is_empty(), "警告なし: {:?}", puffy.warnings);
}

#[test]
fn layer_order_is_kept_while_inflating() {
    let (doc, faces, frame) = half_folded(1.0);
    let m = relax(&doc.cp, &faces, &frame, &settings(true, 0.5, 0.9));
    let (z0, z1) = (mean_height(&m, 0), mean_height(&m, 1));
    assert!(z1 > z0 + 1e-4, "層0が層1より下にある: {z0} < {z1}");
    assert!(m.warnings.is_empty(), "層の警告は出ない: {:?}", m.warnings);
}

