use ori3_cp::{extract_faces, insert_segment, local_violations, validate};
use ori3_model::{Document, EdgeKind, Paper};

fn devil_logical_lines() -> [([f64; 2], [f64; 2]); 22] {
    let sqrt2 = std::f64::consts::SQRT_2;
    let t = sqrt2 - 1.0;
    let q = 2.0 - sqrt2;
    let s = 2.0 * t;
    let e = 2.0 * q - 1.0;
    let a = sqrt2 / 4.0;
    let b = (2.0 + sqrt2) / 4.0;
    let k = 4.0 * t - 1.0;
    [
        ([0.0, 0.0], [1.0, 1.0]),
        ([1.0, 0.0], [0.0, 1.0]),
        ([1.0, 1.0], [0.0, q]),
        ([1.0, 1.0], [q, 0.0]),
        ([0.0, 1.0], [1.0, q]),
        ([1.0, 0.0], [q, 1.0]),
        ([0.0, q], [1.0, q]),
        ([q, 0.0], [q, 1.0]),
        ([0.0, t], [q, 1.0]),
        ([t, 0.0], [1.0, q]),
        ([0.0, t], [t, 0.0]),
        ([q, 1.0], [1.0, q]),
        ([s, 0.0], [t, 1.0]),
        ([0.0, s], [1.0, t]),
        ([q, 0.0], [e, 1.0]),
        ([0.0, q], [1.0, e]),
        ([0.0, s], [s, 0.0]),
        ([e, 1.0], [1.0, e]),
        ([0.0, a], [b, 0.0]),
        ([a, 0.0], [0.0, b]),
        ([0.0, k], [k, 0.0]),
        ([0.0, 0.5], [0.5, 0.0]),
    ]
}

fn rebuilt_precrease() -> Document {
    let mut document = Document::new(Paper {
        width_mm: 250.0,
        height_mm: 250.0,
    });
    for (index, (a, b)) in devil_logical_lines().into_iter().enumerate() {
        let kind = if index == 0 {
            EdgeKind::Valley
        } else {
            EdgeKind::Aux
        };
        insert_segment(&mut document.cp, a, b, kind);
    }
    document
}

#[test]
fn devil_steps_1_to_16_keep_unassigned_precreases_auxiliary() {
    let document = rebuilt_precrease();

    assert_eq!(document.cp.vertices.len(), 92);
    assert_eq!(document.cp.edges.len(), 201);
    assert_eq!(extract_faces(&document.cp).len(), 2);
    assert!(local_violations(&document.cp).is_empty());
    assert!(validate(&document.cp).is_empty());
}
