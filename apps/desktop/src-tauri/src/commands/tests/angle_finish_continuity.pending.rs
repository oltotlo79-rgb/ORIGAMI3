//! 角度ジェスチャーの Follow → canonical 確定境界を実 solver で測る未登録検査。
//!
//! hinge 19 の実操作順における座標成分差と、全6操作順×3確定境界の
//! wrapped希望角誤差を、製品command本体を通して固定する。両方greenになるまで
//! 通常suiteへ登録しない。

use super::super::{
    DocumentStore, PoseOutcome, PoseSolveInput, PoseSolveMode, canonical_document_seed,
    pose_motion_contact_options, pose_solve_core, pose_solve_core_with_mode,
};
use ori3_model::{Driver, EdgeId, FaceId, Frame3D};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const FINISH_HINGE: EdgeId = 19;
/// sa実製品順の実測0.34822へ約15%の余裕を取る。旧上限0.637774より十分小さく、
/// 数値揺らぎの余裕は0.05178なので、回帰を隠すほど広げない。
const MAX_FINISH_COORDINATE_JUMP: f64 = 0.4;
const DESIRED_ERROR_EPSILON_DEG: f64 = 1.0e-9;

#[derive(Debug)]
struct LargestCoordinateJump {
    delta_abs: f64,
    face: FaceId,
    polygon_vertex: usize,
    coordinate: usize,
    follow: [f64; 3],
    canonical: [f64; 3],
}

struct FinishBoundary {
    follow: PoseOutcome,
    canonical: PoseOutcome,
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("crates/ori3-rigid/tests/fixtures/sa-warm-path.ori3")
}

fn fresh_store() -> Mutex<DocumentStore> {
    let store = Mutex::new(DocumentStore::default());
    store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .open(&fixture_path())
        .expect("リポジトリ内の sa fixture を開けるはず");
    store
}

fn driver(hinge: EdgeId, target_angle_deg: f64) -> Driver {
    Driver {
        hinge,
        target_angle_deg,
    }
}

fn desired_angle(hinge: EdgeId) -> f64 {
    match hinge {
        17 => -90.0,
        19 | 21 => 90.0,
        _ => panic!("検査対象でない hinge {hinge}"),
    }
}

fn angle_rows(outcome: &PoseOutcome) -> Vec<(EdgeId, f64)> {
    let mut rows: Vec<_> = outcome
        .result
        .angles
        .iter()
        .map(|(&hinge, &angle)| (hinge, angle))
        .collect();
    rows.sort_unstable_by_key(|&(hinge, _)| hinge);
    rows
}

fn rank_mirror_rows(outcome: &PoseOutcome) -> Vec<(FaceId, u32, bool)> {
    let mut rows: Vec<_> = outcome
        .result
        .frame
        .faces
        .iter()
        .map(|face| (face.face, face.surface_rank, face.mirrored))
        .collect();
    rows.sort_unstable_by_key(|&(face, _, _)| face);
    rows
}

fn assert_command_matches_direct_motion(
    command: &PoseOutcome,
    direct: &ori3_rigid::SolveResult,
    stage: &str,
) {
    let mut command_angles: Vec<_> = command.result.angles.iter().collect();
    let mut direct_angles: Vec<_> = direct.angles.iter().collect();
    command_angles.sort_unstable_by_key(|(hinge, _)| **hinge);
    direct_angles.sort_unstable_by_key(|(hinge, _)| **hinge);
    assert_eq!(
        command_angles.len(),
        direct_angles.len(),
        "{stage}: angle本数"
    );
    for ((command_hinge, command_angle), (direct_hinge, direct_angle)) in
        command_angles.into_iter().zip(direct_angles)
    {
        assert_eq!(command_hinge, direct_hinge, "{stage}: angle hinge");
        assert_eq!(
            command_angle.to_bits(),
            direct_angle.to_bits(),
            "{stage}: hinge {command_hinge} angle"
        );
    }

    let mut command_faces: Vec<_> = command.result.frame.faces.iter().collect();
    let mut direct_faces: Vec<_> = direct.frame.faces.iter().collect();
    command_faces.sort_unstable_by_key(|face| face.face);
    direct_faces.sort_unstable_by_key(|face| face.face);
    assert_eq!(command_faces.len(), direct_faces.len(), "{stage}: face本数");
    for (command_face, direct_face) in command_faces.into_iter().zip(direct_faces) {
        assert_eq!(command_face.face, direct_face.face, "{stage}: face ID");
        assert_eq!(
            command_face.polygon.len(),
            direct_face.polygon.len(),
            "{stage}: face {} polygon本数",
            command_face.face
        );
        for (command_point, direct_point) in command_face.polygon.iter().zip(&direct_face.polygon) {
            assert_eq!(
                command_point.map(f64::to_bits),
                direct_point.map(f64::to_bits),
                "{stage}: face {} polygon座標",
                command_face.face
            );
        }
        assert_eq!(command_face.layer, direct_face.layer, "{stage}: layer");
        assert_eq!(
            command_face.surface_rank, direct_face.surface_rank,
            "{stage}: surface_rank"
        );
        assert_eq!(
            command_face.mirrored, direct_face.mirrored,
            "{stage}: mirrored"
        );
    }
}

