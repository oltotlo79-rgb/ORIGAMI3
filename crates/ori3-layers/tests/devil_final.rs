#[path = "support/devil_final.rs"]
mod devil_final;

use ori3_cp::insert_segment;
use ori3_model::{Document, EdgeKind, Paper};

fn logical_lines() -> [([f64; 2], [f64; 2]); 22] {
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

#[test]
fn full_devil_pose_is_persisted_and_stands_on_three_points() {
    let mut document = Document::new(Paper {
        width_mm: 250.0,
        height_mm: 250.0,
    });
    for (a, b) in logical_lines() {
        insert_segment(&mut document.cp, a, b, EdgeKind::Valley);
    }
    let metrics = devil_final::append_step_144(&mut document);
    assert_eq!(metrics.faces, 110);
    assert_eq!(metrics.hinges, 177);
    assert_eq!(metrics.horn_tips, 2);
    assert_eq!(metrics.boundary_corners, 4);
    assert!(metrics.symmetry_error < 1e-9);
    assert!(metrics.max_seam_gap < 1e-6);
    assert!(metrics.z_span > 0.1);
    assert!(metrics.support_area > 0.1);
    assert!(metrics.centroid_height > 0.1);
}

