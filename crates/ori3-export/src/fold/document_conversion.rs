use std::collections::{BTreeMap, BTreeSet};

use ori3_cp::extract_faces;
use ori3_layers::{FlatState, resolve_driver_edges};
use ori3_model::{Document, EdgeKind, SCHEMA_VERSION, TechniqueKind};
use serde::Serialize;
use serde_json::{Value, json};

use super::conversion::{FoldConversionError, fold_to_document};
use super::types::{
    FoldAssignment, FoldFile, FoldFrame, FoldIssue, FoldIssueCode, FoldIssueSeverity,
};
use super::validation::validate_fold_1_2;

/// The accepted coordinate and angle roundtrip error from roadmap section 12.6.
/// Topology, assignment, and ordering are still compared exactly.
const CONVERSION_EPS: f64 = 1e-9;

/// A neutral FOLD value produced from an ORIGAMI3 document, plus every
/// non-blocking limitation that must be shown before the caller writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldExport {
    pub file: FoldFile,
    pub warnings: Vec<FoldIssue>,
}

/// Convert an ORIGAMI3 document into the approved FOLD 1.2 limited profile.
///
/// The conversion records endpoint angle snapshots, not technique names. Aux
/// edges are written as U because the model intentionally has no F/U
/// distinction; every such edge produces a path-bearing warning.
pub fn document_to_fold(document: &Document) -> Result<FoldExport, FoldConversionError> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    validate_document_header(document, &mut errors);
    record_unsupported_document_fields(document, &mut warnings);

    let Some(base) = canonical_document_geometry(document, &mut warnings, &mut errors) else {
        sort_and_deduplicate(&mut warnings);
        sort_and_deduplicate(&mut errors);
        return Err(FoldConversionError { warnings, errors });
    };

    let needs_faces = document
        .sequence
        .iter()
        .any(|step| step.layer_order.is_some());
    let model_faces = needs_faces.then(|| extract_faces(&document.cp));
    let faces_vertices = model_faces.as_ref().and_then(|faces| {
        let converted = faces
            .iter()
            .map(|face| {
                face.vertices
                    .iter()
                    .map(|vertex| base.vertex_indices.get(vertex).copied())
                    .collect::<Option<Vec<_>>>()
            })
            .collect::<Option<Vec<_>>>();
        if converted.is_none() {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                "$.cp",
                "抽出faceが存在しない頂点IDを参照しているためfaces_verticesへ変換できません",
                None,
            ));
        }
        converted
    });

    if needs_faces && model_faces.as_ref().is_none_or(Vec::is_empty) {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::UnrepresentableFaceOrders,
            "$.sequence",
            "layer_orderをfaceOrdersへ変換するための面を抽出できません",
            None,
        ));
    }

    let root = FoldFrame {
        frame_classes: vec!["creasePattern".to_string()],
        frame_attributes: vec!["2D".to_string()],
        vertices_coords: Some(base.vertices_coords),
        edges_vertices: Some(base.edges_vertices),
        edges_assignment: Some(base.assignments.clone()),
        edges_fold_angle: Some(vec![Some(0.0); base.assignments.len()]),
        faces_vertices,
        ..FoldFrame::default()
    };

    let mut endpoint_angles = vec![0.0; base.assignments.len()];
    let mut file_frames = Vec::with_capacity(document.sequence.len());
    for (step_index, step) in document.sequence.iter().enumerate() {
        apply_step_angles(
            document,
            step_index,
            &base.edge_indices,
            &base.assignments,
            &mut endpoint_angles,
            &mut errors,
        );

        let face_orders = step.layer_order.as_ref().and_then(|points| {
            layer_order_to_face_orders(
                document,
                step_index,
                points,
                &endpoint_angles,
                &base.assignments,
                model_faces.as_deref().unwrap_or_default(),
                &mut errors,
            )
        });
        let fold_angles = endpoint_angles
            .iter()
            .map(|angle| Some(canonical_zero(-angle)))
            .collect();
        file_frames.push(FoldFrame {
            frame_classes: vec!["foldedForm".to_string()],
            frame_attributes: vec!["2D".to_string()],
            frame_parent: Some(step_index),
            frame_inherit: Some(true),
            edges_fold_angle: Some(fold_angles),
            face_orders,
            ..FoldFrame::default()
        });
    }

    let file = FoldFile {
        file_spec: 1.2,
        file_creator: None,
        file_author: None,
        file_title: None,
        file_description: None,
        file_classes: vec!["singleModel".to_string()],
        root,
        file_frames,
        extra_fields: BTreeMap::new(),
    };

    // Reuse the neutral-profile gate so a malformed Document can never make the
    // writer panic or silently emit a wider profile. Generated U warnings are
    // replaced by the more accurate source-Aux warnings collected above.
    errors.extend(validate_fold_1_2(&file).errors);
    if errors.is_empty() {
        match fold_to_document(&file) {
            Ok(reimported) => compare_reimported_coordinates(
                &file,
                &reimported.document,
                &base.vertex_source_indices,
                &mut errors,
            ),
            Err(error) => errors.extend(error.errors),
        }
    }
    sort_and_deduplicate(&mut warnings);
    sort_and_deduplicate(&mut errors);
    if errors.is_empty() {
        Ok(FoldExport { file, warnings })
    } else {
        Err(FoldConversionError { warnings, errors })
    }
}

