use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use ori3_cp::extract_faces;
use ori3_layers::{replay, representative_point};
use ori3_model::{
    CreasePattern, DisplaySettings, Document, DriverLine, Edge, EdgeKind, FoldStep, Paper,
    SCHEMA_VERSION, TechniqueKind, Vertex,
};
use ori3_rigid::{layer_order_conflicts, max_seam_gap, self_intersection_pairs};
use serde_json::{Value, json};

use super::types::{
    FOLD_1_2_PROFILE_NAME, FoldAssignment, FoldFile, FoldFrame, FoldIssue, FoldIssueCode,
    FoldIssueSeverity,
};
use super::validation::validate_fold_1_2;

/// Geometry and angle comparisons use the same normalized boundary as the
/// approved roundtrip contract. Values at this scale are numerical noise, while
/// a visibly distinct vertex or target angle remains many orders of magnitude away.
const CONVERSION_EPS: f64 = 1e-9;
/// The replay endpoint/seam boundary fixed by roadmap section 12.6.
const ENDPOINT_EPS: f64 = 1e-6;
type DirectedFaceOrder = BTreeSet<(usize, usize)>;
type TotalFaceOrder = (Vec<usize>, DirectedFaceOrder);

/// A successfully imported ORIGAMI3 document and every non-blocking loss warning.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldImport {
    pub document: Document,
    pub warnings: Vec<FoldIssue>,
}

/// A rejected conversion. Validation and conversion issues are returned together;
/// callers never need to discard warnings in order to show all blocking reasons.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldConversionError {
    pub warnings: Vec<FoldIssue>,
    pub errors: Vec<FoldIssue>,
}

impl fmt::Display for FoldConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{FOLD_1_2_PROFILE_NAME}へ変換できない項目が{}件あります",
            self.errors.len()
        )
    }
}

impl Error for FoldConversionError {}

/// Convert a validated FOLD 1.1/1.2 limited-profile value into an ORIGAMI3 document.
///
/// The validator is always the first gate. If it reports an error, no `Document`
/// is constructed and all warnings/errors are returned. F/U assignments become
/// [`EdgeKind::Aux`], while the validator's original assignment and JSON-path
/// warnings are carried into [`FoldImport::warnings`].
pub fn fold_to_document(file: &FoldFile) -> Result<FoldImport, FoldConversionError> {
    let validation = validate_fold_1_2(file);
    let mut warnings = validation.warnings;
    if !validation.errors.is_empty() {
        return Err(FoldConversionError {
            warnings,
            errors: validation.errors,
        });
    }

    let mut errors = Vec::new();
    let Some(normalization) = Normalization::from_frame(&file.root, &mut errors) else {
        sort_and_deduplicate(&mut errors);
        return Err(FoldConversionError { warnings, errors });
    };
    if let Some(warning) = normalization.warning(&file.root) {
        warnings.push(warning);
    }

    let Some(cp) = convert_crease_pattern(&file.root, normalization, &mut errors) else {
        sort_and_deduplicate(&mut errors);
        sort_and_deduplicate(&mut warnings);
        return Err(FoldConversionError { warnings, errors });
    };

    let paper = Paper {
        // FOLD has no physical-unit field. These positive virtual millimetre values
        // preserve the side ratio expected by Document's normalized coordinate system.
        width_mm: normalization.width,
        height_mm: normalization.height,
    };
    let mut document = Document {
        schema_version: SCHEMA_VERSION,
        paper,
        cp,
        sequence: Vec::new(),
        display: DisplaySettings::default(),
    };

    let effective = effective_frames(file);
    let root_is_endpoint = effective
        .first()
        .is_some_and(|effective| frame_has_endpoint_semantics(&effective.frame));
    let first_endpoint = if root_is_endpoint { 0 } else { 1 };
    let mut endpoint_sources = Vec::new();
    for (frame_index, effective_frame) in effective.iter().enumerate().skip(first_endpoint) {
        let path = effective_frame_path(frame_index);
        if frame_index > 0
            && let Some(root) = effective.first()
        {
            validate_endpoint_coordinates(
                &effective_frame.frame,
                &root.frame,
                normalization,
                &effective_frame.sources.vertices_coords,
                &mut errors,
            );
        }
        let step_id = match u32::try_from(document.sequence.len()) {
            Ok(id) => id,
            Err(_) => {
                errors.push(issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    &path,
                    "ORIGAMI3のStepIdで表せる手順数を超えています",
                    Some(json!(document.sequence.len())),
                ));
                continue;
            }
        };
        if let Some(step) = convert_step(
            &effective_frame.frame,
            &effective_frame.sources,
            &path,
            step_id,
            &document.cp,
            &mut errors,
        ) {
            document.sequence.push(step);
            endpoint_sources.push((path, effective_frame.sources.face_orders.clone()));
        }
    }

    if errors.is_empty() {
        validate_replay_endpoints(&document, &endpoint_sources, &mut errors);
    }

    sort_and_deduplicate(&mut warnings);
    sort_and_deduplicate(&mut errors);
    if errors.is_empty() {
        Ok(FoldImport { document, warnings })
    } else {
        Err(FoldConversionError { warnings, errors })
    }
}

