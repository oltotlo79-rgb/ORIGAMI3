//! 折り鶴を膨らませたときのたわみ(SIM-013・要件§7.1c)の受け入れテスト。
//!
//! 折り順は `ori3-layers/tests/acceptance_crane.rs` の鶴とまったく同じ
//! (アプリが提供する折り操作だけで折る)。ここではその完成形を基準の形として
//! たわみ計算にかけ、**胴(袋になっている重なり)が自然に膨らむ**ことを見る。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{
    FlatState, FoldThroughResult, flat_state_at, inside_reverse, petal, replay, squash,
};
use ori3_model::{CreasePattern, Document, Driver, EdgeId, FaceId, Frame3D, Paper};
use ori3_rigid::{
    SolveResult, contact_metrics, max_seam_gap, self_intersection_pairs, solve, solve_motion,
    solve_near,
};
use ori3_soft::{OverlapReport, OverlapSettings, SoftMesh, SoftSettings, prevent_overlap, relax};

/// 紙の中心から細い先までの距離(鶴の基本形。1 - √2/2)。
const CORE: f64 = 1.0 - 0.5 * std::f64::consts::SQRT_2;
const SWEEP_HINGE: EdgeId = 24;
const SOLVE_BUDGET: Duration = Duration::from_millis(330);
const CONTACT_DIAGNOSIS_BUDGET: Duration = Duration::from_millis(500);

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<FoldThroughResult, String>;

fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

fn state_of(doc: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(doc, &faces, doc.sequence.len()).expect("平らに畳める");
    (faces, state)
}

/// 重ね折りを1手(`target_layers`を指定すると重なりの一部だけ)。
fn fold_layers(
    doc: &mut Document,
    line: [[f64; 2]; 2],
    keep: [f64; 2],
    target_layers: Option<Vec<FaceId>>,
    direction: FoldDirection,
) {
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
            target_layers,
            direction,
        },
    )
    .expect("折れる指定");
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
}

fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2], direction: FoldDirection) {
    fold_layers(doc, line, keep, None, direction);
}

/// 技法(つぶし折り・花弁折り・中割り折り)を1回適用する。
fn apply(
    doc: &mut Document,
    technique: Technique,
    flap: Vec<FaceId>,
    line: [[f64; 2]; 2],
    reference_point: [f64; 2],
    open_to_back: Option<bool>,
) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = technique(
        &mut cp,
        &faces,
        &state,
        &TechniqueInput {
            flap,
            line,
            reference_point,
            open_to_back,
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

/// 展開図の頂点の位置(頂点ID→座標)。
fn vertex_pos(cp: &CreasePattern) -> HashMap<u32, DVec2> {
    cp.vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect()
}

/// 畳み平面で点`p`を角に持つ層(=その先端を作っている紙)を下から順に返す。
fn layers_tipped_at(doc: &Document, p: DVec2) -> Vec<FaceId> {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let f = faces.iter().find(|f| f.id == *id).expect("層順序の面");
            let pl = state.placements[&f.id];
            f.vertices
                .iter()
                .filter_map(|v| pos.get(v).copied())
                .any(|q| (pl.apply(q) - p).length() < 1e-6)
        })
        .collect()
}

/// 展開図の四角い範囲にすっぽり入っている紙(=紙の1/4=1枚の羽)の層。
fn layers_in_quarter(doc: &Document, b: [f64; 4]) -> Vec<FaceId> {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let f = faces.iter().find(|f| f.id == *id).expect("層順序の面");
            f.vertices.iter().filter_map(|v| pos.get(v)).all(|p| {
                p.x >= b[0] - 1e-9 && p.x <= b[2] + 1e-9 && p.y >= b[1] - 1e-9 && p.y <= b[3] + 1e-9
            })
        })
        .collect()
}

