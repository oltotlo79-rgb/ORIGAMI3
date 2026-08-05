//! replay(手順の再生)のテスト: 決定性・展開図編集への耐性・スキップ継続。

use std::time::{Duration, Instant};

use ori3_cp::extract_faces;
use ori3_geometry::Isometry2;
use ori3_layers::flat_state::FlatState;
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::replay::{ReplayResult, flat_state_at, replay};
use ori3_layers::resolve_driver_edges;
use ori3_model::{
    CreasePattern, Document, DriverLine, Edge, EdgeId, EdgeKind, FoldStep, Frame3D, Paper,
    TechniqueKind, Vertex,
};

/// 正方形を半分に折り続ける手順(x=0.5 → y=0.5 → x=0.25 → y=0.25)。
/// 各ステップは `fold_through` が生成した実物のFoldStep(DriverLine+layer_order)。
/// 手順kまで折った紙の外形は `FOLDED_SIZE[k]`。
const FOLDS: [([[f64; 2]; 2], [f64; 2]); 4] = [
    ([[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5]),
    ([[0.0, 0.5], [0.5, 0.5]], [0.25, 0.25]),
    ([[0.25, 0.0], [0.25, 0.5]], [0.1, 0.25]),
    ([[0.0, 0.25], [0.25, 0.25]], [0.1, 0.1]),
];

/// 手順0..=4まで折ったときの外形(幅, 高さ)。
const FOLDED_SIZE: [(f64, f64); 5] = [
    (1.0, 1.0),
    (0.5, 1.0),
    (0.5, 0.5),
    (0.25, 0.5),
    (0.25, 0.25),
];

fn three_step_document() -> Document {
    folded_document(3)
}

fn folded_document(steps: usize) -> Document {
    folded_document_with_state(steps).0
}

/// `folded_document` と同じ手順を折り、折り終えた平坦状態も返す。
fn folded_document_with_state(steps: usize) -> (Document, FlatState) {
    let mut doc = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let faces = extract_faces(&doc.cp);
    let mut state = FlatState::initial(&doc.cp, &faces);
    for (i, (line, keep_side_point)) in FOLDS.into_iter().take(steps).enumerate() {
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
        .expect("指定した回数だけ折れる");
        state = res.state;
        let mut step = res.step;
        step.id = u32::try_from(i).unwrap();
        doc.sequence.push(step);
    }
    assert_eq!(doc.sequence.len(), steps);
    (doc, state)
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
}

/// 途中のステップを選んだとき、まだ折っていない折り線が曲がっていないこと。
/// 角度指定の無いヒンジをソルバーの自由変数のままにすると、初期値バイアスから
/// 別の枝へ収束して「警告の出ない誤った形」が返るため、外形寸法で検証する。
#[test]
fn each_up_to_shows_only_the_folds_done_so_far() {
    let doc = folded_document(4);
    for (k, (w, h)) in FOLDED_SIZE.into_iter().enumerate() {
        let res = replay(&doc, k, 1.0);
        assert!(res.skipped.is_empty(), "up_to={k} 警告={:?}", res.warnings);
        assert!(
            !has_step_warning(&res, "求まりませんでした"),
            "up_to={k} 警告={:?}",
            res.warnings
        );
        assert!(
            (extent(&res.frame, 0) - w).abs() < 1e-6,
            "up_to={k} 横幅={} 期待={w}",
            extent(&res.frame, 0)
        );
        assert!(
            (extent(&res.frame, 1) - h).abs() < 1e-6,
            "up_to={k} 縦幅={} 期待={h}",
            extent(&res.frame, 1)
        );
        assert!(
            extent(&res.frame, 2) < 1e-6,
            "up_to={k} 厚み={}",
            extent(&res.frame, 2)
        );
    }
}

/// 補間の下端(t=0)は「直前のステップまで折り終えた状態」と完全に一致する。
/// 手順1・t=0は全ての角度が0になる縮退ケースなので、非縮退のk=2以降も確かめる。
#[test]
fn t_zero_matches_the_previous_step_completed() {
    let doc = folded_document(4);
    for k in 1..=4usize {
        assert_eq!(
            frame_bits(&replay(&doc, k, 0.0).frame),
            frame_bits(&replay(&doc, k - 1, 1.0).frame),
            "up_to={k} t=0 が up_to={} t=1 と一致しない",
            k - 1
        );
    }
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

/// 層順序の代表点が1点も現在の面に解決できないステップは、直前の層順序を保つ
/// (resolve_orderの補完結果=面ID順を採用してしまわない)。
#[test]
fn unresolvable_layer_order_keeps_the_previous_layers() {
    let layers = |doc: &Document| -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = replay(doc, 3, 1.0)
            .frame
            .faces
            .iter()
            .map(|f| (f.face, f.layer))
            .collect();
        v.sort_unstable();
        v
    };

    // 最終ステップの層順序を「紙の外の点」だけにした文書と、層順序なしの文書
    let mut broken = three_step_document();
    broken.sequence[2].layer_order = Some(vec![[-1.0, -1.0], [2.0, 2.0]]);
    let mut dropped = three_step_document();
    dropped.sequence[2].layer_order = None;

    let res = replay(&broken, 3, 1.0);
    assert!(
        res.warnings.iter().any(|w| w.contains("代表点")),
        "解決できない代表点は警告に載る: {:?}",
        res.warnings
    );
    assert_eq!(
        layers(&broken),
        layers(&dropped),
        "1点も解決できない層順序は使わず、直前の層順序を保つ"
    );
    // 直前(手順2)の層順序は面ID昇順とは違うので、この検証には意味がある
    assert_ne!(layers(&broken), layers(&three_step_document()));
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
/// 400本の平行な折り線を交互に山谷にした蛇腹(面400・辺1,201)を、40本ずつ10ステップに
/// 分けて完全に畳む手順を与え、`replay(doc, 10, 1.0)` の実時間を測る。
/// 層順序は毎ステップ400点(=全面)を最も重い並び(現在の面の並びの逆順)で指定し、
/// 代表点の解決も含めた全再生の実力を測る。
/// 実測(2026-08-05, 開発機 Windows 11): debug 約0.7秒 / release 約23ms
/// (release実測は `cargo test -p ori3-layers --release --test replay -- --nocapture`)。
/// debugビルドにそのまま3秒の上限を課す(release目標に対し十分厳しい)。
#[test]
fn replay_of_ten_steps_on_400_faces_is_under_three_seconds() {
    const STRIPS: usize = 400;
    const STEPS: usize = 10;
    let cp = accordion_cp(STRIPS);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), STRIPS);
    assert_eq!(cp.edges.len(), 3 * STRIPS + 1);

    // 層順序: 全面の代表点を「現在の面の並びの逆順」で指定する(解決がいちばん重い形)
    let mut layer_order = FlatState::initial(&cp, &faces).to_layer_points(&cp, &faces);
    layer_order.reverse();

    // 折り線i(i=1..STRIPS-1)は y=i/STRIPS の水平線。40本ずつ10ステップに分ける
    let per_step = STRIPS.div_ceil(STEPS);
    let mut doc = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    doc.cp = cp;
    doc.sequence = (0..STEPS)
        .map(|s| FoldStep {
            id: u32::try_from(s).unwrap(),
            kind: TechniqueKind::Simple,
            drivers: (s * per_step..(s + 1) * per_step)
                .filter(|i| (1..STRIPS).contains(i))
                .map(|i| {
                    let y = i as f64 / STRIPS as f64;
                    DriverLine {
                        a: [0.0, y],
                        b: [1.0, y],
                        // 交互の山谷(accordion_cpと同じ規則)を目標角に写す
                        target_angle_deg: if i % 2 == 1 { 180.0 } else { -180.0 },
                    }
                })
                .collect(),
            layer_order: Some(layer_order.clone()),
            note: String::new(),
        })
        .collect();

    let t0 = Instant::now();
    let res = replay(&doc, STEPS, 1.0);
    let dt = t0.elapsed();
    println!("replay(10ステップ・面400) = {dt:?} 警告={:?}", res.warnings);
    assert!(res.skipped.is_empty());
    assert!(res.warnings.is_empty(), "警告={:?}", res.warnings);
    // 蛇腹は完全に畳まれ、幅1・高さ1/400になる
    assert!((extent(&res.frame, 0) - 1.0).abs() < 1e-6);
    assert!(extent(&res.frame, 1) < 1.0 / STRIPS as f64 + 1e-6);
    assert!(
        dt < Duration::from_secs(3),
        "全再生が遅すぎます: {dt:?}(NFR-002: 3秒以内)"
    );
}

