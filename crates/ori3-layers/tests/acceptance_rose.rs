//! 川崎敏和「1分ローズ」受け入れテスト。
//!
//! 型紙の1/4実測値だけを定数として持ち、残りは紙の中心まわりの90°回転で
//! 作る。書籍の複数の小図で構成されるねじり畳み・つぶし折りは、1つの技法の
//! 中間フレームとして検査する。これにより、操作列へ意味のない手順を水増しせず、
//! 各小図の時点で紙がつながっていることを確認できる。

use std::collections::{HashMap, HashSet};

use glam::{DVec2, DVec3};
use ori3_cp::{Face, arc_polyline, extract_faces, insert_polyline, insert_rulings, validate};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{
    FlatMotionInput, FlatState, FoldDirection, LayerTurn, MotionPart, MotionTransform, flat_motion,
    flat_state_at, replay, resolve_driver_edges, squash, twist,
};
use ori3_model::{CreasePattern, Document, DriverLine, EdgeKind, FaceId, Frame3D, Paper, VertexId};
use ori3_rigid::max_seam_gap;

// Cargo.tomlは並列作業の共有ファイルなので変更しない。ori3-softの変形核は
// 独立モジュールとしても使える設計にし、通常はori3_softから同じAPIを公開する。
#[path = "../../ori3-soft/src/cup.rs"]
mod soft_cup;
#[path = "../../ori3-soft/src/curl.rs"]
mod soft_curl;
#[path = "../../ori3-soft/src/symmetry.rs"]
mod soft_symmetry;

#[cfg(test)]
const FIXTURE_011: &str = include_str!("fixtures/rose-011.ori3");
#[cfg(test)]
const FIXTURE_021: &str = include_str!("fixtures/rose-021.ori3");
#[cfg(test)]
const FIXTURE_029: &str = include_str!("fixtures/rose-029.ori3");

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<ori3_layers::FoldThroughResult, String>;

const CENTER: [f64; 2] = [0.5, 0.5];
const CURVE_SEGMENTS: u32 = 3;

#[derive(Clone, Copy)]
struct QuarterLine {
    kind: EdgeKind,
    points: &'static [[f64; 2]],
    curved: bool,
}

// verification/rose-cp.json の north-west 1/4。テストはverification/へ依存せず、
// この6本から残り18本を回転生成する。
const QUARTER: [QuarterLine; 6] = [
    QuarterLine {
        kind: EdgeKind::Valley,
        points: &[[0.239, 1.0], [0.286, 0.822]],
        curved: false,
    },
    QuarterLine {
        kind: EdgeKind::Mountain,
        points: &[[0.421, 1.0], [0.286, 0.822]],
        curved: false,
    },
    QuarterLine {
        kind: EdgeKind::Valley,
        points: &[[0.0, 0.730], [0.286, 0.822]],
        curved: false,
    },
    QuarterLine {
        kind: EdgeKind::Valley,
        points: &[[0.286, 0.822], [0.437, 0.700], [0.500, 0.579]],
        curved: true,
    },
    QuarterLine {
        kind: EdgeKind::Valley,
        points: &[[0.500, 0.579], [0.421, 0.500]],
        curved: false,
    },
    QuarterLine {
        kind: EdgeKind::Mountain,
        points: &[[0.0, 0.579], [0.500, 0.579]],
        curved: false,
    },
];

fn rotate_quarter(mut p: [f64; 2], turns: usize) -> [f64; 2] {
    for _ in 0..turns % 4 {
        p = [1.0 - p[1], p[0]];
    }
    p
}

fn rotate_vec(v: DVec2, radians: f64) -> DVec2 {
    let (s, c) = radians.sin_cos();
    DVec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// 型紙24本を作る。曲線は3分割の折れ線とし、各内点からrulingを入れる。
fn rose_pattern() -> (Document, [Vec<[f64; 2]>; 4]) {
    let mut doc = Document::new(Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    });

    // rulingが全て同じ既存線で止まるよう、先に直線20本を揃える。
    for turn in 0..4 {
        for line in QUARTER.iter().filter(|line| !line.curved) {
            let points = line
                .points
                .iter()
                .map(|&p| rotate_quarter(p, turn))
                .collect::<Vec<_>>();
            insert_polyline(&mut doc.cp, &points, line.kind);
        }
    }

    let curves = std::array::from_fn(|turn| {
        let spec = QUARTER.iter().find(|line| line.curved).unwrap();
        let p = spec
            .points
            .iter()
            .map(|&point| rotate_quarter(point, turn))
            .collect::<Vec<_>>();
        arc_polyline(p[0], p[1], p[2], 0.005, Some(CURVE_SEGMENTS))
    });
    for curve in &curves {
        insert_polyline(&mut doc.cp, curve, EdgeKind::Valley);
    }
    for curve in &curves {
        insert_rulings(&mut doc.cp, curve, [1.0, 1.0], EdgeKind::Valley);
    }

    let warnings = validate(&doc.cp);
    assert!(warnings.is_empty(), "ローズ型紙の警告: {warnings:?}");
    assert_eq!(curves.len(), 4, "曲線は4回回転対称の4本");
    assert!(
        curves
            .iter()
            .all(|curve| curve.len() == CURVE_SEGMENTS as usize + 1),
        "各曲線は同じ分割数"
    );
    (doc, curves)
}

