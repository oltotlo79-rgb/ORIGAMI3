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
                alignment: Some(FoldAlignment {
                    mode: AlignmentMode::PointPoint,
                    picks: vec![
                        AlignmentTarget::Point { p: [1.0, 0.0] },
                        AlignmentTarget::Point { p: [0.0, 1.0] },
                    ],
                }),
                finish_soft: None,
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
                alignment: None,
                finish_soft: None,
                note: String::new(),
            },
        ],
        display: DisplaySettings::default(),
    }
}

/// 種を固定した決定的な擬似乱数(splitmix64)。作品ファイルへ入れる座標を作る。
///
/// 実行するたびに同じ値の列を作るので、どの計算機で走らせても同じ検査になる。
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 0.0以上1.0未満の小数を、仮数部53ビットをすべて使って決定的に作る。
fn next_unit(state: &mut u64) -> f64 {
    ((splitmix64(state) >> 11) as f64) / ((1u64 << 53) as f64)
}

/// 作品ファイルを書き出して読み直したとき、座標が1ビットも変わらないこと。
///
/// `serde_json` は既定では小数の読み込みを「およその精度」で行い、最下位1ビットを
/// 落とすことがある。実測(2026-08-16、20万個の往復)では、既定のままだと
/// 全範囲で 59,621/200,000 個(29.8%)、0〜1000の範囲で 20,888/200,000 個(10.4%)が
/// ずれた。ワークスペースの `Cargo.toml` で `serde_json` の `float_roundtrip` を
/// 有効にすると、同じ測定で 0 個になる。
///
/// この検査は、その指定が外れたら落ちる。実際に `float_roundtrip` を外して実行し、
/// 往復させた 8,752個 の小数のうち 926個 でビットが変わって失敗すること、
/// 最初の不一致が下の具体値(`0.20710678118654754` → `0.20710678118654752`)であることを
/// 確認した(2026-08-16)。
#[test]
fn saved_document_coordinates_survive_json_roundtrip_bit_for_bit() {
    // 実際にずれた具体値。既定の読み込みでは ...ef33 が ...ef32 になる。
    let known_bad = 0.207_106_781_186_547_54_f64;
    assert_eq!(
        known_bad.to_bits(),
        0x3fca_8279_99fc_ef33,
        "検査に使う具体値のビットが変わっている"
    );

    let mut state: u64 = 0x0123_4567_89AB_CDEF;

    // 頂点座標: 3,000点 = 6,000個。1点目は必ず上の具体値にする。
    let mut vertices = vec![Vertex {
        id: 0,
        pos: [known_bad, known_bad],
    }];
    while vertices.len() < 3_000 {
        // 展開図の正規化座標(0〜1)と、mm相当の大きさ(0〜1000)の両方を混ぜる
        let scale = if vertices.len() % 2 == 0 { 1.0 } else { 1000.0 };
        let x = next_unit(&mut state) * scale;
        let y = next_unit(&mut state) * scale;
        vertices.push(Vertex {
            id: vertices.len() as VertexId,
            pos: [x, y],
        });
    }

    // 折り手順の駆動線(1手順あたり4個)と、手順ごとの追加折り線(1本あたり4個)にも
    // 同じ検査を効かせる。作品ファイルに入る小数は頂点座標だけではないため。
    let mut sequence = Vec::new();
    let mut step_creases = Vec::new();
    for step in 0..250u32 {
        sequence.push(FoldStep {
            id: step,
            kind: TechniqueKind::Simple,
            drivers: vec![DriverLine {
                a: [next_unit(&mut state), next_unit(&mut state)],
                b: [next_unit(&mut state), next_unit(&mut state)],
                target_angle_deg: next_unit(&mut state) * 360.0 - 180.0,
            }],
            layer_order: Some(vec![[next_unit(&mut state), next_unit(&mut state)]]),
            alignment: None,
            finish_soft: None,
            note: String::new(),
        });
        step_creases.push(StepCreases {
            step,
            lines: vec![[
                [next_unit(&mut state), next_unit(&mut state)],
                [next_unit(&mut state), next_unit(&mut state)],
            ]],
        });
    }

    let mut document = sample_document();
    document.paper = Paper {
        width_mm: next_unit(&mut state) * 300.0,
        height_mm: next_unit(&mut state) * 300.0,
    };
    document.cp.next_vertex_id = vertices.len() as VertexId;
    document.cp.vertices = vertices;
    document.cp.edges.clear();
    document.cp.next_edge_id = 0;
    document.sequence = sequence;
    let saved = SavedDocument {
        document,
        step_creases,
    };

    let json = serde_json::to_string_pretty(&saved).expect("serialize");
    let back: SavedDocument = serde_json::from_str(&json).expect("deserialize");

    // 往復させた小数を1つずつ数え、ビットが違うものを数える。
    let mut checked = 0usize;
    let mut changed = 0usize;
    let mut first_difference: Option<(f64, f64)> = None;
    let mut compare = |before: f64, after: f64| {
        checked += 1;
        if before.to_bits() != after.to_bits() {
            changed += 1;
            first_difference.get_or_insert((before, after));
        }
    };

    compare(saved.document.paper.width_mm, back.document.paper.width_mm);
    compare(
        saved.document.paper.height_mm,
        back.document.paper.height_mm,
    );
    for (before, after) in saved
        .document
        .cp
        .vertices
        .iter()
        .zip(back.document.cp.vertices.iter())
    {
        for axis in 0..2 {
            compare(before.pos[axis], after.pos[axis]);
        }
    }
    for (before, after) in saved
        .document
        .sequence
        .iter()
        .zip(back.document.sequence.iter())
    {
        for (bd, ad) in before.drivers.iter().zip(after.drivers.iter()) {
            for axis in 0..2 {
                compare(bd.a[axis], ad.a[axis]);
                compare(bd.b[axis], ad.b[axis]);
            }
            compare(bd.target_angle_deg, ad.target_angle_deg);
        }
        let (Some(bo), Some(ao)) = (&before.layer_order, &after.layer_order) else {
            panic!("layer_order が往復で失われた");
        };
        for (bp, ap) in bo.iter().zip(ao.iter()) {
            for axis in 0..2 {
                compare(bp[axis], ap[axis]);
            }
        }
    }
    for (before, after) in saved.step_creases.iter().zip(back.step_creases.iter()) {
        for (bl, al) in before.lines.iter().zip(after.lines.iter()) {
            for end in 0..2 {
                for axis in 0..2 {
                    compare(bl[end][axis], al[end][axis]);
                }
            }
        }
    }

    assert!(
        checked >= 4_000,
        "往復させた座標が少なすぎる: {checked}個。4,000個以上を試すこと"
    );
    assert_eq!(
        changed, 0,
        "作品ファイルを読み直すと座標のビットが変わった: {checked}個中{changed}個。\
         最初の不一致 {first_difference:?}。ワークスペースの Cargo.toml で serde_json の \
         float_roundtrip が外れていないか確認する"
    );
    // 1点目に入れた具体値が、ビットまでそのまま戻ること。
    assert_eq!(
        back.document.cp.vertices[0].pos[0].to_bits(),
        0x3fca_8279_99fc_ef33
    );
    assert_eq!(
        back.document.cp.vertices[0].pos[1].to_bits(),
        0x3fca_8279_99fc_ef33
    );
    assert_eq!(saved, back, "作品ファイル全体が完全に一致すること");
}

