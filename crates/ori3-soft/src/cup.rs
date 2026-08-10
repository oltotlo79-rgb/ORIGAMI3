//! 紙の中心側を持ち上げ、底を回転対称なカップ・円筒状に整える後処理。
//!
//! 内半径までは平らな中心面として同じ高さだけ持ち上げ、内半径から外半径までを
//! C2連続な壁で元の紙へ戻す。外半径上とその外側は厳密に動かさない。形は中心から
//! の距離だけで決まるため、入力が4回回転対称なら出力も同じ対称性を保つ。

use std::collections::BTreeSet;
use std::fmt;

use glam::DVec3;

/// 中心を持ち上げて回転対称な底・壁を作る指定。
///
/// `inner_radius` 内の選択頂点へ `normal * height` を加え、そこから
/// `outer_radius` までを滑らかに元の高さへ戻す。内外半径の差を高さに対して
/// 小さくすると円筒に近い急な壁に、広くすると丸いカップ状になる。
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RadialCupSettings {
    /// 回転対称形の中心。基準平面上に置く。
    pub center: [f64; 3],
    /// 中心を持ち上げる向き。長さは問わない。
    pub normal: [f64; 3],
    /// 同じ高さで持ち上げる中心面の半径。0以上。
    pub inner_radius: f64,
    /// 元の紙へ接続する固定外周の半径。内半径より大きくする。
    pub outer_radius: f64,
    /// 中心面を持ち上げる高さ。負なら法線と反対側へくぼませる。
    pub height: f64,
}

/// 1回の回転対称変形で実際に行った処理の要約。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RadialCupReport {
    /// 重複を除いた選択頂点数。
    pub selected_vertices: usize,
    /// 位置が1ビットでも変わった頂点数。
    pub moved_vertices: usize,
    /// 選択頂点の最大移動距離。
    pub max_displacement: f64,
}

/// 回転対称変形を安全に適用できなかった理由。
///
/// エラー時は[`radial_cup_vertices`]へ渡した座標を一切変更しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadialCupError {
    /// 設定のいずれかにNaNまたは無限大がある。
    NonFiniteSettings,
    /// 内半径が負、外半径が正でない、または外半径が内半径以下である。
    InvalidRadii,
    /// 平面法線が零ベクトルである。
    DegenerateNormal,
    /// 選択頂点番号が座標配列の範囲外である。
    VertexOutOfBounds { vertex: u32, vertex_count: usize },
    /// 入力の選択頂点にNaNまたは無限大がある。
    NonFiniteVertex { vertex: u32 },
    /// 有限な入力から有限な出力を作れなかった。
    NumericalOverflow { vertex: u32 },
}

impl fmt::Display for RadialCupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteSettings => write!(f, "カップ変形の指定に有限でない値があります"),
            Self::InvalidRadii => {
                write!(f, "外半径は正で、0以上の内半径より大きくしてください")
            }
            Self::DegenerateNormal => write!(f, "カップ変形の平面法線を決められません"),
            Self::VertexOutOfBounds {
                vertex,
                vertex_count,
            } => write!(
                f,
                "カップ変形の頂点{vertex}は座標の範囲外です(頂点数{vertex_count})"
            ),
            Self::NonFiniteVertex { vertex } => {
                write!(f, "カップ変形の頂点{vertex}に有限でない座標があります")
            }
            Self::NumericalOverflow { vertex } => {
                write!(f, "頂点{vertex}のカップ変形が数値範囲を超えました")
            }
        }
    }
}

impl std::error::Error for RadialCupError {}

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

fn all_finite(values: [f64; 3]) -> bool {
    values.into_iter().all(f64::is_finite)
}

/// 0〜1を、両端で1階・2階微分が0になる形に滑らかに補間する。
fn smoother_step(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (10.0 + t * (-15.0 + 6.0 * t))
}

/// 選択頂点の中心側を持ち上げ、回転対称なカップ・円筒状の底を作る。
///
/// - 中心からの距離は`normal`に垂直な基準平面への射影で測る。
/// - 内半径内は平らな中心面、内外半径間はC2連続な壁になる。
/// - 外半径上・外側、選択されていない頂点は1ビットも動かさない。
/// - 同じ頂点番号の重複と指定順は結果へ影響しない。
/// - 全入力と全出力を検査してから書き戻すため、エラーは原子的。
///
/// 共有三角形網へ適用すれば、同じ頂点は一度だけ動くため面の境界に裂けは生じない。
pub fn radial_cup_vertices(
    positions: &mut [[f64; 3]],
    vertices: &[u32],
    settings: &RadialCupSettings,
) -> Result<RadialCupReport, RadialCupError> {
    if !all_finite(settings.center)
        || !all_finite(settings.normal)
        || !settings.inner_radius.is_finite()
        || !settings.outer_radius.is_finite()
        || !settings.height.is_finite()
    {
        return Err(RadialCupError::NonFiniteSettings);
    }
    if settings.inner_radius < 0.0
        || settings.outer_radius <= 0.0
        || settings.outer_radius <= settings.inner_radius
    {
        return Err(RadialCupError::InvalidRadii);
    }
    let normal = unit(DVec3::from(settings.normal)).ok_or(RadialCupError::DegenerateNormal)?;
    let center = DVec3::from(settings.center);

    let selected: BTreeSet<u32> = vertices.iter().copied().collect();
    for &vertex in &selected {
        let Some(position) = positions.get(vertex as usize) else {
            return Err(RadialCupError::VertexOutOfBounds {
                vertex,
                vertex_count: positions.len(),
            });
        };
        if !all_finite(*position) {
            return Err(RadialCupError::NonFiniteVertex { vertex });
        }
    }
    if settings.height == 0.0 {
        return Ok(RadialCupReport {
            selected_vertices: selected.len(),
            ..RadialCupReport::default()
        });
    }

    let width = settings.outer_radius - settings.inner_radius;
    let mut updates = Vec::with_capacity(selected.len());
    for &vertex in &selected {
        let original = DVec3::from(positions[vertex as usize]);
        let relative = original - center;
        if !relative.is_finite() {
            return Err(RadialCupError::NumericalOverflow { vertex });
        }
        let axial = relative.dot(normal);
        let planar = relative - normal * axial;
        let radius = planar.length();
        if !axial.is_finite() || !planar.is_finite() || !radius.is_finite() {
            return Err(RadialCupError::NumericalOverflow { vertex });
        }

        let weight = if radius <= settings.inner_radius {
            1.0
        } else if radius >= settings.outer_radius {
            0.0
        } else {
            1.0 - smoother_step((radius - settings.inner_radius) / width)
        };
        if weight == 0.0 {
            updates.push((vertex, original, 0.0));
            continue;
        }
        let curled = original + normal * (settings.height * weight);
        let displacement = (curled - original).length();
        if !curled.is_finite() || !displacement.is_finite() {
            return Err(RadialCupError::NumericalOverflow { vertex });
        }
        updates.push((vertex, curled, displacement));
    }

    let mut report = RadialCupReport {
        selected_vertices: selected.len(),
        ..RadialCupReport::default()
    };
    for (vertex, curled, displacement) in updates {
        let original = positions[vertex as usize];
        if curled.to_array() != original {
            report.moved_vertices += 1;
        }
        report.max_displacement = report.max_displacement.max(displacement);
        positions[vertex as usize] = curled.to_array();
    }
    Ok(report)
}