fn state_of(doc: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&doc.cp);
    let (state, warnings) =
        flat_state_at(doc, &faces, doc.sequence.len()).expect("ローズを平らに畳める");
    assert!(warnings.is_empty(), "平坦再生の警告: {warnings:?}");
    assert_eq!(state.order.len(), faces.len(), "層順序から面が失われない");
    let unique = state.order.iter().copied().collect::<HashSet<_>>();
    assert_eq!(unique.len(), faces.len(), "層順序に重複がない");
    (faces, state)
}

fn apply_technique(
    doc: &mut Document,
    technique: Technique,
    input: TechniqueInput,
    note: &str,
) -> FlatState {
    let (faces, state) = state_of(doc);
    let before_faces = faces.len();
    let mut cp = doc.cp.clone();
    let result = technique(&mut cp, &faces, &state, &input).expect(note);
    assert!(
        result.warnings.is_empty(),
        "{note}: 警告 {:?}",
        result.warnings
    );
    let after_faces = extract_faces(&cp);
    assert!(after_faces.len() >= before_faces, "{note}: 面が失われた");
    let ids = after_faces
        .iter()
        .map(|face| face.id)
        .collect::<HashSet<_>>();
    let order = result.state.order.iter().copied().collect::<HashSet<_>>();
    assert_eq!(ids, order, "{note}: 面と層順序が一致しない");

    let mut step = result.step;
    step.id = u32::try_from(doc.sequence.len()).unwrap();
    step.note = note.to_string();
    doc.cp = cp;
    doc.sequence.push(step);
    result.state
}

fn apply_motion(doc: &mut Document, parts: Vec<MotionPart>, note: &str) -> FlatState {
    let (faces, state) = state_of(doc);
    let before_faces = faces.len();
    let mut cp = doc.cp.clone();
    let result = flat_motion(
        &mut cp,
        &faces,
        &state,
        &FlatMotionInput {
            parts,
            kind: ori3_model::TechniqueKind::Simple,
        },
    )
    .expect(note);
    assert!(
        result.warnings.is_empty(),
        "{note}: 警告 {:?}",
        result.warnings
    );
    let after_faces = extract_faces(&cp);
    assert!(after_faces.len() >= before_faces, "{note}: 面が失われた");
    let ids = after_faces
        .iter()
        .map(|face| face.id)
        .collect::<HashSet<_>>();
    let order = result.state.order.iter().copied().collect::<HashSet<_>>();
    assert_eq!(ids, order, "{note}: 面と層順序が一致しない");
    let mut step = result.step;
    step.id = u32::try_from(doc.sequence.len()).unwrap();
    step.note = note.to_string();
    doc.cp = cp;
    doc.sequence.push(step);
    result.state
}

