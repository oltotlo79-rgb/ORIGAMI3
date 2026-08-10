//! ori3-geometry: 交差・射影・鏡映などの幾何計算の基本部品。

pub mod align;
pub mod isometry;
pub mod primitives;

pub use align::{
    ALIGN_EPS, AlignSolution, AlignmentTargetKind, FoldLine, MovingSide, align_ref_point,
    alignment_steps, angle_bisectors, distance_to_line, existing_line, extend_line,
    fold_point_onto_line, fold_point_onto_line_perpendicular, fold_two_points_onto_two_lines,
    line_through_points, moving_side_of, perpendicular_bisector, perpendicular_through_point,
    reflect_point_across_fold, solve_align, sort_by_cursor, unit_dir,
};
pub use isometry::Isometry2;
pub use primitives::{
    collinear_overlap, dist_point_segment, point_on_segment, reflect_across_line, seg_intersection,
};
