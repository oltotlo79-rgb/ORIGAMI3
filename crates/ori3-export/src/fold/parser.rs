use std::collections::BTreeMap;

use serde_json::{Map, Number, Value};

use super::types::{
    FOLD_1_2_PROFILE_NAME, FoldAssignment, FoldFile, FoldFrame, FoldParseError, FoldParseErrorKind,
};

const FOLD_SPEC_VERSION: f64 = 1.2;

/// FOLD 1.2 JSONを、限定profileの検証前に使う中立な型へ読み込む。
///
/// この関数が拒否するのはJSON構文、既知fieldのJSON型、`file_spec`だけである。
/// 座標の次元、配列どうしの長さ、参照index、frameのつながり、限定profileの
/// 対応可否は値を捨てず、後段のvalidatorへ渡す。
pub fn parse_fold_1_2(json: &str) -> Result<FoldFile, FoldParseError> {
    let value: Value = serde_json::from_str(json).map_err(|error| {
        FoldParseError::new(
            FoldParseErrorKind::InvalidJson,
            "$",
            format!(
                "{FOLD_1_2_PROFILE_NAME}のJSONを読めませんでした（{}行{}列）: {error}",
                error.line(),
                error.column()
            ),
        )
    })?;

    let Value::Object(object) = value else {
        return Err(FoldParseError::new(
            FoldParseErrorKind::RootNotObject,
            "$",
            format!(
                "FOLDのrootはobjectでなければなりません（実際: {}）",
                value_kind(&value)
            ),
        ));
    };

    parse_file(object)
}

fn parse_file(mut object: Map<String, Value>) -> Result<FoldFile, FoldParseError> {
    let file_spec = match object.remove("file_spec") {
        None => {
            return Err(FoldParseError::new(
                FoldParseErrorKind::MissingField,
                "$.file_spec",
                format!("{FOLD_1_2_PROFILE_NAME}にはfile_specが必要です"),
            ));
        }
        Some(value) => finite_number(value, "$.file_spec")?,
    };

    // file_specは幾何計算値ではなくformatの識別子なので、許容差で別版を
    // 1.2として扱わず、JSON numberとして読み取った値をexactに判定する。
    if file_spec != FOLD_SPEC_VERSION {
        return Err(FoldParseError::new(
            FoldParseErrorKind::UnsupportedVersion,
            "$.file_spec",
            format!("{FOLD_1_2_PROFILE_NAME}が読めるfile_specは1.2です（指定: {file_spec}）"),
        ));
    }

    let file_creator = take_optional_string(&mut object, "file_creator", "$")?;
    let file_author = take_optional_string(&mut object, "file_author", "$")?;
    let file_title = take_optional_string(&mut object, "file_title", "$")?;
    let file_description = take_optional_string(&mut object, "file_description", "$")?;
    let file_classes = take_string_array(&mut object, "file_classes", "$")?.unwrap_or_default();
    let raw_file_frames = object.remove("file_frames");

    // FOLDではtop-level object自身もroot frameである。file metadataを取り除いた
    // 同じobjectからframe fieldを読み、frame側の値を別経路で再解釈しない。
    let mut root = parse_frame_fields(&mut object, "$")?;
    let file_frames = parse_file_frames(raw_file_frames)?;

    let mut file_extra_fields = BTreeMap::new();
    let mut root_extra_fields = BTreeMap::new();
    for (key, value) in object {
        if key.starts_with("file_") {
            file_extra_fields.insert(key, value);
        } else {
            root_extra_fields.insert(key, value);
        }
    }
    root.extra_fields = root_extra_fields;

    Ok(FoldFile {
        file_spec,
        file_creator,
        file_author,
        file_title,
        file_description,
        file_classes,
        root,
        file_frames,
        extra_fields: file_extra_fields,
    })
}

fn parse_file_frames(value: Option<Value>) -> Result<Vec<FoldFrame>, FoldParseError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Value::Array(frames) = value else {
        return Err(invalid_type("$.file_frames", "array", &value));
    };

    frames
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let path = format!("$.file_frames[{index}]");
            let Value::Object(mut object) = value else {
                return Err(invalid_type(&path, "object", &value));
            };
            let mut frame = parse_frame_fields(&mut object, &path)?;
            frame.extra_fields = object.into_iter().collect();
            Ok(frame)
        })
        .collect()
}

