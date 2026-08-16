//! ori3-propose: 骨格指定から展開図を自動提案する計算。

pub mod finish;
pub mod generate;
pub mod packing;
pub mod plan;
pub mod plan_generic;
pub mod plan_history;
pub mod skeleton;
pub mod trace;
pub mod triangulate;

pub use finish::{
    FinishGaps, FinishTarget, FinishedForm, MeasuredTip, POSITION_GAP_MAX, TargetTip, count_gap,
    finish_gaps, length_gap, position_gap, width_gap,
};
pub use generate::{LeafSite, LeafVertex, ProposalResult, generate};
pub use packing::{LeafCircle, Packing, pack};
pub use plan::{
    CreaseLine, FoldedMask, MAX_LINES, SearchLimits, SearchStats, StopReason, crease_lines, search,
};
pub use plan_generic::GenericPlanner;
pub use plan_history::HistoryPlanner;
pub use skeleton::{MAX_LEAVES, Skeleton, SkeletonNode, TIP_POS_MAX, TIP_POS_MIN, TipPos2d};
pub use trace::{
    CreaseRole, CreaseTrace, FinishedPart, FoldPlanTrace, MoleculeCorner, MoleculePair,
    MoleculeRelation, MoleculeTrace, PaperSide, RegionRef, RegionTrace, TraceChecks, check_trace,
};
