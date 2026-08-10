//! Folded-state feature extraction and comparison for book-step verification.
//!
//! The oracle deliberately uses only geometry in the folded plane.  Face ids are
//! not part of an expectation because they are regenerated when the crease
//! pattern is split.  Expectations therefore remain useful while candidate
//! folds are being generated and rejected.

use std::collections::{HashMap, HashSet};
use std::fmt;

use glam::DVec2;
use ori3_cp::Face;
use ori3_model::{CreasePattern, EdgeKind, FaceId, VertexId};

use crate::{FlatState, layers_at_point};

const GEOMETRY_EPS: f64 = 1.0e-8;

/// Axis-aligned bounds in the folded plane.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BoundingBox {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl BoundingBox {
    #[must_use]
    pub fn width(self) -> f64 {
        self.max[0] - self.min[0]
    }

    #[must_use]
    pub fn height(self) -> f64 {
        self.max[1] - self.min[1]
    }
}

/// A landmark category that can be specified in expectation data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LandmarkKind {
    PaperCorner,
    CreaseEndpoint,
    /// Match either kind.  Extracted features never use this value.
    Any,
}

/// A mountain/valley sense as seen from above the folded model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldSense {
    Mountain,
    Valley,
}

/// A detected point feature in the folded plane.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LandmarkFeature {
    pub position: [f64; 2],
    pub kind: LandmarkKind,
}

/// A visible part of a crease, with its sense from the current top view.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VisibleCrease {
    pub segment: [[f64; 2]; 2],
    pub kind: FoldSense,
}

/// Global features extracted from one flat folded state.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StepFeatures {
    /// Convex hull of all folded face vertices, counter-clockwise without a
    /// repeated closing vertex.
    pub outline: Vec<[f64; 2]>,
    pub bounding_box: BoundingBox,
    /// Width divided by height.  Degenerate bounds produce `0.0`.
    pub aspect_ratio: f64,
    /// Convex-hull area divided by the unfolded material area.
    pub outline_area_ratio: f64,
    pub landmarks: Vec<LandmarkFeature>,
    pub visible_creases: Vec<VisibleCrease>,
}

/// Position and scalar tolerances used by the comparison.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StepOracleTolerance {
    pub outline_position: f64,
    pub bounding_box: f64,
    pub aspect_ratio: f64,
    pub area_ratio: f64,
    pub landmark_position: f64,
    pub visible_crease_position: f64,
}

impl Default for StepOracleTolerance {
    fn default() -> Self {
        Self {
            outline_position: 1.0e-6,
            bounding_box: 1.0e-6,
            aspect_ratio: 1.0e-6,
            area_ratio: 1.0e-6,
            landmark_position: 1.0e-6,
            visible_crease_position: 1.0e-6,
        }
    }
}

/// A required landmark at a book-diagram position.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LandmarkExpectation {
    pub position: [f64; 2],
    pub kind: LandmarkKind,
}

/// A required local stack depth.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LayerCountExpectation {
    pub position: [f64; 2],
    pub count: usize,
}

/// A required visible crease sense at a book-diagram position.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VisibleCreaseExpectation {
    pub position: [f64; 2],
    pub kind: FoldSense,
}

/// Data-driven expectations for one instruction-book step.
///
/// Optional global fields make it possible to encode a partially known book
/// state.  Position-specific checks are represented by the three probe lists.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StepExpectation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Vec<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounding_box: Option<BoundingBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_area_ratio: Option<f64>,
    #[serde(default)]
    pub landmarks: Vec<LandmarkExpectation>,
    #[serde(default)]
    pub layer_counts: Vec<LayerCountExpectation>,
    #[serde(default)]
    pub visible_creases: Vec<VisibleCreaseExpectation>,
    #[serde(default)]
    pub tolerance: StepOracleTolerance,
}

