//! めり込み警告(SIM-007)の検査。

use ori3_cp::{extract_faces, insert_segment};
use ori3_model::{Document, EdgeKind, Face3D, Frame3D, Paper};
use ori3_rigid::{layer_order_conflicts, self_intersects};

fn frame(faces: Vec<Face3D>) -> Frame3D {
    Frame3D {
        faces,
        warnings: Vec::new(),
    }
}

fn face(id: u32, polygon: &[[f64; 3]]) -> Face3D {
    Face3D {
        face: id,
        polygon: polygon.to_vec(),
        layer: 0,
    }
}

/// 平らに畳んだ状態(全ての点がz≒0)では、重なっていても警告を出さない。
/// 全ての層が同一平面に重なるのが正常な畳み方だから
#[test]
fn flat_stacked_layers_are_not_penetration() {
    let a = face(
        0,
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
    );
    let b = face(
        1,
        &[
            [0.2, 0.2, 0.0],
            [0.8, 0.2, 0.0],
            [0.8, 0.8, 0.0],
            [0.2, 0.8, 0.0],
        ],
    );
    assert!(!self_intersects(&frame(vec![a, b])));
}

#[test]
fn flat_fold_warns_only_for_layer_order_that_contradicts_mountain_valley() {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    insert_segment(&mut document.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Valley);
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 2);
    let crease = document
        .cp
        .edges
        .iter()
        .find(|edge| edge.kind == EdgeKind::Valley)
        .expect("谷折り線");
    let adjacent: Vec<_> = faces
        .iter()
        .filter(|face| face.edges.contains(&crease.id))
        .map(|face| face.id)
        .collect();
    assert_eq!(adjacent.len(), 2);

    let vertex = |id| {
        document
            .cp
            .vertices
            .iter()
            .find(|vertex| vertex.id == id)
            .expect("頂点")
            .pos
    };
    let mut folded_faces: Vec<Face3D> = faces
        .iter()
        .map(|face| {
            let mean_x = face.vertices.iter().map(|&id| vertex(id)[0]).sum::<f64>()
                / face.vertices.len() as f64;
            let mirrored = mean_x > 0.5;
            Face3D {
                face: face.id,
                polygon: face
                    .vertices
                    .iter()
                    .map(|&id| {
                        let [x, y] = vertex(id);
                        [if mirrored { 1.0 - x } else { x }, y, 0.0]
                    })
                    .collect(),
                layer: 0,
            }
        })
        .collect();
    let a = adjacent[0];
    let b = adjacent[1];
    let source_a = faces.iter().find(|face| face.id == a).expect("元面a");
    let a_mirrored = source_a
        .vertices
        .iter()
        .map(|&id| vertex(id)[0])
        .sum::<f64>()
        / source_a.vertices.len() as f64
        > 0.5;
    let b_should_be_above = !a_mirrored; // 谷: 表向きaから見ればbが上
    for face in &mut folded_faces {
        face.layer = if face.face == a {
            u32::from(!b_should_be_above)
        } else if face.face == b {
            u32::from(b_should_be_above)
        } else {
            0
        };
    }
    let mut folded = frame(folded_faces);
    assert!(
        !layer_order_conflicts(&document.cp, &faces, &folded),
        "山谷どおりの層順序は正常"
    );
    for face in &mut folded.faces {
        face.layer = 1 - face.layer;
    }
    assert!(
        layer_order_conflicts(&document.cp, &faces, &folded),
        "上下を反転すると紙が折り目を突き抜ける矛盾になる"
    );
}

#[test]
fn crossing_faces_are_reported() {
    // 水平な面を、垂直な面が真ん中で貫いている
    let flat = face(
        0,
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.5],
        ],
    );
    let upright = face(
        1,
        &[
            [0.5, -0.5, -0.5],
            [0.5, 1.5, -0.5],
            [0.5, 1.5, 0.5],
            [0.5, -0.5, 0.5],
        ],
    );
    assert!(self_intersects(&frame(vec![flat, upright])));
}

#[test]
fn separated_faces_are_not_reported() {
    let a = face(
        0,
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.3],
            [0.0, 1.0, 0.3],
        ],
    );
    let b = face(
        1,
        &[
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.3],
            [0.0, 1.0, 1.3],
        ],
    );
    assert!(!self_intersects(&frame(vec![a, b])));
}

/// 折り目でつながった2面(辺を共有)は、どんな角度でも食い込みとしない
#[test]
fn hinged_faces_touching_along_their_fold_are_not_reported() {
    let a = face(
        0,
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
    );
    // 共有辺 x=1 のまわりに直角に立ち上がった面
    let b = face(
        1,
        &[
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ],
    );
    assert!(!self_intersects(&frame(vec![a, b])));
}

/// 面400枚(NFR-002の想定規模)を折り途中のように重ねても、判定が現実的な時間で終わる。
/// 判定は編集のたびに走るので、遅いと画面が引っかかる
#[test]
fn checks_400_faces_quickly() {
    // 少しずつ傾けて積み重ねた400枚(平らではないが交差はしていない)
    let faces: Vec<Face3D> = (0..400)
        .map(|k| {
            let z = f64::from(k) * 0.002;
            face(
                u32::try_from(k).unwrap(),
                &[
                    [0.0, 0.0, z],
                    [1.0, 0.0, z],
                    [1.0, 1.0, z + 0.001],
                    [0.0, 1.0, z + 0.001],
                ],
            )
        })
        .collect();
    let frame = frame(faces);
    let started = std::time::Instant::now();
    assert!(!self_intersects(&frame));
    let elapsed = started.elapsed();
    println!("面400枚の判定: {elapsed:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "{elapsed:?}"
    );
}

/// 同じ平面(水平でない)に重なった層は、めり込みではなく普通の重なり
#[test]
fn coplanar_slanted_layers_are_not_reported() {
    let a = face(
        0,
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ],
    );
    let b = face(
        1,
        &[
            [0.2, 0.2, 0.2],
            [0.8, 0.2, 0.8],
            [0.8, 0.8, 0.8],
            [0.2, 0.8, 0.2],
        ],
    );
    assert!(!self_intersects(&frame(vec![a, b])));
}
