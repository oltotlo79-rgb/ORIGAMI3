//! 施策2・段階2-Aで先に置く、MoveStepのstore原子性契約。
//!
//! `store.rs`のinline `tests` moduleへ2-Bで登録済み。MoveStep本体、原子的な
//! 候補導出、test-only failpoint/counterが契約を破れば赤くなる。全検査は通常の
//! `#[test]`であり、`#[ignore]`は付けない。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::Mutex,
};

use ori3_cp::Face;
use ori3_model::{Document, FoldStep, Frame3D, SeqOp, StepCreases, StepId, VertexId};
use serde_json::json;

use super::super::{
    DocumentStore, MAX_UNDO, Snapshot, commit_count_for_test, reset_commit_count_for_test,
};
use super::{fold_op, square_store, step, yakko_cp};
use crate::commands::apply_sequence_operation_transactionally;

type CreaseLineBits = [[u64; 2]; 2];

#[derive(Clone, Debug, PartialEq)]
struct StoreProbe {
    document_bytes: Vec<u8>,
    step_creases_bytes: Vec<u8>,
    faces: Vec<Face>,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    dirty: bool,
    path: Option<PathBuf>,
    pose_angles: Option<HashMap<u32, f64>>,
}

fn probe(store: &DocumentStore) -> StoreProbe {
    StoreProbe {
        document_bytes: serde_json::to_vec(&store.doc).expect("Documentをbytes化できる"),
        step_creases_bytes: serde_json::to_vec(&store.step_creases)
            .expect("step_creasesをbytes化できる"),
        faces: store.faces.clone(),
        undo_stack: store.undo_stack.clone(),
        redo_stack: store.redo_stack.clone(),
        dirty: store.dirty,
        path: store.path.clone(),
        pose_angles: store.pose_angles.clone(),
    }
}

fn assert_store_unchanged(before: &StoreProbe, store: &DocumentStore, label: &str) {
    assert_eq!(probe(store), *before, "storeが変わった: {label}");
}

fn crease_line_bits(line: &[[f64; 2]; 2]) -> CreaseLineBits {
    [
        [line[0][0].to_bits(), line[0][1].to_bits()],
        [line[1][0].to_bits(), line[1][1].to_bits()],
    ]
}

fn normalized_step_crease_bits(
    entries: &[StepCreases],
) -> Result<BTreeMap<StepId, Vec<CreaseLineBits>>, StepId> {
    let mut normalized = BTreeMap::new();
    for entry in entries {
        let lines = entry.lines.iter().map(crease_line_bits).collect();
        if normalized.insert(entry.step, lines).is_some() {
            return Err(entry.step);
        }
    }
    Ok(normalized)
}

fn crease_for_step(id: StepId) -> StepCreases {
    // 1/128刻みは二進数で正確に表せる。ここでのbit比較は再計算値の近似比較ではなく、
    // MoveStepが保存済み座標を一切書き換えないことを確認するidentity検査である。
    let x = f64::from(id % 128) / 128.0;
    StepCreases {
        step: id,
        lines: vec![[[x, 0.0], [x, 1.0]]],
    }
}

fn store_with_ids_and_cp(ids: &[StepId], cp: ori3_model::CreasePattern) -> DocumentStore {
    let mut store = square_store();
    store.doc.cp = cp;
    store.doc.sequence = ids.iter().copied().map(step).collect();
    store.step_creases = ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(crease_for_step)
        .collect();
    store.faces = ori3_cp::extract_faces(&store.doc.cp);
    store.undo_stack.clear();
    store.redo_stack.clear();
    store.dirty = false;
    store.path = None;
    store.pose_angles = None;
    store
}

fn store_with_ids(ids: &[StepId]) -> DocumentStore {
    let cp = square_store().doc.cp;
    store_with_ids_and_cp(ids, cp)
}