impl StepExpectation {
    /// Capture all global features as an expectation.  Local layer probes are
    /// intentionally supplied by the caller because boundary points can count
    /// both faces adjacent to a crease.
    #[must_use]
    pub fn from_state(
        cp: &CreasePattern,
        faces: &[Face],
        state: &FlatState,
        layer_probe_points: &[[f64; 2]],
    ) -> Self {
        let features = extract_step_features(cp, faces, state);
        Self {
            outline: Some(features.outline.clone()),
            bounding_box: Some(features.bounding_box),
            aspect_ratio: Some(features.aspect_ratio),
            outline_area_ratio: Some(features.outline_area_ratio),
            landmarks: features
                .landmarks
                .iter()
                .map(|feature| LandmarkExpectation {
                    position: feature.position,
                    kind: feature.kind,
                })
                .collect(),
            layer_counts: layer_probe_points
                .iter()
                .copied()
                .map(|position| LayerCountExpectation {
                    position,
                    count: layer_count_at(cp, faces, state, position),
                })
                .collect(),
            visible_creases: features
                .visible_creases
                .iter()
                .map(|feature| VisibleCreaseExpectation {
                    position: midpoint(feature.segment),
                    kind: feature.kind,
                })
                .collect(),
            tolerance: StepOracleTolerance::default(),
        }
    }
}

/// The measured value of a requested layer-count probe.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LayerCountSample {
    pub position: [f64; 2],
    pub count: usize,
}

/// A structured mismatch suitable both for diagnostics and candidate scoring.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "feature", rename_all = "snake_case")]
pub enum StepDifference {
    OutlineVertexCount {
        expected: usize,
        actual: usize,
    },
    OutlineVertexPosition {
        index: usize,
        expected: [f64; 2],
        actual: [f64; 2],
        distance: f64,
    },
    BoundingBox {
        expected: BoundingBox,
        actual: BoundingBox,
        max_delta: f64,
    },
    AspectRatio {
        expected: f64,
        actual: f64,
        delta: f64,
    },
    OutlineAreaRatio {
        expected: f64,
        actual: f64,
        delta: f64,
    },
    LandmarkMissing {
        kind: LandmarkKind,
        expected_position: [f64; 2],
        nearest_position: Option<[f64; 2]>,
        distance: Option<f64>,
    },
    LayerCount {
        position: [f64; 2],
        expected: usize,
        actual: usize,
    },
    VisibleCreaseMissing {
        position: [f64; 2],
        expected: FoldSense,
        nearest_position: Option<[f64; 2]>,
        distance: Option<f64>,
    },
    VisibleCreaseSense {
        position: [f64; 2],
        expected: FoldSense,
        actual: FoldSense,
    },
}

impl fmt::Display for StepDifference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutlineVertexCount { expected, actual } => write!(
                formatter,
                "輪郭の頂点数が期待{expected}個に対し実際{actual}個"
            ),
            Self::OutlineVertexPosition {
                index,
                expected,
                actual,
                distance,
            } => write!(
                formatter,
                "輪郭頂点{index}が期待({:.6},{:.6})に対し実際({:.6},{:.6})、距離{distance:.6}",
                expected[0], expected[1], actual[0], actual[1]
            ),
            Self::BoundingBox {
                expected,
                actual,
                max_delta,
            } => write!(
                formatter,
                "外接矩形が期待{:?}に対し実際{:?}、最大差{max_delta:.6}",
                expected, actual
            ),
            Self::AspectRatio {
                expected,
                actual,
                delta,
            } => write!(
                formatter,
                "縦横比が期待{expected:.6}に対し実際{actual:.6}、差{delta:.6}"
            ),
            Self::OutlineAreaRatio {
                expected,
                actual,
                delta,
            } => write!(
                formatter,
                "紙全体に対する輪郭面積比が期待{expected:.6}に対し実際{actual:.6}、差{delta:.6}"
            ),
            Self::LandmarkMissing {
                kind,
                expected_position,
                nearest_position,
                distance,
            } => write!(
                formatter,
                "位置({:.6},{:.6})のランドマーク({kind:?})がなく、最近点は{nearest_position:?}、距離{distance:?}",
                expected_position[0], expected_position[1]
            ),
            Self::LayerCount {
                position,
                expected,
                actual,
            } => write!(
                formatter,
                "位置({:.6},{:.6})の層数が期待{expected}に対し実際{actual}",
                position[0], position[1]
            ),
            Self::VisibleCreaseMissing {
                position,
                expected,
                nearest_position,
                distance,
            } => write!(
                formatter,
                "位置({:.6},{:.6})に期待した可視折り目({expected:?})がなく、最近点は{nearest_position:?}、距離{distance:?}",
                position[0], position[1]
            ),
            Self::VisibleCreaseSense {
                position,
                expected,
                actual,
            } => write!(
                formatter,
                "位置({:.6},{:.6})の可視折り目が期待{expected:?}に対し実際{actual:?}",
                position[0], position[1]
            ),
        }
    }
}

