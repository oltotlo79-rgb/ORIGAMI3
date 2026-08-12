//! 正方形の材料座標を、4回対称なローズの螺旋曲面へ写す。
//!
//! 半径には中心からのユークリッド距離ではなく正方形の Minkowski 半径
//! `max(|x|, |y|)` を使う。したがって紙の4辺はすべて同じ外周になり、紙の角だけが
//! 不自然に張り出さない。2組の花びら先端は、材料上の偏角を単調な区分線形写像で
//! 45度交互の軌道へ送る。角度写像と半径写像がともに単調なので、同じ上面座標へ
//! 異なる材料点を押し込むことがない。

use std::collections::BTreeSet;
use std::fmt;

use glam::{DVec2, DVec3};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FourfoldPetalOrbit {
    /// 型紙上でこの4枚を代表する先端の偏角。
    pub material_phase_radians: f64,
    /// 完成形でこの4枚を代表する先端の偏角。
    pub phase_radians: f64,
    /// 外形を外へ出す量。0ならこの組をまだカールしない。
    pub radial_displacement: f64,
    /// 花びら先端を持ち上げる量。
    pub lift: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RadialSpiralSettings {
    /// 型紙上の回転中心。
    pub material_center: [f64; 2],
    /// 出力曲面の中心。zは花底の基準高さ。
    pub world_center: [f64; 3],
    /// 型紙中心から正方形の辺までの距離。
    pub material_half_extent: f64,
    /// 中央菱形先端の正方形半径(紙の辺を1とする)。
    pub core_material_radius: f64,
    /// 花底の付け根の正方形半径。
    pub root_material_radius: f64,
    /// 中央菱形先端の完成半径。
    pub core_radius: f64,
    /// 花底を円筒にした部分の完成半径。
    pub cylinder_radius: f64,
    /// 花びら間の谷における完成外周半径。
    pub boundary_radius: f64,
    /// 花芯の高さ。
    pub height: f64,
    /// 中心でのねじり角。外周では0へ単調に戻す。
    pub twist_radians: f64,
    /// 45度交互に並ぶ2組×4枚の花びら。
    pub petal_orbits: [FourfoldPetalOrbit; 2],
    /// 紙の4角を花底へ折り込む進行度。順に左上・左下・右下・右上。
    pub corner_tucks: [f64; 4],
    /// Each paper corner's three stationary hinge points in material coordinates.
    pub corner_hinges: [[[f64; 2]; 3]; 4],
    /// 折り込んだ紙角を花底基準から下へ送る量。
    pub corner_drop: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RadialSpiralReport {
    pub selected_vertices: usize,
    pub moved_vertices: usize,
    pub max_displacement: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadialSpiralError {
    NonFiniteSettings,
    InvalidScale,
    InvalidRadii,
    InvalidPetalOrbit,
    InvalidCornerTuck,
    VertexOutOfBounds { vertex: u32, vertex_count: usize },
    DuplicateVertex { vertex: u32 },
    NonFiniteMaterialPoint { vertex: u32 },
    NumericalOverflow { vertex: u32 },
}

impl fmt::Display for RadialSpiralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteSettings => write!(f, "ローズ曲面の設定に有限でない値があります"),
            Self::InvalidScale => write!(f, "型紙の半幅は0より大きくしてください"),
            Self::InvalidRadii => write!(f, "材料半径と完成半径は中心から外へ単調にしてください"),
            Self::InvalidPetalOrbit => {
                write!(f, "2組の花びら軌道は各90度区間の内側で単調に並べてください")
            }
            Self::InvalidCornerTuck => write!(f, "紙角の折り込み進行度は0〜1にしてください"),
            Self::VertexOutOfBounds {
                vertex,
                vertex_count,
            } => write!(
                f,
                "ローズ曲面の頂点{vertex}は座標の範囲外です(頂点数{vertex_count})"
            ),
            Self::DuplicateVertex { vertex } => {
                write!(f, "材料頂点{vertex}が重複しています")
            }
            Self::NonFiniteMaterialPoint { vertex } => {
                write!(f, "材料頂点{vertex}に有限でない座標があります")
            }
            Self::NumericalOverflow { vertex } => {
                write!(f, "材料頂点{vertex}のローズ曲面変換が数値範囲を越えました")
            }
        }
    }
}

impl std::error::Error for RadialSpiralError {}

