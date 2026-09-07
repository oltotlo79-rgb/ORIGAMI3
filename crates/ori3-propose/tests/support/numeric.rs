//! 計算小数を含む検査値の共通比較。

use serde::Serialize;
use serde_json::Value;

fn json_values_are_near(got: &Value, want: &Value, tolerance: f64) -> bool {
    match (got, want) {
        (Value::Number(got), Value::Number(want)) if got.is_f64() || want.is_f64() => {
            let (Some(got), Some(want)) = (got.as_f64(), want.as_f64()) else {
                return false;
            };
            (got - want).abs() <= tolerance
        }
        (Value::Array(got), Value::Array(want)) => {
            got.len() == want.len()
                && got
                    .iter()
                    .zip(want)
                    .all(|(got, want)| json_values_are_near(got, want, tolerance))
        }
        (Value::Object(got), Value::Object(want)) => {
            got.len() == want.len()
                && got.iter().all(|(key, got)| {
                    want.get(key)
                        .is_some_and(|want| json_values_are_near(got, want, tolerance))
                })
        }
        _ => got == want,
    }
}

/// JSONへ書ける複合値を、整数・文字列・列挙・構造は厳密に、小数だけ許容差付きで比べる。
pub fn serialized_values_are_near<T: Serialize>(got: &T, want: &T, tolerance: f64) -> bool {
    let got = serde_json::to_value(got).expect("比較する値をJSONへ変換できない");
    let want = serde_json::to_value(want).expect("比較する基準をJSONへ変換できない");
    json_values_are_near(&got, &want, tolerance)
}

/// [`serialized_values_are_near`] の表明版。
#[track_caller]
pub fn assert_serialized_values_near<T: Serialize>(got: &T, want: &T, tolerance: f64, label: &str) {
    assert!(
        serialized_values_are_near(got, want, tolerance),
        "{label}: 小数の差が許容 {tolerance:e} を超えた、または離散値・構造が変わった"
    );
}
