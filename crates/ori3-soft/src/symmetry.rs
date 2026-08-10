//! 共有頂点座標へ、指定軸まわりの180度回転対称を決定的に適用する後処理。
//!
//! 各頂点対は、元の2点からの二乗移動量が最小になる対称位置へ平均する。対どうしで
//! 頂点を共有する指定は拒否し、全入力と全出力を検査してから一括で書き戻す。

use std::collections::BTreeSet;
use std::fmt;

use glam::DVec3;

/// 半回転対称化の中心軸。
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HalfTurnSymmetrySettings {
    /// 対称軸上の任意の点。
    pub center: [f64; 3],
    /// 対称軸の方向。長さは問わない。
    pub axis: [f64; 3],
}

/// 1回の半回転対称化で実際に行った処理の要約。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HalfTurnSymmetryReport {
    /// 処理した頂点対の数。
    pub pairs: usize,
    /// 対に含まれた頂点数。正常終了時は常に`pairs * 2`。
    pub selected_vertices: usize,
    /// 位置が1ビットでも変わった頂点数。
    pub moved_vertices: usize,
    /// 選択頂点の最大移動距離。
    pub max_displacement: f64,
}

/// 半回転対称化を安全に適用できなかった理由。
///
/// エラー時は[`enforce_half_turn_symmetry`]へ渡した座標を一切変更しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HalfTurnSymmetryError {
    /// 中心または軸方向にNaN・無限大がある。
    NonFiniteSettings,
    /// 対称軸が零ベクトルである。
    DegenerateAxis,
    /// 同じ頂点が同一対または複数の対に指定された。
    RepeatedVertex { vertex: u32 },
    /// 頂点番号が座標配列の範囲外である。
    VertexOutOfBounds { vertex: u32, vertex_count: usize },
    /// 選択頂点にNaNまたは無限大がある。
    NonFiniteVertex { vertex: u32 },
    /// 有限な入力から有限な出力を作れなかった。
    NumericalOverflow { vertex: u32 },
}

impl fmt::Display for HalfTurnSymmetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteSettings => write!(f, "半回転対称の指定に有限でない値があります"),
            Self::DegenerateAxis => write!(f, "半回転対称の軸方向を決められません"),
            Self::RepeatedVertex { vertex } => {
                write!(f, "頂点{vertex}が半回転対称の複数箇所に指定されています")
            }
            Self::VertexOutOfBounds {
                vertex,
                vertex_count,
            } => write!(
                f,
                "半回転対称の頂点{vertex}は座標の範囲外です(頂点数{vertex_count})"
            ),
            Self::NonFiniteVertex { vertex } => {
                write!(f, "半回転対称の頂点{vertex}に有限でない座標があります")
            }
            Self::NumericalOverflow { vertex } => {
                write!(f, "頂点{vertex}の半回転対称化が数値範囲を超えました")
            }
        }
    }
}

impl std::error::Error for HalfTurnSymmetryError {}

fn all_finite(values: [f64; 3]) -> bool {
    values.into_iter().all(f64::is_finite)
}

/// 成分の大きさにかかわらず、有限な非零ベクトルを安全に正規化する。
fn unit(v: DVec3) -> Option<DVec3> {
    if !v.is_finite() {
        return None;
    }
    let scale = v.abs().max_element();
    if scale == 0.0 {
        return None;
    }
    let scaled = v / scale;
    let length = scaled.length();
    (length.is_finite() && length > 0.0).then(|| scaled / length)
}

/// `center + axis * t` のまわりに点を180度回転する。
fn half_turn(point: DVec3, center: DVec3, axis: DVec3) -> Option<DVec3> {
    let relative = point - center;
    if !relative.is_finite() {
        return None;
    }
    let axial = relative.dot(axis);
    if !axial.is_finite() {
        return None;
    }
    let parallel = axis * axial;
    let perpendicular = relative - parallel;
    let rotated = center + parallel - perpendicular;
    rotated.is_finite().then_some(rotated)
}

