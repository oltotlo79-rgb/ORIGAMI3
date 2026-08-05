//! replay(手順の再生)のテスト: 決定性・展開図編集への耐性・スキップ継続。

use std::time::{Duration, Instant};

use ori3_cp::extract_faces;
use ori3_layers::flat_state::FlatState;
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::replay::{ReplayResult, replay};
use ori3_layers::resolve_driver_edges;
use ori3_model::{
    CreasePattern, Document, DriverLine, Edge, EdgeId, EdgeKind, FoldStep, Frame3D, Paper,
    TechniqueKind, Vertex,
};

/// 正方形を3回折る手順(x=0.5 → y=0.5 → 畳んだ上でx=0.25)のDocument。
/// 3ステップとも `fold_through` が生成した実物のFoldStep(DriverLine+layer_order)。
fn three_step_document() -> Document {
    let mut doc = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let faces = extract_faces(&doc.cp);
    let mut state = FlatState::initial(&doc.cp, &faces);
    let folds: [([[f64; 2]; 2], [f64; 2]); 3] = [
        ([[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]),
        ([[0.0, 0.5], [0.5, 0.5]], [0.25, 0.25]),
        ([[0.25, 0.0], [0.25, 0.5]], [0.1, 0.25]),
    ];
    for (i, (line, keep_side_point)) in folds.into_iter().enumerate() {
        let faces = extract_faces(&doc.cp);
        let res = fold_through(
            &mut doc.cp,
            &faces,
            &state,
            &FoldThroughInput {
                line,
                keep_side_point,
                target_layers: None,
                direction: FoldDirection::Up,
            },
        )
        .expect("3回とも折れる");
        state = res.state;
        let mut step = res.step;
        step.id = u32::try_from(i).unwrap();
        doc.sequence.push(step);
    }
    assert_eq!(doc.sequence.len(), 3);
    doc
}

/// Frame3Dをビット列へ落とす(浮動小数の完全一致を確かめるため)。
fn frame_bits(frame: &Frame3D) -> Vec<u64> {
    let mut out = Vec::new();
    for f in &frame.faces {
        out.push(u64::from(f.face));
        out.push(u64::from(f.layer));
        for p in &f.polygon {
            out.extend(p.iter().map(|v| v.to_bits()));
        }
        out.push(u64::MAX); // 面の区切り(長さの違いも検出する)
    }
    out
}

fn has_step_warning(res: &ReplayResult, needle: &str) -> bool {
    res.warnings.iter().any(|w| w.contains(needle))
}

/// 立体の指定軸方向の広がり(0=x, 1=y, 2=z)。solveが固定する根の面の位置に
/// 依存しないよう、位置ではなく広がりで畳み上がりを確かめるために使う。
fn extent(frame: &Frame3D, axis: usize) -> f64 {
    let vs: Vec<f64> = frame
        .faces
        .iter()
        .flat_map(|f| f.polygon.iter().map(|p| p[axis]))
        .collect();
    vs.iter().copied().fold(f64::MIN, f64::max) - vs.iter().copied().fold(f64::MAX, f64::min)
}

#[test]
fn replay_twice_is_bit_identical() {
    let doc = three_step_document();
    for (up_to, t) in [(3usize, 1.0f64), (3, 0.5), (2, 0.25), (0, 1.0)] {
        let a = replay(&doc, up_to, t);
        let b = replay(&doc, up_to, t);
        assert_eq!(
            frame_bits(&a.frame),
            frame_bits(&b.frame),
            "up_to={up_to} t={t} の再生結果がビット一致しない"
        );
        assert_eq!(a.skipped, b.skipped);
        assert_eq!(a.warnings, b.warnings);
    }
}

#[test]
fn full_replay_folds_flat_and_layers_are_a_permutation() {
    let doc = three_step_document();
    let res = replay(&doc, 3, 1.0);
    assert!(res.skipped.is_empty(), "警告={:?}", res.warnings);
    assert!(
        !has_step_warning(&res, "収束しませんでした"),
        "警告={:?}",
        res.warnings
    );

    // 8層に畳まれる: 層番号は0..8の並べ替えでちょうど1回ずつ
    assert_eq!(res.frame.faces.len(), 8);
    let mut layers: Vec<u32> = res.frame.faces.iter().map(|f| f.layer).collect();
    layers.sort_unstable();
    assert_eq!(layers, (0..8).collect::<Vec<u32>>());

    // 完全に畳まれている: 幅は1→1/2(手順1)→1/4(手順3)、高さは1→1/2(手順2)
    assert!(
        (extent(&res.frame, 0) - 0.25).abs() < 1e-6,
        "横幅={}",
        extent(&res.frame, 0)
    );
    assert!(
        (extent(&res.frame, 1) - 0.5).abs() < 1e-6,
        "縦幅={}",
        extent(&res.frame, 1)
    );
    assert!(
        extent(&res.frame, 2) < 1e-6,
        "厚み={}",
        extent(&res.frame, 2)
    );
}

#[test]
fn up_to_zero_is_flat_and_layers_follow_face_id_order() {
    let doc = three_step_document();
    let res = replay(&doc, 0, 1.0);
    assert!(res.skipped.is_empty());
    assert!(res.warnings.is_empty(), "警告={:?}", res.warnings);
    // 平ら: z=0、かつ元の展開図の座標のまま
    assert!(
        res.frame
            .faces
            .iter()
            .all(|f| f.polygon.iter().all(|p| p[2].abs() < 1e-12))
    );
    // 初期の層順序は面ID昇順
    let mut by_layer: Vec<(u32, u32)> = res.frame.faces.iter().map(|f| (f.layer, f.face)).collect();
    by_layer.sort_unstable();
    assert!(by_layer.windows(2).all(|w| w[0].1 < w[1].1), "{by_layer:?}");
}

#[test]
fn up_to_and_t_are_clamped() {
    let doc = three_step_document();
    let a = replay(&doc, 3, 1.0);
    let b = replay(&doc, 99, 5.0);
    assert_eq!(
        frame_bits(&a.frame),
        frame_bits(&b.frame),
        "範囲外は丸められる"
    );

    let c = replay(&doc, 1, 0.0);
    let d = replay(&doc, 0, 1.0);
    assert_eq!(
        frame_bits(&c.frame),
        frame_bits(&d.frame),
        "t=0は直前の状態と同じ"
    );
}

#[test]
fn unrelated_aux_line_keeps_every_step_working() {
    let mut doc = three_step_document();
    let before = replay(&doc, 3, 1.0);

    // 手順と無関係な補助線(左下の面を横切る)を展開図に足す。補助線は面の境界には
    // ならないが、交わる折り線(x=0.25)を2本に分割するので、手順が辺IDを参照して
    // いたらここで再生が壊れる。
    let before_fragments = resolve_driver_edges(&doc.cp, &doc.sequence[2].drivers[0]).len();
    ori3_cp::insert_segment(&mut doc.cp, [0.0, 0.25], [0.25, 0.25], EdgeKind::Aux);
    assert!(
        resolve_driver_edges(&doc.cp, &doc.sequence[2].drivers[0]).len() > before_fragments,
        "補助線が折り線を分割している前提"
    );

    let after = replay(&doc, 3, 1.0);
    assert!(after.skipped.is_empty(), "警告={:?}", after.warnings);
    assert!(
        !after.warnings.iter().any(|w| w.contains("手順")),
        "手順に関する警告は出ない: {:?}",
        after.warnings
    );
    // 補助線は面の境界にならないので面の数と層順序は変わらない
    // (面の輪郭には交点が増えるので頂点数だけは増える)
    assert_eq!(after.frame.faces.len(), before.frame.faces.len());
    let layers = |r: &ReplayResult| -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = r.frame.faces.iter().map(|f| (f.face, f.layer)).collect();
        v.sort_unstable();
        v
    };
    assert_eq!(layers(&after), layers(&before));
}

