//! 角度操作を前の姿勢から少しずつ追い、有限な最良候補を最終要求まで運ぶ。

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use ori3_cp::Face;
use ori3_model::{CreasePattern, Driver, EdgeId, EdgeKind, FaceId, Frame3D};

use crate::intersect::{ContactMetrics, PENETRATION_WARNING, contact_metrics, contact_witnesses};
use crate::solver::{self, PreparedTopology, canonical_delta_deg};
use crate::{AngleRelaxation, SolveResult, max_seam_gap, self_intersects, tree};

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
const CANONICAL_MAX_ERROR_TIE_EPS_DEG: f64 = 1e-9;
const CANONICAL_SQUARED_ERROR_TIE_EPS: f64 = 1e-9;
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
    /// 紙の重なり順をどの幾何から決めたか。刻印しない呼出しでは `None`。
    pub surface_order: Option<SurfaceOrderDiagnostics>,
    /// completeな幾何導出を `result.frame` へ刻印できた場合だけtrue。
    /// material seedやnonfinite fallbackを物理補正のauthorityとして使わせない。
    pub surface_order_authoritative: bool,
}

/// 接触を利用者へ知らせる処理と、形を変えて接触を減らす任意補正を分ける。
///
/// `detect` は診断と警告だけを有効にし、角度・頂点を変えない。`prevent` は利用者が
/// 明示的に「重なり防止」を選んだ場合だけ有効にする。どちらも継続法や閉包救済の
/// 有無には影響しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionContactOptions {
    /// 接触を診断して警告へ載せる。形の採否には使わない。
    pub detect: bool,
    /// 接触を減らす候補へ形を差し替える。明示的な重なり防止でだけtrueにする。
    pub prevent: bool,
}

#[derive(Clone, Copy)]
struct MotionSolveContext<'a> {
    topology: &'a PreparedTopology,
    stamp_surface_order: bool,
}

/// 前回角から要求角までを継続法で追い、接触や有限な不収束では停止しない。
///
/// `targets` が `Some` なら各段で [`solve_near`]、`None` なら [`solve`] を使う。
/// `detect_contact` は接触の診断と警告だけを切り替え、返す形には影響しない。
/// 完全折りの表示順位だけに使う近傍probeは、返す角度・物理フレームを変更しない。
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
    solve_motion_with_contact_options(
        cp,
        faces,
        drivers,
        targets,
        warm_start,
        MotionContactOptions {
            detect: detect_contact,
            prevent: false,
        },
    )
}

/// 接触の検出と、利用者が明示した場合だけ行う重なり防止を独立に選んで姿勢を解く。
#[must_use]
pub fn solve_motion_with_contact_options(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    contact: MotionContactOptions,
) -> MotionSolveResult {
    let topology = solver::prepare_topology(cp, faces);
    solve_motion_prepared(
        cp,
        faces,
        drivers,
        targets,
        warm_start,
        contact,
        MotionSolveContext {
            topology: &topology,
            stamp_surface_order: true,
        },
    )
}

