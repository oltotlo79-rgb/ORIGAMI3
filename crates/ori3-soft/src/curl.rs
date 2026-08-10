//! 折り目のない紙の端を、円筒面に沿って局所的にカールさせる後処理。
//!
//! 三角形網が面の境界で頂点を共有していれば、選択頂点を一度だけ動かすことで
//! 隣り合う三角形の接続は切れない。変形は紙を伸ばさない円筒写像で、軸上では
//! 位置と接線が元の平面へ連続する。指定角に達した先は、その地点の接線へ直線で
//! 延ばすため、角度の上限でも折れ曲がらない。

use std::collections::BTreeSet;
use std::fmt;

use glam::DVec3;

/// 折り目のない端を円筒状にカールさせる指定。
///
/// `axis_origin` と `axis_direction` が、動かさない付け根の直線を表す。
/// `toward_tip` はその直線から花びらなどの先端へ向かう、おおよその方向で、
/// 軸に垂直な成分だけを使う。正の `angle_deg` は
/// `axis_direction × toward_tip` 側、負の値はその反対側へ曲げる。
///
/// 軸から先端方向へ `radius * abs(angle_deg.to_radians())` 進むまでを円弧にし、
/// それより先は円弧の終端の接線へ滑らかに延ばす。したがって、半径と角度を
/// 変えるだけで、短い巻き込みから緩い反りまで同じ操作で表せる。
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CurlSettings {
    /// カールを開始する軸上の任意の点。
    pub axis_origin: [f64; 3],
    /// カール軸の方向。長さは問わない。
    pub axis_direction: [f64; 3],
    /// 軸から紙の先端へ向かう方向。軸に垂直でなくてもよい。
    pub toward_tip: [f64; 3],
    /// 円筒の半径。有限かつ正でなければならない。
    pub radius: f64,
    /// 先端側での最大回転角。符号がカールの表裏を決める。
    pub angle_deg: f64,
}

/// 1回のカール変形で実際に行った処理の要約。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CurlReport {
    /// 重複を除いた選択頂点数。
    pub selected_vertices: usize,
    /// 位置が1ビットでも変わった頂点数。
    pub moved_vertices: usize,
    /// 選択頂点の最大移動距離。
    pub max_displacement: f64,
}

/// カールを安全に適用できなかった理由。
///
/// エラー時は[`curl_vertices`]へ渡した網を一切変更しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurlError {
    /// 設定のいずれかにNaNまたは無限大がある。
    NonFiniteSettings,
    /// 半径が0以下である。
    InvalidRadius,
    /// カール軸が零ベクトルである。
    DegenerateAxis,
    /// 先端方向が零ベクトル、またはカール軸とほぼ平行である。
    DegenerateTipDirection,
    /// 選択頂点番号が網の範囲外である。
    VertexOutOfBounds { vertex: u32, vertex_count: usize },
    /// 入力網の選択頂点にNaNまたは無限大がある。
    NonFiniteVertex { vertex: u32 },
    /// 有限な入力から有限な出力を作れなかった。
    NumericalOverflow { vertex: u32 },
}

impl fmt::Display for CurlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteSettings => write!(f, "カールの指定に有限でない値があります"),
            Self::InvalidRadius => write!(f, "カールの半径は0より大きくしてください"),
            Self::DegenerateAxis => write!(f, "カール軸の方向を決められません"),
            Self::DegenerateTipDirection => {
                write!(f, "先端方向はカール軸と異なる向きにしてください")
            }
            Self::VertexOutOfBounds {
                vertex,
                vertex_count,
            } => write!(
                f,
                "カール対象の頂点{vertex}は網の範囲外です(頂点数{vertex_count})"
            ),
            Self::NonFiniteVertex { vertex } => {
                write!(f, "カール対象の頂点{vertex}に有限でない座標があります")
            }
            Self::NumericalOverflow { vertex } => {
                write!(f, "頂点{vertex}のカール計算が数値範囲を超えました")
            }
        }
    }
}

impl std::error::Error for CurlError {}

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

/// sin(x)/x。小さい角度でもカールの先端方向成分を失わない。
fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-4 {
        let x2 = x * x;
        1.0 - x2 / 6.0 + x2 * x2 / 120.0
    } else {
        x.sin() / x
    }
}

/// (1-cos(x))/x。直接引き算したときの小さい角度での桁落ちを避ける。
fn cosc(x: f64) -> f64 {
    if x.abs() < 1e-4 {
        let x2 = x * x;
        x * (0.5 - x2 / 24.0 + x2 * x2 / 720.0)
    } else {
        (1.0 - x.cos()) / x
    }
}

fn all_finite(values: [f64; 3]) -> bool {
    values.into_iter().all(f64::is_finite)
}