/// `last_from..=last_to`を、最後に追加した複合技法の途中経過へ対応させて検査する。
fn verify_book_frames(doc: &Document, last_from: usize, last_to: usize) -> f64 {
    let cp_warnings = validate(&doc.cp);
    assert!(
        cp_warnings.is_empty(),
        "手順{last_from}〜{last_to}: 型紙警告 {cp_warnings:?}"
    );
    let faces = extract_faces(&doc.cp);
    let expected = faces
        .iter()
        .map(|face| (face.id, face.vertices.len()))
        .collect::<HashMap<_, _>>();
    let count = last_to - last_from + 1;
    let mut max_gap = 0.0_f64;
    for book_step in last_from..=last_to {
        let t = (book_step - last_from + 1) as f64 / count as f64;
        let result = replay(doc, doc.sequence.len(), t);
        assert!(
            result.warnings.is_empty(),
            "手順{book_step}: 再生警告 {:?}",
            result.warnings
        );
        assert!(
            result.skipped.is_empty(),
            "手順{book_step}: 再生スキップ {:?}",
            result.skipped
        );
        assert_eq!(
            result.frame.faces.len(),
            faces.len(),
            "手順{book_step}: 面が失われた"
        );
        assert!(
            result.frame.warnings.is_empty(),
            "手順{book_step}: 3D警告 {:?}",
            result.frame.warnings
        );
        let actual = result
            .frame
            .faces
            .iter()
            .map(|face| (face.face, face.polygon.len()))
            .collect::<HashMap<_, _>>();
        assert_eq!(
            actual, expected,
            "手順{book_step}: 面または面頂点が失われた"
        );
        assert!(
            result
                .frame
                .faces
                .iter()
                .flat_map(|face| &face.polygon)
                .flatten()
                .all(|value| value.is_finite()),
            "手順{book_step}: 非有限な3D座標"
        );
        let gap = max_seam_gap(&doc.cp, &faces, &result.frame);
        assert!(gap < 1e-6, "手順{book_step}: 裂け {gap:.9}");
        max_gap = max_gap.max(gap);
        if book_step % 5 == 0 {
            println!("手順{book_step}まで通過。max_seam_gap={max_gap:.3e}");
        }
    }
    max_gap
}

/// 曲線の端の直線谷折りにも接する、つぶす側のfacetを決定的に選ぶ。
fn curve_flap(doc: &Document, curve: &[[f64; 2]]) -> (FaceId, [[f64; 2]; 2], [f64; 2]) {
    let (faces, state) = state_of(doc);
    let positions = doc
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<HashMap<_, _>>();
    let anchor = curve[0];
    let inner = curve[1];
    // 先のつぶし折りで区間が再分割されても、座標線で全断片を解決する。
    let seed_edges = resolve_driver_edges(
        &doc.cp,
        &DriverLine {
            a: anchor,
            b: inner,
            target_angle_deg: 0.0,
        },
    );
    assert!(!seed_edges.is_empty(), "曲線の先頭区間");
    let face = faces
        .iter()
        .filter(|face| face.edges.iter().any(|edge| seed_edges.contains(edge)))
        // 曲線の外側(紙の中心から遠い側)が、書籍でつぶす花びらfacet。
        .max_by(|a, b| {
            let radial = |face: &Face| {
                face.vertices
                    .iter()
                    .map(|vertex| DVec2::from(positions[vertex]))
                    .sum::<DVec2>()
                    .distance(DVec2::splat(0.5) * face.vertices.len() as f64)
                    / face.vertices.len() as f64
            };
            radial(a).total_cmp(&radial(b))
        })
        .expect("曲線外側のfacet");
    let placement = state.placements[&face.id];
    let mapped = |p: [f64; 2]| {
        let q = placement.apply(DVec2::from(p));
        [q.x, q.y]
    };
    let line = [mapped(inner), mapped(anchor)];
    // 背の自由端を、facet内部の代表点の方向へ開く。
    let center = face
        .vertices
        .iter()
        .map(|vertex| DVec2::from(mapped(positions[vertex])))
        .sum::<DVec2>()
        / face.vertices.len() as f64;
    (face.id, line, center.into())
}

fn vertex_at(cp: &CreasePattern, point: [f64; 2]) -> u32 {
    cp.vertices
        .iter()
        .find(|vertex| (DVec2::from(vertex.pos) - DVec2::from(point)).length() < 1e-7)
        .unwrap_or_else(|| panic!("型紙の頂点 {point:?}"))
        .id
}

fn faces_at_cp_points(doc: &Document, points: &[[f64; 2]]) -> Vec<FaceId> {
    let faces = extract_faces(&doc.cp);
    let vertices = points
        .iter()
        .map(|&point| vertex_at(&doc.cp, point))
        .collect::<HashSet<_>>();
    faces
        .iter()
        .filter(|face| face.vertices.iter().any(|vertex| vertices.contains(vertex)))
        .map(|face| face.id)
        .collect()
}

fn faces_on_curve(doc: &Document, curve: &[[f64; 2]]) -> Vec<FaceId> {
    let faces = extract_faces(&doc.cp);
    let mut owners: HashMap<u32, Vec<FaceId>> = HashMap::new();
    for face in &faces {
        for &edge in &face.edges {
            owners.entry(edge).or_default().push(face.id);
        }
    }
    let mut found = HashSet::new();
    for segment in curve.windows(2) {
        let ids = resolve_driver_edges(
            &doc.cp,
            &DriverLine {
                a: segment[0],
                b: segment[1],
                target_angle_deg: 0.0,
            },
        );
        for edge in ids {
            found.extend(owners.get(&edge).into_iter().flatten().copied());
        }
    }
    found.into_iter().collect()
}

