//! 施策2・段階2-Aで先に置く、MoveStepのcommand JSON契約。
//!
//! `commands.rs`のinline `tests` moduleへ2-Bで登録し、parserとtransaction helperを
//! 本番と同じ経路で検査する。`#[ignore]`は付けない。

use std::sync::Mutex;

use ori3_model::{FoldStep, SeqOp, TechniqueKind};
use serde_json::{Value, json};

use crate::store::{DocumentStore, commit_count_for_test, reset_commit_count_for_test};

use super::super::{apply_sequence_operation_transactionally, parse_sequence_operation};

const PARSE_ERROR: &str = "折る操作を読み取れませんでした";

fn parse_error(value: Value) -> String {
    parse_sequence_operation(value).expect_err("不正なMoveStepを拒否する")
}

#[test]
fn move_step_missing_json_fields_are_rejected_100_times_each() {
    for attempt in 0..100 {
        let missing_type = json!({ "id": 7, "to_index": 1 });
        assert_eq!(
            parse_error(missing_type),
            PARSE_ERROR,
            "type欠落 attempt={attempt}"
        );

        let missing_id = json!({ "type": "MoveStep", "to_index": 1 });
        assert_eq!(
            parse_error(missing_id),
            PARSE_ERROR,
            "id欠落 attempt={attempt}"
        );

        let missing_to_index = json!({ "type": "MoveStep", "id": 7 });
        assert_eq!(
            parse_error(missing_to_index),
            PARSE_ERROR,
            "to_index欠落 attempt={attempt}"
        );
    }
}

#[test]
fn move_step_negative_overflow_unknown_type_and_unknown_field_are_rejected() {
    for value in [
        json!({ "type": "MoveStep", "id": 7, "to_index": -1 }),
        json!({ "type": "MoveSteps", "id": 7, "to_index": 1 }),
        json!({ "type": "MoveStep", "id": 7, "to_index": 1, "unexpected": true }),
    ] {
        assert_eq!(parse_error(value), PARSE_ERROR);
    }

    let above_usize: Value =
        serde_json::from_str(r#"{"type":"MoveStep","id":7,"to_index":18446744073709551616}"#)
            .expect("JSON token自体は読める");
    assert_eq!(parse_error(above_usize), PARSE_ERROR);
}

#[test]
fn move_step_usize_max_reaches_store_semantic_range_validation() {
    let value = json!({ "type": "MoveStep", "id": 7, "to_index": usize::MAX });
    let (operation, spatial) = parse_sequence_operation(value).expect("型の範囲内なら読める");
    assert!(spatial.is_none());
    assert!(matches!(
        operation,
        SeqOp::MoveStep {
            id: 7,
            to_index: usize::MAX
        }
    ));
}

#[test]
fn move_step_strict_fields_do_not_break_existing_spatial_envelope() {
    // MoveStepだけを厳密にする。SeqOp全体をdeny_unknown_fieldsにすると、既存の
    // `spatial` envelopeがSeqOp側には余剰fieldに見えて壊れるため、その回帰を固定する。
    let value = json!({
        "type": "PreviewFoldThrough",
        "up_to": 1,
        "line": [[0.0, 0.0], [1.0, 0.0]],
        "keep_side_point": [0.0, 1.0],
        "target_layers": null,
        "direction": "Up",
        "spatial": {
            "from": [0.5, 0.25, -0.25],
            "to": [0.5, 0.5, -0.25],
            "grab_face": 1,
            "mode": "flap"
        }
    });
    let (operation, spatial) = parse_sequence_operation(value).expect("既存payloadを読める");
    assert!(matches!(
        operation,
        SeqOp::PreviewFoldThrough { up_to: 1, .. }
    ));
    let spatial = spatial.expect("立体の当たり点を保つ");
    assert_eq!(spatial.from, [0.5, 0.25, -0.25]);
    assert_eq!(spatial.to, [0.5, 0.5, -0.25]);
    assert_eq!(spatial.grab_face, 1);
}

fn step(id: u32) -> FoldStep {
    FoldStep {
        id,
        kind: TechniqueKind::Simple,
        drivers: Vec::new(),
        layer_order: None,
        alignment: None,
        finish_soft: None,
        note: format!("手順{id}"),
    }
}

#[test]
fn move_step_command_derivation_failure_is_atomic_and_stable_for_100_attempts() {
    for attempt in 0..100 {
        let mut store = DocumentStore::default();
        store
            .apply_seq(SeqOp::PushStep { step: step(1) })
            .expect("1手目を用意する");
        store
            .apply_seq(SeqOp::PushStep { step: step(2) })
            .expect("2手目を用意する");
        // 2-Bのcfg(test) probeはDocument/step_creases/faces/undo/redo/dirty/path/
        // pose_anglesを含み、failpointや観測counter自体は含めない。
        store.fail_next_move_step_derivation_for_test();
        let before = store.atomicity_probe_for_test();
        let state = Mutex::new(store);

        reset_commit_count_for_test();
        // sequence_apply本体も必ずこのpure helperを呼ぶ。Tauri Stateの組立てだけを除き、
        // parse→候補replay/view導出→commit→返却まで本番と同じ経路を通す。
        let error = apply_sequence_operation_transactionally(
            &state,
            json!({ "type": "MoveStep", "id": 1, "to_index": 1 }),
        )
        .expect_err("注入した本番導出panicをcommand errorへ変換する");
        assert_eq!(
            error, "内部エラーが発生しました: MoveStepの導出に失敗しました",
            "attempt={attempt}"
        );
        assert_eq!(commit_count_for_test(), 0, "attempt={attempt}");

        let after = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .atomicity_probe_for_test();
        assert_eq!(after, before, "attempt={attempt}");
    }
}

#[test]
fn move_step_command_same_position_keeps_all_store_state_for_100_attempts() {
    for attempt in 0..100 {
        let mut store = DocumentStore::default();
        store
            .apply_seq(SeqOp::PushStep { step: step(1) })
            .expect("1手目を用意する");
        store
            .apply_seq(SeqOp::PushStep { step: step(2) })
            .expect("2手目を用意する");
        store.store_pose_angles([(987, 12.5)].into_iter().collect());
        let before = store.atomicity_probe_for_test();
        let state = Mutex::new(store);

        reset_commit_count_for_test();
        let view = apply_sequence_operation_transactionally(
            &state,
            json!({ "type": "MoveStep", "id": 1, "to_index": 0 }),
        )
        .expect("同じ位置への移動は成功する");
        assert_eq!(
            view.doc
                .sequence
                .iter()
                .map(|step| step.id)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "attempt={attempt}"
        );
        assert!(view.frame.is_some(), "replay済みview attempt={attempt}");
        assert_eq!(commit_count_for_test(), 0, "attempt={attempt}");

        let after = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .atomicity_probe_for_test();
        assert_eq!(after, before, "attempt={attempt}");
    }
}
