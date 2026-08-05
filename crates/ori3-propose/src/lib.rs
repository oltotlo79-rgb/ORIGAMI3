//! ori3-propose: 骨格指定から展開図を自動提案する計算。

pub mod packing;
pub mod skeleton;

pub use packing::{Packing, pack};
pub use skeleton::{MAX_LEAVES, Skeleton, SkeletonNode};