fn direct_follow(
    store: &Mutex<DocumentStore>,
    desired_before: &[Driver],
    next: Driver,
) -> ori3_rigid::MotionSolveResult {
    let (doc, faces, stored_warm, overlap_enabled, penetration_enabled) = store
        .lock()
        .unwrap_or_else(|state| state.into_inner())
        .pose_inputs();
    let targets: HashMap<_, _> = desired_before
        .iter()
        .filter(|item| item.hinge != next.hinge)
        .map(|item| (item.hinge, item.target_angle_deg))
        .collect();
    ori3_rigid::solve_motion_with_contact_options(
        &doc.cp,
        &faces,
        &[next],
        (!targets.is_empty()).then_some(&targets),
        stored_warm.as_ref(),
        pose_motion_contact_options(overlap_enabled, penetration_enabled),
    )
}

fn direct_canonical(
    store: &Mutex<DocumentStore>,
    desired: &[Driver],
) -> ori3_rigid::MotionSolveResult {
    let (doc, faces, _, overlap_enabled, penetration_enabled) = store
        .lock()
        .unwrap_or_else(|state| state.into_inner())
        .pose_inputs();
    let targets: HashMap<_, _> = desired
        .iter()
        .map(|item| (item.hinge, item.target_angle_deg))
        .collect();
    let document_seed = canonical_document_seed(&doc, &faces, 0, 1.0);
    ori3_rigid::motion::solve_canonical_motion_with_contact_options(
        &doc.cp,
        &faces,
        &[],
        Some(&targets),
        Some(&document_seed),
        pose_motion_contact_options(overlap_enabled, penetration_enabled),
    )
}

fn solve(
    store: &Mutex<DocumentStore>,
    hard: Vec<Driver>,
    preferred: Vec<Driver>,
    warm_seed: Option<Vec<Driver>>,
) -> PoseOutcome {
    pose_solve_core(
        store,
        hard,
        (!preferred.is_empty()).then_some(preferred),
        None,
        warm_seed,
        0,
        1.0,
    )
    .expect("sa の通常角度操作は有限の応答を返すはず")
}

/// Document と希望角だけに由来する、frontend の finish payload。
/// 旧frontend相当の明示seedも添えるが、Canonical modeは候補生成へ使わない。
fn canonical_solve(store: &Mutex<DocumentStore>, mut desired: Vec<Driver>) -> PoseOutcome {
    desired.sort_unstable_by_key(|item| item.hinge);
    let seed = (17..=34)
        .map(|hinge| {
            driver(
                hinge,
                desired
                    .iter()
                    .find(|item| item.hinge == hinge)
                    .map_or(0.0, |item| item.target_angle_deg),
            )
        })
        .collect();
    pose_solve_core_with_mode(
        store,
        PoseSolveInput {
            hard: Vec::new(),
            preferred: Some(desired),
            soft: None,
            warm_seed: Some(seed),
            up_to: 0,
            t: 1.0,
            mode: PoseSolveMode::Canonical,
        },
    )
    .expect("sa のcanonical確定は有限の応答を返すはず")
}

/// UI と同じく、いま触る1本だけ hard、以前の希望角を preferred にする。
fn follow_solve(
    store: &Mutex<DocumentStore>,
    desired_before: &[Driver],
    next: Driver,
) -> PoseOutcome {
    let preferred = desired_before
        .iter()
        .filter(|item| item.hinge != next.hinge)
        .cloned()
        .collect();
    solve(store, vec![next], preferred, None)
}

fn finish_gesture(
    store: &Mutex<DocumentStore>,
    desired_before: &mut Vec<Driver>,
    next: Driver,
) -> (PoseOutcome, PoseOutcome) {
    let follow = follow_solve(store, desired_before, next.clone());
    if let Some(existing) = desired_before
        .iter_mut()
        .find(|item| item.hinge == next.hinge)
    {
        *existing = next;
    } else {
        desired_before.push(next);
    }
    desired_before.sort_unstable_by_key(|item| item.hinge);
    let canonical = canonical_solve(store, desired_before.clone());
    (follow, canonical)
}