/// 予備基本形(4層が輪につながった袋)。
fn preliminary_base() -> Document {
    let mut doc = square_doc();
    fold(
        &mut doc,
        [[0.0, 0.5], [1.0, 0.5]],
        [0.5, 0.25],
        FoldDirection::Up,
    );
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 0.5]],
        [0.25, 0.25],
        FoldDirection::Up,
    );
    for (line, reference) in [
        ([[0.5, 0.0], [0.5, 1.0]], [0.5, 0.1]),
        ([[0.0, 0.5], [1.0, 0.5]], [0.1, 0.5]),
    ] {
        let (_, state) = state_of(&doc);
        let bottom = vec![state.order[0]];
        apply(&mut doc, squash, bottom, line, reference, None);
    }
    doc
}

/// 鶴の基本形(予備基本形の前面と背面を1回ずつ花弁折り)。
fn bird_base() -> Document {
    let mut doc = preliminary_base();
    let center_line = [[0.0, 1.0], [0.5, 0.5]];
    let tip = [0.0, 1.0];
    let (_, state) = state_of(&doc);
    let front = vec![*state.order.last().expect("最前面")];
    apply(&mut doc, petal, front, center_line, tip, None);
    let side_b = layers_tipped_at(&doc, DVec2::new(0.5, 1.0));
    let back: Vec<FaceId> = layers_tipped_at(&doc, DVec2::new(0.0, 0.5))
        .into_iter()
        .filter(|id| side_b.contains(id))
        .collect();
    apply(&mut doc, petal, back, center_line, tip, Some(true));
    doc
}

#[derive(Clone, Copy)]
struct SweepSample {
    angle_deg: u32,
    raw_pairs: usize,
    display_pairs: usize,
    solve_time: Duration,
    raw_time: Duration,
    pbd_time: Duration,
    display_time: Duration,
    seam_time: Duration,
}

struct BirdBaseSweepInputs {
    doc: Document,
    faces: Vec<Face>,
    sign: f64,
    upward_targets: HashMap<EdgeId, f64>,
    downward_targets: HashMap<EdgeId, f64>,
    flat_warm: HashMap<EdgeId, f64>,
    completed_warm: HashMap<EdgeId, f64>,
}

fn assert_finite_sweep_result(
    result: &SolveResult,
    expected_faces: usize,
    direction: &str,
    angle_deg: u32,
) {
    assert!(
        result.closure_rms.is_finite(),
        "{direction} {angle_deg}°: closure RMSがfinite"
    );
    assert_eq!(
        result.frame.faces.len(),
        expected_faces,
        "{direction} {angle_deg}°: 全面を返す"
    );
    assert!(
        result.angles.values().all(|angle| angle.is_finite()),
        "{direction} {angle_deg}°: 全角度がfinite"
    );
    assert!(
        result.relaxations.iter().all(|relaxation| {
            relaxation.target_angle_deg.is_finite()
                && relaxation.actual_angle_deg.is_finite()
                && relaxation.delta_deg.is_finite()
        }),
        "{direction} {angle_deg}°: 譲歩診断がfinite"
    );
    assert!(
        result.frame.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        }),
        "{direction} {angle_deg}°: 全座標がfinite"
    );
}