struct CanonicalDocumentGeometry {
    vertices_coords: Vec<Vec<f64>>,
    edges_vertices: Vec<Vec<usize>>,
    assignments: Vec<FoldAssignment>,
    vertex_indices: BTreeMap<u32, usize>,
    edge_indices: BTreeMap<u32, usize>,
    vertex_source_indices: Vec<usize>,
}

fn validate_document_header(document: &Document, errors: &mut Vec<FoldIssue>) {
    if document.schema_version != SCHEMA_VERSION {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::InvalidValue,
            "$.schema_version",
            format!(
                "現在のORIGAMI3 schema version {SCHEMA_VERSION}だけをFOLD 1.2 限定へ変換できます"
            ),
            Some(json!(document.schema_version)),
        ));
    }
    if !document.paper.width_mm.is_finite()
        || !document.paper.height_mm.is_finite()
        || document.paper.width_mm <= 0.0
        || document.paper.height_mm <= 0.0
    {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::InvalidValue,
            "$.paper",
            "紙の幅と高さは有限の正値でなければなりません",
            Some(json!({
                "width_mm": finite_or_string(document.paper.width_mm),
                "height_mm": finite_or_string(document.paper.height_mm),
            })),
        ));
    }
}

fn record_unsupported_document_fields(document: &Document, warnings: &mut Vec<FoldIssue>) {
    let paper_long_edge = document.paper.width_mm.max(document.paper.height_mm);
    if paper_long_edge.is_finite() && (paper_long_edge - 1.0).abs() > CONVERSION_EPS {
        warnings.push(issue(
            FoldIssueSeverity::Warning,
            FoldIssueCode::UnsupportedField,
            "$.paper",
            "FOLD 1.2 限定には物理的な紙寸法の単位を保存せず、2D座標の縦横比だけを保持します",
            Some(json!({
                "width_mm": finite_or_string(document.paper.width_mm),
                "height_mm": finite_or_string(document.paper.height_mm),
            })),
        ));
    }

    let default_display = ori3_model::DisplaySettings::default();
    let unsupported_display_fields = [
        (
            "$.display.front_color",
            document.display.front_color != default_display.front_color,
            json!(document.display.front_color),
        ),
        (
            "$.display.back_color",
            document.display.back_color != default_display.back_color,
            json!(document.display.back_color),
        ),
        (
            "$.display.grid_divisions",
            document.display.grid_divisions != default_display.grid_divisions,
            json!(document.display.grid_divisions),
        ),
        (
            "$.display.overlap_prevention_enabled",
            document.display.overlap_prevention_enabled
                != default_display.overlap_prevention_enabled,
            json!(document.display.overlap_prevention_enabled),
        ),
        (
            "$.display.penetration_prevention_enabled",
            document.display.penetration_prevention_enabled
                != default_display.penetration_prevention_enabled,
            json!(document.display.penetration_prevention_enabled),
        ),
    ];
    for (path, differs, original_value) in unsupported_display_fields {
        if differs {
            warnings.push(issue(
                FoldIssueSeverity::Warning,
                FoldIssueCode::UnsupportedField,
                path,
                "ORIGAMI3固有の表示設定はFOLD 1.2 限定へ保存しません",
                Some(original_value),
            ));
        }
    }
    if document.display.soft_enabled != default_display.soft_enabled
        || document.display.soft_stiffness != default_display.soft_stiffness
        || document.display.soft_pressure != default_display.soft_pressure
    {
        warnings.push(issue(
            FoldIssueSeverity::Warning,
            FoldIssueCode::UnsupportedField,
            "$.display",
            "仕上げの丸み設定はFOLD 1.2 限定へ保存しません",
            Some(json!({
                "soft_enabled": document.display.soft_enabled,
                "soft_stiffness": finite_or_string(document.display.soft_stiffness),
                "soft_pressure": finite_or_string(document.display.soft_pressure),
            })),
        ));
    }
    for (index, step) in document.sequence.iter().enumerate() {
        let path = format!("$.sequence[{index}]");
        if !matches!(step.kind, TechniqueKind::Simple | TechniqueKind::Pose) {
            warnings.push(issue(
                FoldIssueSeverity::Warning,
                FoldIssueCode::UnsupportedField,
                format!("{path}.kind"),
                "名前付き技法の意味はFOLD 1.2 限定へ保存せずgeneric step frameへ縮退します",
                Some(safe_json(&step.kind)),
            ));
        }
        if !step.note.is_empty() {
            warnings.push(issue(
                FoldIssueSeverity::Warning,
                FoldIssueCode::UnsupportedField,
                format!("{path}.note"),
                "手順の注記はFOLD 1.2 限定へ保存しません",
                Some(Value::String(step.note.clone())),
            ));
        }
        if let Some(alignment) = &step.alignment {
            warnings.push(issue(
                FoldIssueSeverity::Warning,
                FoldIssueCode::UnsupportedField,
                format!("{path}.alignment"),
                "名前付き技法の合わせ条件はFOLD 1.2 限定へ保存しません",
                Some(safe_json(alignment)),
            ));
        }
        if let Some(finish_soft) = &step.finish_soft {
            warnings.push(issue(
                FoldIssueSeverity::Warning,
                FoldIssueCode::UnsupportedField,
                format!("{path}.finish_soft"),
                "仕上げの丸みはFOLD 1.2 限定へ保存しません",
                Some(safe_json(finish_soft)),
            ));
        }
    }
}

