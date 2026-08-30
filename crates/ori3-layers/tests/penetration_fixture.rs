use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use ori3_cp::{extract_faces, validate};
use ori3_layers::{PoseStepInput, ReplayResult, apply_pose_step, replay_with_faces};
use ori3_model::{
    CreasePattern, Document, DriverLine, Edge, EdgeKind, FaceId, Paper, SCHEMA_VERSION,
    TechniqueKind, Vertex,
};
use ori3_rigid::{
    PENETRATION_WARNING, contact_metrics, max_seam_gap, self_intersection_pairs,
    suspect_hinges_for_intersections,
};

const LEFT_CREASE_X: f64 = 0.45;
const RIGHT_CREASE_X: f64 = 0.65;
const LEFT_HINGE: u32 = 8;
const RIGHT_HINGE: u32 = 9;
const LEFT_ANGLE_DEG: f64 = -179.0;
const RIGHT_ANGLE_DEG: f64 = -179.0;
const EXPECTED_INTERSECTIONS: [(FaceId, FaceId); 1] = [(0, 2)];
const EXPECTED_SUSPECTS: [u32; 2] = [LEFT_HINGE, RIGHT_HINGE];

// Measured on 2026-08-27 in all three independent generator runs:
// max_seam_gap=5.55111512312578270e-17. The 1e-12 boundary leaves more than
// four decimal orders of numerical margin while remaining six orders below
// apply_pose_step's rejection boundary of 1e-6.
const MAX_SEAM_GAP: f64 = 1.0e-12;
// Measured max_abs_z=8.72434255841859907e-3 in all three runs. The lower
// boundary is about 79% of that value, so small last-bit changes cannot make a
// genuinely non-flat Pose look flat.
const MIN_MAX_ABS_Z: f64 = 6.9e-3;
// Measured max_penetration=3.48948128745668867e-3 in all three runs. The lower
// boundary is about 79% of that value and remains 2,750 times the 1e-6 contact
// tolerance, separating an interior piercing from mere contact.
const MIN_MAX_PENETRATION: f64 = 2.75e-3;

fn vertex(id: u32, x: f64, y: f64) -> Vertex {
    Vertex { id, pos: [x, y] }
}

fn edge(id: u32, v0: u32, v1: u32, kind: EdgeKind) -> Edge {
    Edge { id, v0, v1, kind }
}

fn pose_driver(x: f64, target_angle_deg: f64) -> DriverLine {
    DriverLine {
        a: [x, 0.0],
        b: [x, 1.0],
        target_angle_deg,
    }
}

fn penetration_warning_document() -> Document {
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    document.display.overlap_prevention_enabled = false;
    document.display.penetration_prevention_enabled = true;
    document.cp = CreasePattern {
        vertices: vec![
            vertex(0, 0.0, 0.0),
            vertex(1, LEFT_CREASE_X, 0.0),
            vertex(2, RIGHT_CREASE_X, 0.0),
            vertex(3, 1.0, 0.0),
            vertex(4, 1.0, 1.0),
            vertex(5, RIGHT_CREASE_X, 1.0),
            vertex(6, LEFT_CREASE_X, 1.0),
            vertex(7, 0.0, 1.0),
        ],
        edges: vec![
            edge(0, 0, 1, EdgeKind::Border),
            edge(1, 1, 2, EdgeKind::Border),
            edge(2, 2, 3, EdgeKind::Border),
            edge(3, 3, 4, EdgeKind::Border),
            edge(4, 4, 5, EdgeKind::Border),
            edge(5, 5, 6, EdgeKind::Border),
            edge(6, 6, 7, EdgeKind::Border),
            edge(7, 7, 0, EdgeKind::Border),
            edge(LEFT_HINGE, 1, 6, EdgeKind::Valley),
            edge(RIGHT_HINGE, 2, 5, EdgeKind::Valley),
        ],
        next_vertex_id: 8,
        next_edge_id: 10,
    };

    let applied = apply_pose_step(
        &mut document,
        PoseStepInput {
            driver_updates: vec![
                pose_driver(LEFT_CREASE_X, LEFT_ANGLE_DEG),
                pose_driver(RIGHT_CREASE_X, RIGHT_ANGLE_DEG),
            ],
            note: "penetration warning acceptance".to_owned(),
        },
    )
    .expect("the two-hinge Pose must be replayable");
    assert!(
        applied.max_seam_gap < MAX_SEAM_GAP,
        "generated Pose seam gap is too large: {:.17e}",
        applied.max_seam_gap
    );
    document
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/penetration-warning.ori3")
}