fn validate_replay_endpoints(
    document: &Document,
    endpoint_sources: &[(String, String)],
    errors: &mut Vec<FoldIssue>,
) {
    let faces = extract_faces(&document.cp);
    for (step_index, (frame_path, face_orders_path)) in endpoint_sources.iter().enumerate() {
        let replayed = catch_unwind(AssertUnwindSafe(|| replay(document, step_index + 1, 1.0)));
        let Ok(replayed) = replayed else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                frame_path,
                "step endpointの再生に失敗したため限定profileとして取込めません",
                None,
            ));
            continue;
        };
        if !replayed.skipped.is_empty() || !replayed.converged || replayed.best_effort {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                frame_path,
                "step endpointを指定どおりの収束解として再生できません",
                Some(json!({
                    "skipped": replayed.skipped,
                    "converged": replayed.converged,
                    "best_effort": replayed.best_effort,
                })),
            ));
        }
        if replayed
            .frame
            .faces
            .iter()
            .flat_map(|face| &face.polygon)
            .flatten()
            .any(|coordinate| !coordinate.is_finite())
        {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                frame_path,
                "step endpointに非有限の3D座標があります",
                None,
            ));
        }
        let seam = max_seam_gap(&document.cp, &faces, &replayed.frame);
        if !seam.is_finite() || seam > ENDPOINT_EPS {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                frame_path,
                format!("step endpointのseam {seam:e}が許容差{ENDPOINT_EPS:e}を超えます"),
                Some(numeric_value(seam)),
            ));
        }
        let intersections = self_intersection_pairs(&replayed.frame);
        if !intersections.is_empty() {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                frame_path,
                format!(
                    "step endpointに{}件のpenetrationがあり、対応対象にできません",
                    intersections.len()
                ),
                Some(json!(intersections)),
            ));
        }
        if document.sequence[step_index].layer_order.is_some()
            && layer_order_conflicts(&document.cp, &faces, &replayed.frame)
        {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnrepresentableFaceOrders,
                face_orders_path,
                "faceOrdersの上下制約が平坦endpointの山谷と矛盾します",
                None,
            ));
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Normalization {
    origin: [f64; 2],
    x_unit: [f64; 2],
    y_unit: [f64; 2],
    scale: f64,
    width: f64,
    height: f64,
}

