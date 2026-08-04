//! 2D等長変換(回転・平行移動・鏡映の組み合わせ)。

use glam::DVec2;
use std::f64::consts::TAU;

/// 2D等長変換: p' = R(rotation) · M(mirrored) · p + translation
/// Mは鏡映フラグ(trueならx軸反転を先に適用)
///
/// `derive(PartialEq)` はフィールドの完全一致比較であり、浮動小数点誤差や
/// 回転角の表現差(θとθ+2πなど)を吸収しない。意味的な同一性判定には
/// [`Isometry2::approx_eq`] を使うこと。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Isometry2 {
    pub rotation: f64,
    pub translation: DVec2,
    pub mirrored: bool,
}

impl Isometry2 {
    pub fn identity() -> Self {
        Isometry2 {
            rotation: 0.0,
            translation: DVec2::ZERO,
            mirrored: false,
        }
    }

    /// 直線(l0,l1)に対する鏡映変換。rotationは[0, 2π)に正規化される。
    pub fn reflection(l0: DVec2, l1: DVec2) -> Self {
        // 直線の角度をφとすると、直線に対する鏡映の線形部分は R(2φ)·M。
        let d = l1 - l0;
        let angle = d.y.atan2(d.x);
        let mut iso = Isometry2 {
            rotation: (2.0 * angle).rem_euclid(TAU),
            translation: DVec2::ZERO,
            mirrored: true,
        };
        // 直線上の点 l0 が不動点になるように平行移動を決める。
        iso.translation = l0 - iso.apply_linear(l0);
        iso
    }

    /// 線形部分(回転+鏡映)のみを適用する。
    fn apply_linear(&self, p: DVec2) -> DVec2 {
        let p = if self.mirrored {
            DVec2::new(p.x, -p.y)
        } else {
            p
        };
        let (sin, cos) = self.rotation.sin_cos();
        DVec2::new(cos * p.x - sin * p.y, sin * p.x + cos * p.y)
    }

    pub fn apply(&self, p: DVec2) -> DVec2 {
        self.apply_linear(p) + self.translation
    }

    /// self ∘ other(otherを先に適用)。rotationは[0, 2π)に正規化される。
    pub fn compose(&self, other: &Isometry2) -> Isometry2 {
        // 線形部分: R(θ1)M1 · R(θ2)M2 = R(θ1 ± θ2) · M(m1 xor m2)
        // (M·R(θ) = R(-θ)·M より、m1がtrueならθ2の符号が反転する)。
        let rotation = if self.mirrored {
            self.rotation - other.rotation
        } else {
            self.rotation + other.rotation
        };
        Isometry2 {
            rotation: rotation.rem_euclid(TAU),
            translation: self.apply(other.translation),
            mirrored: self.mirrored != other.mirrored,
        }
    }

    /// 逆変換。rotationは[0, 2π)に正規化される。
    pub fn inverse(&self) -> Isometry2 {
        // p = L^{-1}(p' - t)。鏡映ありならL^{-1}の回転角はθのまま、なしなら-θ。
        let rotation = if self.mirrored {
            self.rotation
        } else {
            -self.rotation
        };
        let mut inv = Isometry2 {
            rotation: rotation.rem_euclid(TAU),
            translation: DVec2::ZERO,
            mirrored: self.mirrored,
        };
        inv.translation = -inv.apply_linear(self.translation);
        inv
    }

    /// 意味的な同一性判定。回転角は2π剰余で比較し、mirroredは完全一致、
    /// 並進はeps以内の距離なら同一とみなす。
    pub fn approx_eq(&self, other: &Isometry2, eps: f64) -> bool {
        if self.mirrored != other.mirrored {
            return false;
        }
        let diff = (self.rotation - other.rotation).rem_euclid(TAU);
        let angle_diff = diff.min(TAU - diff);
        angle_diff <= eps && (self.translation - other.translation).length() <= eps
    }
}
