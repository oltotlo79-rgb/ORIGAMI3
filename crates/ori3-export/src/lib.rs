//! ori3-export: 展開図・折り図の書き出し(SVG/PNG/PDF)。

pub mod cp_png;
pub mod cp_svg;
pub mod diagram;
pub mod document;
pub mod manual;
pub mod pdf;

pub use cp_png::{DEFAULT_LONG_SIDE_PX, MAX_LONG_SIDE_PX, cp_png};
pub use cp_svg::{CpSvgOptions, cp_svg};
pub use diagram::render_step;
pub use document::{
    SoftGeometrySnapshot, document_json, document_with_soft_geometry_from_json,
    document_with_soft_geometry_json, save_document, save_document_with_soft_geometry,
};
pub use manual::{ManualPdfStats, manual_pdf, manual_pdf_with_stats};
pub use pdf::{diagram_pdf, diagram_svg_pages};
