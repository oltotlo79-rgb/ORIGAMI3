//! 水風船基本形のSIM-015受入検査。
//!
//! 水風船は対角線から畳む4層の袋で、`ori3-soft/tests/soft_crane.rs` の
//! `waterbomb_base` と同じ、追跡済みの折り順をここで読み書きなしに構築する。

use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{extract_faces, Face};
use ori3_layers::fold_through::{fold_through, FoldDirection, FoldThroughInput, FoldThroughResult};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{flat_state_at, replay, squash, FlatState};
use ori3_model::{
    CreasePattern, Document, Face3D, FaceId, FinishSoftSettings, FoldStep, Frame3D, Paper,
    TechniqueKind,
};
use ori3_rigid::{max_seam_gap, self_intersection_pairs};

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<FoldThroughResult, String>;

fn square_document() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

fn state_of(document: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&document.cp);
    let (state, warnings) =
        flat_state_at(document, &faces, document.sequence.len()).expect("水風船を平坦に再生できる");
    assert!(warnings.is_empty(), "水風船の再生警告なし: {warnings:?}");
    (faces, state)
}

fn fold(document: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2]) {
    let (faces, state) = state_of(document);
    let mut cp = document.cp.clone();
    let result = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line,
            keep_side_point: keep,
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("水風船の下ごしらえを折れる");
    assert!(
        result.warnings.is_empty(),
        "水風船の下ごしらえに警告なし: {:?}",
        result.warnings
    );
    let mut step = result.step;
    step.id = u32::try_from(document.sequence.len()).expect("手順数はu32に収まる");
    document.cp = cp;
    document.sequence.push(step);
}

fn apply(
    document: &mut Document,
    technique: Technique,
    flap: Vec<FaceId>,
    line: [[f64; 2]; 2],
    reference_point: [f64; 2],
) {
    let (faces, state) = state_of(document);
    let mut cp = document.cp.clone();
    let result = technique(
        &mut cp,
        &faces,
        &state,
        &TechniqueInput {
            flap,
            line,
            reference_point,
            open_to_back: None,
            polygon: None,
            center: None,
        },
    )
    .expect("水風船の袋をつぶせる");
    assert!(
        result.warnings.is_empty(),
        "水風船のつぶし折りに警告なし: {:?}",
        result.warnings
    );
    let mut step = result.step;
    step.id = u32::try_from(document.sequence.len()).expect("手順数はu32に収まる");
    document.cp = cp;
    document.sequence.push(step);
}

/// 水風船基本形。対角線で畳んだ4層を2回つぶし、4層が輪につながる袋を作る。
fn waterbomb_base() -> Document {
    let mut document = square_document();
    fold(&mut document, [[0.0, 0.0], [1.0, 1.0]], [0.75, 0.25]);
    fold(&mut document, [[1.0, 0.0], [0.5, 0.5]], [0.9, 0.6]);
    for (line, reference) in [
        ([[0.0, 0.0], [1.0, 1.0]], [0.9, 0.9]),
        ([[1.0, 0.0], [0.5, 0.5]], [0.95, 0.05]),
    ] {
        let (_, state) = state_of(&document);
        apply(&mut document, squash, vec![state.order[0]], line, reference);
    }
    document
}

fn explicit_flat_frame(document: &Document, faces: &[Face], state: &FlatState) -> Frame3D {
    let positions = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    Frame3D {
        faces: faces
            .iter()
            .map(|face| {
                let rank = state
                    .order
                    .iter()
                    .position(|id| *id == face.id)
                    .expect("全ての面が層順序にある");
                Face3D {
                    face: face.id,
                    polygon: face
                        .vertices
                        .iter()
                        .map(|vertex| {
                            let point = state.placements[&face.id].apply(positions[vertex]);
                            [point.x, point.y, 0.0]
                        })
                        .collect(),
                    layer: u32::try_from(rank).expect("層順序はu32に収まる"),
                    surface_rank: u32::try_from(rank).expect("層順序はu32に収まる"),
                    mirrored: state.placements[&face.id].mirrored,
                }
            })
            .collect(),
        warnings: Vec::new(),
    }
}

