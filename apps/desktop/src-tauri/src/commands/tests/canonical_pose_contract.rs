//! B の canonical 導出契約。段階1で赤を確認し、案1で緑になったため通常suiteへ登録する。
//!
//! ```text
//! #[path = "canonical_pose_contract.rs"]
//! mod canonical_pose_contract;
//! ```
//!
//! `ignore` や条件付き return は使わず、全検査が通常の `#[test]` として必ず走る。
//! fixture はリポジトリ内のコピーだけを読み、OneDrive は読まない。

use super::super::{
    DocumentStore, PoseOutcome, PoseSolveInput, PoseSolveMode, canonical_document_seed,
    pose_motion_contact_options, pose_solve_core, pose_solve_core_with_mode,
};
use ori3_model::{Driver, EdgeId, EdgeKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, PartialEq, Eq)]
struct FaceSignature {
    face: u32,
    polygon_bits: Vec<[u64; 3]>,
    layer: u32,
    surface_rank: u32,
    mirrored: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct DerivedSignature {
    angle_bits: Vec<(EdgeId, u64)>,
    faces: Vec<FaceSignature>,
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
    .expect("sa の通常姿勢は有限の応答を返すはず")
}

fn result_signature(result: &ori3_rigid::SolveResult) -> DerivedSignature {
    let mut angle_bits: Vec<_> = result
        .angles
        .iter()
        .map(|(&hinge, &angle)| (hinge, angle.to_bits()))
        .collect();
    angle_bits.sort_unstable_by_key(|&(hinge, _)| hinge);
    let mut faces: Vec<_> = result
        .frame
        .faces
        .iter()
        .map(|face| FaceSignature {
            face: face.face,
            polygon_bits: face
                .polygon
                .iter()
                .map(|point| point.map(f64::to_bits))
                .collect(),
            layer: face.layer,
            surface_rank: face.surface_rank,
            mirrored: face.mirrored,
        })
        .collect();
    faces.sort_unstable_by_key(|face| face.face);
    DerivedSignature { angle_bits, faces }
}

fn signature(outcome: &PoseOutcome) -> DerivedSignature {
    result_signature(&outcome.result)
}

fn desired_angle(hinge: EdgeId) -> f64 {
    match hinge {
        17 => -90.0,
        19 | 21 => 90.0,
        _ => panic!("検査対象でない hinge {hinge}"),
    }
}

fn canonical_solve_with_warm(
    store: &Mutex<DocumentStore>,
    mut desired: Vec<Driver>,
    warm_seed: Option<Vec<Driver>>,
) -> PoseOutcome {
    desired.sort_unstable_by_key(|item| item.hinge);
    pose_solve_core_with_mode(
        store,
        PoseSolveInput {
            hard: Vec::new(),
            preferred: Some(desired),
            soft: None,
            warm_seed,
            up_to: 0,
            t: 1.0,
            mode: PoseSolveMode::Canonical,
        },
    )
    .expect("sa のcanonical姿勢は有限の応答を返すはず")
}

/// 確定modeはDocumentと希望値から再導出する。明示seedも無視される契約を
/// 検査で通すため、旧frontend相当のseedを意図的に添えて呼ぶ。
fn canonical_solve(store: &Mutex<DocumentStore>, desired: Vec<Driver>) -> PoseOutcome {
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
    canonical_solve_with_warm(store, desired, Some(seed))
}

/// UI と同じく、いま触る1本だけを hard、既に指定したものを preferred にする。
fn follow_path(store: &Mutex<DocumentStore>, order: [EdgeId; 3]) -> PoseOutcome {
    let mut desired = Vec::<Driver>::new();
    let mut last = None;
    for hinge in order {
        let hard = driver(hinge, desired_angle(hinge));
        let preferred = desired
            .iter()
            .filter(|item| item.hinge != hinge)
            .cloned()
            .collect();
        last = Some(solve(store, vec![hard.clone()], preferred, None));
        if let Some(existing) = desired.iter_mut().find(|item| item.hinge == hinge) {
            *existing = hard;
        } else {
            desired.push(hard);
            desired.sort_unstable_by_key(|item| item.hinge);
        }
    }
    last.expect("3操作なので応答があるはず")
}

/// 全経路の末尾へ同じcanonical payloadを送り、最後に触ったhingeと
/// DocumentStoreの暗黙cacheを比較入力から除く。
fn settle_same_desired(store: &Mutex<DocumentStore>) -> PoseOutcome {
    canonical_solve(
        store,
        vec![driver(17, -90.0), driver(19, 90.0), driver(21, 90.0)],
    )
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
fn different_paths_to_the_same_desired_angles_have_the_same_canonical_result() {
    let signatures: Vec<_> = all_six_orders()
        .into_iter()
        .map(|order| {
            let store = fresh_store();
            let _ = follow_path(&store, order);
            (order, signature(&settle_same_desired(&store)))
        })
        .collect();
    let expected = &signatures[0];
    for actual in &signatures[1..] {
        assert_eq!(
            actual.1, expected.1,
            "同じ希望角でも操作順 {:?} と {:?} で導出結果が違う",
            expected.0, actual.0
        );
    }
}

#[test]
fn canonical_result_ignores_stored_warm_branch() {
    let p = fresh_store();
    let q = fresh_store();
    let _ = follow_path(&p, [17, 19, 21]);
    let _ = follow_path(&q, [21, 19, 17]);

    let from_p = signature(&settle_same_desired(&p));
    let from_q = signature(&settle_same_desired(&q));
    assert_eq!(
        from_p, from_q,
        "同一payloadの結果をDocumentStore.pose_anglesが変えている"
    );
}

#[test]
fn canonical_result_ignores_explicit_warm_without_validating_it() {
    let desired = vec![driver(17, -90.0), driver(19, 90.0), driver(21, 90.0)];
    let expected = signature(&canonical_solve_with_warm(
        &fresh_store(),
        desired.clone(),
        None,
    ));
    let actual = signature(&canonical_solve_with_warm(
        &fresh_store(),
        desired,
        Some(vec![driver(17, f64::NAN), driver(21, f64::INFINITY)]),
    ));
    assert_eq!(
        actual, expected,
        "Canonicalが明示warmの値または有限性を候補生成へ混ぜている"
    );
}

#[test]
fn canonical_command_without_sequence_uses_complete_zero_document_seed_winner() {
    let store = fresh_store();
    let desired = vec![driver(17, -90.0), driver(19, 90.0), driver(21, 90.0)];
    let actual =
        canonical_solve_with_warm(&store, desired.clone(), Some(vec![driver(17, f64::NAN)]));

    let (doc, faces, _, overlap_enabled, penetration_enabled) = store
        .lock()
        .unwrap_or_else(|state| state.into_inner())
        .pose_inputs();
    let seed = canonical_document_seed(&doc, &faces, 0, 1.0);
    let targets: HashMap<_, _> = desired
        .into_iter()
        .map(|item| (item.hinge, item.target_angle_deg))
        .collect();
    let expected = ori3_rigid::motion::solve_canonical_motion_with_contact_options(
        &doc.cp,
        &faces,
        &[],
        Some(&targets),
        Some(&seed),
        pose_motion_contact_options(overlap_enabled, penetration_enabled),
    );

    assert_eq!(
        signature(&actual),
        result_signature(&expected.result),
        "実command経路が全0度Document seedで選ぶcanonical winnerと一致しない"
    );
    assert_eq!(
        seed.len(),
        doc.cp
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .count(),
        "手順なしseedは全ての山谷hingeを含む"
    );
    assert!(seed.values().all(|&angle| angle == 0.0));
}

#[test]
fn sequence_document_seed_fills_replay_missing_hinges_with_zero() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("crates/ori3-rigid/tests/fixtures/check-bird-base.ori3");
    let store = Mutex::new(DocumentStore::default());
    store
        .lock()
        .unwrap_or_else(|state| state.into_inner())
        .open(&path)
        .expect("手順付きfixtureを開けるはず");
    let (doc, faces, _, _, _) = store
        .lock()
        .unwrap_or_else(|state| state.into_inner())
        .pose_inputs();
    let replay = ori3_layers::replay_with_faces(&doc, &faces, 1, 0.5);
    let seed = canonical_document_seed(&doc, &faces, 1, 0.5);
    let hinges: Vec<_> = doc
        .cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|edge| edge.id)
        .collect();