fn canonical_document_geometry(
    document: &Document,
    warnings: &mut Vec<FoldIssue>,
    errors: &mut Vec<FoldIssue>,
) -> Option<CanonicalDocumentGeometry> {
    let vertices = canonical_vertex_order(document);
    let mut vertex_indices = BTreeMap::new();
    let mut vertices_coords = Vec::with_capacity(vertices.len());
    let mut vertex_source_indices = Vec::with_capacity(vertices.len());
    for (fold_index, &(source_index, vertex)) in vertices.iter().enumerate() {
        if vertex_indices.insert(vertex.id, fold_index).is_some() {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                format!("$.cp.vertices[{source_index}].id"),
                "頂点IDが重複しているためFOLD indexへ一意に変換できません",
                Some(json!(vertex.id)),
            ));
        }
        if vertex.pos.iter().any(|value| !value.is_finite()) {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidValue,
                format!("$.cp.vertices[{source_index}].pos"),
                "頂点座標は有限でなければなりません",
                Some(safe_json(&vertex.pos)),
            ));
        }
        vertices_coords.push(vertex.pos.to_vec());
        vertex_source_indices.push(source_index);
    }

    let mut edges = document.cp.edges.iter().enumerate().collect::<Vec<_>>();
    edges.sort_by_key(|(_, edge)| edge.id);
    let mut seen_edge_ids = BTreeSet::new();
    let mut edge_indices = BTreeMap::new();
    let mut edges_vertices = Vec::with_capacity(edges.len());
    let mut assignments = Vec::with_capacity(edges.len());
    for (fold_index, &(source_index, edge)) in edges.iter().enumerate() {
        if !seen_edge_ids.insert(edge.id) {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                format!("$.cp.edges[{source_index}].id"),
                "edge IDが重複しているためFOLD indexへ一意に変換できません",
                Some(json!(edge.id)),
            ));
        }
        edge_indices.insert(edge.id, fold_index);
        let (Some(&v0), Some(&v1)) = (vertex_indices.get(&edge.v0), vertex_indices.get(&edge.v1))
        else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                format!("$.cp.edges[{source_index}]"),
                "edgeが存在しない頂点IDを参照しているためFOLD topologyへ変換できません",
                Some(safe_json(edge)),
            ));
            continue;
        };
        edges_vertices.push(vec![v0, v1]);
        let assignment = match edge.kind {
            EdgeKind::Border => FoldAssignment::Border,
            EdgeKind::Mountain => FoldAssignment::Mountain,
            EdgeKind::Valley => FoldAssignment::Valley,
            EdgeKind::Aux => {
                warnings.push(issue(
                    FoldIssueSeverity::Warning,
                    FoldIssueCode::AssignmentDowngradedToAux,
                    format!("$.cp.edges[{source_index}].kind"),
                    format!(
                        "ORIGAMI3 Auxは元のF/Uを区別できないためFOLD edges_assignment[{fold_index}]へUとして書き出します"
                    ),
                    Some(Value::String("Aux".to_string())),
                ));
                FoldAssignment::Unassigned
            }
        };
        assignments.push(assignment);
    }

    if !errors.is_empty()
        || edges_vertices.len() != document.cp.edges.len()
        || assignments.len() != document.cp.edges.len()
    {
        return None;
    }
    Some(CanonicalDocumentGeometry {
        vertices_coords,
        edges_vertices,
        assignments,
        vertex_indices,
        edge_indices,
        vertex_source_indices,
    })
}