fn write_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                write!(output, "\\u{:04x}", character as u32)
                    .expect("writing JSON to a String cannot fail");
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

fn serialize_document(document: &Document) -> String {
    let mut output = String::new();
    write!(
        output,
        "{{\n  \"schema_version\": {},\n  \"paper\": {{ \"width_mm\": {:?}, \"height_mm\": {:?} }},\n  \"cp\": {{\n    \"vertices\": [\n",
        document.schema_version, document.paper.width_mm, document.paper.height_mm
    )
    .expect("writing JSON to a String cannot fail");
    for (index, vertex) in document.cp.vertices.iter().enumerate() {
        writeln!(
            output,
            "      {{ \"id\": {}, \"pos\": [{:?}, {:?}] }}{}",
            vertex.id,
            vertex.pos[0],
            vertex.pos[1],
            if index + 1 == document.cp.vertices.len() {
                ""
            } else {
                ","
            }
        )
        .expect("writing JSON to a String cannot fail");
    }
    output.push_str("    ],\n    \"edges\": [\n");
    for (index, edge) in document.cp.edges.iter().enumerate() {
        writeln!(
            output,
            "      {{ \"id\": {}, \"v0\": {}, \"v1\": {}, \"kind\": \"{:?}\" }}{}",
            edge.id,
            edge.v0,
            edge.v1,
            edge.kind,
            if index + 1 == document.cp.edges.len() {
                ""
            } else {
                ","
            }
        )
        .expect("writing JSON to a String cannot fail");
    }
    write!(
        output,
        "    ],\n    \"next_vertex_id\": {},\n    \"next_edge_id\": {}\n  }},\n  \"sequence\": [\n",
        document.cp.next_vertex_id, document.cp.next_edge_id
    )
    .expect("writing JSON to a String cannot fail");
    for (step_index, step) in document.sequence.iter().enumerate() {
        assert!(
            step.alignment.is_none(),
            "fixture serializer omits alignment"
        );
        assert!(
            step.finish_soft.is_none(),
            "fixture serializer omits finish_soft"
        );
        write!(
            output,
            "    {{\n      \"id\": {},\n      \"kind\": \"{:?}\",\n      \"drivers\": [\n",
            step.id, step.kind
        )
        .expect("writing JSON to a String cannot fail");
        for (driver_index, driver) in step.drivers.iter().enumerate() {
            writeln!(
                output,
                "        {{ \"a\": [{:?}, {:?}], \"b\": [{:?}, {:?}], \"target_angle_deg\": {:?} }}{}",
                driver.a[0],
                driver.a[1],
                driver.b[0],
                driver.b[1],
                driver.target_angle_deg,
                if driver_index + 1 == step.drivers.len() {
                    ""
                } else {
                    ","
                }
            )
            .expect("writing JSON to a String cannot fail");
        }
        output.push_str("      ],\n      \"layer_order\": ");
        if let Some(layer_order) = &step.layer_order {
            output.push('[');
            for (index, point) in layer_order.iter().enumerate() {
                if index > 0 {
                    output.push_str(", ");
                }
                write!(output, "[{:?}, {:?}]", point[0], point[1])
                    .expect("writing JSON to a String cannot fail");
            }
            output.push(']');
        } else {
            output.push_str("null");
        }
        output.push_str(",\n      \"note\": ");
        write_json_string(&mut output, &step.note);
        write!(
            output,
            "\n    }}{}\n",
            if step_index + 1 == document.sequence.len() {
                ""
            } else {
                ","
            }
        )
        .expect("writing JSON to a String cannot fail");
    }
    let display = &document.display;
    write!(
        output,
        concat!(
            "  ],\n  \"display\": {{\n",
            "    \"front_color\": [{}, {}, {}],\n",
            "    \"back_color\": [{}, {}, {}],\n",
            "    \"grid_divisions\": {},\n",
            "    \"soft_enabled\": {},\n",
            "    \"soft_stiffness\": {:?},\n",
            "    \"soft_pressure\": {:?},\n",
            "    \"overlap_prevention_enabled\": {},\n",
            "    \"penetration_prevention_enabled\": {}\n",
            "  }}\n}}\n"
        ),
        display.front_color[0],
        display.front_color[1],
        display.front_color[2],
        display.back_color[0],
        display.back_color[1],
        display.back_color[2],
        display.grid_divisions,
        display.soft_enabled,
        display.soft_stiffness,
        display.soft_pressure,
        display.overlap_prevention_enabled,
        display.penetration_prevention_enabled
    )
    .expect("writing JSON to a String cannot fail");
    output
}

