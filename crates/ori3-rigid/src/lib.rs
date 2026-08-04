//! ori3-rigid: 折り線の角度から紙の立体形状を計算する剛体折りソルバー。

pub mod tree;

pub use tree::{FoldedFrame, propagate, to_frame3d};