/// perf用: 平行な折り線だけの蛇腹CP(面 `strips` 枚・辺 3*strips+1 本)。
/// 内部頂点が無いのでループ拘束が生まれず、どの角度指定でも姿勢が一意に決まる。
fn accordion_cp(strips: usize) -> CreasePattern {
    let vid = |i: usize, right: bool| u32::try_from(2 * i + usize::from(right)).unwrap();
    let mut vertices = Vec::new();
    for i in 0..=strips {
        let y = i as f64 / strips as f64;
        vertices.push(Vertex {
            id: vid(i, false),
            pos: [0.0, y],
        });
        vertices.push(Vertex {
            id: vid(i, true),
            pos: [1.0, y],
        });
    }
    let mut edges: Vec<Edge> = Vec::new();
    let push = |v0: u32, v1: u32, kind: EdgeKind, edges: &mut Vec<Edge>| {
        edges.push(Edge {
            id: u32::try_from(edges.len()).unwrap(),
            v0,
            v1,
            kind,
        });
    };
    for i in 0..=strips {
        let kind = if i == 0 || i == strips {
            EdgeKind::Border
        } else if i % 2 == 1 {
            EdgeKind::Mountain
        } else {
            EdgeKind::Valley
        };
        push(vid(i, false), vid(i, true), kind, &mut edges);
    }
    for i in 0..strips {
        push(
            vid(i, false),
            vid(i + 1, false),
            EdgeKind::Border,
            &mut edges,
        );
        push(vid(i, true), vid(i + 1, true), EdgeKind::Border, &mut edges);
    }
    CreasePattern {
        next_vertex_id: u32::try_from(vertices.len()).unwrap(),
        next_edge_id: u32::try_from(edges.len()).unwrap(),
        vertices,
        edges,
    }
}

