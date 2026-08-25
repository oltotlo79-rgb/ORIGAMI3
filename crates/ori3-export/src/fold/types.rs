use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde_json::Value;

/// 利用者へ示すprofile名。表示側もこの正確な名称を使う。
pub const FOLD_1_2_PROFILE_NAME: &str = "FOLD 1.2 限定";

/// 利用者から1操作以内で到達できる場所へ示す、対応外の7項目。
pub const FOLD_1_2_UNSUPPORTED_FEATURES: [&str; 7] = [
    "3D座標",
    "枝分かれした手順",
    "動画",
    "名前付き技法の意味",
    "注記",
    "仕上げの丸み",
    "FOLDの「平ら(F)」「未指定(U)」の区別",
];

/// FOLDのedge assignment。限定profileでexact対象なのはB/M/Vだけであり、
/// F/Uはvalidatorが元値とpathを残してAuxへの縮退を警告する。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoldAssignment {
    Border,
    Mountain,
    Valley,
    Flat,
    Unassigned,
    Other(String),
}

impl FoldAssignment {
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        match code {
            "B" => Self::Border,
            "M" => Self::Mountain,
            "V" => Self::Valley,
            "F" => Self::Flat,
            "U" => Self::Unassigned,
            other => Self::Other(other.to_string()),
        }
    }

    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::Border => "B",
            Self::Mountain => "M",
            Self::Valley => "V",
            Self::Flat => "F",
            Self::Unassigned => "U",
            Self::Other(code) => code,
        }
    }
}

/// root frameまたは`file_frames`内の1 frame。
///
/// 可変長配列の長さやindex範囲はparserで捨てず、validatorがJSON path付きで
/// 判定できるように保持する。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FoldFrame {
    pub frame_title: Option<String>,
    pub frame_description: Option<String>,
    pub frame_classes: Vec<String>,
    pub frame_attributes: Vec<String>,
    pub frame_parent: Option<usize>,
    pub frame_inherit: Option<bool>,
    pub vertices_coords: Option<Vec<Vec<f64>>>,
    pub edges_vertices: Option<Vec<Vec<usize>>>,
    pub edges_assignment: Option<Vec<FoldAssignment>>,
    pub edges_fold_angle: Option<Vec<Option<f64>>>,
    pub faces_vertices: Option<Vec<Vec<usize>>>,
    pub face_orders: Option<Vec<Vec<i64>>>,
    pub extra_fields: BTreeMap<String, Value>,
}

/// FOLD 1.1/1.2入力とFOLD 1.2出力に共通するtyped JSON表現。
#[derive(Clone, Debug, PartialEq)]
pub struct FoldFile {
    pub file_spec: f64,
    pub file_creator: Option<String>,
    pub file_author: Option<String>,
    pub file_title: Option<String>,
    pub file_description: Option<String>,
    pub file_classes: Vec<String>,
    pub root: FoldFrame,
    pub file_frames: Vec<FoldFrame>,
    pub extra_fields: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldParseErrorKind {
    InvalidJson,
    RootNotObject,
    MissingField,
    InvalidType,
    InvalidValue,
    UnsupportedVersion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldParseError {
    pub kind: FoldParseErrorKind,
    pub path: String,
    pub message: String,
}

impl FoldParseError {
    pub(crate) fn new(
        kind: FoldParseErrorKind,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for FoldParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl Error for FoldParseError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldIssueSeverity {
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldIssueCode {
    AssignmentDowngradedToAux,
    UnsupportedField,
    UnsupportedGeometry,
    NonLinearFrames,
    UnrepresentableFaceOrders,
    InvalidTopology,
    MissingRequiredField,
    InvalidValue,
}

/// validatorが返すpath付きの警告または拒否理由。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoldIssue {
    pub severity: FoldIssueSeverity,
    pub code: FoldIssueCode,
    pub path: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_value: Option<Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FoldValidation {
    pub warnings: Vec<FoldIssue>,
    pub errors: Vec<FoldIssue>,
}

impl FoldValidation {
    #[must_use]
    pub fn is_supported(&self) -> bool {
        self.errors.is_empty()
    }

    pub(crate) fn warning(&mut self, issue: FoldIssue) {
        self.warnings.push(issue);
    }

    pub(crate) fn error(&mut self, issue: FoldIssue) {
        self.errors.push(issue);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FoldWriteError {
    pub message: String,
    pub issues: Vec<FoldIssue>,
}

impl fmt::Display for FoldWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl Error for FoldWriteError {}

/// §12.6の実測境界。座標・角度はexact比較せず、それぞれのepsilonで比べる。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoldComparisonOptions {
    pub coordinate_epsilon: f64,
    pub angle_epsilon_deg: f64,
}

impl Default for FoldComparisonOptions {
    fn default() -> Self {
        Self {
            coordinate_epsilon: 1e-9,
            angle_epsilon_deg: 1e-9,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FoldDifference {
    pub path: String,
    pub message: String,
    pub left: Option<Value>,
    pub right: Option<Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FoldComparison {
    pub differences: Vec<FoldDifference>,
}

impl FoldComparison {
    #[must_use]
    pub fn is_equivalent(&self) -> bool {
        self.differences.is_empty()
    }
}
