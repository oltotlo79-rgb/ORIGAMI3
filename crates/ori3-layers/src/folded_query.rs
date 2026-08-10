//! Read-only geometric queries for a flat-folded state.
//!
//! [`FoldedQuery`] maps face polygons and crease-pattern edges into the folded
//! plane once, then exposes the positional queries used by diagram language
//! such as "the rightmost corner" and "the second sheet from the front".

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use glam::DVec2;
use ori3_cp::Face;
use ori3_geometry::dist_point_segment;
use ori3_model::{CreasePattern, EPS, EdgeId, FaceId, VertexId};

use crate::flat_state::{
    FlatState, layers_at_point, layers_from_top_at_point, representative_point,
};

/// An axis-aligned bounding box in the folded plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoldedBounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl FoldedBounds {
    /// Returns whether `point` is inside this closed bounding box.
    #[must_use]
    pub fn contains(&self, point: [f64; 2]) -> bool {
        point[0] >= self.min[0]
            && point[0] <= self.max[0]
            && point[1] >= self.min[1]
            && point[1] <= self.max[1]
    }
}

/// Geometry of one face after applying its [`FlatState`] placement.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldedFaceGeometry {
    pub face_id: FaceId,
    /// Boundary vertices in face order, in folded-plane coordinates.
    pub polygon: Vec<[f64; 2]>,
    pub bounds: FoldedBounds,
    /// A point strictly inside the face, in folded-plane coordinates.
    pub representative_point: [f64; 2],
}

/// Common directions used to select an extreme folded vertex.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldedDirection {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl FoldedDirection {
    /// Unit-axis (or diagonal) vector whose dot product is maximized.
    #[must_use]
    pub const fn vector(self) -> [f64; 2] {
        match self {
            Self::Left => [-1.0, 0.0],
            Self::Right => [1.0, 0.0],
            Self::Top => [0.0, 1.0],
            Self::Bottom => [0.0, -1.0],
            Self::TopLeft => [-1.0, 1.0],
            Self::TopRight => [1.0, 1.0],
            Self::BottomLeft => [-1.0, -1.0],
            Self::BottomRight => [1.0, -1.0],
        }
    }
}

/// The face-owned vertex selected by an extreme-direction query.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtremeVertex {
    pub face_id: FaceId,
    pub vertex_id: VertexId,
    pub point: [f64; 2],
    /// Dot product of `point` and the requested direction vector.
    pub projection: f64,
}

/// Result of a closest-edge query.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearestEdge {
    pub edge_id: EdgeId,
    pub distance: f64,
}

/// Errors from construction and strict folded-state selection.
#[derive(Clone, Debug, PartialEq)]
pub enum FoldedQueryError {
    EmptyState,
    InvalidLayerOrder,
    MissingPlacement {
        face_id: FaceId,
    },
    MissingVertex {
        face_id: FaceId,
        vertex_id: VertexId,
    },
    MissingEdgeVertex {
        edge_id: EdgeId,
        vertex_id: VertexId,
    },
    InvalidFaceBoundary {
        face_id: FaceId,
    },
    InvalidPoint {
        point: [f64; 2],
    },
    InvalidDirection {
        direction: [f64; 2],
    },
    PointOutsidePaper {
        point: [f64; 2],
    },
    PointOnBoundary {
        point: [f64; 2],
    },
    InvalidSheetNumber {
        sheet_number: usize,
    },
    InsufficientLayers {
        point: [f64; 2],
        requested: usize,
        available: usize,
    },
    NoEdges,
}

impl fmt::Display for FoldedQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyState => write!(f, "the folded state contains no faces"),
            Self::InvalidLayerOrder => {
                write!(f, "the folded state's layer order does not match its faces")
            }
            Self::MissingPlacement { face_id } => {
                write!(f, "face {face_id} has no folded placement")
            }
            Self::MissingVertex { face_id, vertex_id } => {
                write!(f, "face {face_id} refers to missing vertex {vertex_id}")
            }
            Self::MissingEdgeVertex { edge_id, vertex_id } => {
                write!(f, "edge {edge_id} refers to missing vertex {vertex_id}")
            }
            Self::InvalidFaceBoundary { face_id } => {
                write!(f, "face {face_id} has an invalid boundary")
            }
            Self::InvalidPoint { point } => {
                write!(f, "point ({}, {}) is not finite", point[0], point[1])
            }
            Self::InvalidDirection { direction } => write!(
                f,
                "direction ({}, {}) must be finite and non-zero",
                direction[0], direction[1]
            ),
            Self::PointOutsidePaper { point } => write!(
                f,
                "no folded paper covers point ({}, {})",
                point[0], point[1]
            ),
            Self::PointOnBoundary { point } => write!(
                f,
                "point ({}, {}) lies on a face boundary",
                point[0], point[1]
            ),
            Self::InvalidSheetNumber { sheet_number } => {
                write!(f, "sheet numbers are one-based; got {sheet_number}")
            }
            Self::InsufficientLayers {
                point,
                requested,
                available,
            } => write!(
                f,
                "sheet {requested} from the front was requested at ({}, {}), but only {available} layers cover it",
                point[0], point[1]
            ),
            Self::NoEdges => write!(f, "the folded state contains no mappable edges"),
        }
    }
}

