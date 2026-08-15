//! ori3-layers: 平らに畳んだ状態の紙の重なり順と折り操作の管理。

pub mod compound;
pub mod crease_only;
pub mod flat_motion;
pub mod flat_state;
pub mod fold_network;
pub mod fold_through;
pub mod folded_query;
pub mod plane_pullback;
pub mod pose_motion;
pub mod pose_oracle;
pub mod pose_step;
pub mod precrease_collapse;
pub mod rabbit_ear;
pub mod replay;
pub mod spatial_fold;
pub mod step_oracle;
pub mod techniques;

pub use compound::{CompoundMotionSession, CompoundTechnique, compose_flat_motion_step};
pub use crease_only::{
    CreaseOnlyInput, ReverseOpenCreaseInput, crease_only, reverse_open_crease_sense,
};
pub use flat_motion::{
    FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform, flat_motion,
};
pub use flat_state::{
    FlatState, layers_at_point, layers_from_top_at_point, point_in_face, representative_point,
};
pub use fold_network::{ReverseFoldNetworkInput, reverse_fold_network};
pub use fold_through::{
    FOLD_PENETRATION_WARNING, FoldDirection, FoldThroughInput, FoldThroughProposal,
    FoldThroughResult, fold_through, fold_through_with_additional_crease, propose_fold_through,
    resolve_driver_edges,
};
pub use plane_pullback::{
    FaceCreaseSegments, FoldPlane3D, PlanePullbackResult, pull_back_plane_to_faces,
};
pub use pose_motion::{
    PoseAngleTarget, PoseEdgeActivation, PoseMotionInput, PoseMotionResult,
    solve_and_apply_pose_step,
};
pub use pose_oracle::{
    FaceHit, PoseDepthExpectation, PoseDepthSample, PoseDifference, PoseExpectation, PoseFeatures,
    PoseLandmarkExpectation, PoseLandmarkSample, PoseOracleReport, PoseOracleTolerance, Ray3,
    RaycastError, evaluate_pose, raycast_faces,
};
pub use pose_step::{PoseStepInput, PoseStepResult, apply_pose_step};
pub use precrease_collapse::{PrecreaseCollapseInput, collapse_precrease_network};
pub use rabbit_ear::{RabbitEarInput, rabbit_ear};
pub use replay::{
    LayerTransition, ReplayResult, flat_state_at, replay, replay_with_faces, saved_layer_order_at,
};
pub use spatial_fold::{SpatialFoldInput, SpatialFoldResult, fold_from_plane_3d};
pub use techniques::{
    TechniqueInput, inside_reverse, open_sink, outside_reverse, petal, pleat, squash, swivel, twist,
};
