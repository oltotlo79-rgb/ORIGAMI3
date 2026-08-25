//! 施策2・段階2-Aで先に置き、段階2-Bで登録したMoveStepのserde契約。
//!
//! `tests/movestep_serde.rs`からCargoの統合検査として登録する。`SeqOp::MoveStep`、
//! MoveStep専用の未知field拒否、既存sequence operationとの互換を検査する。

use ori3_model::{Document, Paper, SCHEMA_VERSION, SavedDocument, SeqOp};

fn assert_seq_op_rejected(json: &str) {
    assert!(
        serde_json::from_str::<SeqOp>(json).is_err(),
        "拒否すべきsequence operationを受理した: {json}"
    );
}

#[test]
fn move_step_json_shape_roundtrips() {
    let op = SeqOp::MoveStep { id: 7, to_index: 2 };
    let json = serde_json::to_string(&op).expect("MoveStepをJSONへ書ける");
    assert_eq!(json, r#"{"type":"MoveStep","id":7,"to_index":2}"#);

    let back: SeqOp = serde_json::from_str(&json).expect("MoveStepをJSONから読める");
    let SeqOp::MoveStep { id, to_index } = back else {
        panic!("MoveStep以外へ復元された: {back:?}");
    };
    assert_eq!(id, 7);
    assert_eq!(to_index, 2);
}

#[test]
fn move_step_rejects_missing_type_id_or_to_index() {
    for json in [
        r#"{"id":7,"to_index":2}"#,
        r#"{"type":"MoveStep","to_index":2}"#,
        r#"{"type":"MoveStep","id":7}"#,
    ] {
        assert_seq_op_rejected(json);
    }
}

#[test]
fn move_step_rejects_unknown_type_and_move_step_only_unknown_fields() {
    assert_seq_op_rejected(r#"{"type":"MoveSteps","id":7,"to_index":2}"#);
    assert_seq_op_rejected(r#"{"type":"MoveStep","id":7,"to_index":2,"unexpected":true}"#);
}

#[test]
fn existing_operations_keep_accepting_spatial_envelope_fields() {
    let json = r#"{"type":"RemoveStep","id":7,"spatial":{"from":[0.0,0.0,0.0]}}"#;
    let operation: SeqOp = serde_json::from_str(json).expect("既存操作のspatial envelopeを保つ");
    assert!(matches!(operation, SeqOp::RemoveStep { id: 7 }));
}

#[test]
fn existing_push_and_update_step_fixed_json_shapes_roundtrip() {
    let step = r#"{"id":7,"kind":"Simple","drivers":[],"layer_order":null,"note":"旧手順"}"#;
    for (variant, json) in [
        (
            "PushStep",
            format!(r#"{{"type":"PushStep","step":{step}}}"#),
        ),
        (
            "UpdateStep",
            format!(r#"{{"type":"UpdateStep","step":{step}}}"#),
        ),
    ] {
        let operation: SeqOp = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("既存{variant}を読める: {error}"));
        match (&operation, variant) {
            (SeqOp::PushStep { step }, "PushStep") | (SeqOp::UpdateStep { step }, "UpdateStep") => {
                assert_eq!(step.id, 7);
                assert_eq!(step.note, "旧手順");
            }
            (other, _) => panic!("{variant}以外へ復元された: {other:?}"),
        }
        assert_eq!(
            serde_json::to_string(&operation)
                .unwrap_or_else(|error| panic!("既存{variant}を書ける: {error}")),
            json,
            "既存{variant}の固定JSON shapeを変えない"
        );
    }
}

#[test]
fn move_step_rejects_negative_or_type_overflow_values() {
    assert_seq_op_rejected(r#"{"type":"MoveStep","id":7,"to_index":-1}"#);
    assert_seq_op_rejected(r#"{"type":"MoveStep","id":-1,"to_index":0}"#);
    assert_seq_op_rejected(r#"{"type":"MoveStep","id":4294967296,"to_index":0}"#);
    // 対応OSは64-bit。usize::MAXを1だけ超える値はserde層で拒否する。
    assert_seq_op_rejected(r#"{"type":"MoveStep","id":7,"to_index":18446744073709551616}"#);
}

#[test]
fn move_step_roundtrips_usize_max_before_store_range_validation() {
    let json = format!(r#"{{"type":"MoveStep","id":7,"to_index":{}}}"#, usize::MAX);
    let op: SeqOp = serde_json::from_str(&json).expect("usize::MAX自体は型の範囲内");
    let SeqOp::MoveStep { id, to_index } = op else {
        panic!("MoveStep以外へ復元された: {op:?}");
    };
    assert_eq!(id, 7);
    assert_eq!(to_index, usize::MAX);
}

#[test]
fn legacy_seq_op_and_saved_document_remain_readable_without_schema_bump() {
    let remove_json = r#"{"type":"RemoveStep","id":7}"#;
    let old_remove: SeqOp = serde_json::from_str(remove_json).expect("既存RemoveStep tagを読める");
    assert!(matches!(&old_remove, SeqOp::RemoveStep { id: 7 }));
    assert_eq!(
        serde_json::to_string(&old_remove).expect("既存RemoveStepを書ける"),
        remove_json
    );

    let insert_json = r#"{"type":"InsertStep","index":1,"step":{"id":7,"kind":"Simple","drivers":[],"layer_order":null,"note":"旧手順"}}"#;
    let old_insert: SeqOp = serde_json::from_str(insert_json).expect("既存InsertStep tagを読める");
    match &old_insert {
        SeqOp::InsertStep { index, step } => {
            assert_eq!(*index, 1);
            assert_eq!(step.id, 7);
            assert_eq!(step.note, "旧手順");
        }
        other => panic!("InsertStep以外へ復元された: {other:?}"),
    }
    assert_eq!(
        serde_json::to_string(&old_insert).expect("既存InsertStepを書ける"),
        insert_json
    );

    // MoveStepは一時的なIPC操作であり、.ori3へ操作ログとして保存しない。
    // 旧形式はDocumentだけ（step_creases fieldなし）なので、その形を固定する。
    let document = Document::new(Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    });
    let old_json = serde_json::to_string(&document).expect("旧形式を作れる");
    assert!(!old_json.contains("step_creases"));

    let saved: SavedDocument = serde_json::from_str(&old_json).expect("旧作品を読める");
    assert_eq!(saved.document, document);
    assert!(saved.step_creases.is_empty());
    assert_eq!(saved.document.schema_version, SCHEMA_VERSION);
    assert_eq!(
        SCHEMA_VERSION, 1,
        "MoveStep追加だけでは保存schemaを上げない"
    );
    assert_eq!(
        serde_json::to_string(&saved).expect("旧作品を再び書ける"),
        old_json
    );
}

#[test]
fn fixed_legacy_schema_v1_document_without_newer_optional_fields_remains_readable() {
    // 現行serializerから生成すると、誤って新しい必須fieldを足した場合にも生成側へ
    // 同じfieldが入って偽陰性になる。固定literalを独立oracleとして読む。
    let legacy = r#"{
        "schema_version": 1,
        "paper": {"width_mm": 150.0, "height_mm": 150.0},
        "cp": {
            "vertices": [
                {"id": 0, "pos": [0.0, 0.0]},
                {"id": 1, "pos": [1.0, 0.0]},
                {"id": 2, "pos": [1.0, 1.0]},
                {"id": 3, "pos": [0.0, 1.0]}
            ],
            "edges": [
                {"id": 0, "v0": 0, "v1": 1, "kind": "Border"},
                {"id": 1, "v0": 1, "v1": 2, "kind": "Border"},
                {"id": 2, "v0": 2, "v1": 3, "kind": "Border"},
                {"id": 3, "v0": 3, "v1": 0, "kind": "Border"}
            ],
            "next_vertex_id": 4,
            "next_edge_id": 4
        },
        "sequence": [],
        "display": {
            "front_color": [237, 28, 36],
            "back_color": [255, 255, 255],
            "grid_divisions": 8
        }
    }"#;

    let saved: SavedDocument = serde_json::from_str(legacy).expect("固定した旧schema v1を読める");
    assert_eq!(saved.document.schema_version, 1);
    assert!(saved.document.sequence.is_empty());
    assert!(saved.step_creases.is_empty());
    assert!(!saved.document.display.soft_enabled);
    assert_eq!(saved.document.display.soft_stiffness, 0.5);
    assert_eq!(saved.document.display.soft_pressure, 0.0);
}
