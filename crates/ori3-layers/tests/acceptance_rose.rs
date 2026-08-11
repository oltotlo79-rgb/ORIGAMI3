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

const STEP_INSTRUCTIONS: [&str; 29] = [
    "型紙を重ね、全ての折り筋を強くなぞる",
    "色の付いた表側から裏側へ返す",
    "中央小正方形の4辺を裏から山折り方向へつまむ",
    "太線4本を曲線の山折りにする",
    "別の太線4本を強く山折りする",
    "中央を押してへこませる",
    "太い山折り線をつまんで立てる",
    "★印を支点に太線を動かし、中央をせり上げる",
    "太線2本を押し、中心を時計回りに回転させる",
    "谷折りを確認し、太い山折り線を滑らかにする",
    "横向きに持ち、最初の折り筋で谷折りする",
    "花びら①・②と※印の3枚を持ち、手前へ少し回す",
    "奥の花びら③を出し、①・②・※・③の4枚を重ねてつまむ",
    "袋を膨らませながら破線を軽く谷折りする",
    "袋をつぶして花びら4を作る",
    "花びら4を親指で押さえる",
    "花びら1〜4の層配置を拡大して確認する",
    "同じつぶし折りで6枚目の花びらを作る",
    "次の対象を手前へ少し回す",
    "同じつぶし折りで8枚目の花びらを作る",
    "回した後の層配置を確認する",
    "花びら①・②を開き、手順12〜17と同様につぶして⑦・⑧へかぶせ、①・②を④の下へ戻す",
    "底の三角の角aを内側へ折り込む",
    "角b・c・dも内側へ折り、底を円筒にする",
    "円筒を矢印方向から見る",
    "太い山折り線を滑らかにする",
    "☆印の花びらの角を外側へカールさせる",
    "▲印の角を外側へカールさせる",
    "完成したローズを確認する",
];

fn step_changes_shape(book_step: usize) -> bool {
    !matches!(book_step, 2 | 16 | 17 | 19 | 21 | 25 | 29)
}

#[derive(Clone)]
pub(crate) struct RoseStepArtifact {
    pub(crate) book_step: u32,
    pub(crate) instruction: &'static str,
    pub(crate) changes_shape: bool,
    pub(crate) document: Document,
    pub(crate) frame: Frame3D,
    pub(crate) max_seam_gap: f64,
}

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

fn audit_frame(doc: &Document, frame: &Frame3D, book_step: usize) -> f64 {
    let cp_warnings = validate(&doc.cp);
    assert!(
        cp_warnings.is_empty(),
        "手順{book_step}: 型紙警告 {cp_warnings:?}"
    );
    let faces = extract_faces(&doc.cp);
    let expected = faces
        .iter()
        .map(|face| (face.id, face.vertices.len()))
        .collect::<HashMap<_, _>>();
    assert_eq!(
        frame.faces.len(),
        faces.len(),
        "手順{book_step}: 面が失われた"
    );
    assert!(
        frame.warnings.is_empty(),
        "手順{book_step}: 3D警告 {:?}",
        frame.warnings
    );
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
    let gap = max_seam_gap(&doc.cp, &faces, frame);
    assert!(gap < 1e-6, "手順{book_step}: 裂け {gap:.9}");
    if book_step.is_multiple_of(5) {
        println!("手順{book_step}まで通過。max_seam_gap={gap:.3e}");
    }
    gap
}

fn replay_artifact(doc: &Document, book_step: usize, t: f64) -> RoseStepArtifact {
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
    let gap = audit_frame(doc, &result.frame, book_step);
    RoseStepArtifact {
        book_step: u32::try_from(book_step).unwrap(),
        instruction: STEP_INSTRUCTIONS[book_step - 1],
        changes_shape: step_changes_shape(book_step),
        document: doc.clone(),
        frame: result.frame,
        max_seam_gap: gap,
    }
}

fn duplicate_artifact(source: &RoseStepArtifact, book_step: usize) -> RoseStepArtifact {
    let gap = audit_frame(&source.document, &source.frame, book_step);
    RoseStepArtifact {
        book_step: u32::try_from(book_step).unwrap(),
        instruction: STEP_INSTRUCTIONS[book_step - 1],
        changes_shape: step_changes_shape(book_step),
        document: source.document.clone(),
        frame: source.frame.clone(),
        max_seam_gap: gap,
    }
}

