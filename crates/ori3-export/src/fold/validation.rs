use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::types::{
    FOLD_1_2_PROFILE_NAME, FoldAssignment, FoldFile, FoldFrame, FoldIssue, FoldIssueCode,
    FoldIssueSeverity, FoldValidation,
};

/// Rectangle checks first divide coordinates by the longest boundary side.
/// `1e-9` is therefore the same normalized-coordinate boundary as the approved
/// roundtrip tolerance, while still being far below a visibly different outline.
const NORMALIZED_GEOMETRY_EPS: f64 = 1e-9;
const ANGLE_EPS_DEG: f64 = 1e-9;
const READABLE_FOLD_SPEC_VERSIONS: [f64; 2] = [1.1, 1.2];

/// Validate whether a parsed FOLD file belongs to the approved FOLD 1.1/1.2 profile.
///
/// Parsing and validation are deliberately separate. This function never repairs
/// topology or discards a field: every lossy or unsupported input produces an
/// issue carrying the exact JSON path.
#[must_use]
pub fn validate_fold_1_2(file: &FoldFile) -> FoldValidation {
    let mut validation = FoldValidation::default();

    if !file.file_spec.is_finite() || !READABLE_FOLD_SPEC_VERSIONS.contains(&file.file_spec) {
        record_issue(
            &mut validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidValue,
                "$.file_spec",
                format!(
                    "限定profileが検証できるfile_specは有限の1.1または1.2だけです（指定: {}）",
                    file.file_spec
                ),
                if file.file_spec.is_finite() {
                    Some(json!(file.file_spec))
                } else {
                    None
                },
            ),
        );
    }

    for issue in unsupported_fields(file) {
        record_issue(&mut validation, issue);
    }

    validate_root_frame_link(&file.root, &mut validation);
    validate_frame_links(&file.file_frames, &mut validation);
    validate_declared_frame(&file.root, "$", &mut validation);
    for (index, frame) in file.file_frames.iter().enumerate() {
        validate_declared_frame(frame, &format!("$.file_frames[{index}]"), &mut validation);
    }

    let effective_frames = effective_frames(file);
    for (index, frame) in effective_frames.iter().enumerate() {
        let path = effective_frame_path(index);
        validate_effective_frame(frame, &path, index == 0, &mut validation);
    }
    validate_stable_step_topology(&effective_frames, &mut validation);

    sort_and_deduplicate(&mut validation.warnings);
    sort_and_deduplicate(&mut validation.errors);
    validation
}

/// Return fields and field values which the limited profile cannot preserve.
///
/// Generic metadata/extensions are warnings: a later UI may offer an explicit
/// limited import, but it must show every path. Fields which change geometry or
/// sequence meaning are errors and cannot be imported by approximation.
/// Coordinate dimensions, frame links, assignments and `faceOrders` are
/// structural checks; callers that need the complete decision must use
/// [`validate_fold_1_2`] and show both its warnings and errors.
#[must_use]
pub fn unsupported_fields(file: &FoldFile) -> Vec<FoldIssue> {
    let mut issues = Vec::new();

    metadata_issue(
        &mut issues,
        "$.file_creator",
        file.file_creator.as_ref(),
        "作成tool情報",
    );
    metadata_issue(
        &mut issues,
        "$.file_author",
        file.file_author.as_ref(),
        "作者情報",
    );
    metadata_issue(
        &mut issues,
        "$.file_title",
        file.file_title.as_ref(),
        "作品名",
    );
    metadata_issue(
        &mut issues,
        "$.file_description",
        file.file_description.as_ref(),
        "注記",
    );
    class_issues(&mut issues, &file.file_classes, "$.file_classes", true);
    extra_field_issues(&mut issues, &file.extra_fields, "$", true);

    frame_unsupported_issues(&mut issues, &file.root, "$");
    for (index, frame) in file.file_frames.iter().enumerate() {
        frame_unsupported_issues(&mut issues, frame, &format!("$.file_frames[{index}]"));
    }

    sort_and_deduplicate(&mut issues);
    issues
}

fn frame_unsupported_issues(issues: &mut Vec<FoldIssue>, frame: &FoldFrame, path: &str) {
    metadata_issue(
        issues,
        &field_path(path, "frame_title"),
        frame.frame_title.as_ref(),
        "frame名（名前付き技法の意味を含む）",
    );
    metadata_issue(
        issues,
        &field_path(path, "frame_description"),
        frame.frame_description.as_ref(),
        "frame注記",
    );
    class_issues(
        issues,
        &frame.frame_classes,
        &field_path(path, "frame_classes"),
        false,
    );
    attribute_issues(
        issues,
        &frame.frame_attributes,
        &field_path(path, "frame_attributes"),
    );
    extra_field_issues(issues, &frame.extra_fields, path, false);
}