impl Error for FoldedQueryError {}

#[derive(Clone, Copy, Debug)]
struct FoldedEdgeSegment {
    edge_id: EdgeId,
    face_id: FaceId,
    a: DVec2,
    b: DVec2,
}

/// Precomputed, read-only queries over a [`FlatState`].
pub struct FoldedQuery<'a> {
    faces: &'a [Face],
    state: &'a FlatState,
    face_geometries: Vec<FoldedFaceGeometry>,
    geometry_index: HashMap<FaceId, usize>,
    edge_segments: Vec<FoldedEdgeSegment>,
}

impl<'a> FoldedQuery<'a> {
    /// Precompute all folded face geometry and all mapped CP-edge instances.
    pub fn new(
        cp: &'a CreasePattern,
        faces: &'a [Face],
        state: &'a FlatState,
    ) -> Result<Self, FoldedQueryError> {
        validate_layer_order(faces, state)?;

        let positions: HashMap<VertexId, DVec2> = cp
            .vertices
            .iter()
            .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
            .collect();
        let mut face_geometries = Vec::with_capacity(faces.len());
        let mut geometry_index = HashMap::with_capacity(faces.len());
        let mut local_polygons = Vec::with_capacity(faces.len());

        for face in faces {
            if face.vertices.len() < 3 || face.vertices.len() != face.edges.len() {
                return Err(FoldedQueryError::InvalidFaceBoundary { face_id: face.id });
            }
            let placement = state
                .placements
                .get(&face.id)
                .ok_or(FoldedQueryError::MissingPlacement { face_id: face.id })?;
            let local_polygon: Vec<DVec2> = face
                .vertices
                .iter()
                .map(|&vertex_id| {
                    positions
                        .get(&vertex_id)
                        .copied()
                        .ok_or(FoldedQueryError::MissingVertex {
                            face_id: face.id,
                            vertex_id,
                        })
                })
                .collect::<Result<_, FoldedQueryError>>()?;
            let polygon: Vec<[f64; 2]> = local_polygon
                .iter()
                .map(|&point| placement.apply(point).to_array())
                .collect();
            let bounds = bounds_of(&polygon)
                .ok_or(FoldedQueryError::InvalidFaceBoundary { face_id: face.id })?;
            let local_representative = DVec2::from(representative_point(cp, face));
            if !strictly_inside_polygon(&local_polygon, local_representative) {
                return Err(FoldedQueryError::InvalidFaceBoundary { face_id: face.id });
            }
            let folded_representative = placement.apply(local_representative).to_array();
            geometry_index.insert(face.id, face_geometries.len());
            face_geometries.push(FoldedFaceGeometry {
                face_id: face.id,
                polygon,
                bounds,
                representative_point: folded_representative,
            });
            local_polygons.push(local_polygon);
        }

        let edge_segments = map_edge_segments(cp, faces, state, &positions, &local_polygons)?;
        Ok(Self {
            faces,
            state,
            face_geometries,
            geometry_index,
            edge_segments,
        })
    }

    /// All cached face geometries, in the same order as the input `faces`.
    #[must_use]
    pub fn face_geometries(&self) -> &[FoldedFaceGeometry] {
        &self.face_geometries
    }

    /// Cached geometry for a particular face.
    #[must_use]
    pub fn face_geometry(&self, face_id: FaceId) -> Option<&FoldedFaceGeometry> {
        self.geometry_index
            .get(&face_id)
            .map(|&index| &self.face_geometries[index])
    }

    /// Select the most extreme face-owned vertex in a common direction.
    pub fn extreme(&self, direction: FoldedDirection) -> Result<ExtremeVertex, FoldedQueryError> {
        self.extreme_in_direction(direction.vector())
    }