/// Solves a pose from a fixed, document-derived candidate set.
///
/// Unlike [`solve_motion_with_contact_options`], this API intentionally has no Follow/store
/// warm-start input. Every candidate depends only on the crease pattern, invariant hard pins,
/// preferred targets, and the optional document seed. This makes the selected closure branch
/// independent of gesture order and stale IPC responses.
#[must_use]
pub fn solve_canonical_motion_with_contact_options(
    cp: &CreasePattern,
    faces: &[Face],
    invariant_hard: &[Driver],
    preferred_targets: Option<&HashMap<EdgeId, f64>>,
    document_seed: Option<&HashMap<EdgeId, f64>>,
    contact: MotionContactOptions,
) -> MotionSolveResult {
    solve_canonical_motion_prepared(
        cp,
        faces,
        invariant_hard,
        preferred_targets,
        document_seed,
        contact,
    )
    .0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanonicalCandidateKind {
    AnchoredUniformMinus90 { hinge: EdgeId, sample_index: u8 },
    AnchoredUniformPlus90 { hinge: EdgeId, sample_index: u8 },
    Direct,
    DocumentSeed,
    DocumentOverlay,
    KindSignedPlus90,
    KindSignedMinus90,
    KindSignedPlus180,
    KindSignedMinus180,
    UniformPlus90,
    UniformMinus90,
    UniformPlus180,
    UniformMinus180,
}

impl CanonicalCandidateKind {
    const fn ordinal(self) -> u8 {
        match self {
            // These bounded, symmetric alternatives come first inside an exact score tie. They
            // pin a document-requested hinge without using the Follow frame as candidate input.
            Self::AnchoredUniformMinus90 { sample_index, .. } => sample_index,
            Self::AnchoredUniformPlus90 { sample_index, .. } => 3 + sample_index,
            Self::Direct => 6,
            Self::DocumentSeed => 7,
            Self::DocumentOverlay => 8,
            Self::KindSignedPlus90 => 9,
            Self::KindSignedMinus90 => 10,
            Self::KindSignedPlus180 => 11,
            Self::KindSignedMinus180 => 12,
            Self::UniformPlus90 => 13,
            Self::UniformMinus90 => 14,
            Self::UniformPlus180 => 15,
            Self::UniformMinus180 => 16,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CanonicalCandidateScore {
    finite: bool,
    closed: bool,
    max_target_error: f64,
    squared_target_error: f64,
    ordinal: u8,
}

impl CanonicalCandidateScore {
    fn is_better_than(self, other: Self) -> bool {
        match self.closed.cmp(&other.closed) {
            Ordering::Greater => return true,
            Ordering::Less => return false,
            Ordering::Equal => {}
        }
        match self.finite.cmp(&other.finite) {
            Ordering::Greater => return true,
            Ordering::Less => return false,
            Ordering::Equal => {}
        }
        let max_difference = self.max_target_error - other.max_target_error;
        if max_difference.is_finite() && max_difference.abs() > CANONICAL_MAX_ERROR_TIE_EPS_DEG {
            return max_difference < 0.0;
        }
        let squared_difference = self.squared_target_error - other.squared_target_error;
        if squared_difference.is_finite()
            && squared_difference.abs() > CANONICAL_SQUARED_ERROR_TIE_EPS
        {
            return squared_difference < 0.0;
        }
        self.ordinal < other.ordinal
    }
}

struct CanonicalCandidate {
    kind: CanonicalCandidateKind,
    hard: Vec<Driver>,
    preferred: Option<HashMap<EdgeId, f64>>,
    seed: Option<HashMap<EdgeId, f64>>,
    score: CanonicalCandidateScore,
}

struct CanonicalCandidateSpec {
    kind: CanonicalCandidateKind,
    hard: Vec<Driver>,
    preferred: Option<HashMap<EdgeId, f64>>,
    seed: Option<HashMap<EdgeId, f64>>,
}

fn canonical_candidate_specs(
    cp: &CreasePattern,
    invariant_hard: &[Driver],
    preferred_targets: Option<&HashMap<EdgeId, f64>>,
    document_seed: Option<&HashMap<EdgeId, f64>>,
) -> Vec<CanonicalCandidateSpec> {
    let invariant_hard = canonical_hard_drivers(cp, invariant_hard);
    let invariant_hard = invariant_hard.as_slice();
    let preferred_targets = canonical_fold_targets(cp, preferred_targets);
    let preferred_targets = preferred_targets.as_ref();
    let mut specs = Vec::with_capacity(17);
    for (sample_index, hinge) in canonical_anchor_samples(cp, invariant_hard, preferred_targets)
        .into_iter()
        .enumerate()
    {
        let target = preferred_targets
            .and_then(|targets| targets.get(&hinge))
            .copied()
            .expect("sampled anchors come from preferred targets");
        let mut hard = invariant_hard.to_vec();
        hard.push(Driver {
            hinge,
            target_angle_deg: target,
        });
        hard.sort_unstable_by_key(|driver| driver.hinge);
        let mut remaining = preferred_targets.cloned().unwrap_or_default();
        remaining.remove(&hinge);
        let preferred = (!remaining.is_empty()).then_some(remaining);
        for (kind, angle) in [
            (
                CanonicalCandidateKind::AnchoredUniformMinus90 {
                    hinge,
                    sample_index: sample_index as u8,
                },
                -90.0,
            ),
            (
                CanonicalCandidateKind::AnchoredUniformPlus90 {
                    hinge,
                    sample_index: sample_index as u8,
                },
                90.0,
            ),
        ] {
            specs.push(CanonicalCandidateSpec {
                kind,
                hard: hard.clone(),
                preferred: preferred.clone(),
                seed: Some(canonical_uniform_seed(cp, angle)),
            });
        }
    }

    let mut seeds: Vec<(CanonicalCandidateKind, Option<HashMap<EdgeId, f64>>)> =
        vec![(CanonicalCandidateKind::Direct, None)];
    if let Some(document_seed) = document_seed {
        let document_seed = canonical_document_seed(cp, document_seed);
        seeds.push((
            CanonicalCandidateKind::DocumentSeed,
            Some(document_seed.clone()),
        ));
        let mut overlaid = document_seed.clone();
        if let Some(preferred_targets) = preferred_targets {
            let mut ordered_targets: Vec<_> = preferred_targets.iter().collect();
            ordered_targets.sort_unstable_by_key(|(hinge, _)| **hinge);
            for (&hinge, &target) in ordered_targets {
                if target.is_finite()
                    && cp
                        .edges
                        .iter()
                        .any(|edge| edge.id == hinge && is_fold_kind(edge.kind))
                {
                    overlaid.insert(hinge, target);
                }
            }
        }
        seeds.push((CanonicalCandidateKind::DocumentOverlay, Some(overlaid)));
    }
    seeds.extend([
        (
            CanonicalCandidateKind::KindSignedPlus90,
            Some(canonical_kind_seed(cp, 90.0)),
        ),
        (
            CanonicalCandidateKind::KindSignedMinus90,
            Some(canonical_kind_seed(cp, -90.0)),
        ),
        (
            CanonicalCandidateKind::KindSignedPlus180,
            Some(canonical_kind_seed(cp, 180.0)),
        ),
        (
            CanonicalCandidateKind::KindSignedMinus180,
            Some(canonical_kind_seed(cp, -180.0)),
        ),
        (
            CanonicalCandidateKind::UniformPlus90,
            Some(canonical_uniform_seed(cp, 90.0)),
        ),
        (
            CanonicalCandidateKind::UniformMinus90,
            Some(canonical_uniform_seed(cp, -90.0)),
        ),
        (
            CanonicalCandidateKind::UniformPlus180,
            Some(canonical_uniform_seed(cp, 180.0)),
        ),
        (
            CanonicalCandidateKind::UniformMinus180,
            Some(canonical_uniform_seed(cp, -180.0)),
        ),
    ]);
    specs.extend(
        seeds
            .into_iter()
            .map(|(kind, seed)| CanonicalCandidateSpec {
                kind,
                hard: invariant_hard.to_vec(),
                preferred: preferred_targets.cloned(),
                seed,
            }),
    );
    specs
}

fn solve_canonical_motion_prepared(
    cp: &CreasePattern,
    faces: &[Face],
    invariant_hard: &[Driver],
    preferred_targets: Option<&HashMap<EdgeId, f64>>,
    document_seed: Option<&HashMap<EdgeId, f64>>,
    contact: MotionContactOptions,
) -> (MotionSolveResult, CanonicalCandidateKind) {
    let topology = solver::prepare_topology(cp, faces);
    let no_stamp = MotionSolveContext {
        topology: &topology,
        stamp_surface_order: false,
    };
    let invariant_hard = canonical_hard_drivers(cp, invariant_hard);
    let invariant_hard = invariant_hard.as_slice();
    let preferred_targets = canonical_fold_targets(cp, preferred_targets);
    let preferred_targets = preferred_targets.as_ref();
    let specs = canonical_candidate_specs(cp, invariant_hard, preferred_targets, document_seed);

    let mut selected = None::<CanonicalCandidate>;
    for spec in specs {
        let solved = solve_motion_prepared(
            cp,
            faces,
            &spec.hard,
            spec.preferred.as_ref(),
            spec.seed.as_ref(),
            contact,
            no_stamp,
        );
        let score = canonical_candidate_score(
            cp,
            faces,
            invariant_hard,
            preferred_targets,
            &solved.result,
            spec.kind.ordinal(),
        );
        let candidate = CanonicalCandidate {
            kind: spec.kind,
            hard: spec.hard,
            preferred: spec.preferred,
            seed: spec.seed,
            score,
        };
        if selected
            .as_ref()
            .is_none_or(|current| candidate.score.is_better_than(current.score))
        {
            selected = Some(candidate);
        }
    }
    let selected = selected.expect("canonical candidate set always includes direct solve");
    let stamped = solve_motion_prepared(
        cp,
        faces,
        &selected.hard,
        selected.preferred.as_ref(),
        selected.seed.as_ref(),
        contact,
        MotionSolveContext {
            topology: &topology,
            stamp_surface_order: true,
        },
    );
    (stamped, selected.kind)
}

fn canonical_anchor_samples(
    cp: &CreasePattern,
    invariant_hard: &[Driver],
    preferred_targets: Option<&HashMap<EdgeId, f64>>,
) -> Vec<EdgeId> {
    let hard: BTreeSet<_> = invariant_hard.iter().map(|driver| driver.hinge).collect();
    let folds: BTreeSet<_> = cp
        .edges
        .iter()
        .filter(|edge| is_fold_kind(edge.kind))
        .map(|edge| edge.id)
        .collect();
    let mut requested: Vec<_> = preferred_targets
        .into_iter()
        .flat_map(|targets| targets.iter())
        .filter_map(|(&hinge, &target)| {
            (target.is_finite() && folds.contains(&hinge) && !hard.contains(&hinge))
                .then_some(hinge)
        })
        .collect();
    requested.sort_unstable();
    requested.dedup();
    if requested.len() <= 3 {
        return requested;
    }
    let last = requested.len() - 1;
    vec![
        requested[0],
        requested[requested.len() / 2],
        requested[last],
    ]
}

const fn is_fold_kind(kind: EdgeKind) -> bool {
    matches!(kind, EdgeKind::Mountain | EdgeKind::Valley)
}

fn canonical_hard_drivers(cp: &CreasePattern, invariant_hard: &[Driver]) -> Vec<Driver> {
    let folds: BTreeSet<_> = cp
        .edges
        .iter()
        .filter(|edge| is_fold_kind(edge.kind))
        .map(|edge| edge.id)
        .collect();
    let mut hard: Vec<_> = invariant_hard
        .iter()
        .filter(|driver| folds.contains(&driver.hinge) && driver.target_angle_deg.is_finite())
        .cloned()
        .collect();
    hard.sort_unstable_by(|left, right| {
        left.hinge
            .cmp(&right.hinge)
            .then_with(|| left.target_angle_deg.total_cmp(&right.target_angle_deg))
    });
    hard
}

fn canonical_document_seed(
    cp: &CreasePattern,
    document_seed: &HashMap<EdgeId, f64>,
) -> HashMap<EdgeId, f64> {
    cp.edges
        .iter()
        .filter(|edge| is_fold_kind(edge.kind))
        .filter_map(|edge| {
            document_seed
                .get(&edge.id)
                .copied()
                .filter(|angle| angle.is_finite())
                .map(|angle| (edge.id, angle))
        })
        .collect()
}

fn canonical_fold_targets(
    cp: &CreasePattern,
    preferred_targets: Option<&HashMap<EdgeId, f64>>,
) -> Option<HashMap<EdgeId, f64>> {
    let targets: HashMap<_, _> = cp
        .edges
        .iter()
        .filter(|edge| is_fold_kind(edge.kind))
        .filter_map(|edge| {
            preferred_targets?
                .get(&edge.id)
                .copied()
                .filter(|target| target.is_finite())
                .map(|target| (edge.id, target))
        })
        .collect();
    (!targets.is_empty()).then_some(targets)
}

fn canonical_kind_seed(cp: &CreasePattern, signed_magnitude: f64) -> HashMap<EdgeId, f64> {
    cp.edges
        .iter()
        .filter(|edge| is_fold_kind(edge.kind))
        .map(|edge| {
            let fold_sign = if edge.kind == EdgeKind::Valley {
                -1.0
            } else {
                1.0
            };
            (edge.id, fold_sign * signed_magnitude)
        })
        .collect()
}

fn canonical_uniform_seed(cp: &CreasePattern, angle: f64) -> HashMap<EdgeId, f64> {
    cp.edges
        .iter()
        .filter(|edge| is_fold_kind(edge.kind))
        .map(|edge| (edge.id, angle))
        .collect()
}

fn canonical_candidate_score(
    cp: &CreasePattern,
    faces: &[Face],
    invariant_hard: &[Driver],
    preferred_targets: Option<&HashMap<EdgeId, f64>>,
    result: &SolveResult,
    ordinal: u8,
) -> CanonicalCandidateScore {
    let (max_target_error, squared_target_error, target_errors_finite) =
        canonical_requested_errors(&result.angles, invariant_hard, preferred_targets);
    let finite = target_errors_finite
        && max_target_error.is_finite()
        && squared_target_error.is_finite()
        && is_finite_result(result, faces.len());
    let seam = if finite {
        max_seam_gap(cp, faces, &result.frame)
    } else {
        f64::INFINITY
    };
    let closed = finite
        && result.closure_rms <= CLOSURE_TOLERANCE
        && seam.is_finite()
        && seam < SEAM_TEAR_TOLERANCE;
    CanonicalCandidateScore {
        finite,
        closed,
        max_target_error: if target_errors_finite {
            max_target_error
        } else {
            f64::INFINITY
        },
        squared_target_error: if target_errors_finite {
            squared_target_error
        } else {
            f64::INFINITY
        },
        ordinal,
    }
}

fn canonical_requested_errors(
    angles: &HashMap<EdgeId, f64>,
    invariant_hard: &[Driver],
    preferred_targets: Option<&HashMap<EdgeId, f64>>,
) -> (f64, f64, bool) {
    let mut requested = BTreeMap::new();
    if let Some(preferred_targets) = preferred_targets {
        for (&hinge, &target) in preferred_targets {
            requested.insert(hinge, target);
        }
    }
    let mut ordered_hard: Vec<_> = invariant_hard.iter().collect();
    ordered_hard.sort_unstable_by(|left, right| {
        left.hinge
            .cmp(&right.hinge)
            .then_with(|| left.target_angle_deg.total_cmp(&right.target_angle_deg))
    });
    for driver in ordered_hard {
        requested.insert(driver.hinge, driver.target_angle_deg);
    }

    let mut max_target_error = 0.0_f64;
    let mut squared_target_error = 0.0_f64;
    let mut finite = true;
    for (hinge, target) in requested {
        let actual = angles.get(&hinge).copied().unwrap_or(0.0);
        let error = canonical_delta_deg(actual, target).abs();
        finite &= error.is_finite();
        max_target_error = max_target_error.max(error);
        squared_target_error += error * error;
    }
    (
        max_target_error,
        squared_target_error,
        finite && max_target_error.is_finite() && squared_target_error.is_finite(),
    )
}

/// 手順を持たない一時姿勢を、物理的な重なり順を刻まずに追従計算する。
///
/// `fold_all` だけが使うcrate内入口。通常の姿勢操作では、手順や実際に通った経路から
/// 表示順を導出する既存契約を保つ。一斉折りはその根拠を持たないため、導出処理自体を
/// 走らせず、返却frameへ偶然の上下関係が混ざることも防ぐ。
pub(crate) fn solve_motion_without_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    contact: MotionContactOptions,
) -> MotionSolveResult {
    let topology = solver::prepare_topology(cp, faces);
    solve_motion_prepared(
        cp,
        faces,
        drivers,
        targets,
        warm_start,
        contact,
        MotionSolveContext {
            topology: &topology,
            stamp_surface_order: false,
        },
    )
}

/// 指定された姿勢を1回だけ解き、その姿勢だけから表示用の重なり順を刻印する。
///
/// 接触の検出や補正、平らな状態からの追従は行わない。追従経路ではなく、同じ指定を
/// 単発で解いた結果を比較する診断に使う。接触検出の設定でsolver経路を切り替えない
/// ため、この用途を明示的な別APIとして分離している。
#[doc(hidden)]
#[must_use]
pub fn solve_motion_once(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
) -> SolveResult {
    let topology = solver::prepare_topology(cp, faces);
    let mut result = solve_requested_prepared(cp, faces, drivers, targets, warm_start, &topology);
    stamp_motion_surface_order(cp, faces, drivers, targets, &topology, &[], &mut result);
    result
}

fn solve_motion_prepared(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    warm_start: Option<&HashMap<EdgeId, f64>>,
    contact: MotionContactOptions,
    context: MotionSolveContext<'_>,
) -> MotionSolveResult {
    let followed = solve_motion_from(cp, faces, drivers, targets, warm_start, contact, context);
    if !contact.prevent
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
    let restarted = solve_motion_from(cp, faces, drivers, targets, None, contact, context);
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
    contact: MotionContactOptions,
    context: MotionSolveContext<'_>,
) -> MotionSolveResult {
    let topology = context.topology;
    let inspect_contact = contact.detect || contact.prevent;
    // warmの全角度を一時hardにして、有限な不収束角も解き直さず正確にt=0へ戻す。
    // warmが無い初回、または有限フレームを作れない場合だけ平らな導出形を使う。
    let initial = warm_start.map_or_else(
        || solver::solve_prepared(cp, faces, &[], None, topology),
        |warm| solve_warm_pose_prepared(cp, faces, warm, topology),
    );
    let flat = (!is_finite_result(&initial, faces.len()))
        .then(|| solver::solve_prepared(cp, faces, &[], None, topology));
    let mut last_finite = if is_finite_result(&initial, faces.len()) {
        Some(initial)
    } else {
        flat.filter(|result| is_finite_result(result, faces.len()))
    };
    // 閉路を持つ紙では、同じ完全折り角を満たす運動枝が複数あり得る。表示順だけが
    // flat起点の別枝へ飛ばないよう、物理解が実際に通った有限Frameを残しておく。
    let retain_surface_path = context.stamp_surface_order && !topology.forest().loops.is_empty();
    let mut surface_motion_path = if retain_surface_path {
        last_finite
            .as_ref()
            .map(|result| vec![result.frame.clone()])
            .unwrap_or_default()
    } else {
        Vec::new()
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
    let mut last_finite_intersects = inspect_contact.then(|| {
        last_finite
            .as_ref()
            .is_some_and(|result| self_intersects(&result.frame))
    });
    let mut contact_detected = contact.detect && last_finite_intersects == Some(true);
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
            topology,
            prevent_contact: contact.prevent,
        };
        // 利用者が指定した角度が全て同時に成り立つなら、それがそのまま答え。
        // ここで先に確かめておかないと、譲った姿勢が次の段の出発点になり、
        // ずれが後の段へ積み上がる。成り立たない段だけ、譲りを許す計算へ進む。
        if let Some(mut exact) = reseed.all_specified() {
            iterations = iterations.saturating_add(exact.iterations);
            exact.iterations = iterations;
            let exact_intersects = inspect_contact.then(|| self_intersects(&exact.frame));
            contact_detected |= contact.detect && exact_intersects == Some(true);
            if retain_surface_path {
                surface_motion_path.push(exact.frame.clone());
            }
            last_finite = Some(exact);
            last_finite_intersects = exact_intersects;
            final_failure = None;
            continue;
        }
        let mut candidate = if step == steps && moving_outward {
            step_targets.as_ref().map_or_else(
                || solver::solve_prepared(cp, faces, &step_drivers, step_warm, topology),
                |targets| {
                    solver::solve_near_exact_prepared(
                        cp,
                        faces,
                        &step_drivers,
                        targets,
                        step_warm,
                        topology,
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
                topology,
            )
        };
        if step == steps
            && let Some(targets) = step_targets.as_ref()
            && is_finite_result(&candidate, faces.len())
            && (!contact.prevent || !self_intersects(&candidate.frame))
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
                topology,
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
                topology,
            );
            for alternative in [exact, from_start] {
                if is_finite_result(&alternative, faces.len())
                    && (!contact.prevent || !self_intersects(&alternative.frame))
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
        let raw_contact = inspect_contact
            && is_finite_result(&candidate, faces.len())
            && self_intersects(&candidate.frame);
        let mut candidate_intersects = inspect_contact.then_some(raw_contact);
        // 大作品では継続法の内部点ごとに再投影すると330msを越え得る。利用者へ返す
        // 最終点は必ず補正し、小作品だけは途中点も同じ非交差枝へ乗せる。
        if contact.prevent && raw_contact && (step == steps || faces.len() <= 100) {
            candidate = avoid_contact(
                ContactAvoidanceContext {
                    cp,
                    faces,
                    drivers: &step_drivers,
                    targets: step_targets.as_ref(),
                    warm: last_finite.as_ref().map(|result| &result.angles),
                    topology,
                },
                candidate,
                contact.detect,
            );
            // 補正後は別Frameなので、必要になった時点で改めて診断する。
            candidate_intersects = None;
        }
        iterations = iterations.saturating_add(candidate.iterations);
        if is_finite_result(&candidate, faces.len()) {
            // raw_contact=falseならcandidateは補正されず同一Frame、trueなら検出済み。
            // ここで同じ全面組を再走査してもcontact_detectedの値は変わらない。
            contact_detected |= contact.detect && raw_contact;
            candidate.iterations = iterations;
            if retain_surface_path {
                surface_motion_path.push(candidate.frame.clone());
            }
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
        (None, None) => (solver::solve_prepared(cp, faces, &[], None, topology), None),
    };
    // resultが直前に診断したFrameそのものなら、その結果を再利用する。接触補正や
    // 非有限fallbackでFrameが変わった場合だけ従来どおり全面診断する。
    if contact.prevent && known_result_intersects.unwrap_or_else(|| self_intersects(&result.frame))
    {
        // 継続内部で接触回避枝が行き止まりになっても、呼出し時の直前姿勢から
        // 最終要求へ直接解いた成立・非交差枝があればそれを失わない。接触時だけの
        // 固定1候補で、seam閾値を満たす場合に限って採用する。
        let mut direct =
            solve_requested_prepared(cp, faces, drivers, targets, warm_start, topology);
        if is_finite_result(&direct, faces.len())
            && !self_intersects(&direct.frame)
            && max_seam_gap(cp, faces, &direct.frame) >= 1e-6
            && let Some(targets) = targets
        {
            let exact = solver::solve_near_exact_prepared(
                cp, faces, drivers, targets, warm_start, topology,
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
    if contact.prevent && faces.len() <= 100 && self_intersects(&result.frame) {
        // 1回目の閉包補正で接触面が変わると、次のwitness経路で初めて有効な
        // 非木ヒンジが現れる。小作品だけ固定2pass目を許し、同じ辞書式順位で
        // 改善した有限候補だけを採る。
        result = avoid_contact(
            ContactAvoidanceContext {
                cp,
                faces,
                drivers,
                targets,
                warm: warm_start,
                topology,
            },
            result,
            contact.detect,
        );
    }
    if contact_detected
        && !result
            .frame
            .warnings
            .iter()
            .any(|warning| warning == PENETRATION_WARNING)
    {
        result.frame.warnings.push(PENETRATION_WARNING.to_string());
    }
    let (surface_order, surface_order_authoritative) = if context.stamp_surface_order {
        let (diagnostics, authoritative) = stamp_motion_surface_order(
            cp,
            faces,
            drivers,
            targets,
            topology,
            &surface_motion_path,
            &mut result,
        );
        (Some(diagnostics), authoritative)
    } else {
        (None, false)
    };
    MotionSolveResult {
        result,
        contact_detected,
        contact_stopped: false,
        surface_order,
        surface_order_authoritative,
    }
}

/// 重なり順をどの幾何から決めたか。どれも面の番号ではなく紙の位置から決める。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceOrderSource {
    /// 呼出し元のwarm姿勢から最終姿勢まで、物理解が実際に通った経路で決めた。
    SolvedMotionPath,
    /// 平らな状態から22点のcheckpointをsolveし直した経路の実深度で決めた。
    SolvedFlatPath,
    /// 完全に折り切った面があるため、`tree::fold_frame` が伝播経路から作った順を保つ。
    FoldFramePath,
    /// いまの姿勢で、重なっている面どうしの実深度の差で決めた。
    CurrentDepths,
    /// 実深度が完全に同じで決まらない面対が残ったため、平らな状態からの伝播経路で決めた。
    PropagatedFlatPath,
    /// 面が1つも重なっておらず、決めるべき上下が存在しなかった。
    NoOverlap,
}

/// 重なり順をどう決めたかの内訳。利用者へは出さず、検査と調査のために返す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceOrderDiagnostics {
    /// どの幾何から決めたか。面の番号順は選択肢に無い。
    pub source: SurfaceOrderSource,
    /// 180°の折り目が決める厳密な上下と食い違ったため捨てた、深度由来の制約の数。
    pub dropped_depth_constraints: usize,
    /// 実面積で重なっているのに上下を決められなかった面対の数。
    pub unresolved_overlaps: usize,
    /// 上下が輪になっていたため落とした制約の数。
    pub broken_constraints: usize,
}

/// `result.frame` へ重なり順を刻印し、どの幾何から決めたかを返す。
fn stamp_motion_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    topology: &PreparedTopology,
    motion_path: &[Frame3D],
    result: &mut SolveResult,
) -> (SurfaceOrderDiagnostics, bool) {
    if !is_finite_result(result, faces.len()) {
        return (
            SurfaceOrderDiagnostics {
                source: SurfaceOrderSource::FoldFramePath,
                dropped_depth_constraints: 0,
                unresolved_overlaps: 0,
                broken_constraints: 0,
            },
            false,
        );
    }
    let canonical_targets = canonical_surface_targets(&result.angles);
    // 180°に折り切った折り目が決める厳密な上下。深度より先に効かせる。
    //
    // どの折り目を折り切ったとみなすかは、経路の終点と**同じ境目**でなければならない。
    // 終点だけを平らにして折り目の向きを入れないと、同じ形を2つの別の規則で説明する
    // ことになる。実測では、駆動した折り目1本だけが180°付近にある姿勢
    // (`folded-sample.ori3` の辺310・361)で、179.999°側にだけ向きの制約が無く、
    // その1組の上下が180°側と逆になっていた。
    let stack_angles = canonical_targets
        .iter()
        .map(|(&hinge, &angle)| (hinge, crate::surface_order::snap_to_flat(angle)))
        .collect::<HashMap<_, _>>();
    let exact_constraints = exact_stack_constraints_of(faces, topology, &stack_angles);
    let needs_canonical_path = canonical_targets
        .values()
        .any(|angle| angle.abs() >= crate::surface_order::STACK_FLAT_THRESHOLD_DEG);
    let snapped_frame = (needs_canonical_path && !topology.forest().loops.is_empty())
        .then(|| snapped_motion_surface_frame(cp, faces, topology, &stack_angles, &result.angles))
        .flatten();
    let surface_exact_frame = snapped_frame.unwrap_or_else(|| result.frame.clone());
    let report = |source, derived: &crate::surface_order::SurfaceOrder| SurfaceOrderDiagnostics {
        source,
        dropped_depth_constraints: derived.dropped_depth_constraints,
        unresolved_overlaps: derived.unresolved_overlaps,
        broken_constraints: derived.broken_constraints,
    };
    // 明示driverが1本だけの179.999°/exact endpointは、閉路の特異点に複数の進入枝がある。
    // 呼出し元のwarmが既にendpointにいるrefreshでも同じ答えにするため、この狭い条件だけは
    // 平らな側からdriver自身を22点で追った経路を正本にする。preferred targetを伴う操作や
    // 複数hardの操作は文脈を失うため、下の実motion pathを引き続き優先する。
    if needs_canonical_path
        && let Some(derived) =
            single_driver_flat_surface_order(cp, faces, drivers, targets, topology)
        && crate::surface_order::stamp_surface_order(&mut result.frame, &derived.order).is_ok()
    {
        return (report(SurfaceOrderSource::SolvedFlatPath, &derived), true);
    }
    // 閉路の無い紙は同じ指定に分岐が無い。呼出し元warmの短い経路より、平らな状態から
    // 全指定角を再生したcanonical pathを先に使い、179.999°と180°の境界を安定させる。
    // 閉路を持つ紙は進入枝が物理文脈なので、この分岐へ入れず下の実motion pathを優先する。
    if needs_canonical_path
        && topology.forest().loops.is_empty()
        && let Some(derived) = canonical_motion_surface_order(
            cp,
            faces,
            topology,
            &canonical_targets,
            &exact_constraints,
        )
        && derived.complete
        && crate::surface_order::stamp_surface_order(&mut result.frame, &derived.order).is_ok()
    {
        return (report(SurfaceOrderSource::SolvedFlatPath, &derived), true);
    }
    if needs_canonical_path
        && let Some(derived) = complete_motion_path_surface_order(
            cp,
            faces,
            motion_path,
            &surface_exact_frame,
            &exact_constraints,
        )
        && crate::surface_order::stamp_surface_order(&mut result.frame, &derived.order).is_ok()
    {
        return (report(SurfaceOrderSource::SolvedMotionPath, &derived), true);
    }
    if needs_canonical_path
        && let Some(derived) = driver_approach_surface_order(
            cp,
            faces,
            drivers,
            targets,
            topology,
            result,
            SurfaceEndpoint {
                frame: &surface_exact_frame,
                constraints: &exact_constraints,
            },
        )
        && crate::surface_order::stamp_surface_order(&mut result.frame, &derived.order).is_ok()
    {
        return (report(SurfaceOrderSource::SolvedMotionPath, &derived), true);
    }
    // canonical probeが有限形を作れない場合も、完全折りではtree::fold_frameが
    // 全ヒンジのclamp経路から作った順位を保持する。
    if needs_canonical_path {
        return (
            SurfaceOrderDiagnostics {
                source: SurfaceOrderSource::FoldFramePath,
                dropped_depth_constraints: 0,
                unresolved_overlaps: 0,
                broken_constraints: 0,
            },
            false,
        );
    }
    let derived = crate::surface_order::derive_surface_order_from_current_depths(
        cp,
        faces,
        &result.frame,
        &exact_constraints,
    );
    // 「決められなかった」「比較できなかった」「上下が輪になった」のどれかが
    // 残っていれば、その場の深度は答えを持っていない。
    let needs_path = derived.as_ref().map_or(true, |derived| !derived.complete);
    if needs_path {
        // 実深度が完全に同じ面が重なっている。最終形だけでは物理的な上下を
        // 決められないので、平らな状態から現在の角度までを伝播し、最後に
        // 離れていた点の上下で決める。面の番号順へは落とさない。
        if let Some(propagated) =
            propagated_flat_path_surface_order(cp, faces, topology, result, &exact_constraints)
            && propagated.complete
            && crate::surface_order::stamp_surface_order(&mut result.frame, &propagated.order)
                .is_ok()
        {
            return (
                report(SurfaceOrderSource::PropagatedFlatPath, &propagated),
                true,
            );
        }
    }
    let Ok(derived) = derived else {
        // `validate_order` だけがここへ来る。面集合が食い違う呼出しは無いが、
        // 起きたときも止めずに、いま刻印されている順を残す。
        return (
            SurfaceOrderDiagnostics {
                source: SurfaceOrderSource::FoldFramePath,
                dropped_depth_constraints: 0,
                unresolved_overlaps: 0,
                broken_constraints: 0,
            },
            false,
        );
    };
    let source = if derived.resolved_overlaps > 0 {
        SurfaceOrderSource::CurrentDepths
    } else if derived.unresolved_overlaps > 0
        || derived.skipped_pairs > 0
        || derived.broken_constraints > 0
    {
        // 経路でも決められなかった。決まった分だけを残し、決まらなかったことを
        // 呼出し元へ知らせる。面の番号順を答えとして採らない。
        SurfaceOrderSource::PropagatedFlatPath
    } else {
        // 重なっている面が1組も無い。決めるべき上下が存在しない。
        SurfaceOrderSource::NoOverlap
    };
    let authoritative = derived.complete
        && crate::surface_order::stamp_surface_order(&mut result.frame, &derived.order).is_ok();
    (report(source, &derived), authoritative)
}

/// 閉路endpointではexact制約のDAGがcompleteでも、どちら側からその枝へ入ったかは
/// 証明できない。実経路で少なくとも1対の深度差を測れた場合だけ、exact制約と組み
/// 合わせたcomplete結果をauthorityとして返す。重なりが無い場合は測る対自体が無い。
fn complete_motion_path_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    motion_path: &[Frame3D],
    exact_frame: &Frame3D,
    exact_constraints: &[(FaceId, FaceId)],
) -> Option<crate::surface_order::SurfaceOrder> {
    if motion_path.is_empty() {
        return None;
    }
    let derived = crate::surface_order::derive_surface_order_from_frame_path(
        cp,
        faces,
        motion_path,
        exact_frame,
        exact_constraints,
    )
    .ok()?;
    (derived.complete && (derived.resolved_overlaps == 0 || derived.sampled_depth_constraints > 0))
        .then_some(derived)
}

/// preferred targetを持たない単一driverの179.999°/exact endpointを平らな側から再生する。
///
/// 180°の閉路はJacobianが特異で、endpointのwarmから少し戻すだけでは別の閉包枝へ
/// 着地し得る。単一driverなら進入側はdriverの符号で一意に指定できるため、呼出し元の
/// warmを使わず全checkpointを順に解く。返却する角度やpolygonは変更せず、重なり順を
/// 決めるためのFrame列だけを作る。
fn single_driver_flat_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    topology: &PreparedTopology,
) -> Option<crate::surface_order::SurfaceOrder> {
    if targets.is_some() || drivers.len() != 1 {
        return None;
    }
    let driver = &drivers[0];
    let final_checkpoint = crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG
        .last()
        .copied()
        .unwrap_or(180.0);
    if !driver.target_angle_deg.is_finite()
        || driver.target_angle_deg.abs() < final_checkpoint - RELAXATION_EPS_DEG
    {
        return None;
    }

    let mut warm = None;
    let mut path_frames =
        Vec::with_capacity(crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG.len());
    for &checkpoint in &crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG {
        let checkpoint_driver = [Driver {
            hinge: driver.hinge,
            target_angle_deg: driver.target_angle_deg.signum() * checkpoint,
        }];
        let solved =
            solve_requested_prepared(cp, faces, &checkpoint_driver, None, warm.as_ref(), topology);
        if !is_finite_result(&solved, faces.len()) {
            return None;
        }
        let actual = solved.angles.get(&driver.hinge).copied()?;
        if canonical_delta_deg(actual, checkpoint_driver[0].target_angle_deg).abs()
            > RELAXATION_EPS_DEG
        {
            return None;
        }
        warm = Some(solved.angles);
        path_frames.push(solved.frame);
    }
    // 179.999°と180°の返却解では、従属ヒンジがflat判定の境目をまたぐ場合がある。
    // ここで返却解側のexact集合を混ぜると同じ進入経路に別の面集合を当ててしまうため、
    // 最後のcheckpoint自身から終点Frameとexact集合を作り、その経路が示す上下を導く。
    let probe_angles = warm.as_ref()?;
    let probe_stack_angles = canonical_surface_targets(probe_angles)
        .into_iter()
        .map(|(hinge, angle)| (hinge, crate::surface_order::snap_to_flat(angle)))
        .collect::<HashMap<_, _>>();
    let probe_exact_constraints = exact_stack_constraints_of(faces, topology, &probe_stack_angles);
    let probe_exact_frame =
        snapped_motion_surface_frame(cp, faces, topology, &probe_stack_angles, probe_angles)?;
    let derived = crate::surface_order::derive_surface_order_from_frame_path(
        cp,
        faces,
        &path_frames,
        &probe_exact_frame,
        &probe_exact_constraints,
    )
    .ok()?;
    (derived.complete && derived.resolved_overlaps > 0).then_some(derived)
}

struct SurfaceEndpoint<'a> {
    frame: &'a Frame3D,
    constraints: &'a [(FaceId, FaceId)],
}

/// 既にexact endpointにいる同角度refreshでも、current driverを片側の実角度へ戻して
/// からendpointへ近付ければ運動枝を幾何で再確認できる。hard driverとpreferred targetの
/// 区分は呼出し時のまま保ち、返す物理Frameや角度は一切変更しない。
fn driver_approach_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    drivers: &[Driver],
    targets: Option<&HashMap<EdgeId, f64>>,
    topology: &PreparedTopology,
    result: &SolveResult,
    endpoint: SurfaceEndpoint<'_>,
) -> Option<crate::surface_order::SurfaceOrder> {
    if topology.forest().loops.is_empty() || (drivers.is_empty() && targets.is_none()) {
        return None;
    }
    let checkpoints = &crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG
        [crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG.len() - 4..];
    let approaches_flat = |angle: f64| {
        angle.is_finite()
            && angle.abs()
                >= crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG
                    .last()
                    .copied()
                    .unwrap_or(180.0)
                    - RELAXATION_EPS_DEG
    };
    if !drivers
        .iter()
        .any(|driver| approaches_flat(driver.target_angle_deg))
        && !targets
            .into_iter()
            .flat_map(HashMap::values)
            .copied()
            .any(approaches_flat)
    {
        return None;
    }

    let mut warm = result.angles.clone();
    let mut path_frames = Vec::with_capacity(checkpoints.len());
    for &checkpoint in checkpoints {
        let approach = |angle: f64| {
            if approaches_flat(angle) {
                angle.signum() * checkpoint
            } else {
                angle
            }
        };
        let checkpoint_drivers = drivers
            .iter()
            .map(|driver| Driver {
                hinge: driver.hinge,
                target_angle_deg: approach(driver.target_angle_deg),
            })
            .collect::<Vec<_>>();
        let checkpoint_targets = targets.map(|targets| {
            targets
                .iter()
                .map(|(&hinge, &angle)| (hinge, approach(angle)))
                .collect::<HashMap<_, _>>()
        });
        let solved = solve_requested_prepared(
            cp,
            faces,
            &checkpoint_drivers,
            checkpoint_targets.as_ref(),
            Some(&warm),
            topology,
        );
        if !is_finite_result(&solved, faces.len()) {
            return None;
        }
        for driver in &checkpoint_drivers {
            let actual = solved.angles.get(&driver.hinge).copied()?;
            if canonical_delta_deg(actual, driver.target_angle_deg).abs() > RELAXATION_EPS_DEG {
                return None;
            }
        }
        warm = solved.angles;
        path_frames.push(solved.frame);
    }
    complete_motion_path_surface_order(
        cp,
        faces,
        &path_frames,
        endpoint.frame,
        endpoint.constraints,
    )
}

/// `snap_to_flat` した全ヒンジを、現在の解をwarmにして同時に満たす表示順専用Frame。
/// 物理解の返却Frameは変えず、完全な束でどの面対が重なるかを選ぶためだけに使う。
fn snapped_motion_surface_frame(
    cp: &CreasePattern,
    faces: &[Face],
    topology: &PreparedTopology,
    stack_angles: &HashMap<EdgeId, f64>,
    warm: &HashMap<EdgeId, f64>,
) -> Option<Frame3D> {
    let mut drivers = stack_angles
        .iter()
        .map(|(&hinge, &target_angle_deg)| Driver {
            hinge,
            target_angle_deg,
        })
        .collect::<Vec<_>>();
    drivers.sort_unstable_by_key(|driver| driver.hinge);
    let solved = solve_requested_prepared(cp, faces, &drivers, None, Some(warm), topology);
    if !is_finite_result(&solved, faces.len()) {
        return None;
    }
    for driver in &drivers {
        let actual = solved.angles.get(&driver.hinge).copied()?;
        if canonical_delta_deg(actual, driver.target_angle_deg).abs() > RELAXATION_EPS_DEG {
            return None;
        }
    }
    Some(solved.frame)
}

/// 平らな状態から現在の角度まで、solveせずに伝播だけで再生した経路で上下を決める。
///
/// 完全に重なった束は最終形だけでは上下を決められない。折る途中では必ず離れて
/// いるので、終点に最も近い「離れていた姿勢」の上下をそのまま採る。
fn propagated_flat_path_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    topology: &PreparedTopology,
    result: &SolveResult,
    exact_constraints: &[(FaceId, FaceId)],
) -> Option<crate::surface_order::SurfaceOrder> {
    let forest = topology.forest();
    let final_rad = forest
        .hinges
        .iter()
        .map(|hinge| {
            result
                .angles
                .get(hinge)
                .copied()
                .unwrap_or(0.0)
                .to_radians()
        })
        .collect::<Vec<_>>();
    if final_rad.iter().any(|angle| !angle.is_finite()) {
        return None;
    }
    let mut path_frames =
        Vec::with_capacity(crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG.len());
    for &checkpoint in &crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG {
        let checkpoint = checkpoint.to_radians();
        let clamped = final_rad
            .iter()
            .map(|angle| angle.signum() * angle.abs().min(checkpoint))
            .collect::<Vec<_>>();
        let folded = tree::fold_frame(forest, faces, &clamped);
        path_frames.push(tree::to_frame3d_with_surface_order(
            cp, faces, &folded, false,
        ));
    }
    crate::surface_order::derive_surface_order_from_frame_path(
        cp,
        faces,
        &path_frames,
        &result.frame,
        exact_constraints,
    )
    .ok()
}