fn parse_frame_fields(
    object: &mut Map<String, Value>,
    path: &str,
) -> Result<FoldFrame, FoldParseError> {
    Ok(FoldFrame {
        frame_title: take_optional_string(object, "frame_title", path)?,
        frame_description: take_optional_string(object, "frame_description", path)?,
        frame_classes: take_string_array(object, "frame_classes", path)?.unwrap_or_default(),
        frame_attributes: take_string_array(object, "frame_attributes", path)?.unwrap_or_default(),
        frame_parent: take_optional_usize(object, "frame_parent", path)?,
        frame_inherit: take_optional_bool(object, "frame_inherit", path)?,
        vertices_coords: take_nested_f64_array(object, "vertices_coords", path)?,
        edges_vertices: take_nested_usize_array(object, "edges_vertices", path)?,
        edges_assignment: take_assignment_array(object, "edges_assignment", path)?,
        edges_fold_angle: take_nullable_f64_array(object, "edges_foldAngle", path)?,
        faces_vertices: take_nested_usize_array(object, "faces_vertices", path)?,
        face_orders: take_nested_i64_array(object, "faceOrders", path)?,
        extra_fields: BTreeMap::new(),
    })
}

fn take_optional_string(
    object: &mut Map<String, Value>,
    key: &str,
    parent_path: &str,
) -> Result<Option<String>, FoldParseError> {
    let Some(value) = object.remove(key) else {
        return Ok(None);
    };
    let path = field_path(parent_path, key);
    match value {
        Value::String(value) => Ok(Some(value)),
        value => Err(invalid_type(&path, "string", &value)),
    }
}