    /// Select the face-owned vertex with the largest dot product with `direction`.
    pub fn extreme_in_direction(
        &self,
        direction: [f64; 2],
    ) -> Result<ExtremeVertex, FoldedQueryError> {
        if !is_finite(direction) || direction == [0.0, 0.0] {
            return Err(FoldedQueryError::InvalidDirection { direction });
        }
        let direction = DVec2::from(direction);
        let mut best: Option<ExtremeVertex> = None;
        for face in self.faces {
            let geometry = self
                .face_geometry(face.id)
                .ok_or(FoldedQueryError::InvalidFaceBoundary { face_id: face.id })?;
            for (&vertex_id, &point) in face.vertices.iter().zip(&geometry.polygon) {
                let projection = DVec2::from(point).dot(direction);
                let candidate = ExtremeVertex {
                    face_id: face.id,
                    vertex_id,
                    point,
                    projection,
                };
                if best.is_none_or(|current| extreme_precedes(candidate, current)) {
                    best = Some(candidate);
                }
            }
        }
        best.ok_or(FoldedQueryError::EmptyState)
    }

    /// Faces covering `point`, ordered bottom to top.
    #[must_use]
    pub fn layers_at_point(&self, point: [f64; 2]) -> Vec<FaceId> {
        if !is_finite(point) {
            return Vec::new();
        }
        let point = DVec2::from(point);
        self.state
            .order
            .iter()
            .copied()
            .filter(|face_id| {
                let Some(geometry) = self.face_geometry(*face_id) else {
                    return false;
                };
                bounds_contains_with_epsilon(geometry.bounds, point)
                    && point_in_folded_polygon(&geometry.polygon, point)
            })
            .collect()
    }

    /// Return the one-based `sheet_number` from the front (top) at `point`.
    pub fn nth_from_top_at_point(
        &self,
        point: [f64; 2],
        sheet_number: usize,
    ) -> Result<FaceId, FoldedQueryError> {
        validate_selection_point(point, sheet_number)?;
        if self.point_is_on_boundary(point) {
            return Err(FoldedQueryError::PointOnBoundary { point });
        }
        select_nth_from_top(&self.layers_at_point(point), point, sheet_number)
    }

    /// Find the closest mapped CP edge and its Euclidean distance to `point`.
    ///
    /// All CP edge kinds are included, including auxiliary construction edges.
    pub fn nearest_edge(&self, point: [f64; 2]) -> Result<NearestEdge, FoldedQueryError> {
        self.nearest_edge_matching(point, |_| true)
    }

    /// Find the closest mapped CP-edge instance that belongs to `face_id`.
    ///
    /// A material edge can have several coincident instances in the folded plane when different
    /// sheets overlap. Filtering by the owning/containing face prevents the deterministic global
    /// edge-ID tie-break from selecting an edge on a different sheet.
    pub fn nearest_edge_on_face(
        &self,
        point: [f64; 2],
        face_id: FaceId,
    ) -> Result<NearestEdge, FoldedQueryError> {
        if self.face_geometry(face_id).is_none() {
            return Err(FoldedQueryError::MissingPlacement { face_id });
        }
        self.nearest_edge_matching(point, |edge| edge.face_id == face_id)
    }

    /// Select a sheet at the strictly interior `layer_seed`, then find its closest edge to
    /// `edge_point`.
    ///
    /// `sheet_number` is one-based from the front (top). `edge_point` may lie directly on a face
    /// boundary; it is deliberately separate from `layer_seed`, for which the strict
    /// [`Self::nth_from_top_at_point`] rules still apply.
    pub fn nearest_edge_on_sheet(
        &self,
        edge_point: [f64; 2],
        layer_seed: [f64; 2],
        sheet_number: usize,
    ) -> Result<NearestEdge, FoldedQueryError> {
        if !is_finite(edge_point) {
            return Err(FoldedQueryError::InvalidPoint { point: edge_point });
        }
        let face_id = self.nth_from_top_at_point(layer_seed, sheet_number)?;
        self.nearest_edge_on_face(edge_point, face_id)
    }

    fn point_is_on_boundary(&self, point: [f64; 2]) -> bool {
        let point = DVec2::from(point);
        self.face_geometries
            .iter()
            .any(|geometry| point_on_folded_boundary(&geometry.polygon, point))
    }

