//! 折り筋を先に引いた展開図で、折り途中の姿勢が正しく作られるかの検査。
//!
//! 通常実行される検査が1件、残りは数値を出すための測定台(`#[ignore]`)である。
//!
//! `scratchpad/bird-petal-squash-report.md` に載せた数値は、すべてこのファイルで測った。
//!
//! 測定台の実行のしかた(直列):
//! ```text
//! cargo test -p ori3-layers --test bird_pose_scan -- --ignored --test-threads=1 --nocapture
//! ```

use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{FlatState, FoldThroughResult, flat_state_at, petal, replay, squash};
use ori3_model::{CreasePattern, Document, EdgeKind, FaceId, Paper};
use ori3_rigid::{contact_metrics, max_seam_gap, self_intersection_pairs};

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<FoldThroughResult, String>;

fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

fn fold_layers(
    doc: &mut Document,
    line: [[f64; 2]; 2],
    keep: [f64; 2],
    target_layers: Option<Vec<FaceId>>,
    direction: FoldDirection,
) -> (FlatState, Vec<String>) {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line,
            keep_side_point: keep,
            target_layers,
            direction,
        },
    )
    .expect("折れる指定");
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
    (res.state, res.warnings)
}

fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2], direction: FoldDirection) {
    let (_, warnings) = fold_layers(doc, line, keep, None, direction);
    assert!(warnings.is_empty(), "警告なしで折れる: {warnings:?}");
}

fn apply(
    doc: &mut Document,
    technique: Technique,
    flap: Vec<FaceId>,
    line: [[f64; 2]; 2],
    reference_point: [f64; 2],
    open_to_back: Option<bool>,
) -> FlatState {
    let faces = extract_faces(&doc.cp);
    let up_to = doc.sequence.len();
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("平らな状態から折る");
    let mut cp = doc.cp.clone();
    let res = technique(
        &mut cp,
        &faces,
        &state,
        &TechniqueInput {
            flap,
            line,
            reference_point,
            open_to_back,
            polygon: None,
            center: None,
        },
    )
    .expect("折れる指定");
    assert!(
        res.warnings.is_empty(),
        "警告なしで折れる: {:?}",
        res.warnings
    );
    let mut step = res.step;
    step.id = u32::try_from(up_to).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
    res.state
}

fn state_of(doc: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&doc.cp);
    let (state, warnings) = flat_state_at(doc, &faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "再生の警告: {warnings:?}");
    (faces, state)
}

fn vertex_pos(cp: &CreasePattern) -> HashMap<u32, DVec2> {
    cp.vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect()
}

fn layers_tipped_at(doc: &Document, p: DVec2) -> Vec<FaceId> {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let f = faces.iter().find(|f| f.id == *id).expect("層順序の面");
            let pl = state.placements[&f.id];
            f.vertices
                .iter()
                .filter_map(|v| pos.get(v))
                .any(|&q| (pl.apply(q) - p).length() < 1e-6)
        })
        .collect()
}

fn preliminary_base() -> Document {
    let mut doc = square_doc();
    fold(
        &mut doc,
        [[0.0, 0.5], [1.0, 0.5]],
        [0.5, 0.25],
        FoldDirection::Up,
    );
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 0.5]],
        [0.25, 0.25],
        FoldDirection::Up,
    );
    for (line, reference) in [
        ([[0.5, 0.0], [0.5, 1.0]], [0.5, 0.1]),
        ([[0.0, 0.5], [1.0, 0.5]], [0.1, 0.5]),
    ] {
        let (_, state) = state_of(&doc);
        let bottom = vec![state.order[0]];
        apply(&mut doc, squash, bottom, line, reference, None);
    }
    doc
}

fn bird_base() -> Document {
    let mut doc = preliminary_base();
    let center_line = [[0.0, 1.0], [0.5, 0.5]];
    let tip = [0.0, 1.0];

    let (_, state) = state_of(&doc);
    let front = vec![*state.order.last().expect("最前面")];
    apply(&mut doc, petal, front, center_line, tip, None);

    let side_b = layers_tipped_at(&doc, DVec2::new(0.5, 1.0));
    let back: Vec<FaceId> = layers_tipped_at(&doc, DVec2::new(0.0, 0.5))
        .into_iter()
        .filter(|id| side_b.contains(id))
        .collect();
    assert_eq!(back.len(), 1, "背面はまだ1枚のまま(実際 {back:?})");
    apply(&mut doc, petal, back, center_line, tip, Some(true));
    doc
}

