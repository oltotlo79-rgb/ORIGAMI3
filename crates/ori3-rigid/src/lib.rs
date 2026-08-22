//! ori3-rigid: 折り線の角度から紙の立体形状を計算する剛体折りソルバー。

pub mod intersect;
pub mod motion;
pub mod seam;
pub mod solver;
pub mod support;
mod surface_order;
pub mod symmetry;
pub mod tree;

pub use intersect::{
    ContactMetrics, ContactWitness, MAX_CONTACT_WITNESSES, PENETRATION_WARNING, contact_metrics,
    contact_witnesses, derive_layer_order, layer_order_conflicts, self_intersection_pairs,
    self_intersects, suspect_hinges, suspect_hinges_for_intersections,
};
pub use motion::{
    MotionContactOptions, MotionSolveResult, SurfaceOrderDiagnostics, SurfaceOrderSource,
    solve_motion, solve_motion_once, solve_motion_with_contact_options,
};
pub use seam::max_seam_gap;
pub use solver::{
    AngleRelaxation, SolveResult, solve, solve_near, solve_near_exact,
    solve_near_exact_without_surface_order,
};
pub use support::{
    DEFAULT_SUPPORT_TOLERANCE, SupportError, SupportPlane, ThreePointSupport, three_point_support,
    three_point_support_with_tolerance,
};
pub use surface_order::{
    AuthoritativeSurfaceOrder, SurfaceOrderProvenance, solve_authoritative_surface_order,
    stamp_surface_order,
};
pub use symmetry::{
    DEFAULT_REFLECTION_ANGLE_TOLERANCE_DEG, DEFAULT_REFLECTION_PROJECTIONS,
    ReflectionSymmetryError, ReflectionSymmetrySolveResult, solve_near_with_reflection_symmetry,
};
pub use tree::{
    FoldedFrame, propagate, surface_order_from_angles, surface_order_from_angles_flat_path,
    to_frame3d,
};
