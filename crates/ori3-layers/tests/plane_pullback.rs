use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_layers::{FoldPlane3D, point_in_face, pull_back_plane_to_faces};
use ori3_model::{CreasePattern, Edge, EdgeKind, Face3D, Frame3D, Paper, Vertex};

const STRICT_TOLERANCE: f64 = 1e-9;

#[test]
fn one_rigid_face_is_pulled_back_by_its_own_isometry() {
    let document = ori3_model::Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let faces = extract_faces(&document.cp);
    let frame = frame_from_map(&document.cp, &faces, |point, _| {
        [1.0 + point.x, 2.0, 3.0 + point.y]
    });

    let result = pull_back_plane_to_faces(
        &document.cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [1.25, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        },
    );

    assert!(result.warnings.is_empty(), "警告={:?}", result.warnings);
    assert_eq!(result.faces.len(), 1);
    assert_eq!(result.faces[0].segments.len(), 1);
    assert_segment_close(
        result.faces[0].segments[0],
        [[0.25, 0.0], [0.25, 1.0]],
        STRICT_TOLERANCE,
    );
}

#[test]
fn ninety_degree_pullback_endpoints_stay_inside_their_faces() {
    let (cp, faces, frame) = ninety_degree_two_face_frame();
    let result = horizontal_cut(&cp, &faces, &frame);

    assert!(result.warnings.is_empty(), "警告={:?}", result.warnings);
    assert_eq!(result.faces.len(), 2);
    for face_result in &result.faces {
        let face = faces
            .iter()
            .find(|face| face.id == face_result.face)
            .expect("結果の面が入力にもある");
        assert_eq!(
            face_result.segments.len(),
            1,
            "面{}の結果={:?}",
            face.id,
            face_result.segments
        );
        let segment = face_result.segments[0];
        let midpoint = (DVec2::from(segment[0]) + DVec2::from(segment[1])) * 0.5;
        for point in [segment[0], segment[1], midpoint.to_array()] {
            assert!(
                point_in_face(&cp, face, point),
                "面{}の範囲外に端点または中点があります: {point:?}",
                face.id
            );
        }
    }
}

#[test]
fn ninety_degree_pullbacks_share_the_same_edge_endpoint() {
    let (cp, faces, frame) = ninety_degree_two_face_frame();
    let result = horizontal_cut(&cp, &faces, &frame);

    let shared: Vec<[f64; 2]> = result
        .faces
        .iter()
        .flat_map(|face| face.segments.iter().flat_map(|segment| *segment))
        .filter(|point| (point[0] - 0.5).abs() < STRICT_TOLERANCE)
        .collect();
    assert_eq!(shared.len(), 2, "共有辺上の端点={shared:?}");
    let gap = (DVec2::from(shared[0]) - DVec2::from(shared[1])).length();
    assert!(
        gap < STRICT_TOLERANCE,
        "90°二面の共有辺で端点が裂けています: gap={gap:.17e}, points={shared:?}"
    );
    println!("ninety_degree_shared_edge_gap={gap:.17e}");
    for point in shared {
        let expected = DVec2::new(0.5, 0.375);
        let error = (DVec2::from(point) - expected).length();
        assert!(
            error < STRICT_TOLERANCE,
            "共有端点が明示した位置と違います: error={error:.17e}, point={point:?}"
        );
    }
}

#[test]
fn plane_parallel_to_vertical_face_returns_empty_face_and_warning() {
    let (cp, faces, frame) = ninety_degree_two_face_frame();
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.75, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        },
    );
    let vertical = faces
        .iter()
        .find(|face| face.id == vertical_face_id(&frame))
        .expect("垂直面");
    let output = result
        .faces
        .iter()
        .find(|output| output.face == vertical.id)
        .expect("右面の空結果も返る");

    assert!(output.segments.is_empty());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains(&format!("面 {}", vertical.id))
                && warning.contains("平行")),
        "平行な垂直面の理由を警告する: {:?}",
        result.warnings
    );
}

