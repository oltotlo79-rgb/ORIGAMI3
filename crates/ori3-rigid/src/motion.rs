//! 角度操作を前の姿勢から少しずつ追い、有限な最良候補を最終要求まで運ぶ。

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use ori3_cp::Face;
use ori3_model::{CreasePattern, Driver, EdgeId, EdgeKind, FaceId, Frame3D};

use crate::intersect::{ContactMetrics, contact_metrics, contact_witnesses};
use crate::solver::{self, PreparedTopology, canonical_delta_deg};
use crate::{
    AngleRelaxation, SolveResult, max_seam_gap, self_intersects, solve, solve_near,
    solve_near_exact, tree,
};

/// 小さな作品で使う目標角の刻み。通常の16ms入力はこれより小さいため1段だけになる。
const TARGET_STEP_DEG: f64 = 5.0;
/// 完全に折った状態とみなす角度の幅(度)。この近くは計算上の特異点になる。
const NEAR_FLAT_FOLD_DEG: f64 = 5.0;
/// 接触補正で同時に動かすヒンジの上限。診断時の速度見積もりと同じ値。
const MAX_CONTACT_HINGES: usize = 64;
/// 交差形と非交差形の間で、mediumを最小限だけ譲らせる固定二分回数。
const CONTACT_LINE_SEARCH_STEPS: usize = 8;
/// solverと同じ閉包収束閾値。接触より閉包を常に上位へ置く順位付けに使う。
const CLOSURE_TOLERANCE: f64 = 1e-13;
const RELAXATION_EPS_DEG: f64 = 1e-6;
/// 紙が裂けたとみなす辺の離れ(紙の長辺を1とした値)。表示でも検査でも同じ値を使う。
const SEAM_TEAR_TOLERANCE: f64 = 1e-6;
const CONTACT_BEST_EFFORT_WARNING: &str =
    "紙の貫通を完全には避けられないため、貫通が最も少ない有限形で追従しています";
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
    let followed = solve_motion_from(cp, faces, drivers, targets, warm_start, detect_contact);
    if !detect_contact
        || warm_start.is_none()
        || !is_finite_result(&followed.result, faces.len())
        || !self_intersects(&followed.result.frame)
    {
        return followed;
    }
    // 紙は自分を通り抜けられない。前の姿勢から追っても食い込みが残るときは、
    // 平らな状態から同じ要求まで1回だけ追い直す。折り鶴の花弁折りを170°まで
    // 折る操作では、前の姿勢から追うと29組・食い込み1.053e-1になるのに対し、
    // 平らから追い直すと0組で解ける(指定からのずれも178.1°→102.2°と小さい)。
    // 追い直しは1回だけで、そこでも食い込むなら元の結果を返すので操作は止まらない。
    let restarted = solve_motion_from(cp, faces, drivers, targets, None, detect_contact);
    let improved = is_finite_result(&restarted.result, faces.len())
        && !self_intersects(&restarted.result.frame)
        && max_seam_gap(cp, faces, &restarted.result.frame)
            <= max_seam_gap(cp, faces, &followed.result.frame).max(SEAM_TEAR_TOLERANCE);
    if improved { restarted } else { followed }
}