/// 各頂点対を、指定軸まわりの180度回転対称となる最近接位置へ補正する。
///
/// 対を`[va, vb]`とし、半回転を`R`とすると、補正後は
/// `a = (va + R(vb)) / 2`, `b = R(a)`。これは`b = R(a)`を満たす点のうち、
/// 元の2点からの二乗移動量を最小にする。和のオーバーフローを避けるため、実装では
/// `va / 2 + R(vb) / 2`として同じ平均を求める。
///
/// - 対の向きと対リストの順序は結果に影響しない。
/// - 対に含まれない頂点は1ビットも変更しない。
/// - 同じ頂点を2回指定すると曖昧な逐次補正をせずエラーにする。
/// - 全補正位置を検査してから書き戻すため、エラーは原子的。
pub fn enforce_half_turn_symmetry(
    positions: &mut [[f64; 3]],
    pairs: &[[u32; 2]],
    settings: &HalfTurnSymmetrySettings,
) -> Result<HalfTurnSymmetryReport, HalfTurnSymmetryError> {
    if !all_finite(settings.center) || !all_finite(settings.axis) {
        return Err(HalfTurnSymmetryError::NonFiniteSettings);
    }
    let axis = unit(DVec3::from(settings.axis)).ok_or(HalfTurnSymmetryError::DegenerateAxis)?;
    let center = DVec3::from(settings.center);

    let mut seen = BTreeSet::new();
    let mut canonical = Vec::with_capacity(pairs.len());
    for &[first, second] in pairs {
        for vertex in [first, second] {
            if !seen.insert(vertex) {
                return Err(HalfTurnSymmetryError::RepeatedVertex { vertex });
            }
            let Some(position) = positions.get(vertex as usize) else {
                return Err(HalfTurnSymmetryError::VertexOutOfBounds {
                    vertex,
                    vertex_count: positions.len(),
                });
            };
            if !all_finite(*position) {
                return Err(HalfTurnSymmetryError::NonFiniteVertex { vertex });
            }
        }
        canonical.push(if first < second {
            [first, second]
        } else {
            [second, first]
        });
    }

    let mut updates = Vec::with_capacity(canonical.len());
    for [first, second] in canonical {
        let original_a = DVec3::from(positions[first as usize]);
        let original_b = DVec3::from(positions[second as usize]);
        let rotated_b = half_turn(original_b, center, axis)
            .ok_or(HalfTurnSymmetryError::NumericalOverflow { vertex: second })?;
        let corrected_a = original_a * 0.5 + rotated_b * 0.5;
        if !corrected_a.is_finite() {
            return Err(HalfTurnSymmetryError::NumericalOverflow { vertex: first });
        }
        let corrected_b = half_turn(corrected_a, center, axis)
            .ok_or(HalfTurnSymmetryError::NumericalOverflow { vertex: second })?;
        let displacement_a = (corrected_a - original_a).length();
        let displacement_b = (corrected_b - original_b).length();
        if !displacement_a.is_finite() || !displacement_b.is_finite() {
            return Err(HalfTurnSymmetryError::NumericalOverflow { vertex: first });
        }
        updates.push((
            [first, second],
            [corrected_a, corrected_b],
            [displacement_a, displacement_b],
        ));
    }

    let mut report = HalfTurnSymmetryReport {
        pairs: pairs.len(),
        selected_vertices: pairs.len() * 2,
        ..HalfTurnSymmetryReport::default()
    };
    for ([first, second], corrected, displacement) in updates {
        for ((vertex, point), distance) in
            [first, second].into_iter().zip(corrected).zip(displacement)
        {
            if positions[vertex as usize] != point.to_array() {
                report.moved_vertices += 1;
            }
            report.max_displacement = report.max_displacement.max(distance);
            positions[vertex as usize] = point.to_array();
        }
    }
    Ok(report)
}