#[test]
fn plane_coplanar_with_vertical_face_warns_but_keeps_other_boundary_result() {
    let (cp, faces, frame) = ninety_degree_two_face_frame();
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.5, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        },
    );
    let vertical = faces
        .iter()
        .find(|face| face.id == vertical_face_id(&frame))
        .expect("垂直面");
    let horizontal = faces
        .iter()
        .find(|face| face.id != vertical.id)
        .expect("左面");

    assert!(
        result
            .faces
            .iter()
            .find(|output| output.face == vertical.id)
            .expect("右面")
            .segments
            .is_empty()
    );
    assert_eq!(
        result
            .faces
            .iter()
            .find(|output| output.face == horizontal.id)
            .expect("左面")
            .segments
            .len(),
        1,
        "別の面では共有境界が有効な線分になる"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains(&format!("面 {}", vertical.id))
                && warning.contains("同一平面")),
        "共面で一意に決まらない理由を警告する: {:?}",
        result.warnings
    );
}

#[test]
fn missing_frame_face_does_not_hide_other_face_results() {
    let (cp, faces, mut frame) = ninety_degree_two_face_frame();
    let missing_id = frame.faces.pop().expect("2面目").face;
    let result = horizontal_cut(&cp, &faces, &frame);

    assert_eq!(result.faces.len(), faces.len(), "入力した全ての面を返す");
    assert!(
        result
            .faces
            .iter()
            .find(|output| output.face == missing_id)
            .expect("欠落面にも結果行がある")
            .segments
            .is_empty()
    );
    assert!(
        result
            .faces
            .iter()
            .filter(|output| output.face != missing_id)
            .any(|output| !output.segments.is_empty()),
        "姿勢がある面は計算を続ける"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains(&format!("面 {missing_id}"))
                && warning.contains("フレームに無い")),
        "欠落理由={:?}",
        result.warnings
    );
}

#[test]
fn concave_face_keeps_each_disconnected_intersection_interval() {
    let cp = concave_u_pattern();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1);
    let frame = frame_from_map(&cp, &faces, |point, _| [point.x, point.y, 0.0]);
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.0, 2.0, 0.0],
            normal: [0.0, 1.0, 0.0],
        },
    );

    assert!(result.warnings.is_empty(), "警告={:?}", result.warnings);
    assert_eq!(result.faces[0].segments.len(), 2);
    assert_segment_close(
        result.faces[0].segments[0],
        [[0.0, 2.0], [1.0, 2.0]],
        STRICT_TOLERANCE,
    );
    assert_segment_close(
        result.faces[0].segments[1],
        [[2.0, 2.0], [3.0, 2.0]],
        STRICT_TOLERANCE,
    );
}

#[test]
fn invalid_plane_returns_empty_rows_instead_of_an_error() {
    let (cp, faces, frame) = ninety_degree_two_face_frame();
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 0.0],
        },
    );

    assert_eq!(result.faces.len(), faces.len());
    assert!(result.faces.iter().all(|face| face.segments.is_empty()));
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].contains("法線"));
}

#[test]
fn plane_touching_only_one_vertex_returns_empty_face_and_warning() {
    let document = ori3_model::Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let faces = extract_faces(&document.cp);
    let frame = frame_from_map(&document.cp, &faces, |point, _| [point.x, point.y, 0.0]);
    let result = pull_back_plane_to_faces(
        &document.cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.0, 0.0, 0.0],
            normal: [1.0, 1.0, 0.0],
        },
    );

    assert!(result.faces[0].segments.is_empty());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("頂点に接するだけ")),
        "一点接触の理由={:?}",
        result.warnings
    );
}

#[test]
fn pullback_tolerance_is_normalized_to_cp_distance() {
    let (cp, faces, frame) = identity_square_frame();
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [1.5e-9, 0.0, 0.0],
            normal: [0.5, 0.0, 0.75_f64.sqrt()],
        },
    );

    assert!(result.warnings.is_empty(), "警告={:?}", result.warnings);
    assert_eq!(result.faces[0].segments.len(), 1);
    assert_segment_close(
        result.faces[0].segments[0],
        [[1.5e-9, 0.0], [1.5e-9, 1.0]],
        STRICT_TOLERANCE,
    );
    let boundary_error = result.faces[0].segments[0]
        .iter()
        .map(|point| (point[0] - 1.5e-9).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        boundary_error < STRICT_TOLERANCE,
        "未正規化EPSで境界x=0へ吸着してはいけません: error={boundary_error:.17e}"
    );
}