fn take_string_array(
    object: &mut Map<String, Value>,
    key: &str,
    parent_path: &str,
) -> Result<Option<Vec<String>>, FoldParseError> {
    let Some(value) = object.remove(key) else {
        return Ok(None);
    };
    let path = field_path(parent_path, key);
    let Value::Array(values) = value else {
        return Err(invalid_type(&path, "array", &value));
    };

    let values = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| match value {
            Value::String(value) => Ok(value),
            value => Err(invalid_type(&index_path(&path, index), "string", &value)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(values))
}

fn take_optional_bool(
    object: &mut Map<String, Value>,
    key: &str,
    parent_path: &str,
) -> Result<Option<bool>, FoldParseError> {
    let Some(value) = object.remove(key) else {
        return Ok(None);
    };
    let path = field_path(parent_path, key);
    match value {
        Value::Bool(value) => Ok(Some(value)),
        value => Err(invalid_type(&path, "boolean", &value)),
    }
}

fn take_optional_usize(
    object: &mut Map<String, Value>,
    key: &str,
    parent_path: &str,
) -> Result<Option<usize>, FoldParseError> {
    let Some(value) = object.remove(key) else {
        return Ok(None);
    };
    let path = field_path(parent_path, key);
    parse_usize(value, &path).map(Some)
}

fn take_nested_f64_array(
    object: &mut Map<String, Value>,
    key: &str,
    parent_path: &str,
) -> Result<Option<Vec<Vec<f64>>>, FoldParseError> {
    let Some(value) = object.remove(key) else {
        return Ok(None);
    };
    let path = field_path(parent_path, key);
    let Value::Array(rows) = value else {
        return Err(invalid_type(&path, "array", &value));
    };

    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(row_index, row)| {
            let row_path = index_path(&path, row_index);
            let Value::Array(values) = row else {
                return Err(invalid_type(&row_path, "array", &row));
            };
            values
                .into_iter()
                .enumerate()
                .map(|(column_index, value)| {
                    finite_number(value, &index_path(&row_path, column_index))
                })
                .collect()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(rows))
}

fn take_nested_usize_array(
    object: &mut Map<String, Value>,
    key: &str,
    parent_path: &str,
) -> Result<Option<Vec<Vec<usize>>>, FoldParseError> {
    let Some(value) = object.remove(key) else {
        return Ok(None);
    };
    let path = field_path(parent_path, key);
    let Value::Array(rows) = value else {
        return Err(invalid_type(&path, "array", &value));
    };

    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(row_index, row)| {
            let row_path = index_path(&path, row_index);
            let Value::Array(values) = row else {
                return Err(invalid_type(&row_path, "array", &row));
            };
            values
                .into_iter()
                .enumerate()
                .map(|(column_index, value)| {
                    parse_usize(value, &index_path(&row_path, column_index))
                })
                .collect()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(rows))
}

fn take_nested_i64_array(
    object: &mut Map<String, Value>,
    key: &str,
    parent_path: &str,
) -> Result<Option<Vec<Vec<i64>>>, FoldParseError> {
    let Some(value) = object.remove(key) else {
        return Ok(None);
    };
    let path = field_path(parent_path, key);
    let Value::Array(rows) = value else {
        return Err(invalid_type(&path, "array", &value));
    };

    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(row_index, row)| {
            let row_path = index_path(&path, row_index);
            let Value::Array(values) = row else {
                return Err(invalid_type(&row_path, "array", &row));
            };
            values
                .into_iter()
                .enumerate()
                .map(|(column_index, value)| parse_i64(value, &index_path(&row_path, column_index)))
                .collect()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(rows))
}

fn take_assignment_array(
    object: &mut Map<String, Value>,
    key: &str,
    parent_path: &str,
) -> Result<Option<Vec<FoldAssignment>>, FoldParseError> {
    let Some(value) = object.remove(key) else {
        return Ok(None);
    };
    let path = field_path(parent_path, key);
    let Value::Array(values) = value else {
        return Err(invalid_type(&path, "array", &value));
    };

    let assignments = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| match value {
            Value::String(code) => Ok(FoldAssignment::from_code(&code)),
            value => Err(invalid_type(&index_path(&path, index), "string", &value)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(assignments))
}

fn take_nullable_f64_array(
    object: &mut Map<String, Value>,
    key: &str,
    parent_path: &str,
) -> Result<Option<Vec<Option<f64>>>, FoldParseError> {
    let Some(value) = object.remove(key) else {
        return Ok(None);
    };
    let path = field_path(parent_path, key);
    let Value::Array(values) = value else {
        return Err(invalid_type(&path, "array", &value));
    };

    let values = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            if value.is_null() {
                Ok(None)
            } else {
                finite_number(value, &index_path(&path, index)).map(Some)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(values))
}

fn finite_number(value: Value, path: &str) -> Result<f64, FoldParseError> {
    let Value::Number(number) = value else {
        return Err(invalid_type(path, "number", &value));
    };
    number_as_finite_f64(&number).ok_or_else(|| {
        FoldParseError::new(
            FoldParseErrorKind::InvalidValue,
            path,
            "有限のf64で表せるnumberでなければなりません",
        )
    })
}

fn number_as_finite_f64(number: &Number) -> Option<f64> {
    number.as_f64().filter(|value| value.is_finite())
}

fn parse_usize(value: Value, path: &str) -> Result<usize, FoldParseError> {
    let Value::Number(number) = value else {
        return Err(invalid_type(path, "non-negative integer", &value));
    };
    let value = number.as_u64().ok_or_else(|| {
        FoldParseError::new(
            FoldParseErrorKind::InvalidValue,
            path,
            "0以上のJSON integerでなければなりません",
        )
    })?;
    usize::try_from(value).map_err(|_| {
        FoldParseError::new(
            FoldParseErrorKind::InvalidValue,
            path,
            "この環境でindexとして表せる範囲を超えています",
        )
    })
}

fn parse_i64(value: Value, path: &str) -> Result<i64, FoldParseError> {
    let Value::Number(number) = value else {
        return Err(invalid_type(path, "integer", &value));
    };
    number.as_i64().ok_or_else(|| {
        FoldParseError::new(
            FoldParseErrorKind::InvalidValue,
            path,
            "i64で表せるJSON integerでなければなりません",
        )
    })
}

fn invalid_type(path: &str, expected: &str, value: &Value) -> FoldParseError {
    FoldParseError::new(
        FoldParseErrorKind::InvalidType,
        path,
        format!(
            "{expected}でなければなりません（実際: {}）",
            value_kind(value)
        ),
    )
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn field_path(parent: &str, field: &str) -> String {
    format!("{parent}.{field}")
}

fn index_path(parent: &str, index: usize) -> String {
    format!("{parent}[{index}]")
}