fn metadata_issue(issues: &mut Vec<FoldIssue>, path: &str, value: Option<&String>, label: &str) {
    let Some(value) = value else {
        return;
    };
    issues.push(issue(
        FoldIssueSeverity::Warning,
        FoldIssueCode::UnsupportedField,
        path,
        format!("{FOLD_1_2_PROFILE_NAME}は{label}を保持しません"),
        Some(Value::String(value.clone())),
    ));
}

fn class_issues(issues: &mut Vec<FoldIssue>, classes: &[String], path: &str, file_level: bool) {
    for (index, class) in classes.iter().enumerate() {
        let class_path = index_path(path, index);
        let accepted = if file_level {
            class == "singleModel"
        } else {
            matches!(class.as_str(), "creasePattern" | "foldedForm")
        };
        if accepted {
            continue;
        }

        let blocking = matches!(class.as_str(), "animation" | "multiModel");
        issues.push(issue(
            if blocking {
                FoldIssueSeverity::Error
            } else {
                FoldIssueSeverity::Warning
            },
            FoldIssueCode::UnsupportedField,
            &class_path,
            if blocking {
                format!("{class} classは線形な単一作品ではありません")
            } else {
                format!("{class} classの意味は{FOLD_1_2_PROFILE_NAME}で保持しません")
            },
            Some(Value::String(class.clone())),
        ));
    }
}

fn attribute_issues(issues: &mut Vec<FoldIssue>, attributes: &[String], path: &str) {
    for (index, attribute) in attributes.iter().enumerate() {
        if matches!(attribute.as_str(), "2D" | "manifold" | "orientable") {
            continue;
        }
        let blocking = matches!(
            attribute.as_str(),
            "3D" | "nonManifold" | "nonOrientable" | "selfIntersecting"
        );
        issues.push(issue(
            if blocking {
                FoldIssueSeverity::Error
            } else {
                FoldIssueSeverity::Warning
            },
            if blocking {
                FoldIssueCode::UnsupportedGeometry
            } else {
                FoldIssueCode::UnsupportedField
            },
            index_path(path, index),
            if blocking {
                format!("{attribute} geometryは{FOLD_1_2_PROFILE_NAME}の対象外です")
            } else {
                format!("{attribute} attributeの意味は保持しません")
            },
            Some(Value::String(attribute.clone())),
        ));
    }
}

fn extra_field_issues(
    issues: &mut Vec<FoldIssue>,
    fields: &BTreeMap<String, Value>,
    parent_path: &str,
    file_level: bool,
) {
    for (key, value) in fields {
        let blocking = geometry_or_sequence_field(key);
        let scope = if file_level { "file" } else { "frame" };
        issues.push(issue(
            if blocking {
                FoldIssueSeverity::Error
            } else {
                FoldIssueSeverity::Warning
            },
            FoldIssueCode::UnsupportedField,
            unknown_field_path(parent_path, key),
            if blocking {
                format!("{key}は未対応の{scope} geometryまたは手順を表すため取込めません")
            } else {
                format!("未知の{scope} field {key}は保持しません")
            },
            Some(value.clone()),
        ));
    }
}

fn geometry_or_sequence_field(key: &str) -> bool {
    matches!(
        key,
        "file_animation"
            | "frame_time"
            | "frame_duration"
            | "frame_rate"
            | "vertices_coords3d"
            | "edges_curve"
            | "edges_curves"
            | "edges_bezier"
            | "edges_controlPoints"
            | "faces_holes"
            | "faces_cutouts"
    )
}

fn validate_root_frame_link(root: &FoldFrame, validation: &mut FoldValidation) {
    if let Some(parent) = root.frame_parent {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::NonLinearFrames,
                "$.frame_parent",
                "root frameはparentを持てません",
                Some(json!(parent)),
            ),
        );
    }
    if let Some(inherit) = root.frame_inherit {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::NonLinearFrames,
                "$.frame_inherit",
                "root frameは別frameを継承できません",
                Some(json!(inherit)),
            ),
        );
    }
}

fn validate_frame_links(frames: &[FoldFrame], validation: &mut FoldValidation) {
    for (index, frame) in frames.iter().enumerate() {
        let expected_parent = index;
        let parent_path = format!("$.file_frames[{index}].frame_parent");
        match frame.frame_parent {
            Some(parent) if parent == expected_parent => {}
            None => record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::NonLinearFrames,
                    &parent_path,
                    format!(
                        "直列frame {}のparentは直前frame {expected_parent}でなければなりません",
                        index + 1
                    ),
                    None,
                ),
            ),
            Some(parent) => record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::NonLinearFrames,
                    &parent_path,
                    format!(
                        "直列frame {}のparentは{expected_parent}です（指定: {parent}）",
                        index + 1
                    ),
                    Some(json!(parent)),
                ),
            ),
        }
    }
}

