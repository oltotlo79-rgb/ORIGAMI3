//! ori3-export: 展開図・折り図の書き出しと「FOLD 1.2 限定」の中立変換。

pub mod cp_png;
pub mod cp_svg;
pub mod diagram;
pub mod document;
pub mod fold;
pub mod manual;
pub mod pdf;

pub use cp_png::{DEFAULT_LONG_SIDE_PX, MAX_LONG_SIDE_PX, cp_png};
pub use cp_svg::{CpSvgOptions, cp_svg};
pub use diagram::render_step;
pub use document::{document_json, save_document};
pub use fold::{
    FOLD_1_2_PROFILE_NAME, FOLD_1_2_UNSUPPORTED_FEATURES, canonicalize_fold_1_2, compare_fold_1_2,
    document_to_fold, fold_to_document, parse_fold_1_2, unsupported_fields, validate_fold_1_2,
    write_fold_1_2,
};
pub use manual::{ManualPdfStats, manual_pdf, manual_pdf_with_stats};
pub use pdf::{diagram_pdf, diagram_svg_pages};