#[test]
fn test_document_json_roundtrip() {
    let doc = sample_document();
    let json = serde_json::to_string_pretty(&doc).expect("serialize");
    assert!(
        !json.contains("\"mirrored\""),
        "表示用Face3Dは保存作品Documentへ入らない"
    );
    let back: Document = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(doc, back);
}

#[test]
fn old_fold_step_without_finish_soft_loads_as_unrecorded() {
    let old = r#"{"id":7,"kind":"Pose","drivers":[],"layer_order":null,"note":""}"#;
    let step: FoldStep = serde_json::from_str(old).expect("旧形式の仕上げ手順を読む");

    assert_eq!(step.finish_soft, None);
    assert!(
        !serde_json::to_string(&step)
            .expect("旧形式相当の手順を書き出す")
            .contains("finish_soft"),
        "未記録の任意欄は作品へ書き足さない"
    );
}

#[test]
fn finish_soft_round_trips_three_values_only_with_measured_tolerance() {
    let values = [
        FinishSoftSettings {
            enabled: true,
            stiffness: 0.17,
            pressure: 0.08,
        },
        FinishSoftSettings {
            enabled: false,
            stiffness: 0.52,
            pressure: 0.41,
        },
        FinishSoftSettings {
            enabled: true,
            stiffness: 0.88,
            pressure: 0.76,
        },
    ];
    let mut document = Document::new(Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    });
    document.sequence = values
        .iter()
        .enumerate()
        .map(|(index, &finish_soft)| FoldStep {
            id: u32::try_from(index).expect("3手はu32に収まる"),
            kind: TechniqueKind::Pose,
            drivers: Vec::new(),
            layer_order: None,
            alignment: None,
            finish_soft: Some(finish_soft),
            note: String::new(),
        })
        .collect();
    let saved = SavedDocument::new(document);

    let json = serde_json::to_string_pretty(&saved).expect("仕上げ値を保存する");
    let shape: serde_json::Value = serde_json::from_str(&json).expect("JSONの形を調べる");
    let expected_keys = std::collections::BTreeSet::from(["enabled", "pressure", "stiffness"]);
    for (index, step) in shape["sequence"]
        .as_array()
        .expect("sequenceは配列")
        .iter()
        .enumerate()
    {
        let finish = step["finish_soft"]
            .as_object()
            .unwrap_or_else(|| panic!("手順{}にfinish_softがある", index + 1));
        let actual_keys = finish
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            actual_keys, expected_keys,
            "仕上げ欄には3値だけを保存し、3D頂点や計算設定を含めない"
        );
    }

    let restored: SavedDocument = serde_json::from_str(&json).expect("仕上げ値を読み直す");
    let restored_values = restored
        .document
        .sequence
        .iter()
        .map(|step| step.finish_soft.expect("3手とも仕上げ値を持つ"))
        .collect::<Vec<_>>();

    // 実測最大差は0。既往のJSON最下位差1.11e-16に対して1e-12は約9,000倍の
    // 余裕を持ち、別の記録値どうしの最小差0.33より11桁以上小さいため別物を通さない。
    const TOLERANCE: f64 = 1e-12;
    let mut max_difference = 0.0_f64;
    for (expected, actual) in values.iter().zip(&restored_values) {
        assert_eq!(actual.enabled, expected.enabled);
        max_difference = max_difference.max((actual.stiffness - expected.stiffness).abs());
        max_difference = max_difference.max((actual.pressure - expected.pressure).abs());
    }
    let min_distinct_difference = values
        .windows(2)
        .flat_map(|pair| {
            [
                (pair[1].stiffness - pair[0].stiffness).abs(),
                (pair[1].pressure - pair[0].pressure).abs(),
            ]
        })
        .fold(f64::INFINITY, f64::min);
    assert!(
        min_distinct_difference > 0.32 && min_distinct_difference < 0.34,
        "別値を区別する最小差の実測は0.33: {min_distinct_difference}"
    );
    assert!(TOLERANCE < min_distinct_difference);
    assert!(
        max_difference <= TOLERANCE,
        "仕上げ値のJSON往復差 {max_difference:e} が実測根拠の許容差 {TOLERANCE:e} を超えた"
    );
    assert_eq!(max_difference, 0.0, "今回の実測最大差を固定する");
    assert_eq!(
        restored.document.schema_version, 1,
        "加算式の任意欄なので版1を維持する"
    );
}

