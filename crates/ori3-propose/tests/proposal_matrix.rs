//! 施策1-Dの単独runnerが使う、提案結果の負荷非依存matrix probe。
//!
//! 通常の`cargo test`では最小の1件を1回だけ確かめる。100回の32組は
//! `run-proposal-matrix.ps1`が環境変数を設定してこの同じ検査を起動する。

use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread::{self, JoinHandle};

use ori3_model::{CreasePattern, Document, FoldStep, Paper};
use ori3_propose::{
    body_on_paper, generate, pack, search_to_completion_with_control, verify_search_completion,
    CompletionTolerance, FinishTarget, FoldGoal, FoldSession, GapWeights, LeafSite, Packing,
    PoseScan, ProposalResult, SearchBudget, SearchControl, SearchStop, SearchWatchdog, Skeleton,
    SkeletonNode, TipSite, VerifiedPlan,
};
use serde::Serialize;

const PACK_STARTS: usize = 8;
const SEED: u64 = 1;
const MATRIX_ITERATIONS: usize = 100;
const EXPECTED_CANDIDATE_HASH: u64 = 0xb540_4e82_2ccd_3603;
const EXPECTED_STOP_HASH: u64 = 0xea05_a0f8_b887_39bb;
const PAPER: Paper = Paper {
    width_mm: 150.0,
    height_mm: 150.0,
};
const PLAN_BUDGET: SearchBudget = SearchBudget {
    max_states: 2,
    branch: 2,
    max_depth: SearchBudget::DEFAULT.max_depth,
    rank_scan: SearchBudget::DEFAULT.rank_scan,
    scan: SearchBudget::DEFAULT.scan,
};
const TIME_FREE_WATCHDOG: SearchWatchdog = SearchWatchdog {
    max_millis: 3_600_000,
};
const PLAN_REBUILD_SCAN: PoseScan = PoseScan { steps: 0 };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Computation {
    Serial,
    Parallel,
}

impl Computation {
    fn from_environment() -> Self {
        match environment("ORI3_PROPOSAL_MATRIX_COMPUTATION", "serial").as_str() {
            "serial" => Self::Serial,
            "parallel" => Self::Parallel,
            other => panic!("計算方法はserialまたはparallelでなければならない: {other}"),
        }
    }