fn canonical_vertex_order(document: &Document) -> Vec<(usize, &ori3_model::Vertex)> {
    let mut vertices = document.cp.vertices.iter().enumerate().collect::<Vec<_>>();
    vertices.sort_by_key(|(_, vertex)| vertex.id);

    let mut boundary_vertices = BTreeSet::new();
    let mut boundary_neighbours = BTreeMap::<u32, Vec<u32>>::new();
    for edge in &document.cp.edges {
        if edge.kind != EdgeKind::Border {
            continue;
        }
        boundary_vertices.insert(edge.v0);
        boundary_vertices.insert(edge.v1);
        boundary_neighbours
            .entry(edge.v0)
            .or_default()
            .push(edge.v1);
        boundary_neighbours
            .entry(edge.v1)
            .or_default()
            .push(edge.v0);
    }
    let positions = document
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<BTreeMap<_, _>>();
    let minimum = boundary_vertices
        .iter()
        .filter_map(|id| positions.get(id))
        .fold([f64::INFINITY, f64::INFINITY], |minimum, point| {
            [minimum[0].min(point[0]), minimum[1].min(point[1])]
        });
    let bottom_left = boundary_vertices.iter().copied().find(|id| {
        positions.get(id).is_some_and(|point| {
            (point[0] - minimum[0]).abs() <= CONVERSION_EPS
                && (point[1] - minimum[1]).abs() <= CONVERSION_EPS
        })
    });
    let bottom_next = bottom_left.and_then(|bottom_left| {
        boundary_neighbours
            .get(&bottom_left)
            .into_iter()
            .flatten()
            .copied()
            .filter(|id| {
                positions.get(id).is_some_and(|point| {
                    (point[1] - minimum[1]).abs() <= CONVERSION_EPS
                        && point[0] > minimum[0] + CONVERSION_EPS
                })
            })
            .min_by(|left, right| positions[left][0].total_cmp(&positions[right][0]))
    });
    let (Some(bottom_left), Some(bottom_next)) = (bottom_left, bottom_next) else {
        return vertices;
    };

    vertices.sort_by_key(|(_, vertex)| {
        if vertex.id == bottom_left {
            (0_u8, vertex.id)
        } else if vertex.id == bottom_next {
            (1_u8, vertex.id)
        } else {
            (2_u8, vertex.id)
        }
    });
    vertices
}