/// `positions` の選択頂点を、折り目を追加せず円筒面に沿ってカールさせる。
///
/// - 軸上と軸より付け根側の頂点は選択されていても1ビットも動かさない。
/// - `vertices` にない頂点は変更しない。
/// - 同じ頂点番号を複数回指定しても変形は1回だけで、結果は指定順に依存しない。
/// - 全入力を検査して新しい座標を全て求めてから書き戻すため、エラーは原子的。
///
/// 頂点を共有している選択境界では単一の座標だけを更新するので、隣り合う面の間に
/// 裂けは生じない。滑らかな見た目には、十分細かく分割した網へ適用する。
pub fn curl_vertices(
    positions: &mut [[f64; 3]],
    vertices: &[u32],
    settings: &CurlSettings,
) -> Result<CurlReport, CurlError> {
    if !all_finite(settings.axis_origin)
        || !all_finite(settings.axis_direction)
        || !all_finite(settings.toward_tip)
        || !settings.radius.is_finite()
        || !settings.angle_deg.is_finite()
    {
        return Err(CurlError::NonFiniteSettings);
    }
    if settings.radius <= 0.0 {
        return Err(CurlError::InvalidRadius);
    }

    let axis = unit(DVec3::from(settings.axis_direction)).ok_or(CurlError::DegenerateAxis)?;
    let raw_tip =
        unit(DVec3::from(settings.toward_tip)).ok_or(CurlError::DegenerateTipDirection)?;
    let tip_perpendicular = raw_tip - axis * raw_tip.dot(axis);
    // 1e-9 rad未満では表裏を決める法線が数値雑音に支配される。
    if tip_perpendicular.length_squared() <= ori3_model::EPS * ori3_model::EPS {
        return Err(CurlError::DegenerateTipDirection);
    }
    let toward_tip = unit(tip_perpendicular).ok_or(CurlError::DegenerateTipDirection)?;
    let curl_normal = unit(axis.cross(toward_tip)).ok_or(CurlError::DegenerateTipDirection)?;
    let origin = DVec3::from(settings.axis_origin);
    let max_angle = settings.angle_deg.to_radians();
    if !max_angle.is_finite() {
        return Err(CurlError::NonFiniteSettings);
    }

    let selected: BTreeSet<u32> = vertices.iter().copied().collect();
    for &vertex in &selected {
        let Some(position) = positions.get(vertex as usize) else {
            return Err(CurlError::VertexOutOfBounds {
                vertex,
                vertex_count: positions.len(),
            });
        };
        if !all_finite(*position) {
            return Err(CurlError::NonFiniteVertex { vertex });
        }
    }

    let abs_max_angle = max_angle.abs();
    if abs_max_angle == 0.0 {
        return Ok(CurlReport {
            selected_vertices: selected.len(),
            ..CurlReport::default()
        });
    }

    let mut updates = Vec::with_capacity(selected.len());
    for &vertex in &selected {
        let original = DVec3::from(positions[vertex as usize]);
        let relative = original - origin;
        if !relative.is_finite() {
            return Err(CurlError::NumericalOverflow { vertex });
        }
        let distance = relative.dot(toward_tip);
        let axial = relative.dot(axis);
        let normal = relative.dot(curl_normal);
        if !distance.is_finite() || !axial.is_finite() || !normal.is_finite() {
            return Err(CurlError::NumericalOverflow { vertex });
        }

        // 付け根側は厳密な素通しにし、境界の丸め誤差で継ぎ目を動かさない。
        if distance <= ori3_model::EPS {
            updates.push((vertex, original, 0.0));
            continue;
        }

        let angle_to_vertex = distance / settings.radius;
        let reaches_limit = angle_to_vertex > abs_max_angle;
        let angle = max_angle.signum() * angle_to_vertex.min(abs_max_angle);
        let arc_length = if reaches_limit {
            settings.radius * abs_max_angle
        } else {
            distance
        };
        if !angle.is_finite() || !arc_length.is_finite() {
            return Err(CurlError::NumericalOverflow { vertex });
        }

        // 円弧の中心線。sinc/cosc形にすると半径が非常に大きい緩いカールでも
        // radius * sin(distance/radius) の中間値を巨大化させずに求められる。
        let mut center =
            toward_tip * (arc_length * sinc(angle)) + curl_normal * (arc_length * cosc(angle));
        let tangent = toward_tip * angle.cos() + curl_normal * angle.sin();
        let local_normal = -toward_tip * angle.sin() + curl_normal * angle.cos();
        if reaches_limit {
            center += tangent * (distance - arc_length);
        }
        let curled = origin + axis * axial + center + local_normal * normal;
        if !curled.is_finite() {
            return Err(CurlError::NumericalOverflow { vertex });
        }
        let displacement = (curled - original).length();
        if !displacement.is_finite() {
            return Err(CurlError::NumericalOverflow { vertex });
        }
        updates.push((vertex, curled, displacement));
    }

    let mut report = CurlReport {
        selected_vertices: selected.len(),
        ..CurlReport::default()
    };
    for (vertex, curled, displacement) in updates {
        let original = DVec3::from(positions[vertex as usize]);
        if curled.to_array() != original.to_array() {
            report.moved_vertices += 1;
        }
        report.max_displacement = report.max_displacement.max(displacement);
        positions[vertex as usize] = curled.to_array();
    }
    Ok(report)
}