fn validate_declared_frame(frame: &FoldFrame, path: &str, validation: &mut FoldValidation) {
    if let Some(vertices) = &frame.vertices_coords {
        validate_declared_vertices(vertices, &field_path(path, "vertices_coords"), validation);
    }
    if let Some(edges) = &frame.edges_vertices {
        validate_declared_edges(edges, &field_path(path, "edges_vertices"), validation);
    }
    if let Some(assignments) = &frame.edges_assignment {
        validate_declared_assignments(
            assignments,
            &field_path(path, "edges_assignment"),
            validation,
        );
    }
    if let Some(faces) = &frame.faces_vertices {
        validate_declared_faces(faces, &field_path(path, "faces_vertices"), validation);
    }
}

fn validate_declared_vertices(vertices: &[Vec<f64>], path: &str, validation: &mut FoldValidation) {
    for (index, vertex) in vertices.iter().enumerate() {
        for (component_index, &component) in vertex.iter().enumerate() {
            if !component.is_finite() {
                record_issue(
                    validation,
                    issue(
                        FoldIssueSeverity::Error,
                        FoldIssueCode::InvalidValue,
                        format!("{path}[{index}][{component_index}]"),
                        "vertices_coordsの各成分は有限のf64でなければなりません",
                        None,
                    ),
                );
            }
        }
        if vertex.len() == 2 {
            continue;
        }
        let three_dimensional = vertex.len() == 3;
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                if three_dimensional {
                    FoldIssueCode::UnsupportedGeometry
                } else {
                    FoldIssueCode::InvalidValue
                },
                index_path(path, index),
                if three_dimensional {
                    "3D vertices_coordsを2Dへ切り捨てて取込むことはできません".to_string()
                } else {
                    format!(
                        "vertices_coordsは2成分でなければなりません（成分数: {}）",
                        vertex.len()
                    )
                },
                Some(json!(vertex)),
            ),
        );
    }
}

fn validate_declared_edges(edges: &[Vec<usize>], path: &str, validation: &mut FoldValidation) {
    for (index, edge) in edges.iter().enumerate() {
        if edge.len() != 2 {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    index_path(path, index),
                    format!(
                        "edges_verticesは2頂点でなければなりません（要素数: {}）",
                        edge.len()
                    ),
                    Some(json!(edge)),
                ),
            );
        }
    }
}

fn validate_declared_assignments(
    assignments: &[FoldAssignment],
    path: &str,
    validation: &mut FoldValidation,
) {
    for (index, assignment) in assignments.iter().enumerate() {
        let assignment_path = index_path(path, index);
        match assignment {
            FoldAssignment::Flat | FoldAssignment::Unassigned => record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Warning,
                    FoldIssueCode::AssignmentDowngradedToAux,
                    &assignment_path,
                    format!(
                        "assignment {}はAuxへ縮退します。元指定とpathを警告として保持します",
                        assignment.code()
                    ),
                    Some(Value::String(assignment.code().to_string())),
                ),
            ),
            FoldAssignment::Other(code) => record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidValue,
                    &assignment_path,
                    format!("assignment {code}は限定profileのB/M/V/F/Uではありません"),
                    Some(Value::String(code.clone())),
                ),
            ),
            FoldAssignment::Border | FoldAssignment::Mountain | FoldAssignment::Valley => {}
        }
    }
}

fn validate_declared_faces(faces: &[Vec<usize>], path: &str, validation: &mut FoldValidation) {
    for (index, face) in faces.iter().enumerate() {
        if face.len() < 3 {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    index_path(path, index),
                    "faces_verticesの各面には3頂点以上が必要です",
                    Some(json!(face)),
                ),
            );
        }
    }
}

fn effective_frames(file: &FoldFile) -> Vec<FoldFrame> {
    let mut effective = vec![file.root.clone()];
    for frame in &file.file_frames {
        let resolved = match (frame.frame_inherit, effective.last()) {
            (Some(true), Some(parent)) => inherit_frame(parent, frame),
            _ => frame.clone(),
        };
        effective.push(resolved);
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

fn validate_effective_frame(
    frame: &FoldFrame,
    path: &str,
    initial: bool,
    validation: &mut FoldValidation,
) {
    require_field(
        frame.vertices_coords.is_some(),
        &field_path(path, "vertices_coords"),
        "2D頂点座標",
        validation,
    );
    require_field(
        frame.edges_vertices.is_some(),
        &field_path(path, "edges_vertices"),
        "edge topology",
        validation,
    );
    require_field(
        frame.edges_assignment.is_some(),
        &field_path(path, "edges_assignment"),
        "edge assignment",
        validation,
    );

    validate_topology(frame, path, validation);
    validate_angles(frame, path, validation);
    validate_faces(frame, path, validation);
    validate_face_orders(frame, path, validation);
    if initial {
        validate_rectangular_single_sheet(frame, path, validation);
    }
}

fn require_field(present: bool, path: &str, label: &str, validation: &mut FoldValidation) {
    if !present {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::MissingRequiredField,
                path,
                format!("{FOLD_1_2_PROFILE_NAME}には{label}が必要です"),
                None,
            ),
        );
    }
}