/// 折り筋つきの展開図でも、鳥の基本形の6手はどれも折り途中で紙をすり抜けない。
///
/// # 何を守っているか
///
/// 完成形の折り目を**先に全部引いてから**畳む展開図(提案機能が作るのはこれ)では、
/// まだ折っていない折り目も展開図の上に存在する。`extract_faces` はそこで面を分けるので、
/// 何もしないとその辺が「曲がるちょうつがい」として数えられ、
/// **利用者がまだ折っていない場所が折り途中だけ勝手に折れて**、紙が別の紙を突き抜けた。
///
/// 花弁折りではもう1つ、**この手が動かさない折り目(前の手順が決めた角)**を
/// ソルバーが逃げ道にして開いてしまう不具合があった。
/// どちらも「この手が動かさない折り目は折り途中でも動かさない」で直る
/// (`crates/ori3-layers/src/replay.rs` の `StepPath::held`)。
///
/// # 実測(2026-08-22、この検査と同じ21姿勢。紙の幅は1)
///
/// | 手 | 技法 | どちらも押さえない | まだ折っていない折り目だけ押さえる | **両方押さえる(いま)** |
/// |---:|---|---|---|---|
/// | 3 | つぶし折り | 自己交差 **7組**・最深 2.850453e-1 | 0組 | **0組** |
/// | 4 | つぶし折り | 自己交差 **9組**・最深 2.058969e-1 | 0組 | **0組** |
/// | 5 | 花弁折り | 自己交差 **6組**・最深 2.046746e-1 | **1組**・最深 7.745151e-2 | **0組** |
/// | 6 | 花弁折り | 自己交差 **6組**・最深 1.891851e-1 | **3組**・最深 2.351486e-2 | **0組** |
///
/// # 数値の決め方
///
/// - 自己交差の**組数**は個数なので、`0` と厳密に比べてよい(`CLAUDE.md` §10.7.9)。
/// - 裂けの量は計算した小数なので、既存の受け入れ上限 `1e-6`
///   (`crates/ori3-layers/src/pose_step.rs` と同じ値)で比べる。
///   ここだけ緩めても厳しくもしていない。実測の最大は手6の 2.448156e-13 で、
///   上限に**7桁**の余裕がある。
#[test]
fn every_step_of_the_bird_base_folds_without_passing_paper_through_itself() {
    const MAX_SEAM_GAP: f64 = 1e-6;
    let doc = bird_base();
    let faces = extract_faces(&doc.cp);
    assert_eq!(doc.sequence.len(), 6, "鳥の基本形は6手");
    assert_eq!(
        doc.sequence
            .iter()
            .map(|step| step.kind)
            .collect::<Vec<_>>(),
        vec![
            ori3_model::TechniqueKind::Simple,
            ori3_model::TechniqueKind::Simple,
            ori3_model::TechniqueKind::Squash,
            ori3_model::TechniqueKind::Squash,
            ori3_model::TechniqueKind::Petal,
            ori3_model::TechniqueKind::Petal,
        ],
        "半分に折る2回・つぶし折り2回・花弁折り2回"
    );
    // 手3・手4のつぶし折りの時点では、展開図に花弁折りの折り目が既にある。
    for up_to in [3usize, 4] {
        let unfolded = unspecified_crease_count(&doc, up_to);
        assert!(
            unfolded > 0,
            "手{up_to}の時点では、まだ折っていない折り目が展開図に残っている(実際 {unfolded}本)"
        );
    }
    for up_to in 1..=doc.sequence.len() {
        for k in 0..=20 {
            let t = f64::from(k) / 20.0;
            let replayed = replay(&doc, up_to, t);
            assert!(
                replayed.skipped.is_empty(),
                "手{up_to} t={t:.2}: 飛ばした手順がある {:?}",
                replayed.skipped
            );
            assert!(
                replayed.warnings.is_empty(),
                "手{up_to} t={t:.2}: 警告が出た {:?}",
                replayed.warnings
            );
            assert_eq!(
                replayed.frame.faces.len(),
                faces.len(),
                "手{up_to} t={t:.2}: 面が欠けた"
            );
            assert!(
                replayed.frame.faces.iter().all(|f| f
                    .polygon
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite())),
                "手{up_to} t={t:.2}: 座標が有限でない"
            );
            let pairs = self_intersection_pairs(&replayed.frame);
            assert!(
                pairs.is_empty(),
                "手{up_to} t={t:.2}: 紙がすり抜けた {}組(最深 {:.6e}、紙の幅は1) {:?}",
                pairs.len(),
                contact_metrics(&replayed.frame).max_penetration,
                &pairs[..pairs.len().min(8)]
            );
            let gap = max_seam_gap(&doc.cp, &faces, &replayed.frame);
            assert!(
                gap < MAX_SEAM_GAP,
                "手{up_to} t={t:.2}: 紙が裂けた(実際 {gap:.6e}、上限 {MAX_SEAM_GAP:.0e})"
            );
        }
    }
}