#[test]
fn numerically_ill_conditioned_near_parallel_plane_returns_a_warning() {
    let (cp, faces, frame) = identity_square_frame();
    let tangent_component = 1.1e-9;
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.5, 0.0, 0.0],
            normal: [
                tangent_component,
                0.0,
                (1.0 - tangent_component * tangent_component).sqrt(),
            ],
        },
    );

    assert!(result.faces[0].segments.is_empty());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("数値的に近平行") && warning.contains("推定CP誤差")),
        "近平行で精度を保証できない理由={:?}",
        result.warnings
    );
}

#[test]
fn shallow_well_conditioned_plane_returns_the_correct_segment() {
    let (cp, faces, frame) = identity_square_frame();
    let tangent_component = 1.0e-4;
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.5, 0.0, 0.0],
            normal: [
                tangent_component,
                0.0,
                (1.0 - tangent_component * tangent_component).sqrt(),
            ],
        },
    );

    assert!(result.warnings.is_empty(), "警告={:?}", result.warnings);
    assert_eq!(result.faces[0].segments.len(), 1);
    assert_segment_close(
        result.faces[0].segments[0],
        [[0.5, 0.0], [0.5, 1.0]],
        STRICT_TOLERANCE,
    );
}

#[test]
fn rounded_rigid_frame_near_parallel_returns_warning_instead_of_inaccurate_segment() {
    let (cp, faces, mut frame) = identity_square_frame();
    frame.faces[0].polygon = vec![
        [0.23456789, -0.34567891, 0.45678912],
        [1.096069686284305, -0.7486121881057208, 0.7657542125167662],
        [0.5946415878177722, -1.3279908614103604, 1.4083195515975426],
        [-0.2668602084665327, -0.9250575833046395, 1.0993544590807764],
    ];
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.6653187881421525, -0.5471455490528605, 0.611271666258383],
            normal: [
                -0.07990317212547023,
                -0.7084949731245674,
                -0.7011778348903408,
            ],
        },
    );

    assert!(
        result.faces[0].segments.is_empty(),
        "丸め誤差で真値x=0.5から1e-9以上ずれた線分を返してはいけない: {:?}",
        result.faces[0].segments
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("数値的に近平行") && warning.contains("推定CP誤差")),
        "精度を保証できない理由を数値付きで返す: {:?}",
        result.warnings
    );
}

#[test]
fn plane_normal_scale_does_not_change_the_pullback() {
    let (cp, faces, frame) = identity_square_frame();
    for scale in [1e-300, 1.0, 1e300] {
        let result = pull_back_plane_to_faces(
            &cp,
            &faces,
            &frame,
            FoldPlane3D {
                origin: [0.25, 0.0, 0.0],
                normal: [scale, 0.0, 0.0],
            },
        );
        assert!(
            result.warnings.is_empty(),
            "scale={scale:.1e}, warnings={:?}",
            result.warnings
        );
        assert_eq!(result.faces[0].segments.len(), 1, "scale={scale:.1e}");
        assert_segment_close(
            result.faces[0].segments[0],
            [[0.25, 0.0], [0.25, 1.0]],
            STRICT_TOLERANCE,
        );
    }
}

#[test]
fn finite_nonrigid_face_continues_with_a_warning() {
    let (cp, faces, mut frame) = identity_square_frame();
    frame.faces[0].polygon[0][2] = 1e-6;
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.25, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        },
    );

    assert!(
        !result.faces[0].segments.is_empty(),
        "有限なbest-effort変換では結果を返す"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("計算を続けます")),
        "非剛体残差を数値付きで警告する: {:?}",
        result.warnings
    );
}