fn soft_artifact(doc: &Document, frame: &Frame3D, book_step: usize) -> RoseStepArtifact {
    let gap = audit_frame(doc, frame, book_step);
    RoseStepArtifact {
        book_step: u32::try_from(book_step).unwrap(),
        instruction: STEP_INSTRUCTIONS[book_step - 1],
        changes_shape: step_changes_shape(book_step),
        document: doc.clone(),
        frame: frame.clone(),
        max_seam_gap: gap,
    }
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
    artifacts: Vec<RoseStepArtifact>,
    frame29: Frame3D,
    positions29: Vec<[f64; 3]>,
    gaps: Vec<f64>,
    center: DVec3,
    tips: Vec<VertexId>,
    scale: f64,
}

/// 正確座標の型紙を、手順21(8枚の花びら)まで畳む。
fn rose_through_21() -> (Document, Document, Vec<RoseStepArtifact>) {
    let (mut doc, curves) = rose_pattern();
    let step1 = replay_artifact(&doc, 1, 1.0);
    let step2 = duplicate_artifact(&step1, 2);
    let step3 = duplicate_artifact(&step2, 3);
    let mut artifacts = vec![step1, step2, step3];

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
    for book_step in 4..=11 {
        let progress = (book_step - 3) as f64 / 8.0;
        artifacts.push(replay_artifact(&doc, book_step, progress));
    }
    let checkpoint11 = doc.clone();

    // 原図の12〜15は1回目のつぶし折りの準備・途中・完了。16・17は
    // 完成した花びらを押さえて拡大確認するだけなので、15の形をそのまま残す。
    {
        let turn = 0usize;
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
            // 既存rose-021の履歴注記はバイト互換のため維持する。16・17が
            // 保持・拡大だけである正しい区分はRoseStepArtifact側へ記録する。
            "手順12〜17: 花びら1〜4",
        );
        for (book_step, progress) in [(12usize, 0.25), (13, 0.50), (14, 0.75), (15, 1.0)] {
            artifacts.push(replay_artifact(&doc, book_step, progress));
        }
        let step15 = artifacts.last().expect("手順15").clone();
        let step16 = duplicate_artifact(&step15, 16);
        artifacts.push(step16.clone());
        artifacts.push(duplicate_artifact(&step16, 17));
    }

    // 原図18は6枚目を作る反復操作、19は次の対象を手前へ回すだけ。
    {
        let turn = 3usize;
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
            "手順18: 花びら5・6",
        );
        let step18 = replay_artifact(&doc, 18, 1.0);
        artifacts.push(step18.clone());
        artifacts.push(duplicate_artifact(&step18, 19));
    }

    // 原図20は8枚目を作り、21は回した後の層配置を示すだけ。
    {
        let turn = 2usize;
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
            // 既存rose-021の履歴注記はバイト互換のため維持する。工程19が
            // 向き変更だけである正しい区分はRoseStepArtifact側へ記録する。
            "手順19〜20: 花びら7・8",
        );
        let step20 = replay_artifact(&doc, 20, 1.0);
        artifacts.push(step20.clone());
        artifacts.push(duplicate_artifact(&step20, 21));
    }

    assert_eq!(artifacts.len(), 21, "手順1〜21を1件ずつ収集");
    (checkpoint11, doc, artifacts)
}