/// Full comparison output.  `differences` is empty exactly when the state
/// satisfies every supplied expectation.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StepOracleReport {
    pub actual: StepFeatures,
    pub layer_samples: Vec<LayerCountSample>,
    pub differences: Vec<StepDifference>,
}

impl StepOracleReport {
    #[must_use]
    pub fn is_match(&self) -> bool {
        self.differences.is_empty()
    }

    #[must_use]
    pub fn explanations(&self) -> Vec<String> {
        self.differences.iter().map(ToString::to_string).collect()
    }
}

/// Extract outline, size, landmark and visible-crease features.
#[must_use]
pub fn extract_step_features(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
) -> StepFeatures {
    let positions = vertex_positions(cp);
    let folded_polygons = folded_face_polygons(faces, state, &positions);
    let folded_points = folded_polygons
        .iter()
        .flat_map(|(_, polygon)| polygon.iter().copied())
        .collect::<Vec<_>>();
    let outline = convex_hull(folded_points.clone());
    let bounding_box = bounds(&folded_points);
    let aspect_ratio = if bounding_box.height().abs() <= GEOMETRY_EPS {
        0.0
    } else {
        bounding_box.width() / bounding_box.height()
    };
    let unfolded_area = faces
        .iter()
        .map(|face| {
            let polygon = face
                .vertices
                .iter()
                .filter_map(|vertex| positions.get(vertex).copied())
                .collect::<Vec<_>>();
            polygon_area(&polygon).abs()
        })
        .sum::<f64>();
    let outline_area_ratio = if unfolded_area <= GEOMETRY_EPS {
        0.0
    } else {
        polygon_area(&outline).abs() / unfolded_area
    };

    StepFeatures {
        outline: outline.iter().map(|point| point.to_array()).collect(),
        bounding_box,
        aspect_ratio,
        outline_area_ratio,
        landmarks: extract_landmarks(cp, faces, state, &positions),
        visible_creases: extract_visible_creases(cp, faces, state, &positions, &folded_polygons),
    }
}

/// Count sheets covering an interior point in the folded plane.
///
/// Points exactly on an edge deliberately count every incident face; book-step
/// expectations should therefore use a small interior offset.
#[must_use]
pub fn layer_count_at(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    position: [f64; 2],
) -> usize {
    layers_at_point(cp, faces, state, position).len()
}

/// Compare one flat state against externally supplied expectation data.
#[must_use]
pub fn evaluate_step(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    expectation: &StepExpectation,
) -> StepOracleReport {
    let actual = extract_step_features(cp, faces, state);
    let mut differences = Vec::new();

    compare_outline(&actual, expectation, &mut differences);
    compare_size(&actual, expectation, &mut differences);
    compare_landmarks(&actual, expectation, &mut differences);

    let layer_samples = expectation
        .layer_counts
        .iter()
        .map(|expected| {
            let count = layer_count_at(cp, faces, state, expected.position);
            if count != expected.count {
                differences.push(StepDifference::LayerCount {
                    position: expected.position,
                    expected: expected.count,
                    actual: count,
                });
            }
            LayerCountSample {
                position: expected.position,
                count,
            }
        })
        .collect();

    compare_visible_creases(&actual, expectation, &mut differences);

    StepOracleReport {
        actual,
        layer_samples,
        differences,
    }
}

fn compare_outline(
    actual: &StepFeatures,
    expectation: &StepExpectation,
    differences: &mut Vec<StepDifference>,
) {
    let Some(expected_points) = &expectation.outline else {
        return;
    };
    let expected = convex_hull(expected_points.iter().copied().map(DVec2::from).collect());
    if expected.len() != actual.outline.len() {
        differences.push(StepDifference::OutlineVertexCount {
            expected: expected.len(),
            actual: actual.outline.len(),
        });
        return;
    }
    for (index, (&expected, &actual)) in expected.iter().zip(&actual.outline).enumerate() {
        let actual = DVec2::from(actual);
        let distance = expected.distance(actual);
        if distance > expectation.tolerance.outline_position {
            differences.push(StepDifference::OutlineVertexPosition {
                index,
                expected: expected.to_array(),
                actual: actual.to_array(),
                distance,
            });
        }
    }
}