fn add_failure_sentinels(store: &mut DocumentStore) {
    let mut undo_doc = store.doc.clone();
    if let Some(first) = undo_doc.sequence.first_mut() {
        first.note = "undo sentinel".to_string();
    }
    let mut redo_doc = store.doc.clone();
    if let Some(first) = redo_doc.sequence.first_mut() {
        first.note = "redo sentinel".to_string();
    }
    store.undo_stack = vec![Snapshot {
        doc: undo_doc,
        step_creases: store.step_creases.clone(),
    }];
    store.redo_stack = vec![Snapshot {
        doc: redo_doc,
        step_creases: store.step_creases.clone(),
    }];
    store.dirty = true;
    store.path = Some(PathBuf::from("movestep-contract-sentinel.ori3"));
    store.pose_angles = Some(HashMap::from([(0, 37.5)]));
}

fn ids(document: &Document) -> Vec<StepId> {
    document.sequence.iter().map(|item| item.id).collect()
}

fn direct_move(sequence: &mut Vec<FoldStep>, from: usize, to_index: usize) {
    let moved = sequence.remove(from);
    sequence.insert(to_index, moved);
}

fn direct_expected(before: &Document, id: StepId, to_index: usize) -> Document {
    let mut expected = before.clone();
    let from = expected
        .sequence
        .iter()
        .position(|candidate| candidate.id == id)
        .expect("oracleの対象IDがある");
    direct_move(&mut expected.sequence, from, to_index);
    expected
}

fn assert_one_move_history_roundtrip(
    before: &Document,
    expected: &Document,
    before_creases: &[StepCreases],
    store: &mut DocumentStore,
) {
    assert_eq!(store.undo_stack.len(), 1, "backend commitは1回だけ");
    assert!(store.redo_stack.is_empty(), "新しい移動でstale redoを消す");
    assert!(store.dirty, "実移動はdirtyにする");
    assert_eq!(store.doc, *expected);
    assert_eq!(store.step_creases, before_creases);

    store.undo().expect("Undo 1回で戻せる");
    assert_eq!(store.doc, *before);
    assert_eq!(store.step_creases, before_creases);
    assert_eq!(store.redo_stack.len(), 1);

    store.redo().expect("Redo 1回で進められる");
    assert_eq!(store.doc, *expected);
    assert_eq!(store.step_creases, before_creases);
}

#[test]
fn move_step_commits_once_and_roundtrips_with_one_undo_and_redo() {
    let cases = [
        (10, 2, vec![20, 30, 10, 40], "先頭から中央"),
        (20, 3, vec![10, 30, 40, 20], "中央から末尾"),
        (40, 0, vec![40, 10, 20, 30], "末尾から先頭"),
    ];

    for (id, to_index, expected_ids, label) in cases {
        for initial_dirty in [false, true] {
            let mut store = store_with_ids(&[10, 20, 30, 40]);
            store.dirty = initial_dirty;
            store.path = Some(PathBuf::from("movestep-success-sentinel.ori3"));
            store.pose_angles = Some(HashMap::from([(0, 12.5)]));
            let before = store.doc.clone();
            let before_creases = store.step_creases.clone();
            let faces_before = store.faces.clone();
            let path_before = store.path.clone();
            store.redo_stack.push(Snapshot {
                doc: before.clone(),
                step_creases: before_creases.clone(),
            });
            let expected = direct_expected(&before, id, to_index);

            reset_commit_count_for_test();
            let state = Mutex::new(store);
            let view = apply_sequence_operation_transactionally(
                &state,
                json!({ "type": "MoveStep", "id": id, "to_index": to_index }),
            )
            .unwrap_or_else(|error| panic!("{label}が成功する: {error}"));
            let mut store = state.into_inner().expect("成功時はlockがpoisonしない");
            assert_eq!(commit_count_for_test(), 1, "{label}: commit exactly 1");
            assert_eq!(ids(&view.doc), expected_ids, "{label}");
            assert_eq!(store.faces, faces_before, "sequence移動ではfaces不変");
            assert_eq!(store.path, path_before, "{label}: path不変");
            assert_eq!(
                store.pose_angles,
                Some(view.angles.clone()),
                "{label}: command成功後は返却viewのfinite anglesをwarm startへ保存"
            );
            assert_one_move_history_roundtrip(&before, &expected, &before_creases, &mut store);
        }
    }
}

