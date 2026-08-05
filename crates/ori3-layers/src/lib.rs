//! ori3-layers: 平らに畳んだ状態の紙の重なり順と折り操作の管理。

pub mod flat_motion;
pub mod flat_state;
pub mod fold_through;
pub mod replay;
pub mod techniques;

pub use flat_motion::{
    FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform, flat_motion,
};
pub use flat_state::{FlatState, point_in_face, representative_point};
pub use fold_through::{
    FoldDirection, FoldThroughInput, FoldThroughResult, fold_through, resolve_driver_edges,
};
pub use replay::{ReplayResult, flat_state_at, replay, replay_with_faces};
pub use techniques::{TechniqueInput, inside_reverse, outside_reverse, pleat};