/// 手順22: 最初の花びら1・2を花びら4の直下へ戻し、8枚の輪を閉じる。
fn close_petal_ring(doc: &mut Document, curves: &[Vec<[f64; 2]>; 4]) {
    let (_, state) = state_of(doc);
    let sw_tips = [
        rotate_quarter([0.239, 1.0], 1),
        rotate_quarter([0.421, 1.0], 1),
    ];
    let tip_faces = faces_at_cp_points(doc, &sw_tips);
    let mut first_petals = state
        .order
        .iter()
        .copied()
        .filter(|face| tip_faces.contains(face))
        .collect::<Vec<_>>();
    // 境界2点の子孫のうち外側4層が、書籍の最初の花びら1・2。
    if first_petals.len() > 4 {
        first_petals = first_petals[first_petals.len() - 4..].to_vec();
    }
    assert_eq!(
        first_petals.len(),
        4,
        "花びら1・2は連続4層: {first_petals:?}"
    );

    let nw = faces_on_curve(doc, &curves[0]);
    let anchor = state
        .order
        .iter()
        .rev()
        .copied()
        .find(|face| nw.contains(face) && !first_petals.contains(face))
        .expect("花びら4の基準層");
    apply_motion(
        doc,
        vec![MotionPart {
            layers: first_petals,
            region: Vec::new(),
            transform: MotionTransform::Stay,
            turn: LayerTurn::Beside {
                anchor,
                direction: FoldDirection::Down,
            },
            reverse_layers: Some(false),
        }],
        "川崎1分ローズ 手順22: 花びら1・2を花びら4の下へ戻して閉環",
    );
    verify_book_frames(doc, 22, 22);
}

/// 剛体再生の面ごとの頂点コピーを、CP頂点IDごとの共有座標へ集約する。
/// 同じ共有配列から全コピーを書き戻すため、軟体変形後にも継ぎ目は裂けない。
fn shared_frame(doc: &Document) -> (Vec<Face>, Frame3D, Vec<[f64; 3]>, Vec<VertexId>) {
    let faces = extract_faces(&doc.cp);
    let replayed = replay(doc, doc.sequence.len(), 1.0);
    assert!(
        replayed.warnings.is_empty(),
        "軟体変形前の再生警告: {:?}",
        replayed.warnings
    );
    assert!(
        replayed.skipped.is_empty(),
        "軟体変形前の再生スキップ: {:?}",
        replayed.skipped
    );
    assert!(
        replayed.frame.warnings.is_empty(),
        "軟体変形前の3D警告: {:?}",
        replayed.frame.warnings
    );

    let frame_faces = replayed
        .frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    assert_eq!(frame_faces.len(), faces.len(), "軟体変形前に面が失われた");
    let mut positions = vec![[f64::NAN; 3]; doc.cp.next_vertex_id as usize];
    for face in &faces {
        let face3d = frame_faces[&face.id];
        assert_eq!(
            face3d.polygon.len(),
            face.vertices.len(),
            "面{}の頂点数",
            face.id
        );
        for (&vertex, &point) in face.vertices.iter().zip(&face3d.polygon) {
            assert!(
                point.into_iter().all(f64::is_finite),
                "頂点{vertex}が非有限"
            );
            let slot = &mut positions[vertex as usize];
            if slot[0].is_nan() {
                *slot = point;
            } else {
                let gap = (DVec3::from(*slot) - DVec3::from(point)).length();
                assert!(gap < 1e-6, "共有頂点{vertex}が変形前から裂けている: {gap}");
            }
        }
    }
    let vertices = doc
        .cp
        .vertices
        .iter()
        .map(|vertex| vertex.id)
        .collect::<Vec<_>>();
    assert!(
        vertices
            .iter()
            .all(|vertex| positions[*vertex as usize].into_iter().all(f64::is_finite)),
        "全CP頂点が3D面に現れる"
    );
    (faces, replayed.frame, positions, vertices)
}

fn write_shared_positions(faces: &[Face], frame: &mut Frame3D, positions: &[[f64; 3]]) {
    let by_id = faces
        .iter()
        .map(|face| (face.id, face))
        .collect::<HashMap<_, _>>();
    for face3d in &mut frame.faces {
        let face = by_id[&face3d.face];
        assert_eq!(face.vertices.len(), face3d.polygon.len());
        for (&vertex, point) in face.vertices.iter().zip(&mut face3d.polygon) {
            *point = positions[vertex as usize];
        }
    }
}