fn compare_size(
    actual: &StepFeatures,
    expectation: &StepExpectation,
    differences: &mut Vec<StepDifference>,
) {
    if let Some(expected) = expectation.bounding_box {
        let delta = expected
            .min
            .into_iter()
            .chain(expected.max)
            .zip(
                actual
                    .bounding_box
                    .min
                    .into_iter()
                    .chain(actual.bounding_box.max),
            )
            .map(|(expected, actual)| (expected - actual).abs())
            .fold(0.0, f64::max);
        if delta > expectation.tolerance.bounding_box {
            differences.push(StepDifference::BoundingBox {
                expected,
                actual: actual.bounding_box,
                max_delta: delta,
            });
        }
    }
    if let Some(expected) = expectation.aspect_ratio {
        let delta = (expected - actual.aspect_ratio).abs();
        if delta > expectation.tolerance.aspect_ratio {
            differences.push(StepDifference::AspectRatio {
                expected,
                actual: actual.aspect_ratio,
                delta,
            });
        }
    }
    if let Some(expected) = expectation.outline_area_ratio {
        let delta = (expected - actual.outline_area_ratio).abs();
        if delta > expectation.tolerance.area_ratio {
            differences.push(StepDifference::OutlineAreaRatio {
                expected,
                actual: actual.outline_area_ratio,
                delta,
            });
        }
    }
}

fn compare_landmarks(
    actual: &StepFeatures,
    expectation: &StepExpectation,
    differences: &mut Vec<StepDifference>,
) {
    for expected in &expectation.landmarks {
        let nearest = actual
            .landmarks
            .iter()
            .filter(|feature| landmark_kinds_match(expected.kind, feature.kind))
            .map(|feature| {
                (
                    feature,
                    DVec2::from(feature.position).distance(DVec2::from(expected.position)),
                )
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right));
        if nearest.is_none_or(|(_, distance)| distance > expectation.tolerance.landmark_position) {
            differences.push(StepDifference::LandmarkMissing {
                kind: expected.kind,
                expected_position: expected.position,
                nearest_position: nearest.map(|(feature, _)| feature.position),
                distance: nearest.map(|(_, distance)| distance),
            });
        }
    }
}

fn compare_visible_creases(
    actual: &StepFeatures,
    expectation: &StepExpectation,
    differences: &mut Vec<StepDifference>,
) {
    for expected in &expectation.visible_creases {
        let point = DVec2::from(expected.position);
        let nearest_same = actual
            .visible_creases
            .iter()
            .filter(|crease| crease.kind == expected.kind)
            .map(|crease| (crease, distance_to_segment(point, crease.segment)))
            .min_by(|(_, left), (_, right)| left.total_cmp(right));
        if nearest_same
            .is_some_and(|(_, distance)| distance <= expectation.tolerance.visible_crease_position)
        {
            continue;
        }
        let nearest = actual
            .visible_creases
            .iter()
            .map(|crease| (crease, distance_to_segment(point, crease.segment)))
            .min_by(|(_, left), (_, right)| left.total_cmp(right));
        match nearest {
            Some((crease, distance))
                if distance <= expectation.tolerance.visible_crease_position =>
            {
                differences.push(StepDifference::VisibleCreaseSense {
                    position: expected.position,
                    expected: expected.kind,
                    actual: crease.kind,
                });
            }
            _ => differences.push(StepDifference::VisibleCreaseMissing {
                position: expected.position,
                expected: expected.kind,
                nearest_position: nearest.map(|(crease, _)| midpoint(crease.segment)),
                distance: nearest.map(|(_, distance)| distance),
            }),
        }
    }
}

fn landmark_kinds_match(expected: LandmarkKind, actual: LandmarkKind) -> bool {
    expected == LandmarkKind::Any || expected == actual
}