fn compare_reimported_coordinates(
    file: &FoldFile,
    document: &Document,
    source_indices: &[usize],
    errors: &mut Vec<FoldIssue>,
) {
    let Some(expected) = &file.root.vertices_coords else {
        return;
    };
    if expected.len() != document.cp.vertices.len() {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::InvalidTopology,
            "$.cp.vertices",
            "FOLD往復で頂点数を保持できません",
            Some(json!({"before": expected.len(), "after": document.cp.vertices.len()})),
        ));
        return;
    }
    for (fold_index, (expected, actual)) in expected.iter().zip(&document.cp.vertices).enumerate() {
        let source_index = source_indices
            .get(fold_index)
            .copied()
            .unwrap_or(fold_index);
        let differs = expected.len() != 2
            || (expected[0] - actual.pos[0]).abs() > CONVERSION_EPS
            || (expected[1] - actual.pos[1]).abs() > CONVERSION_EPS;
        if differs {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                format!("$.cp.vertices[{source_index}].pos"),
                "Document座標は原点・軸・長辺1のcanonical紙座標でなく、FOLD往復で1e-9以内に保持できません",
                Some(safe_json(expected)),
            ));
        }
    }
}

fn apply_step_angles(
    document: &Document,
    step_index: usize,
    edge_indices: &BTreeMap<u32, usize>,
    assignments: &[FoldAssignment],
    endpoint_angles: &mut [f64],
    errors: &mut Vec<FoldIssue>,
) {
    let step = &document.sequence[step_index];
    let mut updates = BTreeMap::<usize, (f64, usize)>::new();
    for (driver_index, driver) in step.drivers.iter().enumerate() {
        let path = format!("$.sequence[{step_index}].drivers[{driver_index}]");
        if driver
            .a
            .iter()
            .chain(&driver.b)
            .any(|value| !value.is_finite())
        {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidValue,
                &path,
                "DriverLineの端点は有限でなければなりません",
                Some(safe_json(driver)),
            ));
            continue;
        }
        let angle = driver.target_angle_deg;
        if !angle.is_finite() || !(-180.0..=180.0).contains(&angle) {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidValue,
                format!("{path}.target_angle_deg"),
                "driver角は有限の-180度以上180度以下でなければなりません",
                Some(finite_or_string(angle)),
            ));
            continue;
        }
        let resolved = resolve_driver_edges(&document.cp, driver);
        if resolved.is_empty() {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                &path,
                "DriverLineをMountain/Valley edgeへ1件も解決できません",
                Some(safe_json(driver)),
            ));
            continue;
        }
        for edge_id in resolved {
            let Some(&edge_index) = edge_indices.get(&edge_id) else {
                errors.push(issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    &path,
                    format!("解決したedge ID {edge_id}がcanonical topologyにありません"),
                    Some(json!(edge_id)),
                ));
                continue;
            };
            // replayと同じく、同一stepで同じedgeへ複数指定があれば後の
            // DriverLineがendpoint authorityになる。
            updates.insert(edge_index, (canonical_zero(angle), driver_index));
        }
    }

    for (edge_index, (angle, driver_index)) in updates {
        let inconsistent = match assignments.get(edge_index) {
            Some(FoldAssignment::Mountain) => angle < -CONVERSION_EPS,
            Some(FoldAssignment::Valley) => angle > CONVERSION_EPS,
            _ => true,
        };
        if inconsistent {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidValue,
                format!("$.sequence[{step_index}].drivers[{driver_index}].target_angle_deg"),
                format!("ORIGAMI3 edge assignmentとdriver角 {angle}度の符号が一致しません"),
                Some(finite_or_string(angle)),
            ));
            continue;
        }
        endpoint_angles[edge_index] = angle;
    }
}

