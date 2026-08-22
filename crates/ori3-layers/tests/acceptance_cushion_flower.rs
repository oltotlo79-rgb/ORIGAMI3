//! 八枚花弁の座布団花の受入検査。
//!
//! 正方形を対角線と十字で八つの三角面に分け、中心のくぼみ、各花弁先端の丸み、
//! 半回転対称を一つの完成フレームへ順に適用する。複雑な手順書に依存せず、
//! SIM-014 の合成を確認できる標準的な対称題材である。

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces, insert_segment, validate};
use ori3_layers::{flat_state_at, replay};
use ori3_model::{CreasePattern, Document, EdgeKind, Frame3D, Paper, VertexId};
use ori3_rigid::max_seam_gap;

#[path = "../../ori3-soft/src/cup.rs"]
mod soft_cup;
#[path = "../../ori3-soft/src/curl.rs"]
mod soft_curl;
#[path = "../../ori3-soft/src/symmetry.rs"]
mod soft_symmetry;

const CENTER: [f64; 2] = [0.5, 0.5];
const PETAL_TIPS: [[f64; 2]; 8] = [
    [0.5, 1.0],
    [1.0, 1.0],
    [1.0, 0.5],
    [1.0, 0.0],
    [0.5, 0.0],
    [0.0, 0.0],
    [0.0, 0.5],
    [0.0, 1.0],
];