impl Normalization {
    fn from_frame(frame: &FoldFrame, errors: &mut Vec<FoldIssue>) -> Option<Self> {
        let (Some(vertices), Some(edges), Some(assignments)) = (
            frame.vertices_coords.as_ref(),
            frame.edges_vertices.as_ref(),
            frame.edges_assignment.as_ref(),
        ) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::MissingRequiredField,
                "$.vertices_coords",
                "座標正規化にはroot frameの頂点・edge・assignmentが必要です",
                None,
            ));
            return None;
        };

        let Some(boundary) = boundary_cycle(edges, assignments) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                "$.edges_assignment",
                "正方形または長方形の単一B境界cycleを座標正規化へ使えません",
                frame.edges_vertices.as_ref().map(|value| json!(value)),
            ));
            return None;
        };
        let Some(corners) = rectangle_corner_indices(&boundary, vertices) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                "$.vertices_coords",
                "B境界から正方形または長方形の4 cornerを決定できません",
                frame.vertices_coords.as_ref().map(|value| json!(value)),
            ));
            return None;
        };
        let (Some(origin), Some(corner_x), Some(corner_y)) = (
            point(vertices, corners[0]),
            point(vertices, corners[1]),
            point(vertices, corners[3]),
        ) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                "$.vertices_coords",
                "B境界cornerの2D座標を解決できません",
                frame.vertices_coords.as_ref().map(|value| json!(value)),
            ));
            return None;
        };
        let x_axis = subtract(corner_x, origin);
        let y_axis = subtract(corner_y, origin);
        let x_length = length(x_axis);
        let y_length = length(y_axis);
        let scale = x_length.max(y_length);
        if !scale.is_finite() || scale <= 0.0 || x_length <= 0.0 || y_length <= 0.0 {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                "$.vertices_coords",
                "正方形または長方形の正の2辺を座標から決定できません",
                frame.vertices_coords.as_ref().map(|value| json!(value)),
            ));
            return None;
        }
        Some(Self {
            origin,
            x_unit: [x_axis[0] / x_length, x_axis[1] / x_length],
            y_unit: [y_axis[0] / y_length, y_axis[1] / y_length],
            scale,
            width: x_length / scale,
            height: y_length / scale,
        })
    }

    fn transform(self, point: [f64; 2]) -> [f64; 2] {
        let offset = subtract(point, self.origin);
        [
            dot(offset, self.x_unit) / self.scale,
            dot(offset, self.y_unit) / self.scale,
        ]
    }

    fn is_identity(self) -> bool {
        approximately(self.origin[0], 0.0)
            && approximately(self.origin[1], 0.0)
            && approximately(self.x_unit[0], 1.0)
            && approximately(self.x_unit[1], 0.0)
            && approximately(self.y_unit[0], 0.0)
            && approximately(self.y_unit[1], 1.0)
            && approximately(self.scale, 1.0)
    }

    fn warning(self, frame: &FoldFrame) -> Option<FoldIssue> {
        (!self.is_identity()).then(|| {
            issue(
                FoldIssueSeverity::Warning,
                FoldIssueCode::UnsupportedGeometry,
                "$.vertices_coords",
                "FOLD座標の平行移動・回転・scaleはDocumentへ保存できないため、長辺1の2D座標へsimilarity正規化しました",
                Some(json!({
                    "origin": self.origin,
                    "x_unit": self.x_unit,
                    "y_unit": self.y_unit,
                    "scale": self.scale,
                    "vertices_coords": frame.vertices_coords,
                })),
            )
        })
    }
}

