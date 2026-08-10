#[path = "support/devil_fixture.rs"]
mod devil_fixture;

use ori3_model::{Document, Paper};

#[test]
fn complete_checkpoint_serializer_contains_the_document_schema() {
    let document = Document::new(Paper {
        width_mm: 250.0,
        height_mm: 250.0,
    });
    let json = devil_fixture::document_json(&document);
    assert!(json.starts_with("{\n  \"schema_version\":1,"));
    assert!(json.contains("\"cp\":{"));
    assert!(json.contains("\"next_vertex_id\":4"));
    assert!(json.contains("\"sequence\":[]"));
    assert!(json.contains("\"display\":{"));
    assert!(json.ends_with("}\n"));
}

