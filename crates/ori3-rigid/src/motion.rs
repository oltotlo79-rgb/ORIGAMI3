//! 角度操作を前の姿勢から少しずつ追い、有限な最良候補を最終要求まで運ぶ。

use std::collections::HashMap;

use ori3_cp::Face;
use ori3_model::{CreasePattern, Driver, EdgeId};

use crate::{SolveResult, self_intersects, solve, solve_near};

/// 小さな作品で使う目標角の刻み。通常の16ms入力はこれより小さいため1段だけになる。
const TARGET_STEP_DEG: f64 = 5.0;
/// 停止しない接触診断付き継続法の結果。
#[derive(Clone, Debug)]
pub struct MotionSolveResult {
    pub result: SolveResult,
    /// 要求までの経路で紙どうしの接触を検出したか。計算は停止しない。
    pub contact_detected: bool,
    /// 旧呼出し元との一時的な互換値。停止契約は廃止したため常にfalse。
    pub contact_stopped: bool,
}

/// 前回角から要求角までを継続法で追い、接触や有限な不収束では停止しない。
///
/// `targets` が `Some` なら各段で [`solve_near`]、`None` なら [`solve`] を使う。
/// `detect_contact == false` はソルバーを1回だけ呼び、結果をそのまま返す。
/// 接触判定は剛体フレームへ掛けるため、後段の表示用重なり補正とは独立している。
#[must_use]
pub fn solve_motion(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    detect_contact: bool,
) -> MotionSolveResult {
    if !detect_contact {
        let requested = solve_requested(cp, faces, drivers, targets, warm_start);
        let result = if is_finite_result(&requested, faces.len()) {
            requested
        } else {
            let previous = warm_start
                .map(|warm| solve_warm_pose(cp, faces, warm))
                .filter(|candidate| is_finite_result(candidate, faces.len()))
                .or_else(|| {
                    let flat = solve(cp, faces, &[], None);
                    is_finite_result(&flat, faces.len()).then_some(flat)
                });
            let iterations = requested.iterations;
            previous.map_or(requested.clone(), |previous| {
                previous_with_failure(previous, requested, iterations)
            })
        };
        return MotionSolveResult {
            result,
            contact_detected: false,
            contact_stopped: false,
        };
    }

    // warmの全角度を一時hardにして、有限な不収束角も解き直さず正確にt=0へ戻す。
    // warmが無い初回、または有限フレームを作れない場合だけ平らな導出形を使う。
    let initial = warm_start.map_or_else(
        || solve(cp, faces, &[], None),
        |warm| solve_warm_pose(cp, faces, warm),
    );
    let flat = (!is_finite_result(&initial, faces.len())).then(|| solve(cp, faces, &[], None));
    let mut last_finite = if is_finite_result(&initial, faces.len()) {
        Some(initial)
    } else {
        flat.filter(|result| is_finite_result(result, faces.len()))
    };
    let start_angles = last_finite
        .as_ref()
        .map(|result| result.angles.clone())
        .unwrap_or_default();
    let steps = continuation_steps(
        faces.len(),
        max_requested_delta(&start_angles, drivers, targets),
    );
    let mut iterations = last_finite.as_ref().map_or(0, |result| result.iterations);
    let mut contact_detected = last_finite
        .as_ref()
        .is_some_and(|result| self_intersects(&result.frame));
    let mut final_failure = None;

    for step in 1..=steps {
        let t = step as f64 / steps as f64;
        let mut candidate = solve_at(
            cp,
            faces,
            drivers,
            targets,
            &start_angles,
            t,
            last_finite.as_ref().map(|result| &result.angles),
        );
        iterations = iterations.saturating_add(candidate.iterations);
        if is_finite_result(&candidate, faces.len()) {
            contact_detected |= self_intersects(&candidate.frame);
            candidate.iterations = iterations;
            last_finite = Some(candidate);
            final_failure = None;
        } else if step == steps {
            final_failure = Some(candidate);
        }
    }

    let result = match (last_finite, final_failure) {
        (Some(previous), Some(failed)) => previous_with_failure(previous, failed, iterations),
        (Some(mut result), None) => {
            result.iterations = iterations;
            result
        }
        (None, Some(mut failed)) => {
            failed.iterations = iterations;
            failed
        }
        (None, None) => solve(cp, faces, &[], None),
    };
    MotionSolveResult {
        result,
        contact_detected,
        contact_stopped: false,
    }
}

fn solve_warm_pose(cp: &CreasePattern, faces: &[Face], warm: &HashMap<EdgeId, f64>) -> SolveResult {
    let mut drivers: Vec<Driver> = warm
        .iter()
        .filter_map(|(&hinge, &target_angle_deg)| {
            (target_angle_deg.is_finite() && (-180.0..=180.0).contains(&target_angle_deg))
                .then_some(Driver {
                    hinge,
                    target_angle_deg,
                })
        })
        .collect();
    drivers.sort_unstable_by_key(|driver| driver.hinge);
    solve(cp, faces, &drivers, Some(warm))
}

fn is_finite_result(result: &SolveResult, expected_faces: usize) -> bool {
    result.closure_rms.is_finite()
        && result.angles.values().all(|angle| angle.is_finite())
        && result.relaxations.iter().all(|relaxation| {
            relaxation.target_angle_deg.is_finite()
                && relaxation.actual_angle_deg.is_finite()
                && relaxation.delta_deg.is_finite()
        })
        && result.frame.faces.len() == expected_faces
        && result.frame.faces.iter().all(|face| {
            face.polygon
                .iter()
                .flatten()
                .all(|coordinate| coordinate.is_finite())
        })
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

/// 最終要求で有限形を1つも作れなかった場合だけ、直前の有限形へ警告を載せる。
fn previous_with_failure(
    mut previous: SolveResult,
    failed: SolveResult,
    iterations: u32,
) -> SolveResult {
    previous.converged = false;
    previous.best_effort = true;
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