#[test]
fn nonfinite_in_plane_gradient_returns_empty_face_and_warning() {
    let (cp, faces, _) = identity_square_frame();
    let frame = frame_from_map(&cp, &faces, |point, _| {
        if (point.x - 1.0).abs() < STRICT_TOLERANCE && point.y.abs() < STRICT_TOLERANCE {
            [f64::MAX, 0.0, 0.0]
        } else {
            [0.0, 0.0, 0.0]
        }
    });
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        },
    );

    assert!(result.faces[0].segments.is_empty());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("面内勾配が有限になりません")),
        "有限だが極端に壊れた面もpanic/NaN線分にせず理由を返す: {:?}",
        result.warnings
    );
}

#[test]
fn moving_plane_origin_along_the_plane_does_not_change_the_pullback() {
    let (cp, faces, frame) = identity_square_frame();
    let baseline = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.5, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        },
    );
    let translated = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.5, 1e300, 0.0],
            normal: [1.0, 0.0, 0.0],
        },
    );

    assert!(
        translated.warnings.is_empty(),
        "平面内方向の原点移動は警告条件を変えない: {:?}",
        translated.warnings
    );
    assert_eq!(baseline.faces[0].segments.len(), 1);
    assert_eq!(translated.faces[0].segments.len(), 1);
    assert_segment_close(
        translated.faces[0].segments[0],
        baseline.faces[0].segments[0],
        STRICT_TOLERANCE,
    );
}

#[test]
fn seam_gap_is_warned_and_shared_endpoint_is_reconciled() {
    let (cp, faces, mut frame) = ninety_degree_two_face_frame();
    let shifted_face = vertical_face_id(&frame);
    for point in &mut frame
        .faces
        .iter_mut()
        .find(|face| face.face == shifted_face)
        .expect("垂直面")
        .polygon
    {
        point[1] += 2e-9;
    }

    let result = horizontal_cut(&cp, &faces, &frame);
    let shared: Vec<[f64; 2]> = result
        .faces
        .iter()
        .flat_map(|face| face.segments.iter().flat_map(|segment| *segment))
        .filter(|point| (point[0] - 0.5).abs() < STRICT_TOLERANCE)
        .collect();
    assert_eq!(shared.len(), 2, "共有端点={shared:?}");
    let gap = (DVec2::from(shared[0]) - DVec2::from(shared[1])).length();
    assert!(
        gap < STRICT_TOLERANCE,
        "補正後の共有端点差={gap:.17e}, points={shared:?}"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("共有辺") && warning.contains("そろえ")),
        "seam差と補正を警告する: {:?}",
        result.warnings
    );
}

#[test]
fn mismatched_shared_edge_endpoint_counts_return_a_warning() {
    let (cp, faces, _) = ninety_degree_two_face_frame();
    let mut frame = frame_from_map(&cp, &faces, |point, _| [point.x, point.y, 0.0]);
    let right_face = faces
        .iter()
        .find(|face| {
            face.vertices.iter().all(|vertex_id| {
                cp.vertices
                    .iter()
                    .find(|vertex| vertex.id == *vertex_id)
                    .expect("面頂点")
                    .pos[0]
                    >= 0.5 - STRICT_TOLERANCE
            })
        })
        .expect("右面")
        .id;
    for point in &mut frame
        .faces
        .iter_mut()
        .find(|face| face.face == right_face)
        .expect("右面の3D姿勢")
        .polygon
    {
        point[2] += 5e-10;
    }
    let tangent_component = 1e-4;
    let result = pull_back_plane_to_faces(
        &cp,
        &faces,
        &frame,
        FoldPlane3D {
            origin: [0.5, 0.0, 0.0],
            normal: [
                tangent_component,
                0.0,
                (1.0 - tangent_component * tangent_component).sqrt(),
            ],
        },
    );

    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("端点数") && warning.contains("一致しません")),
        "共有辺を片面だけが切る場合も理由を返す: {:?}",
        result.warnings
    );
}

fn horizontal_cut(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
) -> ori3_layers::PlanePullbackResult {
    pull_back_plane_to_faces(
        cp,
        faces,
        frame,
        FoldPlane3D {
            origin: [0.0, 0.375, 0.0],
            normal: [0.0, 1.0, 0.0],
        },
    )
}