fn validate_topology(frame: &FoldFrame, path: &str, validation: &mut FoldValidation) {
    let (Some(vertices), Some(edges), Some(assignments)) = (
        frame.vertices_coords.as_ref(),
        frame.edges_vertices.as_ref(),
        frame.edges_assignment.as_ref(),
    ) else {
        return;
    };

    if vertices.is_empty() {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                field_path(path, "vertices_coords"),
                "頂点が1件もありません",
                Some(json!([])),
            ),
        );
    }
    if edges.len() != assignments.len() {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                field_path(path, "edges_assignment"),
                format!(
                    "edges_vertices {}件に対してedges_assignmentは{}件です",
                    edges.len(),
                    assignments.len()
                ),
                Some(json!(assignments.len())),
            ),
        );
    }
    if let Some(angles) = &frame.edges_fold_angle
        && angles.len() != edges.len()
    {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::InvalidTopology,
                field_path(path, "edges_foldAngle"),
                format!(
                    "edges_vertices {}件に対してedges_foldAngleは{}件です",
                    edges.len(),
                    angles.len()
                ),
                Some(json!(angles.len())),
            ),
        );
    }

    let mut seen = BTreeSet::new();
    for (edge_index, edge) in edges.iter().enumerate() {
        if edge.len() != 2 {
            continue;
        }
        for (endpoint_index, &vertex) in edge.iter().enumerate() {
            if vertex >= vertices.len() {
                record_issue(
                    validation,
                    issue(
                        FoldIssueSeverity::Error,
                        FoldIssueCode::InvalidTopology,
                        format!("{path}.edges_vertices[{edge_index}][{endpoint_index}]"),
                        format!(
                            "頂点index {vertex}はvertices_coords {}件の範囲外です",
                            vertices.len()
                        ),
                        Some(json!(vertex)),
                    ),
                );
            }
        }
        if edge[0] == edge[1] {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    format!("{path}.edges_vertices[{edge_index}]"),
                    "edgeの両端を同じ頂点にはできません",
                    Some(json!(edge)),
                ),
            );
        }
        let canonical = canonical_edge(edge[0], edge[1]);
        if !seen.insert(canonical) {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    format!("{path}.edges_vertices[{edge_index}]"),
                    "同じ2頂点を結ぶedgeが重複しています",
                    Some(json!(edge)),
                ),
            );
        }
    }
}

fn validate_angles(frame: &FoldFrame, path: &str, validation: &mut FoldValidation) {
    let (Some(angles), Some(assignments)) = (
        frame.edges_fold_angle.as_ref(),
        frame.edges_assignment.as_ref(),
    ) else {
        return;
    };

    for (index, (angle, assignment)) in angles.iter().zip(assignments).enumerate() {
        let Some(angle) = angle else {
            continue;
        };
        let angle_path = format!("{path}.edges_foldAngle[{index}]");
        if !(-180.0..=180.0).contains(angle) {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidValue,
                    &angle_path,
                    "fold angleは-180度以上180度以下でなければなりません",
                    Some(json!(angle)),
                ),
            );
            continue;
        }

        let inconsistent = match assignment {
            FoldAssignment::Border | FoldAssignment::Flat | FoldAssignment::Unassigned => {
                angle.abs() > ANGLE_EPS_DEG
            }
            // FOLDの符号はMが負、Vが正。ORIGAMI3側との反転はmodel変換段階で行う。
            FoldAssignment::Mountain => *angle > ANGLE_EPS_DEG,
            FoldAssignment::Valley => *angle < -ANGLE_EPS_DEG,
            FoldAssignment::Other(_) => false,
        };
        if inconsistent {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidValue,
                    &angle_path,
                    format!(
                        "assignment {}とFOLD fold angle {angle}度の符号が一致しません",
                        assignment.code()
                    ),
                    Some(json!(angle)),
                ),
            );
        }
    }
}