/// 17、21 は Follow と finish の両方を通し、実UIと同じ保存warmにしたうえで、
/// 最後の hinge 19 の pointer-up 境界だけを返す。
fn hinge_19_finish_boundary() -> FinishBoundary {
    let store = fresh_store();
    let (_, faces, _, overlap_enabled, penetration_enabled) = store
        .lock()
        .unwrap_or_else(|state| state.into_inner())
        .pose_inputs();
    eprintln!(
        "ANGLE_FINISH_INPUT faces={} overlap_enabled={overlap_enabled} \
         penetration_enabled={penetration_enabled}",
        faces.len()
    );

    let mut desired = Vec::new();
    let direct_follow_17 = direct_follow(&store, &desired, driver(17, -90.0));
    let (follow_17, canonical_17) = finish_gesture(&store, &mut desired, driver(17, -90.0));
    assert_command_matches_direct_motion(&follow_17, &direct_follow_17.result, "follow17");
    let direct_canonical_17 = direct_canonical(&store, &desired);
    assert_command_matches_direct_motion(&canonical_17, &direct_canonical_17.result, "canonical17");
    eprintln!(
        "ANGLE_FINISH_STAGE follow17={:?} canonical17={:?}",
        angle_rows(&follow_17),
        angle_rows(&canonical_17)
    );

    let direct_follow_21 = direct_follow(&store, &desired, driver(21, 90.0));
    let (follow_21, canonical_21) = finish_gesture(&store, &mut desired, driver(21, 90.0));
    assert_command_matches_direct_motion(&follow_21, &direct_follow_21.result, "follow21");
    let direct_canonical_21 = direct_canonical(&store, &desired);
    assert_command_matches_direct_motion(&canonical_21, &direct_canonical_21.result, "canonical21");
    eprintln!(
        "ANGLE_FINISH_STAGE follow21={:?} canonical21={:?}",
        angle_rows(&follow_21),
        angle_rows(&canonical_21)
    );

    let stored_after_21 = store
        .lock()
        .unwrap_or_else(|state| state.into_inner())
        .pose_inputs()
        .2
        .expect("canonical21を次のFollow warmへ保存する");
    let mut stored_after_21_rows: Vec<_> = stored_after_21.iter().collect();
    let mut canonical_21_rows: Vec<_> = canonical_21.result.angles.iter().collect();
    stored_after_21_rows.sort_unstable_by_key(|(hinge, _)| **hinge);
    canonical_21_rows.sort_unstable_by_key(|(hinge, _)| **hinge);
    assert_eq!(stored_after_21_rows.len(), canonical_21_rows.len());
    for ((stored_hinge, stored_angle), (canonical_hinge, canonical_angle)) in
        stored_after_21_rows.into_iter().zip(canonical_21_rows)
    {
        assert_eq!(stored_hinge, canonical_hinge);
        assert_eq!(
            stored_angle.to_bits(),
            canonical_angle.to_bits(),
            "canonical21 cacheのhinge {stored_hinge}が表示actualと違う"
        );
    }
    eprintln!("ANGLE_FINISH_STORED_AFTER_21 bits_equal=true");

    let direct_follow_19 = direct_follow(&store, &desired, driver(FINISH_HINGE, 90.0));
    let (follow, canonical) = finish_gesture(&store, &mut desired, driver(FINISH_HINGE, 90.0));
    assert_command_matches_direct_motion(&follow, &direct_follow_19.result, "follow19");
    let direct_canonical_19 = direct_canonical(&store, &desired);
    assert_command_matches_direct_motion(&canonical, &direct_canonical_19.result, "canonical19");
    assert!(
        desired.iter().any(|item| item.hinge == FINISH_HINGE),
        "検査payloadにhinge 19を必ず含める"
    );
    eprintln!(
        "ANGLE_FINISH_STAGE follow19={:?} canonical19={:?} \
         canonical_rank_mirror={:?}",
        angle_rows(&follow),
        angle_rows(&canonical),
        rank_mirror_rows(&canonical),
    );
    FinishBoundary { follow, canonical }
}

fn largest_coordinate_jump(follow: &Frame3D, canonical: &Frame3D) -> LargestCoordinateJump {
    assert_eq!(
        follow.faces.len(),
        canonical.faces.len(),
        "finish前後でface本数が変わった"
    );
    let mut largest = LargestCoordinateJump {
        delta_abs: 0.0,
        face: 0,
        polygon_vertex: 0,
        coordinate: 0,
        follow: [0.0; 3],
        canonical: [0.0; 3],
    };
    for before_face in &follow.faces {
        let after_face = canonical
            .faces
            .iter()
            .find(|face| face.face == before_face.face)
            .unwrap_or_else(|| panic!("canonical frameにface {}が無い", before_face.face));
        assert_eq!(
            before_face.polygon.len(),
            after_face.polygon.len(),
            "face {}のpolygon頂点数がfinish前後で変わった",
            before_face.face
        );
        for (polygon_vertex, (&before, &after)) in before_face
            .polygon
            .iter()
            .zip(&after_face.polygon)
            .enumerate()
        {
            for coordinate in 0..3 {
                let delta_abs = (before[coordinate] - after[coordinate]).abs();
                assert!(
                    delta_abs.is_finite(),
                    "finish前後の座標成分差は有限であること"
                );
                if delta_abs > largest.delta_abs {
                    largest = LargestCoordinateJump {
                        delta_abs,
                        face: before_face.face,
                        polygon_vertex,
                        coordinate,
                        follow: before,
                        canonical: after,
                    };
                }
            }
        }
    }
    largest
}