fn assert_overlap_report_quality(
    report: &OverlapReport,
    raw_pairs: &[(FaceId, FaceId)],
    display_pairs: &[(FaceId, FaceId)],
    displayed: &Frame3D,
    // 展開図と面は常に一組で使うので、まとめて受け取る(引数を増やしすぎない)
    geometry: (&CreasePattern, &[Face]),
    direction: &str,
    angle_deg: u32,
) {
    let (cp, faces) = geometry;
    assert!(
        displayed.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        }),
        "{direction} {angle_deg}°: PBD後の全座標がfinite"
    );
    assert!(
        [
            report.total_depth_before,
            report.total_depth_after,
            report.candidate_total_depth,
            report.max_depth_before,
            report.max_depth_after,
            report.candidate_max_depth,
            report.candidate_max_relative_edge_error,
            report.candidate_max_face_planarity_error,
            report.candidate_max_seam_gap,
            report.target_gap,
        ]
        .into_iter()
        .all(f64::is_finite),
        "{direction} {angle_deg}°: Reportの全品質値がfinite: {report:?}"
    );
    assert!(
        !report.attempted || report.candidate_finite,
        "{direction} {angle_deg}°: 計算したPBD候補がfinite: {report:?}"
    );
    assert!(
        display_pairs.iter().all(|pair| raw_pairs.contains(pair)),
        "{direction} {angle_deg}°: 表示交差はraw交差のsubset: raw={raw_pairs:?} display={display_pairs:?}"
    );
    assert!(
        report.penetrations_after <= report.penetrations_before
            && report.total_depth_after <= report.total_depth_before
            && report.max_depth_after <= report.max_depth_before,
        "{direction} {angle_deg}°: signed penetrationを増やさない: {report:?}"
    );
    assert!(
        report.candidate_max_relative_edge_error <= 1e-6
            && report.candidate_max_face_planarity_error <= 1e-6
            && report.candidate_max_seam_gap <= 1e-6,
        "{direction} {angle_deg}°: PBD候補の剛性品質: {report:?}"
    );
    let display_seam = max_seam_gap(cp, faces, displayed);
    assert!(
        display_seam <= 1e-6,
        "{direction} {angle_deg}°: PBD表示seam={display_seam:e}"
    );
}

