//! 提案した展開図を折ったときに記録される「紙の重なり順」の検査。
//!
//! # なぜこの検査が要るか
//!
//! 提案の折り方が通る手続き(`ori3_layers::collapse_precrease_network`)は、
//! 以前は重なり順をまったく組み替えていなかった。畳む前の順を分かれた面へ配り
//! 直すだけだったので、平らな1枚の紙から始めると**面の番号順**がそのまま答えに
//! なっていた。面の番号は面を取り出した順に振る導出値で、紙とも幾何とも関係が無い。
//!
//! 直す前の実測(2026-08-17。骨格=根に葉を n 本ぶら下げた星形、紙は正方形、
//! 充填の種 1、候補は充填の上位4件):
//!
//! | 出っぱり | 畳めた手 | 重なり順が面の番号順 | 折り返した紙が上下に散らばった手 |
//! |---|---:|---:|---:|
//! | 4本 | 5件 | 5件 | 4件 |
//! | 6本 | 14件 | 14件 | 12件 |
//! | 8本 | 15件 | 15件 | 12件 |
//! | 12本 | 11件 | 11件 | 5件 |
//! | **合計** | **45件** | **45件** | **33件** |
//!
//! 直した後は同じ45件で、面の番号順 **0件** / 散らばり **0件**
//! (読み取った重なりは前後とも481組で同じ)。
//!
//! 探索を通さず、**畳む手続きそのもの**を1手ずつ呼んで見る。探索が何手見つけるかに
//! 左右されず、出っぱり4本・12本でも標本が取れるためである。
//! 骨格・紙・充填の種はこの検査コードに直接書いてある(`CLAUDE.md` §10.1)。

use std::collections::BTreeSet;

use ori3_cp::extract_faces;
use ori3_layers::flat_state::{FlatState, layers_at_point, representative_point};
use ori3_layers::precrease_collapse::{PrecreaseCollapseInput, collapse_precrease_network};
use ori3_model::{Document, FaceId, Paper};
use ori3_propose::skeleton::{Skeleton, SkeletonNode};
use ori3_propose::{FoldSession, generate, pack};

/// 根に葉を `leaves` 本ぶら下げた星形の骨格。
fn star(leaves: u32) -> Skeleton {
    let mut nodes = vec![SkeletonNode::new(0, None, 0.0)];
    for id in 1..=leaves {
        nodes.push(SkeletonNode::new(id, Some(0), 1.0));
    }
    Skeleton { nodes }
}

/// 1手だけ畳んだ結果。
struct Folded {
    faces: usize,
    /// 記録された重なり順(下→上)。
    order: Vec<FaceId>,
    /// 折り返した紙が、折り返さなかった紙の**上**に来ている重なりの数。
    moved_above: usize,
    /// 折り返した紙が**下**に来ている重なりの数。
    moved_below: usize,
}

/// 提案の展開図を1手だけ畳んだ結果を集める。
///
/// 畳めなかった手・警告の出た手は数えない(そこは重なり順の話ではない)。
fn one_move_folds(leaves: u32) -> Vec<Folded> {
    let paper = Paper {
        width_mm: 150.0,
        height_mm: 150.0,
    };
    let skeleton = star(leaves);
    let mut out = Vec::new();
    for packing in pack(&skeleton, 1.0, 1.0, 1, 8) {
        let Ok(proposal) = generate(&skeleton, &packing, 1.0, 1.0) else {
            continue;
        };
        let mut document = Document::new(paper.clone());
        document.cp = proposal.cp;
        let Ok(session) = FoldSession::new(&document) else {
            continue;
        };
        let faces = extract_faces(&document.cp);
        let state = FlatState::initial(&document.cp, &faces);
        for fold_line in session.fold_lines() {
            let mut cp = document.cp.clone();
            let Ok(result) = collapse_precrease_network(
                &mut cp,
                &faces,
                &state,
                &PrecreaseCollapseInput {
                    lines: vec![[fold_line.a, fold_line.b]],
                    target_layers: None,
                },
            ) else {
                continue;
            };
            if !result.warnings.is_empty() {
                continue;
            }
            let folded_faces = extract_faces(&cp);
            let (mut moved_above, mut moved_below) = (0usize, 0usize);
            // 紙の各所で、重なっている紙を下から順に読む。折り返した紙(裏返って
            // いる紙)は、折り返さなかった紙に対して常に同じ側でなければならない。
            for face in &folded_faces {
                let point = representative_point(&cp, face);
                let here = result.state.placements[&face.id].apply(point.into());
                let stack = layers_at_point(&cp, &folded_faces, &result.state, [here.x, here.y]);
                for (position, lower) in stack.iter().enumerate() {
                    for upper in stack.iter().skip(position + 1) {
                        let lower_moved = result.state.placements[lower].mirrored;
                        let upper_moved = result.state.placements[upper].mirrored;
                        if lower_moved == upper_moved {
                            continue;
                        }
                        if upper_moved {
                            moved_above += 1;
                        } else {
                            moved_below += 1;
                        }
                    }
                }
            }
            out.push(Folded {
                faces: folded_faces.len(),
                order: result.state.order.clone(),
                moved_above,
                moved_below,
            });
        }
    }
    out
}