fn vertex_positions(cp: &CreasePattern) -> HashMap<VertexId, DVec2> {
    cp.vertices
        .iter()
        .map(|vertex| (vertex.id, DVec2::from(vertex.pos)))
        .collect()
}

fn folded_face_polygons(
    faces: &[Face],
    state: &FlatState,
    positions: &HashMap<VertexId, DVec2>,
) -> Vec<(FaceId, Vec<DVec2>)> {
    faces
        .iter()
        .filter_map(|face| {
            let placement = state.placements.get(&face.id)?;
            let polygon = face
                .vertices
                .iter()
                .filter_map(|vertex| positions.get(vertex).copied())
                .map(|point| placement.apply(point))
                .collect::<Vec<_>>();
            Some((face.id, polygon))
        })
        .collect()
}

fn extract_landmarks(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    positions: &HashMap<VertexId, DVec2>,
) -> Vec<LandmarkFeature> {
    let mut border_directions: HashMap<VertexId, Vec<DVec2>> = HashMap::new();
    let mut crease_degree: HashMap<VertexId, usize> = HashMap::new();
    for edge in &cp.edges {
        let (Some(&a), Some(&b)) = (positions.get(&edge.v0), positions.get(&edge.v1)) else {
            continue;
        };
        match edge.kind {
            EdgeKind::Border => {
                border_directions.entry(edge.v0).or_default().push(b - a);
                border_directions.entry(edge.v1).or_default().push(a - b);
            }
            EdgeKind::Mountain | EdgeKind::Valley => {
                *crease_degree.entry(edge.v0).or_default() += 1;
                *crease_degree.entry(edge.v1).or_default() += 1;
            }
            EdgeKind::Aux => {}
        }
    }

    let paper_corners = border_directions
        .iter()
        .filter_map(|(&vertex, directions)| {
            directions
                .iter()
                .enumerate()
                .any(|(index, left)| {
                    directions[index + 1..].iter().any(|right| {
                        left.perp_dot(*right).abs() > GEOMETRY_EPS * left.length() * right.length()
                    })
                })
                .then_some(vertex)
        })
        .collect::<HashSet<_>>();
    let crease_endpoints = crease_degree
        .into_iter()
        .filter_map(|(vertex, degree)| (degree != 2).then_some(vertex))
        .collect::<HashSet<_>>();

    let mut features = Vec::new();
    for face in faces {
        let Some(placement) = state.placements.get(&face.id) else {
            continue;
        };
        for vertex in &face.vertices {
            let Some(&local) = positions.get(vertex) else {
                continue;
            };
            let position = placement.apply(local).to_array();
            if paper_corners.contains(vertex) {
                push_unique_landmark(
                    &mut features,
                    LandmarkFeature {
                        position,
                        kind: LandmarkKind::PaperCorner,
                    },
                );
            }
            if crease_endpoints.contains(vertex) {
                push_unique_landmark(
                    &mut features,
                    LandmarkFeature {
                        position,
                        kind: LandmarkKind::CreaseEndpoint,
                    },
                );
            }
        }
    }
    features.sort_by(|left, right| {
        landmark_sort_key(left.kind)
            .cmp(&landmark_sort_key(right.kind))
            .then(left.position[0].total_cmp(&right.position[0]))
            .then(left.position[1].total_cmp(&right.position[1]))
    });
    features
}

fn push_unique_landmark(features: &mut Vec<LandmarkFeature>, candidate: LandmarkFeature) {
    if features.iter().any(|feature| {
        feature.kind == candidate.kind
            && DVec2::from(feature.position).distance(DVec2::from(candidate.position))
                <= GEOMETRY_EPS
    }) {
        return;
    }
    features.push(candidate);
}

fn landmark_sort_key(kind: LandmarkKind) -> u8 {
    match kind {
        LandmarkKind::PaperCorner => 0,
        LandmarkKind::CreaseEndpoint => 1,
        LandmarkKind::Any => 2,
    }
}

