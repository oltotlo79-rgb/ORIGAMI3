//! ori3-propose: 骨格指定から展開図を自動提案する計算。

pub mod enumerate;
pub mod finish;
pub mod generate;
pub mod packing;
pub mod plan;
pub mod plan_generic;
pub mod plan_history;
pub mod search;
pub mod skeleton;
pub mod trace;
pub mod triangulate;
pub mod verify;

pub use enumerate::{
    FoldLine, FoldSession, MAX_SEAM_GAP, MoveReport, PoseProblem, PoseScan, RejectedMove,
    Unverified, VerifiedMove,
};
pub use finish::{
    FinishGaps, FinishTarget, FinishedForm, MeasuredTip, POSITION_GAP_MAX, TargetTip, count_gap,
    finish_gaps, length_gap, position_gap, width_gap,
};
pub use generate::{LeafSite, LeafVertex, ProposalResult, generate};
pub use packing::{LeafCircle, Packing, TipTargets, body_on_paper, pack, tip_targets};
pub use plan::{
    CreaseLine, FoldedMask, MAX_LINES, SearchLimits, SearchStats, StopReason, crease_lines, search,
};
pub use plan_generic::GenericPlanner;
pub use plan_history::HistoryPlanner;
pub use search::{
    CompletionTolerance, FLAP_RADIUS, FoldGoal, GapWeights, MIN_TIP_LENGTH, RankedMove,
    SCORE_QUANTUM, SearchBudget, SearchOutcome, SearchStop, TipSite, search_to_completion,
    search_to_finish,
};
pub use skeleton::{MAX_LEAVES, Skeleton, SkeletonNode, TIP_POS_MAX, TIP_POS_MIN, TipPos2d};
pub use trace::{
    CreaseRole, CreaseTrace, FinishedPart, FoldPlanTrace, MoleculeCorner, MoleculePair,
    MoleculeRelation, MoleculeTrace, PaperSide, RegionRef, RegionTrace, TraceChecks, check_trace,
};
pub use verify::{
    CheckedToFinish, FinalCheck, PartialPlan, StepCheck, StepFailure, VerifiedPlan, VerifyFailure,
    VerifyReport, verify_fold_order, verify_search_completion, verify_search_outcome,
};
