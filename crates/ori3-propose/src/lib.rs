//! ori3-propose: 骨格指定から展開図を自動提案する計算。

pub mod generate;
pub mod packing;
pub mod skeleton;
pub mod triangulate;

pub use generate::{ProposalResult, generate};
pub use packing::{Packing, pack};
pub use skeleton::{MAX_LEAVES, Skeleton, SkeletonNode, TIP_POS_MAX, TIP_POS_MIN, TipPos2d};