fn convert_crease_pattern(
    frame: &FoldFrame,
    normalization: Normalization,
    errors: &mut Vec<FoldIssue>,
) -> Option<CreasePattern> {
    let (vertices, edges, assignments) = (
        frame.vertices_coords.as_ref()?,
        frame.edges_vertices.as_ref()?,
        frame.edges_assignment.as_ref()?,
    );

    let mut model_vertices = Vec::with_capacity(vertices.len());
    for (index, vertex) in vertices.iter().enumerate() {
        let Ok(id) = u32::try_from(index) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                format!("$.vertices_coords[{index}]"),
                "頂点indexをORIGAMI3のVertexIdで表せません",
                Some(json!(index)),
            ));
            continue;
        };
        let Some(position) = vector2(vertex) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidValue,
                format!("$.vertices_coords[{index}]"),
                "頂点座標を有限の2D位置へ変換できません",
                Some(json!(vertex)),
            ));
            continue;
        };
        model_vertices.push(Vertex {
            id,
            pos: normalization.transform(position),
        });
    }

    let mut model_edges = Vec::with_capacity(edges.len());
    for (index, (edge, assignment)) in edges.iter().zip(assignments).enumerate() {
        let (Some(&first), Some(&second)) = (edge.first(), edge.get(1)) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                format!("$.edges_vertices[{index}]"),
                "edgeの2頂点をORIGAMI3へ変換できません",
                Some(json!(edge)),
            ));
            continue;
        };
        let (Ok(id), Ok(v0), Ok(v1)) = (
            u32::try_from(index),
            u32::try_from(first),
            u32::try_from(second),
        ) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                format!("$.edges_vertices[{index}]"),
                "edgeまたは頂点indexをORIGAMI3のu32 IDで表せません",
                Some(json!(edge)),
            ));
            continue;
        };
        let kind = match assignment {
            FoldAssignment::Border => EdgeKind::Border,
            FoldAssignment::Mountain => EdgeKind::Mountain,
            FoldAssignment::Valley => EdgeKind::Valley,
            FoldAssignment::Flat | FoldAssignment::Unassigned => EdgeKind::Aux,
            FoldAssignment::Other(_) => continue,
        };
        model_edges.push(Edge { id, v0, v1, kind });
    }

    let (Ok(next_vertex_id), Ok(next_edge_id)) =
        (u32::try_from(vertices.len()), u32::try_from(edges.len()))
    else {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::InvalidTopology,
            "$",
            "頂点またはedge件数をORIGAMI3の次IDで表せません",
            Some(json!({"vertices": vertices.len(), "edges": edges.len()})),
        ));
        return None;
    };
    if !errors.is_empty() {
        return None;
    }
    Some(CreasePattern {
        vertices: model_vertices,
        edges: model_edges,
        next_vertex_id,
        next_edge_id,
    })
}

fn convert_step(
    frame: &FoldFrame,
    sources: &FrameSources,
    frame_path: &str,
    id: u32,
    cp: &CreasePattern,
    errors: &mut Vec<FoldIssue>,
) -> Option<FoldStep> {
    let error_count_before = errors.len();
    let mut drivers = Vec::new();
    let (Some(edges), Some(assignments)) = (
        frame.edges_vertices.as_ref(),
        frame.edges_assignment.as_ref(),
    ) else {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::MissingRequiredField,
            frame_path,
            "step frameのedge topologyとassignmentを解決できません",
            None,
        ));
        return None;
    };

    let flat_endpoint = assignments
        .iter()
        .enumerate()
        .all(|(edge_index, assignment)| {
            if !matches!(
                assignment,
                FoldAssignment::Mountain | FoldAssignment::Valley
            ) {
                return true;
            }
            frame
                .edges_fold_angle
                .as_ref()
                .and_then(|angles| angles.get(edge_index))
                .copied()
                .flatten()
                .is_some_and(|angle| {
                    angle.abs() <= CONVERSION_EPS || (angle.abs() - 180.0).abs() <= CONVERSION_EPS
                })
        });
    if frame
        .face_orders
        .as_ref()
        .is_some_and(|orders| !orders.is_empty())
        && !flat_endpoint
    {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::UnrepresentableFaceOrders,
            &sources.face_orders,
            "layer_orderは平坦終点だけに保存できるため、0度または±180度でないendpointのfaceOrdersは取込めません",
            frame.face_orders.as_ref().map(|orders| json!(orders)),
        ));
    }

    for (edge_index, (edge, assignment)) in edges.iter().zip(assignments).enumerate() {
        if !matches!(
            assignment,
            FoldAssignment::Mountain | FoldAssignment::Valley
        ) {
            continue;
        }
        let angle = frame
            .edges_fold_angle
            .as_ref()
            .and_then(|angles| angles.get(edge_index))
            .copied()
            .flatten();
        let angle_path = format!("{}[{edge_index}]", sources.edges_fold_angle);
        let Some(fold_angle) = angle else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::MissingRequiredField,
                &angle_path,
                "step endpointのM/V edgeには有限のedges_foldAngleが必要で、未指定を0度とは推測しません",
                frame
                    .edges_fold_angle
                    .as_ref()
                    .and_then(|angles| angles.get(edge_index))
                    .map(|_| Value::Null),
            ));
            continue;
        };
        let (Some(&first), Some(&second)) = (edge.first(), edge.get(1)) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                format!("{}[{edge_index}]", sources.edges_vertices),
                "Driver線分の2頂点を解決できません",
                Some(json!(edge)),
            ));
            continue;
        };
        let (Some(a), Some(b)) = (cp.vertices.get(first), cp.vertices.get(second)) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                format!("{}[{edge_index}]", sources.edges_vertices),
                "Driver線分が存在しない頂点を参照しています",
                Some(json!(edge)),
            ));
            continue;
        };
        let target = if approximately(fold_angle, 0.0) {
            0.0
        } else {
            -fold_angle
        };
        drivers.push(DriverLine {
            a: a.pos,
            b: b.pos,
            // FOLD: M<0/V>0. ORIGAMI3: Mountain>0/Valley<0.
            target_angle_deg: target,
        });
    }

    let layer_order = convert_face_orders(frame, sources, cp, errors);
    if errors.len() != error_count_before {
        return None;
    }
    Some(FoldStep {
        id,
        kind: if flat_endpoint {
            TechniqueKind::Simple
        } else {
            TechniqueKind::Pose
        },
        drivers,
        layer_order,
        alignment: None,
        finish_soft: None,
        note: String::new(),
    })
}