/// 八枚の三角花弁を作る。折り線は中央でのみ交差し、箱やマチを持たないため
/// 平坦状態と共有頂点の対応を安定して得られる。
fn cushion_flower_document() -> Document {
    let mut document = Document::new(Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    });
    for (from, to, kind) in [
        ([0.0, 0.0], [1.0, 1.0], EdgeKind::Valley),
        ([1.0, 0.0], [0.0, 1.0], EdgeKind::Valley),
        ([0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain),
        ([0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain),
    ] {
        insert_segment(&mut document.cp, from, to, kind);
    }
    document.display.soft_enabled = true;
    document
}

fn vertex_at(cp: &CreasePattern, point: [f64; 2]) -> VertexId {
    cp.vertices
        .iter()
        .find(|vertex| (DVec2::from(vertex.pos) - DVec2::from(point)).length() < 1e-7)
        .unwrap_or_else(|| panic!("花弁の頂点 {point:?}"))
        .id
}

/// 共有頂点を一度だけ記録した座標列へ集約する。
fn shared_positions(
    cp: &CreasePattern,
    faces: &[Face],
    frame: &Frame3D,
) -> (Vec<[f64; 3]>, Vec<VertexId>) {
    let frame_faces = frame
        .faces
        .iter()
        .map(|face| (face.face, face))
        .collect::<HashMap<_, _>>();
    let mut positions = vec![[f64::NAN; 3]; cp.next_vertex_id as usize];
    for face in faces {
        let face3d = frame_faces[&face.id];
        assert_eq!(
            face3d.polygon.len(),
            face.vertices.len(),
            "面{}の頂点数",
            face.id
        );
        for (&vertex, &point) in face.vertices.iter().zip(&face3d.polygon) {
            assert!(point.into_iter().all(f64::is_finite), "頂点{vertex}が有限");
            let slot = &mut positions[vertex as usize];
            if slot[0].is_nan() {
                *slot = point;
            } else {
                let gap = (DVec3::from(*slot) - DVec3::from(point)).length();
                assert!(gap < 1e-6, "共有頂点{vertex}の裂け: {gap}");
            }
        }
    }
    let vertices = cp
        .vertices
        .iter()
        .map(|vertex| vertex.id)
        .collect::<Vec<_>>();
    assert!(
        vertices
            .iter()
            .all(|vertex| positions[*vertex as usize].into_iter().all(f64::is_finite)),
        "全ての型紙頂点が3Dへ写る"
    );
    (positions, vertices)
}

fn write_shared_positions(faces: &[Face], frame: &mut Frame3D, positions: &[[f64; 3]]) {
    let by_id = faces
        .iter()
        .map(|face| (face.id, face))
        .collect::<HashMap<_, _>>();
    for face3d in &mut frame.faces {
        let face = by_id[&face3d.face];
        assert_eq!(
            face3d.polygon.len(),
            face.vertices.len(),
            "面{}の頂点数",
            face.id
        );
        for (&vertex, point) in face.vertices.iter().zip(&mut face3d.polygon) {
            *point = positions[vertex as usize];
        }
    }
}

#[test]
fn eight_petal_cushion_flower_combines_cup_curl_and_half_turn_symmetry() {
    let document = cushion_flower_document();
    assert!(validate(&document.cp).is_empty(), "型紙の警告なし");
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 8, "八枚の三角花弁");

    let replayed = replay(&document, document.sequence.len(), 1.0);
    assert!(replayed.warnings.is_empty(), "再生の警告なし");
    assert!(replayed.skipped.is_empty(), "飛ばした工程なし");
    assert!(replayed.frame.warnings.is_empty(), "3D警告なし");
    assert_eq!(replayed.frame.faces.len(), faces.len(), "3D面数");

    let (flat, flat_warnings) =
        flat_state_at(&document, &faces, document.sequence.len()).expect("平坦状態を得る");
    assert!(flat_warnings.is_empty(), "平坦状態の警告なし");

    let mut frame = replayed.frame;
    let (mut positions, vertices) = shared_positions(&document.cp, &faces, &frame);
    let center_vertex = vertex_at(&document.cp, CENTER);
    let center = DVec3::from(positions[center_vertex as usize]);
    let normal = DVec3::Z;
    let planar_radius = |point: DVec3| (point - center).reject_from(normal).length();
    let scale = vertices
        .iter()
        .map(|vertex| planar_radius(DVec3::from(positions[*vertex as usize])))
        .fold(0.0_f64, f64::max);
    assert!(scale > 0.1 && scale.is_finite(), "花の大きさ: {scale}");

    let cup = soft_cup::radial_cup_vertices(
        &mut positions,
        &vertices,
        &soft_cup::RadialCupSettings {
            center: center.to_array(),
            normal: normal.to_array(),
            inner_radius: scale * 0.12,
            outer_radius: scale * 0.80,
            height: scale * 0.20,
        },
    )
    .expect("中心をくぼませる");
    assert!(
        cup.moved_vertices > 0 && cup.max_displacement > scale * 0.05,
        "中心のくぼみを作る: {cup:?}"
    );

    let smooth = soft_cup::radial_cup_vertices(
        &mut positions,
        &vertices,
        &soft_cup::RadialCupSettings {
            center: center.to_array(),
            normal: normal.to_array(),
            inner_radius: scale * 0.08,
            outer_radius: scale * 0.90,
            height: scale * 0.025,
        },
    )
    .expect("くぼみを滑らかにする");
    assert!(
        smooth.moved_vertices > 0 && smooth.max_displacement > 0.0,
        "滑らかな丸みを作る: {smooth:?}"
    );

    let tips = PETAL_TIPS
        .iter()
        .map(|&point| vertex_at(&document.cp, point))
        .collect::<Vec<_>>();
    assert_eq!(tips.len(), 8, "八つの花弁先端");
    let before_curl = positions.clone();
    for (index, &tip) in tips.iter().enumerate() {
        let before = DVec3::from(positions[tip as usize]);
        let toward_tip = (before - center).reject_from(normal);
        let distance = toward_tip.length();
        assert!(distance > scale * 0.30, "花弁{}の根元と先端", index + 1);
        let report = soft_curl::curl_vertices(
            &mut positions,
            &[tip],
            &soft_curl::CurlSettings {
                axis_origin: center.to_array(),
                axis_direction: normal.cross(toward_tip).normalize().to_array(),
                toward_tip: toward_tip.to_array(),
                radius: distance * 0.40,
                angle_deg: if index == 0 { 66.0 } else { 62.0 },
            },
        )
        .unwrap_or_else(|error| panic!("花弁{}を丸める: {error}", index + 1));
        let after = DVec3::from(positions[tip as usize]);
        assert_eq!(report.moved_vertices, 1, "花弁{}が動く", index + 1);
        assert!(
            planar_radius(after) < planar_radius(before) * 0.90,
            "花弁{}が中心へ丸まる",
            index + 1
        );
        assert!(
            after.z < before.z - scale * 0.005,
            "花弁{}が下方へ丸まる",
            index + 1
        );
    }
    for (index, &tip) in tips.iter().enumerate() {
        let displacement = (DVec3::from(positions[tip as usize])
            - DVec3::from(before_curl[tip as usize]))
        .length();
        assert!(
            displacement > scale * 0.005,
            "花弁{}の先端が丸まりで動く",
            index + 1
        );
    }

    let symmetry = soft_symmetry::enforce_half_turn_symmetry(
        &mut positions,
        &[
            [tips[0], tips[4]],
            [tips[1], tips[5]],
            [tips[2], tips[6]],
            [tips[3], tips[7]],
        ],
        &soft_symmetry::HalfTurnSymmetrySettings {
            center: center.to_array(),
            axis: normal.to_array(),
        },
    )
    .expect("対向する花弁を対称にする");
    assert_eq!(symmetry.pairs, 4, "対向する四組");
    assert_eq!(symmetry.selected_vertices, 8, "八つの花弁先端");
    assert!(
        symmetry.moved_vertices > 0 && symmetry.max_displacement.is_finite(),
        "対称補正が花弁へ働く: {symmetry:?}"
    );

    write_shared_positions(&faces, &mut frame, &positions);
    assert!(document.display.soft_enabled, "柔らかい表示を保存する");
    assert_eq!(
        frame.faces.len(),
        flat.placements.len(),
        "soft frameと平坦配置の面数"
    );
    for face in &frame.faces {
        assert_eq!(
            face.mirrored, flat.placements[&face.face].mirrored,
            "面{}のsoft表示と平坦配置の鏡映偶奇",
            face.face
        );
    }

    let gap = max_seam_gap(&document.cp, &faces, &frame);
    assert!(gap < 1e-6, "丸めた花弁の裂け: {gap}");
}