#[test]
fn step_with_missing_fold_lines_is_skipped_and_later_steps_continue() {
    let mut doc = three_step_document();
    // 手順2(y=0.5)が参照する折り線を展開図から全て削除する
    let victims: Vec<EdgeId> = doc.sequence[1]
        .drivers
        .iter()
        .flat_map(|d| resolve_driver_edges(&doc.cp, d))
        .collect();
    assert!(!victims.is_empty());
    ori3_cp::remove_edges(&mut doc.cp, &victims);

    let res = replay(&doc, 3, 1.0);
    assert_eq!(
        res.skipped,
        vec![doc.sequence[1].id],
        "手順2だけが飛ばされる"
    );
    assert!(
        has_step_warning(
            &res,
            "手順2の折り線が見つからないため、この手順を飛ばしました"
        ),
        "警告={:?}",
        res.warnings
    );
    // 手順1・3は続行する: 幅は1/4まで畳まれ(手順1と3)、高さは1のまま(手順2は不成立)
    assert!(
        (extent(&res.frame, 0) - 0.25).abs() < 1e-6,
        "横幅={}",
        extent(&res.frame, 0)
    );
    assert!(
        (extent(&res.frame, 1) - 1.0).abs() < 1e-6,
        "縦幅={}",
        extent(&res.frame, 1)
    );
    // 飛ばした手順の層順序は使わず、直前の層順序を保つ(層は全面の並べ替えのまま)
    let mut layers: Vec<u32> = res.frame.faces.iter().map(|f| f.layer).collect();
    layers.sort_unstable();
    assert_eq!(
        layers,
        (0..u32::try_from(res.frame.faces.len()).unwrap()).collect::<Vec<u32>>()
    );
}

