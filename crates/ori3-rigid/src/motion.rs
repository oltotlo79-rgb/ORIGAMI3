//! 角度操作を前の姿勢から少しずつ追い、紙どうしが交差する直前で止める。

use std::collections::HashMap;

use ori3_cp::Face;
use ori3_model::{CreasePattern, Driver, EdgeId};

use crate::{SolveResult, self_intersects, solve, solve_near};

/// 小さな作品で使う目標角の刻み。通常の16ms入力はこれより小さいため1段だけになる。
const TARGET_STEP_DEG: f64 = 5.0;
/// 接触区間を安全側から詰める回数。大きな作品では1フレームの費用を抑えて2回にする。
const SMALL_BISECTIONS: usize = 3;
const LARGE_BISECTIONS: usize = 2;

/// 接触停止付き継続法の結果。
#[derive(Clone, Debug)]
pub struct MotionSolveResult {
    pub result: SolveResult,
    /// 収束失敗ではなく、紙どうしの接触を検出して安全側で止めたか。
    pub contact_stopped: bool,
}

/// 前回角から要求角までを継続法で追い、必要なら交差直前の安全解を返す。
///
/// `targets` が `Some` なら各段で [`solve_near`]、`None` なら [`solve`] を使う。
/// `prevent_contact == false` は従来関数を1回だけ呼び、結果をそのまま返す。
/// 接触判定は剛体フレームへ掛けるため、後段の表示用重なり補正とは独立している。
#[must_use]
pub fn solve_motion(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    prevent_contact: bool,
) -> MotionSolveResult {
    if !prevent_contact {
        return MotionSolveResult {
            result: solve_requested(cp, faces, drivers, targets, warm_start),
            contact_stopped: false,
        };
    }

    // warm_startは直前に表示した収束解。driverを一度外して同じ角度から解けば、
    // t=0の閉じたフレームと全ヒンジ角を得られる。初回は全角度0の平坦解になる。
    let mut accepted = solve(cp, faces, &[], warm_start);
    if !accepted.converged {
        return MotionSolveResult {
            result: accepted,
            contact_stopped: false,
        };
    }
    let start_angles = accepted.angles.clone();
    let steps = continuation_steps(
        faces.len(),
        max_requested_delta(&start_angles, drivers, targets),
    );
    let bisections = if faces.len() > 100 {
        LARGE_BISECTIONS
    } else {
        SMALL_BISECTIONS
    };
    let mut iterations = accepted.iterations;
    let mut accepted_t = 0.0;
    // 既に食い込んだ作品を開いた直後でも、逆向きに抜く動きは妨げない。
    // 一度安全になった後の safe→intersect 遷移だけを停止対象にする。
    let mut guard_armed = !self_intersects(&accepted.frame);

    for step in 1..=steps {
        let t = step as f64 / steps as f64;
        let mut candidate = solve_at(
            cp,
            faces,
            drivers,
            targets,
            &start_angles,
            t,
            Some(&accepted.angles),
        );
        iterations = iterations.saturating_add(candidate.iterations);
        if !candidate.converged {
            return MotionSolveResult {
                result: previous_with_failure(accepted, candidate, iterations),
                contact_stopped: false,
            };
        }

        let intersects = self_intersects(&candidate.frame);
        if guard_armed && intersects {
            // [accepted_t, t] は安全→交差の区間。常に安全側だけをwarm startへ採用し、
            // 2〜3回の二分探索で「ぶつかる直前」へ寄せる。
            let mut low = accepted_t;
            let mut high = t;
            for _ in 0..bisections {
                let mid = (low + high) * 0.5;
                let mid_result = solve_at(
                    cp,
                    faces,
                    drivers,
                    targets,
                    &start_angles,
                    mid,
                    Some(&accepted.angles),
                );
                iterations = iterations.saturating_add(mid_result.iterations);
                if mid_result.converged && !self_intersects(&mid_result.frame) {
                    accepted = mid_result;
                    low = mid;
                } else {
                    high = mid;
                }
            }
            accepted.iterations = iterations;
            return MotionSolveResult {
                result: accepted,
                contact_stopped: true,
            };
        }

        if !intersects {
            guard_armed = true;
        }
        candidate.iterations = iterations;
        accepted = candidate;
        accepted_t = t;
    }

    MotionSolveResult {
        result: accepted,
        contact_stopped: false,
    }
}