fn solve_motion_from(
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
    let topology = solver::prepare_topology(cp, faces);
    let initial = warm_start.map_or_else(
        || solver::solve_prepared(cp, faces, &[], None, &topology),
        |warm| solve_warm_pose_prepared(cp, faces, warm, &topology),
    );
    let flat = (!is_finite_result(&initial, faces.len()))
        .then(|| solver::solve_prepared(cp, faces, &[], None, &topology));
    let mut last_finite = if is_finite_result(&initial, faces.len()) {
        Some(initial)
    } else {
        flat.filter(|result| is_finite_result(result, faces.len()))
    };
    let start_angles = last_finite
        .as_ref()
        .map(|result| result.angles.clone())
        .unwrap_or_default();
    // 完全に折った状態(±180°)の近くは計算上の特異点で、そこから一気に解くと
    // 紙が遠くへ飛ぶ。小さな要求でも分割を細かくして、実際の紙のように連続して動かす。
    // やっこさんで1本を4°動かす場合、分割1回では他が179.9°暴れ、4回では160.3°に収まる。
    let near_fully_folded = start_angles
        .values()
        .any(|angle| (angle.abs() - 180.0).abs() < NEAR_FLAT_FOLD_DEG);
    let steps = continuation_steps(
        faces.len(),
        max_requested_delta(&start_angles, drivers, targets),
        near_fully_folded,
    );
    let mut iterations = last_finite.as_ref().map_or(0, |result| result.iterations);
    let mut last_finite_intersects = last_finite
        .as_ref()
        .map(|result| self_intersects(&result.frame));
    let mut contact_detected = last_finite_intersects.unwrap_or(false);
    let mut final_failure = None;

    for step in 1..=steps {
        let t = step as f64 / steps as f64;
        let step_drivers = interpolated_drivers(drivers, &start_angles, t);
        let step_targets = interpolated_targets(targets, &start_angles, t);
        let step_warm = last_finite.as_ref().map(|result| &result.angles);
        let moving_outward = step_drivers.iter().all(|driver| {
            let start = start_angles.get(&driver.hinge).copied().unwrap_or(0.0);
            driver.target_angle_deg.abs() + RELAXATION_EPS_DEG >= start.abs()
        });
        let reseed = Reseed {
            cp,
            faces,
            drivers: &step_drivers,
            targets: step_targets.as_ref(),
            start_angles: &start_angles,
            warm: step_warm,
            topology: &topology,
        };
        // 利用者が指定した角度が全て同時に成り立つなら、それがそのまま答え。
        // ここで先に確かめておかないと、譲った姿勢が次の段の出発点になり、
        // ずれが後の段へ積み上がる。成り立たない段だけ、譲りを許す計算へ進む。
        if let Some(mut exact) = reseed.all_specified() {
            iterations = iterations.saturating_add(exact.iterations);
            exact.iterations = iterations;
            last_finite = Some(exact);
            last_finite_intersects = Some(false);
            final_failure = None;
            continue;
        }
        let mut candidate = if step == steps && moving_outward {
            step_targets.as_ref().map_or_else(
                || solver::solve_prepared(cp, faces, &step_drivers, step_warm, &topology),
                |targets| {
                    solver::solve_near_exact_prepared(
                        cp,
                        faces,
                        &step_drivers,
                        targets,
                        step_warm,
                        &topology,
                    )
                },
            )
        } else {
            solve_requested_prepared(
                cp,
                faces,
                &step_drivers,
                step_targets.as_ref(),
                step_warm,
                &topology,
            )
        };
        if step == steps
            && let Some(targets) = step_targets.as_ref()
            && is_finite_result(&candidate, faces.len())
            && !self_intersects(&candidate.frame)
            && max_seam_gap(cp, faces, &candidate.frame) >= SEAM_TEAR_TOLERANCE
        {
            // medium付き通常解が非交差のまま厳密seamだけを僅かに外したときに限り、
            // ばね0の最終閉包段を1回使う。別の交差枝へ移る候補は採用しない。
            let exact = solver::solve_near_exact_prepared(
                cp,
                faces,
                &step_drivers,
                targets,
                step_warm,
                &topology,
            );
            // 分割の途中で別の枝へ逸れると、最終段の初期値が既に悪くなっていて、
            // そこから解き直しても紙が裂けたままになる。分割前の姿勢を初期値にして
            // 一度で解き直した候補も試す。折り鶴の28°では、途中経由の解が裂け3.5e-5
            // なのに対し、分割前から解くと7.3e-15かつ交差なしになる。
            let from_start = solver::solve_near_prepared(
                cp,
                faces,
                &step_drivers,
                targets,
                Some(&start_angles),
                &topology,
            );
            for alternative in [exact, from_start] {
                if is_finite_result(&alternative, faces.len())
                    && !self_intersects(&alternative.frame)
                    && max_seam_gap(cp, faces, &alternative.frame)
                        < max_seam_gap(cp, faces, &candidate.frame)
                {
                    candidate = alternative;
                }
            }
        }
        if step == steps && !candidate.converged && is_finite_result(&candidate, faces.len()) {
            candidate = reseed.rescue(candidate);
        }
        let raw_contact =
            is_finite_result(&candidate, faces.len()) && self_intersects(&candidate.frame);
        let mut candidate_intersects = Some(raw_contact);
        // 大作品では継続法の内部点ごとに再投影すると330msを越え得る。利用者へ返す
        // 最終点は必ず補正し、小作品だけは途中点も同じ非交差枝へ乗せる。
        if raw_contact && (step == steps || faces.len() <= 100) {
            candidate = avoid_contact(
                cp,
                faces,
                &step_drivers,
                step_targets.as_ref(),
                last_finite.as_ref().map(|result| &result.angles),
                candidate,
            );
            // 補正後は別Frameなので、必要になった時点で改めて診断する。
            candidate_intersects = None;
        }
        iterations = iterations.saturating_add(candidate.iterations);
        if is_finite_result(&candidate, faces.len()) {
            // raw_contact=falseならcandidateは補正されず同一Frame、trueなら検出済み。
            // ここで同じ全面組を再走査してもcontact_detectedの値は変わらない。
            contact_detected |= raw_contact;
            candidate.iterations = iterations;
            last_finite = Some(candidate);
            last_finite_intersects = candidate_intersects;
            final_failure = None;
        } else if step == steps {
            final_failure = Some(candidate);
        }
    }

    let (mut result, known_result_intersects) = match (last_finite, final_failure) {
        (Some(previous), Some(failed)) => (
            previous_with_failure(previous, failed, iterations),
            last_finite_intersects,
        ),
        (Some(mut result), None) => {
            result.iterations = iterations;
            (result, last_finite_intersects)
        }
        (None, Some(mut failed)) => {
            failed.iterations = iterations;
            (failed, None)
        }
        (None, None) => (
            solver::solve_prepared(cp, faces, &[], None, &topology),
            None,
        ),
    };
    // resultが直前に診断したFrameそのものなら、その結果を再利用する。接触補正や
    // 非有限fallbackでFrameが変わった場合だけ従来どおり全面診断する。
    if known_result_intersects.unwrap_or_else(|| self_intersects(&result.frame)) {
        // 継続内部で接触回避枝が行き止まりになっても、呼出し時の直前姿勢から
        // 最終要求へ直接解いた成立・非交差枝があればそれを失わない。接触時だけの
        // 固定1候補で、seam閾値を満たす場合に限って採用する。
        let mut direct =
            solve_requested_prepared(cp, faces, drivers, targets, warm_start, &topology);
        if is_finite_result(&direct, faces.len())
            && !self_intersects(&direct.frame)
            && max_seam_gap(cp, faces, &direct.frame) >= 1e-6
            && let Some(targets) = targets
        {
            let exact = solver::solve_near_exact_prepared(
                cp, faces, drivers, targets, warm_start, &topology,
            );
            if is_finite_result(&exact, faces.len())
                && !self_intersects(&exact.frame)
                && max_seam_gap(cp, faces, &exact.frame) < max_seam_gap(cp, faces, &direct.frame)
            {
                direct = exact;
            }
        }
        if is_finite_result(&direct, faces.len())
            && !self_intersects(&direct.frame)
            && max_seam_gap(cp, faces, &direct.frame) < 1e-6
        {
            direct.iterations = result.iterations.saturating_add(direct.iterations);
            result = direct;
        }
    }
    if faces.len() <= 100 && self_intersects(&result.frame) {
        // 1回目の閉包補正で接触面が変わると、次のwitness経路で初めて有効な
        // 非木ヒンジが現れる。小作品だけ固定2pass目を許し、同じ辞書式順位で
        // 改善した有限候補だけを採る。
        result = avoid_contact(cp, faces, drivers, targets, warm_start, result);
    }
    MotionSolveResult {
        result,
        contact_detected,
        contact_stopped: false,
    }
}

