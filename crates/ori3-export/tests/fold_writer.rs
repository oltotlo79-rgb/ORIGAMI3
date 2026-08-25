use ori3_export::fold::{FoldAssignment, FoldFile, FoldIssueCode, parse_fold_1_2, write_fold_1_2};
use serde_json::{Value, json};

const SERIALIZED_NUMBER_EPSILON: f64 = 1e-12;
const MINIMAL_SUPPORTED: &str = include_str!("fixtures/fold/minimal-supported.fold");
const FU_ASSIGNMENTS: &str = include_str!("fixtures/fold/fu-assignments.fold");
const LINEAR_STEPS: &str = include_str!("fixtures/fold/linear-steps.fold");

fn fixture(json: &str) -> FoldFile {
    parse_fold_1_2(json).expect("手書きfixtureをtyped FOLDとして読める")
}

fn minimal_file() -> FoldFile {
    fixture(MINIMAL_SUPPORTED)
}

fn parse_written(file: &FoldFile) -> Value {
    let json = write_fold_1_2(file).expect("限定profileをJSONへ書ける");
    serde_json::from_str(&json).expect("writerの出力はJSONとして読める")
}

#[test]
fn writer_is_pretty_and_deterministic_without_reordering_semantic_arrays() {
    let file = minimal_file();
    let first = write_fold_1_2(&file).expect("最小profileを書ける");

    assert!(
        first.contains("\n  \""),
        "2空白のpretty JSONである: {first}"
    );
    assert!(!first.contains('\r'), "改行はLFへ統一する");

    // 同じtyped入力のbyte決定性を調べる比較であり、計算結果の小数を
    // 期待文字列へ固定する検査ではない。
    for iteration in 0..10 {
        let repeated = write_fold_1_2(&file).expect("同じ入力を繰り返し書ける");
        assert_eq!(repeated, first, "同じ入力の{iteration}回目も同じJSONになる");
    }

    let value: Value = serde_json::from_str(&first).expect("writerの出力を読める");
    assert!(
        value.get("root").is_none(),
        "root frameはtop-levelへ展開する"
    );
    assert_eq!(
        value["edges_vertices"],
        json!([[0, 1], [1, 2], [2, 3], [3, 0], [0, 2]]),
        "edge topologyの順序を保つ"
    );
    assert_eq!(
        value["edges_assignment"],
        json!(["B", "B", "B", "B", "M"]),
        "assignmentの順序を保つ"
    );

    let written_x = value["vertices_coords"][1][0]
        .as_f64()
        .expect("座標を数として書く");
    assert!(
        (written_x - 1.0).abs() <= SERIALIZED_NUMBER_EPSILON,
        "JSON化だけの実測差0へ、桁違いを十分区別できる1e-12の余裕を取る: {written_x}"
    );
}

#[test]
fn writer_preserves_fu_assignments_instead_of_silently_changing_them() {
    let file = fixture(FU_ASSIGNMENTS);

    let value = parse_written(&file);
    assert_eq!(value["edges_assignment"][4], "F");
    assert_eq!(value["edges_assignment"][5], "U");
}

#[test]
fn writer_preserves_optional_typed_metadata() {
    let mut file = minimal_file();
    file.file_author = Some("author".to_string());
    file.file_title = Some("title".to_string());
    file.file_description = Some("description".to_string());
    file.root.frame_title = Some("root title".to_string());
    file.root.frame_description = Some("root description".to_string());

    let value = parse_written(&file);
    assert_eq!(value["file_author"], "author");
    assert_eq!(value["file_title"], "title");
    assert_eq!(value["file_description"], "description");
    assert_eq!(value["frame_title"], "root title");
    assert_eq!(value["frame_description"], "root description");
}

#[test]
fn writer_keeps_a_linear_parent_chain_without_expanding_inherited_fields() {
    let file = fixture(LINEAR_STEPS);

    let value = parse_written(&file);
    let frames = value["file_frames"].as_array().expect("2つのstep frame");
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["frame_parent"], 0);
    assert_eq!(frames[1]["frame_parent"], 1);
    assert_eq!(frames[0]["frame_inherit"], true);
    assert_eq!(frames[1]["frame_inherit"], true);
    assert!(
        frames[0].get("vertices_coords").is_none(),
        "writerは欠けたfieldを勝手にsnapshot展開しない"
    );
    assert!(
        frames[1].get("edges_vertices").is_none(),
        "writerは継承されるedge topologyを勝手に複製しない"
    );
}