/// 提案の展開図を1手畳んだとき、記録される重なり順が面の番号順にならない。
///
/// 標本の数の下限は実測から取っている。修正時(2026-08-17)は
/// 4本=**5件** / 6本=**14件** / 8本=**15件** / 12本=**11件** の合計**45件**。
/// 後続成果を含む再測定(2026-08-21)では合計**56件**、番号順0件だった。
/// 下限はいちばん少ない4本の実測5件に対して余裕を見た **3件** にしてある
/// (§10.7.9。実測をそのまま境目にしない)。
#[test]
fn a_proposal_fold_never_records_the_face_index_order() {
    let mut samples = 0usize;
    let mut index_ordered = 0usize;
    for leaves in [4u32, 6, 8, 12] {
        let folds = one_move_folds(leaves);
        println!(
            "PROPOSAL_STACK_SAMPLES leaves={leaves} samples={}",
            folds.len()
        );
        assert!(
            folds.len() >= 3,
            "出っぱり{leaves}本で畳めた手が {} 件しか無く、重なり順を測れない",
            folds.len()
        );
        for fold in &folds {
            assert_eq!(
                fold.order.iter().copied().collect::<BTreeSet<_>>().len(),
                fold.faces,
                "出っぱり{leaves}本: 重なり順に面が抜けているか二重に入っている"
            );
            let mut sorted = fold.order.clone();
            sorted.sort_unstable();
            if fold.order == sorted {
                index_ordered += 1;
            }
            samples += 1;
        }
    }
    assert_eq!(
        index_ordered, 0,
        "紙の重なり順が面の番号順そのものになった手が {index_ordered} 件ある(標本 {samples} 件)"
    );
    assert!(samples >= 20, "標本が {samples} 件しか集まらなかった");
    println!("PROPOSAL_STACK_ORDER samples={samples} face_id_ordered={index_ordered}");
}

/// 1本の直線で折り返した紙は、折り返さなかった紙に対して**ひとまとまり**で
/// 上か下のどちらかに来る。
///
/// 上に来る場所と下に来る場所が混ざっていたら、紙が紙をすり抜けている。
/// 重なりの読み取りは `ori3_layers::layers_at_point`(画面の層選択と同じ道)で行う。
/// 実測(2026-08-17、直した後): 45件で読み取った重なりは **481組**、
/// 混ざっている手は **0件**。直す前は同じ481組で **33件**が混ざっていた。
/// 後続成果を含む再測定(2026-08-21)では **881組**、混ざり0件だった。
#[test]
fn a_proposal_fold_puts_the_turned_paper_all_on_one_side() {
    let mut mixed = 0usize;
    let mut overlaps = 0usize;
    for leaves in [4u32, 6, 8, 12] {
        for fold in one_move_folds(leaves) {
            overlaps += fold.moved_above + fold.moved_below;
            if fold.moved_above > 0 && fold.moved_below > 0 {
                mixed += 1;
            }
        }
    }
    assert!(
        overlaps >= 200,
        "重なりが {overlaps} 組しか読めておらず、検査になっていない"
    );
    assert_eq!(
        mixed, 0,
        "折り返した紙が上と下に散らばった手が {mixed} 件ある(読んだ重なり {overlaps} 組)"
    );
    println!("PROPOSAL_STACK_PENETRATION overlaps={overlaps} mixed={mixed}");
}

/// 同じ展開図を何度畳んでも、同じ重なり順になる。
#[test]
fn a_proposal_fold_records_the_same_stack_every_run() {
    let expected = one_move_folds(6)
        .iter()
        .map(|fold| fold.order.clone())
        .collect::<Vec<_>>();
    assert!(!expected.is_empty(), "標本が1件も取れない");
    for run in 1..5 {
        let again = one_move_folds(6)
            .iter()
            .map(|fold| fold.order.clone())
            .collect::<Vec<_>>();
        assert_eq!(again, expected, "{run}回目で重なり順が変わった");
    }
}