fn attach_penetration_warning_like_store(
    replayed: &mut ReplayResult,
    intersections: &[(FaceId, FaceId)],
) {
    let mut added: Vec<&'static str> = Vec::new();
    if !intersections.is_empty()
        && !replayed
            .frame
            .warnings
            .iter()
            .any(|warning| warning == PENETRATION_WARNING)
    {
        replayed.frame.warnings.push(PENETRATION_WARNING.to_owned());
        added.push(PENETRATION_WARNING);
    }
    for warning in added {
        if !replayed.warnings.iter().any(|existing| existing == warning) {
            replayed.warnings.push(warning.to_owned());
        }
    }
}

fn assert_penetration_acceptance(document: &Document) {
    assert_eq!(document.schema_version, SCHEMA_VERSION);
    assert!(document.display.penetration_prevention_enabled);
    assert!(!document.display.overlap_prevention_enabled);

    let cp_warnings = validate(&document.cp);
    assert!(
        cp_warnings.is_empty(),
        "fixture CP validation warnings: {cp_warnings:?}"
    );
    let faces = extract_faces(&document.cp);
    assert_eq!(faces.len(), 3, "fixture must remain a three-panel chain");
    assert_eq!(document.cp.vertices.len(), 8);
    assert_eq!(document.cp.edges.len(), 10);
    let fold_crease_count = document
        .cp
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .count();
    assert_eq!(fold_crease_count, 2);
    assert_eq!(document.sequence.len(), 1);
    assert_eq!(document.sequence[0].kind, TechniqueKind::Pose);
    assert_eq!(document.sequence[0].drivers.len(), 2);
    assert_eq!(
        document.sequence[0]
            .drivers
            .iter()
            .map(|driver| driver.target_angle_deg)
            .collect::<Vec<_>>(),
        vec![LEFT_ANGLE_DEG, RIGHT_ANGLE_DEG]
    );
    assert!(document.sequence[0].layer_order.is_none());

    let mut replayed = replay_with_faces(document, &faces, document.sequence.len(), 1.0);
    assert!(
        replayed.skipped.is_empty(),
        "raw skipped: {:?}",
        replayed.skipped
    );
    assert!(
        replayed.warnings.is_empty(),
        "raw replay warnings: {:?}",
        replayed.warnings
    );
    assert!(
        replayed.frame.warnings.is_empty(),
        "raw frame warnings: {:?}",
        replayed.frame.warnings
    );

    let intersections = self_intersection_pairs(&replayed.frame);
    assert_eq!(intersections, EXPECTED_INTERSECTIONS);
    assert_eq!(replayed.driver_hinges, EXPECTED_SUSPECTS);
    let suspects = suspect_hinges_for_intersections(
        &document.cp,
        &faces,
        &intersections,
        &replayed.driver_hinges,
    );
    assert_eq!(suspects, EXPECTED_SUSPECTS);

    let seam_gap = max_seam_gap(&document.cp, &faces, &replayed.frame);
    let max_abs_z = replayed
        .frame
        .faces
        .iter()
        .flat_map(|face| face.polygon.iter())
        .map(|point| point[2].abs())
        .fold(0.0_f64, f64::max);
    let metrics = contact_metrics(&replayed.frame);
    assert_eq!(metrics.pair_count, intersections.len());
    assert!(seam_gap < MAX_SEAM_GAP, "max_seam_gap={seam_gap:.17e}");
    assert!(max_abs_z > MIN_MAX_ABS_Z, "max_abs_z={max_abs_z:.17e}");
    assert!(
        metrics.max_penetration > MIN_MAX_PENETRATION,
        "max_penetration={:.17e}",
        metrics.max_penetration
    );

    println!(
        "penetration fixture: faces={} fold_creases={} intersections={intersections:?} suspects={suspects:?} max_seam_gap={seam_gap:.17e} max_abs_z={max_abs_z:.17e} pair_count={} max_penetration={:.17e} total_penetration={:.17e}",
        faces.len(),
        fold_crease_count,
        metrics.pair_count,
        metrics.max_penetration,
        metrics.total_penetration
    );

    attach_penetration_warning_like_store(&mut replayed, &intersections);
    let replay_penetration_count = replayed
        .warnings
        .iter()
        .filter(|warning| warning.as_str() == PENETRATION_WARNING)
        .count();
    let frame_penetration_count = replayed
        .frame
        .warnings
        .iter()
        .filter(|warning| warning.as_str() == PENETRATION_WARNING)
        .count();
    assert_eq!(replayed.warnings.len(), 1, "no unrelated replay warning");
    assert_eq!(replay_penetration_count, 1);
    assert_eq!(
        replayed.frame.warnings.len(),
        1,
        "no unrelated frame warning"
    );
    assert_eq!(frame_penetration_count, 1);

    // Stage 1 records the JSX mapping: a non-empty final warning list renders one
    // status badge, and a non-empty suspect list renders one cause-candidate guide.
    // These are input cardinalities, not a claim that this Rust test renders the UI.
    let status_badge_count = usize::from(!replayed.warnings.is_empty());
    let cause_guide_count = usize::from(!suspects.is_empty());
    assert_eq!(status_badge_count, 1);
    assert_eq!(cause_guide_count, 1);
}