/// `up_to` 手までの手順が一度も指定していない折り目の本数。
fn unspecified_crease_count(doc: &Document, up_to: usize) -> usize {
    let mut specified: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for step in doc.sequence.iter().take(up_to) {
        for d in &step.drivers {
            for e in ori3_layers::fold_through::resolve_driver_edges(&doc.cp, d) {
                specified.insert(e);
            }
        }
    }
    doc.cp
        .edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .filter(|e| !specified.contains(&e.id))
        .count()
}

/// 各手を21姿勢で走査し、裂け・すり抜け・警告を出す。
#[test]
#[ignore = "診断専用"]
fn zz_scan_bird_base_reference_poses() {
    let doc = bird_base();
    let faces = extract_faces(&doc.cp);
    println!("手数={} 面数={}", doc.sequence.len(), faces.len());
    for (i, step) in doc.sequence.iter().enumerate() {
        println!(
            "--- 手{} kind={:?} drivers={:?}",
            i,
            step.kind,
            step.drivers
                .iter()
                .map(|d| (d.a, d.b, d.target_angle_deg))
                .collect::<Vec<_>>()
        );
    }
    for up_to in 1..=doc.sequence.len() {
        let step = &doc.sequence[up_to - 1];
        println!("=== up_to={up_to} kind={:?}", step.kind);
        for k in 0..=20 {
            let t = f64::from(k) / 20.0;
            let r = replay(&doc, up_to, t);
            let pairs = self_intersection_pairs(&r.frame);
            let metrics = contact_metrics(&r.frame);
            let gap = max_seam_gap(&doc.cp, &faces, &r.frame);
            let finite = r
                .frame
                .faces
                .iter()
                .all(|f| f.polygon.iter().flatten().all(|v| v.is_finite()));
            println!(
                "  t={t:.2} 面={} 交差組={} 最深={:.6e} 裂け={gap:.6e} 有限={finite} skipped={:?} 警告={}",
                r.frame.faces.len(),
                pairs.len(),
                metrics.max_penetration,
                r.skipped,
                r.warnings.len()
            );
            if !pairs.is_empty() && k > 0 {
                println!("      交差={:?}", &pairs[..pairs.len().min(8)]);
            }
            for w in &r.warnings {
                println!("      警告: {w}");
            }
        }
    }
}

/// つぶし折り1回だけを21姿勢で走査する(いちばん単純な退化した積み直し)。
#[test]
#[ignore = "診断専用"]
fn zz_scan_single_squash_poses() {
    let mut doc = square_doc();
    fold(
        &mut doc,
        [[0.0, 0.5], [1.0, 0.5]],
        [0.5, 0.25],
        FoldDirection::Up,
    );
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 0.5]],
        [0.25, 0.25],
        FoldDirection::Up,
    );
    let (_, state) = state_of(&doc);
    let bottom = vec![state.order[0]];
    let before_kinds: Vec<(u32, EdgeKind)> = doc.cp.edges.iter().map(|e| (e.id, e.kind)).collect();
    apply(
        &mut doc,
        squash,
        bottom,
        [[0.5, 0.0], [0.5, 1.0]],
        [0.5, 0.1],
        None,
    );
    let after_kinds: Vec<(u32, EdgeKind)> = doc.cp.edges.iter().map(|e| (e.id, e.kind)).collect();
    for ((id, a), (_, b)) in before_kinds.iter().zip(after_kinds.iter()) {
        if a != b {
            println!("山谷が変わった辺 {id}: {a:?} -> {b:?}");
        }
    }
    let step = doc.sequence.last().expect("手");
    println!(
        "つぶし折りの drivers = {:?}",
        step.drivers
            .iter()
            .map(|d| (d.a, d.b, d.target_angle_deg))
            .collect::<Vec<_>>()
    );
    let faces = extract_faces(&doc.cp);
    for k in 0..=20 {
        let t = f64::from(k) / 20.0;
        let r = replay(&doc, doc.sequence.len(), t);
        let pairs = self_intersection_pairs(&r.frame);
        let metrics = contact_metrics(&r.frame);
        let gap = max_seam_gap(&doc.cp, &faces, &r.frame);
        println!(
            "  t={t:.2} 交差組={} 最深={:.6e} 裂け={gap:.6e} 警告={:?}",
            pairs.len(),
            metrics.max_penetration,
            r.warnings
        );
        // 面ごとの高さ範囲
        let mut zs: Vec<(FaceId, f64, f64, u32)> = r
            .frame
            .faces
            .iter()
            .map(|f| {
                let lo = f.polygon.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min);
                let hi = f
                    .polygon
                    .iter()
                    .map(|p| p[2])
                    .fold(f64::NEG_INFINITY, f64::max);
                (f.face, lo, hi, f.surface_rank)
            })
            .collect();
        zs.sort_by_key(|e| e.0);
        println!("      面の高さ = {zs:?}");
        if !pairs.is_empty() {
            println!("      交差 = {pairs:?}");
        }
    }
}