/// 解けた角度から、180°の折り目が決める厳密な上下の組を作る。
fn exact_stack_constraints_of(
    faces: &[Face],
    topology: &PreparedTopology,
    angles: &HashMap<EdgeId, f64>,
) -> Vec<(FaceId, FaceId)> {
    let forest = topology.forest();
    let angles_rad = forest
        .hinges
        .iter()
        .map(|hinge| angles.get(hinge).copied().unwrap_or(0.0).to_radians())
        .collect::<Vec<_>>();
    let transforms = tree::propagate_with(forest, faces.len(), &angles_rad);
    tree::exact_stack_constraints(forest, faces, &angles_rad, &transforms)
}

/// 刻印する重なり順は、いま表示している姿勢そのものを説明しなければならない。
/// そのため経路の終点には、要求した折り目だけでなく**解けた全ヒンジ角**を使う。
///
/// 以前は要求した折り目だけを終点にしていた。自由に動く折り目が多い操作では、
/// 経路の終点が表示している形とは別の形になり、その別の形の重なり順を刻印して
/// いた。実測では240通り中8通りで、上に来るべき面と表示が食い違っていた。
fn canonical_surface_targets(angles: &HashMap<EdgeId, f64>) -> BTreeMap<EdgeId, f64> {
    angles
        .iter()
        .filter(|(_, angle)| angle.is_finite())
        .map(|(&hinge, &angle)| (hinge, angle.clamp(-180.0, 180.0)))
        .collect()
}