fn run_bird_base_sweep(
    doc: &Document,
    faces: &[Face],
    direction: &str,
    magnitudes: &[u32],
    sign: f64,
    targets: &HashMap<EdgeId, f64>,
    mut warm: HashMap<EdgeId, f64>,
) -> Vec<SweepSample> {
    let mut order: Vec<FaceId> = faces.iter().map(|face| face.id).collect();
    order.sort_unstable();
    let settings = OverlapSettings::default();
    let mut samples = Vec::with_capacity(magnitudes.len());

    for &angle_deg in magnitudes {
        let requested = sign * f64::from(angle_deg);
        let hard = [Driver {
            hinge: SWEEP_HINGE,
            target_angle_deg: requested,
        }];
        let started = Instant::now();
        let motion = solve_motion(&doc.cp, faces, &hard, Some(targets), Some(&warm), true);
        let solve_time = started.elapsed();
        assert!(
            solve_time < SOLVE_BUDGET,
            "{direction} {angle_deg}°: solve {solve_time:?} < {SOLVE_BUDGET:?}"
        );
        assert!(!motion.contact_stopped, "接触で操作を止めない");
        assert_finite_sweep_result(&motion.result, faces.len(), direction, angle_deg);
        let actual = motion
            .result
            .angles
            .get(&SWEEP_HINGE)
            .copied()
            .expect("代表hinge #24の実角");
        assert!(
            (actual - requested).abs() < 1e-9,
            "{direction} {angle_deg}°: hard誤差={}° angles={:?}",
            (actual - requested).abs(),
            motion.result.angles
        );

        let started = Instant::now();
        let raw_pairs = self_intersection_pairs(&motion.result.frame);
        let raw_time = started.elapsed();
        assert!(
            raw_time < CONTACT_DIAGNOSIS_BUDGET,
            "{direction} {angle_deg}°: raw診断 {raw_time:?} < {CONTACT_DIAGNOSIS_BUDGET:?}"
        );
        if !raw_pairs.is_empty() {
            let ordinary = solve_near(&doc.cp, faces, &hard, targets, Some(&warm));
            let ordinary_pairs = self_intersection_pairs(&ordinary.frame);
            let ordinary_metrics = contact_metrics(&ordinary.frame);
            let motion_metrics = contact_metrics(&motion.result.frame);
            panic!(
                "{direction} {angle_deg}°: motion最終(PBD前)={raw_pairs:?}, metrics(pair/max/total)={}/{:.9e}/{:.9e}; 直接solve_near通常解={ordinary_pairs:?}, metrics={}/{:.9e}/{:.9e}",
                motion_metrics.pair_count,
                motion_metrics.max_penetration,
                motion_metrics.total_penetration,
                ordinary_metrics.pair_count,
                ordinary_metrics.max_penetration,
                ordinary_metrics.total_penetration
            );
        }

        let started = Instant::now();
        let seam = max_seam_gap(&doc.cp, faces, &motion.result.frame);
        let seam_time = started.elapsed();
        if seam >= 1e-6 {
            let ordinary = solve_near(&doc.cp, faces, &hard, targets, Some(&warm));
            let ordinary_pairs = self_intersection_pairs(&ordinary.frame);
            let ordinary_metrics = contact_metrics(&ordinary.frame);
            let ordinary_seam = max_seam_gap(&doc.cp, faces, &ordinary.frame);
            let motion_metrics = contact_metrics(&motion.result.frame);
            panic!(
                "{direction} {angle_deg}°: motion最終(PBD前) seam={seam:.9e}, pairs={raw_pairs:?}, metrics(pair/max/total)={}/{:.9e}/{:.9e}; 直接solve_near通常解 seam={ordinary_seam:.9e}, pairs={ordinary_pairs:?}, metrics={}/{:.9e}/{:.9e}",
                motion_metrics.pair_count,
                motion_metrics.max_penetration,
                motion_metrics.total_penetration,
                ordinary_metrics.pair_count,
                ordinary_metrics.max_penetration,
                ordinary_metrics.total_penetration
            );
        }

        let mut displayed = motion.result.frame.clone();
        let started = Instant::now();
        let report = prevent_overlap(
            &doc.cp,
            faces,
            &mut displayed,
            &order,
            &order,
            0.5,
            &settings,
        );
        let pbd_time = started.elapsed();
        assert!(
            pbd_time < CONTACT_DIAGNOSIS_BUDGET,
            "{direction} {angle_deg}°: PBD {pbd_time:?} < {CONTACT_DIAGNOSIS_BUDGET:?}"
        );

        let started = Instant::now();
        let display_pairs = self_intersection_pairs(&displayed);
        let display_time = started.elapsed();
        assert!(
            display_time < CONTACT_DIAGNOSIS_BUDGET,
            "{direction} {angle_deg}°: 表示診断 {display_time:?} < {CONTACT_DIAGNOSIS_BUDGET:?}"
        );
        assert!(
            display_pairs.is_empty(),
            "{direction} {angle_deg}°: PBD後交差={display_pairs:?}, report={report:?}"
        );
        assert_eq!(
            report.intersection_pairs_before,
            raw_pairs.len(),
            "{direction} {angle_deg}°: Reportのraw組数"
        );
        assert_eq!(
            report.intersection_pairs_after,
            display_pairs.len(),
            "{direction} {angle_deg}°: Reportの表示組数"
        );
        assert_overlap_report_quality(
            &report,
            &raw_pairs,
            &display_pairs,
            &displayed,
            (&doc.cp, faces),
            direction,
            angle_deg,
        );

        warm = motion.result.angles;
        samples.push(SweepSample {
            angle_deg,
            raw_pairs: raw_pairs.len(),
            display_pairs: display_pairs.len(),
            solve_time,
            raw_time,
            pbd_time,
            display_time,
            seam_time,
        });
    }
    samples
}

fn p95_and_max(
    samples: &[SweepSample],
    select: impl Fn(&SweepSample) -> Duration,
) -> (Duration, Duration) {
    let mut values: Vec<Duration> = samples.iter().map(select).collect();
    values.sort_unstable();
    let index = (values.len() * 95).div_ceil(100).saturating_sub(1);
    (values[index], *values.last().expect("掃引サンプルあり"))
}