fn validate_faces(frame: &FoldFrame, path: &str, validation: &mut FoldValidation) {
    let Some(faces) = &frame.faces_vertices else {
        return;
    };
    let vertex_count = frame.vertices_coords.as_ref().map_or(0, Vec::len);
    let edge_set = frame
        .edges_vertices
        .as_ref()
        .map(|edges| {
            edges
                .iter()
                .filter(|edge| edge.len() == 2)
                .map(|edge| canonical_edge(edge[0], edge[1]))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let mut edge_uses = BTreeMap::<(usize, usize), usize>::new();

    for (face_index, face) in faces.iter().enumerate() {
        if face.len() < 3 {
            continue;
        }
        let face_path = format!("{path}.faces_vertices[{face_index}]");
        let mut distinct = BTreeSet::new();
        for (vertex_index, &vertex) in face.iter().enumerate() {
            if vertex >= vertex_count {
                record_issue(
                    validation,
                    issue(
                        FoldIssueSeverity::Error,
                        FoldIssueCode::InvalidTopology,
                        format!("{face_path}[{vertex_index}]"),
                        format!("faceの頂点index {vertex}は範囲外です"),
                        Some(json!(vertex)),
                    ),
                );
            }
            if !distinct.insert(vertex) {
                record_issue(
                    validation,
                    issue(
                        FoldIssueSeverity::Error,
                        FoldIssueCode::InvalidTopology,
                        &face_path,
                        "同じface内で頂点が重複しています",
                        Some(json!(face)),
                    ),
                );
            }
        }
        for (&first, &second) in face
            .iter()
            .zip(face.iter().cycle().skip(1))
            .take(face.len())
        {
            let edge = canonical_edge(first, second);
            if !edge_set.contains(&edge) {
                record_issue(
                    validation,
                    issue(
                        FoldIssueSeverity::Error,
                        FoldIssueCode::InvalidTopology,
                        &face_path,
                        format!(
                            "face境界edge [{}, {}] がedges_verticesにありません",
                            edge.0, edge.1
                        ),
                        Some(json!([edge.0, edge.1])),
                    ),
                );
            }
            *edge_uses.entry(edge).or_default() += 1;
        }
    }

    for (edge, count) in edge_uses {
        if count > 2 {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::UnsupportedGeometry,
                    field_path(path, "faces_vertices"),
                    format!(
                        "edge [{}, {}] を{count}面が共有する非多様体は対象外です",
                        edge.0, edge.1
                    ),
                    Some(json!([edge.0, edge.1])),
                ),
            );
        }
    }
}

fn validate_face_orders(frame: &FoldFrame, path: &str, validation: &mut FoldValidation) {
    let Some(orders) = &frame.face_orders else {
        return;
    };
    if orders.is_empty() {
        return;
    }
    let Some(faces) = &frame.faces_vertices else {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnrepresentableFaceOrders,
                field_path(path, "faceOrders"),
                "faceOrdersを面の代表点へ変換するにはfaces_verticesが必要です",
                None,
            ),
        );
        return;
    };

    let mut graph = vec![BTreeSet::<usize>::new(); faces.len()];
    let mut invalid = false;
    for (order_index, order) in orders.iter().enumerate() {
        let order_path = format!("{path}.faceOrders[{order_index}]");
        if order.len() != 3 {
            invalid = true;
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::UnrepresentableFaceOrders,
                    &order_path,
                    "faceOrdersの各制約は[face, face, sign]の3要素でなければなりません",
                    Some(json!(order)),
                ),
            );
            continue;
        }
        let (Ok(first), Ok(second)) = (usize::try_from(order[0]), usize::try_from(order[1])) else {
            invalid = true;
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::UnrepresentableFaceOrders,
                    &order_path,
                    "faceOrdersのface indexは0以上でなければなりません",
                    Some(json!(order)),
                ),
            );
            continue;
        };
        if first >= faces.len() || second >= faces.len() || first == second {
            invalid = true;
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::UnrepresentableFaceOrders,
                    &order_path,
                    "faceOrdersは異なる有効なface indexを参照する必要があります",
                    Some(json!(order)),
                ),
            );
            continue;
        }
        if !matches!(order[2], -1 | 1) {
            invalid = true;
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::UnrepresentableFaceOrders,
                    format!("{order_path}[2]"),
                    "faceOrdersのsignは-1または1でなければなりません",
                    Some(json!(order[2])),
                ),
            );
            continue;
        }
        // FOLD 1.2 の +1 は「first が second より上」、-1 はその逆。
        // ORIGAMI3 の layer_order は下→上なので、graph も下→上へ向ける。
        let directed = if order[2] == 1 {
            (second, first)
        } else {
            (first, second)
        };
        graph[directed.0].insert(directed.1);
    }
    if invalid {
        return;
    }

    let mut indegree = vec![0_usize; faces.len()];
    for targets in &graph {
        for &target in targets {
            indegree[target] += 1;
        }
    }
    let mut removed = vec![false; faces.len()];
    let mut unique = true;
    for _ in 0..faces.len() {
        let available = indegree
            .iter()
            .enumerate()
            .filter(|&(index, &degree)| !removed[index] && degree == 0)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if available.is_empty() {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::UnrepresentableFaceOrders,
                    field_path(path, "faceOrders"),
                    "循環するfaceOrdersはlayer_orderへ変換できません",
                    Some(json!(orders)),
                ),
            );
            return;
        }
        if available.len() != 1 {
            unique = false;
        }
        let selected = available[0];
        removed[selected] = true;
        for &target in &graph[selected] {
            indegree[target] -= 1;
        }
    }
    if !unique {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnrepresentableFaceOrders,
                field_path(path, "faceOrders"),
                "非循環でも順序が一意でないfaceOrdersは、意味を足さずlayer_orderへ変換できません",
                Some(json!(orders)),
            ),
        );
    }
}