fn layer_order_to_face_orders(
    document: &Document,
    step_index: usize,
    points: &[[f64; 2]],
    endpoint_angles: &[f64],
    assignments: &[FoldAssignment],
    faces: &[ori3_cp::Face],
    errors: &mut Vec<FoldIssue>,
) -> Option<Vec<Vec<i64>>> {
    let path = format!("$.sequence[{step_index}].layer_order");
    if points.len() != faces.len() || points.iter().flatten().any(|value| !value.is_finite()) {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::UnrepresentableFaceOrders,
            &path,
            format!(
                "layer_orderは有限な代表点を全face {}件にちょうど1件ずつ持つ必要があります",
                faces.len()
            ),
            Some(safe_json(&points)),
        ));
        return None;
    }
    let flat_endpoint = assignments
        .iter()
        .zip(endpoint_angles)
        .all(|(assignment, angle)| {
            !matches!(
                assignment,
                FoldAssignment::Mountain | FoldAssignment::Valley
            ) || angle.abs() <= CONVERSION_EPS
                || (angle.abs() - 180.0).abs() <= CONVERSION_EPS
        });
    if !flat_endpoint {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::UnrepresentableFaceOrders,
            &path,
            "非平坦endpointのlayer_orderをFOLD faceOrdersへ意味を変えずに変換できません",
            Some(safe_json(&points)),
        ));
        return None;
    }

    let (resolved, resolve_warnings) = FlatState::resolve_order(&document.cp, faces, points);
    if !resolve_warnings.is_empty() {
        errors.push(issue(
            FoldIssueSeverity::Error,
            FoldIssueCode::UnrepresentableFaceOrders,
            &path,
            format!(
                "layer_orderの代表点を全faceへ一意に解決できません: {}",
                resolve_warnings.join(" / ")
            ),
            Some(safe_json(&points)),
        ));
        return None;
    }
    let face_indices = faces
        .iter()
        .enumerate()
        .map(|(index, face)| (face.id, index))
        .collect::<BTreeMap<_, _>>();
    let mut order = Vec::with_capacity(resolved.len());
    for face_id in resolved {
        let Some(&index) = face_indices.get(&face_id) else {
            errors.push(issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnrepresentableFaceOrders,
                &path,
                format!("layer_orderが未知のface ID {face_id}へ解決されました"),
                Some(json!(face_id)),
            ));
            return None;
        };
        order.push(index);
    }

    order
        .windows(2)
        .map(|pair| {
            // FOLD +1 means first is above second; `order` is bottom→top.
            let upper = i64::try_from(pair[1]).ok()?;
            let lower = i64::try_from(pair[0]).ok()?;
            Some(vec![upper, lower, 1])
        })
        .collect()
}

fn canonical_zero(value: f64) -> f64 {
    if value.abs() <= CONVERSION_EPS {
        0.0
    } else {
        value
    }
}

fn finite_or_string(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map_or_else(|| Value::String(value.to_string()), Value::Number)
}

fn safe_json<T: Serialize + ?Sized>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or_else(|error| Value::String(error.to_string()))
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
