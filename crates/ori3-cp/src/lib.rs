//! ori3-cp: 展開図(平面グラフ)の管理・面抽出・スナップ・作図補助。

pub mod construct;
pub mod faces;
pub mod flatfold;
pub mod graph;
mod spatial;
pub mod validate;

pub use construct::{bisector, direction_lines, divide_points, perpendicular};
pub use faces::{Face, extract_faces};
pub use flatfold::local_violations;
pub use graph::{insert_segment, move_vertex, remove_edges};
pub use validate::validate;
