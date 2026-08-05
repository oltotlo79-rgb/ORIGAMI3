//! Document のJSON往復(serialize→deserialize)と Document::new の検証。

use ori3_model::*;

/// 各型を一通り含む Document を組み立てる。
fn sample_document() -> Document {
    Document {
        schema_version: SCHEMA_VERSION,
        paper: Paper {
            width_mm: 150.0,
            height_mm: 100.0,
        },
        cp: CreasePattern {
            vertices: vec![
                Vertex {
                    id: 0,
                    pos: [0.0, 0.0],
                },
                Vertex {
                    id: 1,
                    pos: [1.0, 0.0],
                },
                Vertex {
                    id: 2,
                    pos: [0.5, 0.5],
                },
            ],
            edges: vec![
                Edge {
                    id: 0,
                    v0: 0,
                    v1: 1,
                    kind: EdgeKind::Border,
                },
                Edge {
                    id: 1,
                    v0: 1,
                    v1: 2,
                    kind: EdgeKind::Mountain,
                },
                Edge {
                    id: 2,
                    v0: 2,
                    v1: 0,
                    kind: EdgeKind::Valley,
                },
            ],
            next_vertex_id: 3,
            next_edge_id: 3,
        },
        sequence: vec![
            FoldStep {
                id: 0,
                kind: TechniqueKind::Simple,
                drivers: vec![DriverLine {
                    a: [0.5, 0.0],
                    b: [0.5, 1.0],
                    target_angle_deg: 180.0,
                }],
                layer_order: Some(vec![[0.25, 0.25], [0.75, 0.25]]),
                note: "半分に折る".to_string(),
            },
            FoldStep {
                id: 1,
                kind: TechniqueKind::Pose,
                drivers: vec![DriverLine {
                    a: [0.0, 0.0],
                    b: [0.5, 0.5],
                    target_angle_deg: -90.0,
                }],
                layer_order: None,
                note: String::new(),
            },
        ],
        display: DisplaySettings {
            front_color: [237, 28, 36],
            back_color: [255, 255, 255],
            grid_divisions: 8,
        },
    }
}

#[test]
fn test_document_json_roundtrip() {
    let doc = sample_document();
    let json = serde_json::to_string_pretty(&doc).expect("serialize");
    let back: Document = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(doc, back);
}

#[test]
fn test_edit_op_tagged_roundtrip() {
    // EditOp は #[serde(tag = "type")] の内部タグ形式で往復できること。
    let op = EditOp::AddSegment {
        a: [0.0, 0.0],
        b: [1.0, 1.0],
        kind: EdgeKind::Aux,
    };
    let json = serde_json::to_string(&op).expect("serialize");
    assert!(json.contains("\"type\":\"AddSegment\""), "json = {json}");
    let back: EditOp = serde_json::from_str(&json).expect("deserialize");
    match back {
        EditOp::AddSegment { a, b, kind } => {
            assert_eq!(a, [0.0, 0.0]);
            assert_eq!(b, [1.0, 1.0]);
            assert_eq!(kind, EdgeKind::Aux);
        }
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[test]
fn test_seq_op_tagged_roundtrip() {
    let op = SeqOp::RemoveStep { id: 7 };
    let json = serde_json::to_string(&op).expect("serialize");
    assert!(json.contains("\"type\":\"RemoveStep\""), "json = {json}");
    let back: SeqOp = serde_json::from_str(&json).expect("deserialize");
    match back {
        SeqOp::RemoveStep { id } => assert_eq!(id, 7),
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[test]
fn test_document_new_landscape() {
    // 150×100mm: 長辺=幅が1.0、高さは 100/150 = 2/3 に正規化される。
    let doc = Document::new(Paper {
        width_mm: 150.0,
        height_mm: 100.0,
    });
    assert_eq!(doc.schema_version, SCHEMA_VERSION);
    assert_eq!(doc.paper.width_mm, 150.0);
    assert_eq!(doc.paper.height_mm, 100.0);
    assert!(doc.sequence.is_empty());

    let cp = &doc.cp;
    assert_eq!(cp.vertices.len(), 4);
    assert_eq!(cp.edges.len(), 4);
    assert_eq!(cp.next_vertex_id, 4);
    assert_eq!(cp.next_edge_id, 4);

    let h = 100.0 / 150.0;
    let expected = [[0.0, 0.0], [1.0, 0.0], [1.0, h], [0.0, h]];
    for (i, exp) in expected.iter().enumerate() {
        let v = &cp.vertices[i];
        assert_eq!(v.id, i as VertexId);
        assert!(
            (v.pos[0] - exp[0]).abs() < EPS && (v.pos[1] - exp[1]).abs() < EPS,
            "vertex {i}: got {:?}, expected {exp:?}",
            v.pos
        );
    }
    for (i, e) in cp.edges.iter().enumerate() {
        assert_eq!(e.id, i as EdgeId);
        assert_eq!(e.kind, EdgeKind::Border);
        assert_eq!(e.v0, i as VertexId);
        assert_eq!(e.v1, ((i + 1) % 4) as VertexId);
    }

    assert_eq!(doc.display.front_color, [237, 28, 36]);
    assert_eq!(doc.display.back_color, [255, 255, 255]);
    assert_eq!(doc.display.grid_divisions, 8);
}

#[test]
#[should_panic(expected = "紙のサイズは正の値でなければならない")]
fn test_document_new_rejects_zero_size() {
    let _ = Document::new(Paper {
        width_mm: 0.0,
        height_mm: 100.0,
    });
}

#[test]
#[should_panic(expected = "紙のサイズは正の値でなければならない")]
fn test_document_new_rejects_negative_size() {
    let _ = Document::new(Paper {
        width_mm: 150.0,
        height_mm: -10.0,
    });
}

#[test]
fn test_document_new_portrait_and_square() {
    // 縦長: 長辺=高さが1.0、幅は 100/150 = 2/3。
    let doc = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 150.0,
    });
    let w = 100.0 / 150.0;
    assert!((doc.cp.vertices[2].pos[0] - w).abs() < EPS);
    assert!((doc.cp.vertices[2].pos[1] - 1.0).abs() < EPS);

    // 正方形: 1.0×1.0。
    let doc = Document::new(Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    });
    assert!((doc.cp.vertices[2].pos[0] - 1.0).abs() < EPS);
    assert!((doc.cp.vertices[2].pos[1] - 1.0).abs() < EPS);
}