fn compact_pair_ranges(samples: &[SweepSample]) -> String {
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let mut end = start;
        while end + 1 < samples.len()
            && samples[end + 1].raw_pairs == samples[start].raw_pairs
            && samples[end + 1].display_pairs == samples[start].display_pairs
        {
            end += 1;
        }
        ranges.push(format!(
            "{}→{}°:{}/{}",
            samples[start].angle_deg,
            samples[end].angle_deg,
            samples[start].raw_pairs,
            samples[start].display_pairs
        ));
        start = end + 1;
    }
    ranges.join(", ")
}

fn print_sweep_summary(label: &str, samples: &[SweepSample]) {
    let solve = p95_and_max(samples, |sample| sample.solve_time);
    let raw = p95_and_max(samples, |sample| sample.raw_time);
    let pbd = p95_and_max(samples, |sample| sample.pbd_time);
    let display = p95_and_max(samples, |sample| sample.display_time);
    let seam = p95_and_max(samples, |sample| sample.seam_time);
    println!(
        "{label}: solve p95/max={:?}/{:?}, raw={:?}/{:?}, PBD={:?}/{:?}, display={:?}/{:?}, seam={:?}/{:?}; angle→raw/display [{}]",
        solve.0,
        solve.1,
        raw.0,
        raw.1,
        pbd.0,
        pbd.1,
        display.0,
        display.1,
        seam.0,
        seam.1,
        compact_pair_ranges(samples)
    );
}

fn bird_base_sweep_inputs() -> BirdBaseSweepInputs {
    let doc = bird_base();
    let faces = extract_faces(&doc.cp);
    assert_eq!(faces.len(), 14, "診断と同じ鶴の基本形");
    let completed = replay(&doc, doc.sequence.len(), 1.0);
    let completed_angle = completed
        .hinge_angles
        .get(&SWEEP_HINGE)
        .copied()
        .expect("鶴の代表hinge #24");
    assert!(
        (completed_angle.abs() - 180.0).abs() < 1e-6,
        "完成時の#24は±180°: {completed_angle}"
    );
    let sign = completed_angle.signum();
    let mut upward_targets: HashMap<EdgeId, f64> = completed
        .sequence_targets
        .iter()
        .filter(|driver| driver.hinge != SWEEP_HINGE)
        .map(|driver| (driver.hinge, 0.0))
        .collect();
    upward_targets.remove(&SWEEP_HINGE);
    let mut downward_targets: HashMap<EdgeId, f64> = completed
        .sequence_targets
        .iter()
        .filter(|driver| driver.hinge != SWEEP_HINGE)
        .map(|driver| (driver.hinge, driver.target_angle_deg))
        .collect();
    downward_targets.remove(&SWEEP_HINGE);
    let flat = solve(&doc.cp, &faces, &[], None);
    assert!(flat.angles.values().all(|angle| angle.abs() < 1e-12));
    BirdBaseSweepInputs {
        doc,
        faces,
        sign,
        upward_targets,
        downward_targets,
        flat_warm: flat.angles,
        completed_warm: completed.hinge_angles,
    }
}

#[test]
fn bird_base_bidirectional_one_degree_sweep_stays_non_intersecting() {
    let inputs = bird_base_sweep_inputs();
    let upward_angles: Vec<u32> = (0..=180).collect();
    let downward_angles: Vec<u32> = (0..=180).rev().collect();
    let upward = run_bird_base_sweep(
        &inputs.doc,
        &inputs.faces,
        "0→180",
        &upward_angles,
        inputs.sign,
        &inputs.upward_targets,
        inputs.flat_warm,
    );
    let downward = run_bird_base_sweep(
        &inputs.doc,
        &inputs.faces,
        "180→0",
        &downward_angles,
        inputs.sign,
        &inputs.downward_targets,
        inputs.completed_warm,
    );
    print_sweep_summary("鶴1°上昇", &upward);
    print_sweep_summary("鶴1°下降", &downward);
}

