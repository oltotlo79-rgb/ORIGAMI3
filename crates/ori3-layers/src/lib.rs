//! ori3-layers: 平らに畳んだ状態の紙の重なり順と折り操作の管理。

pub mod compound;
pub mod crease_only;
pub mod flat_motion;
pub mod flat_state;
pub mod fold_network;
pub mod fold_target;
pub mod fold_through;
pub mod folded_query;
pub mod plane_pullback;
pub mod pose_motion;
pub mod pose_oracle;
pub mod pose_step;
pub mod precrease_collapse;
pub mod rabbit_ear;
pub mod replay;
pub mod spatial_crease_only;
pub mod spatial_fold;
pub mod single_reflection_plan;
pub mod composite_motion_plan;
pub mod step_oracle;
pub mod techniques;
pub mod technique_classification;

pub use compound::{CompoundMotionSession, CompoundTechnique, compose_flat_motion_step};
pub use crease_only::{
    CreaseOnlyInput, ReverseOpenCreaseInput, crease_only, reverse_open_crease_sense,
};
pub use flat_motion::{
    FlatMotionInput, HalfPlane, LayerTurn, MotionPart, MotionTransform, flat_motion,
    flat_motion_with_evidence,
};
pub use flat_state::{
    FlatState, layers_at_point, layers_from_top_at_point, point_in_face, representative_point,
};
pub use fold_network::{ReverseFoldNetworkInput, reverse_fold_network};
pub use fold_target::{
    COMPLETE_FOLD_ENDPOINT_EPS_DEG, FoldLineSection, FoldTargetAnalysis, FullFoldSign,
    HingeObservation, PleatAnalysis, PleatAnalysisError, PleatCountLimit, PleatPair,
    PleatSectionAnalysis, TopAction, analyze_fold_target_at_state, analyze_pleats,
    analyze_single_section_from_top, target_faces_for_pleat_count,
};
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
    LayerTransition, ReplayResult, flat_state_at, flat_state_with_declared_angles_at,
    fold_target_analysis_at, prefer_saved_order_when_rank_conflicts, replay, replay_with_faces,
    saved_layer_order_at,
};
pub use spatial_crease_only::{
    CanonicalNonflatPose, FaceRigidTransform3, MaterialVertex3D, NewMaterialVertex,
    SpatialCreaseOnlyError, SpatialCreaseOnlyInput, SpatialCreaseOnlyResult,
    SurfaceRelationFromTop, TopSurfaceObservation, TopSurfaceProvider,
    crease_only_top_from_material_line,
};
pub use spatial_fold::{SpatialFoldInput, SpatialFoldMode, SpatialFoldResult, fold_from_plane_3d};
pub use technique_classification::{
    AutomaticTechniqueMatch, CanonicalAdjacency, CanonicalDriver, CanonicalFaceKey,
    CanonicalSupport, FoldThroughOrigin, TechniqueClassificationRequest, TechniqueEvidence,
    TechniqueWitness, assign_technique_classification, automatic_match_from_witnesses,
    carry_over_technique_classification, classify_aligned_motion, classify_fold_through_step,
    classify_motion_plan, classify_sim011_motion, display_kind_for_technique,
};
pub use techniques::{
    TechniqueInput, inside_reverse, open_sink, outside_reverse, petal, pleat, squash, swivel, twist,
};