    const fn contract_tag(self) -> &'static str {
        match self {
            Self::Serial => "serial",
            Self::Parallel => "parallel",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct MatrixFoldPlanDetails {
    steps: Vec<FoldStep>,
    cp: CreasePattern,
    planned: usize,
    checked: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum MatrixFoldPlan {
    CheckedToFinish {
        #[serde(flatten)]
        details: MatrixFoldPlanDetails,
    },
    Partial {
        #[serde(flatten)]
        details: MatrixFoldPlanDetails,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct MatrixCandidate {
    cp: CreasePattern,
    scale: f64,
    violations: usize,
    warnings: Vec<String>,
    sites: Vec<LeafSite>,
    fold_plan: Option<MatrixFoldPlan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RequestContract {
    candidate_json: String,
    candidate_hash: u64,
    stop_contract: String,
    stop_hash: u64,
    first_candidate_hash: u64,
    first_stop: String,
}

/// 混雑cellだけが所有するCPU負荷。Dropで全threadを止めてjoinするため、
/// 検査失敗やpanicでも次のcellへ負荷を残さない。
struct CpuLoad {
    stop: Arc<AtomicBool>,
    progress: Vec<Arc<AtomicU64>>,
    workers: Vec<JoinHandle<()>>,
}

impl CpuLoad {
    fn start() -> Self {
        let worker_count = thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let stop = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(Barrier::new(worker_count + 1));
        let mut progress = Vec::with_capacity(worker_count);
        let mut workers = Vec::with_capacity(worker_count);
        for lane in 0..worker_count {
            let stop = Arc::clone(&stop);
            let ready = Arc::clone(&ready);
            let lane_progress = Arc::new(AtomicU64::new(0));
            progress.push(Arc::clone(&lane_progress));
            workers.push(thread::spawn(move || {
                let mut value = 0x9e37_79b9_7f4a_7c15_u64 ^ lane as u64;
                ready.wait();
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..4_096 {
                        value = value
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(1_442_695_040_888_963_407);
                        value ^= value.rotate_left(17);
                    }
                    lane_progress.fetch_add(1, Ordering::Relaxed);
                    black_box(value);
                }
            }));
        }
        ready.wait();
        while progress
            .iter()
            .any(|counter| counter.load(Ordering::Relaxed) == 0)
        {
            thread::yield_now();
        }
        Self {
            stop,
            progress,
            workers,
        }
    }

    fn worker_count(&self) -> usize {
        self.workers.len()
    }
}

impl Drop for CpuLoad {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for worker in self.workers.drain(..) {
            worker.join().expect("CPU負荷threadがpanicした");
        }
        assert!(
            self.progress
                .iter()
                .all(|counter| counter.load(Ordering::Relaxed) > 0),
            "開始を確認できなかったCPU負荷threadがある"
        );
    }
}

fn environment(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn bounded_environment(name: &str, default: usize, allowed: &[usize]) -> usize {
    let value = std::env::var(name).map_or(default, |raw| {
        raw.parse::<usize>()
            .unwrap_or_else(|error| panic!("{name}={raw}を数として読めない: {error}"))
    });
    assert!(allowed.contains(&value), "{name}={value}は許されていない");
    value
}

fn requested_iterations() -> usize {
    match std::env::var("ORI3_PROPOSAL_MATRIX_ITERATIONS") {
        Ok(raw) => {
            let value = raw.parse::<usize>().unwrap_or_else(|error| {
                panic!("ORI3_PROPOSAL_MATRIX_ITERATIONS={raw}を数として読めない: {error}")
            });
            assert_eq!(
                value, MATRIX_ITERATIONS,
                "受入runnerの100回を短縮してはならない"
            );
            value
        }
        Err(std::env::VarError::NotPresent) => 1,
        Err(error) => panic!("反復回数を読めない: {error}"),
    }
}

fn star(leaves: u32) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    nodes.extend((1..=leaves).map(|id| SkeletonNode::new(id, Some(0), 1.0)));
    Skeleton { nodes }
}

fn contract_hash(text: &str) -> u64 {
    text.as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

fn calculate_candidate(
    skeleton: &Skeleton,
    packing: Packing,
    paper: &Paper,
) -> Result<(MatrixCandidate, SearchStop), String> {
    let long = paper.width_mm.max(paper.height_mm);
    let (paper_w, paper_h) = (paper.width_mm / long, paper.height_mm / long);
    let proposal = generate(skeleton, &packing, paper_w, paper_h)
        .map_err(|error| format!("展開図候補を生成できない: {error}"))?;
    calculate_plan(skeleton, packing, proposal, paper, paper_w, paper_h)
}

fn calculate_plan(
    skeleton: &Skeleton,
    packing: Packing,
    proposal: ProposalResult,
    paper: &Paper,
    paper_w: f64,
    paper_h: f64,
) -> Result<(MatrixCandidate, SearchStop), String> {
    let mut document = Document::new(paper.clone());
    document.cp = proposal.cp.clone();
    let session = FoldSession::new(&document)
        .map_err(|error| format!("生成候補から折り始められない: {error}"))?;
    let goal = FoldGoal {
        target: FinishTarget::from_skeleton(skeleton),
        body: body_on_paper(skeleton, &packing, paper_w, paper_h),
        sites: proposal
            .sites
            .iter()
            .map(|site| TipSite {
                leaf_id: site.circle.leaf_id,
                material: site.vertex.map_or(site.circle.center, |vertex| vertex.pos),
            })
            .collect(),
        layer_target: None,
    };
    let not_cancelled = || false;
    let control = SearchControl::new(TIME_FREE_WATCHDOG, &not_cancelled);
    let outcome = search_to_completion_with_control(
        &session,
        &goal,
        GapWeights::DEFAULT,
        PLAN_BUDGET,
        CompletionTolerance::DEFAULT,
        &control,
    )
    .map_err(|abort| format!("比較専用探索が中断した: {abort:?}"))?;
    let stop = outcome.stop;
    let planned = outcome.steps.len();
    let verified = verify_search_completion(
        &session,
        &outcome,
        &goal,
        GapWeights::DEFAULT,
        PoseScan::DEFAULT,
        CompletionTolerance::DEFAULT,
    );
    let checked_to_finish = matches!(&verified, VerifiedPlan::CheckedToFinish(_));
    let report = verified.report();
    let mut walk = session;
    for step in &report.steps {
        let Some(Ok(mv)) = walk.check_move(step.id, PLAN_REBUILD_SCAN) else {
            break;
        };
        if walk.apply(&mv).is_err() {
            break;
        }
    }
    let folded = walk.document();
    let fold_plan = if folded.sequence.is_empty() {
        None
    } else {
        let details = MatrixFoldPlanDetails {
            steps: folded.sequence.clone(),
            cp: folded.cp.clone(),
            planned,
            checked: folded.sequence.len(),
        };
        if checked_to_finish
            && details.checked == details.planned
            && details.planned == report.requested
        {
            Some(MatrixFoldPlan::CheckedToFinish { details })
        } else {
            Some(MatrixFoldPlan::Partial { details })
        }
    };
    Ok((
        MatrixCandidate {
            cp: proposal.cp,
            scale: packing.scale,
            violations: proposal.violations,
            warnings: proposal.warnings,
            sites: proposal.sites,
            fold_plan,
        },
        stop,
    ))
}

fn execute_request(
    candidate_count: usize,
    computation: Computation,
) -> Result<RequestContract, String> {
    let skeleton = star(6);
    let long = PAPER.width_mm.max(PAPER.height_mm);
    let packings = pack(
        &skeleton,
        PAPER.width_mm / long,
        PAPER.height_mm / long,
        SEED,
        PACK_STARTS,
    );
    if packings.len() < candidate_count {
        return Err(format!(
            "候補が{candidate_count}件必要だが{}件しかない",
            packings.len()
        ));
    }
    let selected: Vec<_> = packings
        .into_iter()
        .take(candidate_count)
        .enumerate()
        .collect();
    let mut calculated = match computation {
        Computation::Serial => selected
            .into_iter()
            .map(|(index, packing)| {
                calculate_candidate(&skeleton, packing, &PAPER).map(|result| (index, result))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Computation::Parallel => thread::scope(|scope| {
            let workers: Vec<_> = selected
                .into_iter()
                .map(|(index, packing)| {
                    let skeleton = &skeleton;
                    scope.spawn(move || {
                        calculate_candidate(skeleton, packing, &PAPER).map(|result| (index, result))
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .map_err(|_| "候補workerがpanicした".to_owned())?
                })
                .collect::<Result<Vec<_>, String>>()
        })?,
    };
    calculated.sort_by_key(|(index, _)| *index);
    let (candidates, stops): (Vec<_>, Vec<_>) = calculated
        .into_iter()
        .map(|(_, (candidate, stop))| (candidate, stop))
        .unzip();
    let candidate_json =
        serde_json::to_string(&candidates).map_err(|error| format!("候補JSON: {error}"))?;
    let first_candidate_json = serde_json::to_string(
        candidates
            .first()
            .ok_or_else(|| "候補が0件になった".to_owned())?,
    )
    .map_err(|error| format!("先頭候補JSON: {error}"))?;
    let stop_contract = stops
        .iter()
        .map(|stop| stop.contract_tag())
        .collect::<Vec<_>>()
        .join("|");
    let first_stop = stops
        .first()
        .ok_or_else(|| "停止理由が0件になった".to_owned())?
        .contract_tag()
        .to_owned();
    Ok(RequestContract {
        candidate_hash: contract_hash(&candidate_json),
        stop_hash: contract_hash(&stop_contract),
        first_candidate_hash: contract_hash(&first_candidate_json),
        candidate_json,
        stop_contract,
        first_stop,
    })
}

fn execute_requests(
    request_count: usize,
    candidate_count: usize,
    computation: Computation,
) -> Result<Vec<RequestContract>, String> {
    if request_count == 1 {
        return execute_request(candidate_count, computation).map(|contract| vec![contract]);
    }
    let ready = Arc::new(Barrier::new(request_count));
    thread::scope(|scope| {
        let workers: Vec<_> = (0..request_count)
            .map(|_| {
                let ready = Arc::clone(&ready);
                scope.spawn(move || {
                    ready.wait();
                    execute_request(candidate_count, computation)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .map_err(|_| "同時要求workerがpanicした".to_owned())?
            })
            .collect::<Result<Vec<_>, String>>()
    })
}

#[test]
fn proposal_matrix_contract() {
    let candidate_count = bounded_environment("ORI3_PROPOSAL_MATRIX_CANDIDATES", 1, &[1, 4]);
    let request_count = bounded_environment("ORI3_PROPOSAL_MATRIX_REQUESTS", 1, &[1, 2]);
    let computation = Computation::from_environment();
    let load = environment("ORI3_PROPOSAL_MATRIX_LOAD", "idle");
    assert!(matches!(load.as_str(), "idle" | "busy"));
    let expected_profile = environment(
        "ORI3_PROPOSAL_MATRIX_PROFILE",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    );
    let actual_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    assert_eq!(expected_profile, actual_profile, "組み立てprofileが違う");
    let iterations = requested_iterations();
    let busy_load = (load == "busy").then(CpuLoad::start);
    let load_threads = busy_load.as_ref().map_or(0, CpuLoad::worker_count);

    let mut reference: Option<RequestContract> = None;
    for iteration in 0..iterations {
        let requests = execute_requests(request_count, candidate_count, computation)
            .unwrap_or_else(|error| panic!("反復{}: {error}", iteration + 1));
        assert_eq!(requests.len(), request_count);
        for (request_index, request) in requests.into_iter().enumerate() {
            if let Some(want) = &reference {
                assert_eq!(
                    &request,
                    want,
                    "反復{}・要求{}で結果または停止理由が入れ替わった",
                    iteration + 1,
                    request_index + 1
                );
            } else {
                reference = Some(request);
            }
        }
    }
    let reference = reference.expect("結果が1件もない");
    if candidate_count == 4 {
        assert_eq!(
            reference.candidate_hash, EXPECTED_CANDIDATE_HASH,
            "1-Aで固定した候補JSON契約が変わった"
        );
        assert_eq!(
            reference.stop_hash, EXPECTED_STOP_HASH,
            "1-Aで固定した通常停止理由契約が変わった"
        );
    }
    println!(
        "PROPOSAL_MATRIX_RESULT profile={actual_profile} candidates={candidate_count} requests={request_count} computation={} load={load} load_threads={load_threads} iterations={iterations} candidate_hash={:016x} stop_hash={:016x} first_candidate_hash={:016x} first_stop={} stops={}",
        computation.contract_tag(),
        reference.candidate_hash,
        reference.stop_hash,
        reference.first_candidate_hash,
        reference.first_stop,
        reference.stop_contract,
    );
}