#[test]
fn move_step_store_commit_leaves_pose_cache_for_the_command_layer() {
    let mut store = store_with_ids(&[1, 2, 3]);
    let pose_before = Some(HashMap::from([(0, 12.5)]));
    store.pose_angles = pose_before.clone();

    reset_commit_count_for_test();
    let view = store
        .apply_seq(SeqOp::MoveStep { id: 1, to_index: 2 })
        .expect("storeは候補replayをcommit前に導出する");

    assert!(view.frame.is_some(), "store戻り値は候補replay済み");
    assert_eq!(commit_count_for_test(), 1);
    assert_eq!(
        store.pose_angles, pose_before,
        "store確定直後はwarm startを触らず、command成功後の保存へ委ねる"
    );
}

#[test]
fn move_step_keeps_the_history_limit_at_100() {
    const HISTORY_LIMIT: usize = 100;
    assert_eq!(MAX_UNDO, HISTORY_LIMIT, "SYS-002の上限をliteralで固定する");
    let mut store = store_with_ids(&[1, 2, 3]);
    for marker in 0..(HISTORY_LIMIT - 1) {
        let mut doc = store.doc.clone();
        doc.sequence[0].note = format!("history {marker}");
        store.undo_stack.push(Snapshot {
            doc,
            step_creases: store.step_creases.clone(),
        });
    }
    let original_oldest = store.undo_stack[0].clone();
    let before_move = Snapshot {
        doc: store.doc.clone(),
        step_creases: store.step_creases.clone(),
    };

    reset_commit_count_for_test();
    store
        .apply_seq(SeqOp::MoveStep { id: 1, to_index: 2 })
        .expect("99件から100件目を積める");
    assert_eq!(
        store.undo_stack.len(),
        HISTORY_LIMIT,
        "最大100件を減らさない"
    );
    assert_eq!(commit_count_for_test(), 1);
    assert_eq!(store.undo_stack.first(), Some(&original_oldest));
    assert_eq!(store.undo_stack.last(), Some(&before_move));

    let expected_oldest_after_evict = store.undo_stack[1].clone();
    let before_at_capacity = Snapshot {
        doc: store.doc.clone(),
        step_creases: store.step_creases.clone(),
    };
    reset_commit_count_for_test();
    store
        .apply_seq(SeqOp::MoveStep { id: 1, to_index: 0 })
        .expect("100件の上限時も新しい1履歴を積める");
    assert_eq!(store.undo_stack.len(), HISTORY_LIMIT);
    assert_eq!(commit_count_for_test(), 1);
    assert_eq!(store.undo_stack.first(), Some(&expected_oldest_after_evict));
    assert_eq!(store.undo_stack.last(), Some(&before_at_capacity));
}

#[test]
fn move_step_two_steps_work_in_both_directions() {
    for (id, to_index) in [(1, 1), (2, 0)] {
        let mut store = store_with_ids(&[1, 2]);
        store
            .apply_seq(SeqOp::MoveStep { id, to_index })
            .expect("2手の両方向移動が成功する");
        assert_eq!(ids(&store.doc), vec![2, 1]);
        assert_eq!(store.undo_stack.len(), 1);
    }
}