fn extract_visible_creases(
    cp: &CreasePattern,
    faces: &[Face],
    state: &FlatState,
    positions: &HashMap<VertexId, DVec2>,
    folded_polygons: &[(FaceId, Vec<DVec2>)],
) -> Vec<VisibleCrease> {
    let boundary_segments = folded_polygons
        .iter()
        .flat_map(|(_, polygon)| polygon_segments(polygon))
        .collect::<Vec<_>>();
    let mut visible = Vec::new();

    for edge in cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
    {
        let mut owners = faces
            .iter()
            .filter(|face| face.edges.contains(&edge.id))
            .map(|face| face.id)
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();
        let Some(reference_face) = owners.iter().find_map(|face_id| {
            state
                .placements
                .get(face_id)
                .map(|placement| (*face_id, placement))
        }) else {
            continue;
        };
        let (Some(&local_a), Some(&local_b)) = (positions.get(&edge.v0), positions.get(&edge.v1))
        else {
            continue;
        };
        let a = reference_face.1.apply(local_a);
        let b = reference_face.1.apply(local_b);
        if a.distance(b) <= GEOMETRY_EPS {
            continue;
        }
        let parameters = split_parameters(a, b, &boundary_segments);
        let mut edge_parts: Vec<VisibleCrease> = Vec::new();
        for window in parameters.windows(2) {
            let (start, end) = (window[0], window[1]);
            if end - start <= GEOMETRY_EPS {
                continue;
            }
            let sample = a.lerp(b, (start + end) * 0.5);
            let Some(top_face) = layers_at_point(cp, faces, state, sample.to_array())
                .last()
                .copied()
            else {
                continue;
            };
            if !owners.contains(&top_face) {
                continue;
            }
            let Some(top_placement) = state.placements.get(&top_face) else {
                continue;
            };
            let kind = fold_sense(edge.kind, top_placement.mirrored);
            let segment = [a.lerp(b, start).to_array(), a.lerp(b, end).to_array()];
            if let Some(last) = edge_parts.last_mut()
                && last.kind == kind
                && DVec2::from(last.segment[1]).distance(DVec2::from(segment[0])) <= GEOMETRY_EPS
            {
                last.segment[1] = segment[1];
            } else {
                edge_parts.push(VisibleCrease { segment, kind });
            }
        }
        for part in edge_parts {
            if !visible
                .iter()
                .any(|known| same_visible_crease(*known, part))
            {
                visible.push(part);
            }
        }
    }
    visible.sort_by(|left, right| {
        fold_sense_sort_key(left.kind)
            .cmp(&fold_sense_sort_key(right.kind))
            .then(left.segment[0][0].total_cmp(&right.segment[0][0]))
            .then(left.segment[0][1].total_cmp(&right.segment[0][1]))
            .then(left.segment[1][0].total_cmp(&right.segment[1][0]))
            .then(left.segment[1][1].total_cmp(&right.segment[1][1]))
    });
    visible
}

fn fold_sense(kind: EdgeKind, mirrored: bool) -> FoldSense {
    match (kind, mirrored) {
        (EdgeKind::Mountain, false) | (EdgeKind::Valley, true) => FoldSense::Mountain,
        (EdgeKind::Valley, false) | (EdgeKind::Mountain, true) => FoldSense::Valley,
        (EdgeKind::Border | EdgeKind::Aux, _) => {
            unreachable!("only mountain/valley edges are passed to fold_sense")
        }
    }
}

fn fold_sense_sort_key(kind: FoldSense) -> u8 {
    match kind {
        FoldSense::Mountain => 0,
        FoldSense::Valley => 1,
    }
}

fn same_visible_crease(left: VisibleCrease, right: VisibleCrease) -> bool {
    left.kind == right.kind
        && ((points_near(left.segment[0], right.segment[0])
            && points_near(left.segment[1], right.segment[1]))
            || (points_near(left.segment[0], right.segment[1])
                && points_near(left.segment[1], right.segment[0])))
}

fn points_near(left: [f64; 2], right: [f64; 2]) -> bool {
    DVec2::from(left).distance(DVec2::from(right)) <= GEOMETRY_EPS
}

fn split_parameters(a: DVec2, b: DVec2, boundaries: &[[DVec2; 2]]) -> Vec<f64> {
    let mut parameters = vec![0.0, 1.0];
    for &[c, d] in boundaries {
        add_intersection_parameters(&mut parameters, a, b, c, d);
    }
    parameters.sort_by(f64::total_cmp);
    parameters.dedup_by(|left, right| (*left - *right).abs() <= GEOMETRY_EPS);
    parameters
}

