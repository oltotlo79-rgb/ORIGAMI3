//! 一斉折りの第4標本（カエル）だけを、時間を測らずに50出力確認する。
//!
//! フロント用fixtureは`acceptance_frog.rs`が正本からread-only照合するものを使う。
//! この検査は解の座標や収束結果を期待値にせず、割合・符号・有限性・面完全性だけを確認する。

use std::collections::{BTreeMap, BTreeSet};

use ori3_cp::extract_faces;
use ori3_model::{CreasePattern, Edge, EdgeKind, Vertex};
use ori3_rigid::{fold_all_targets, solve_fold_all_preview};
use serde::Deserialize;

#[derive(Deserialize)]
struct FrogFixture {
    vertices: Vec<Vertex>,
    edges: Vec<Edge>,
}

fn frog_crease_pattern() -> CreasePattern {
    let fixture: FrogFixture = serde_json::from_str(include_str!("../../src/lib/__fixtures__/frog.json"))
        .expect("正本とread-only照合済みのカエルfixtureを読める");
    let next_vertex_id = fixture
        .vertices
        .iter()
        .map(|vertex| vertex.id)
        .max()
        .expect("カエルに頂点がある")
        + 1;
    let next_edge_id = fixture
        .edges
        .iter()
        .map(|edge| edge.id)
        .max()
        .expect("カエルに辺がある")
        + 1;
    CreasePattern {
        vertices: fixture.vertices,
        edges: fixture.edges,
        next_vertex_id,
        next_edge_id,
    }
}

#[test]
fn frog_fourth_sample_has_fifty_finite_complete_signed_outputs_without_timing() {
    let cp = frog_crease_pattern();
    let faces = extract_faces(&cp);
    let expected_faces: BTreeSet<_> = faces.iter().map(|face| face.id).collect();
    let kinds: BTreeMap<_, _> = cp
        .edges
        .iter()
        .map(|edge| (edge.id, edge.kind))
        .collect();
    let mut output_count = 0;

    for percent in [0.0, 25.0, 50.0, 75.0, 100.0] {
        for repetition in 1..=10 {
            let targets = fold_all_targets(&cp, &faces, percent)
                .unwrap_or_else(|error| panic!("カエル {percent}% {repetition}回目の目標: {error}"));
            assert_eq!(targets.len(), 248, "カエルの有効な山谷ヒンジ数");
            for target in &targets {
                let expected = match kinds[&target.hinge] {
                    EdgeKind::Mountain => 180.0 * percent / 100.0,
                    EdgeKind::Valley => -180.0 * percent / 100.0,
                    EdgeKind::Border | EdgeKind::Aux => {
                        panic!("カエル {percent}%が山谷以外の辺{}を返した", target.hinge)
                    }
                };
                assert_eq!(target.target_angle_deg, expected);
            }

            let preview = solve_fold_all_preview(&cp, &faces, percent, None).unwrap_or_else(|error| {
                panic!("カエル {percent}% {repetition}回目の一斉折り: {error}")
            });
            assert_eq!(preview.requested_percent, percent);
            assert_eq!(preview.requested_angles, targets);
            let actual_faces: BTreeSet<_> = preview
                .motion
                .result
                .frame
                .faces
                .iter()
                .map(|face| face.face)
                .collect();
            assert_eq!(actual_faces, expected_faces, "カエル {percent}%の返却面欠落");
            assert!(preview.motion.result.closure_rms.is_finite());
            assert!(preview.motion.result.frame.faces.iter().all(|face| {
                face.polygon
                    .iter()
                    .flatten()
                    .all(|coordinate| coordinate.is_finite())
            }));
            output_count += 1;
        }
    }

    assert_eq!(output_count, 50);
}