#[test]
fn step_with_partially_missing_fold_lines_continues_with_warning() {
    let mut doc = three_step_document();
    // 手順3のDriverLineは畳んだ層ごとに1本ずつ(x=0.25/0.75の上下)。
    // そのうち1本ぶんの辺だけを削除すると「一部が見つかりません」になる。
    assert!(doc.sequence[2].drivers.len() >= 2);
    let victims = resolve_driver_edges(&doc.cp, &doc.sequence[2].drivers[0]);
    assert!(!victims.is_empty());
    ori3_cp::remove_edges(&mut doc.cp, &victims);

    let res = replay(&doc, 3, 1.0);
    assert!(res.skipped.is_empty(), "残りの折り線で続行する");
    assert!(
        has_step_warning(&res, "手順3の折り線の一部が見つかりません"),
        "警告={:?}",
        res.warnings
    );
}

#[test]
fn steps_without_drivers_are_not_skipped() {
    let mut doc = three_step_document();
    doc.sequence.push(FoldStep {
        id: 99,
        kind: TechniqueKind::Pose,
        drivers: Vec::new(),
        layer_order: None,
        note: String::new(),
    });
    let res = replay(&doc, 4, 1.0);
    assert!(
        res.skipped.is_empty(),
        "折り線を持たない手順は飛ばし扱いにしない"
    );
    assert!(
        !res.warnings.iter().any(|w| w.contains("手順4")),
        "警告={:?}",
        res.warnings
    );
}

/// NFR-002: 10ステップ・面400の全再生が3秒以内。
///
/// 20×20のミウラ折り(面400・辺840)に、中央の山ヒンジを2°ずつ深くする10ステップの
/// 手順を与え、`replay(doc, 10, 1.0)` の実時間を測る(10回のsolveをwarm startで
/// 連鎖する、いちばん重い使い方)。
/// 実測(2026-08-05, 開発機 Windows 11): debug 約1.0〜1.1秒 / release 約36ms
/// (release実測は `cargo test -p ori3-layers --release --test replay -- --nocapture`)。
/// debugビルドにそのまま3秒の上限を課す(release目標に対し十分厳しい)。
#[test]
fn replay_of_ten_steps_on_400_faces_is_under_three_seconds() {
    let cp = miura_cp(20, 20);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 400);
    // 中央付近の縦ヒンジ(山)を線分で指定する
    let hinge = 20 / 2 * (20 + 1) + 20 / 2;
    let e = &cp.edges[hinge];
    assert_eq!(e.kind, EdgeKind::Mountain);
    let pos = |id: u32| cp.vertices.iter().find(|v| v.id == id).unwrap().pos;
    let (a, b) = (pos(e.v0), pos(e.v1));

    let mut doc = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    doc.cp = cp;
    doc.sequence = (1..=10u32)
        .map(|i| FoldStep {
            id: i,
            kind: TechniqueKind::Simple,
            drivers: vec![DriverLine {
                a,
                b,
                target_angle_deg: 2.0 * f64::from(i),
            }],
            layer_order: None,
            note: String::new(),
        })
        .collect();

    let t0 = Instant::now();
    let res = replay(&doc, 10, 1.0);
    let dt = t0.elapsed();
    println!("replay(10ステップ・面400) = {dt:?} 警告={:?}", res.warnings);
    assert!(res.skipped.is_empty());
    assert!(
        dt < Duration::from_secs(3),
        "全再生が遅すぎます: {dt:?}(NFR-002: 3秒以内)"
    );
}

/// perf用: nc×nr面のミウラ折りCP(ori3-rigid の tests/perf_miura.rs と同じ構成)。
fn miura_cp(nc: usize, nr: usize) -> CreasePattern {
    let s = 0.35;
    let dx = 1.0 / nc as f64;
    let dy = 1.0 / (nr as f64 + s);
    let vid = |i: usize, j: usize| u32::try_from(j * (nc + 1) + i).unwrap();
    let mut vertices = Vec::new();
    for j in 0..=nr {
        for i in 0..=nc {
            vertices.push(Vertex {
                id: vid(i, j),
                pos: [
                    i as f64 * dx,
                    (j as f64 + if i % 2 == 1 { s } else { 0.0 }) * dy,
                ],
            });
        }
    }
    let mut edges: Vec<Edge> = Vec::new();
    for j in 0..nr {
        for i in 0..=nc {
            let kind = if i == 0 || i == nc {
                EdgeKind::Border
            } else if (i + j) % 2 == 0 {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            edges.push(Edge {
                id: u32::try_from(edges.len()).unwrap(),
                v0: vid(i, j),
                v1: vid(i, j + 1),
                kind,
            });
        }
    }
    for j in 0..=nr {
        for i in 0..nc {
            let kind = if j == 0 || j == nr {
                EdgeKind::Border
            } else if j % 2 == 1 {
                EdgeKind::Mountain
            } else {
                EdgeKind::Valley
            };
            edges.push(Edge {
                id: u32::try_from(edges.len()).unwrap(),
                v0: vid(i, j),
                v1: vid(i + 1, j),
                kind,
            });
        }
    }
    CreasePattern {
        next_vertex_id: u32::try_from(vertices.len()).unwrap(),
        next_edge_id: u32::try_from(edges.len()).unwrap(),
        vertices,
        edges,
    }
}