fn convert_face_orders(
    frame: &FoldFrame,
    sources: &FrameSources,
    cp: &CreasePattern,
    errors: &mut Vec<FoldIssue>,
) -> Option<Vec<[f64; 2]>> {
    let orders = frame.face_orders.as_ref()?;
    if orders.is_empty() {
        return None;
    }
    let faces = frame.faces_vertices.as_ref()?;
    let Some((order, directed)) = total_face_order(orders, faces.len()) else {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::UnrepresentableFaceOrders,
            &sources.face_orders,
            "faceOrdersを一意な下→上の順序へ変換できません",
            Some(json!(orders)),
        ));
        return None;
    };

    let expected = order
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .collect::<BTreeSet<_>>();
    if directed != expected {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::UnrepresentableFaceOrders,
            &sources.face_orders,
            "faceOrdersの正規化済み制約は隣接chainとexact一致せず、layer_orderから損失なく復元できません",
            Some(json!(orders)),
        ));
        return None;
    }

    let model_faces = extract_faces(cp);
    let mut mapped = Vec::with_capacity(faces.len());
    let mut used = BTreeSet::new();
    for (face_index, fold_face) in faces.iter().enumerate() {
        let found = model_faces.iter().find(|model_face| {
            !used.contains(&model_face.id) && cycles_match(fold_face, &model_face.vertices)
        });
        match found {
            Some(model_face) => {
                used.insert(model_face.id);
                mapped.push(model_face);
            }
            None => errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnrepresentableFaceOrders,
                format!("{}[{face_index}]", sources.faces_vertices),
                "このFOLD faceはORIGAMI3の面へ1対1対応せず、faceOrdersを意味どおり保持できません",
                Some(json!(fold_face)),
            )),
        }
    }
    if mapped.len() != faces.len() {
        return None;
    }
    if mapped.len() != model_faces.len() || used.len() != model_faces.len() {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::UnrepresentableFaceOrders,
            &sources.face_orders,
            "faces_verticesはORIGAMI3で抽出される全faceと1対1対応せず、layer_orderの暗黙補完なしには保持できません",
            Some(json!({
                "fold_faces": faces.len(),
                "model_faces": model_faces.len(),
            })),
        ));
        return None;
    }

    let mut points = Vec::with_capacity(order.len());
    for face_index in order {
        let Some(model_face) = mapped.get(face_index) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnrepresentableFaceOrders,
                &sources.face_orders,
                "faceOrdersが変換済みfaceの範囲外を参照しています",
                Some(json!(orders)),
            ));
            return None;
        };
        points.push(representative_point(cp, model_face));
    }
    Some(points)
}