fn finished_replay_coordinates(
    mut document: Document,
    enabled: bool,
    label: &str,
) -> Vec<(FaceId, [f64; 3])> {
    // 3回測定の最大gapは0。明示した平坦層の組立てで出る丸めだけを許すため、
    // モデル共通EPS(1e-9)を境界にする。可視の裂け(1e-6)より十分小さい。
    const FLAT_GAP_TOLERANCE: f64 = 1e-9;
    let settings = FinishSoftSettings {
        enabled,
        stiffness: 0.52,
        pressure: 0.41,
    };
    let step_id = u32::try_from(document.sequence.len()).expect("手順数はu32に収まる");
    document.sequence.push(FoldStep {
        id: step_id,
        kind: TechniqueKind::Pose,
        drivers: Vec::new(),
        layer_order: None,
        alignment: None,
        finish_soft: Some(settings),
        note: "SIM-015仕上げ確定".to_string(),
    });
    let up_to = document.sequence.len();
    assert_eq!(
        document.finish_soft_at(up_to, 1.0),
        Some(settings),
        "{label}: 完了位置で記録した仕上げ値を選ぶ"
    );

    let faces = extract_faces(&document.cp);
    let (flat, flat_warnings) =
        flat_state_at(&document, &faces, up_to).expect("仕上げPoseまで平坦に再生できる");
    assert!(
        flat_warnings.is_empty(),
        "{label}: 明示平坦層の再生警告なし: {flat_warnings:?}"
    );
    let flat_frame = explicit_flat_frame(&document, &faces, &flat);
    assert!(
        self_intersection_pairs(&flat_frame).is_empty(),
        "{label}: 明示した平坦層にすり抜けはない"
    );
    let flat_gap = max_seam_gap(&document.cp, &faces, &flat_frame);
    assert!(
        flat_gap < FLAT_GAP_TOLERANCE,
        "{label}: 明示した平坦層はつながる (gap={flat_gap:.3e})"
    );

    let replayed = replay(&document, up_to, 1.0);
    assert!(
        replayed.warnings.is_empty() && replayed.skipped.is_empty(),
        "{label}: 仕上げ{}で完全に再生する: warnings={:?}, skipped={:?}",
        if enabled { "on" } else { "off" },
        replayed.warnings,
        replayed.skipped
    );

    replayed
        .frame
        .faces
        .iter()
        .flat_map(|face| {
            face.polygon.iter().map(move |point| {
                assert!(
                    point.iter().all(|coordinate| coordinate.is_finite()),
                    "{label}: 再生座標は有限"
                );
                (face.face, *point)
            })
        })
        .collect()
}

/// SIM-015: 水風船基本形は仕上げon/offの双方で再生できる。3回の位置差は実測0
/// なので、丸めだけを許す1e-12を上限にした。これは平坦層の判定EPS(1e-9)の
/// 1/1000で、見える位置ずれを許容しない。
#[test]
fn balloon_replays_with_finish_soft_on_and_off_three_times_without_penetration() {
    const POSITION_TOLERANCE: f64 = 1e-12;

    let mut configurations_replayed = 0usize;
    let mut observed_max_delta = 0.0_f64;
    for enabled in [false, true] {
        let mut baseline: Option<Vec<(FaceId, [f64; 3])>> = None;
        for run in 1..=3 {
            let label = format!(
                "水風船/仕上げ{}/{}回目",
                if enabled { "on" } else { "off" },
                run
            );
            let coordinates = finished_replay_coordinates(waterbomb_base(), enabled, &label);
            if let Some(reference) = &baseline {
                assert_eq!(
                    coordinates.len(),
                    reference.len(),
                    "{label}: 再生した頂点数は基準と一致する"
                );
                for ((face, point), (reference_face, reference_point)) in
                    coordinates.iter().zip(reference)
                {
                    assert_eq!(face, reference_face, "{label}: 面IDは基準と一致する");
                    let delta = point
                        .iter()
                        .zip(reference_point)
                        .map(|(left, right)| (left - right).abs())
                        .fold(0.0_f64, f64::max);
                    observed_max_delta = observed_max_delta.max(delta);
                    assert!(
                        delta <= POSITION_TOLERANCE,
                        "{label}: 位置差 {delta:.3e} は許容値 {POSITION_TOLERANCE:.3e} 以下"
                    );
                }
            } else {
                baseline = Some(coordinates);
            }
        }
        configurations_replayed += 1;
    }
    assert_eq!(
        configurations_replayed, 2,
        "仕上げon/offの2/2で水風船を再生する"
    );
    assert!(
        observed_max_delta <= POSITION_TOLERANCE,
        "3回実測の最大位置差 {observed_max_delta:.3e} は許容値以下"
    );
}
