use std::cmp::Ordering;

use serde_json::{Map, Number, Value};

use super::types::{
    FoldComparison, FoldComparisonOptions, FoldDifference, FoldFile, FoldWriteError,
};
use super::writer::fold_value;

/// 限定profileを、object key順と`faceOrders`集合を正規化したcompact JSONにする。
///
/// 頂点・辺のarray順はFOLD内のindexそのものであるため並べ替えない。edge topologyと
/// assignmentはそのindex順でexact比較し、座標・角度だけは[`compare_fold_1_2`]で
/// 明示epsilonを使う。
pub fn canonicalize_fold_1_2(file: &FoldFile) -> Result<String, FoldWriteError> {
    let value = canonical_value(fold_value(file)?);
    serde_json::to_string(&value).map_err(|error| FoldWriteError {
        message: format!("FOLD 1.2 限定JSONをcanonical化できませんでした: {error}"),
        issues: Vec::new(),
    })
}

/// 2つの限定profileを、topology等はexact、座標・角度は指定epsilonで比較する。
pub fn compare_fold_1_2(
    left: &FoldFile,
    right: &FoldFile,
    options: FoldComparisonOptions,
) -> Result<FoldComparison, FoldWriteError> {
    let mut comparison = FoldComparison::default();
    if !options.coordinate_epsilon.is_finite() || options.coordinate_epsilon < 0.0 {
        comparison.differences.push(FoldDifference {
            path: "$".to_string(),
            message: "coordinate_epsilonは有限の0以上でなければなりません".to_string(),
            left: None,
            right: None,
        });
        return Ok(comparison);
    }
    if !options.angle_epsilon_deg.is_finite() || options.angle_epsilon_deg < 0.0 {
        comparison.differences.push(FoldDifference {
            path: "$".to_string(),
            message: "angle_epsilon_degは有限の0以上でなければなりません".to_string(),
            left: None,
            right: None,
        });
        return Ok(comparison);
    }

    let left_value = canonical_value(fold_value(left)?);
    let right_value = canonical_value(fold_value(right)?);
    compare_values(
        "$",
        &left_value,
        &right_value,
        options,
        &mut comparison.differences,
    );
    Ok(comparison)
}

fn canonical_value(value: Value) -> Value {
    match value {
        Value::Object(map) => canonical_object(map),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(canonical_value).collect::<Vec<_>>())
        }
        Value::Number(number) if number.as_f64() == Some(0.0) => Value::Number(Number::from(0)),
        other => other,
    }
}

fn canonical_object(mut map: Map<String, Value>) -> Value {
    if let Some(face_orders) = map.remove("faceOrders") {
        map.insert("faceOrders".to_string(), canonical_face_orders(face_orders));
    }

    let mut entries = map.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut sorted = Map::new();
    for (key, value) in entries {
        sorted.insert(key, canonical_value(value));
    }
    Value::Object(sorted)
}

fn canonical_face_orders(value: Value) -> Value {
    let Value::Array(rows) = value else {
        return canonical_value(value);
    };
    let mut rows = rows
        .into_iter()
        .map(|row| match row {
            Value::Array(mut triple) if triple.len() == 3 => {
                if triple[2].as_i64() == Some(-1) {
                    triple.swap(0, 1);
                    triple[2] = Value::Number(Number::from(1));
                }
                Value::Array(triple.into_iter().map(canonical_value).collect())
            }
            other => canonical_value(other),
        })
        .collect::<Vec<_>>();
    rows.sort_by(compare_json_values);
    rows.dedup();
    Value::Array(rows)
}

fn compare_json_values(left: &Value, right: &Value) -> Ordering {
    serde_json::to_string(left)
        .unwrap_or_default()
        .cmp(&serde_json::to_string(right).unwrap_or_default())
}

fn compare_values(
    path: &str,
    left: &Value,
    right: &Value,
    options: FoldComparisonOptions,
    differences: &mut Vec<FoldDifference>,
) {
    match (left, right) {
        (Value::Object(left_map), Value::Object(right_map)) => {
            for (key, left_value) in left_map {
                let child_path = object_path(path, key);
                if let Some(right_value) = right_map.get(key) {
                    compare_values(&child_path, left_value, right_value, options, differences);
                } else {
                    differences.push(FoldDifference {
                        path: child_path,
                        message: "右側にfieldがありません".to_string(),
                        left: Some(left_value.clone()),
                        right: None,
                    });
                }
            }
            for (key, right_value) in right_map {
                if !left_map.contains_key(key) {
                    differences.push(FoldDifference {
                        path: object_path(path, key),
                        message: "左側にfieldがありません".to_string(),
                        left: None,
                        right: Some(right_value.clone()),
                    });
                }
            }
        }
        (Value::Array(left_values), Value::Array(right_values)) => {
            if left_values.len() != right_values.len() {
                differences.push(FoldDifference {
                    path: path.to_string(),
                    message: format!(
                        "array長が異なります: {} != {}",
                        left_values.len(),
                        right_values.len()
                    ),
                    left: Some(Value::from(left_values.len())),
                    right: Some(Value::from(right_values.len())),
                });
            }
            for (index, (left_value, right_value)) in
                left_values.iter().zip(right_values).enumerate()
            {
                compare_values(
                    &format!("{path}[{index}]"),
                    left_value,
                    right_value,
                    options,
                    differences,
                );
            }
        }
        (Value::Number(left_number), Value::Number(right_number)) => {
            if let Some(epsilon) = numeric_epsilon(path, options) {
                let left_number = left_number.as_f64();
                let right_number = right_number.as_f64();
                match (left_number, right_number) {
                    (Some(left_number), Some(right_number))
                        if (left_number - right_number).abs() <= epsilon => {}
                    (Some(left_number), Some(right_number)) => differences.push(FoldDifference {
                        path: path.to_string(),
                        message: format!(
                            "数値差{:e}が許容差{epsilon:e}を超えます",
                            (left_number - right_number).abs()
                        ),
                        left: Some(left.clone()),
                        right: Some(right.clone()),
                    }),
                    _ => push_exact_difference(path, left, right, differences),
                }
            } else if left != right {
                push_exact_difference(path, left, right, differences);
            }
        }
        _ if left == right => {}
        _ => push_exact_difference(path, left, right, differences),
    }
}

fn numeric_epsilon(path: &str, options: FoldComparisonOptions) -> Option<f64> {
    if path.contains(".vertices_coords[") {
        Some(options.coordinate_epsilon)
    } else if path.contains(".edges_foldAngle[") {
        Some(options.angle_epsilon_deg)
    } else {
        None
    }
}

fn push_exact_difference(
    path: &str,
    left: &Value,
    right: &Value,
    differences: &mut Vec<FoldDifference>,
) {
    differences.push(FoldDifference {
        path: path.to_string(),
        message: "exact値が異なります".to_string(),
        left: Some(left.clone()),
        right: Some(right.clone()),
    });
}

fn object_path(parent: &str, key: &str) -> String {
    if key
        .chars()
        .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        format!("{parent}.{key}")
    } else {
        let escaped = serde_json::to_string(key).unwrap_or_else(|_| "\"?\"".to_string());
        format!("{parent}[{escaped}]")
    }
}