#[test]
fn bird_base_bidirectional_sixteen_degree_jumps_stay_non_intersecting() {
    let inputs = bird_base_sweep_inputs();
    let mut upward_angles: Vec<u32> = (0..=180).step_by(16).collect();
    if upward_angles.last() != Some(&180) {
        upward_angles.push(180);
    }
    let mut downward_angles: Vec<u32> = (0..=180).rev().step_by(16).collect();
    if downward_angles.last() != Some(&0) {
        downward_angles.push(0);
    }
    let upward = run_bird_base_sweep(
        &inputs.doc,
        &inputs.faces,
        "0→180（16°飛び）",
        &upward_angles,
        inputs.sign,
        &inputs.upward_targets,
        inputs.flat_warm,
    );
    let downward = run_bird_base_sweep(
        &inputs.doc,
        &inputs.faces,
        "180→0（16°飛び）",
        &downward_angles,
        inputs.sign,
        &inputs.downward_targets,
        inputs.completed_warm,
    );
    print_sweep_summary("鶴16°飛び上昇", &upward);
    print_sweep_summary("鶴16°飛び下降", &downward);
}

/// 折り鶴(鶴の基本形の細い先を首・尾・頭にし、羽を下げる)。
fn crane() -> Document {
    let mut doc = bird_base();
    let center = DVec2::new(0.0, CORE);
    let (down15, right15) = (-15.0_f64).to_radians().sin_cos();
    let points = layers_tipped_at(&doc, DVec2::ZERO);
    apply(
        &mut doc,
        inside_reverse,
        points[3..].to_vec(),
        [[center.x, center.y], [right15, center.y + down15]],
        [0.2, 0.5],
        None,
    );
    let tail = layers_tipped_at(&doc, DVec2::ZERO);
    apply(
        &mut doc,
        inside_reverse,
        tail,
        [[center.x, center.y], [right15, center.y - down15]],
        [-0.2, 0.5],
        None,
    );
    let up60 = DVec2::new(0.5, 0.75_f64.sqrt());
    let hinge = center + up60 * (0.75 * CORE);
    let head = layers_tipped_at(&doc, center + up60 * CORE);
    apply(
        &mut doc,
        inside_reverse,
        head,
        [[hinge.x, hinge.y], [hinge.x + right15, hinge.y - down15]],
        [hinge.x + 0.0866, hinge.y - 0.05],
        None,
    );
    for (quarter, direction) in [
        ([0.5, 0.0, 1.0, 0.5], FoldDirection::Down),
        ([0.0, 0.5, 0.5, 1.0], FoldDirection::Up),
    ] {
        let wing = layers_in_quarter(&doc, quarter);
        fold_layers(
            &mut doc,
            [[-1.0, 0.6], [1.0, 0.6]],
            [0.0, 0.3],
            Some(wing),
            direction,
        );
    }
    doc
}

fn settings(pressure: f64) -> SoftSettings {
    SoftSettings {
        enabled: true,
        subdivision: 2,
        stiffness: 0.5,
        pressure,
        iterations: 20,
    }
}

/// 網全体の厚み(平らに畳んだ面に垂直な向きの広がり)。
fn thickness(m: &SoftMesh) -> f64 {
    let zs: Vec<f64> = m.positions.iter().map(|p| p[2]).collect();
    zs.iter().copied().fold(f64::MIN, f64::max) - zs.iter().copied().fold(f64::MAX, f64::min)
}

