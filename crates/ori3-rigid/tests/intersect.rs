//! めり込み警告(SIM-007)の検査。

use ori3_model::{Face3D, Frame3D};
use ori3_rigid::self_intersects;

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