#[test]
fn move_step_keeps_step_creases_bit_identical_by_step_id() {
    // 折り鶴の先頭2手と同じ、縦半分→横半分の2手を通常APIで作る。
    let mut store = square_store();
    store
        .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
        .expect("1手目を作る");
    store
        .apply_seq(fold_op(1, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
        .expect("2手目を作る");
    store.undo_stack.clear();
    store.redo_stack.clear();
    store.dirty = false;

    let before_bytes = serde_json::to_vec(&store.step_creases).expect("来歴をbytes化できる");
    let before = normalized_step_crease_bits(&store.step_creases)
        .unwrap_or_else(|id| panic!("移動前からstep crease ID {id}が重複"));
    assert_eq!(before.len(), 2, "clean fixtureは各stepに来歴1件");

    store
        .apply_seq(SeqOp::MoveStep { id: 1, to_index: 0 })
        .expect("2手目を先頭へ動かせる");

    let after = normalized_step_crease_bits(&store.step_creases)
        .unwrap_or_else(|id| panic!("移動後にstep crease ID {id}が重複"));
    assert_eq!(before, after, "step IDで正規化した全座標bitが一致");
    assert_eq!(
        serde_json::to_vec(&store.step_creases).expect("来歴をbytes化できる"),
        before_bytes,
        "MoveStepは内部のstep_creases vector自体を書き換えない"
    );
    assert_eq!(
        after.keys().copied().collect::<BTreeSet<_>>(),
        ids(&store.doc).into_iter().collect(),
        "clean fixtureでは欠落0件"
    );
}

#[test]
fn move_step_does_not_synthesize_legacy_crease_history() {
    let mut store = store_with_ids(&[1, 2, 3]);
    store.step_creases.clear();
    store
        .apply_seq(SeqOp::MoveStep { id: 1, to_index: 2 })
        .expect("来歴のない旧作品でも移動できる");
    assert!(
        store.step_creases.is_empty(),
        "旧作品へ推測した来歴を足さない"
    );
}

#[test]
fn move_step_preserves_stale_internal_crease_history_entries_byte_for_byte() {
    let mut store = store_with_ids(&[1, 2, 3]);
    store.step_creases.push(crease_for_step(999));
    let before = serde_json::to_vec(&store.step_creases).expect("来歴をbytes化できる");

    store
        .apply_seq(SeqOp::MoveStep { id: 1, to_index: 2 })
        .expect("余剰の旧来歴があっても手順は動かせる");

    assert_eq!(
        serde_json::to_vec(&store.step_creases).expect("来歴をbytes化できる"),
        before,
        "MoveStepは削除済みstepの内部来歴も勝手に消さない"
    );
}

#[test]
fn move_step_missing_id_is_atomic_for_100_attempts() {
    for (dirty_group, initial_dirty) in [false, true].into_iter().enumerate() {
        let mut store = store_with_ids(&[1, 2, 3]);
        add_failure_sentinels(&mut store);
        store.dirty = initial_dirty;
        let before = probe(&store);

        for local_attempt in 0..50 {
            let attempt = dirty_group * 50 + local_attempt;
            reset_commit_count_for_test();
            let error = store
                .apply_seq(SeqOp::MoveStep {
                    id: 999,
                    to_index: 1,
                })
                .expect_err("存在しないIDを拒否する");
            assert_eq!(error, "手順ID 999 が見つかりません", "attempt={attempt}");
            assert_eq!(commit_count_for_test(), 0, "attempt={attempt}");
            assert_store_unchanged(&before, &store, &format!("missing attempt={attempt}"));
        }
    }
}

#[test]
fn move_step_out_of_range_is_atomic_for_100_attempts() {
    let invalid = [3, 4, usize::MAX];

    for (dirty_group, initial_dirty) in [false, true].into_iter().enumerate() {
        let mut store = store_with_ids(&[1, 2, 3]);
        add_failure_sentinels(&mut store);
        store.dirty = initial_dirty;
        let before = probe(&store);

        for local_attempt in 0..50 {
            let attempt = dirty_group * 50 + local_attempt;
            let to_index = invalid[attempt % invalid.len()];
            reset_commit_count_for_test();
            let error = store
                .apply_seq(SeqOp::MoveStep { id: 2, to_index })
                .expect_err("len以上の最終indexを拒否する");
            assert_eq!(
                error,
                format!("移動先 {to_index} が手順の数を超えています"),
                "attempt={attempt}"
            );
            assert_eq!(commit_count_for_test(), 0, "attempt={attempt}");
            assert_store_unchanged(&before, &store, &format!("range attempt={attempt}"));
        }
    }
}

#[test]
fn move_step_duplicate_ids_are_rejected_atomically_for_100_attempts() {
    for (dirty_group, initial_dirty) in [false, true].into_iter().enumerate() {
        let mut store = store_with_ids(&[1, 2, 2]);
        add_failure_sentinels(&mut store);
        store.dirty = initial_dirty;
        let before = probe(&store);

        for local_attempt in 0..50 {
            let attempt = dirty_group * 50 + local_attempt;
            // 偶数回は重複していない対象、奇数回は重複対象を選ぶ。sequence内のどこかに
            // 重複があれば、対象だけに限定せずMoveStep全体を防御する。
            let id = if attempt % 2 == 0 { 1 } else { 2 };
            reset_commit_count_for_test();
            let error = store
                .apply_seq(SeqOp::MoveStep { id, to_index: 0 })
                .expect_err("重複IDを拒否する");
            assert_eq!(error, "同じ折り手順が二重に入っています");
            assert_eq!(commit_count_for_test(), 0, "attempt={attempt}");
            assert_store_unchanged(&before, &store, &format!("duplicate attempt={attempt}"));
        }
    }
}

#[test]
fn move_step_validation_precedence_is_deterministic() {
    let mut duplicate = store_with_ids(&[1, 2, 2]);
    reset_commit_count_for_test();
    assert_eq!(
        duplicate
            .apply_seq(SeqOp::MoveStep {
                id: 999,
                to_index: usize::MAX,
            })
            .expect_err("重複を最初に検査する"),
        "同じ折り手順が二重に入っています"
    );
    assert_eq!(commit_count_for_test(), 0);

    let mut unique = store_with_ids(&[1, 2, 3]);
    reset_commit_count_for_test();
    assert_eq!(
        unique
            .apply_seq(SeqOp::MoveStep {
                id: 999,
                to_index: usize::MAX,
            })
            .expect_err("ID不存在を範囲より先に検査する"),
        "手順ID 999 が見つかりません"
    );
    assert_eq!(commit_count_for_test(), 0);
}

#[test]
fn move_step_derivation_failure_is_atomic_for_100_attempts() {
    for attempt in 0..100 {
        let mut store = store_with_ids(&[1, 2, 3]);
        add_failure_sentinels(&mut store);
        store.dirty = attempt % 2 != 0;
        // 2-Bで、MoveStep候補の最後のreplay/view導出（commitより前）へ実装する
        // cfg(test)注入口。製品APIには公開せず、他の並列testへ失敗指定を漏らさない。
        store.fail_next_move_step_derivation_for_test();
        let before = probe(&store);

        reset_commit_count_for_test();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            store.apply_seq(SeqOp::MoveStep { id: 1, to_index: 2 })
        }));
        let payload = outcome.expect_err("注入した導出panicを成功扱いしない");
        let message = payload
            .downcast_ref::<&str>()
            .map(|value| (*value).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "詳細不明".to_string());
        assert_eq!(message, "MoveStepの導出に失敗しました", "attempt={attempt}");
        assert_eq!(commit_count_for_test(), 0, "失敗候補はcommitしない");
        assert_store_unchanged(&before, &store, &format!("derive attempt={attempt}"));
    }
}

