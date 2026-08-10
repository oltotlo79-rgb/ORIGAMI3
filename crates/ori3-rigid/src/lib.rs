//! ori3-rigid: 折り線の角度から紙の立体形状を計算する剛体折りソルバー。

pub mod intersect;
pub mod motion;
pub mod seam;
pub mod solver;
pub mod tree;

pub use intersect::{
    PENETRATION_WARNING, layer_order_conflicts, self_intersection_pairs, self_intersects,
    suspect_hinges, suspect_hinges_for_intersections,
};
pub use motion::{MotionSolveResult, solve_motion};
pub use seam::max_seam_gap;
pub use solver::{SolveResult, solve, solve_near};
pub use tree::{FoldedFrame, propagate, to_frame3d};