    fn nearest_edge_matching(
        &self,
        point: [f64; 2],
        mut include: impl FnMut(&FoldedEdgeSegment) -> bool,
    ) -> Result<NearestEdge, FoldedQueryError> {
        if !is_finite(point) {
            return Err(FoldedQueryError::InvalidPoint { point });
        }
        let point = DVec2::from(point);
        self.edge_segments
            .iter()
            .filter(|edge| include(edge))
            .map(|edge| (edge, dist_point_segment(point, edge.a, edge.b)))
            .min_by(|(left_edge, left_distance), (right_edge, right_distance)| {
                left_distance
                    .total_cmp(right_distance)
                    .then(left_edge.edge_id.cmp(&right_edge.edge_id))
                    .then(left_edge.face_id.cmp(&right_edge.face_id))
            })
            .map(|(edge, distance)| NearestEdge {
                edge_id: edge.edge_id,
                distance,
            })
            .ok_or(FoldedQueryError::NoEdges)
    }
}

/// Strict, one-based wrapper around the existing local-layer primitives.
///
/// Unlike [`layers_from_top_at_point`], this reports an error instead of
/// silently returning a shorter selection when the requested sheet is absent.
pub fn nth_from_top_at_point(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    point: [f64; 2],
    sheet_number: usize,
) -> Result<FaceId, FoldedQueryError> {
    validate_layer_order(faces, state)?;
    validate_selection_point(point, sheet_number)?;
    if point_is_on_any_face_boundary(cp, faces, state, point)? {
        return Err(FoldedQueryError::PointOnBoundary { point });
    }

    let local_layers = layers_at_point(cp, faces, state, point);
    select_nth_from_top(&local_layers, point, sheet_number)?;

    layers_from_top_at_point(cp, faces, state, point, sheet_number - 1, 1)
        .into_iter()
        .next()
        .ok_or(FoldedQueryError::InsufficientLayers {
            point,
            requested: sheet_number,
            available: local_layers.len(),
        })
}

fn validate_selection_point(point: [f64; 2], sheet_number: usize) -> Result<(), FoldedQueryError> {
    if !is_finite(point) {
        return Err(FoldedQueryError::InvalidPoint { point });
    }
    if sheet_number == 0 {
        return Err(FoldedQueryError::InvalidSheetNumber { sheet_number });
    }
    Ok(())
}

fn select_nth_from_top(
    local_layers: &[FaceId],
    point: [f64; 2],
    sheet_number: usize,
) -> Result<FaceId, FoldedQueryError> {
    if local_layers.is_empty() {
        return Err(FoldedQueryError::PointOutsidePaper { point });
    }
    if local_layers.len() < sheet_number {
        return Err(FoldedQueryError::InsufficientLayers {
            point,
            requested: sheet_number,
            available: local_layers.len(),
        });
    }
    Ok(local_layers[local_layers.len() - sheet_number])
}