    assert_eq!(seed.len(), hinges.len());
    for hinge in hinges {
        assert_eq!(
            seed.get(&hinge),
            Some(replay.hinge_angles.get(&hinge).unwrap_or(&0.0)),
            "hinge {hinge} のreplay角または0度補完が違う"
        );
    }
}

#[test]
fn auxiliary_edges_are_not_inserted_into_canonical_document_seed() {
    let store = fresh_store();
    let (mut doc, faces, _, _, _) = store
        .lock()
        .unwrap_or_else(|state| state.into_inner())
        .pose_inputs();
    let auxiliary = doc
        .cp
        .edges
        .iter_mut()
        .find(|edge| edge.kind == EdgeKind::Mountain)
        .expect("sa fixtureに山折りがあるはず");
    let auxiliary_id = auxiliary.id;
    auxiliary.kind = EdgeKind::Aux;

    let seed = canonical_document_seed(&doc, &faces, 0, 1.0);
    assert!(
        !seed.contains_key(&auxiliary_id),
        "補助線を0度の折りヒンジ候補へ混ぜている"
    );
    assert_eq!(
        seed.len(),
        doc.cp
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .count()
    );
}

#[test]
fn one_angle_undo_restores_complete_derived_state() {
    let store = fresh_store();

    // 操作直前: 19=90 → 21=90。
    let _ = solve(&store, vec![driver(19, 90.0)], Vec::new(), None);
    let _ = solve(&store, vec![driver(21, 90.0)], vec![driver(19, 90.0)], None);
    let before = canonical_solve(&store, vec![driver(19, 90.0), driver(21, 90.0)]);

    // 1ジェスチャー: 17=-90。
    let _ = solve(
        &store,
        vec![driver(17, -90.0)],
        vec![driver(19, 90.0), driver(21, 90.0)],
        None,
    );
    let _ = canonical_solve(
        &store,
        vec![driver(17, -90.0), driver(19, 90.0), driver(21, 90.0)],
    );

    // Undoは希望値だけを戻す。操作直前も復元後も同じcanonical入力から再導出する。
    let after_one_undo = canonical_solve(&store, vec![driver(19, 90.0), driver(21, 90.0)]);

    assert_eq!(
        signature(&after_one_undo),
        signature(&before),
        "Undo 1回でactual・全頂点・layer・surface_rank・mirroredが戻っていない"
    );
}

