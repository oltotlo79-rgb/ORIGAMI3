//! ori3-geometry: 交差・射影・鏡映などの幾何計算の基本部品。

pub mod isometry;
pub mod primitives;

pub use isometry::Isometry2;
pub use primitives::{dist_point_segment, point_on_segment, reflect_across_line, seg_intersection};