fn solve_warm_pose(cp: &CreasePattern, faces: &[Face], warm: &HashMap<EdgeId, f64>) -> SolveResult {
    let topology = solver::prepare_topology(cp, faces);
    solve_warm_pose_prepared(cp, faces, warm, &topology)
}

fn solve_warm_pose_prepared(
    cp: &CreasePattern,
    faces: &[Face],
    warm: &HashMap<EdgeId, f64>,
    topology: &PreparedTopology,
) -> SolveResult {
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
    solver::solve_prepared(cp, faces, &drivers, Some(warm), topology)
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

fn solve_requested_prepared(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    topology: &PreparedTopology,
) -> SolveResult {
    targets.map_or_else(
        || solver::solve_prepared(cp, faces, drivers, warm_start, topology),
        |targets| solver::solve_near_prepared(cp, faces, drivers, targets, warm_start, topology),
    )
}

fn interpolated_drivers(
    drivers: &[Driver],
    start_angles: &HashMap<EdgeId, f64>,
    t: f64,
) -> Vec<Driver> {
    drivers
        .iter()
        .map(|driver| Driver {
            hinge: driver.hinge,
            target_angle_deg: interpolate(
                start_angles.get(&driver.hinge).copied().unwrap_or(0.0),
                driver.target_angle_deg,
                t,
            ),
        })
        .collect()
}

fn interpolated_targets(
    targets: Option<&HashMap<EdgeId, f64>>,
    start_angles: &HashMap<EdgeId, f64>,
    t: f64,
) -> Option<HashMap<EdgeId, f64>> {
    targets.map(|targets| {
        targets
            .iter()
            .map(|(&hinge, &target)| {
                (
                    hinge,
                    interpolate(start_angles.get(&hinge).copied().unwrap_or(0.0), target, t),
                )
            })
            .collect::<HashMap<_, _>>()
    })
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
fn continuation_steps(face_count: usize, max_delta_deg: f64, near_fully_folded: bool) -> usize {
    let wanted = (max_delta_deg / TARGET_STEP_DEG).ceil().max(1.0) as usize;
    let cap = match face_count {
        0..=100 => 4,
        101..=300 => 2,
        _ => 1,
    };
    // 特異点の近くだけは、要求が小さくても上限まで刻む。
    if near_fully_folded { cap } else { wanted.min(cap) }
}

#[derive(Clone)]
struct ContactCandidate {
    result: SolveResult,
    contact: ContactMetrics,
    medium_energy: f64,
    free_warm_distance: f64,
    sorted_angles: Vec<(EdgeId, f64)>,
}

impl ContactCandidate {
    fn new(
        result: SolveResult,
        drivers: &[Driver],
        targets: Option<&HashMap<EdgeId, f64>>,
        warm: Option<&HashMap<EdgeId, f64>>,
    ) -> Option<Self> {
        if !is_finite_result(&result, result.frame.faces.len()) {
            return None;
        }
        let hard: BTreeSet<EdgeId> = drivers.iter().map(|driver| driver.hinge).collect();
        let mut hinges_by_edge_id: Vec<_> = result.angles.keys().copied().collect();
        hinges_by_edge_id.sort_unstable();
        let (medium_energy, free_warm_distance) =
            angle_priority_costs(&result.angles, &hinges_by_edge_id, &hard, targets, warm);
        if !medium_energy.is_finite() || !free_warm_distance.is_finite() {
            return None;
        }
        let sorted_angles: Vec<_> = hinges_by_edge_id
            .into_iter()
            .map(|hinge| (hinge, result.angles[&hinge]))
            .collect();
        Some(Self {
            contact: contact_metrics(&result.frame),
            result,
            medium_energy,
            free_warm_distance,
            sorted_angles,
        })
    }

    /// hard/finiteは構築時の前提。閉包を接触より上、mediumを接触より下に置く。
    fn is_better_than(&self, other: &Self) -> bool {
        let self_closed = self.result.closure_rms < CLOSURE_TOLERANCE;
        let other_closed = other.result.closure_rms < CLOSURE_TOLERANCE;
        match self_closed.cmp(&other_closed) {
            Ordering::Greater => return true,
            Ordering::Less => return false,
            Ordering::Equal => {}
        }
        if !self_closed {
            match self.result.closure_rms.total_cmp(&other.result.closure_rms) {
                Ordering::Less => return true,
                Ordering::Greater => return false,
                Ordering::Equal => {}
            }
        }
        match self
            .contact
            .max_penetration
            .total_cmp(&other.contact.max_penetration)
        {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
        match self
            .contact
            .total_penetration
            .total_cmp(&other.contact.total_penetration)
        {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
        match self.contact.pair_count.cmp(&other.contact.pair_count) {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
        match self.medium_energy.total_cmp(&other.medium_energy) {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
        match self.free_warm_distance.total_cmp(&other.free_warm_distance) {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
        self.sorted_angles
            .iter()
            .zip(&other.sorted_angles)
            .find_map(|(left, right)| {
                left.1
                    .total_cmp(&right.1)
                    .ne(&Ordering::Equal)
                    .then(|| left.1.total_cmp(&right.1) == Ordering::Less)
            })
            .unwrap_or(false)
    }
}

/// 最終要求で紙が閉じなかったときに、初期値を変えて解き直すための一式。
struct Reseed<'a> {
    cp: &'a CreasePattern,
    faces: &'a [Face],
    drivers: &'a [Driver],
    targets: Option<&'a HashMap<EdgeId, f64>>,
    start_angles: &'a HashMap<EdgeId, f64>,
    warm: Option<&'a HashMap<EdgeId, f64>>,
    topology: &'a PreparedTopology,
}

impl Reseed<'_> {
    /// 紙が実際に裂けているときだけ、決まった初期値をいくつか試して解き直す。
    /// 閉じて・交差せず・裂けも増えない解が見つかれば、その中で前の姿勢に最も近い
    /// ものへ差し替える。見つからなければ元のまま返す(操作は止めない)。
    ///
    /// 前の姿勢から連続に追うだけでは、折り目の角度によっては閉じた形へ辿り着けない
    /// ことがある。折り鶴で1本を−150°〜−160°へ折ると閉包RMSが2.835e-3〜9.177e-3
    /// (紙が裂けて見える大きさ)になるが、閉じた形自体は存在し、初期値を変えると
    /// 1.441e-14以下まで下がる。刻みを5°/2°/1°/0.5°と変えても同じ範囲で起きるため、
    /// 分割を細かくしても解決しない。
    fn rescue(&self, candidate: SolveResult) -> SolveResult {
        let candidate_gap = max_seam_gap(self.cp, self.faces, &candidate.frame);
        if candidate_gap < SEAM_TEAR_TOLERANCE {
            // 閉包残差が残っていても紙が裂けていないなら、見た目は正しい。
            // ここで別の枝の解へ移ると、以降の追従がその枝に乗り換えてしまう。
            // 鳥の基本形を180°から戻す掃引では、残差2.559e-3でも裂けは1.249e-15で、
            // 解き直すと後の173°で裂けが1.479e-5まで広がった。
            return candidate;
        }
        let mut best: Option<(f64, SolveResult)> = None;
        for seed in self.seeds() {
            let solved = self.solve_from(seed.as_ref());
            if !solved.converged
                || !is_finite_result(&solved, self.faces.len())
                || self_intersects(&solved.frame)
                || max_seam_gap(self.cp, self.faces, &solved.frame) > candidate_gap
            {
                continue;
            }
            let distance = self.distance_from_previous(&solved.angles);
            if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                best = Some((distance, solved));
            }
        }
        match best {
            Some((_, mut solved)) => {
                solved.iterations = solved.iterations.saturating_add(candidate.iterations);
                solved.relaxations = collect_relaxations(&solved.angles, self.drivers, self.targets);
                solved
            }
            None => candidate,
        }
    }

    /// 利用者が指定した角度を**全て**固定して解いた形。成り立たなければ `None`。
    ///
    /// 希望角は「どうしても必要なときだけ譲る」ものなのに、譲らなくても解ける
    /// 場面でも譲っていた。利用者の画面で実際に起きた例(鳥の基本形の途中、
    /// 8本を180°・2本を0°に折った状態から、別の2本を−5°動かす)では、
    /// 既に折ってある折り目が最大179.3°ほどけて紙が食い込んだ。
    /// 同じ姿勢を12本すべて固定して解くと、−10°でも−90°でも−180°でも
    /// 閉包1e-14以下・自己交差0組で解ける。つまり譲る必要は無かった。
    ///
    /// これを継続法の**各段**で先に確かめる。最終段だけで直すと、途中で譲った
    /// 姿勢が次の段の出発点になり、ずれが後の段へ積み上がってしまう。
    /// 成り立たない段だけ、従来どおり譲りを許す計算へ進むので操作は止まらない。
    fn all_specified(&self) -> Option<SolveResult> {
        let targets = self.targets?;
        let fixed: BTreeSet<EdgeId> = self.drivers.iter().map(|driver| driver.hinge).collect();
        let mut all: Vec<Driver> = self.drivers.to_vec();
        for (&hinge, &target_angle_deg) in targets {
            if !fixed.contains(&hinge) {
                all.push(Driver {
                    hinge,
                    target_angle_deg,
                });
            }
        }
        if all.len() == self.drivers.len() {
            return None;
        }
        all.sort_unstable_by_key(|driver| driver.hinge);
        let mut solved =
            solver::solve_prepared(self.cp, self.faces, &all, self.warm, self.topology);
        if !solved.converged
            || !is_finite_result(&solved, self.faces.len())
            || self_intersects(&solved.frame)
            || max_seam_gap(self.cp, self.faces, &solved.frame) >= SEAM_TEAR_TOLERANCE
        {
            return None;
        }
        solved.relaxations = collect_relaxations(&solved.angles, self.drivers, self.targets);
        Some(solved)
    }

    /// 試す初期値。回数を固定するので、解けない場合でも所要時間は増え続けない。
    fn seeds(&self) -> Vec<Option<HashMap<EdgeId, f64>>> {
        let mut seeds = vec![None, Some(self.start_angles.clone())];
        // 「全ての折り目を要求と同じだけ折った形」から解くと、平らに近い側の
        // 局所解へ落ちずに、深く折れた側の解へ届く。
        if let Some(deepest) = self
            .drivers
            .iter()
            .map(|driver| driver.target_angle_deg)
            .max_by(|left, right| left.abs().total_cmp(&right.abs()))
        {
            seeds.push(Some(
                self.cp
                    .edges
                    .iter()
                    .filter(|edge| edge.kind != EdgeKind::Border)
                    .map(|edge| (edge.id, deepest))
                    .collect(),
            ));
        }
        seeds
    }

    fn solve_from(&self, seed: Option<&HashMap<EdgeId, f64>>) -> SolveResult {
        self.targets.map_or_else(
            || solver::solve_prepared(self.cp, self.faces, self.drivers, seed, self.topology),
            |targets| {
                solver::solve_near_exact_prepared(
                    self.cp,
                    self.faces,
                    self.drivers,
                    targets,
                    seed,
                    self.topology,
                )
            },
        )
    }

    /// 前の姿勢からの角度の隔たり。近い解ほど紙の見た目が飛ばない。
    fn distance_from_previous(&self, angles: &HashMap<EdgeId, f64>) -> f64 {
        let previous = self.warm.unwrap_or(self.start_angles);
        angles
            .iter()
            .map(|(hinge, &angle)| {
                canonical_delta_deg(angle, previous.get(hinge).copied().unwrap_or(0.0)).powi(2)
            })
            .sum::<f64>()
            .sqrt()
    }
}

/// 通常解が交差したときだけ、交差面間のヒンジを譲らせた候補を作る。
///
/// 閉路外ヒンジは閉包残差へ影響しないので直接動かせる。閉路上のヒンジを含む場合
/// だけ既存の疎ソルバーへ1回再投影し、hard→閉包→接触→medium→freeの順位で通常解
/// と比較する。どの候補も有限でなくなる場合は通常解を返すため操作を止めない。
fn avoid_contact(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    warm: Option<&HashMap<EdgeId, f64>>,
    original: SolveResult,
) -> SolveResult {
    let Some(original_rank) = ContactCandidate::new(original.clone(), drivers, targets, warm)
    else {
        return original;
    };
    if original_rank.contact.pair_count == 0 {
        return original;
    }

    let forest = tree::build_forest(cp, faces);
    let hard: BTreeSet<EdgeId> = drivers.iter().map(|driver| driver.hinge).collect();
    let pairs: Vec<_> = contact_witnesses(&original.frame)
        .into_iter()
        .map(|witness| witness.faces)
        .collect();
    let candidates = contact_hinge_candidates(&forest, faces, &pairs, &hard, targets);
    if candidates.is_empty() {
        return with_contact_warning(original);
    }

    let search = ContactAngleSearch {
        cp,
        faces,
        forest: &forest,
        warnings: &original.frame.warnings,
        hard: &hard,
        targets,
        warm,
    };
    let mut best = ContactAngleBest {
        angles: original.angles.clone(),
        frame: original.frame.clone(),
        contact: original_rank.contact.clone(),
        medium_energy: original_rank.medium_energy,
        free_warm_distance: original_rank.free_warm_distance,
    };
    // 直接配置では侵入深さが浅くても交差組が多い候補が勝つことがある。一方、
    // 閉包再投影すると交差組が少ない候補だけが0組へ収束する場合があるため、
    // 最終順位とは別に「組数最小」の再投影anchorを1つだけ保持する。
    let mut pair_anchor = ContactAngleBest {
        angles: original.angles.clone(),
        frame: original.frame.clone(),
        contact: original_rank.contact.clone(),
        medium_energy: original_rank.medium_energy,
        free_warm_distance: original_rank.free_warm_distance,
    };
    // まず1本ずつ平らな向きへ開く。木構造のmediumもここでは固定値ではない。
    for &hinge in &candidates {
        let mut angles = original.angles.clone();
        angles.insert(hinge, 0.0);
        search.consider(angles, &mut best, &mut pair_anchor);
    }
    // 複数の独立接触には、関係するヒンジを共同で譲らせる候補も1つだけ評価する。
    let mut together = original.angles.clone();
    for &hinge in &candidates {
        together.insert(hinge, 0.0);
    }
    let together_anchor = together.clone();
    let loop_closures: BTreeSet<EdgeId> = forest
        .loops
        .iter()
        .map(|closure| forest.hinges[closure.hinge])
        .collect();
    let mut loop_candidates: Vec<EdgeId> = candidates
        .iter()
        .copied()
        .filter(|hinge| loop_closures.contains(hinge))
        .collect();
    loop_candidates.sort_unstable_by(|left, right| {
        let left_fold = original.angles.get(left).copied().unwrap_or(0.0).abs();
        let right_fold = original.angles.get(right).copied().unwrap_or(0.0).abs();
        right_fold
            .total_cmp(&left_fold)
            .then_with(|| left.cmp(right))
    });
    let mut loop_anchor = original.angles.clone();
    if let Some(&hinge) = loop_candidates.first() {
        loop_anchor.insert(hinge, 0.0);
    }
    search.consider(together, &mut best, &mut pair_anchor);
    if best.contact == original_rank.contact {
        // 0°側で改善しない分岐では、同じ折り目の反対側も全体で1候補だけ試す。
        let mut opposite = original.angles.clone();
        for &hinge in &candidates {
            if let Some(&angle) = original.angles.get(&hinge) {
                opposite.insert(hinge, -angle);
            }
        }
        search.consider(opposite, &mut best, &mut pair_anchor);
    }
    if best.angles == original.angles {
        // 非木の閉路ヒンジはtree直接配置へ現れないが、閉包再投影では周囲の
        // 折り目を動かして接触を解ける。直接指標が同値でも共同0°anchorを1回は
        // 再投影することで、その解を候補から落とさない。
        best.angles.clone_from(&together_anchor);
        best.frame = frame_from_angles(cp, faces, &forest, &best.angles, &original.frame.warnings);
    }
    if faces.len() <= 100 && best.contact.pair_count > 0 && loop_anchor != original.angles {
        // 非木ヒンジはtree直接配置では幾何へ現れない。接触経路上で最も深く
        // 折れている1本を主anchorにし、閉包再投影による周辺ヒンジの譲歩を評価する。
        // 共同anchorは固定2候補目として残す。
        best.angles.clone_from(&loop_anchor);
        best.frame = frame_from_angles(cp, faces, &forest, &best.angles, &original.frame.warnings);
    }

    // 非交差まで到達できたなら、接触側との間を固定回数で二分し、medium/freeが譲る
    // 量を必要最小限にする。時刻打切りを使わないので結果は常に決定的。
    let clear_goal = (best.contact.pair_count == 0).then(|| best.angles.clone());
    let mut projection_anchors = Vec::new();
    if let Some(goal) = clear_goal.clone() {
        projection_anchors.push(goal);
    }
    if faces.len() <= 100 {
        // 小作品では主anchorを含む非木ヒンジ候補を最大4本まで閉包再投影する。
        // 直接tree配置に現れない自由度を1本ずつ評価し、最初の非交差枝で止める。
        for &hinge in loop_candidates.iter().take(4) {
            if projection_anchors.len() >= 4 {
                break;
            }
            let mut goal = original.angles.clone();
            goal.insert(hinge, 0.0);
            if goal != best.angles && !projection_anchors.contains(&goal) {
                projection_anchors.push(goal);
            }
        }
        if together_anchor != best.angles
            && !projection_anchors.contains(&together_anchor)
            && projection_anchors.len() < 3
        {
            projection_anchors.push(together_anchor.clone());
        }
    }
    if projection_anchors.is_empty() && pair_anchor.contact.pair_count < best.contact.pair_count {
        projection_anchors.push(pair_anchor.angles.clone());
    }
    if let Some(goal) = &clear_goal {
        let mut low = 0.0;
        let mut high = 1.0;
        for _ in 0..CONTACT_LINE_SEARCH_STEPS {
            let mid = 0.5 * (low + high);
            let angles = interpolate_angle_maps(&original.angles, goal, mid);
            let frame = frame_from_angles(cp, faces, &forest, &angles, &original.frame.warnings);
            if contact_metrics(&frame).pair_count == 0 {
                high = mid;
                best.angles = angles;
                best.frame = frame;
            } else {
                low = mid;
            }
        }
    }

    let on_cycle = hinges_on_cycles(&forest, faces.len());
    let changed: Vec<EdgeId> = best
        .angles
        .iter()
        .filter_map(|(&hinge, &angle)| {
            let before = original.angles.get(&hinge).copied().unwrap_or(0.0);
            (canonical_delta_deg(angle, before).abs() >= RELAXATION_EPS_DEG).then_some(hinge)
        })
        .collect();
    let mut candidate = if changed.iter().any(|hinge| on_cycle.contains(hinge)) {
        let mut contact_targets = targets.cloned().unwrap_or_default();
        for &hinge in &changed {
            if let Some(&angle) = best.angles.get(&hinge) {
                contact_targets.insert(hinge, angle);
            }
        }
        let mut solved = solve_near_exact(cp, faces, drivers, &contact_targets, Some(&best.angles));
        solved.iterations = original.iterations.saturating_add(solved.iterations);
        solved.relaxations = collect_relaxations(&solved.angles, drivers, targets);
        solved
    } else {
        let mut direct = original.clone();
        direct.angles = best.angles;
        direct.frame = best.frame;
        direct.relaxations = collect_relaxations(&direct.angles, drivers, targets);
        direct
    };

    let mut candidate_rank = ContactCandidate::new(candidate.clone(), drivers, targets, warm);

    // 木の直接配置では見えない非木自由度を、閉包後の接触指標で比較する。
    // 大作品は1候補、小作品も主anchor込み最大4本で固定し、時刻依存打切りはしない。
    for goal in projection_anchors {
        if candidate_rank
            .as_ref()
            .is_none_or(|rank| rank.contact.pair_count == 0)
        {
            break;
        }
        let goal_changed: Vec<EdgeId> = goal
            .iter()
            .filter_map(|(&hinge, &angle)| {
                let before = original.angles.get(&hinge).copied().unwrap_or(0.0);
                (canonical_delta_deg(angle, before).abs() >= RELAXATION_EPS_DEG).then_some(hinge)
            })
            .collect();
        let mut anchor = if goal_changed.iter().any(|hinge| on_cycle.contains(hinge)) {
            let mut contact_targets = targets.cloned().unwrap_or_default();
            for &hinge in &goal_changed {
                if let Some(&angle) = goal.get(&hinge) {
                    contact_targets.insert(hinge, angle);
                }
            }
            let mut solved = solve_near_exact(cp, faces, drivers, &contact_targets, Some(&goal));
            solved.iterations = original.iterations.saturating_add(solved.iterations);
            solved.relaxations = collect_relaxations(&solved.angles, drivers, targets);
            solved
        } else {
            let mut direct = original.clone();
            direct.angles = goal;
            direct.frame =
                frame_from_angles(cp, faces, &forest, &direct.angles, &original.frame.warnings);
            direct.relaxations = collect_relaxations(&direct.angles, drivers, targets);
            direct
        };
        if let Some(anchor_rank) = ContactCandidate::new(anchor.clone(), drivers, targets, warm)
            && candidate_rank
                .as_ref()
                .is_none_or(|rank| anchor_rank.is_better_than(rank))
        {
            anchor.iterations = anchor.iterations.max(candidate.iterations);
            candidate = anchor;
            candidate_rank = Some(anchor_rank);
        }
    }

    let Some(candidate_rank) = candidate_rank else {
        return with_contact_warning(original);
    };
    if candidate_rank.is_better_than(&original_rank) {
        if candidate_rank.contact.pair_count > 0 {
            candidate = with_contact_warning(candidate);
        }
        candidate
    } else {
        with_contact_warning(original)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
struct OrderedF64(f64);

struct ContactAngleSearch<'a> {
    cp: &'a CreasePattern,
    faces: &'a [Face],
    forest: &'a tree::Forest,
    warnings: &'a [String],
    hard: &'a BTreeSet<EdgeId>,
    targets: Option<&'a HashMap<EdgeId, f64>>,
    warm: Option<&'a HashMap<EdgeId, f64>>,
}

struct ContactAngleBest {
    angles: HashMap<EdgeId, f64>,
    frame: Frame3D,
    contact: ContactMetrics,
    medium_energy: f64,
    free_warm_distance: f64,
}

impl ContactAngleSearch<'_> {
    fn consider(
        &self,
        angles: HashMap<EdgeId, f64>,
        best: &mut ContactAngleBest,
        pair_anchor: &mut ContactAngleBest,
    ) {
        let frame = frame_from_angles(self.cp, self.faces, self.forest, &angles, self.warnings);
        let contact = contact_metrics(&frame);
        let (medium, free_warm) = angle_priority_costs(
            &angles,
            &self.forest.hinges,
            self.hard,
            self.targets,
            self.warm,
        );
        let better = (
            OrderedF64(contact.max_penetration),
            OrderedF64(contact.total_penetration),
            contact.pair_count,
            OrderedF64(medium),
            OrderedF64(free_warm),
        ) < (
            OrderedF64(best.contact.max_penetration),
            OrderedF64(best.contact.total_penetration),
            best.contact.pair_count,
            OrderedF64(best.medium_energy),
            OrderedF64(best.free_warm_distance),
        );
        let pair_anchor_better = (
            contact.pair_count,
            OrderedF64(contact.max_penetration),
            OrderedF64(contact.total_penetration),
            OrderedF64(medium),
            OrderedF64(free_warm),
        ) < (
            pair_anchor.contact.pair_count,
            OrderedF64(pair_anchor.contact.max_penetration),
            OrderedF64(pair_anchor.contact.total_penetration),
            OrderedF64(pair_anchor.medium_energy),
            OrderedF64(pair_anchor.free_warm_distance),
        );
        if pair_anchor_better {
            pair_anchor.angles = angles.clone();
            pair_anchor.frame = frame.clone();
            pair_anchor.contact = contact.clone();
            pair_anchor.medium_energy = medium;
            pair_anchor.free_warm_distance = free_warm;
        }
        if better {
            best.angles = angles;
            best.frame = frame;
            best.contact = contact;
            best.medium_energy = medium;
            best.free_warm_distance = free_warm;
        }
    }
}

fn angle_priority_costs(
    angles: &HashMap<EdgeId, f64>,
    hinges_by_edge_id: &[EdgeId],
    hard: &BTreeSet<EdgeId>,
    targets: Option<&HashMap<EdgeId, f64>>,
    warm: Option<&HashMap<EdgeId, f64>>,
) -> (f64, f64) {
    let medium = hinges_by_edge_id
        .iter()
        .filter_map(|hinge| {
            let target = targets?.get(hinge)?;
            if hard.contains(hinge) || !target.is_finite() {
                return None;
            }
            angles
                .get(hinge)
                .map(|actual| canonical_delta_deg(*actual, *target).powi(2))
        })
        .sum();
    let free = hinges_by_edge_id
        .iter()
        .filter_map(|hinge| {
            let seed = warm?.get(hinge)?;
            if hard.contains(hinge)
                || targets.is_some_and(|targets| targets.contains_key(hinge))
                || !seed.is_finite()
            {
                return None;
            }
            angles
                .get(hinge)
                .map(|actual| canonical_delta_deg(*actual, *seed).powi(2))
        })
        .sum();
    (medium, free)
}

fn frame_from_angles(
    cp: &CreasePattern,
    faces: &[Face],
    forest: &tree::Forest,
    angles: &HashMap<EdgeId, f64>,
    warnings: &[String],
) -> Frame3D {
    let radians: Vec<f64> = forest
        .hinges
        .iter()
        .map(|hinge| angles.get(hinge).copied().unwrap_or(0.0).to_radians())
        .collect();
    let folded = tree::fold_frame(forest, faces, &radians);
    let mut frame = tree::to_frame3d(cp, faces, &folded);
    frame.warnings = warnings.to_vec();
    frame
}

fn interpolate_angle_maps(
    start: &HashMap<EdgeId, f64>,
    end: &HashMap<EdgeId, f64>,
    t: f64,
) -> HashMap<EdgeId, f64> {
    start
        .iter()
        .map(|(&hinge, &from)| {
            let to = end.get(&hinge).copied().unwrap_or(from);
            (hinge, interpolate(from, to, t))
        })
        .collect()
}

fn collect_relaxations(
    angles: &HashMap<EdgeId, f64>,
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
) -> Vec<AngleRelaxation> {
    let hard: BTreeSet<EdgeId> = drivers.iter().map(|driver| driver.hinge).collect();
    let mut entries: Vec<_> = targets
        .into_iter()
        .flat_map(|targets| targets.iter())
        .filter(|(hinge, target)| !hard.contains(hinge) && target.is_finite())
        .filter_map(|(&hinge, &target)| {
            let actual = *angles.get(&hinge)?;
            let delta = canonical_delta_deg(actual, target);
            (delta.abs() >= RELAXATION_EPS_DEG).then_some(AngleRelaxation {
                hinge,
                target_angle_deg: target,
                actual_angle_deg: actual,
                delta_deg: delta,
            })
        })
        .collect();
    entries.sort_unstable_by_key(|entry| entry.hinge);
    entries
}

fn contact_hinge_candidates(
    forest: &tree::Forest,
    faces: &[Face],
    pairs: &[(FaceId, FaceId)],
    hard: &BTreeSet<EdgeId>,
    targets: Option<&HashMap<EdgeId, f64>>,
) -> Vec<EdgeId> {
    let face_index: HashMap<FaceId, usize> = faces
        .iter()
        .enumerate()
        .map(|(index, face)| (face.id, index))
        .collect();
    let mut adjacency = vec![Vec::<(usize, usize)>::new(); faces.len()];
    for (hinge, occurrence) in forest.hinge_occ.iter().enumerate() {
        let (left, right) = (occurrence[0].0, occurrence[1].0);
        adjacency[left].push((right, hinge));
        adjacency[right].push((left, hinge));
    }
    for neighbours in &mut adjacency {
        neighbours.sort_unstable_by_key(|&(_, hinge)| forest.hinges[hinge]);
    }
    let mut frequency = BTreeMap::<EdgeId, usize>::new();
    for &(left, right) in pairs.iter().take(32) {
        let (Some(&start), Some(&goal)) = (face_index.get(&left), face_index.get(&right)) else {
            continue;
        };
        let mut previous = vec![None::<(usize, usize)>; faces.len()];
        let mut visited = vec![false; faces.len()];
        let mut queue = VecDeque::from([start]);
        visited[start] = true;
        while let Some(face) = queue.pop_front() {
            if face == goal {
                break;
            }
            for &(next, hinge) in &adjacency[face] {
                if visited[next] {
                    continue;
                }
                visited[next] = true;
                previous[next] = Some((face, hinge));
                queue.push_back(next);
            }
        }
        let mut cursor = goal;
        while cursor != start {
            let Some((parent, hinge)) = previous[cursor] else {
                break;
            };
            let edge = forest.hinges[hinge];
            if !hard.contains(&edge) {
                *frequency.entry(edge).or_default() += 1;
            }
            cursor = parent;
        }
    }
    let mut ranked: Vec<_> = frequency.into_iter().collect();
    ranked.sort_unstable_by(|(left_edge, left_count), (right_edge, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| {
                let left_medium = targets.is_some_and(|targets| targets.contains_key(left_edge));
                let right_medium = targets.is_some_and(|targets| targets.contains_key(right_edge));
                left_medium.cmp(&right_medium)
            })
            .then_with(|| left_edge.cmp(right_edge))
    });
    ranked
        .into_iter()
        .take(MAX_CONTACT_HINGES)
        .map(|(hinge, _)| hinge)
        .collect()
}

fn hinges_on_cycles(forest: &tree::Forest, face_count: usize) -> BTreeSet<EdgeId> {
    let mut adjacency = vec![Vec::<(usize, usize)>::new(); face_count];
    for step in &forest.steps {
        adjacency[step.parent].push((step.child, step.hinge));
        adjacency[step.child].push((step.parent, step.hinge));
    }
    let mut on_cycle = BTreeSet::new();
    for closure in &forest.loops {
        on_cycle.insert(forest.hinges[closure.hinge]);
        let mut previous = vec![None::<(usize, usize)>; face_count];
        let mut visited = vec![false; face_count];
        let mut queue = VecDeque::from([closure.from]);
        visited[closure.from] = true;
        while let Some(face) = queue.pop_front() {
            if face == closure.to {
                break;
            }
            for &(next, hinge) in &adjacency[face] {
                if visited[next] {
                    continue;
                }
                visited[next] = true;
                previous[next] = Some((face, hinge));
                queue.push_back(next);
            }
        }
        let mut cursor = closure.to;
        while cursor != closure.from {
            let Some((parent, hinge)) = previous[cursor] else {
                break;
            };
            on_cycle.insert(forest.hinges[hinge]);
            cursor = parent;
        }
    }
    on_cycle
}

fn with_contact_warning(mut result: SolveResult) -> SolveResult {
    if !result
        .frame
        .warnings
        .iter()
        .any(|warning| warning == CONTACT_BEST_EFFORT_WARNING)
    {
        result
            .frame
            .warnings
            .push(CONTACT_BEST_EFFORT_WARNING.to_string());
    }
    result.best_effort = true;
    result
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
    use super::{ContactCandidate, angle_priority_costs, continuation_steps};
    use crate::{ContactMetrics, SolveResult};
    use ori3_model::Frame3D;
    use std::collections::{BTreeSet, HashMap};

    #[test]
    fn large_models_use_fewer_deterministic_steps() {
        assert_eq!(continuation_steps(20, 180.0, false), 4);
        assert_eq!(continuation_steps(200, 180.0, false), 2);
        assert_eq!(continuation_steps(400, 180.0, false), 1);
        assert_eq!(continuation_steps(400, 1.0, false), 1);
        // 完全に折った状態の近くでは、要求が小さくても上限まで刻む。
        assert_eq!(continuation_steps(20, 1.0, false), 1);
        assert_eq!(continuation_steps(20, 1.0, true), 4);
    }

    fn ranked_candidate(contact: ContactMetrics, medium_energy: f64) -> ContactCandidate {
        ContactCandidate {
            result: SolveResult {
                frame: Frame3D {
                    faces: Vec::new(),
                    warnings: Vec::new(),
                },
                converged: true,
                angles: HashMap::new(),
                closure_rms: 0.0,
                best_effort: false,
                relaxations: Vec::new(),
                iterations: 0,
            },
            contact,
            medium_energy,
            free_warm_distance: 0.0,
            sorted_angles: Vec::new(),
        }
    }

    #[test]
    fn contact_candidate_rank_precedes_keep_energy() {
        let penetrating = ranked_candidate(
            ContactMetrics {
                pair_count: 1,
                max_penetration: 1e-3,
                total_penetration: 1e-3,
            },
            0.0,
        );
        let yielding = ranked_candidate(ContactMetrics::default(), 10_000.0);

        assert!(yielding.is_better_than(&penetrating));
        assert!(!penetrating.is_better_than(&yielding));
    }

    #[test]
    fn angle_priority_costs_are_bit_stable_across_hashmap_orders() {
        let ordered_hinges: Vec<_> = (1..=10).collect();
        let angle_entries: Vec<_> = ordered_hinges
            .iter()
            .copied()
            .map(|hinge| (hinge, 0.0))
            .collect();
        let target_entries = [
            (1, 180.0),
            (2, 1.5e-6),
            (3, 1.5e-6),
            (4, 1.5e-6),
            (5, 1.5e-6),
        ];
        let warm_entries = [
            (6, 180.0),
            (7, 1.5e-6),
            (8, 1.5e-6),
            (9, 1.5e-6),
            (10, 1.5e-6),
        ];
        let make_map = |entries: &[(u32, f64)], reversed: bool| {
            if reversed {
                entries.iter().rev().copied().collect::<HashMap<_, _>>()
            } else {
                entries.iter().copied().collect::<HashMap<_, _>>()
            }
        };

        let angles_forward = make_map(&angle_entries, false);
        let angles_reverse = make_map(&angle_entries, true);
        let targets_forward = make_map(&target_entries, false);
        let targets_reverse = make_map(&target_entries, true);
        let warm_forward = make_map(&warm_entries, false);
        let warm_reverse = make_map(&warm_entries, true);
        let hard = BTreeSet::new();

        let forward = angle_priority_costs(
            &angles_forward,
            &ordered_hinges,
            &hard,
            Some(&targets_forward),
            Some(&warm_forward),
        );
        let reverse = angle_priority_costs(
            &angles_reverse,
            &ordered_hinges,
            &hard,
            Some(&targets_reverse),
            Some(&warm_reverse),
        );

        assert_eq!(forward.0.to_bits(), reverse.0.to_bits());
        assert_eq!(forward.1.to_bits(), reverse.1.to_bits());
    }
}