fn validate_rectangular_single_sheet(
    frame: &FoldFrame,
    path: &str,
    validation: &mut FoldValidation,
) {
    let (Some(vertices), Some(edges), Some(assignments)) = (
        frame.vertices_coords.as_ref(),
        frame.edges_vertices.as_ref(),
        frame.edges_assignment.as_ref(),
    ) else {
        return;
    };
    if vertices.iter().any(|vertex| vertex.len() != 2) || edges.len() != assignments.len() {
        return;
    }
    let raw_points = vertices
        .iter()
        .map(|vertex| [vertex[0], vertex[1]])
        .collect::<Vec<_>>();
    let minimum = raw_points.iter().fold(
        [f64::INFINITY, f64::INFINITY],
        |[minimum_x, minimum_y], point| [minimum_x.min(point[0]), minimum_y.min(point[1])],
    );
    let maximum = raw_points.iter().fold(
        [f64::NEG_INFINITY, f64::NEG_INFINITY],
        |[maximum_x, maximum_y], point| [maximum_x.max(point[0]), maximum_y.max(point[1])],
    );
    let geometry_scale = (maximum[0] - minimum[0]).max(maximum[1] - minimum[1]);
    if !geometry_scale.is_finite() || geometry_scale <= 0.0 {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                field_path(path, "vertices_coords"),
                "単一紙の外形には正の幅と高さが必要です",
                Some(json!(vertices)),
            ),
        );
        return;
    }
    let points = raw_points
        .iter()
        .map(|point| {
            [
                (point[0] - minimum[0]) / geometry_scale,
                (point[1] - minimum[1]) / geometry_scale,
            ]
        })
        .collect::<Vec<_>>();
    let valid_edges = edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| {
            edge.len() == 2
                && edge[0] < points.len()
                && edge[1] < points.len()
                && edge[0] != edge[1]
        })
        .map(|(index, edge)| (index, edge[0], edge[1]))
        .collect::<Vec<_>>();

    validate_unvertexed_crossings(&points, &valid_edges, path, validation);

    let boundary_edges = valid_edges
        .iter()
        .filter(|&&(index, _, _)| assignments[index] == FoldAssignment::Border)
        .copied()
        .collect::<Vec<_>>();
    let boundary_path = field_path(path, "edges_assignment");
    let Some(cycle) = boundary_cycle(&boundary_edges) else {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                &boundary_path,
                "B edgeは穴のない単一紙の境界cycleを1つだけ作る必要があります",
                Some(json!(
                    boundary_edges
                        .iter()
                        .map(|&(index, _, _)| index)
                        .collect::<Vec<_>>()
                )),
            ),
        );
        return;
    };
    let polygon = cycle.iter().map(|&index| points[index]).collect::<Vec<_>>();
    if polygon_self_intersects(&polygon) {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                &boundary_path,
                "B edgeの境界が自己交差しています",
                None,
            ),
        );
        return;
    }

    let corners = rectangle_corners(&polygon);
    let Some(corners) = corners else {
        record_issue(
            validation,
            issue(
                FoldIssueSeverity::Error,
                FoldIssueCode::UnsupportedGeometry,
                &boundary_path,
                "単一紙の外形は正方形または長方形でなければなりません",
                Some(json!(polygon)),
            ),
        );
        return;
    };

    for (vertex_index, &point) in points.iter().enumerate() {
        if !point_in_rectangle(point, &corners) {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::UnsupportedGeometry,
                    format!("{path}.vertices_coords[{vertex_index}]"),
                    "頂点が長方形の単一紙の外にあります",
                    Some(json!(raw_points[vertex_index])),
                ),
            );
        }
    }
}