fn settings_are_finite(settings: &RadialSpiralSettings) -> bool {
    settings.material_center.into_iter().all(f64::is_finite)
        && settings.world_center.into_iter().all(f64::is_finite)
        && settings.material_half_extent.is_finite()
        && settings.core_material_radius.is_finite()
        && settings.root_material_radius.is_finite()
        && settings.core_radius.is_finite()
        && settings.cylinder_radius.is_finite()
        && settings.boundary_radius.is_finite()
        && settings.height.is_finite()
        && settings.twist_radians.is_finite()
        && settings.corner_drop.is_finite()
        && settings.corner_tucks.into_iter().all(f64::is_finite)
        && settings
            .corner_hinges
            .into_iter()
            .flatten()
            .flatten()
            .all(f64::is_finite)
        && settings.petal_orbits.iter().all(|orbit| {
            orbit.material_phase_radians.is_finite()
                && orbit.phase_radians.is_finite()
                && orbit.radial_displacement.is_finite()
                && orbit.lift.is_finite()
        })
}

fn smooth_step(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn positive_quarter_delta(from: f64, to: f64) -> f64 {
    (to - from).rem_euclid(std::f64::consts::FRAC_PI_2)
}

/// 材料上の2先端を、完成形の2先端へ送る4回対称な単調区分線形写像。
fn warp_fourfold_angle(theta: f64, settings: &RadialSpiralSettings) -> f64 {
    let quarter = std::f64::consts::FRAC_PI_2;
    let source_start = settings.petal_orbits[0].material_phase_radians;
    let source_split = positive_quarter_delta(
        source_start,
        settings.petal_orbits[1].material_phase_radians,
    );
    let target_start = settings.petal_orbits[0].phase_radians;
    let target_split = positive_quarter_delta(target_start, settings.petal_orbits[1].phase_radians);
    let turns = ((theta - source_start) / quarter).floor();
    let local = theta - source_start - turns * quarter;
    let mapped_local = if local <= source_split {
        local * target_split / source_split
    } else {
        target_split + (local - source_split) * (quarter - target_split) / (quarter - source_split)
    };
    target_start + turns * quarter + mapped_local
}

fn baseline_radius(q: f64, settings: &RadialSpiralSettings) -> f64 {
    if q <= settings.core_material_radius {
        settings.core_radius * q / settings.core_material_radius
    } else if q <= settings.root_material_radius {
        let t = (q - settings.core_material_radius)
            / (settings.root_material_radius - settings.core_material_radius);
        settings.core_radius + (settings.cylinder_radius - settings.core_radius) * t
    } else {
        let t = (q - settings.root_material_radius) / (1.0 - settings.root_material_radius);
        settings.cylinder_radius + (settings.boundary_radius - settings.cylinder_radius) * t
    }
}

fn triangle_corner_weight(point: DVec2, corner: DVec2, a: DVec2, b: DVec2) -> Option<f64> {
    let edge_a = a - corner;
    let edge_b = b - corner;
    let offset = point - corner;
    let determinant = edge_a.perp_dot(edge_b);
    let tolerance = edge_a.length().max(edge_b.length()).max(1.0) * 256.0 * f64::EPSILON;
    if determinant.abs() <= tolerance {
        return None;
    }
    let weight_a = offset.perp_dot(edge_b) / determinant;
    let weight_b = edge_a.perp_dot(offset) / determinant;
    let weight_corner = 1.0 - weight_a - weight_b;
    (weight_a >= -tolerance && weight_b >= -tolerance && weight_corner >= -tolerance)
        .then_some(weight_corner.clamp(0.0, 1.0))
}

fn corner_hat(material: DVec2, settings: &RadialSpiralSettings) -> f64 {
    let center = DVec2::from(settings.material_center);
    let h = settings.material_half_extent;
    let corners = [
        center + DVec2::new(-h, h),
        center + DVec2::new(-h, -h),
        center + DVec2::new(h, -h),
        center + DVec2::new(h, h),
    ];
    corners
        .into_iter()
        .zip(settings.corner_hinges)
        .zip(settings.corner_tucks)
        .map(|((corner, hinges), progress)| {
            let [a, middle, b] = hinges.map(DVec2::from);
            let weight = triangle_corner_weight(material, corner, a, middle)
                .into_iter()
                .chain(triangle_corner_weight(material, corner, middle, b))
                .fold(0.0_f64, f64::max);
            progress * weight
        })
        .fold(0.0_f64, f64::max)
}

fn validate(settings: &RadialSpiralSettings) -> Result<(), RadialSpiralError> {
    if !settings_are_finite(settings) {
        return Err(RadialSpiralError::NonFiniteSettings);
    }
    if settings.material_half_extent <= 0.0 {
        return Err(RadialSpiralError::InvalidScale);
    }
    if !(0.0 < settings.core_material_radius
        && settings.core_material_radius < settings.root_material_radius
        && settings.root_material_radius < 1.0
        && 0.0 < settings.core_radius
        && settings.core_radius < settings.cylinder_radius
        && settings.cylinder_radius < settings.boundary_radius
        && settings.height > 0.0
        && settings.corner_drop >= 0.0)
    {
        return Err(RadialSpiralError::InvalidRadii);
    }
    if settings
        .corner_tucks
        .into_iter()
        .any(|progress| !(0.0..=1.0).contains(&progress))
    {
        return Err(RadialSpiralError::InvalidCornerTuck);
    }
    let center = DVec2::from(settings.material_center);
    let h = settings.material_half_extent;
    let corners = [
        center + DVec2::new(-h, h),
        center + DVec2::new(-h, -h),
        center + DVec2::new(h, -h),
        center + DVec2::new(h, h),
    ];
    if corners
        .into_iter()
        .zip(settings.corner_hinges)
        .any(|(corner, hinges)| {
            let [a, middle, b] = hinges.map(DVec2::from);
            (a - corner).perp_dot(middle - corner).abs() <= 1e-12
                || (middle - corner).perp_dot(b - corner).abs() <= 1e-12
        })
    {
        return Err(RadialSpiralError::InvalidCornerTuck);
    }
    let source_delta = positive_quarter_delta(
        settings.petal_orbits[0].material_phase_radians,
        settings.petal_orbits[1].material_phase_radians,
    );
    let target_delta = positive_quarter_delta(
        settings.petal_orbits[0].phase_radians,
        settings.petal_orbits[1].phase_radians,
    );
    let quarter = std::f64::consts::FRAC_PI_2;
    let max_radial_displacement = settings
        .petal_orbits
        .iter()
        .map(|orbit| orbit.radial_displacement)
        .fold(0.0_f64, f64::max);
    // smooth_stepの最大微分は1.5。外帯で半径が戻らないことを設定時に保証する。
    let monotone_margin =
        settings.boundary_radius - settings.cylinder_radius - 1.5 * max_radial_displacement;
    if source_delta <= 1e-12
        || source_delta >= quarter - 1e-12
        || target_delta <= 1e-12
        || target_delta >= quarter - 1e-12
        || settings
            .petal_orbits
            .iter()
            .any(|orbit| orbit.radial_displacement < 0.0 || orbit.lift < 0.0)
        || monotone_margin <= 0.0
    {
        return Err(RadialSpiralError::InvalidPetalOrbit);
    }
    Ok(())
}

/// 材料頂点を、半径順序を保つ4回対称ローズ曲面へ一括変換する。
pub fn radial_spiral_vertices(
    positions: &mut [[f64; 3]],
    material_points: &[(u32, [f64; 2])],
    settings: &RadialSpiralSettings,
) -> Result<RadialSpiralReport, RadialSpiralError> {
    validate(settings)?;

    let material_center = DVec2::from(settings.material_center);
    let world_center = DVec3::from(settings.world_center);
    let mut seen = BTreeSet::new();
    let mut updates = Vec::with_capacity(material_points.len());
    for &(vertex, material) in material_points {
        if vertex as usize >= positions.len() {
            return Err(RadialSpiralError::VertexOutOfBounds {
                vertex,
                vertex_count: positions.len(),
            });
        }
        if !seen.insert(vertex) {
            return Err(RadialSpiralError::DuplicateVertex { vertex });
        }
        let material = DVec2::from(material);
        if !material.is_finite() {
            return Err(RadialSpiralError::NonFiniteMaterialPoint { vertex });
        }
        let relative = material - material_center;
        let rho = (relative.x.abs().max(relative.y.abs()) / settings.material_half_extent)
            .clamp(0.0, 1.0);
        let theta = if relative.length_squared() > 0.0 {
            relative.y.atan2(relative.x)
        } else {
            settings.petal_orbits[0].material_phase_radians
        };

        // 角面のヒンジ頂点は動かさず、正確な紙角頂点だけを内側へ送る。
        // 面内は既存の三角形が線形補間するので、これは角面のPL hat関数になる。
        let corner_progress = corner_hat(material, settings);
        let q = rho * (1.0 - (1.0 - settings.root_material_radius) * corner_progress * rho);
        let warped = warp_fourfold_angle(theta, settings);
        let phi = warped + settings.twist_radians * (1.0 - q);
        let outer_progress = smooth_step(
            (q - settings.root_material_radius) / (1.0 - settings.root_material_radius),
        );
        let mut radial_lift = 0.0;
        let mut vertical_lift = 0.0;
        for orbit in &settings.petal_orbits {
            let lobe = (4.0 * (phi - orbit.phase_radians)).cos().max(0.0).powi(2);
            radial_lift += orbit.radial_displacement * outer_progress * lobe;
            vertical_lift += orbit.lift * outer_progress * lobe;
        }
        let mapped_radius = baseline_radius(q, settings) + radial_lift;
        let (sin, cos) = phi.sin_cos();
        let z = settings.height * (1.0 - rho) + vertical_lift
            - settings.corner_drop * corner_progress * rho * rho;
        let mapped = world_center + DVec3::new(mapped_radius * cos, mapped_radius * sin, z);
        if !mapped.is_finite() {
            return Err(RadialSpiralError::NumericalOverflow { vertex });
        }
        let original = DVec3::from(positions[vertex as usize]);
        let displacement = mapped.distance(original);
        if !displacement.is_finite() {
            return Err(RadialSpiralError::NumericalOverflow { vertex });
        }
        updates.push((vertex, mapped, displacement));
    }

    let mut report = RadialSpiralReport {
        selected_vertices: updates.len(),
        ..RadialSpiralReport::default()
    };
    for (vertex, mapped, displacement) in updates {
        if positions[vertex as usize] != mapped.to_array() {
            report.moved_vertices += 1;
        }
        report.max_displacement = report.max_displacement.max(displacement);
        positions[vertex as usize] = mapped.to_array();
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> RadialSpiralSettings {
        RadialSpiralSettings {
            material_center: [0.5, 0.5],
            world_center: [0.5, 0.5, 0.0],
            material_half_extent: 0.5,
            core_material_radius: 0.158,
            root_material_radius: 0.644,
            core_radius: 0.079,
            cylinder_radius: 0.158,
            boundary_radius: 0.34,
            height: 0.20,
            twist_radians: std::f64::consts::FRAC_PI_4,
            petal_orbits: [
                FourfoldPetalOrbit {
                    material_phase_radians: 1.72,
                    phase_radians: std::f64::consts::FRAC_PI_2,
                    radial_displacement: 0.02,
                    lift: 0.02,
                },
                FourfoldPetalOrbit {
                    material_phase_radians: 2.05,
                    phase_radians: 3.0 * std::f64::consts::FRAC_PI_4,
                    radial_displacement: 0.04,
                    lift: 0.02,
                },
            ],
            corner_tucks: [0.0; 4],
            corner_hinges: [
                [[0.239, 1.0], [0.286, 0.822], [0.0, 0.730]],
                [[0.0, 0.239], [0.178, 0.286], [0.270, 0.0]],
                [[0.761, 0.0], [0.714, 0.178], [1.0, 0.270]],
                [[1.0, 0.761], [0.822, 0.714], [0.730, 1.0]],
            ],
            corner_drop: 0.079,
        }
    }

    #[test]
    fn four_quarter_turns_remain_exactly_symmetric() {
        let material = [
            (0, [0.5, 0.75]),
            (1, [0.25, 0.5]),
            (2, [0.5, 0.25]),
            (3, [0.75, 0.5]),
        ];
        let mut positions = vec![[0.0; 3]; 4];
        radial_spiral_vertices(&mut positions, &material, &settings()).expect("4回対称曲面");
        for index in 0..4 {
            let point = DVec3::from(positions[index]);
            let next = DVec3::from(positions[(index + 1) % 4]);
            let delta = point - DVec3::new(0.5, 0.5, 0.0);
            let rotated = DVec3::new(0.5 - delta.y, 0.5 + delta.x, point.z);
            assert!(
                rotated.distance(next) < 1e-14,
                "{index}: {rotated:?}/{next:?}"
            );
        }
    }

    #[test]
    fn the_two_source_tip_orbits_become_45_degrees_apart() {
        let settings = settings();
        let inner = warp_fourfold_angle(settings.petal_orbits[0].material_phase_radians, &settings);
        let outer = warp_fourfold_angle(settings.petal_orbits[1].material_phase_radians, &settings);
        assert!((inner - std::f64::consts::FRAC_PI_2).abs() < 1e-14);
        assert!((outer - 3.0 * std::f64::consts::FRAC_PI_4).abs() < 1e-14);
    }

    #[test]
    fn only_the_requested_paper_corner_is_tucked() {
        let material = [
            (0, [0.0, 1.0]),
            (1, [0.0, 0.0]),
            (2, [1.0, 0.0]),
            (3, [1.0, 1.0]),
        ];
        let mut positions = vec![[0.0; 3]; 4];
        let mut settings = settings();
        settings.corner_tucks = [1.0, 0.0, 0.0, 0.0];
        for orbit in &mut settings.petal_orbits {
            orbit.radial_displacement = 0.0;
            orbit.lift = 0.0;
        }
        radial_spiral_vertices(&mut positions, &material, &settings).expect("紙角aだけを折る");
        let center = DVec2::new(0.5, 0.5);
        let radii = positions
            .iter()
            .map(|point| DVec2::from([point[0], point[1]]).distance(center))
            .collect::<Vec<_>>();
        assert!(radii[0] < radii[1]);
        assert!((radii[1] - radii[2]).abs() < 1e-14);
        assert!((radii[2] - radii[3]).abs() < 1e-14);
        assert!(positions[0][2] < positions[1][2]);
    }
}
