//! 2D等長変換(回転・平行移動・鏡映の組み合わせ)。

use glam::DVec2;

/// 2D等長変換: p' = R(rotation) · M(mirrored) · p + translation
/// Mは鏡映フラグ(trueならx軸反転を先に適用)
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

    /// 直線(l0,l1)に対する鏡映変換
    pub fn reflection(l0: DVec2, l1: DVec2) -> Self {
        // 直線の角度をφとすると、直線に対する鏡映の線形部分は R(2φ)·M。
        let d = l1 - l0;
        let angle = d.y.atan2(d.x);
        let mut iso = Isometry2 {
            rotation: 2.0 * angle,
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

    /// self ∘ other(otherを先に適用)
    pub fn compose(&self, other: &Isometry2) -> Isometry2 {
        // 線形部分: R(θ1)M1 · R(θ2)M2 = R(θ1 ± θ2) · M(m1 xor m2)
        // (M·R(θ) = R(-θ)·M より、m1がtrueならθ2の符号が反転する)。
        let rotation = if self.mirrored {
            self.rotation - other.rotation
        } else {
            self.rotation + other.rotation
        };
        Isometry2 {
            rotation,
            translation: self.apply(other.translation),
            mirrored: self.mirrored != other.mirrored,
        }
    }

    pub fn inverse(&self) -> Isometry2 {
        // p = L^{-1}(p' - t)。鏡映ありならL^{-1}の回転角はθのまま、なしなら-θ。
        let rotation = if self.mirrored {
            self.rotation
        } else {
            -self.rotation
        };
        let mut inv = Isometry2 {
            rotation,
            translation: DVec2::ZERO,
            mirrored: self.mirrored,
        };
        inv.translation = -inv.apply_linear(self.translation);
        inv
    }
}