#[test]
fn move_step_same_position_is_successful_noop() {
    for dirty in [false, true] {
        let mut store = store_with_ids(&[10, 20, 30]);
        add_failure_sentinels(&mut store);
        store.dirty = dirty;
        let before = probe(&store);

        reset_commit_count_for_test();
        let view = store
            .apply_seq(SeqOp::MoveStep {
                id: 20,
                to_index: 1,
            })
            .expect("同一位置は成功する");
        assert_eq!(view.doc, store.doc);
        assert_eq!(view.step_creases, store.step_creases);
        assert_eq!(view.faces, store.faces);
        assert_eq!(commit_count_for_test(), 0, "同一位置はcommitしない");
        assert_store_unchanged(&before, &store, &format!("dirty={dirty}"));

        let mut command_store = store_with_ids(&[10, 20, 30]);
        add_failure_sentinels(&mut command_store);
        command_store.dirty = dirty;
        let command_before = probe(&command_store);
        let state = Mutex::new(command_store);
        reset_commit_count_for_test();
        let command_view = apply_sequence_operation_transactionally(
            &state,
            json!({ "type": "MoveStep", "id": 20, "to_index": 1 }),
        )
        .expect("command経路でも同一位置は成功する");
        assert!(command_view.frame.is_some(), "返却viewはreplay済み");
        assert_eq!(commit_count_for_test(), 0, "command経路でもcommitしない");
        let command_store = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_store_unchanged(
            &command_before,
            &command_store,
            &format!("command dirty={dirty}"),
        );
    }
}

