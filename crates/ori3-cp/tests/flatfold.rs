//! 局所平坦折り判定(CPE-009)の検査: 前川定理・川崎定理。

use ori3_cp::{insert_segment, local_violations};
use ori3_model::{Document, EdgeKind, Paper};

fn square() -> Document {
    Document::new(Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    })
}

/// 正方形の中心に、指定した種類の折り目を放射状に引く(角度は度)
fn radial(kinds: &[(f64, EdgeKind)]) -> Document {
    let mut doc = square();
    for &(deg, kind) in kinds {
        let (s, c) = deg.to_radians().sin_cos();
        // 中心から縁まで届く長さ(正方形の半対角より長い)
        let (x, y) = (0.5 + c, 0.5 + s);
        insert_segment(&mut doc.cp, [0.5, 0.5], [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)], kind);
    }
    doc
}

#[test]
fn flat_foldable_vertex_has_no_violation() {
    // 4本の折り目が90°ずつ:山3・谷1(前川)、90+90=180(川崎)
    let doc = radial(&[
        (0.0, EdgeKind::Mountain),
        (90.0, EdgeKind::Mountain),
        (180.0, EdgeKind::Mountain),
        (270.0, EdgeKind::Valley),
    ]);
    assert!(local_violations(&doc.cp).is_empty(), "cp={:?}", doc.cp);
}

#[test]
fn maekawa_violation_is_reported() {
    // 山4・谷0 → 山−谷=4(±2でない)。角の和は満たしている
    let doc = radial(&[
        (0.0, EdgeKind::Mountain),
        (90.0, EdgeKind::Mountain),
        (180.0, EdgeKind::Mountain),
        (270.0, EdgeKind::Mountain),
    ]);
    let v = local_violations(&doc.cp);
    assert_eq!(v.len(), 1, "中心1点だけが違反: {v:?}");
}

#[test]
fn kawasaki_violation_is_reported() {
    // 山3・谷1(前川は満たす)だが角が 90/90/60/120 で1つおきの和が150°
    let doc = radial(&[
        (0.0, EdgeKind::Mountain),
        (90.0, EdgeKind::Mountain),
        (180.0, EdgeKind::Mountain),
        (240.0, EdgeKind::Valley),
    ]);
    assert_eq!(local_violations(&doc.cp).len(), 1);
}

#[test]
fn odd_degree_vertex_is_reported() {
    // 3本(奇数)は1つおきに分けられないので違反
    let doc = radial(&[
        (0.0, EdgeKind::Mountain),
        (120.0, EdgeKind::Mountain),
        (240.0, EdgeKind::Valley),
    ]);
    assert_eq!(local_violations(&doc.cp).len(), 1);
}

#[test]
fn border_vertices_and_aux_lines_are_ignored() {
    // 紙の縁の上の点(対角線の端)は内部の点ではないので検査しない
    let mut doc = square();
    insert_segment(&mut doc.cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Mountain);
    assert!(local_violations(&doc.cp).is_empty());

    // 補助線は折りに関与しないので数えない(中心は山2本の直線のままで違反なし)
    let mut doc = radial(&[(0.0, EdgeKind::Mountain), (180.0, EdgeKind::Mountain)]);
    insert_segment(&mut doc.cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Aux);
    assert!(local_violations(&doc.cp).is_empty(), "cp={:?}", doc.cp);
}

#[test]
fn result_is_sorted_and_deterministic() {
    // 2か所の内部頂点(どちらも山4本)を作り、昇順で2件返ることを確かめる
    let mut doc = square();
    for cx in [0.3_f64, 0.7_f64] {
        for (dx, dy) in [(0.2, 0.0), (-0.2, 0.0), (0.0, 0.2), (0.0, -0.2)] {
            insert_segment(
                &mut doc.cp,
                [cx, 0.5],
                [cx + dx, 0.5 + dy],
                EdgeKind::Mountain,
            );
        }
    }
    let v = local_violations(&doc.cp);
    assert_eq!(v.len(), 2, "v={v:?}");
    assert!(v[0] < v[1]);
    assert_eq!(v, local_violations(&doc.cp));
}