fn total_face_order(orders: &[Vec<i64>], face_count: usize) -> Option<TotalFaceOrder> {
    let mut directed = BTreeSet::new();
    for triple in orders {
        if triple.len() != 3 || !matches!(triple[2], -1 | 1) {
            return None;
        }
        let (Ok(first), Ok(second)) = (usize::try_from(triple[0]), usize::try_from(triple[1]))
        else {
            return None;
        };
        if first >= face_count || second >= face_count || first == second {
            return None;
        }
        // FOLD +1 means `first` is above `second`; ORIGAMI3 stores bottom→top.
        directed.insert(if triple[2] == 1 {
            (second, first)
        } else {
            (first, second)
        });
    }

    let mut graph = vec![Vec::new(); face_count];
    let mut indegree = vec![0_usize; face_count];
    for &(from, to) in &directed {
        graph[from].push(to);
        indegree[to] += 1;
    }
    let mut removed = vec![false; face_count];
    let mut result = Vec::with_capacity(face_count);
    for _ in 0..face_count {
        let mut available = indegree
            .iter()
            .enumerate()
            .filter(|&(index, degree)| !removed[index] && *degree == 0)
            .map(|(index, _)| index);
        let selected = available.next()?;
        if available.next().is_some() {
            return None;
        }
        removed[selected] = true;
        result.push(selected);
        for &target in &graph[selected] {
            indegree[target] = indegree[target].checked_sub(1)?;
        }
    }
    Some((result, directed))
}

#[derive(Clone, Debug)]
struct EffectiveFrame {
    frame: FoldFrame,
    sources: FrameSources,
}

#[derive(Clone, Debug)]
struct FrameSources {
    vertices_coords: String,
    edges_vertices: String,
    edges_assignment: String,
    edges_fold_angle: String,
    faces_vertices: String,
    face_orders: String,
}

impl FrameSources {
    fn root() -> Self {
        Self {
            vertices_coords: "$.vertices_coords".to_string(),
            edges_vertices: "$.edges_vertices".to_string(),
            edges_assignment: "$.edges_assignment".to_string(),
            edges_fold_angle: "$.edges_foldAngle".to_string(),
            faces_vertices: "$.faces_vertices".to_string(),
            face_orders: "$.faceOrders".to_string(),
        }
    }

    fn child(index: usize, frame: &FoldFrame, parent: &Self) -> Self {
        let base = format!("$.file_frames[{index}]");
        let inherits = frame.frame_inherit == Some(true);
        Self {
            vertices_coords: inherited_source(
                inherits,
                frame.vertices_coords.is_some(),
                &parent.vertices_coords,
                format!("{base}.vertices_coords"),
            ),
            edges_vertices: inherited_source(
                inherits,
                frame.edges_vertices.is_some(),
                &parent.edges_vertices,
                format!("{base}.edges_vertices"),
            ),
            edges_assignment: inherited_source(
                inherits,
                frame.edges_assignment.is_some(),
                &parent.edges_assignment,
                format!("{base}.edges_assignment"),
            ),
            edges_fold_angle: inherited_source(
                inherits,
                frame.edges_fold_angle.is_some(),
                &parent.edges_fold_angle,
                format!("{base}.edges_foldAngle"),
            ),
            faces_vertices: inherited_source(
                inherits,
                frame.faces_vertices.is_some(),
                &parent.faces_vertices,
                format!("{base}.faces_vertices"),
            ),
            face_orders: inherited_source(
                inherits,
                frame.face_orders.is_some(),
                &parent.face_orders,
                format!("{base}.faceOrders"),
            ),
        }
    }
}

fn inherited_source(inherits: bool, declared: bool, parent: &str, child: String) -> String {
    if inherits && !declared {
        parent.to_string()
    } else {
        child
    }
}

fn effective_frames(file: &FoldFile) -> Vec<EffectiveFrame> {
    let mut effective = vec![EffectiveFrame {
        frame: file.root.clone(),
        sources: FrameSources::root(),
    }];
    for (index, frame) in file.file_frames.iter().enumerate() {
        let Some(parent) = effective.last() else {
            break;
        };
        let sources = FrameSources::child(index, frame, &parent.sources);
        let resolved = if frame.frame_inherit == Some(true) {
            inherit_frame(&parent.frame, frame)
        } else {
            frame.clone()
        };
        effective.push(EffectiveFrame {
            frame: resolved,
            sources,
        });
    }
    effective
}