#[test]
fn the_body_of_the_crane_puffs_up() {
    let doc = crane();
    let faces = extract_faces(&doc.cp);
    let frame = replay(&doc, doc.sequence.len(), 1.0).frame;
    let flat = relax(&doc.cp, &faces, &frame, &settings(0.0));
    let puffy = relax(&doc.cp, &faces, &frame, &settings(0.9));
    assert!(
        thickness(&flat) < 0.05,
        "膨らませなければ平らなまま: {}",
        thickness(&flat)
    );
    assert!(
        thickness(&puffy) > thickness(&flat) + 0.1,
        "胴が膨らんで厚みが出る: {} → {}",
        thickness(&flat),
        thickness(&puffy)
    );
    assert!(puffy.warnings.is_empty(), "警告なし: {:?}", puffy.warnings);
    // 紙が伸びないこと。鶴には非常に短い辺(細い先)があり、そこでは割合の
    // 変化が大きく出るので、紙の一辺=1.0に対する長さの変化そのもので見る。
    let len = |m: &SoftMesh, t: &[u32; 3]| {
        let p = |i: u32| glam::DVec3::from(m.positions[i as usize]);
        (p(t[1]) - p(t[0])).length()
    };
    let worst = puffy
        .triangles
        .iter()
        .zip(&flat.triangles)
        .map(|(a, b)| (len(&puffy, a) - len(&flat, b)).abs())
        .fold(0.0, f64::max);
    assert!(
        worst < 0.02,
        "紙は伸びない: 辺の長さの変化 最大{worst}(紙の一辺=1.0)"
    );
}

/// 風船(水風船)の基本形。予備基本形と同じ折り順を**対角線**で行ったもの
/// (山谷が逆=予備基本形を裏返した形)。4層が輪につながった袋になる。
fn waterbomb_base() -> Document {
    let mut doc = square_doc();
    fold(
        &mut doc,
        [[0.0, 0.0], [1.0, 1.0]],
        [0.75, 0.25],
        FoldDirection::Up,
    );
    fold(
        &mut doc,
        [[1.0, 0.0], [0.5, 0.5]],
        [0.9, 0.6],
        FoldDirection::Up,
    );
    for (line, reference) in [
        ([[0.0, 0.0], [1.0, 1.0]], [0.9, 0.9]),
        ([[1.0, 0.0], [0.5, 0.5]], [0.95, 0.05]),
    ] {
        let (_, state) = state_of(&doc);
        let bottom = vec![state.order[0]];
        apply(&mut doc, squash, bottom, line, reference, None);
    }
    doc
}

/// 網の広がり(x, y, z それぞれの幅)。
fn extents(m: &SoftMesh) -> [f64; 3] {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in &m.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
}

#[test]
fn a_waterbomb_base_becomes_round_when_inflated() {
    let doc = waterbomb_base();
    let faces = extract_faces(&doc.cp);
    let frame = replay(&doc, doc.sequence.len(), 1.0).frame;
    let mut layers: Vec<u32> = frame.faces.iter().map(|f| f.layer).collect();
    layers.sort_unstable();
    layers.dedup();
    assert_eq!(layers, vec![0, 1, 2, 3], "4層の袋になっている: {layers:?}");

    let flat = relax(&doc.cp, &faces, &frame, &settings(0.0));
    // 空気を入れ続けた形を見るので反復は多め(既定の20回では膨らみ切らない)
    let mut blow = settings(0.9);
    blow.iterations = 120;
    let puffy = relax(&doc.cp, &faces, &frame, &blow);
    assert!(
        thickness(&flat) < 0.01,
        "折り上がりは平ら: {}",
        thickness(&flat)
    );
    let e = extents(&puffy);
    // 平らな紙のままなら厚みは幅に比べてごく薄い。丸くなったかどうかを
    // 「厚みが幅の何割か」で見る(風船なので3方向の大きさが近づく)。
    let round = e[2] / e[0].max(e[1]);
    assert!(
        round > 0.5,
        "平たい紙ではなく丸くなる: 広がり{e:?} 丸み{round}"
    );
    // 本物の風船と同じように、膨らむぶん紙が引き寄せられて横幅は縮む
    assert!(
        e[0] < extents(&flat)[0],
        "膨らむと横幅は縮む: {} → {}",
        extents(&flat)[0],
        e[0]
    );
    assert!(puffy.warnings.is_empty(), "警告なし: {:?}", puffy.warnings);
}