// ---------------------------------------------------------------------------
// flat_state_at(手順から現在の平坦状態を導出する)
// ---------------------------------------------------------------------------

/// 手順を折り重ねた文書から導出した平坦状態が、fold_throughが返した状態と
/// そのまま一致する。1〜4手順(裏返った層を含む)で確かめる。
///
/// 導出も fold_through の出力も3D表示と同じ座標系(根面=最小面IDが恒等変換)に
/// そろえてあるので、全体のずれは残らない。
#[test]
fn flat_state_at_matches_fold_through_state() {
    for steps in 1..=4usize {
        let (doc, expected) = folded_document_with_state(steps);
        let faces = extract_faces(&doc.cp);
        let (state, _) = flat_state_at(&doc, &faces, steps).expect("平坦なのでErrにならない");

        assert_eq!(state.order, expected.order, "{steps}手順目の層順序");
        assert_eq!(
            state.placements.len(),
            expected.placements.len(),
            "{steps}手順目の面数"
        );
        assert!(
            expected.placements.values().any(|p| p.mirrored),
            "{steps}手順目には裏返った層がある"
        );
        let root = *expected.order.iter().min().expect("面がある");
        assert!(
            state.placements[&root].approx_eq(&Isometry2::identity(), 1e-12),
            "{steps}手順目: 根面(最小面ID)は恒等変換"
        );
        assert!(
            expected.placements[&root].approx_eq(&Isometry2::identity(), 1e-12),
            "{steps}手順目: fold_throughの出力も根面が恒等変換にそろっている"
        );
        for (id, want) in &expected.placements {
            let got = state.placements.get(id).expect("同じ面IDが揃う");
            assert!(
                got.approx_eq(want, 1e-9),
                "{steps}手順目の面{id}の配置が違う: got={got:?}, want={want:?}"
            );
        }
    }
}

/// 動かさない側に根面(最小面ID)がある折り方では、fold_throughの状態と完全に一致する
/// (全体のずれが生じない)。
#[test]
fn flat_state_at_matches_exactly_when_root_face_is_kept() {
    // 面0は右半分になるので、右を残して折ると根面は動かない
    let mut doc = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    let faces = extract_faces(&doc.cp);
    let initial = FlatState::initial(&doc.cp, &faces);
    let res = fold_through(
        &mut doc.cp,
        &faces,
        &initial,
        &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            keep_side_point: [0.75, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("半分折りは成功する");
    let mut step = res.step;
    step.id = 0;
    doc.sequence.push(step);

    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, 1).expect("平坦なのでErrにならない");
    let root = *state.order.iter().min().expect("面がある");
    assert!(
        res.state.placements[&root].approx_eq(&Isometry2::identity(), 1e-12),
        "この折り方では根面は動かない"
    );
    assert_eq!(state.order, res.state.order);
    for (id, want) in &res.state.placements {
        assert!(
            state.placements[id].approx_eq(want, 1e-9),
            "面{id}の配置が違う: got={:?}, want={want:?}",
            state.placements[id]
        );
    }
}

/// 折り途中(平坦でない)状態からは折れない。
#[test]
fn flat_state_at_rejects_unfolded_state() {
    let (mut doc, _) = folded_document_with_state(1);
    // 目標角を±180°から90°へ書き換えると、再生しても平らにならない
    for d in &mut doc.sequence[0].drivers {
        d.target_angle_deg = 90.0;
    }
    let faces = extract_faces(&doc.cp);
    let err = flat_state_at(&doc, &faces, 1).unwrap_err();
    assert!(err.contains("折り途中"), "err={err}");

    // 手順0(まだ折っていない平らな紙)は平坦状態として扱える
    let (state, _) = flat_state_at(&doc, &faces, 0).expect("平らな紙は平坦");
    assert!(
        state
            .placements
            .values()
            .all(|p| p.approx_eq(&Isometry2::identity(), 1e-12)),
        "折る前は全ての面が恒等変換"
    );
}

/// 導出した平坦状態の上に、さらに折り操作を重ねられる。
#[test]
fn flat_state_at_feeds_next_fold_through() {
    let (mut doc, _) = folded_document_with_state(2);
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(&doc, &faces, 2).expect("平坦");
    let before_edges = doc.cp.edges.len();
    let res = fold_through(
        &mut doc.cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.25, 0.0], [0.25, 0.5]],
            keep_side_point: [0.1, 0.25],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("畳んだ状態の上に折れる");
    assert!(doc.cp.edges.len() > before_edges, "展開図に折り線が増える");
    assert!(!res.step.drivers.is_empty(), "手順に折り線が記録される");
    // 4層すべてを折るので、折り上がりは8層
    assert_eq!(res.state.order.len(), 8);
}
