//! `FoldFile`をFOLD 1.2 JSONへ書き出す。

use serde_json::{Map, Number, Value};

use super::types::{
    FoldAssignment, FoldFile, FoldFrame, FoldIssue, FoldIssueCode, FoldIssueSeverity,
    FoldWriteError,
};
use super::validation::validate_fold_1_2;

const FOLD_SPEC_VERSION: f64 = 1.2;

/// typed FOLDを、読みやすく決定的なFOLD 1.2 JSONへ変換する。
///
/// parserを通らず直接組み立てた値も受け取れる公開境界なので、writer自身も
/// unknown field、未対応assignment、非finite値を検査する。不正な値はJSONの
/// `null`等へ置き換えず、path付きの[`FoldWriteError`]として返す。
pub fn write_fold_1_2(file: &FoldFile) -> Result<String, FoldWriteError> {
    let value = fold_value(file)?;
    serde_json::to_string_pretty(&value).map_err(|error| FoldWriteError {
        message: format!("FOLD 1.2 限定をJSONへ変換できませんでした: {error}"),
        issues: vec![write_issue(
            FoldIssueCode::InvalidValue,
            "$",
            format!("JSONへ変換できませんでした: {error}"),
            None,
        )],
    })
}

/// typed FOLDのfieldを、root frameをtop-levelへ展開したJSON valueへ変換する。
///
/// canonicalizerからも利用できるようcrate内へ公開するが、この関数自身は
/// canonicalizeしない。頂点・辺・frame・face orderの配列順を入力のまま保つ。
pub(crate) fn fold_value(file: &FoldFile) -> Result<Value, FoldWriteError> {
    let mut issues = validate_fold_1_2(file).errors;
    collect_writer_issues(file, &mut issues);
    sort_and_deduplicate_issues(&mut issues);
    if !issues.is_empty() {
        return Err(FoldWriteError {
            message: format!(
                "FOLD 1.2 限定を書き出せませんでした: {}件の問題があります",
                issues.len()
            ),
            issues,
        });
    }

    let mut object = Map::new();
    object.insert(
        "file_spec".to_string(),
        finite_number(file.file_spec, "$.file_spec")?,
    );
    insert_optional_string(&mut object, "file_creator", &file.file_creator);
    insert_optional_string(&mut object, "file_author", &file.file_author);
    insert_optional_string(&mut object, "file_title", &file.file_title);
    insert_optional_string(&mut object, "file_description", &file.file_description);
    object.insert("file_classes".to_string(), string_array(&file.file_classes));
    insert_frame_fields(&mut object, &file.root, "$")?;

    let frames = file
        .file_frames
        .iter()
        .enumerate()
        .map(|(index, frame)| frame_value(frame, &format!("$.file_frames[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    object.insert("file_frames".to_string(), Value::Array(frames));

    Ok(Value::Object(object))
}

fn collect_writer_issues(file: &FoldFile, issues: &mut Vec<FoldIssue>) {
    if !file.file_spec.is_finite() {
        issues.push(write_issue(
            FoldIssueCode::InvalidValue,
            "$.file_spec",
            "file_specはfiniteな数でなければなりません",
            None,
        ));
    } else if file.file_spec != FOLD_SPEC_VERSION {
        issues.push(write_issue(
            FoldIssueCode::InvalidValue,
            "$.file_spec",
            format!(
                "FOLD 1.2 限定を書き出せるfile_specは{FOLD_SPEC_VERSION}です（指定: {}）",
                file.file_spec
            ),
            Some(Value::from(file.file_spec)),
        ));
    }

    collect_extra_field_issues("$", &file.extra_fields, issues);
    collect_frame_issues(&file.root, "$", issues);
    for (index, frame) in file.file_frames.iter().enumerate() {
        collect_frame_issues(frame, &format!("$.file_frames[{index}]"), issues);
    }
}

fn collect_frame_issues(frame: &FoldFrame, prefix: &str, issues: &mut Vec<FoldIssue>) {
    collect_extra_field_issues(prefix, &frame.extra_fields, issues);

    if let Some(vertices) = &frame.vertices_coords {
        for (vertex_index, coords) in vertices.iter().enumerate() {
            for (component_index, coordinate) in coords.iter().enumerate() {
                if !coordinate.is_finite() {
                    issues.push(write_issue(
                        FoldIssueCode::InvalidValue,
                        format!("{prefix}.vertices_coords[{vertex_index}][{component_index}]"),
                        "頂点座標はfiniteな数でなければなりません",
                        None,
                    ));
                }
            }
        }
    }

    if let Some(assignments) = &frame.edges_assignment {
        for (edge_index, assignment) in assignments.iter().enumerate() {
            if let FoldAssignment::Other(code) = assignment {
                issues.push(write_issue(
                    FoldIssueCode::UnsupportedField,
                    format!("{prefix}.edges_assignment[{edge_index}]"),
                    format!("assignment「{code}」はFOLD 1.2 限定の対象外です"),
                    Some(Value::String(code.clone())),
                ));
            }
        }
    }

    if let Some(angles) = &frame.edges_fold_angle {
        for (edge_index, angle) in angles.iter().enumerate() {
            if angle.is_some_and(|value| !value.is_finite()) {
                issues.push(write_issue(
                    FoldIssueCode::InvalidValue,
                    format!("{prefix}.edges_foldAngle[{edge_index}]"),
                    "折り角はnullまたはfiniteな数でなければなりません",
                    None,
                ));
            }
        }
    }
}

fn collect_extra_field_issues(
    prefix: &str,
    extra_fields: &std::collections::BTreeMap<String, Value>,
    issues: &mut Vec<FoldIssue>,
) {
    for (name, value) in extra_fields {
        issues.push(write_issue(
            FoldIssueCode::UnsupportedField,
            object_path(prefix, name),
            format!("field「{name}」はFOLD 1.2 限定の対象外です"),
            Some(value.clone()),
        ));
    }
}

fn object_path(parent: &str, key: &str) -> String {
    let mut characters = key.chars();
    let identifier = characters
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if identifier {
        format!("{parent}.{key}")
    } else {
        let escaped = serde_json::to_string(key).unwrap_or_else(|_| "\"?\"".to_string());
        format!("{parent}[{escaped}]")
    }
}

fn frame_value(frame: &FoldFrame, prefix: &str) -> Result<Value, FoldWriteError> {
    let mut object = Map::new();
    insert_frame_fields(&mut object, frame, prefix)?;
    Ok(Value::Object(object))
}

fn insert_frame_fields(
    object: &mut Map<String, Value>,
    frame: &FoldFrame,
    prefix: &str,
) -> Result<(), FoldWriteError> {
    insert_optional_string(object, "frame_title", &frame.frame_title);
    insert_optional_string(object, "frame_description", &frame.frame_description);
    object.insert(
        "frame_classes".to_string(),
        string_array(&frame.frame_classes),
    );
    object.insert(
        "frame_attributes".to_string(),
        string_array(&frame.frame_attributes),
    );
    if let Some(parent) = frame.frame_parent {
        object.insert("frame_parent".to_string(), Value::from(parent));
    }
    if let Some(inherit) = frame.frame_inherit {
        object.insert("frame_inherit".to_string(), Value::Bool(inherit));
    }

    if let Some(vertices) = &frame.vertices_coords {
        let mut rows = Vec::with_capacity(vertices.len());
        for (vertex_index, coords) in vertices.iter().enumerate() {
            let mut values = Vec::with_capacity(coords.len());
            for (component_index, coordinate) in coords.iter().enumerate() {
                values.push(finite_number(
                    *coordinate,
                    &format!("{prefix}.vertices_coords[{vertex_index}][{component_index}]"),
                )?);
            }
            rows.push(Value::Array(values));
        }
        object.insert("vertices_coords".to_string(), Value::Array(rows));
    }

    if let Some(edges) = &frame.edges_vertices {
        object.insert("edges_vertices".to_string(), usize_rows(edges));
    }
    if let Some(assignments) = &frame.edges_assignment {
        object.insert(
            "edges_assignment".to_string(),
            Value::Array(
                assignments
                    .iter()
                    .map(|assignment| Value::String(assignment.code().to_string()))
                    .collect(),
            ),
        );
    }
    if let Some(angles) = &frame.edges_fold_angle {
        let values = angles
            .iter()
            .enumerate()
            .map(|(edge_index, angle)| match angle {
                Some(value) => {
                    finite_number(*value, &format!("{prefix}.edges_foldAngle[{edge_index}]"))
                }
                None => Ok(Value::Null),
            })
            .collect::<Result<Vec<_>, _>>()?;
        object.insert("edges_foldAngle".to_string(), Value::Array(values));
    }
    if let Some(faces) = &frame.faces_vertices {
        object.insert("faces_vertices".to_string(), usize_rows(faces));
    }
    if let Some(face_orders) = &frame.face_orders {
        object.insert(
            "faceOrders".to_string(),
            Value::Array(
                face_orders
                    .iter()
                    .map(|row| Value::Array(row.iter().copied().map(Value::from).collect()))
                    .collect(),
            ),
        );
    }

    Ok(())
}

fn finite_number(value: f64, path: &str) -> Result<Value, FoldWriteError> {
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| FoldWriteError {
            message: format!("FOLD 1.2 限定を書き出せませんでした: {path}が非finiteです"),
            issues: vec![write_issue(
                FoldIssueCode::InvalidValue,
                path,
                "値はfiniteな数でなければなりません",
                None,
            )],
        })
}

fn usize_rows(rows: &[Vec<usize>]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| Value::Array(row.iter().copied().map(Value::from).collect()))
            .collect(),
    )
}

