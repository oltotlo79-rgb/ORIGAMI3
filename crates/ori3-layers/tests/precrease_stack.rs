//! 同時に畳む折り([`collapse_precrease_network`])が記録する紙の重なり順が、
//! 畳んだ形の幾何から決まっていることの検査。
//!
//! # なぜこの検査が要るか
//!
//! この手続きは以前、重なり順を**まったく組み替えていなかった**。畳む前の順を
//! 分かれた面へそのまま配り直すだけだったので、平らな1枚の紙から始めると
//! 「面の番号順」がそのまま答えになっていた。面の番号は面を取り出した順に振る
//! 導出値で、紙とも幾何とも関係が無い。
//!
//! 直す前の実測(2026-08-17、この検査と同じ入力):
//!
//! | 折った回数 | 普通の折り操作の重なり順 | 同時に畳む折りの重なり順 | 一致 |
//! |---|---|---|---|
//! | 1回 | `[(833,333), (167,667)]` | 同左 | はい |
//! | 2回 | `[(833,833), (333,833), (167,167), (667,167)]` | `[(167,167), (667,167), (833,833), (333,833)]` | **いいえ** |
//! | 3回 | `[(583,833), (417,667), (417,167), (583,333), (917,167), (83,333), (83,833), (917,667)]` | `[(417,167), (583,333), (417,667), (583,833), (917,167), (917,667), (83,833), (83,333)]` | **いいえ** |
//!
//! (数字は面の代表点の材質座標を1000倍して丸めたもの。下→上の並び)
//!
//! 展開図と折る直線はこの検査コードに直接書いてある(`CLAUDE.md` §10.1)。

use std::collections::HashMap;

use ori3_cp::extract_faces;
use ori3_layers::flat_state::{FlatState, representative_point};
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::precrease_collapse::{PrecreaseCollapseInput, collapse_precrease_network};
use ori3_model::{CreasePattern, Document, EdgeKind, Face3D, Frame3D, Paper};

/// 折る直線と「動かさない側」の点。1辺1の正方形の材質座標。
type Fold = ([[f64; 2]; 2], [f64; 2]);

/// 半分に折る → さらに半分に折る → 4分の1で折る、の3手。
const FOLDS: [Fold; 3] = [
    ([[0.5, 0.0], [0.5, 1.0]], [0.1, 0.5]),
    ([[0.0, 0.5], [1.0, 0.5]], [0.25, 0.1]),
    ([[0.25, 0.0], [0.25, 1.0]], [0.4, 0.25]),
];

fn square() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

/// 重なり順を「面の代表点の材質座標」で表す(下→上)。
///
/// 面の番号は展開図の作り方で変わるので、2つの道の答えを面の番号で比べられない。
/// 紙のどこがどの高さに来たかで比べる。座標は計算で出た小数なので、
/// 1000分の1へ丸めてから比べる(§10.7.7)。この紙でいちばん近い代表点どうしは
/// 実測で 1/12 (=0.083) 離れており、丸めで別の面と混ざらない。
fn material_stack(cp: &CreasePattern, state: &FlatState) -> Vec<(i64, i64)> {
    let faces = extract_faces(cp);
    state
        .order
        .iter()
        .map(|id| {
            let face = faces.iter().find(|face| face.id == *id).expect("面がある");
            let point = representative_point(cp, face);
            (
                (point[0] * 1_000.0).round() as i64,
                (point[1] * 1_000.0).round() as i64,
            )
        })
        .collect()
}

/// 普通の折り操作を `count` 回まで行う。
fn fold_ordinary(count: usize) -> (Document, FlatState) {
    let mut document = square();
    let faces = extract_faces(&document.cp);
    let mut state = FlatState::initial(&document.cp, &faces);
    for (line, keep_side_point) in FOLDS.iter().take(count) {
        let faces = extract_faces(&document.cp);
        state = fold_through(
            &mut document.cp,
            &faces,
            &state,
            &FoldThroughInput {
                line: *line,
                keep_side_point: *keep_side_point,
                direction: FoldDirection::Up,
                target_layers: None,
            },
        )
        .expect("普通の折り操作")
        .state;
    }
    (document, state)
}

/// 展開図にある折り目を、同じ直線に乗るものごとにまとめた一覧。
fn crease_lines(cp: &CreasePattern) -> Vec<[[f64; 2]; 2]> {
    let position = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.pos))
        .collect::<HashMap<_, _>>();
    let mut lines: Vec<[[f64; 2]; 2]> = Vec::new();
    for edge in &cp.edges {
        if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let (Some(&a), Some(&b)) = (position.get(&edge.v0), position.get(&edge.v1)) else {
            continue;
        };
        let on_known_line = lines.iter().any(|line| {
            let direction = [line[1][0] - line[0][0], line[1][1] - line[0][1]];
            let length = direction[0].hypot(direction[1]);
            let distance = |point: [f64; 2]| {
                (direction[0] * (point[1] - line[0][1]) - direction[1] * (point[0] - line[0][0]))
                    .abs()
                    / length
            };
            distance(a) <= 1e-9 && distance(b) <= 1e-9
        });
        if !on_known_line {
            lines.push([a, b]);
        }
    }
    lines
}