#[test]
#[ignore = "tracked fixture is regenerated only by an explicit command"]
fn regenerate_penetration_warning_fixture() {
    let generated = (0..3)
        .map(|_| {
            let document = penetration_warning_document();
            assert_penetration_acceptance(&document);
            serialize_document(&document).into_bytes()
        })
        .collect::<Vec<_>>();
    assert_eq!(generated[0], generated[1], "generation 1 and 2 differ");
    assert_eq!(generated[0], generated[2], "generation 1 and 3 differ");

    let path = fixture_path();
    std::fs::create_dir_all(path.parent().expect("fixture directory"))
        .expect("create fixture directory");
    std::fs::write(&path, &generated[0]).expect("write penetration warning fixture");
}

#[test]
fn penetration_warning_fixture_intersects_on_replay() {
    let stored = std::fs::read(fixture_path()).expect("read tracked penetration fixture");
    let stored_text = std::str::from_utf8(&stored).expect("tracked fixture is UTF-8");
    let schema_marker = format!("\"schema_version\": {SCHEMA_VERSION}");
    assert_eq!(stored_text.matches(&schema_marker).count(), 1);
    assert_eq!(stored_text.matches("\"kind\": \"Pose\"").count(), 1);
    assert_eq!(stored_text.matches("\"layer_order\": null").count(), 1);
    for key in [
        "front_color",
        "back_color",
        "grid_divisions",
        "soft_enabled",
        "soft_stiffness",
        "soft_pressure",
        "overlap_prevention_enabled",
        "penetration_prevention_enabled",
    ] {
        assert_eq!(
            stored_text.matches(&format!("\"{key}\"")).count(),
            1,
            "display key {key}"
        );
    }
    assert_eq!(
        stored_text
            .matches("\"overlap_prevention_enabled\": false")
            .count(),
        1
    );
    assert_eq!(
        stored_text
            .matches("\"penetration_prevention_enabled\": true")
            .count(),
        1
    );
    for forbidden in [
        "step_creases",
        "\"alignment\"",
        "\"finish_soft\"",
        "NaN",
        "Infinity",
        "inf",
    ] {
        assert!(
            !stored_text.contains(forbidden),
            "fixture contains forbidden token {forbidden}"
        );
    }
    assert!(!stored_text.contains('\r'), "fixture must use LF newlines");
    assert!(
        stored_text.ends_with('\n') && !stored_text.ends_with("\n\n"),
        "fixture must end in exactly one LF"
    );
    let document = penetration_warning_document();
    let generated = serialize_document(&document).into_bytes();
    // The checked-in bytes equal a fresh public-API construction byte-for-byte.
    // Replaying that constructed Document therefore replays the exact stored data
    // without adding a JSON dependency to ori3-layers.
    assert_eq!(
        stored, generated,
        "tracked fixture needs explicit regeneration"
    );
    assert_penetration_acceptance(&document);
}