fn solve_requested(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
) -> SolveResult {
    targets.map_or_else(
        || solve(cp, faces, drivers, warm_start),
        |targets| solve_near(cp, faces, drivers, targets, warm_start),
    )
}

fn solve_at(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    start_angles: &HashMap<EdgeId, f64>,
    t: f64,
    warm_start: Option<&HashMap<EdgeId, f64>>,
) -> SolveResult {
    let drivers: Vec<Driver> = drivers
        .iter()
        .map(|driver| Driver {
            hinge: driver.hinge,
            target_angle_deg: interpolate(
                start_angles.get(&driver.hinge).copied().unwrap_or(0.0),
                driver.target_angle_deg,
                t,
            ),
        })
        .collect();
    let targets = targets.map(|targets| {
        targets
            .iter()
            .map(|(&hinge, &target)| {
                (
                    hinge,
                    interpolate(start_angles.get(&hinge).copied().unwrap_or(0.0), target, t),
                )
            })
            .collect::<HashMap<_, _>>()
    });
    solve_requested(cp, faces, &drivers, targets.as_ref(), warm_start)
}

fn interpolate(from: f64, to: f64, t: f64) -> f64 {
    from + (to - from) * t
}

fn max_requested_delta(
    start_angles: &HashMap<EdgeId, f64>,
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
) -> f64 {
    drivers
        .iter()
        .map(|driver| {
            (driver.target_angle_deg - start_angles.get(&driver.hinge).copied().unwrap_or(0.0))
                .abs()
        })
        .chain(targets.into_iter().flat_map(|targets| {
            targets.iter().map(|(&hinge, &target)| {
                (target - start_angles.get(&hinge).copied().unwrap_or(0.0)).abs()
            })
        }))
        .filter(|delta| delta.is_finite())
        .fold(0.0, f64::max)
}

/// 面数に応じて決定的に段数を落とす。実時間で変えると同じ入力の結果が端末負荷で
/// 変わるため、形の規模だけを使う。面400では従来solve+交差判定を各1回に抑える。
fn continuation_steps(face_count: usize, max_delta_deg: f64) -> usize {
    let wanted = (max_delta_deg / TARGET_STEP_DEG).ceil().max(1.0) as usize;
    let cap = match face_count {
        0..=100 => 4,
        101..=300 => 2,
        _ => 1,
    };
    wanted.min(cap)
}

/// 中間段で閉包が解けなければ、壊れた候補を見せず直前の収束解へ警告だけ載せる。
fn previous_with_failure(
    mut previous: SolveResult,
    failed: SolveResult,
    iterations: u32,
) -> SolveResult {
    previous.converged = false;
    previous.iterations = iterations;
    for warning in failed.frame.warnings {
        if !previous.frame.warnings.contains(&warning) {
            previous.frame.warnings.push(warning);
        }
    }
    if !previous
        .frame
        .warnings
        .iter()
        .any(|warning| warning.contains("収束していません"))
    {
        previous
            .frame
            .warnings
            .push("追従計算が収束していません".to_string());
    }
    previous
}

#[cfg(test)]
mod tests {
    use super::continuation_steps;

    #[test]
    fn large_models_use_fewer_deterministic_steps() {
        assert_eq!(continuation_steps(20, 180.0), 4);
        assert_eq!(continuation_steps(200, 180.0), 2);
        assert_eq!(continuation_steps(400, 180.0), 1);
        assert_eq!(continuation_steps(400, 1.0), 1);
    }
}