fn string_array(values: &[String]) -> Value {
    Value::Array(values.iter().cloned().map(Value::String).collect())
}

fn insert_optional_string(object: &mut Map<String, Value>, name: &str, value: &Option<String>) {
    if let Some(value) = value {
        object.insert(name.to_string(), Value::String(value.clone()));
    }
}

fn write_issue(
    code: FoldIssueCode,
    path: impl Into<String>,
    message: impl Into<String>,
    original_value: Option<Value>,
) -> FoldIssue {
    FoldIssue {
        severity: FoldIssueSeverity::Error,
        code,
        path: path.into(),
        message: message.into(),
        original_value,
    }
}

fn sort_and_deduplicate_issues(issues: &mut Vec<FoldIssue>) {
    issues.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| issue_code_order(left.code).cmp(&issue_code_order(right.code)))
            .then_with(|| left.message.cmp(&right.message))
    });
    issues.dedup();
}

fn issue_code_order(code: FoldIssueCode) -> u8 {
    match code {
        FoldIssueCode::AssignmentDowngradedToAux => 0,
        FoldIssueCode::UnsupportedField => 1,
        FoldIssueCode::UnsupportedGeometry => 2,
        FoldIssueCode::NonLinearFrames => 3,
        FoldIssueCode::UnrepresentableFaceOrders => 4,
        FoldIssueCode::InvalidTopology => 5,
        FoldIssueCode::MissingRequiredField => 6,
        FoldIssueCode::InvalidValue => 7,
    }
}