fn validate_unvertexed_crossings(
    points: &[[f64; 2]],
    edges: &[(usize, usize, usize)],
    path: &str,
    validation: &mut FoldValidation,
) {
    for (left_position, &(left_index, left_a, left_b)) in edges.iter().enumerate() {
        for &(right_index, right_a, right_b) in &edges[left_position + 1..] {
            if left_a == right_a || left_a == right_b || left_b == right_a || left_b == right_b {
                continue;
            }
            if segments_intersect(
                points[left_a],
                points[left_b],
                points[right_a],
                points[right_b],
            ) {
                record_issue(
                    validation,
                    issue(
                        FoldIssueSeverity::Error,
                        FoldIssueCode::InvalidTopology,
                        format!("{path}.edges_vertices[{right_index}]"),
                        format!(
                            "edge {left_index}との交点に共有vertexがなく、edge topologyを保持できません"
                        ),
                        Some(json!([right_a, right_b])),
                    ),
                );
            }
        }
    }
}

fn boundary_cycle(edges: &[(usize, usize, usize)]) -> Option<Vec<usize>> {
    if edges.len() < 4 {
        return None;
    }
    let mut adjacency = BTreeMap::<usize, Vec<usize>>::new();
    for &(_, first, second) in edges {
        adjacency.entry(first).or_default().push(second);
        adjacency.entry(second).or_default().push(first);
    }
    if adjacency.values().any(|neighbors| neighbors.len() != 2) {
        return None;
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
    }

    let start = *adjacency.keys().next()?;
    let mut cycle = Vec::with_capacity(adjacency.len());
    let mut previous = None;
    let mut current = start;
    loop {
        if cycle.contains(&current) {
            return None;
        }
        cycle.push(current);
        let neighbors = adjacency.get(&current)?;
        let next = match previous {
            None => neighbors[0],
            Some(previous) if neighbors[0] == previous => neighbors[1],
            Some(_) => neighbors[0],
        };
        if next == start {
            break;
        }
        previous = Some(current);
        current = next;
    }

    (cycle.len() == adjacency.len() && edges.len() == cycle.len()).then_some(cycle)
}

fn rectangle_corners(polygon: &[[f64; 2]]) -> Option<[[f64; 2]; 4]> {
    if polygon.len() < 4 {
        return None;
    }
    let mut corners = Vec::new();
    for (index, &current) in polygon.iter().enumerate() {
        let previous = polygon[(index + polygon.len() - 1) % polygon.len()];
        let next = polygon[(index + 1) % polygon.len()];
        let incoming = subtract(current, previous);
        let outgoing = subtract(next, current);
        let scale = length(incoming) * length(outgoing);
        if scale <= 0.0 {
            return None;
        }
        if cross(incoming, outgoing).abs() > NORMALIZED_GEOMETRY_EPS * scale {
            corners.push(current);
        }
    }
    let corners: [[f64; 2]; 4] = corners.try_into().ok()?;
    let sides = [
        subtract(corners[1], corners[0]),
        subtract(corners[2], corners[1]),
        subtract(corners[3], corners[2]),
        subtract(corners[0], corners[3]),
    ];
    let lengths = sides.map(length);
    let longest = lengths.into_iter().fold(0.0_f64, f64::max);
    if longest <= 0.0 {
        return None;
    }
    for (index, side) in sides.iter().enumerate() {
        let next = (index + 1) % 4;
        if lengths[index] <= NORMALIZED_GEOMETRY_EPS * longest
            || dot(*side, sides[next]).abs()
                > NORMALIZED_GEOMETRY_EPS * lengths[index] * lengths[next]
        {
            return None;
        }
    }
    if cross(sides[0], sides[2]).abs() > NORMALIZED_GEOMETRY_EPS * lengths[0] * lengths[2]
        || cross(sides[1], sides[3]).abs() > NORMALIZED_GEOMETRY_EPS * lengths[1] * lengths[3]
        || (lengths[0] - lengths[2]).abs() > NORMALIZED_GEOMETRY_EPS * longest
        || (lengths[1] - lengths[3]).abs() > NORMALIZED_GEOMETRY_EPS * longest
    {
        return None;
    }
    Some(corners)
}

fn point_in_rectangle(point: [f64; 2], corners: &[[f64; 2]; 4]) -> bool {
    let horizontal = subtract(corners[1], corners[0]);
    let vertical = subtract(corners[3], corners[0]);
    let width = length(horizontal);
    let height = length(vertical);
    let offset = subtract(point, corners[0]);
    let horizontal_position = dot(offset, horizontal) / width;
    let vertical_position = dot(offset, vertical) / height;
    let tolerance = NORMALIZED_GEOMETRY_EPS * width.max(height);
    horizontal_position >= -tolerance
        && horizontal_position <= width + tolerance
        && vertical_position >= -tolerance
        && vertical_position <= height + tolerance
}