fn audit_soft_frame(doc: &Document, faces: &[Face], frame: &Frame3D, book_step: usize) -> f64 {
    let cp_warnings = validate(&doc.cp);
    assert!(
        cp_warnings.is_empty(),
        "手順{book_step}: 型紙警告 {cp_warnings:?}"
    );
    assert!(
        frame.warnings.is_empty(),
        "手順{book_step}: 3D警告 {:?}",
        frame.warnings
    );
    let expected = faces
        .iter()
        .map(|face| (face.id, face.vertices.len()))
        .collect::<HashMap<_, _>>();
    let actual = frame
        .faces
        .iter()
        .map(|face| (face.face, face.polygon.len()))
        .collect::<HashMap<_, _>>();
    assert_eq!(
        actual, expected,
        "手順{book_step}: 面または面頂点が失われた"
    );
    assert!(
        frame
            .faces
            .iter()
            .flat_map(|face| &face.polygon)
            .flatten()
            .all(|value| value.is_finite()),
        "手順{book_step}: 非有限な3D座標"
    );
    let gap = max_seam_gap(&doc.cp, faces, frame);
    assert!(gap < 1e-6, "手順{book_step}: 裂け {gap:.9}");
    if book_step.is_multiple_of(5) {
        println!("手順{book_step}まで通過。max_seam_gap={gap:.3e}");
    }
    gap
}

fn mean_position(positions: &[[f64; 3]], vertices: &[VertexId]) -> DVec3 {
    vertices
        .iter()
        .map(|vertex| DVec3::from(positions[*vertex as usize]))
        .sum::<DVec3>()
        / vertices.len() as f64
}

#[allow(dead_code)]
#[derive(Clone)]
struct CompletedRose {
    checkpoint11: Document,
    checkpoint21: Document,
    checkpoint29: Document,
    frame29: Frame3D,
    positions29: Vec<[f64; 3]>,
    gaps: Vec<f64>,
    center: DVec3,
    tips: Vec<VertexId>,
    scale: f64,
}

/// 正確座標の型紙を、手順21(8枚の花びら)まで畳む。
fn rose_through_21() -> (Document, Document, [f64; 2]) {
    let (mut doc, curves) = rose_pattern();

    let polygon = vec![
        [0.500, 0.579],
        [0.579, 0.500],
        [0.500, 0.421],
        [0.421, 0.500],
    ];
    let center = DVec2::from(CENTER);
    let mid = (DVec2::from(polygon[0]) + DVec2::from(polygon[1])) * 0.5;
    let target = center + rotate_vec(mid - center, 30.0_f64.to_radians()) * 2.0;
    apply_technique(
        &mut doc,
        twist,
        TechniqueInput {
            flap: Vec::new(),
            line: [[0.0, 0.0], [1.0, 0.0]],
            reference_point: target.into(),
            open_to_back: Some(false),
            polygon: Some(polygon),
            center: Some(CENTER),
        },
        "川崎1分ローズ 手順4〜11: ねじり畳み",
    );
    let gap11 = verify_book_frames(&doc, 4, 11);
    let checkpoint11 = doc.clone();

    // 書籍の手順15・18・20で、曲線の背を開いて花びら4・6・8まで作る。
    // 各技法の準備動作も中間フレームとして12〜21へ対応させる。
    for (turn, range, note) in [
        (0usize, (12usize, 17usize), "手順12〜17: 花びら1〜4"),
        (3usize, (18usize, 18usize), "手順18: 花びら5・6"),
        (2usize, (19usize, 20usize), "手順19〜20: 花びら7・8"),
    ] {
        let (flap, line, reference_point) = curve_flap(&doc, &curves[turn]);
        apply_technique(
            &mut doc,
            squash,
            TechniqueInput {
                flap: vec![flap],
                line,
                reference_point,
                open_to_back: Some(false),
                polygon: None,
                center: None,
            },
            note,
        );
        verify_book_frames(&doc, range.0, range.1);
    }
    let gap21 = verify_book_frames(&doc, 21, 21);
    (checkpoint11, doc, [gap11, gap21])
}