/// 全明示ヒンジを同じ角度checkpointでclampして平らな状態から再生する。
/// 元のhard/preferred区分は捨て、EdgeId順の同等なdriverとして扱う。
fn canonical_motion_surface_order(
    cp: &CreasePattern,
    faces: &[Face],
    topology: &PreparedTopology,
    final_targets: &BTreeMap<EdgeId, f64>,
    exact_constraints: &[(FaceId, FaceId)],
) -> Option<crate::surface_order::SurfaceOrder> {
    if final_targets.is_empty() {
        return None;
    }
    let mut warm = None;
    let mut path_frames =
        Vec::with_capacity(crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG.len());
    for &checkpoint in &crate::surface_order::SURFACE_PATH_CHECKPOINT_DEG {
        let clamp = |angle: f64| angle.signum() * angle.abs().min(checkpoint);
        let checkpoint_drivers = final_targets
            .iter()
            .map(|(&hinge, &angle)| Driver {
                hinge,
                target_angle_deg: clamp(angle),
            })
            .collect::<Vec<_>>();
        let solved = solve_requested_prepared(
            cp,
            faces,
            &checkpoint_drivers,
            None,
            warm.as_ref(),
            topology,
        );
        if !is_finite_result(&solved, faces.len()) {
            return None;
        }
        for driver in &checkpoint_drivers {
            let actual = solved.angles.get(&driver.hinge).copied()?;
            if canonical_delta_deg(actual, driver.target_angle_deg).abs() > RELAXATION_EPS_DEG {
                return None;
            }
        }
        path_frames.push(solved.frame);
        warm = Some(solved.angles);
    }
    // 経路の終点は、重なっている面対を選ぶための平らな束を作るためだけに使う。
    // どの折り目を折り切ったとみなすかは `STACK_FLAT_THRESHOLD_DEG` にそろえる。
    let exact_drivers = final_targets
        .iter()
        .map(|(&hinge, &target)| Driver {
            hinge,
            target_angle_deg: crate::surface_order::snap_to_flat(target),
        })
        .collect::<Vec<_>>();
    let exact = solve_requested_prepared(cp, faces, &exact_drivers, None, warm.as_ref(), topology);
    if !is_finite_result(&exact, faces.len()) {
        return None;
    }
    for driver in &exact_drivers {
        let actual = exact.angles.get(&driver.hinge).copied()?;
        if canonical_delta_deg(actual, driver.target_angle_deg).abs() > RELAXATION_EPS_DEG {
            return None;
        }
    }
    crate::surface_order::derive_surface_order_from_frame_path(
        cp,
        faces,
        &path_frames,
        &exact.frame,
        exact_constraints,
    )
    .ok()
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
    if near_fully_folded {
        cap
    } else {
        wanted.min(cap)
    }
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
    prevent_contact: bool,
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
                || (self.prevent_contact && self_intersects(&solved.frame))
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
                solved.relaxations =
                    collect_relaxations(&solved.angles, self.drivers, self.targets);
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
            || (self.prevent_contact && self_intersects(&solved.frame))
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
struct ContactAvoidanceContext<'a> {
    cp: &'a CreasePattern,
    faces: &'a [Face],
    drivers: &'a [Driver],
    targets: Option<&'a HashMap<EdgeId, f64>>,
    warm: Option<&'a HashMap<EdgeId, f64>>,
    topology: &'a PreparedTopology,
}