#[test]
fn writer_preserves_face_orders_and_their_face_index_definition() {
    let file = minimal_file();

    let value = parse_written(&file);
    assert_eq!(value["faces_vertices"], json!([[0, 1, 2], [0, 2, 3]]));
    assert_eq!(value["faceOrders"], json!([[0, 1, 1]]));
}

#[test]
fn writer_rejects_other_assignment_with_its_edge_path() {
    let mut file = fixture(FU_ASSIGNMENTS);
    file.root
        .edges_assignment
        .as_mut()
        .expect("assignmentを持つ")[4] = FoldAssignment::Other("C".to_string());

    let error = write_fold_1_2(&file).expect_err("C assignmentを黙って出力しない");
    assert!(error.issues.iter().any(|issue| {
        issue.path == "$.edges_assignment[4]"
            && issue.code == FoldIssueCode::UnsupportedField
            && issue.original_value == Some(json!("C"))
    }));
}

#[test]
fn writer_rejects_every_unknown_field_instead_of_dropping_it() {
    let mut file = minimal_file();
    file.extra_fields
        .insert("custom_file".to_string(), json!({ "kept": true }));
    file.extra_fields
        .insert("vendor.extension".to_string(), json!("value"));
    file.extra_fields.insert(String::new(), json!("empty"));
    file.extra_fields
        .insert("1vendor".to_string(), json!("numeric prefix"));
    file.root
        .extra_fields
        .insert("custom_frame".to_string(), json!(7));

    let error = write_fold_1_2(&file).expect_err("unknown fieldを黙って落とさない");
    let paths = error
        .issues
        .iter()
        .filter(|issue| issue.code == FoldIssueCode::UnsupportedField)
        .map(|issue| issue.path.as_str())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"$.custom_file"));
    assert!(paths.contains(&"$.custom_frame"));
    assert!(paths.contains(&"$[\"vendor.extension\"]"));
    assert!(paths.contains(&"$[\"\"]"));
    assert!(paths.contains(&"$[\"1vendor\"]"));
}

#[test]
fn writer_rejects_non_1_2_spec_even_when_the_typed_value_bypasses_parser() {
    for version in [1.1, 2.0] {
        let mut file = minimal_file();
        file.file_spec = version;

        let error = write_fold_1_2(&file).expect_err("公開writerもfile_spec 1.2だけを受ける");
        assert!(error.issues.iter().any(|issue| {
            issue.path == "$.file_spec"
                && issue.code == FoldIssueCode::InvalidValue
                && issue.original_value.as_ref().and_then(Value::as_f64) == Some(version)
        }));
    }
}

#[test]
fn writer_rejects_non_finite_numbers_without_panicking() {
    let mut non_finite_spec = minimal_file();
    non_finite_spec.file_spec = f64::NAN;

    let mut non_finite_coordinate = minimal_file();
    non_finite_coordinate
        .root
        .vertices_coords
        .as_mut()
        .expect("座標を持つ")[2][1] = f64::INFINITY;

    let mut non_finite_angle = minimal_file();
    non_finite_angle
        .root
        .edges_fold_angle
        .as_mut()
        .expect("角度を持つ")[3] = Some(f64::NEG_INFINITY);

    for (file, path) in [
        (non_finite_spec, "$.file_spec"),
        (non_finite_coordinate, "$.vertices_coords[2][1]"),
        (non_finite_angle, "$.edges_foldAngle[3]"),
    ] {
        let error = write_fold_1_2(&file).expect_err("非finite値はerrorとして返す");
        assert!(
            error
                .issues
                .iter()
                .any(|issue| { issue.path == path && issue.code == FoldIssueCode::InvalidValue }),
            "拒否した値のpathを示す: expected={path}, issues={:?}",
            error.issues
        );
    }
}

#[test]
fn writer_rejects_face_orders_without_faces_vertices() {
    let mut file = minimal_file();
    file.root.faces_vertices = None;
    file.root.face_orders = Some(vec![vec![0, 1, 1]]);

    let error =
        write_fold_1_2(&file).expect_err("face indexの定義がないfaceOrdersを成功扱いしない");
    assert!(error.issues.iter().any(|issue| {
        issue.path == "$.faceOrders" && issue.code == FoldIssueCode::UnrepresentableFaceOrders
    }));
}
