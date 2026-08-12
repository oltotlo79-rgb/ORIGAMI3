//! Minimal, dependency-free serializer for complete `.ori3` checkpoint documents.
//!
//! `ori3-layers` intentionally has no `serde_json` dependency.  Acceptance tests still need
//! real documents (not the reduced face-only fixture used by the crane test), so this module
//! writes every persisted `Document` field explicitly.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use ori3_model::{
    AlignmentMode, AlignmentTarget, Document, EdgeKind, FoldAlignment, TechniqueKind,
};

fn edge_kind(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Border => "Border",
        EdgeKind::Mountain => "Mountain",
        EdgeKind::Valley => "Valley",
        EdgeKind::Aux => "Aux",
    }
}

fn technique(kind: TechniqueKind) -> &'static str {
    match kind {
        TechniqueKind::Simple => "Simple",
        TechniqueKind::Pleat => "Pleat",
        TechniqueKind::InsideReverse => "InsideReverse",
        TechniqueKind::OutsideReverse => "OutsideReverse",
        TechniqueKind::Petal => "Petal",
        TechniqueKind::Squash => "Squash",
        TechniqueKind::OpenSink => "OpenSink",
        TechniqueKind::Swivel => "Swivel",
        TechniqueKind::Twist => "Twist",
        TechniqueKind::Pose => "Pose",
    }
}

fn alignment_mode(mode: AlignmentMode) -> &'static str {
    match mode {
        AlignmentMode::ThroughTwoPoints => "throughTwoPoints",
        AlignmentMode::PointPoint => "pointPoint",
        AlignmentMode::LineLine => "lineLine",
        AlignmentMode::PointPerpendicularLine => "pointPerpendicularLine",
        AlignmentMode::PointLineThrough => "pointLineThrough",
        AlignmentMode::PointToLinePointToLine => "pointToLinePointToLine",
        AlignmentMode::PointLinePerpendicular => "pointLinePerpendicular",
        AlignmentMode::ExistingLine => "existingLine",
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => write!(out, "\\u{:04x}", u32::from(c)).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_point(out: &mut String, point: [f64; 2]) {
    write!(out, "[{:?},{:?}]", point[0], point[1]).unwrap();
}

fn write_alignment(out: &mut String, alignment: &FoldAlignment) {
    write!(
        out,
        "{{\"mode\":{},\"picks\":[",
        json_string(alignment_mode(alignment.mode))
    )
    .unwrap();
    for (index, pick) in alignment.picks.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        match pick {
            AlignmentTarget::Point { p } => {
                out.push_str("{\"kind\":\"point\",\"p\":");
                write_point(out, *p);
                out.push('}');
            }
            AlignmentTarget::Line { a, b } => {
                out.push_str("{\"kind\":\"line\",\"a\":");
                write_point(out, *a);
                out.push_str(",\"b\":");
                write_point(out, *b);
                out.push('}');
            }
        }
    }
    out.push_str("]}");
}

/// Serialize a complete `Document` in the schema consumed by ORIGAMI3.
#[must_use]
pub fn document_json(document: &Document) -> String {
    let mut out = String::with_capacity(
        256 + document.cp.vertices.len() * 80
            + document.cp.edges.len() * 80
            + document.sequence.len() * 160,
    );
    write!(
        out,
        "{{\n  \"schema_version\":{},\n  \"paper\":{{\"width_mm\":{:?},\"height_mm\":{:?}}},\n  \"cp\":{{\n    \"vertices\":[",
        document.schema_version, document.paper.width_mm, document.paper.height_mm
    )
    .unwrap();
    for (index, vertex) in document.cp.vertices.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write!(out, "{{\"id\":{},\"pos\":", vertex.id).unwrap();
        write_point(&mut out, vertex.pos);
        out.push('}');
    }
    out.push_str("],\n    \"edges\":[");
    for (index, edge) in document.cp.edges.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write!(
            out,
            "{{\"id\":{},\"v0\":{},\"v1\":{},\"kind\":{}}}",
            edge.id,
            edge.v0,
            edge.v1,
            json_string(edge_kind(edge.kind))
        )
        .unwrap();
    }
    write!(
        out,
        "],\n    \"next_vertex_id\":{},\n    \"next_edge_id\":{}\n  }},\n  \"sequence\":[",
        document.cp.next_vertex_id, document.cp.next_edge_id
    )
    .unwrap();
    for (index, step) in document.sequence.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write!(
            out,
            "{{\"id\":{},\"kind\":{},\"drivers\":[",
            step.id,
            json_string(technique(step.kind))
        )
        .unwrap();
        for (driver_index, driver) in step.drivers.iter().enumerate() {
            if driver_index > 0 {
                out.push(',');
            }
            out.push_str("{\"a\":");
            write_point(&mut out, driver.a);
            out.push_str(",\"b\":");
            write_point(&mut out, driver.b);
            write!(out, ",\"target_angle_deg\":{:?}}}", driver.target_angle_deg).unwrap();
        }
        out.push_str("],\"layer_order\":");
        match &step.layer_order {
            Some(points) => {
                out.push('[');
                for (point_index, point) in points.iter().enumerate() {
                    if point_index > 0 {
                        out.push(',');
                    }
                    write_point(&mut out, *point);
                }
                out.push(']');
            }
            None => out.push_str("null"),
        }
        if let Some(alignment) = &step.alignment {
            out.push_str(",\"alignment\":");
            write_alignment(&mut out, alignment);
        }
        write!(out, ",\"note\":{}}}", json_string(&step.note)).unwrap();
    }
    let display = &document.display;
    write!(
        out,
        "],\n  \"display\":{{\"front_color\":{:?},\"back_color\":{:?},\"grid_divisions\":{},\"soft_enabled\":{},\"soft_stiffness\":{:?},\"soft_pressure\":{:?},\"overlap_prevention_enabled\":{},\"penetration_prevention_enabled\":{}}}\n}}\n",
        display.front_color,
        display.back_color,
        display.grid_divisions,
        display.soft_enabled,
        display.soft_stiffness,
        display.soft_pressure,
        display.overlap_prevention_enabled,
        display.penetration_prevention_enabled,
    )
    .unwrap();
    out
}

/// 悪魔のチェックポイントを1つ書き出す。
///
/// 既定の書き出し先は一時フォルダ。以前は `tests/fixtures/` を直接上書きしていたため、
/// テストを走らせるたびにコミット済みのfixtureが最終桁だけ書き換わり、作業ツリーが
/// 汚れた。同じファイルを `folded_query.rs` や `step_oracle.rs` が読むので、
/// テストの実行順で読む内容が変わる状態にもなっていた。
///
/// fixtureを更新したいときだけ `ORI3_DEVIL_FIXTURE_OUT=<保存先フォルダ>` を指定する。
#[allow(dead_code)] // Used by acceptance_devil; the serializer unit test includes this module too.
pub fn write_checkpoint(document: &Document, step: u32) -> PathBuf {
    let directory = std::env::var_os("ORI3_DEVIL_FIXTURE_OUT").map_or_else(
        || std::env::temp_dir().join("ori3-devil-checkpoints"),
        |out| Path::new(&out).to_path_buf(),
    );
    std::fs::create_dir_all(&directory).expect("create Devil fixture directory");
    let path = directory.join(format!("devil-{step:03}.ori3"));
    std::fs::write(&path, document_json(document)).expect("write complete Devil checkpoint");
    path
}