fn wrapped_angle_error(actual: f64, target: f64) -> f64 {
    let positive = (actual - target).rem_euclid(360.0);
    positive.min(360.0 - positive)
}

fn maximum_desired_angle_error(outcome: &PoseOutcome, desired: &[Driver]) -> f64 {
    desired.iter().fold(0.0_f64, |maximum, item| {
        let actual = outcome
            .result
            .angles
            .get(&item.hinge)
            .unwrap_or_else(|| panic!("実角にhinge {}が無い", item.hinge));
        let error = wrapped_angle_error(*actual, item.target_angle_deg);
        assert!(error.is_finite(), "希望角誤差は有限であること");
        maximum.max(error)
    })
}

fn desired_actual_rows(outcome: &PoseOutcome, desired: &[Driver]) -> Vec<(EdgeId, f64, f64, f64)> {
    desired
        .iter()
        .map(|item| {
            let actual = *outcome
                .result
                .angles
                .get(&item.hinge)
                .unwrap_or_else(|| panic!("実角にhinge {}が無い", item.hinge));
            (
                item.hinge,
                item.target_angle_deg,
                actual,
                wrapped_angle_error(actual, item.target_angle_deg),
            )
        })
        .collect()
}

fn all_six_orders() -> [[EdgeId; 3]; 6] {
    [
        [17, 19, 21],
        [17, 21, 19],
        [19, 17, 21],
        [19, 21, 17],
        [21, 17, 19],
        [21, 19, 17],
    ]
}

#[test]
fn hinge_19_finish_keeps_maximum_coordinate_jump_below_previous_branch_gap() {
    let boundary = hinge_19_finish_boundary();
    let largest = largest_coordinate_jump(
        &boundary.follow.result.frame,
        &boundary.canonical.result.frame,
    );
    eprintln!(
        "ANGLE_FINISH_COORDINATE_JUMP hinge={FINISH_HINGE} max={:.17e} detail={largest:?}",
        largest.delta_abs
    );
    assert!(
        largest.delta_abs < MAX_FINISH_COORDINATE_JUMP,
        "hinge {FINISH_HINGE} のpointer-upで座標成分の最大移動が上限以上になった: \
         max={:.17e}, limit={MAX_FINISH_COORDINATE_JUMP:.17e}, face={}, polygon_vertex={}, \
         coordinate={}, follow={:?}, canonical={:?}",
        largest.delta_abs,
        largest.face,
        largest.polygon_vertex,
        largest.coordinate,
        largest.follow,
        largest.canonical,
    );
}

#[test]
fn all_six_paths_and_eighteen_finish_boundaries_do_not_worsen_wrapped_desired_error() {
    let mut measured_boundaries = 0_usize;
    for order in all_six_orders() {
        let store = fresh_store();
        let mut desired = Vec::new();
        for (gesture_index, hinge) in order.into_iter().enumerate() {
            let (follow, canonical) =
                finish_gesture(&store, &mut desired, driver(hinge, desired_angle(hinge)));
            let follow_error = maximum_desired_angle_error(&follow, &desired);
            let canonical_error = maximum_desired_angle_error(&canonical, &desired);
            let follow_rows = desired_actual_rows(&follow, &desired);
            let canonical_rows = desired_actual_rows(&canonical, &desired);
            eprintln!(
                "ANGLE_FINISH_DESIRED_ERROR order={order:?} gesture={} hinge={hinge} \
                 follow={follow_error:.17e} canonical={canonical_error:.17e} \
                 follow_rows={follow_rows:?} canonical_rows={canonical_rows:?}",
                gesture_index + 1,
            );
            assert!(
                canonical_error <= follow_error + DESIRED_ERROR_EPSILON_DEG,
                "操作順{order:?}の{}回目(hinge {hinge})のpointer-upで希望角への最大誤差が悪化した: \
                 follow={follow_error:.17e}, canonical={canonical_error:.17e}, \
                 epsilon={DESIRED_ERROR_EPSILON_DEG:.17e}",
                gesture_index + 1,
            );
            measured_boundaries += 1;
        }
    }
    assert_eq!(measured_boundaries, 6 * 3, "全6順×3境界を測る");
}