/// 手順23〜29の底の折り込み・円筒化・丸み・8枚のカールを、
/// 共有頂点を保ったまま施す。
fn shape_rose_to_29(
    mut doc: Document,
    checkpoint11: Document,
    checkpoint21: Document,
    mut artifacts: Vec<RoseStepArtifact>,
) -> CompletedRose {
    let (faces, mut frame, mut positions, vertices) = shared_frame(&doc);
    doc.display.soft_enabled = true;
    doc.display.soft_stiffness = 0.72;
    doc.display.soft_pressure = 0.20;
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
    let tuck_corner = |turn: usize, positions: &mut Vec<[f64; 3]>| {
        let corner = center_vertices[turn];
        let before = DVec3::from(positions[corner as usize]);
        let toward_corner = (before - tuck_center).reject_from(normal);
        let distance = toward_corner.length();
        assert!(distance > scale * 0.01, "底のカド{}", turn + 1);
        let report = soft_curl::curl_vertices(
            positions,
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
    };
    tuck_corner(0, &mut positions);
    write_shared_positions(&faces, &mut frame, &positions);
    artifacts.push(soft_artifact(&doc, &frame, 23));
    for turn in 1..4 {
        tuck_corner(turn, &mut positions);
    }
    center = mean_position(&positions, &center_vertices);

    // 原図24の「底を円筒にする」までをここで完了する。25はその円筒を
    // 矢印方向から見るだけなので、同じ到達形状を別番号で保存する。
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
    .expect("手順24の円筒化");
    assert!(
        cup.moved_vertices > 0 && cup.max_displacement > scale * 0.05,
        "底を円筒化できた: {cup:?}"
    );
    write_shared_positions(&faces, &mut frame, &positions);
    let step24 = soft_artifact(&doc, &frame, 24);
    artifacts.push(step24.clone());
    artifacts.push(duplicate_artifact(&step24, 25));

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
    artifacts.push(soft_artifact(&doc, &frame, 26));

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
    for orbit in 0..2 {
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
        if orbit == 0 {
            artifacts.push(soft_artifact(&doc, &frame, 27));
        }
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

    // 原図28のカールを左右対称にそろえて完成形まで仕上げる。原図29は
    // 新しい操作のない完成図なので、この同じ形を29にも保存する。
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
    .expect("手順28の左右対称仕上げ");
    assert_eq!(symmetry.pairs, 4);
    assert_eq!(symmetry.selected_vertices, 8);
    assert!(
        symmetry.moved_vertices > 0 && symmetry.max_displacement.is_finite(),
        "左右対称仕上げを適用した: {symmetry:?}"
    );
    write_shared_positions(&faces, &mut frame, &positions);
    let step28 = soft_artifact(&doc, &frame, 28);
    artifacts.push(step28.clone());
    artifacts.push(duplicate_artifact(&step28, 29));
    assert_eq!(artifacts.len(), 29, "全29工程を1件ずつ収集");
    assert!(
        artifacts
            .iter()
            .enumerate()
            .all(|(index, artifact)| artifact.book_step as usize == index + 1),
        "工程番号が1〜29の連番"
    );
    let gaps = artifacts
        .iter()
        .map(|artifact| artifact.max_seam_gap)
        .collect::<Vec<_>>();
    let center = mean_position(&positions, &center_vertices);

    CompletedRose {
        checkpoint11,
        checkpoint21,
        checkpoint29: doc,
        artifacts,
        frame29: frame,
        positions29: positions,
        gaps,
        center,
        tips,
        scale,
    }
}

fn complete_rose() -> CompletedRose {
    let (checkpoint11, mut doc, mut artifacts) = rose_through_21();
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
    artifacts.push(replay_artifact(&doc, 22, 1.0));
    shape_rose_to_29(doc, checkpoint11, checkpoint21, artifacts)
}

/// 原図の工程番号と1対1の到達状態。向き変更・保持・完成図だけの工程も、
/// 直前と同じFrameを明示的に1件持つ。
#[allow(dead_code)]
pub(crate) fn rose_step_artifacts() -> Vec<RoseStepArtifact> {
    complete_rose().artifacts
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

#[cfg(test)]
fn assert_same_frame(first: &Frame3D, second: &Frame3D, label: &str) {
    assert_eq!(first.warnings, second.warnings, "{label}: 3D警告");
    assert_eq!(first.faces.len(), second.faces.len(), "{label}: 面数");
    for (a, b) in first.faces.iter().zip(&second.faces) {
        assert_eq!(a.face, b.face, "{label}: 面ID");
        assert_eq!(a.layer, b.layer, "{label}: 層番号");
        assert_eq!(a.polygon, b.polygon, "{label}: 面頂点");
    }
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
    assert_eq!(completed.artifacts.len(), 29, "原図29工程の到達状態");
    assert!(
        completed
            .artifacts
            .iter()
            .all(|artifact| !artifact.instruction.is_empty()),
        "全工程に原図対応の説明がある"
    );
    assert!(
        completed.artifacts.iter().all(|artifact| {
            artifact.changes_shape == step_changes_shape(artifact.book_step as usize)
        }),
        "形状不変工程の判定を成果物へ残す"
    );
    assert_eq!(completed.gaps.len(), 29, "全工程を個別に裂け検査");
    assert!(
        completed.gaps.iter().all(|gap| *gap < 1e-6),
        "全手順で裂けが許容値未満: {:?}",
        completed.gaps
    );
    let largest_gap = completed.gaps.iter().copied().fold(0.0_f64, f64::max);
    println!("ローズ29工程の最大max_seam_gap={largest_gap:.3e}");
    assert_eq!(completed.gaps[28], 0.0, "手順29の裂けは従来どおり0");
    for &(first, second) in &[
        (1usize, 2usize),
        (15, 16),
        (16, 17),
        (18, 19),
        (20, 21),
        (24, 25),
        (28, 29),
    ] {
        assert_same_frame(
            &completed.artifacts[first - 1].frame,
            &completed.artifacts[second - 1].frame,
            &format!("形状不変の手順{first}→{second}"),
        );
    }
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