#[test]
fn finish_soft_replay_uses_the_latest_completed_pose_at_each_position() {
    let a = FinishSoftSettings {
        enabled: true,
        stiffness: 0.17,
        pressure: 0.08,
    };
    let b = FinishSoftSettings {
        enabled: false,
        stiffness: 0.52,
        pressure: 0.41,
    };
    let c = FinishSoftSettings {
        enabled: true,
        stiffness: 0.88,
        pressure: 0.76,
    };
    let step = |id, kind, finish_soft| FoldStep {
        id,
        kind,
        drivers: Vec::new(),
        layer_order: None,
        alignment: None,
        finish_soft,
        note: String::new(),
    };
    let mut document = Document::new(Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    });
    document.sequence = vec![
        step(1, TechniqueKind::Pose, Some(a)),
        step(2, TechniqueKind::Simple, None),
        step(3, TechniqueKind::Pose, Some(b)),
        step(4, TechniqueKind::Pose, Some(c)),
    ];

    for (up_to, expected) in [(1, a), (2, a), (3, b), (4, c)] {
        assert_eq!(document.finish_soft_at(up_to, 1.0), Some(expected));
    }
    assert_eq!(document.finish_soft_at(3, 0.0), Some(a));
    assert_eq!(document.finish_soft_at(3, 0.5), Some(a));
    assert_eq!(document.finish_soft_at(3, 1.0), Some(b));
    assert_eq!(document.finish_soft_at(3, f64::NAN), Some(b));
    assert_eq!(document.finish_soft_at(usize::MAX, 1.0), Some(c));

    for step in &mut document.sequence {
        step.finish_soft = None;
    }
    assert_eq!(
        document.finish_soft_at(usize::MAX, 1.0),
        None,
        "全手順に欄がない旧作品は従来のDisplay値へ委ねる"
    );
}

