#[path = "support/devil_verify.rs"]
mod devil_verify;

use ori3_model::{Document, Paper};

#[test]
fn initial_sheet_passes_the_same_face_and_seam_checks() {
    let document = Document::new(Paper {
        width_mm: 250.0,
        height_mm: 250.0,
    });
    assert_eq!(devil_verify::verify_book_step(&document, 0), 0.0);
}