/// 手順23〜29の底の折り込み・円筒化・丸み・8枚のカールを、
/// 共有頂点を保ったまま施す。
fn shape_rose_to_29(
    mut doc: Document,
    checkpoint11: Document,
    checkpoint21: Document,
    mut gaps: Vec<f64>,
) -> CompletedRose {
    let (faces, mut frame, mut positions, vertices) = shared_frame(&doc);
    let center_vertices = (0..4)
        .map(|turn| vertex_at(&doc.cp, rotate_quarter([0.5, 0.579], turn)))
        .collect::<Vec<_>>();
    let normal = DVec3::Z;
    let mut center = mean_position(&positions, &center_vertices);
    let planar_radius = |point: DVec3, origin: DVec3| {
        let delta = point - origin;
        (delta - normal * delta.dot(normal)).length()
    };
    let scale = vertices
        .iter()
        .map(|vertex| planar_radius(DVec3::from(positions[*vertex as usize]), center))
        .fold(0.0_f64, f64::max);
    assert!(
        scale > 0.1 && scale.is_finite(),
        "畳んだ花の有効半径: {scale}"
    );

    // 手順23・24: 中央菱形の4つのカドを順に内側へ巻き込み、底を閉じる。
    // 折り畳まれた多層の閉ループを平面反転すると側辺が裂けるため、
    // 共有頂点のまま軸回りに曲げ、半径と高さの両方が内側へ移ることを確かめる。
    let tuck_center = center;
    for (turns, book_step) in [(0usize..1, 23usize), (1usize..4, 24usize)] {
        for turn in turns {
            let corner = center_vertices[turn];
            let before = DVec3::from(positions[corner as usize]);
            let toward_corner = (before - tuck_center).reject_from(normal);
            let distance = toward_corner.length();
            assert!(distance > scale * 0.01, "底のカド{}", turn + 1);
            let report = soft_curl::curl_vertices(
                &mut positions,
                &[corner],
                &soft_curl::CurlSettings {
                    axis_origin: tuck_center.to_array(),
                    axis_direction: normal.cross(toward_corner).to_array(),
                    toward_tip: toward_corner.to_array(),
                    radius: distance * 0.55,
                    angle_deg: 68.0,
                },
            )
            .unwrap_or_else(|error| panic!("底のカド{}の折り込み: {error}", turn + 1));
            let after = DVec3::from(positions[corner as usize]);
            assert_eq!(report.moved_vertices, 1, "底のカド{}を動かす", turn + 1);
            assert!(
                planar_radius(after, tuck_center) < planar_radius(before, tuck_center) * 0.90,
                "底のカド{}が内側へ入る",
                turn + 1
            );
            assert!(
                after.z < before.z - scale * 0.005,
                "底のカド{}が花の内側へ入る",
                turn + 1
            );
        }
        write_shared_positions(&faces, &mut frame, &positions);
        gaps.push(audit_soft_frame(&doc, &faces, &frame, book_step));
    }
    center = mean_position(&positions, &center_vertices);

    // 手順25: 中心を持ち上げ、遷移帯で底を円筒状の壁へつなぐ。
    let cup = soft_cup::radial_cup_vertices(
        &mut positions,
        &vertices,
        &soft_cup::RadialCupSettings {
            center: center.to_array(),
            normal: normal.to_array(),
            inner_radius: scale * 0.15,
            outer_radius: scale * 0.62,
            height: scale * 0.20,
        },
    )
    .expect("手順25の円筒化");
    assert!(
        cup.moved_vertices > 0 && cup.max_displacement > scale * 0.05,
        "底を円筒化できた: {cup:?}"
    );
    write_shared_positions(&faces, &mut frame, &positions);
    gaps.push(audit_soft_frame(&doc, &faces, &frame, 25));

    // 手順26: 太い山折りを、より広いC2連続の丸みで滑らかにする。
    center = mean_position(&positions, &center_vertices);
    let smooth = soft_cup::radial_cup_vertices(
        &mut positions,
        &vertices,
        &soft_cup::RadialCupSettings {
            center: center.to_array(),
            normal: normal.to_array(),
            inner_radius: scale * 0.08,
            outer_radius: scale * 0.76,
            height: scale * 0.025,
        },
    )
    .expect("手順26の丸み付け");
    assert!(
        smooth.moved_vertices > 0 && smooth.max_displacement > 0.0,
        "山折り線を滑らかにできた: {smooth:?}"
    );
    write_shared_positions(&faces, &mut frame, &positions);
    gaps.push(audit_soft_frame(&doc, &faces, &frame, 26));

    // 型紙上の境界2点×4回転を8枚の先端とする。各CP頂点は最寄りの先端へ
    // 一意に割り当て、同じ頂点へ二重にカールを掛けない。
    let tip_points = [
        (0..4)
            .map(|turn| rotate_quarter([0.239, 1.0], turn))
            .collect::<Vec<_>>(),
        (0..4)
            .map(|turn| rotate_quarter([0.421, 1.0], turn))
            .collect::<Vec<_>>(),
    ];
    let tips = tip_points
        .iter()
        .flatten()
        .map(|&point| vertex_at(&doc.cp, point))
        .collect::<Vec<_>>();
    assert_eq!(
        tips.iter().copied().collect::<HashSet<_>>().len(),
        8,
        "8枚の花びら先端"
    );
    let cp_positions = doc
        .cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<HashMap<_, _>>();
    let all_tip_points = tip_points.iter().flatten().copied().collect::<Vec<_>>();
    let anchor_radius = DVec2::from([0.286, 0.822]).distance(DVec2::from(CENTER));
    let mut groups = vec![Vec::<VertexId>::new(); 8];
    for &vertex in &vertices {
        let point = DVec2::from(cp_positions[&vertex]);
        if point.distance(DVec2::from(CENTER)) + 1e-9 < anchor_radius {
            continue;
        }
        let nearest = all_tip_points
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = point.distance_squared(DVec2::from(**a));
                let db = point.distance_squared(DVec2::from(**b));
                da.total_cmp(&db)
            })
            .expect("花びら先端")
            .0;
        groups[nearest].push(vertex);
    }
    for (index, &tip) in tips.iter().enumerate() {
        if !groups[index].contains(&tip) {
            groups[index].push(tip);
        }
    }

    let before_curl = positions.clone();
    for (orbit, book_step) in [(0usize, 27usize), (1usize, 28usize)] {
        for turn in 0..4 {
            let index = orbit * 4 + turn;
            let root = vertex_at(&doc.cp, rotate_quarter([0.286, 0.822], turn));
            let root_position = DVec3::from(positions[root as usize]);
            let raw_tip = DVec3::from(positions[tips[index] as usize]) - root_position;
            let toward_tip = raw_tip - normal * raw_tip.dot(normal);
            let distance = toward_tip.length();
            assert!(distance > scale * 0.03, "花びら{}の付け根と先端", index + 1);
            let axis = normal.cross(toward_tip).normalize();
            let report = soft_curl::curl_vertices(
                &mut positions,
                &groups[index],
                &soft_curl::CurlSettings {
                    axis_origin: root_position.to_array(),
                    axis_direction: axis.to_array(),
                    toward_tip: toward_tip.to_array(),
                    radius: distance * 0.40,
                    angle_deg: 62.0,
                },
            )
            .unwrap_or_else(|error| panic!("花びら{}のカール: {error}", index + 1));
            assert!(report.moved_vertices > 0, "花びら{}が動いた", index + 1);
            assert!(
                report.max_displacement > scale * 0.005,
                "花びら{}のカール量",
                index + 1
            );
        }
        write_shared_positions(&faces, &mut frame, &positions);
        gaps.push(audit_soft_frame(&doc, &faces, &frame, book_step));
    }

    for (index, &tip) in tips.iter().enumerate() {
        let displacement = (DVec3::from(positions[tip as usize])
            - DVec3::from(before_curl[tip as usize]))
        .length();
        assert!(
            displacement > scale * 0.005,
            "花びら{}の先端が外側へカール",
            index + 1
        );
    }

    // 手順29の最終整形: 対向する4組を、軸回り半回転で最近接の左右対称形へ
    // そろえる。共有頂点そのものを補正するので、この仕上げでも裂けは生じない。
    let symmetry_center = mean_position(&positions, &center_vertices);
    let symmetry_pairs = [
        [tips[0], tips[2]],
        [tips[1], tips[3]],
        [tips[4], tips[6]],
        [tips[5], tips[7]],
    ];
    let symmetry = soft_symmetry::enforce_half_turn_symmetry(
        &mut positions,
        &symmetry_pairs,
        &soft_symmetry::HalfTurnSymmetrySettings {
            center: symmetry_center.to_array(),
            axis: normal.to_array(),
        },
    )
    .expect("手順29の左右対称仕上げ");
    assert_eq!(symmetry.pairs, 4);
    assert_eq!(symmetry.selected_vertices, 8);
    assert!(
        symmetry.moved_vertices > 0 && symmetry.max_displacement.is_finite(),
        "左右対称仕上げを適用した: {symmetry:?}"
    );
    write_shared_positions(&faces, &mut frame, &positions);
    gaps.push(audit_soft_frame(&doc, &faces, &frame, 29));
    doc.display.soft_enabled = true;
    doc.display.soft_stiffness = 0.72;
    doc.display.soft_pressure = 0.20;
    let center = mean_position(&positions, &center_vertices);

    CompletedRose {
        checkpoint11,
        checkpoint21,
        checkpoint29: doc,
        frame29: frame,
        positions29: positions,
        gaps,
        center,
        tips,
        scale,
    }
}