fn identity_square_frame() -> (CreasePattern, Vec<Face>, Frame3D) {
    let document = ori3_model::Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let faces = extract_faces(&document.cp);
    let frame = frame_from_map(&document.cp, &faces, |point, _| [point.x, point.y, 0.0]);
    (document.cp, faces, frame)
}

fn ninety_degree_two_face_frame() -> (CreasePattern, Vec<Face>, Frame3D) {
    let mut document = ori3_model::Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let added =
        ori3_cp::insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
    let hinge = *added.first().expect("中央の折り辺");
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 2);
    // 角度は計算結果から逆算せず、検査入力として90°を明示する。
    let angles = HashMap::from([(hinge, 90.0)]);
    let folded = ori3_rigid::propagate(&document.cp, &faces, &angles);
    let frame = ori3_rigid::to_frame3d(&document.cp, &faces, &folded);
    assert!(
        frame.warnings.is_empty(),
        "90°姿勢の警告={:?}",
        frame.warnings
    );
    (document.cp, faces, frame)
}

fn frame_from_map(
    cp: &CreasePattern,
    faces: &[Face],
    map: impl Fn(DVec2, &Face) -> [f64; 3],
) -> Frame3D {
    Frame3D {
        faces: faces
            .iter()
            .map(|face| Face3D {
                face: face.id,
                polygon: face
                    .vertices
                    .iter()
                    .map(|vertex_id| {
                        let point = DVec2::from(
                            cp.vertices
                                .iter()
                                .find(|vertex| vertex.id == *vertex_id)
                                .expect("面頂点")
                                .pos,
                        );
                        map(point, face)
                    })
                    .collect(),
                layer: 0,
                mirrored: false,
            })
            .collect(),
        warnings: Vec::new(),
    }
}

fn vertical_face_id(frame: &Frame3D) -> u32 {
    frame
        .faces
        .iter()
        .find(|face| {
            let (minimum, maximum) = face.polygon.iter().map(|point| point[2]).fold(
                (f64::INFINITY, f64::NEG_INFINITY),
                |(minimum, maximum), z| (minimum.min(z), maximum.max(z)),
            );
            maximum - minimum > 0.25
        })
        .expect("90°で立った面")
        .face
}

fn concave_u_pattern() -> CreasePattern {
    let positions = [
        [0.0, 0.0],
        [3.0, 0.0],
        [3.0, 3.0],
        [2.0, 3.0],
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 3.0],
        [0.0, 3.0],
    ];
    let vertices: Vec<Vertex> = positions
        .into_iter()
        .enumerate()
        .map(|(index, pos)| Vertex {
            id: u32::try_from(index).expect("頂点ID"),
            pos,
        })
        .collect();
    let edges: Vec<Edge> = (0..vertices.len())
        .map(|index| Edge {
            id: u32::try_from(index).expect("辺ID"),
            v0: u32::try_from(index).expect("始点ID"),
            v1: u32::try_from((index + 1) % vertices.len()).expect("終点ID"),
            kind: EdgeKind::Border,
        })
        .collect();
    CreasePattern {
        next_vertex_id: u32::try_from(vertices.len()).expect("次の頂点ID"),
        next_edge_id: u32::try_from(edges.len()).expect("次の辺ID"),
        vertices,
        edges,
    }
}

fn assert_segment_close(actual: [[f64; 2]; 2], expected: [[f64; 2]; 2], tolerance: f64) {
    let direct = (DVec2::from(actual[0]) - DVec2::from(expected[0]))
        .length()
        .max((DVec2::from(actual[1]) - DVec2::from(expected[1])).length());
    let reversed = (DVec2::from(actual[0]) - DVec2::from(expected[1]))
        .length()
        .max((DVec2::from(actual[1]) - DVec2::from(expected[0])).length());
    let error = direct.min(reversed);
    assert!(
        error < tolerance,
        "線分端点差={error:.17e}, actual={actual:?}, expected={expected:?}, tolerance={tolerance:.1e}"
    );
}
