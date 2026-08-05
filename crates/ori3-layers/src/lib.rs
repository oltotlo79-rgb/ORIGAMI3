//! ori3-layers: 平らに畳んだ状態の紙の重なり順と折り操作の管理。

pub mod flat_state;

pub use flat_state::{FlatState, point_in_face, representative_point};