/// 予備基本形だけ(展開図に花弁折りの折り目が無い)を21姿勢で走査する。
/// 鳥の基本形の走査との差は「展開図に後の手の折り目があるか」だけである。
#[test]
#[ignore = "診断専用"]
fn zz_scan_preliminary_base_poses() {
    let doc = preliminary_base();
    let faces = extract_faces(&doc.cp);
    println!("手数={} 面数={}", doc.sequence.len(), faces.len());
    for up_to in 1..=doc.sequence.len() {
        let step = &doc.sequence[up_to - 1];
        let mut worst = 0usize;
        let mut worst_depth = 0.0_f64;
        let mut bad_t = Vec::new();
        for k in 0..=20 {
            let t = f64::from(k) / 20.0;
            let r = replay(&doc, up_to, t);
            let pairs = self_intersection_pairs(&r.frame);
            let m = contact_metrics(&r.frame);
            if !pairs.is_empty() {
                bad_t.push((t, pairs.len(), m.max_penetration));
            }
            worst = worst.max(pairs.len());
            worst_depth = worst_depth.max(m.max_penetration);
        }
        println!(
            "up_to={up_to} kind={:?} 最大交差組={worst} 最深={worst_depth:.6e} 交差した姿勢={bad_t:?}",
            step.kind
        );
    }
}

/// 鳥の基本形の3手目(つぶし折り)の途中で、手順が指定していない折り目が
/// 何度になっているかを出す。
#[test]
#[ignore = "診断専用"]
fn zz_scan_bird_squash_hinge_angles() {
    let doc = bird_base();
    for (i, e) in doc.cp.edges.iter().enumerate() {
        let _ = i;
        println!("辺 {} v{}-v{} kind={:?}", e.id, e.v0, e.v1, e.kind);
    }
    for up_to in [3usize, 4] {
        for k in [0usize, 5, 10, 20] {
            let t = f64::from(u32::try_from(k).unwrap()) / 20.0;
            let r = replay(&doc, up_to, t);
            let mut angles: Vec<(u32, f64)> =
                r.hinge_angles.iter().map(|(e, a)| (*e, *a)).collect();
            angles.sort_by_key(|e| e.0);
            let nonzero: Vec<(u32, f64)> = angles
                .iter()
                .copied()
                .filter(|(_, a)| a.abs() > 1e-6)
                .collect();
            println!(
                "up_to={up_to} t={t:.2} 指定角={:?}",
                r.sequence_targets
                    .iter()
                    .map(|d| (d.hinge, d.target_angle_deg))
                    .collect::<Vec<_>>()
            );
            println!("  0でないヒンジ角={nonzero:?}");
        }
    }
}