#[test]
fn move_step_zero_and_one_step_boundaries_are_deterministic() {
    let mut empty = store_with_ids(&[]);
    add_failure_sentinels(&mut empty);
    let empty_before = probe(&empty);
    reset_commit_count_for_test();
    let error = empty
        .apply_seq(SeqOp::MoveStep { id: 1, to_index: 0 })
        .expect_err("0手では範囲判定より先にID不存在になる");
    assert_eq!(error, "手順ID 1 が見つかりません");
    assert_eq!(commit_count_for_test(), 0);
    assert_store_unchanged(&empty_before, &empty, "zero steps");

    for dirty in [false, true] {
        let mut single = store_with_ids(&[1]);
        add_failure_sentinels(&mut single);
        single.dirty = dirty;
        let single_before = probe(&single);
        reset_commit_count_for_test();
        let view = single
            .apply_seq(SeqOp::MoveStep { id: 1, to_index: 0 })
            .expect("1手の唯一の位置は成功するno-op");
        assert_eq!(view.doc, single.doc);
        assert_eq!(view.step_creases, single.step_creases);
        assert_eq!(view.faces, single.faces);
        assert_eq!(commit_count_for_test(), 0);
        assert_store_unchanged(&single_before, &single, &format!("one step dirty={dirty}"));
    }
}

fn material_points(faces: &[Face], frame: &Frame3D) -> BTreeMap<(u32, usize, VertexId), [f64; 3]> {
    let mut points = BTreeMap::new();
    for face in faces {
        let spatial = frame
            .faces
            .iter()
            .find(|candidate| candidate.face == face.id)
            .unwrap_or_else(|| panic!("frameにface {}がある", face.id));
        assert_eq!(face.vertices.len(), spatial.polygon.len());
        for (boundary_index, (vertex, point)) in
            face.vertices.iter().zip(&spatial.polygon).enumerate()
        {
            assert!(
                points
                    .insert((face.id, boundary_index, *vertex), *point)
                    .is_none(),
                "face/boundary/material vertex対応が一意"
            );
        }
    }
    points
}

fn euclidean_distance(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}

#[derive(Clone, Copy, Debug, Default)]
struct ReplayMetrics {
    cases: usize,
    max_vertex_distance: f64,
    max_seam: f64,
    penetration_pairs: usize,
    non_finite_coordinates: usize,
}

impl ReplayMetrics {
    fn include(&mut self, other: Self) {
        self.cases += other.cases;
        self.max_vertex_distance = self.max_vertex_distance.max(other.max_vertex_distance);
        self.max_seam = self.max_seam.max(other.max_seam);
        self.penetration_pairs += other.penetration_pairs;
        self.non_finite_coordinates += other.non_finite_coordinates;
    }
}