/// 平らに畳んだ状態を、重なり順を高さに写した立体として組み立てる
/// (`ori3_rigid::layer_order_conflicts` に渡すため)。
fn flat_frame(cp: &CreasePattern, state: &FlatState) -> Frame3D {
    let faces = extract_faces(cp);
    let position = cp
        .vertices
        .iter()
        .map(|vertex| (vertex.id, glam::DVec2::from(vertex.pos)))
        .collect::<HashMap<_, _>>();
    let rank = state
        .order
        .iter()
        .enumerate()
        .map(|(rank, face)| (*face, rank as u32))
        .collect::<HashMap<_, _>>();
    Frame3D {
        faces: faces
            .iter()
            .map(|face| {
                let placement = state.placements[&face.id];
                Face3D {
                    face: face.id,
                    polygon: face
                        .vertices
                        .iter()
                        .filter_map(|vertex| position.get(vertex))
                        .map(|point| {
                            let folded = placement.apply(*point);
                            [folded.x, folded.y, 0.0]
                        })
                        .collect(),
                    layer: rank[&face.id],
                    surface_rank: rank[&face.id],
                    mirrored: placement.mirrored,
                }
            })
            .collect(),
        warnings: Vec::new(),
    }
}

/// 同じ展開図(山谷まで同じ)を、普通の折り操作の道と同時に畳む道の両方で通すと、
/// 紙の重なり順が一致する。
#[test]
fn collapsing_a_finished_crease_pattern_stacks_the_paper_like_the_ordinary_fold() {
    for count in 1..=FOLDS.len() {
        let (ordinary, ordinary_state) = fold_ordinary(count);
        let expected = material_stack(&ordinary.cp, &ordinary_state);

        // 折り上がった展開図(山谷が決まっている)を、まっさらな平らの状態から
        // 一度に畳み直す。
        let mut cp = ordinary.cp.clone();
        let lines = crease_lines(&cp);
        let faces = extract_faces(&cp);
        let state = FlatState::initial(&cp, &faces);
        let result = collapse_precrease_network(
            &mut cp,
            &faces,
            &state,
            &PrecreaseCollapseInput {
                lines,
                target_layers: None,
            },
        )
        .unwrap_or_else(|error| panic!("{count}手ぶんの畳みに失敗した: {error}"));
        assert!(
            result.warnings.is_empty(),
            "{count}手: 警告が出た {:?}",
            result.warnings
        );
        assert_eq!(
            material_stack(&cp, &result.state),
            expected,
            "{count}手: 2つの道で紙の重なり順が食い違った"
        );
    }
}

/// 畳んだ結果の重なり順は、展開図の山谷すべてと食い違わない。
///
/// 折り目でつながる2面のどちらが上かは、山谷と面の裏返りで一意に決まる。
/// その照合は `ori3_rigid::layer_order_conflicts` が1か所で持っている。
#[test]
fn the_collapsed_stack_agrees_with_every_mountain_and_valley() {
    for count in 1..=FOLDS.len() {
        let (ordinary, _) = fold_ordinary(count);
        let mut cp = ordinary.cp.clone();
        let lines = crease_lines(&cp);
        let faces = extract_faces(&cp);
        let state = FlatState::initial(&cp, &faces);
        let result = collapse_precrease_network(
            &mut cp,
            &faces,
            &state,
            &PrecreaseCollapseInput {
                lines,
                target_layers: None,
            },
        )
        .expect("畳み");
        let faces = extract_faces(&cp);
        let frame = flat_frame(&cp, &result.state);
        assert!(
            !ori3_rigid::layer_order_conflicts(&cp, &faces, &frame),
            "{count}手: 重なり順が展開図の山谷と食い違っている"
        );
    }
}

/// 何度実行しても同じ重なり順になる。
#[test]
fn the_collapsed_stack_is_the_same_every_run() {
    let (ordinary, _) = fold_ordinary(FOLDS.len());
    let lines = crease_lines(&ordinary.cp);
    let mut first: Option<Vec<(i64, i64)>> = None;
    for run in 0..10 {
        let mut cp = ordinary.cp.clone();
        let faces = extract_faces(&cp);
        let state = FlatState::initial(&cp, &faces);
        let result = collapse_precrease_network(
            &mut cp,
            &faces,
            &state,
            &PrecreaseCollapseInput {
                lines: lines.clone(),
                target_layers: None,
            },
        )
        .expect("畳み");
        let stack = material_stack(&cp, &result.state);
        match &first {
            None => first = Some(stack),
            Some(expected) => assert_eq!(&stack, expected, "{run}回目で重なり順が変わった"),
        }
    }
}