fn add_intersection_parameters(parameters: &mut Vec<f64>, a: DVec2, b: DVec2, c: DVec2, d: DVec2) {
    let line = b - a;
    let boundary = d - c;
    let denominator = line.perp_dot(boundary);
    if denominator.abs() > GEOMETRY_EPS {
        let t = (c - a).perp_dot(boundary) / denominator;
        let u = (c - a).perp_dot(line) / denominator;
        if (-GEOMETRY_EPS..=1.0 + GEOMETRY_EPS).contains(&t)
            && (-GEOMETRY_EPS..=1.0 + GEOMETRY_EPS).contains(&u)
        {
            parameters.push(t.clamp(0.0, 1.0));
        }
        return;
    }
    if (c - a).perp_dot(line).abs() > GEOMETRY_EPS * line.length() {
        return;
    }
    let length_squared = line.length_squared();
    if length_squared <= GEOMETRY_EPS * GEOMETRY_EPS {
        return;
    }
    for point in [c, d] {
        let t = (point - a).dot(line) / length_squared;
        if (-GEOMETRY_EPS..=1.0 + GEOMETRY_EPS).contains(&t) {
            parameters.push(t.clamp(0.0, 1.0));
        }
    }
}

fn polygon_segments(polygon: &[DVec2]) -> Vec<[DVec2; 2]> {
    if polygon.len() < 2 {
        return Vec::new();
    }
    (0..polygon.len())
        .map(|index| [polygon[index], polygon[(index + 1) % polygon.len()]])
        .collect()
}

fn convex_hull(mut points: Vec<DVec2>) -> Vec<DVec2> {
    points.sort_by(|left, right| left.x.total_cmp(&right.x).then(left.y.total_cmp(&right.y)));
    points.dedup_by(|left, right| left.distance(*right) <= GEOMETRY_EPS);
    if points.len() <= 2 {
        return points;
    }

    let mut lower: Vec<DVec2> = Vec::new();
    for &point in &points {
        while lower.len() >= 2 {
            let count = lower.len();
            let incoming = lower[count - 1] - lower[count - 2];
            let outgoing = point - lower[count - 1];
            let turn = incoming.perp_dot(outgoing);
            if turn > GEOMETRY_EPS * incoming.length() * outgoing.length() {
                break;
            }
            lower.pop();
        }
        lower.push(point);
    }
    let mut upper: Vec<DVec2> = Vec::new();
    for &point in points.iter().rev() {
        while upper.len() >= 2 {
            let count = upper.len();
            let incoming = upper[count - 1] - upper[count - 2];
            let outgoing = point - upper[count - 1];
            let turn = incoming.perp_dot(outgoing);
            if turn > GEOMETRY_EPS * incoming.length() * outgoing.length() {
                break;
            }
            upper.pop();
        }
        upper.push(point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn bounds(points: &[DVec2]) -> BoundingBox {
    if points.is_empty() {
        return BoundingBox::default();
    }
    let mut min = DVec2::splat(f64::INFINITY);
    let mut max = DVec2::splat(f64::NEG_INFINITY);
    for &point in points {
        min = min.min(point);
        max = max.max(point);
    }
    BoundingBox {
        min: min.to_array(),
        max: max.to_array(),
    }
}

fn polygon_area(polygon: &[DVec2]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }
    0.5 * (0..polygon.len())
        .map(|index| polygon[index].perp_dot(polygon[(index + 1) % polygon.len()]))
        .sum::<f64>()
}

fn distance_to_segment(point: DVec2, segment: [[f64; 2]; 2]) -> f64 {
    let a = DVec2::from(segment[0]);
    let b = DVec2::from(segment[1]);
    let direction = b - a;
    let length_squared = direction.length_squared();
    if length_squared <= GEOMETRY_EPS * GEOMETRY_EPS {
        return point.distance(a);
    }
    let parameter = ((point - a).dot(direction) / length_squared).clamp(0.0, 1.0);
    point.distance(a + parameter * direction)
}

fn midpoint(segment: [[f64; 2]; 2]) -> [f64; 2] {
    [
        (segment[0][0] + segment[1][0]) * 0.5,
        (segment[0][1] + segment[1][1]) * 0.5,
    ]
}