/// 決め手の実験: まだ折っていない折り目を「0°の希望」として手順に入れると、
/// 途中の姿勢のすり抜けが消えるかを測る。
#[test]
#[ignore = "診断専用"]
fn zz_experiment_pin_unspecified_creases_to_zero() {
    let full = bird_base();
    let faces = extract_faces(&full.cp);

    // 手順1〜4(予備基本形)だけを残し、展開図は花弁折りの折り目を含む全体のまま。
    let mut doc = full.clone();
    doc.sequence.truncate(4);

    // 手順が指定していない折り目(=花弁折りで生まれる折り目)を数える。
    let mut specified: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for step in &doc.sequence {
        for d in &step.drivers {
            for e in ori3_layers::fold_through::resolve_driver_edges(&doc.cp, d) {
                specified.insert(e);
            }
        }
    }
    println!("手順が指定している折り目 = {:?}", specified);
    let pos: HashMap<u32, [f64; 2]> = doc.cp.vertices.iter().map(|v| (v.id, v.pos)).collect();
    let mut extra: Vec<ori3_model::DriverLine> = Vec::new();
    for e in &doc.cp.edges {
        if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) || specified.contains(&e.id) {
            continue;
        }
        let (Some(&a), Some(&b)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
            continue;
        };
        extra.push(ori3_model::DriverLine {
            a,
            b,
            target_angle_deg: 0.0,
        });
    }
    println!("手順が指定していない折り目 = {}本", extra.len());

    for pinned in [false, true] {
        let mut d = doc.clone();
        if pinned {
            d.sequence[0].drivers.extend(extra.iter().cloned());
        }
        for up_to in 1..=d.sequence.len() {
            let mut worst = 0usize;
            let mut depth = 0.0_f64;
            for k in 0..=20 {
                let t = f64::from(k) / 20.0;
                let r = replay(&d, up_to, t);
                worst = worst.max(self_intersection_pairs(&r.frame).len());
                depth = depth.max(contact_metrics(&r.frame).max_penetration);
            }
            let end = replay(&d, up_to, 1.0);
            let gap = max_seam_gap(&d.cp, &faces, &end.frame);
            println!(
                "0°で押さえる={pinned} up_to={up_to} kind={:?} 最大交差組={worst} 最深={depth:.6e} 終点の裂け={gap:.6e}",
                d.sequence[up_to - 1].kind
            );
        }
    }
}

/// 決め手2: 同じ文書のまま、まだ折っていない折り目だけを「折り筋(Aux)」に戻すと
/// (=面が分かれず、ヒンジが増えない)、途中の姿勢のすり抜けが消えるかを測る。
#[test]
#[ignore = "診断専用"]
fn zz_experiment_demote_unfolded_creases_to_aux() {
    let full = bird_base();
    let mut doc = full.clone();
    doc.sequence.truncate(4);

    let mut specified: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for step in &doc.sequence {
        for d in &step.drivers {
            for e in ori3_layers::fold_through::resolve_driver_edges(&doc.cp, d) {
                specified.insert(e);
            }
        }
    }
    for demote in [false, true] {
        let mut d = doc.clone();
        if demote {
            for e in d.cp.edges.iter_mut() {
                if matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley)
                    && !specified.contains(&e.id)
                {
                    e.kind = EdgeKind::Aux;
                }
            }
        }
        let faces = extract_faces(&d.cp);
        for up_to in 1..=d.sequence.len() {
            let mut worst = 0usize;
            let mut depth = 0.0_f64;
            for k in 0..=20 {
                let t = f64::from(k) / 20.0;
                let r = replay(&d, up_to, t);
                worst = worst.max(self_intersection_pairs(&r.frame).len());
                depth = depth.max(contact_metrics(&r.frame).max_penetration);
            }
            let end = replay(&d, up_to, 1.0);
            println!(
                "折り筋へ戻す={demote} 面数={} up_to={up_to} kind={:?} 最大交差組={worst} 最深={depth:.6e} 終点の裂け={:.6e}",
                faces.len(),
                d.sequence[up_to - 1].kind,
                max_seam_gap(&d.cp, &faces, &end.frame)
            );
        }
    }
}

/// 6手すべてについて、「その手までに手順が指定した折り目」以外を折り筋(Aux)へ戻し、
/// 21姿勢を測る。花弁折りも同じ扱いで直るかを見る。
#[test]
#[ignore = "診断専用"]
fn zz_experiment_demote_per_prefix_all_six_steps() {
    let full = bird_base();
    for up_to in 1..=full.sequence.len() {
        for demote in [false, true] {
            let mut d = full.clone();
            d.sequence.truncate(up_to);
            let mut specified: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
            for step in &d.sequence {
                for dr in &step.drivers {
                    for e in ori3_layers::fold_through::resolve_driver_edges(&d.cp, dr) {
                        specified.insert(e);
                    }
                }
            }
            if demote {
                for e in d.cp.edges.iter_mut() {
                    if matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley)
                        && !specified.contains(&e.id)
                    {
                        e.kind = EdgeKind::Aux;
                    }
                }
            }
            let faces = extract_faces(&d.cp);
            let mut worst = 0usize;
            let mut depth = 0.0_f64;
            let mut gap = 0.0_f64;
            let mut warned = 0usize;
            let mut skipped = 0usize;
            for k in 0..=20 {
                let t = f64::from(k) / 20.0;
                let r = replay(&d, up_to, t);
                worst = worst.max(self_intersection_pairs(&r.frame).len());
                depth = depth.max(contact_metrics(&r.frame).max_penetration);
                gap = gap.max(max_seam_gap(&d.cp, &faces, &r.frame));
                warned += r.warnings.len();
                skipped += r.skipped.len();
            }
            println!(
                "up_to={up_to} kind={:?} 折り筋へ戻す={demote} 指定済={} 面数={} 最大交差組={worst} 最深={depth:.6e} 最大裂け={gap:.6e} 警告={warned} skipped={skipped}",
                d.sequence[up_to - 1].kind,
                specified.len(),
                faces.len()
            );
        }
    }
}