fn avoid_contact(
    context: ContactAvoidanceContext<'_>,
    original: SolveResult,
    report_contact: bool,
) -> SolveResult {
    let ContactAvoidanceContext {
        cp,
        faces,
        drivers,
        targets,
        warm,
        topology,
    } = context;
    let Some(original_rank) = ContactCandidate::new(original.clone(), drivers, targets, warm)
    else {
        return original;
    };
    if original_rank.contact.pair_count == 0 {
        return original;
    }

    let forest = topology.forest();
    let hard: BTreeSet<EdgeId> = drivers.iter().map(|driver| driver.hinge).collect();
    let pairs: Vec<_> = contact_witnesses(&original.frame)
        .into_iter()
        .map(|witness| witness.faces)
        .collect();
    let candidates = contact_hinge_candidates(forest, faces, &pairs, &hard, targets);
    if candidates.is_empty() {
        return with_contact_warning(original, report_contact);
    }

    let search = ContactAngleSearch {
        cp,
        faces,
        forest,
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
        best.frame = frame_from_angles(cp, faces, forest, &best.angles, &original.frame.warnings);
    }
    if faces.len() <= 100 && best.contact.pair_count > 0 && loop_anchor != original.angles {
        // 非木ヒンジはtree直接配置では幾何へ現れない。接触経路上で最も深く
        // 折れている1本を主anchorにし、閉包再投影による周辺ヒンジの譲歩を評価する。
        // 共同anchorは固定2候補目として残す。
        best.angles.clone_from(&loop_anchor);
        best.frame = frame_from_angles(cp, faces, forest, &best.angles, &original.frame.warnings);
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
            let frame = frame_from_angles(cp, faces, forest, &angles, &original.frame.warnings);
            if contact_metrics(&frame).pair_count == 0 {
                high = mid;
                best.angles = angles;
                best.frame = frame;
            } else {
                low = mid;
            }
        }
    }

    let on_cycle = hinges_on_cycles(forest, faces.len());
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
        let mut solved = solver::solve_near_exact_prepared(
            cp,
            faces,
            drivers,
            &contact_targets,
            Some(&best.angles),
            topology,
        );
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
            let mut solved = solver::solve_near_exact_prepared(
                cp,
                faces,
                drivers,
                &contact_targets,
                Some(&goal),
                topology,
            );
            solved.iterations = original.iterations.saturating_add(solved.iterations);
            solved.relaxations = collect_relaxations(&solved.angles, drivers, targets);
            solved
        } else {
            let mut direct = original.clone();
            direct.angles = goal;
            direct.frame =
                frame_from_angles(cp, faces, forest, &direct.angles, &original.frame.warnings);
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
        return with_contact_warning(original, report_contact);
    };
    if candidate_rank.is_better_than(&original_rank) {
        if candidate_rank.contact.pair_count > 0 {
            candidate = with_contact_warning(candidate, report_contact);
        }
        candidate
    } else {
        with_contact_warning(original, report_contact)
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

fn with_contact_warning(mut result: SolveResult, enabled: bool) -> SolveResult {
    if enabled
        && !result
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
    use super::{
        CanonicalCandidateKind, CanonicalCandidateScore, ContactCandidate, angle_priority_costs,
        canonical_anchor_samples, canonical_candidate_specs, canonical_document_seed,
        canonical_kind_seed, canonical_requested_errors, canonical_uniform_seed,
        continuation_steps, interpolate_angle_maps, interpolated_drivers, interpolated_targets,
        max_requested_delta, solve_canonical_motion_prepared, solve_motion,
        solve_motion_with_contact_options, stamp_motion_surface_order,
    };
    use crate::{ContactMetrics, SolveResult, solver};
    use ori3_cp::extract_faces;
    use ori3_model::{CreasePattern, Document, Driver, Edge, EdgeKind, Frame3D, Paper, Vertex};
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

    #[test]
    fn surface_order_authority_requires_a_finite_complete_stamp() {
        let document = Document::new(Paper {
            width_mm: 100.0,
            height_mm: 100.0,
        });
        let faces = extract_faces(&document.cp);
        let solved = solve_motion(&document.cp, &faces, &[], None, None, false);

        assert!(solved.surface_order_authoritative);

        let topology = solver::prepare_topology(&document.cp, &faces);
        let mut nonfinite = solved.result;
        nonfinite.closure_rms = f64::NAN;
        let (_, authoritative) = stamp_motion_surface_order(
            &document.cp,
            &faces,
            &[],
            None,
            &topology,
            &[],
            &mut nonfinite,
        );

        assert!(
            !authoritative,
            "nonfinite fallbackは物理順のauthorityではない"
        );

        let (canonical, kind) = solve_canonical_motion_prepared(
            &document.cp,
            &faces,
            &[],
            None,
            None,
            super::MotionContactOptions {
                detect: false,
                prevent: false,
            },
        );
        assert_eq!(kind, CanonicalCandidateKind::Direct);
        assert!(canonical.surface_order.is_some());
        assert!(canonical.surface_order_authoritative);
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

    #[test]
    fn canonical_candidate_rank_prefers_closed_then_error_then_stable_ordinal() {
        let ordered_kinds = [
            CanonicalCandidateKind::AnchoredUniformMinus90 {
                hinge: 17,
                sample_index: 0,
            },
            CanonicalCandidateKind::AnchoredUniformMinus90 {
                hinge: 19,
                sample_index: 1,
            },
            CanonicalCandidateKind::AnchoredUniformMinus90 {
                hinge: 21,
                sample_index: 2,
            },
            CanonicalCandidateKind::AnchoredUniformPlus90 {
                hinge: 17,
                sample_index: 0,
            },
            CanonicalCandidateKind::AnchoredUniformPlus90 {
                hinge: 19,
                sample_index: 1,
            },
            CanonicalCandidateKind::AnchoredUniformPlus90 {
                hinge: 21,
                sample_index: 2,
            },
            CanonicalCandidateKind::Direct,
            CanonicalCandidateKind::DocumentSeed,
            CanonicalCandidateKind::DocumentOverlay,
            CanonicalCandidateKind::KindSignedPlus90,
            CanonicalCandidateKind::KindSignedMinus90,
            CanonicalCandidateKind::KindSignedPlus180,
            CanonicalCandidateKind::KindSignedMinus180,
            CanonicalCandidateKind::UniformPlus90,
            CanonicalCandidateKind::UniformMinus90,
            CanonicalCandidateKind::UniformPlus180,
            CanonicalCandidateKind::UniformMinus180,
        ];
        for (ordinal, kind) in ordered_kinds.into_iter().enumerate() {
            assert_eq!(usize::from(kind.ordinal()), ordinal);
        }

        let closed = CanonicalCandidateScore {
            finite: true,
            closed: true,
            max_target_error: 90.0,
            squared_target_error: 8_100.0,
            ordinal: 9,
        };
        let open_exact = CanonicalCandidateScore {
            finite: true,
            closed: false,
            max_target_error: 0.0,
            squared_target_error: 0.0,
            ordinal: 0,
        };
        assert!(closed.is_better_than(open_exact));

        let smaller_l2 = CanonicalCandidateScore {
            max_target_error: 90.0 + 0.5e-9,
            squared_target_error: 8_000.0,
            ordinal: 10,
            ..closed
        };
        assert!(smaller_l2.is_better_than(closed));

        let stable_earlier = CanonicalCandidateScore {
            ordinal: 3,
            ..closed
        };
        assert!(stable_earlier.is_better_than(closed));
    }

    #[test]
    fn canonical_requested_error_includes_hard_and_hard_overrides_preferred() {
        let angles = HashMap::from([(17, 10.0), (19, 0.0)]);
        let preferred = HashMap::from([(17, -90.0), (19, 90.0)]);
        let hard = [Driver {
            hinge: 17,
            target_angle_deg: 20.0,
        }];

        let (maximum, squared, finite) =
            canonical_requested_errors(&angles, &hard, Some(&preferred));

        assert!(finite);
        assert_eq!(maximum, 90.0);
        assert_eq!(squared, 10.0_f64.powi(2) + 90.0_f64.powi(2));
    }

    #[test]
    fn canonical_anchor_sampling_is_bounded_and_excludes_aux_and_hard() {
        let mut document = sa_document();
        document.cp.edges.push(Edge {
            id: 35,
            v0: 0,
            v1: 1,
            kind: EdgeKind::Aux,
        });
        let requested = HashMap::from([
            (17, -90.0),
            (19, 90.0),
            (21, 90.0),
            (23, 90.0),
            (25, 90.0),
            (35, 45.0),
        ]);
        let hard = [
            Driver {
                hinge: 17,
                target_angle_deg: -90.0,
            },
            Driver {
                hinge: 35,
                target_angle_deg: 45.0,
            },
        ];
        assert_eq!(
            canonical_anchor_samples(&document.cp, &hard, Some(&requested)),
            vec![19, 23, 25]
        );

        let raw_seed = HashMap::from([(17, 1.0), (19, 2.0), (35, 3.0), (999, 4.0)]);
        let sanitized = canonical_document_seed(&document.cp, &raw_seed);
        assert_eq!(sanitized, HashMap::from([(17, 1.0), (19, 2.0)]));
        assert!(!canonical_kind_seed(&document.cp, 90.0).contains_key(&35));
        assert!(!canonical_uniform_seed(&document.cp, -90.0).contains_key(&35));

        let specs =
            canonical_candidate_specs(&document.cp, &hard, Some(&requested), Some(&raw_seed));
        assert_eq!(specs.len(), 17);
        assert!(specs.iter().all(|spec| {
            spec.seed
                .as_ref()
                .is_none_or(|seed| !seed.contains_key(&35) && !seed.contains_key(&999))
                && spec
                    .preferred
                    .as_ref()
                    .is_none_or(|targets| !targets.contains_key(&35))
                && spec.hard.iter().all(|driver| driver.hinge != 35)
        }));
    }

    #[test]
    fn continuation_preserves_signed_path_across_180() {
        let start = HashMap::from([(17, 180.0)]);
        let drivers = [Driver {
            hinge: 17,
            target_angle_deg: -175.0,
        }];
        let targets = HashMap::from([(17, -175.0)]);
        let fractions = [0.25, 0.5, 0.75, 1.0];
        let expected = [91.25, 2.5, -86.25, -175.0].map(f64::to_bits);

        assert_eq!(max_requested_delta(&start, &drivers, None), 355.0);
        assert_eq!(max_requested_delta(&start, &[], Some(&targets)), 355.0);

        let driver_checkpoints = fractions.map(|t| {
            interpolated_drivers(&drivers, &start, t)[0]
                .target_angle_deg
                .to_bits()
        });
        let target_checkpoints = fractions.map(|t| {
            interpolated_targets(Some(&targets), &start, t).expect("targets are present")[&17]
                .to_bits()
        });
        let contact_checkpoints =
            fractions.map(|t| interpolate_angle_maps(&start, &targets, t)[&17].to_bits());

        assert_eq!(driver_checkpoints, expected);
        assert_eq!(target_checkpoints, expected);
        assert_eq!(
            contact_checkpoints, expected,
            "接触回避の二分探索も同じ符号付き経路を通る"
        );
    }

    fn sa_document() -> Document {
        fn vertex(id: u32, x: f64, y: f64) -> Vertex {
            Vertex { id, pos: [x, y] }
        }
        fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
            Edge { id, v0, v1, kind }
        }

        let mut document = Document::new(Paper {
            width_mm: 150.0,
            height_mm: 150.0,
        });
        document.cp = CreasePattern {
            vertices: vec![
                vertex(0, 0.0, 0.0),
                vertex(1, 1.0, 0.0),
                vertex(2, 1.0, 1.0),
                vertex(3, 0.0, 1.0),
                vertex(4, 0.0, 0.5),
                vertex(5, 1.0, 0.5),
                vertex(6, 0.5, 1.0),
                vertex(7, 0.5, 0.0),
                vertex(8, 0.5, 0.5),
                vertex(9, 0.207_106_781_186_547_52, 0.5),
                vertex(10, 0.5, 0.792_893_218_813_452_5),
                vertex(11, 0.5, 0.207_106_781_186_547_52),
                vertex(12, 0.792_893_218_813_452_5, 0.5),
            ],
            edges: vec![
                edge(4, 3, 4, EdgeKind::Border),
                edge(5, 4, 0, EdgeKind::Border),
                edge(6, 1, 5, EdgeKind::Border),
                edge(7, 5, 2, EdgeKind::Border),
                edge(9, 2, 6, EdgeKind::Border),
                edge(10, 6, 3, EdgeKind::Border),
                edge(11, 0, 7, EdgeKind::Border),
                edge(12, 7, 1, EdgeKind::Border),
                edge(17, 0, 8, EdgeKind::Valley),
                edge(18, 8, 2, EdgeKind::Valley),
                edge(19, 4, 9, EdgeKind::Mountain),
                edge(20, 9, 8, EdgeKind::Mountain),
                edge(21, 0, 9, EdgeKind::Mountain),
                edge(22, 9, 3, EdgeKind::Mountain),
                edge(23, 6, 10, EdgeKind::Mountain),
                edge(24, 10, 8, EdgeKind::Mountain),
                edge(25, 2, 10, EdgeKind::Mountain),
                edge(26, 10, 3, EdgeKind::Mountain),
                edge(27, 8, 11, EdgeKind::Mountain),
                edge(28, 11, 7, EdgeKind::Mountain),
                edge(29, 0, 11, EdgeKind::Mountain),
                edge(30, 11, 1, EdgeKind::Mountain),
                edge(31, 8, 12, EdgeKind::Mountain),
                edge(32, 12, 5, EdgeKind::Mountain),
                edge(33, 2, 12, EdgeKind::Mountain),
                edge(34, 12, 1, EdgeKind::Mountain),
            ],
            next_vertex_id: 13,
            next_edge_id: 35,
        };
        document
    }

    fn max_vertex_delta(left: &Frame3D, right: &Frame3D) -> f64 {
        let mut left_faces: Vec<_> = left.faces.iter().collect();
        let mut right_faces: Vec<_> = right.faces.iter().collect();
        left_faces.sort_unstable_by_key(|face| face.face);
        right_faces.sort_unstable_by_key(|face| face.face);
        left_faces
            .into_iter()
            .zip(right_faces)
            .flat_map(|(left_face, right_face)| {
                assert_eq!(left_face.face, right_face.face);
                left_face.polygon.iter().zip(&right_face.polygon).flat_map(
                    |(left_point, right_point)| {
                        (0..3).map(move |axis| (left_point[axis] - right_point[axis]).abs())
                    },
                )
            })
            .fold(0.0_f64, f64::max)
    }

    fn assert_pose_bits_eq(left: &SolveResult, right: &SolveResult) {
        let mut left_angles: Vec<_> = left.angles.iter().collect();
        let mut right_angles: Vec<_> = right.angles.iter().collect();
        left_angles.sort_unstable_by_key(|(hinge, _)| **hinge);
        right_angles.sort_unstable_by_key(|(hinge, _)| **hinge);
        assert_eq!(left_angles.len(), right_angles.len());
        for ((left_hinge, left_angle), (right_hinge, right_angle)) in
            left_angles.into_iter().zip(right_angles)
        {
            assert_eq!(left_hinge, right_hinge);
            assert_eq!(left_angle.to_bits(), right_angle.to_bits());
        }

        let mut left_faces: Vec<_> = left.frame.faces.iter().collect();
        let mut right_faces: Vec<_> = right.frame.faces.iter().collect();
        left_faces.sort_unstable_by_key(|face| face.face);
        right_faces.sort_unstable_by_key(|face| face.face);
        assert_eq!(left_faces.len(), right_faces.len());
        for (left_face, right_face) in left_faces.into_iter().zip(right_faces) {
            assert_eq!(left_face.face, right_face.face);
            assert_eq!(left_face.layer, right_face.layer);
            assert_eq!(left_face.surface_rank, right_face.surface_rank);
            assert_eq!(left_face.mirrored, right_face.mirrored);
            assert_eq!(left_face.polygon.len(), right_face.polygon.len());
            for (left_point, right_point) in left_face.polygon.iter().zip(&right_face.polygon) {
                for axis in 0..3 {
                    assert_eq!(left_point[axis].to_bits(), right_point[axis].to_bits());
                }
            }
        }
    }

    #[test]
    fn canonical_sa_selects_bounded_anchor_and_is_seed_order_independent() {
        let document = sa_document();
        let faces = extract_faces(&document.cp);
        let targets_forward = HashMap::from([(17, -90.0), (19, 90.0), (21, 90.0)]);
        let targets_reverse = HashMap::from([(21, 90.0), (19, 90.0), (17, -90.0)]);
        let document_seed_forward: HashMap<_, _> = document
            .cp
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .map(|edge| (edge.id, 0.0))
            .collect();
        let document_seed_reverse: HashMap<_, _> = document
            .cp
            .edges
            .iter()
            .rev()
            .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .map(|edge| (edge.id, 0.0))
            .collect();
        let contact = super::MotionContactOptions {
            detect: true,
            prevent: false,
        };
        let specs = canonical_candidate_specs(
            &document.cp,
            &[],
            Some(&targets_forward),
            Some(&document_seed_forward),
        );
        assert_eq!(specs.len(), 17);
        assert_eq!(
            canonical_anchor_samples(&document.cp, &[], Some(&targets_forward)),
            vec![17, 19, 21]
        );
        let anchor_19_minus = specs
            .iter()
            .find(|spec| {
                spec.kind
                    == CanonicalCandidateKind::AnchoredUniformMinus90 {
                        hinge: 19,
                        sample_index: 1,
                    }
            })
            .expect("sampled hinge 19 minus candidate");
        assert_eq!(
            anchor_19_minus.hard,
            vec![Driver {
                hinge: 19,
                target_angle_deg: 90.0,
            }]
        );
        assert_eq!(
            anchor_19_minus.preferred,
            Some(HashMap::from([(17, -90.0), (21, 90.0)]))
        );
        assert_eq!(anchor_19_minus.seed.as_ref().unwrap()[&19], -90.0);

        let (first, selected) = solve_canonical_motion_prepared(
            &document.cp,
            &faces,
            &[],
            Some(&targets_forward),
            Some(&document_seed_forward),
            contact,
        );
        assert_eq!(
            selected,
            CanonicalCandidateKind::AnchoredUniformMinus90 {
                hinge: 19,
                sample_index: 1,
            }
        );
        let (second, selected_again) = solve_canonical_motion_prepared(
            &document.cp,
            &faces,
            &[],
            Some(&targets_reverse),
            Some(&document_seed_reverse),
            contact,
        );
        assert_eq!(selected_again, selected);
        assert_pose_bits_eq(&first.result, &second.result);
        assert_eq!(first.result.iterations, second.result.iterations);

        let max_error = targets_forward
            .iter()
            .map(|(&hinge, &target)| {
                super::canonical_delta_deg(first.result.angles[&hinge], target).abs()
            })
            .fold(0.0_f64, f64::max);
        assert!((max_error - 90.0).abs() <= 1e-9, "max_error={max_error}");

        let desired_17 = HashMap::from([(17, -90.0)]);
        let finish_17 = solve_canonical_motion_prepared(
            &document.cp,
            &faces,
            &[],
            Some(&desired_17),
            Some(&document_seed_forward),
            contact,
        )
        .0;
        let desired_17_21 = HashMap::from([(17, -90.0), (21, 90.0)]);
        let finish_21 = solve_canonical_motion_prepared(
            &document.cp,
            &faces,
            &[],
            Some(&desired_17_21),
            Some(&document_seed_forward),
            contact,
        )
        .0;
        assert_pose_bits_eq(
            &finish_17.result,
            &solve_canonical_motion_prepared(
                &document.cp,
                &faces,
                &[],
                Some(&desired_17),
                Some(&document_seed_forward),
                contact,
            )
            .0
            .result,
        );
        let hard_19 = [Driver {
            hinge: 19,
            target_angle_deg: 90.0,
        }];
        let follow_19 = solve_motion_with_contact_options(
            &document.cp,
            &faces,
            &hard_19,
            Some(&desired_17_21),
            Some(&finish_21.result.angles),
            contact,
        );
        let jump = max_vertex_delta(&follow_19.result.frame, &first.result.frame);
        assert!(jump < 0.637_774, "hard19 jump={jump}");
    }

    #[test]
    fn canonical_sa_is_never_worse_than_follow_at_all_18_gesture_boundaries() {
        let document = sa_document();
        let faces = extract_faces(&document.cp);
        let document_seed: HashMap<_, _> = document
            .cp
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
            .map(|edge| (edge.id, 0.0))
            .collect();
        let target = |hinge| match hinge {
            17 => -90.0,
            19 | 21 => 90.0,
            _ => unreachable!("sa scan only uses hinges 17, 19, and 21"),
        };
        let orders = [
            [17, 19, 21],
            [17, 21, 19],
            [19, 17, 21],
            [19, 21, 17],
            [21, 17, 19],
            [21, 19, 17],
        ];
        let contact = super::MotionContactOptions {
            detect: false,
            prevent: false,
        };
        let mut checked = 0usize;

        for order in orders {
            let mut desired = HashMap::new();
            let mut warm = document_seed.clone();
            for active in order {
                let driver = [Driver {
                    hinge: active,
                    target_angle_deg: target(active),
                }];
                let follow = solve_motion_with_contact_options(
                    &document.cp,
                    &faces,
                    &driver,
                    (!desired.is_empty()).then_some(&desired),
                    Some(&warm),
                    contact,
                );
                desired.insert(active, target(active));
                let canonical = solve_canonical_motion_prepared(
                    &document.cp,
                    &faces,
                    &[],
                    Some(&desired),
                    Some(&document_seed),
                    contact,
                )
                .0;
                let wrapped_max_error = |result: &SolveResult| {
                    desired
                        .iter()
                        .map(|(&hinge, &wanted)| {
                            super::canonical_delta_deg(result.angles[&hinge], wanted).abs()
                        })
                        .fold(0.0_f64, f64::max)
                };
                let follow_error = wrapped_max_error(&follow.result);
                let canonical_error = wrapped_max_error(&canonical.result);
                assert!(
                    canonical_error <= follow_error + 1e-9,
                    "order={order:?} boundary={} active={active} canonical={canonical_error} follow={follow_error}",
                    desired.len()
                );
                warm = canonical.result.angles;
                checked += 1;
            }
        }
        assert_eq!(checked, 18);
    }
}