fn assert_returned_replay_matches_direct_oracle(
    actual_doc: &Document,
    actual_faces: &[Face],
    actual_frame: &Frame3D,
    oracle_doc: &Document,
    label: &str,
) -> ReplayMetrics {
    const VERTEX_TOLERANCE: f64 = 1e-9;
    const SEAM_TOLERANCE: f64 = 1e-6;
    assert_eq!(
        ori3_model::EPS,
        VERTEX_TOLERANCE,
        "§6.4の固定境界と共通EPSを同時に固定する"
    );

    let oracle_faces = ori3_cp::extract_faces(&oracle_doc.cp);
    let oracle =
        ori3_layers::replay_with_faces(oracle_doc, &oracle_faces, oracle_doc.sequence.len(), 1.0);

    let actual_points = material_points(actual_faces, actual_frame);
    let oracle_points = material_points(&oracle_faces, &oracle.frame);
    assert_eq!(
        actual_points.keys().collect::<Vec<_>>(),
        oracle_points.keys().collect::<Vec<_>>()
    );

    // 独立replay経路の既存折り鶴標本では実測最大差0.0。実測0を境界にせず、
    // 紙の長辺=1の共通EPSである1e-9まで余裕を取る。face/vertex ID対応は完全一致で
    // 固定し、別頂点との誤対応を許容差で飲み込まない。
    let max_vertex_distance = actual_points
        .iter()
        .map(|(key, point)| euclidean_distance(*point, oracle_points[key]))
        .fold(0.0_f64, f64::max);
    assert!(
        max_vertex_distance <= VERTEX_TOLERANCE,
        "{label}: vertex distance={max_vertex_distance:.17e}"
    );

    let mut metrics = ReplayMetrics {
        cases: 1,
        max_vertex_distance,
        ..ReplayMetrics::default()
    };

    // 鳥の基本形21姿勢の既存実測最大seamは2.448156e-13。計測値そのものを
    // 境界にせず、紙の長辺=1で製品が裂けと判断する1e-6（約7桁の余裕）を使う。
    for (kind, doc, faces, frame) in [
        ("actual", actual_doc, actual_faces, actual_frame),
        ("oracle", oracle_doc, oracle_faces.as_slice(), &oracle.frame),
    ] {
        let seam = ori3_rigid::max_seam_gap(&doc.cp, faces, frame);
        metrics.max_seam = metrics.max_seam.max(seam);
        assert!(seam.is_finite(), "{label}: {kind} seamがfinite");
        assert!(seam <= SEAM_TOLERANCE, "{label}: {kind} seam={seam:.17e}");
        // penetrationは計算されたdepthの小数一致ではなく、離散的な交差face組数0を
        // 完全一致で固定する。
        let penetration_pairs = ori3_rigid::self_intersection_pairs(frame).len();
        metrics.penetration_pairs += penetration_pairs;
        assert_eq!(penetration_pairs, 0, "{label}: {kind} penetration");
        let non_finite_coordinates = frame
            .faces
            .iter()
            .flat_map(|face| &face.polygon)
            .flatten()
            .filter(|coordinate| !coordinate.is_finite())
            .count();
        metrics.non_finite_coordinates += non_finite_coordinates;
        assert_eq!(non_finite_coordinates, 0, "{label}: {kind}の非finite座標");
    }
    metrics
}