#[test]
fn saved_document_keeps_step_creases_next_to_the_document_fields() {
    let saved = SavedDocument {
        document: sample_document(),
        step_creases: vec![
            StepCreases {
                step: 0,
                // 既にある折り筋で折った手順。線を1本も足していない
                lines: Vec::new(),
            },
            StepCreases {
                step: 1,
                lines: vec![[[0.0, 0.5], [1.0, 0.5]]],
            },
        ],
    };

    let json = serde_json::to_string_pretty(&saved).expect("serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("JSONとして読める");
    assert!(
        value.get("schema_version").is_some() && value.get("cp").is_some(),
        "作品の項目は今までどおり一番外側にある: {json}"
    );
    assert_eq!(
        value["step_creases"][0]["lines"].as_array().map(Vec::len),
        Some(0)
    );
    let back: SavedDocument = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(saved, back);
}

#[test]
fn old_files_without_step_creases_load_as_no_history() {
    // 旧形式 = Documentだけを書き出したファイル
    let json = serde_json::to_string(&sample_document()).expect("serialize");
    assert!(!json.contains("step_creases"));

    let saved: SavedDocument = serde_json::from_str(&json).expect("旧形式も読める");

    assert_eq!(saved.document, sample_document());
    assert!(saved.step_creases.is_empty());
    // 来歴が無ければ書き出す内容も旧形式と同じ
    assert_eq!(serde_json::to_string(&saved).expect("serialize"), json);
}

#[test]
fn old_face3d_without_mirrored_defaults_to_front() {
    let old = r#"{"face":7,"polygon":[[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,1.0,0.0]],"layer":2}"#;
    let face: Face3D = serde_json::from_str(old).expect("旧soft geometry frameを読み込む");
    assert_eq!(face.surface_rank, 0);
    assert!(!face.mirrored);
}

#[test]
fn test_alignment_modes_use_desktop_camel_case_and_roundtrip() {
    let cases = [
        (AlignmentMode::ThroughTwoPoints, "throughTwoPoints"),
        (AlignmentMode::PointPoint, "pointPoint"),
        (AlignmentMode::LineLine, "lineLine"),
        (
            AlignmentMode::PointPerpendicularLine,
            "pointPerpendicularLine",
        ),
        (AlignmentMode::PointLineThrough, "pointLineThrough"),
        (
            AlignmentMode::PointToLinePointToLine,
            "pointToLinePointToLine",
        ),
        (
            AlignmentMode::PointLinePerpendicular,
            "pointLinePerpendicular",
        ),
        (AlignmentMode::ExistingLine, "existingLine"),
    ];

    for (mode, name) in cases {
        let json = serde_json::to_string(&mode).expect("serialize AlignmentMode");
        assert_eq!(json, format!(r#""{name}""#));
        let back: AlignmentMode = serde_json::from_str(&json).expect("deserialize AlignmentMode");
        assert_eq!(back, mode);
    }
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

/// 表示補正の項目が無い古い作品ファイルも、それぞれの既定値で読めること。
#[test]
fn test_display_settings_defaults_for_old_files() {
    let old = r#"{"front_color":[1,2,3],"back_color":[4,5,6],"grid_divisions":8}"#;
    let d: ori3_model::DisplaySettings = serde_json::from_str(old).expect("deserialize");
    assert!(!d.soft_enabled, "たわみの既定はオフ");
    assert_eq!(d.soft_stiffness, 0.5);
    assert_eq!(d.soft_pressure, 0.0);
    // 形を変える補正は、古い作品でも利用者の明示選択なしに有効化しない。
    assert!(!d.overlap_prevention_enabled, "重なり防止の既定はオフ");
    assert!(d.penetration_prevention_enabled, "食い込み検出の既定はオン");
}

#[test]
fn test_edit_op_set_display_roundtrip() {
    // 紙の色・方眼の分割数の変更(PAP-003 / CPE-003)も内部タグ形式で往復できること。
    let op = EditOp::SetDisplay {
        display: ori3_model::DisplaySettings {
            front_color: [1, 2, 3],
            back_color: [4, 5, 6],
            grid_divisions: 12,
            soft_enabled: true,
            soft_stiffness: 0.25,
            soft_pressure: 0.75,
            overlap_prevention_enabled: false,
            penetration_prevention_enabled: false,
        },
    };
    let json = serde_json::to_string(&op).expect("serialize");
    assert!(json.contains("\"type\":\"SetDisplay\""), "json = {json}");
    let back: EditOp = serde_json::from_str(&json).expect("deserialize");
    match back {
        EditOp::SetDisplay { display } => {
            assert_eq!(display.front_color, [1, 2, 3]);
            assert_eq!(display.back_color, [4, 5, 6]);
            assert_eq!(display.grid_divisions, 12);
            // たわみの指定(SIM-015)もパラメータとして往復する
            assert!(display.soft_enabled);
            assert_eq!(display.soft_stiffness, 0.25);
            assert_eq!(display.soft_pressure, 0.75);
            assert!(!display.overlap_prevention_enabled);
            assert!(!display.penetration_prevention_enabled);
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
fn fold_through_variants_preserve_declarative_pose_before_exactly() {
    let pose_before = serde_json::json!({
        "drivers": [
            { "edge_id": 4, "target_angle_deg": 180.0 },
            { "edge_id": 9, "target_angle_deg": -180.0 },
            { "edge_id": 12, "target_angle_deg": 0.0 }
        ]
    });
    let cases = [
        serde_json::json!({
            "type": "FoldThrough",
            "up_to": 12,
            "line": [[0.25, 0.0], [0.25, 1.0]],
            "keep_side_point": [0.1, 0.5],
            "target_layers": [2, 3, 6, 7, 10, 11, 12, 13, 14, 15],
            "direction": "Down",
            "pose_before": pose_before.clone()
        }),
        serde_json::json!({
            "type": "PreviewFoldThrough",
            "up_to": 12,
            "line": [[0.25, 0.0], [0.25, 1.0]],
            "keep_side_point": [0.1, 0.5],
            "target_layers": [2, 3, 6, 7, 10, 11, 12, 13, 14, 15],
            "direction": "Down",
            "pose_before": pose_before.clone()
        }),
    ];

    for expected in cases {
        let op: SeqOp = serde_json::from_value(expected.clone())
            .expect("pose_before付きの折る要求を読み込める");
        let actual = serde_json::to_value(op).expect("折る要求を書き戻せる");

        assert_eq!(
            actual, expected,
            "+180度・-180度・明示0度を含む利用者指定を欠落・同一視せず保つ"
        );
        let drivers = actual["pose_before"]["drivers"]
            .as_array()
            .expect("pose_beforeには指定した3本が残る");
        assert_eq!(drivers.len(), 3, "明示0度を含む3本すべてを残す");
        assert_eq!(
            drivers[0]["target_angle_deg"]
                .as_f64()
                .expect("+180度は数値")
                .to_bits(),
            180.0_f64.to_bits()
        );
        assert_eq!(
            drivers[1]["target_angle_deg"]
                .as_f64()
                .expect("-180度は数値")
                .to_bits(),
            (-180.0_f64).to_bits()
        );
        assert_eq!(
            drivers[2]["target_angle_deg"]
                .as_f64()
                .expect("明示0度は数値")
                .to_bits(),
            0.0_f64.to_bits()
        );
    }
}

#[test]
fn old_fold_through_variants_without_pose_before_remain_readable() {
    let old_requests = [
        serde_json::json!({
            "type": "FoldThrough",
            "up_to": 2,
            "line": [[0.0, 0.5], [1.0, 0.5]],
            "keep_side_point": [0.5, 0.25],
            "target_layers": [3],
            "direction": "Down"
        }),
        serde_json::json!({
            "type": "PreviewFoldThrough",
            "up_to": 1,
            "line": [[0.7, 0.0], [0.7, 1.0]],
            "keep_side_point": [0.6, 0.5],
            "target_layers": [0],
            "direction": "Up"
        }),
    ];

    for old_request in old_requests {
        let op: SeqOp = serde_json::from_value(old_request.clone())
            .expect("pose_beforeが無い従来形式も読み込める");
        let restored = serde_json::to_value(op).expect("従来形式を書き戻せる");

        assert_eq!(restored, old_request, "従来の要求内容を変えない");
        assert!(
            restored.get("pose_before").is_none(),
            "指定が無い従来形式へ新しい欄を勝手に書き足さない"
        );
    }
}

/// 畳んだ状態への折り操作は画面(TypeScript)から送られてくるので、
/// JSONの形(内部タグ・snake_caseのフィールド名・向きの表記)を固定する。
#[test]
fn test_seq_op_fold_through_json_shape() {
    let op = SeqOp::FoldThrough {
        up_to: 2,
        line: [[0.0, 0.5], [1.0, 0.5]],
        keep_side_point: [0.5, 0.25],
        target_layers: Some(vec![3]),
        direction: FoldDirection::Down,
        alignment: None,
        accept_additional_crease: false,
        pose_before: None,
    };
    let json = serde_json::to_string(&op).expect("serialize");
    assert_eq!(
        json,
        r#"{"type":"FoldThrough","up_to":2,"line":[[0.0,0.5],[1.0,0.5]],"keep_side_point":[0.5,0.25],"target_layers":[3],"direction":"Down"}"#
    );
    let back: SeqOp = serde_json::from_str(&json).expect("deserialize");
    match back {
        SeqOp::FoldThrough {
            up_to,
            line,
            keep_side_point,
            target_layers,
            direction,
            alignment,
            accept_additional_crease,
            pose_before,
        } => {
            assert_eq!(up_to, 2);
            assert_eq!(line, [[0.0, 0.5], [1.0, 0.5]]);
            assert_eq!(keep_side_point, [0.5, 0.25]);
            assert_eq!(target_layers, Some(vec![3]));
            assert_eq!(direction, FoldDirection::Down);
            assert_eq!(alignment, None);
            assert!(!accept_additional_crease);
            assert_eq!(pose_before, None);
        }
        other => panic!("unexpected variant: {other:?}"),
    }
    // 対象層の指定なし(全層)はnullで往復する
    let json = serde_json::to_string(&SeqOp::FoldThrough {
        up_to: 0,
        line: [[0.0, 0.0], [1.0, 1.0]],
        keep_side_point: [1.0, 0.0],
        target_layers: None,
        direction: FoldDirection::Up,
        alignment: None,
        accept_additional_crease: false,
        pose_before: None,
    })
    .expect("serialize");
    assert!(json.contains(r#""target_layers":null"#), "json = {json}");
    assert!(json.contains(r#""direction":"Up""#), "json = {json}");

    let accepted = serde_json::to_string(&SeqOp::FoldThrough {
        up_to: 2,
        line: [[0.0, 0.5], [1.0, 0.5]],
        keep_side_point: [0.5, 0.25],
        target_layers: Some(vec![3]),
        direction: FoldDirection::Down,
        alignment: None,
        accept_additional_crease: true,
        pose_before: None,
    })
    .expect("serialize");
    assert!(
        accepted.contains(r#""accept_additional_crease":true"#),
        "json = {accepted}"
    );
}

/// 名前付き技法に閉じない層操作も、複数部分をまとめたままIPCで往復できる。
#[test]
fn test_seq_op_flat_motion_json_roundtrip() {
    let seam = [[0.5, 0.0], [0.5, 1.0]];
    let op = SeqOp::FlatMotion {
        up_to: 3,
        parts: vec![
            MotionPart {
                layers: vec![2],
                region: Vec::new(),
                transform: MotionTransform::Reflect(vec![seam]),
                turn: LayerTurn::Keep,
                reverse_layers: None,
            },
            MotionPart {
                layers: vec![0],
                region: Vec::new(),
                transform: MotionTransform::Stay,
                turn: LayerTurn::Outside(FoldDirection::Up),
                reverse_layers: None,
            },
            MotionPart {
                layers: vec![1, 3],
                region: vec![HalfPlane {
                    line: [[0.0, 0.25], [1.0, 0.25]],
                    inside_point: [0.5, 0.0],
                }],
                transform: MotionTransform::Stay,
                turn: LayerTurn::Inside(FoldDirection::Down),
                reverse_layers: Some(true),
            },
            MotionPart {
                layers: vec![4],
                region: Vec::new(),
                transform: MotionTransform::Stay,
                turn: LayerTurn::Beside {
                    anchor: 7,
                    direction: FoldDirection::Down,
                },
                reverse_layers: None,
            },
        ],
        kind: TechniqueKind::Pose,
    };

    let json = serde_json::to_string(&op).expect("serialize");
    assert!(json.contains(r#""type":"FlatMotion""#), "json = {json}");
    assert!(json.contains(r#""Reflect""#), "json = {json}");
    assert!(json.contains(r#""Outside""#), "json = {json}");
    assert!(json.contains(r#""Inside""#), "json = {json}");
    assert!(json.contains(r#""Beside""#), "json = {json}");
    assert!(json.contains(r#""reverse_layers":true"#), "json = {json}");

    let back: SeqOp = serde_json::from_str(&json).expect("deserialize");
    let SeqOp::FlatMotion { up_to, parts, kind } = back else {
        panic!("unexpected variant: {back:?}");
    };
    assert_eq!(up_to, 3);
    assert_eq!(kind, TechniqueKind::Pose);
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0].transform, MotionTransform::Reflect(vec![seam]));
    assert_eq!(parts[0].turn, LayerTurn::Keep);
    assert_eq!(parts[1].turn, LayerTurn::Outside(FoldDirection::Up));
    assert_eq!(parts[2].region[0].inside_point, [0.5, 0.0]);
    assert_eq!(parts[2].turn, LayerTurn::Inside(FoldDirection::Down));
    assert_eq!(parts[2].reverse_layers, Some(true));
    assert_eq!(
        parts[3].turn,
        LayerTurn::Beside {
            anchor: 7,
            direction: FoldDirection::Down,
        }
    );
}

#[test]
fn test_fold_step_without_alignment_remains_readable() {
    let old = r#"{"id":3,"kind":"Simple","drivers":[],"layer_order":null,"note":""}"#;
    let step: FoldStep = serde_json::from_str(old).expect("旧形式を読み込む");
    assert_eq!(step.alignment, None);
}

#[test]
fn test_seq_op_preview_fold_through_json_shape() {
    let op = SeqOp::PreviewFoldThrough {
        up_to: 1,
        line: [[0.7, 0.0], [0.7, 1.0]],
        keep_side_point: [0.6, 0.5],
        target_layers: Some(vec![0]),
        direction: FoldDirection::Up,
        pose_before: None,
    };
    let json = serde_json::to_string(&op).expect("serialize");
    assert_eq!(
        json,
        r#"{"type":"PreviewFoldThrough","up_to":1,"line":[[0.7,0.0],[0.7,1.0]],"keep_side_point":[0.6,0.5],"target_layers":[0],"direction":"Up"}"#
    );
    assert!(matches!(
        serde_json::from_str::<SeqOp>(&json).expect("deserialize"),
        SeqOp::PreviewFoldThrough {
            up_to: 1,
            direction: FoldDirection::Up,
            ..
        }
    ));
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