#[test]
fn canonical_result_is_stable_ten_times_and_after_reopen() {
    let mut baseline = None;
    for repetition in 0..10 {
        for order in all_six_orders() {
            // fresh_storeが毎回fixtureを開き直すので、cacheの無い再読込境界も通る。
            let store = fresh_store();
            let _ = follow_path(&store, order);
            let actual = signature(&settle_same_desired(&store));
            match &baseline {
                Some(expected) => assert_eq!(
                    &actual, expected,
                    "反復{repetition}・操作順{order:?}が基準と違う"
                ),
                None => baseline = Some(actual),
            }
        }
    }
}

#[test]
fn stale_completion_cannot_poison_the_next_canonical_result() {
    // 画面で破棄された古い応答が、計算後のcache書戻しだけを完了した状態を
    // `store_pose_angles` で決定的に作る。非同期の壁時計順に合否を依存させない。
    let stale_source = fresh_store();
    let stale = follow_path(&stale_source, [17, 19, 21]).result.angles;

    let clean = fresh_store();
    let expected = signature(&settle_same_desired(&clean));

    let poisoned = fresh_store();
    poisoned
        .lock()
        .unwrap_or_else(|state| state.into_inner())
        .store_pose_angles(stale);
    let actual = signature(&settle_same_desired(&poisoned));

    assert_eq!(
        actual, expected,
        "画面で不採用になった旧応答のcacheが次のCanonical結果を変えている"
    );
}