fn point_is_on_any_face_boundary(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    point: [f64; 2],
) -> Result<bool, FoldedQueryError> {
    let positions: HashMap<VertexId, DVec2> = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect();
    let folded_point = DVec2::from(point);
    for face in faces {
        if face.vertices.len() < 3 || face.vertices.len() != face.edges.len() {
            return Err(FoldedQueryError::InvalidFaceBoundary { face_id: face.id });
        }
        let placement = state
            .placements
            .get(&face.id)
            .ok_or(FoldedQueryError::MissingPlacement { face_id: face.id })?;
        let local_point = placement.inverse().apply(folded_point);
        for (index, &vertex_id) in face.vertices.iter().enumerate() {
            let next_vertex_id = face.vertices[(index + 1) % face.vertices.len()];
            let a = positions
                .get(&vertex_id)
                .copied()
                .ok_or(FoldedQueryError::MissingVertex {
                    face_id: face.id,
                    vertex_id,
                })?;
            let b =
                positions
                    .get(&next_vertex_id)
                    .copied()
                    .ok_or(FoldedQueryError::MissingVertex {
                        face_id: face.id,
                        vertex_id: next_vertex_id,
                    })?;
            if dist_point_segment(local_point, a, b) <= EPS {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn validate_layer_order(faces: &[Face], state: &FlatState) -> Result<(), FoldedQueryError> {
    if faces.is_empty() || state.order.is_empty() {
        return Err(FoldedQueryError::EmptyState);
    }
    let mut face_ids: Vec<FaceId> = faces.iter().map(|face| face.id).collect();
    let mut ordered_ids = state.order.clone();
    face_ids.sort_unstable();
    ordered_ids.sort_unstable();
    if face_ids != ordered_ids || face_ids.windows(2).any(|ids| ids[0] == ids[1]) {
        return Err(FoldedQueryError::InvalidLayerOrder);
    }
    Ok(())
}

fn bounds_of(polygon: &[[f64; 2]]) -> Option<FoldedBounds> {
    let first = *polygon.first()?;
    if !is_finite(first) {
        return None;
    }
    let mut min = first;
    let mut max = first;
    for &point in &polygon[1..] {
        if !is_finite(point) {
            return None;
        }
        min[0] = min[0].min(point[0]);
        min[1] = min[1].min(point[1]);
        max[0] = max[0].max(point[0]);
        max[1] = max[1].max(point[1]);
    }
    Some(FoldedBounds { min, max })
}

fn bounds_contains_with_epsilon(bounds: FoldedBounds, point: DVec2) -> bool {
    point.x >= bounds.min[0] - EPS
        && point.x <= bounds.max[0] + EPS
        && point.y >= bounds.min[1] - EPS
        && point.y <= bounds.max[1] + EPS
}

fn map_edge_segments(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    positions: &HashMap<VertexId, DVec2>,
    local_polygons: &[Vec<DVec2>],
) -> Result<Vec<FoldedEdgeSegment>, FoldedQueryError> {
    let mut segments = Vec::new();
    for edge in &cp.edges {
        let a = positions
            .get(&edge.v0)
            .copied()
            .ok_or(FoldedQueryError::MissingEdgeVertex {
                edge_id: edge.id,
                vertex_id: edge.v0,
            })?;
        let b = positions
            .get(&edge.v1)
            .copied()
            .ok_or(FoldedQueryError::MissingEdgeVertex {
                edge_id: edge.id,
                vertex_id: edge.v1,
            })?;
        let midpoint = (a + b) * 0.5;
        for (face, polygon) in faces.iter().zip(local_polygons) {
            if !point_in_polygon(polygon, midpoint) {
                continue;
            }
            let placement = state
                .placements
                .get(&face.id)
                .ok_or(FoldedQueryError::MissingPlacement { face_id: face.id })?;
            segments.push(FoldedEdgeSegment {
                edge_id: edge.id,
                face_id: face.id,
                a: placement.apply(a),
                b: placement.apply(b),
            });
        }
    }
    Ok(segments)
}

fn extreme_precedes(candidate: ExtremeVertex, current: ExtremeVertex) -> bool {
    candidate.projection > current.projection
        || (candidate.projection == current.projection
            && (candidate.face_id, candidate.vertex_id) < (current.face_id, current.vertex_id))
}

fn is_finite(point: [f64; 2]) -> bool {
    point[0].is_finite() && point[1].is_finite()
}

fn point_in_polygon(polygon: &[DVec2], point: DVec2) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    point_on_polygon_boundary(polygon, point) || crossing_number_is_odd(polygon, point)
}

fn strictly_inside_polygon(polygon: &[DVec2], point: DVec2) -> bool {
    polygon.len() >= 3
        && !point_on_polygon_boundary(polygon, point)
        && crossing_number_is_odd(polygon, point)
}

fn point_on_polygon_boundary(polygon: &[DVec2], point: DVec2) -> bool {
    polygon.iter().enumerate().any(|(index, &a)| {
        dist_point_segment(point, a, polygon[(index + 1) % polygon.len()]) <= EPS
    })
}

fn crossing_number_is_odd(polygon: &[DVec2], point: DVec2) -> bool {
    let mut inside = false;
    for (index, &a) in polygon.iter().enumerate() {
        let b = polygon[(index + 1) % polygon.len()];
        if (a.y > point.y) != (b.y > point.y) {
            let crossing_x = a.x + (point.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if point.x < crossing_x {
                inside = !inside;
            }
        }
    }
    inside
}

fn point_in_folded_polygon(polygon: &[[f64; 2]], point: DVec2) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    if point_on_folded_boundary(polygon, point) {
        return true;
    }

    let mut inside = false;
    for (index, &a) in polygon.iter().enumerate() {
        let b = polygon[(index + 1) % polygon.len()];
        if (a[1] > point.y) != (b[1] > point.y) {
            let crossing_x = a[0] + (point.y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if point.x < crossing_x {
                inside = !inside;
            }
        }
    }
    inside
}

fn point_on_folded_boundary(polygon: &[[f64; 2]], point: DVec2) -> bool {
    polygon.iter().enumerate().any(|(index, &a)| {
        dist_point_segment(
            point,
            DVec2::from(a),
            DVec2::from(polygon[(index + 1) % polygon.len()]),
        ) <= EPS
    })
}