fn complete_rose() -> CompletedRose {
    let (checkpoint11, mut doc, rigid_gaps) = rose_through_21();
    let checkpoint21 = doc.clone();
    let curves = std::array::from_fn(|turn| {
        let spec = QUARTER.iter().find(|line| line.curved).unwrap();
        let p = spec
            .points
            .iter()
            .map(|&point| rotate_quarter(point, turn))
            .collect::<Vec<_>>();
        arc_polyline(p[0], p[1], p[2], 0.005, Some(CURVE_SEGMENTS))
    });
    close_petal_ring(&mut doc, &curves);
    shape_rose_to_29(
        doc,
        checkpoint11,
        checkpoint21,
        rigid_gaps.into_iter().collect(),
    )
}

/// ori3-export側のfixture生成器からも、同じ自己完結した折りを再利用する。
#[allow(dead_code)]
pub(crate) fn rose_checkpoint_artifacts() -> (Document, Document, Document, Frame3D) {
    let completed = complete_rose();
    (
        completed.checkpoint11,
        completed.checkpoint21,
        completed.checkpoint29,
        completed.frame29,
    )
}

#[test]
fn rose_reaches_book_step_29_with_eight_curled_petals() {
    let completed = complete_rose();
    assert!(FIXTURE_011.contains("\"schema_version\": 1"));
    assert!(FIXTURE_021.contains("\"schema_version\": 1"));
    assert!(
        FIXTURE_029.contains("\"book_step\": 29") && FIXTURE_029.contains("\"soft_geometry\""),
        "手順29 fixtureには完成曲面を保存する"
    );
    assert_eq!(completed.checkpoint11.sequence.len(), 1, "手順11の保存点");
    assert_eq!(completed.checkpoint21.sequence.len(), 4, "手順21の保存点");
    assert!(
        completed.gaps.iter().all(|gap| *gap < 1e-6),
        "全手順で裂けが許容値未満: {:?}",
        completed.gaps
    );
    assert_eq!(
        completed.checkpoint29.sequence.len(),
        5,
        "ねじり1・曲線つぶし3・閉環1。底とカールは折り目のない共有頂点変形"
    );
    assert!(completed.checkpoint29.display.soft_enabled);
    assert!(completed.frame29.warnings.is_empty());
    assert_eq!(
        completed.tips.iter().copied().collect::<HashSet<_>>().len(),
        8,
        "異なる8枚の花びら"
    );

    let tip_positions = completed
        .tips
        .iter()
        .map(|tip| DVec3::from(completed.positions29[*tip as usize]))
        .collect::<Vec<_>>();
    for first in 0..tip_positions.len() {
        for second in first + 1..tip_positions.len() {
            let distance = tip_positions[first].distance(tip_positions[second]);
            assert!(
                distance > completed.scale * 0.01,
                "花びら{}と{}の先端を識別できる: {distance}",
                first + 1,
                second + 1
            );
        }
    }

    let tip_height =
        tip_positions.iter().map(|point| point.z).sum::<f64>() / tip_positions.len() as f64;
    assert!(
        completed.center.z - tip_height > completed.scale * 0.08,
        "花の中心が外周より持ち上がる: center={}, tips={tip_height}",
        completed.center.z
    );

    // 各4枚組の180度反対側を比較する。左右の輪郭・高さが一致するため、
    // 花全体が中心に対して左右対称であることを直接確認できる。
    for &(first, second) in &[(0usize, 2usize), (1, 3), (4, 6), (5, 7)] {
        let a = tip_positions[first];
        let b = tip_positions[second];
        let radial_a = (a - completed.center).reject_from(DVec3::Z).length();
        let radial_b = (b - completed.center).reject_from(DVec3::Z).length();
        assert!(
            (radial_a - radial_b).abs() < completed.scale * 1e-5,
            "左右の花びら半径: {first}/{second}: {radial_a}/{radial_b}"
        );
        assert!(
            (a.z - b.z).abs() < completed.scale * 1e-5,
            "左右の花びら高さ: {first}/{second}: {}/{},",
            a.z,
            b.z
        );
        let midpoint = (a + b) * 0.5;
        assert!(
            (midpoint - completed.center).reject_from(DVec3::Z).length() < completed.scale * 1e-5,
            "左右の花びらの中点が花芯に一致: {first}/{second}"
        );
    }
}
