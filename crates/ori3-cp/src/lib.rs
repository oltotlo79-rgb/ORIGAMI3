//! ori3-cp: 展開図(平面グラフ)の管理・面抽出・スナップ・作図補助。

pub mod faces;
pub mod graph;
pub mod validate;

pub use faces::{Face, extract_faces};
pub use graph::{insert_segment, move_vertex, remove_edges};
pub use validate::validate;