/// 花弁折り(手5・手6)の残った自己交差が、どの面・どの時点・どの角度で起きているかを出す。
///
/// 「指定角」は `replay` が組み立てた折り途中の希望角(区間分けを反映した値)、
/// 「実角」はソルバーが返した角である。両者の差が大きい折り目は、
/// **その時点の希望角では紙がつながらない**ことを意味する。
#[test]
#[ignore = "診断専用"]
fn zz_scan_petal_pose_detail() {
    let doc = bird_base();
    let faces = extract_faces(&doc.cp);
    let pos: HashMap<u32, DVec2> = vertex_pos(&doc.cp);
    for (i, f) in faces.iter().enumerate() {
        let pts: Vec<String> = f
            .vertices
            .iter()
            .map(|v| {
                let p = pos[v];
                format!("({:.4},{:.4})", p.x, p.y)
            })
            .collect();
        println!("面{i} id={:?} 頂点={}", f.id, pts.join(" "));
    }
    for up_to in [5usize, 6] {
        println!("=== up_to={up_to}");
        let step = &doc.sequence[up_to - 1];
        for d in &step.drivers {
            println!(
                "  手の折り線 a={:?} b={:?} 目標={:.1}",
                d.a, d.b, d.target_angle_deg
            );
        }
        for k in 0..=20 {
            let t = f64::from(k) / 20.0;
            let r = replay(&doc, up_to, t);
            let want: HashMap<u32, f64> = r
                .sequence_targets
                .iter()
                .map(|d| (d.hinge, d.target_angle_deg))
                .collect();
            let mut rows: Vec<(u32, f64, f64)> = r
                .hinge_angles
                .iter()
                .map(|(&e, &got)| (e, want.get(&e).copied().unwrap_or(f64::NAN), got))
                .collect();
            rows.sort_by_key(|row| row.0);
            let moved: Vec<String> = rows
                .iter()
                .filter(|(_, want, got)| want.abs() > 0.5 || got.abs() > 0.5)
                .map(|(e, want, got)| format!("{e}:希望{want:.1}/実{got:.1}"))
                .collect();
            let pairs = self_intersection_pairs(&r.frame);
            println!(
                "  t={t:.2} 交差={:?} 最深={:.6e}",
                pairs,
                contact_metrics(&r.frame).max_penetration
            );
            println!("      {}", moved.join(" "));
        }
    }
}

/// 予備基本形(4面)の折り終わり直前の高さを出す(診断専用)。
#[test]
#[ignore = "診断専用"]
fn zz_scan_preliminary_end_heights() {
    let doc = preliminary_base();
    for t in [0.5f64, 0.9, 0.95, 0.99, 0.999, 1.0] {
        let r = replay(&doc, doc.sequence.len(), t);
        let zs: Vec<(u32, f64, f64)> = r
            .frame
            .faces
            .iter()
            .map(|f| {
                let zmin = f.polygon.iter().map(|v| v[2]).fold(f64::MAX, f64::min);
                let zmax = f.polygon.iter().map(|v| v[2]).fold(f64::MIN, f64::max);
                (f.face, zmin, zmax)
            })
            .collect();
        let all_min = zs.iter().map(|v| v.1).fold(f64::MAX, f64::min);
        let all_max = zs.iter().map(|v| v.2).fold(f64::MIN, f64::max);
        let mut angles: Vec<(u32, f64)> = r.hinge_angles.iter().map(|(e, a)| (*e, *a)).collect();
        angles.sort_by_key(|a| a.0);
        println!("t={t} 高さの幅={:.6e} 面ごと={:?}", all_max - all_min, zs);
        println!("    角={:?}", angles);
    }
}