fn inherit_frame(parent: &FoldFrame, child: &FoldFrame) -> FoldFrame {
    let mut result = parent.clone();
    if child.frame_title.is_some() {
        result.frame_title.clone_from(&child.frame_title);
    }
    if child.frame_description.is_some() {
        result
            .frame_description
            .clone_from(&child.frame_description);
    }
    if !child.frame_classes.is_empty() {
        result.frame_classes.clone_from(&child.frame_classes);
    }
    if !child.frame_attributes.is_empty() {
        result.frame_attributes.clone_from(&child.frame_attributes);
    }
    overlay(&mut result.vertices_coords, &child.vertices_coords);
    overlay(&mut result.edges_vertices, &child.edges_vertices);
    overlay(&mut result.edges_assignment, &child.edges_assignment);
    overlay(&mut result.edges_fold_angle, &child.edges_fold_angle);
    overlay(&mut result.faces_vertices, &child.faces_vertices);
    overlay(&mut result.face_orders, &child.face_orders);
    result.frame_parent = child.frame_parent;
    result.frame_inherit = child.frame_inherit;
    result.extra_fields.clone_from(&child.extra_fields);
    result
}

fn overlay<T: Clone>(target: &mut Option<T>, source: &Option<T>) {
    if source.is_some() {
        target.clone_from(source);
    }
}

fn frame_has_endpoint_semantics(frame: &FoldFrame) -> bool {
    frame
        .frame_classes
        .iter()
        .any(|class| class == "foldedForm")
        || frame.edges_fold_angle.as_ref().is_some_and(|angles| {
            angles
                .iter()
                .flatten()
                .any(|angle| angle.abs() > CONVERSION_EPS)
        })
        || frame
            .face_orders
            .as_ref()
            .is_some_and(|orders| !orders.is_empty())
}

fn validate_endpoint_coordinates(
    frame: &FoldFrame,
    root: &FoldFrame,
    normalization: Normalization,
    source_path: &str,
    errors: &mut Vec<FoldIssue>,
) {
    let (Some(endpoint), Some(initial)) = (
        frame.vertices_coords.as_ref(),
        root.vertices_coords.as_ref(),
    ) else {
        return;
    };
    if endpoint.len() != initial.len() {
        return;
    }
    for (index, (endpoint_vertex, initial_vertex)) in endpoint.iter().zip(initial).enumerate() {
        let (Some(endpoint_point), Some(initial_point)) =
            (vector2(endpoint_vertex), vector2(initial_vertex))
        else {
            continue;
        };
        let endpoint_normalized = normalization.transform(endpoint_point);
        let initial_normalized = normalization.transform(initial_point);
        if (endpoint_normalized[0] - initial_normalized[0]).abs() > CONVERSION_EPS
            || (endpoint_normalized[1] - initial_normalized[1]).abs() > CONVERSION_EPS
        {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                format!("{source_path}[{index}]"),
                "step frame固有の2D頂点位置は現行Documentへ直接保存できず、角度からの終点一致も未証明なので取込めません",
                Some(json!(endpoint_vertex)),
            ));
        }
    }
}

fn boundary_cycle(edges: &[Vec<usize>], assignments: &[FoldAssignment]) -> Option<Vec<usize>> {
    let mut adjacency = BTreeMap::<usize, Vec<usize>>::new();
    let mut boundary_count = 0_usize;
    for (edge, assignment) in edges.iter().zip(assignments) {
        if *assignment != FoldAssignment::Border || edge.len() != 2 || edge[0] == edge[1] {
            continue;
        }
        boundary_count += 1;
        adjacency.entry(edge[0]).or_default().push(edge[1]);
        adjacency.entry(edge[1]).or_default().push(edge[0]);
    }
    if boundary_count < 4 || adjacency.values().any(|neighbors| neighbors.len() != 2) {
        return None;
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
    }
    let start = *adjacency.keys().next()?;
    let mut previous = None;
    let mut current = start;
    let mut cycle = Vec::with_capacity(adjacency.len());
    loop {
        if cycle.contains(&current) {
            return None;
        }
        cycle.push(current);
        let neighbors = adjacency.get(&current)?;
        let next = match previous {
            None => *neighbors.first()?,
            Some(prior) if neighbors.first() == Some(&prior) => *neighbors.get(1)?,
            Some(_) => *neighbors.first()?,
        };
        if next == start {
            break;
        }
        previous = Some(current);
        current = next;
    }
    (cycle.len() == adjacency.len() && cycle.len() == boundary_count).then_some(cycle)
}