#[test]
fn move_step_all_legal_head_middle_tail_cases_for_lengths_2_through_100() {
    let mut total_cases = 0usize;
    let mut changed_cases = 0usize;
    let mut noop_cases = 0usize;
    let mut replay_metrics = ReplayMetrics::default();

    for len in 2usize..=100 {
        let original_ids = (0..len)
            .map(|index| 1_000 + index as StepId)
            .collect::<Vec<_>>();
        let sources = BTreeSet::from([0, len / 2, len - 1]);
        for from in sources {
            for to_index in 0..len {
                total_cases += 1;
                let store = store_with_ids_and_cp(&original_ids, yakko_cp());
                let before_creases = normalized_step_crease_bits(&store.step_creases)
                    .expect("clean fixtureのstep crease IDは一意");
                let id = original_ids[from];
                let mut oracle_doc = store.doc.clone();
                direct_move(&mut oracle_doc.sequence, from, to_index);

                let state = Mutex::new(store);
                let view = apply_sequence_operation_transactionally(
                    &state,
                    json!({ "type": "MoveStep", "id": id, "to_index": to_index }),
                )
                .unwrap_or_else(|error| panic!("len={len} from={from} to={to_index}: {error}"));
                let store = state.lock().expect("成功時はlockがpoisonしない");
                let label = format!("len={len} from={from} to={to_index}");
                assert_eq!(store.doc, oracle_doc, "{label}: Document全体");
                assert_eq!(view.doc, store.doc, "{label}: 返却Document");
                assert_eq!(view.faces, store.faces, "{label}: 返却faces");
                assert_eq!(ids(&store.doc), ids(&oracle_doc), "{label}: sequence");
                assert_eq!(store.doc.sequence.len(), len, "{label}: count");
                assert_eq!(
                    ids(&store.doc).into_iter().collect::<BTreeSet<_>>(),
                    original_ids.iter().copied().collect(),
                    "{label}: ID set"
                );
                assert_eq!(
                    normalized_step_crease_bits(&store.step_creases)
                        .expect("移動後もstep crease IDは一意"),
                    before_creases,
                    "{label}: step creases"
                );

                if from == to_index {
                    noop_cases += 1;
                    assert!(store.undo_stack.is_empty(), "{label}: no-op history=0");
                    assert!(!store.dirty, "{label}: no-op dirty変化0");
                } else {
                    changed_cases += 1;
                    assert_eq!(store.undo_stack.len(), 1, "{label}: history=1");
                    assert!(store.dirty, "{label}: dirty");
                }

                // 移動前の終点ではなく、履歴APIを通さず直接Vecを並べた独立cloneがoracle。
                let actual_frame = view
                    .frame
                    .as_ref()
                    .expect("MoveStepはcommit前に候補replayを導出し、その同じframeを返す");
                replay_metrics.include(assert_returned_replay_matches_direct_oracle(
                    &view.doc,
                    &view.faces,
                    actual_frame,
                    &oracle_doc,
                    &label,
                ));
            }
        }
    }

    assert_eq!(total_cases, 15_145);
    assert_eq!(changed_cases, 14_849);
    assert_eq!(noop_cases, 296);
    // 実折り2 caseは専用test
    // `move_step_real_two_fold_crane_prefix_matches_direct_oracle_both_directions` が
    // 同じ `run_real_two_fold_oracle_cases()` を呼び、2方向とcase数まで独立名で保証する。
    // ここでは15,145 synthetic caseだけを集計し、同じoracleを二重実行しない。
    assert_eq!(replay_metrics.cases, 15_145);
    eprintln!(
        "[MoveStep §6.4-6実測] legal_cases={total_cases} synthetic_replay_cases={} \
         max_vertex_distance={:.17e} max_seam={:.17e} \
         penetration_pairs={} non_finite_coordinates={}",
        replay_metrics.cases,
        replay_metrics.max_vertex_distance,
        replay_metrics.max_seam,
        replay_metrics.penetration_pairs,
        replay_metrics.non_finite_coordinates,
    );
}

fn run_real_two_fold_oracle_cases() -> ReplayMetrics {
    let mut metrics = ReplayMetrics::default();
    for (id, to_index, label) in [(0, 1, "先→後"), (1, 0, "後→先")] {
        let mut store = square_store();
        store
            .apply_seq(fold_op(0, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]))
            .expect("1手目を作る");
        store
            .apply_seq(fold_op(1, [[0.0, 0.5], [1.0, 0.5]], [0.5, 0.25]))
            .expect("2手目を作る");
        store.undo_stack.clear();
        store.redo_stack.clear();
        store.dirty = false;
        let mut oracle_doc = store.doc.clone();
        let from = oracle_doc
            .sequence
            .iter()
            .position(|candidate| candidate.id == id)
            .expect("対象ID");
        direct_move(&mut oracle_doc.sequence, from, to_index);

        let state = Mutex::new(store);
        let view = apply_sequence_operation_transactionally(
            &state,
            json!({ "type": "MoveStep", "id": id, "to_index": to_index }),
        )
        .unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(view.doc, oracle_doc, "{label}: 返却Document");
        let actual_frame = view
            .frame
            .as_ref()
            .expect("実折りでもcommit前に導出したframeを返す");
        metrics.include(assert_returned_replay_matches_direct_oracle(
            &view.doc,
            &view.faces,
            actual_frame,
            &oracle_doc,
            label,
        ));
    }
    metrics
}

#[test]
fn move_step_real_two_fold_crane_prefix_matches_direct_oracle_both_directions() {
    let metrics = run_real_two_fold_oracle_cases();
    assert_eq!(metrics.cases, 2);
}
