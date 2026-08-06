//! ori3-rigid: 折り線の角度から紙の立体形状を計算する剛体折りソルバー。

pub mod intersect;
pub mod solver;
pub mod tree;

pub use intersect::{PENETRATION_WARNING, self_intersects};
pub use solver::{SolveResult, solve, solve_near};
pub use tree::{FoldedFrame, propagate, to_frame3d};