fn polygon_self_intersects(polygon: &[[f64; 2]]) -> bool {
    for (first, &first_point) in polygon.iter().enumerate() {
        let first_next = (first + 1) % polygon.len();
        for (second, &second_point) in polygon.iter().enumerate().skip(first + 1) {
            let second_next = (second + 1) % polygon.len();
            if first == second
                || first == second_next
                || first_next == second
                || first_next == second_next
            {
                continue;
            }
            if segments_intersect(
                first_point,
                polygon[first_next],
                second_point,
                polygon[second_next],
            ) {
                return true;
            }
        }
    }
    false
}

fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let scale = length(subtract(b, a)).max(length(subtract(d, c))).max(1.0);
    let tolerance = NORMALIZED_GEOMETRY_EPS * scale * scale;
    let ab_c = cross(subtract(b, a), subtract(c, a));
    let ab_d = cross(subtract(b, a), subtract(d, a));
    let cd_a = cross(subtract(d, c), subtract(a, c));
    let cd_b = cross(subtract(d, c), subtract(b, c));

    if ((ab_c > tolerance && ab_d < -tolerance) || (ab_c < -tolerance && ab_d > tolerance))
        && ((cd_a > tolerance && cd_b < -tolerance) || (cd_a < -tolerance && cd_b > tolerance))
    {
        return true;
    }

    (ab_c.abs() <= tolerance && point_on_segment(c, a, b, scale))
        || (ab_d.abs() <= tolerance && point_on_segment(d, a, b, scale))
        || (cd_a.abs() <= tolerance && point_on_segment(a, c, d, scale))
        || (cd_b.abs() <= tolerance && point_on_segment(b, c, d, scale))
}

fn point_on_segment(point: [f64; 2], first: [f64; 2], second: [f64; 2], scale: f64) -> bool {
    let tolerance = NORMALIZED_GEOMETRY_EPS * scale;
    point[0] >= first[0].min(second[0]) - tolerance
        && point[0] <= first[0].max(second[0]) + tolerance
        && point[1] >= first[1].min(second[1]) - tolerance
        && point[1] <= first[1].max(second[1]) + tolerance
}

fn validate_stable_step_topology(frames: &[FoldFrame], validation: &mut FoldValidation) {
    let Some(initial) = frames.first() else {
        return;
    };
    for (index, frame) in frames.iter().enumerate().skip(1) {
        let path = effective_frame_path(index);
        if frame.edges_vertices != initial.edges_vertices {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    field_path(&path, "edges_vertices"),
                    "線形step frameの途中でedge topologyを変えることはできません",
                    frame.edges_vertices.as_ref().map(|value| json!(value)),
                ),
            );
        }
        if frame.edges_assignment != initial.edges_assignment {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    field_path(&path, "edges_assignment"),
                    "線形step frameの途中でedge assignmentを変えることはできません",
                    frame.edges_assignment.as_ref().map(|assignments| {
                        Value::Array(
                            assignments
                                .iter()
                                .map(|assignment| Value::String(assignment.code().to_string()))
                                .collect(),
                        )
                    }),
                ),
            );
        }
        if frame.vertices_coords.as_ref().map(Vec::len)
            != initial.vertices_coords.as_ref().map(Vec::len)
        {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    field_path(&path, "vertices_coords"),
                    "線形step frameの途中で頂点数を変えることはできません",
                    frame
                        .vertices_coords
                        .as_ref()
                        .map(|value| json!(value.len())),
                ),
            );
        }
        if frame.faces_vertices.is_some()
            && initial.faces_vertices.is_some()
            && frame.faces_vertices != initial.faces_vertices
        {
            record_issue(
                validation,
                issue(
                    FoldIssueSeverity::Error,
                    FoldIssueCode::InvalidTopology,
                    field_path(&path, "faces_vertices"),
                    "線形step frameの途中でface topologyを変えることはできません",
                    frame.faces_vertices.as_ref().map(|value| json!(value)),
                ),
            );
        }
    }
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

fn record_issue(validation: &mut FoldValidation, issue: FoldIssue) {
    match issue.severity {
        FoldIssueSeverity::Warning => validation.warning(issue),
        FoldIssueSeverity::Error => validation.error(issue),
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

fn effective_frame_path(index: usize) -> String {
    if index == 0 {
        "$".to_string()
    } else {
        format!("$.file_frames[{}]", index - 1)
    }
}

fn field_path(parent: &str, field: &str) -> String {
    format!("{parent}.{field}")
}

fn unknown_field_path(parent: &str, field: &str) -> String {
    let identifier = field.chars().enumerate().all(|(index, character)| {
        character == '_'
            || character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit())
    });
    if identifier && !field.is_empty() {
        field_path(parent, field)
    } else {
        format!("{parent}[{}]", Value::String(field.to_string()))
    }
}

fn index_path(parent: &str, index: usize) -> String {
    format!("{parent}[{index}]")
}

fn canonical_edge(first: usize, second: usize) -> (usize, usize) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
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
