//! FOLD 1.1/1.2取込とFOLD 1.2書出しのうち、ORIGAMI3が扱う限定profileの中立なJSON層。
//!
//! parse・profile検証・write・canonical比較に加え、ORIGAMI3 modelとの
//! 警告付き双方向変換を扱う。保存transactionと画面表示は後続段階の責務とする。

mod canonical;
mod conversion;
mod document_conversion;
mod parser;
mod types;
mod validation;
mod writer;

pub use canonical::{canonicalize_fold_1_2, compare_fold_1_2};
pub use conversion::{FoldConversionError, FoldImport, fold_to_document};
pub use document_conversion::{FoldExport, document_to_fold};
pub use parser::parse_fold_1_2;
pub use types::{
    FOLD_1_2_PROFILE_NAME, FOLD_1_2_UNSUPPORTED_FEATURES, FoldAssignment, FoldComparison,
    FoldComparisonOptions, FoldDifference, FoldFile, FoldFrame, FoldIssue, FoldIssueCode,
    FoldIssueSeverity, FoldParseError, FoldParseErrorKind, FoldValidation, FoldWriteError,
};
pub use validation::{unsupported_fields, validate_fold_1_2};
pub use writer::write_fold_1_2;