fn rectangle_corner_indices(boundary: &[usize], vertices: &[Vec<f64>]) -> Option<[usize; 4]> {
    let mut corners = Vec::new();
    for (position, &current_index) in boundary.iter().enumerate() {
        let previous = point(
            vertices,
            boundary[(position + boundary.len() - 1) % boundary.len()],
        )?;
        let current = point(vertices, current_index)?;
        let next = point(vertices, boundary[(position + 1) % boundary.len()])?;
        let incoming = subtract(current, previous);
        let outgoing = subtract(next, current);
        let scale = length(incoming) * length(outgoing);
        if scale <= 0.0 {
            return None;
        }
        if cross(incoming, outgoing).abs() > CONVERSION_EPS * scale {
            corners.push(current_index);
        }
    }
    corners.try_into().ok()
}

fn cycles_match(fold: &[usize], model: &[u32]) -> bool {
    if fold.len() != model.len() || fold.is_empty() {
        return false;
    }
    let Ok(model_first) = usize::try_from(model[0]) else {
        return false;
    };
    fold.iter()
        .enumerate()
        .filter(|&(_, value)| *value == model_first)
        .any(|(start, _)| {
            (0..fold.len()).all(|offset| {
                usize::try_from(model[offset]).ok() == Some(fold[(start + offset) % fold.len()])
            }) || (0..fold.len()).all(|offset| {
                usize::try_from(model[offset]).ok()
                    == Some(fold[(start + fold.len() - offset) % fold.len()])
            })
        })
}

fn vector2(value: &[f64]) -> Option<[f64; 2]> {
    let (&x, &y) = (value.first()?, value.get(1)?);
    (value.len() == 2 && x.is_finite() && y.is_finite()).then_some([x, y])
}

fn point(vertices: &[Vec<f64>], index: usize) -> Option<[f64; 2]> {
    vector2(vertices.get(index)?)
}

fn effective_frame_path(index: usize) -> String {
    if index == 0 {
        "$".to_string()
    } else {
        format!("$.file_frames[{}]", index - 1)
    }
}

fn approximately(left: f64, right: f64) -> bool {
    (left - right).abs() <= CONVERSION_EPS
}

fn subtract(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn dot(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[0] + left[1] * right[1]
}

fn cross(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[1] - left[1] * right[0]
}

fn length(vector: [f64; 2]) -> f64 {
    dot(vector, vector).sqrt()
}

fn issue(
    severity: FoldIssueSeverity,
    code: FoldIssueCode,
    path: impl Into<String>,
    message: impl Into<String>,
    original_value: Option<Value>,
) -> FoldIssue {
    FoldIssue {
        severity,
        code,
        path: path.into(),
        message: message.into(),
        original_value,
    }
}

fn numeric_value(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map_or_else(|| Value::String(value.to_string()), Value::Number)
}

fn sort_and_deduplicate(issues: &mut Vec<FoldIssue>) {
    issues.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| issue_code_rank(left.code).cmp(&issue_code_rank(right.code)))
            .then_with(|| left.message.cmp(&right.message))
    });
    issues.dedup();
}

fn issue_code_rank(code: FoldIssueCode) -> u8 {
    match code {
        FoldIssueCode::AssignmentDowngradedToAux => 0,
        FoldIssueCode::UnsupportedField => 1,
        FoldIssueCode::UnsupportedGeometry => 2,
        FoldIssueCode::NonLinearFrames => 3,
        FoldIssueCode::UnrepresentableFaceOrders => 4,
        FoldIssueCode::InvalidTopology => 5,
        FoldIssueCode::MissingRequiredField => 6,
        FoldIssueCode::InvalidValue => 7,
    }
}
