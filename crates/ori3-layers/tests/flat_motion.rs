//! 汎用の折り操作([`flat_motion`])のテスト。
//!
//! 目的は「実際の紙で折れる動きはすべてこの1つの部品で表せる」ことの確認
//! (要件定義書の設計原則0・SIM-011)。技法として仕上げることは目的ではなく、
//! 設計フェーズで洗い出した動きが**表現できる**ことを1件ずつ示す。
//!
//! - 単純折り / 一部の層だけ折る / 奇数の重なり
//! - 既存の折り目を開く(角度0°へ)
//! - 紙が1mmも動かない遷移(重なり順と山谷だけが変わる)
//! - 層ごとに逆向きに回す(中割り折り・かぶせ折りの中身)
//! - 鏡映2回=回転を含む動き(つぶし折り・ねじり折りの中身)
//! - 開く動きと折る動きを1回の操作にまとめる(花弁折りの中身)
//! - 領域の中だけ重なりを裏返す(沈め折りの中身)
//! - 裂ける指定は警告して続行、定義できない指定はErrで展開図を触らない

use std::collections::{HashMap, HashSet};

use glam::DVec2;
use ori3_cp::{Face, extract_faces};
use ori3_geometry::Isometry2;
use ori3_layers::flat_state::representative_point;
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, FoldThroughResult, fold_through};
use ori3_layers::{
    FlatMotionInput, FlatState, HalfPlane, LayerTurn, MotionPart, MotionTransform, flat_motion,
    flat_state_at, replay,
};
use ori3_model::{CreasePattern, Document, EdgeId, EdgeKind, FaceId, Paper, TechniqueKind};

/// 折り終わる直前(t=0.99)に浮いていると判断する高さ。
const LIFT_EPS: f64 = 1e-3;

fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

fn state_of(doc: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(doc, &faces, doc.sequence.len()).expect("平らな状態から動かす");
    (faces, state)
}

/// 従来の折り操作で1手折る(手順にも記録する)。
fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2], direction: FoldDirection) {
    let (faces, state) = state_of(doc);
    let mut cp = doc.cp.clone();
    let res = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line,
            keep_side_point: keep,
            target_layers: None,
            direction,
        },
    )
    .expect("折れる指定");
    let mut step = res.step;
    step.id = u32::try_from(doc.sequence.len()).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
}

/// 汎用の折り操作を1手適用する(手順にも記録する)。
fn motion(doc: &mut Document, parts: Vec<MotionPart>) -> FoldThroughResult {
    let (faces, state) = state_of(doc);
    let mut cp = doc.cp.clone();
    let res = flat_motion(
        &mut cp,
        &faces,
        &state,
        &FlatMotionInput {
            parts,
            kind: TechniqueKind::Simple,
        },
    )
    .expect("動かせる指定");
    let mut step = res.step.clone();
    step.id = u32::try_from(doc.sequence.len()).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
    res
}

/// 記録した手順から状態を求め直すと、返ってきた平坦状態と一致する。
fn assert_replay_matches(doc: &Document, res: &FoldThroughResult, label: &str) {
    let faces = extract_faces(&doc.cp);
    let (again, _) = flat_state_at(doc, &faces, doc.sequence.len())
        .unwrap_or_else(|e| panic!("{label}: 再生で平らな状態が求まらない: {e}"));
    assert_eq!(res.state.order, again.order, "{label}: 層順序が再生と一致する");
    for f in &faces {
        assert!(
            res.state.placements[&f.id].approx_eq(&again.placements[&f.id], 1e-6),
            "{label}: 面 {} の配置が再生と一致する({:?} と {:?})",
            f.id,
            res.state.placements[&f.id],
            again.placements[&f.id]
        );
    }
}

/// 畳んだ紙の見えている範囲(全ての面を配置で写した外接矩形)。
fn bbox(cp: &CreasePattern, faces: &[Face], state: &FlatState) -> ([f64; 2], [f64; 2]) {
    let pos: HashMap<u32, DVec2> = cp
        .vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect();
    let mut lo = DVec2::splat(f64::INFINITY);
    let mut hi = DVec2::splat(f64::NEG_INFINITY);
    for f in faces {
        let pl = state.placements[&f.id];
        for p in f.vertices.iter().filter_map(|v| pos.get(v)) {
            let q = pl.apply(*p);
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    ([lo.x, lo.y], [hi.x, hi.y])
}

fn kinds(cp: &CreasePattern) -> HashMap<EdgeId, EdgeKind> {
    cp.edges.iter().map(|e| (e.id, e.kind)).collect()
}

fn edge_midpoint(cp: &CreasePattern, id: EdgeId) -> DVec2 {
    let e = cp.edges.iter().find(|e| e.id == id).expect("辺が存在する");
    let p = |v: u32| DVec2::from(cp.vertices.iter().find(|x| x.id == v).unwrap().pos);
    (p(e.v0) + p(e.v1)) * 0.5
}

/// `up_to` 手目の層順序が、折り終わる直前(t=0.99)の高さから読んだ重なりと一致するか。
fn assert_display_order(doc: &Document, up_to: usize, label: &str) {
    let frame = replay(doc, up_to, 0.99).frame;
    let mut lifted: HashSet<FaceId> = HashSet::new();
    let mut sign = 0.0f64;
    for f in &frame.faces {
        let z = f
            .polygon
            .iter()
            .map(|p| p[2])
            .max_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(0.0);
        if z.abs() > LIFT_EPS {
            lifted.insert(f.face);
            if sign == 0.0 {
                sign = z.signum();
            }
        }
    }
    assert!(!lifted.is_empty(), "{label}: 折り終わる直前には層が浮いている");
    let faces = extract_faces(&doc.cp);
    let (state, _) = flat_state_at(doc, &faces, up_to).expect("畳んだ状態");
    let order = state.order;
    let n = lifted.len();
    let (slice, where_) = if sign > 0.0 {
        (&order[order.len() - n..], "上側")
    } else {
        (&order[..n], "下側")
    };
    let got: HashSet<FaceId> = slice.iter().copied().collect();
    assert_eq!(
        got, lifted,
        "{label}: 浮いた層({lifted:?})は層順序の{where_}に固まる(order={order:?})"
    );
}

// ---------------------------------------------------------------------------
// 1. 単純折り: 従来の折り操作とまったく同じ結果になる
// ---------------------------------------------------------------------------

#[test]
fn simple_fold_gives_the_same_result_as_fold_through() {
    let doc = square_doc();
    let faces = extract_faces(&doc.cp);
    let state = FlatState::initial(&doc.cp, &faces);
    let line = [[0.5, 0.0], [0.5, 1.0]];

    let mut cp_a = doc.cp.clone();
    let a = fold_through(
        &mut cp_a,
        &faces,
        &state,
        &FoldThroughInput {
            line,
            keep_side_point: [0.25, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("従来の折り");

    let mut cp_b = doc.cp.clone();
    let b = flat_motion(
        &mut cp_b,
        &faces,
        &state,
        &FlatMotionInput {
            parts: vec![MotionPart::fold(
                Vec::new(),
                line,
                [0.75, 0.5],
                FoldDirection::Up,
            )],
            kind: TechniqueKind::Simple,
        },
    )
    .expect("汎用の折り");

    assert_eq!(cp_a, cp_b, "展開図が一致する");
    assert_eq!(a.state.order, b.state.order, "層順序が一致する");
    assert_eq!(a.step.drivers, b.step.drivers, "記録する折り線が一致する");
    assert_eq!(a.added_edges, b.added_edges);
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
}

// ---------------------------------------------------------------------------
// 2. 折り目を開く(角度0°へ)
// ---------------------------------------------------------------------------

#[test]
fn opening_a_crease_brings_the_paper_back_flat() {
    let mut doc = square_doc();
    fold(&mut doc, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5], FoldDirection::Up);

    let (faces, state) = state_of(&doc);
    assert_eq!(faces.len(), 2);
    // 裏返っている(折り返された)層を、折り目の線で鏡映して開く。
    let folded_layer = faces
        .iter()
        .find(|f| state.placements[&f.id].mirrored)
        .expect("折り返された層がある")
        .id;
    // 折り目は畳み平面では紙の縁に見えている。展開図では x=0.5。
    let (lo, hi) = bbox(&doc.cp, &faces, &state);
    let seam = [[lo[0], lo[1]], [lo[0], hi[1]]];

    let res = motion(&mut doc, vec![MotionPart::open(vec![folded_layer], seam)]);

    assert!(res.warnings.is_empty(), "{:?}", res.warnings);
    assert!(res.added_edges.is_empty(), "開く動きは折り線を増やさない");
    assert_eq!(res.step.drivers.len(), 1, "折り目1本ぶんの記録");
    assert_eq!(
        res.step.drivers[0].target_angle_deg, 0.0,
        "開いた折り目は0°で記録される"
    );
    // 全ての面が同じ配置(平ら)に戻る
    let after = extract_faces(&doc.cp);
    let first = res.state.placements[&after[0].id];
    for f in &after {
        assert!(
            res.state.placements[&f.id].approx_eq(&first, 1e-9),
            "面 {} が平らに戻る",
            f.id
        );
    }
    assert_replay_matches(&doc, &res, "折り目を開く");
}

// ---------------------------------------------------------------------------
// 3. 紙が1mmも動かない遷移(重なり順と山谷だけが変わる)
// ---------------------------------------------------------------------------

#[test]
fn restacking_without_moving_paper_only_changes_layers_and_creases() {
    // 正方形を2回半分に折った4層。ここから紙をまったく動かさずに、
    // いちばん下の層をいちばん上へ回す(つぶし折りの退化ケースと同じ形の遷移)。
    let mut doc = square_doc();
    fold(&mut doc, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5], FoldDirection::Up);
    let (faces, state) = state_of(&doc);
    let (lo, hi) = bbox(&doc.cp, &faces, &state);
    let mid_y = 0.5 * (lo[1] + hi[1]);
    fold(
        &mut doc,
        [[lo[0], mid_y], [hi[0], mid_y]],
        [0.5 * (lo[0] + hi[0]), lo[1] + 0.25 * (hi[1] - lo[1])],
        FoldDirection::Up,
    );

    let (faces, state) = state_of(&doc);
    assert_eq!(faces.len(), 4, "4層になる");
    let before_kinds = kinds(&doc.cp);
    let bottom = state.order[0];
    let expected: Vec<FaceId> = state.order[1..]
        .iter()
        .copied()
        .chain(std::iter::once(bottom))
        .collect();

    let res = motion(
        &mut doc,
        vec![MotionPart::restack(
            vec![bottom],
            LayerTurn::Outside(FoldDirection::Up),
        )],
    );

    assert!(res.warnings.is_empty(), "{:?}", res.warnings);
    assert!(res.added_edges.is_empty(), "折り線は1本も増えない");
    // 紙は1mmも動かない
    for f in &faces {
        assert!(
            res.state.placements[&f.id].approx_eq(&state.placements[&f.id], 1e-12),
            "面 {} の配置は変わらない",
            f.id
        );
    }
    assert_eq!(res.state.order, expected, "いちばん下の層が上へ回る");
    // 変わるのは折り目1組(いちばん下の層に接する2本)の山谷だけ
    let after_kinds = kinds(&doc.cp);
    let changed: Vec<EdgeId> = after_kinds
        .iter()
        .filter(|(id, k)| before_kinds.get(id) != Some(k))
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(changed.len(), 2, "山谷が入れ替わるのは2本(changed={changed:?})");
    assert_eq!(res.step.drivers.len(), 2, "その2本ぶんだけ手順に記録される");
    for d in &res.step.drivers {
        assert!(d.target_angle_deg.abs() == 180.0, "{d:?}");
    }
    assert_replay_matches(&doc, &res, "紙が動かない遷移");
}

/// `Beside` の置き場所が見つからないときも、入れる側は向きで決まる。
///
/// 基準面の紙が全部動くと「その隣」を指す層が1つも残らない。この場合でも
/// 向こう側(Down)を指定した紙は重なりの下へ、手前側(Up)を指定した紙は上へ入る
/// (向きを無視して常に最上へ入れると、Downの指定が効かない)。
#[test]
fn beside_without_a_neighbour_still_follows_the_direction() {
    for direction in [FoldDirection::Up, FoldDirection::Down] {
        let mut doc = square_doc();
        fold(&mut doc, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5], FoldDirection::Up);
        let (faces, state) = state_of(&doc);
        let (lo, hi) = bbox(&doc.cp, &faces, &state);
        let mid_y = 0.5 * (lo[1] + hi[1]);
        fold(
            &mut doc,
            [[lo[0], mid_y], [hi[0], mid_y]],
            [0.5 * (lo[0] + hi[0]), lo[1] + 0.25 * (hi[1] - lo[1])],
            FoldDirection::Up,
        );

        let (_, state) = state_of(&doc);
        assert_eq!(state.order.len(), 4, "4層になる");
        let top = *state.order.last().expect("最前面");
        let rest: Vec<FaceId> = state.order[..3].to_vec();
        let expected: Vec<FaceId> = match direction {
            FoldDirection::Up => state.order.clone(),
            FoldDirection::Down => std::iter::once(top).chain(rest).collect(),
        };

        let res = motion(
            &mut doc,
            vec![MotionPart::restack(
                vec![top],
                LayerTurn::Beside {
                    anchor: top,
                    direction,
                },
            )],
        );
        assert_eq!(
            res.state.order, expected,
            "{direction:?} を指定した紙はその側へ入る"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. 層ごとに逆向きへ回す(中割り折り・かぶせ折りの中身)
// ---------------------------------------------------------------------------

#[test]
fn one_motion_can_turn_each_layer_the_opposite_way() {
    // 半分に折った2層。先端(上端側)を、下の層は上へ・上の層は下へ回すと、
    // 先端が2枚の間に挟まる(=中割り折りの重なり)。
    let mut doc = square_doc();
    fold(&mut doc, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5], FoldDirection::Up);

    let (faces, state) = state_of(&doc);
    let (lo, hi) = bbox(&doc.cp, &faces, &state);
    let cut = lo[1] + 0.75 * (hi[1] - lo[1]);
    let line = [[lo[0], cut], [hi[0], cut]];
    let tip_point = [0.5 * (lo[0] + hi[0]), lo[1] + 0.9 * (hi[1] - lo[1])];
    let lower = state.order[0];
    let upper = state.order[1];

    let part = |layer: FaceId, dir: FoldDirection| MotionPart {
        layers: vec![layer],
        region: vec![HalfPlane {
            line,
            inside_point: tip_point,
        }],
        transform: MotionTransform::Reflect(vec![line]),
        turn: LayerTurn::Inside(dir),
        reverse_layers: None,
    };
    let res = motion(
        &mut doc,
        vec![
            part(lower, FoldDirection::Up),
            part(upper, FoldDirection::Down),
        ],
    );

    assert!(res.warnings.is_empty(), "紙は裂けない: {:?}", res.warnings);
    let after = extract_faces(&doc.cp);
    assert_eq!(after.len(), 4, "2層それぞれが先端と胴に分かれる");
    assert_eq!(res.state.order.len(), 4);

    // 先端(動いた面)は真ん中2枚に収まる
    let moved: HashSet<FaceId> = after
        .iter()
        .filter(|f| {
            let r = DVec2::from(representative_point(&doc.cp, f));
            let q = res.state.placements[&f.id].apply(r);
            // 動いた先端は、折り線をまたいで反対側(先端側でない方)へ写っている
            r.y > cut && q.y < cut || r.y < cut && q.y > cut
        })
        .map(|f| f.id)
        .collect();
    assert_eq!(moved.len(), 2, "動いたのは2枚の先端");
    let middle: HashSet<FaceId> = res.state.order[1..3].iter().copied().collect();
    assert_eq!(middle, moved, "先端は胴2枚の間に挟まる(order={:?})", res.state.order);

    // 背(2層をつなぐ折り目)は先端側だけ山谷が入れ替わる
    let seam_kinds: Vec<(f64, EdgeKind)> = doc
        .cp
        .edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley))
        .map(|e| (edge_midpoint(&doc.cp, e.id), e.kind))
        .filter(|(m, _)| (m.x - 0.5).abs() < 1e-9)
        .map(|(m, k)| (m.y, k))
        .collect();
    assert_eq!(seam_kinds.len(), 2, "背は先端側と胴側の2本に分かれる");
    assert_ne!(
        seam_kinds[0].1, seam_kinds[1].1,
        "先端の背は山谷が入れ替わる({seam_kinds:?})"
    );
    assert_replay_matches(&doc, &res, "層ごとに逆向き");
}

// ---------------------------------------------------------------------------
// 5. 鏡映2回=回転を含む動き(つぶし折り・ねじり折りの中身)
// ---------------------------------------------------------------------------

#[test]
fn one_motion_can_use_a_rotation_made_of_two_reflections() {
    // 正方形を「2回半分に折る」動きを1回の操作で表す。奥の1/4は鏡映2回=回転で動く。
    let mut doc = square_doc();
    let vline = [[0.5, 0.0], [0.5, 1.0]];
    let hline = [[0.0, 0.5], [1.0, 0.5]];
    let quadrant = |inside: [f64; 2], t: MotionTransform| MotionPart {
        layers: Vec::new(),
        region: vec![
            HalfPlane {
                line: vline,
                inside_point: inside,
            },
            HalfPlane {
                line: hline,
                inside_point: inside,
            },
        ],
        transform: t,
        turn: LayerTurn::Outside(FoldDirection::Up),
        reverse_layers: None,
    };
    let res = motion(
        &mut doc,
        vec![
            quadrant([0.75, 0.25], MotionTransform::Reflect(vec![vline])),
            // 右上は「縦で折ってから横で折る」= 回転
            quadrant([0.75, 0.75], MotionTransform::Reflect(vec![vline, hline])),
            quadrant([0.25, 0.75], MotionTransform::Reflect(vec![hline])),
        ],
    );

    assert!(res.warnings.is_empty(), "紙は裂けない: {:?}", res.warnings);
    let after = extract_faces(&doc.cp);
    assert_eq!(after.len(), 4, "4つの1/4に分かれる");

    // 全ての1/4が同じ1/4の場所へ重なる
    for f in &after {
        let r = DVec2::from(representative_point(&doc.cp, f));
        let q = res.state.placements[&f.id].apply(r);
        assert!(
            (0.0 - 1e-9..=0.5 + 1e-9).contains(&q.x) && (0.0 - 1e-9..=0.5 + 1e-9).contains(&q.y),
            "面 {} は1/4へ畳まれる(実際: {q:?})",
            f.id
        );
    }

    // 右上の1/4は裏返らない(回転)、左上・右下は裏返る(鏡映1回)
    let by_quadrant = |right: bool, top: bool| -> Isometry2 {
        let f = after
            .iter()
            .find(|f| {
                let r = representative_point(&doc.cp, f);
                (r[0] > 0.5) == right && (r[1] > 0.5) == top
            })
            .expect("その象限の面がある");
        res.state.placements[&f.id]
    };
    let base = by_quadrant(false, false); // 動かない左下
    let rotated = by_quadrant(true, true).compose(&base.inverse());
    assert!(!rotated.mirrored, "右上は鏡映2回なので裏返らない");
    assert!(
        (rotated.rotation - std::f64::consts::PI).abs() < 1e-9,
        "右上は180°回転する(実際 {}rad)",
        rotated.rotation
    );
    assert!(by_quadrant(true, false).compose(&base.inverse()).mirrored);
    assert!(by_quadrant(false, true).compose(&base.inverse()).mirrored);

    // 山谷は重なり順から決め直されるので、中心の頂点で前川定理を満たす
    let m = doc
        .cp
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Mountain)
        .count();
    let v = doc
        .cp
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Valley)
        .count();
    assert_eq!(m + v, 4, "折り線は4本");
    assert_eq!(
        (m as i32 - v as i32).abs(),
        2,
        "山と谷の差は2(前川定理。山{m}本・谷{v}本)"
    );

    assert_replay_matches(&doc, &res, "回転を含む動き");
    assert_display_order(&doc, doc.sequence.len(), "回転を含む動き");
}

// ---------------------------------------------------------------------------
// 6. 一部の層だけ・奇数の重なりでも動かせる(裂ける指定は警告して続行)
// ---------------------------------------------------------------------------

#[test]
fn moving_only_some_layers_of_an_odd_stack_warns_but_continues() {
    // 段折りで3層(奇数)にしてから、上2枚だけを折る。
    let mut doc = square_doc();
    fold(&mut doc, [[0.75, 0.0], [0.75, 1.0]], [0.5, 0.5], FoldDirection::Up);
    fold(&mut doc, [[1.0, 0.0], [1.0, 1.0]], [0.9, 0.5], FoldDirection::Down);

    let (faces, state) = state_of(&doc);
    assert_eq!(faces.len(), 3, "3層(奇数)になる");
    let (lo, hi) = bbox(&doc.cp, &faces, &state);
    let cut = 0.5 * (lo[1] + hi[1]);
    let line = [[lo[0], cut], [hi[0], cut]];
    let upper = [0.5 * (lo[0] + hi[0]), hi[1] - 0.1 * (hi[1] - lo[1])];
    let targets = vec![state.order[1], state.order[2]];

    let before = doc.cp.clone();
    let res = motion(
        &mut doc,
        vec![MotionPart::fold(
            targets.clone(),
            line,
            upper,
            FoldDirection::Up,
        )],
    );

    assert!(
        res.warnings.iter().any(|w| w.contains("裂け")),
        "つながりが切れる指定は警告する: {:?}",
        res.warnings
    );
    assert_ne!(doc.cp, before, "警告付きでも折りは実行される");
    assert_eq!(res.state.order.len(), 5, "上2枚がそれぞれ2つに分かれる");
    // 動いた紙は重なりのいちばん上に固まる。座標系は動いていない層(order[0])を
    // 基準にそろえて比べる(全体の等長変換は折るたびに付け直されるため)。
    let after = extract_faces(&doc.cp);
    let parent_of = |nf: &Face| -> FaceId {
        let r = representative_point(&doc.cp, nf);
        faces
            .iter()
            .find(|f| ori3_layers::point_in_face(&before, f, r))
            .expect("親の面が見つかる")
            .id
    };
    let anchor = after
        .iter()
        .find(|nf| parent_of(nf) == state.order[0])
        .expect("動かない層の子");
    let new_base = res.state.placements[&anchor.id].inverse();
    let old_base = state.placements[&state.order[0]].inverse();
    let moved: HashSet<FaceId> = after
        .iter()
        .filter(|nf| {
            let now = new_base.compose(&res.state.placements[&nf.id]);
            let was = old_base.compose(&state.placements[&parent_of(nf)]);
            !now.approx_eq(&was, 1e-9)
        })
        .map(|f| f.id)
        .collect();
    assert_eq!(moved.len(), 2, "動いたのは指定した2枚の上半分");
    let top: HashSet<FaceId> = res.state.order[res.state.order.len() - moved.len()..]
        .iter()
        .copied()
        .collect();
    assert_eq!(top, moved, "動いた層が上に固まる(order={:?})", res.state.order);
}

// ---------------------------------------------------------------------------
// 7. 開く動きと折る動きを1回にまとめる(花弁折り・つぶし折りの中身)
// ---------------------------------------------------------------------------

#[test]
fn one_motion_can_open_one_crease_and_make_another() {
    let mut doc = square_doc();
    fold(&mut doc, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5], FoldDirection::Up);

    let (faces, state) = state_of(&doc);
    let (lo, hi) = bbox(&doc.cp, &faces, &state);
    let seam = [[lo[0], lo[1]], [lo[0], hi[1]]];
    let folded_layer = faces
        .iter()
        .find(|f| state.placements[&f.id].mirrored)
        .expect("折り返された層")
        .id;
    let flat_layer = faces
        .iter()
        .find(|f| !state.placements[&f.id].mirrored)
        .expect("動いていない層")
        .id;
    // 開く層と反対の端を、同じ操作の中で折る
    let cut_x = lo[0] + 0.75 * (hi[0] - lo[0]);
    let cut = [[cut_x, lo[1]], [cut_x, hi[1]]];

    let res = motion(
        &mut doc,
        vec![
            MotionPart::open(vec![folded_layer], seam),
            MotionPart::fold(
                vec![flat_layer],
                cut,
                [hi[0], 0.5 * (lo[1] + hi[1])],
                FoldDirection::Up,
            ),
        ],
    );

    assert!(res.warnings.is_empty(), "{:?}", res.warnings);
    assert!(
        res.step.drivers.iter().any(|d| d.target_angle_deg == 0.0),
        "開いた折り目は0°で記録される: {:?}",
        res.step.drivers
    );
    assert!(
        res.step
            .drivers
            .iter()
            .any(|d| d.target_angle_deg.abs() == 180.0),
        "新しく折った折り目は±180°で記録される: {:?}",
        res.step.drivers
    );
    assert_eq!(res.added_edges.len(), 1, "新しい折り線は1本");
    assert_replay_matches(&doc, &res, "開く+折る");
}

// ---------------------------------------------------------------------------
// 8. 領域の中だけ重なりを裏返す(沈め折りの組み合わせ的な中身)
// ---------------------------------------------------------------------------

#[test]
fn layers_inside_a_region_can_be_turned_inside_out_without_moving() {
    let mut doc = square_doc();
    fold(&mut doc, [[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5], FoldDirection::Up);
    let (faces, state) = state_of(&doc);
    let (lo, hi) = bbox(&doc.cp, &faces, &state);
    let mid_y = 0.5 * (lo[1] + hi[1]);
    fold(
        &mut doc,
        [[lo[0], mid_y], [hi[0], mid_y]],
        [0.5 * (lo[0] + hi[0]), lo[1] + 0.25 * (hi[1] - lo[1])],
        FoldDirection::Up,
    );

    let (faces, state) = state_of(&doc);
    let (lo, hi) = bbox(&doc.cp, &faces, &state);
    // 角の三角形の領域(1本の斜め線で切り取る)の中だけ、重なりを裏返す
    let line = [[lo[0], 0.5 * (lo[1] + hi[1])], [0.5 * (lo[0] + hi[0]), lo[1]]];
    let inside = [lo[0] + 0.1 * (hi[0] - lo[0]), lo[1] + 0.1 * (hi[1] - lo[1])];
    let before_order = state.order.clone();

    let res = motion(
        &mut doc,
        vec![MotionPart {
            layers: Vec::new(),
            region: vec![HalfPlane {
                line,
                inside_point: inside,
            }],
            transform: MotionTransform::Stay,
            turn: LayerTurn::Keep,
            reverse_layers: Some(true),
        }],
    );

    assert!(!res.added_edges.is_empty(), "領域の境界が折り線になる");
    // 紙はまったく動かない
    for f in &faces {
        let child = extract_faces(&doc.cp)
            .into_iter()
            .find(|nf| {
                ori3_layers::point_in_face(&doc.cp, f, representative_point(&doc.cp, nf))
            })
            .expect("元の面に含まれる新しい面");
        assert!(
            res.state.placements[&child.id].approx_eq(&state.placements[&f.id], 1e-12),
            "面 {} は動かない",
            f.id
        );
    }
    // 領域の中の4枚は重なり順が逆になり、外の並びは元のまま
    let inner: Vec<FaceId> = res
        .state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let f = extract_faces(&doc.cp)
                .into_iter()
                .find(|f| f.id == *id)
                .unwrap();
            let q = res.state.placements[id].apply(DVec2::from(representative_point(&doc.cp, &f)));
            let d = DVec2::from(line[1]) - DVec2::from(line[0]);
            let s = d.perp_dot(q - DVec2::from(line[0]));
            let si = d.perp_dot(DVec2::from(inside) - DVec2::from(line[0]));
            s * si > 0.0
        })
        .collect();
    assert_eq!(inner.len(), 4, "領域の中は4枚");
    assert_ne!(
        inner, before_order,
        "領域の中の重なり順が入れ替わる(order={:?})",
        res.state.order
    );
}

// ---------------------------------------------------------------------------
// 9. 定義できない指定はErrで、展開図を一切変更しない
// ---------------------------------------------------------------------------

#[test]
fn undefined_inputs_error_without_touching_the_crease_pattern() {
    let doc = square_doc();
    let faces = extract_faces(&doc.cp);
    let state = FlatState::initial(&doc.cp, &faces);
    let before = doc.cp.clone();
    let mut cp = doc.cp.clone();

    let run = |cp: &mut CreasePattern, parts: Vec<MotionPart>| {
        flat_motion(
            cp,
            &faces,
            &state,
            &FlatMotionInput {
                parts,
                kind: TechniqueKind::Simple,
            },
        )
    };

    // 動かす紙の指定が無い
    let err = run(&mut cp, Vec::new()).expect_err("部分が無ければエラー");
    assert!(err.contains("指定されていません"), "{err}");
    assert_eq!(cp, before);

    // 退化した折り線
    let err = run(
        &mut cp,
        vec![MotionPart::fold(
            Vec::new(),
            [[0.5, 0.5], [0.5, 0.5]],
            [0.75, 0.5],
            FoldDirection::Up,
        )],
    )
    .expect_err("退化した折り線はエラー");
    assert!(err.contains("一致"), "{err}");
    assert_eq!(cp, before);

    // 動かす側を示す点が折り線の上にある
    let err = run(
        &mut cp,
        vec![MotionPart {
            layers: Vec::new(),
            region: vec![HalfPlane {
                line: [[0.5, 0.0], [0.5, 1.0]],
                inside_point: [0.5, 0.5],
            }],
            transform: MotionTransform::Stay,
            turn: LayerTurn::Keep,
            reverse_layers: None,
        }],
    )
    .expect_err("側が決まらない指定はエラー");
    assert!(err.contains("折り線上"), "{err}");
    assert_eq!(cp, before);

    // 存在しない面だけを指定した(残る対象が無い)
    let err = run(
        &mut cp,
        vec![MotionPart::restack(
            vec![999],
            LayerTurn::Outside(FoldDirection::Up),
        )],
    )
    .expect_err("対象が無ければエラー");
    assert!(err.contains("対象の層がありません"), "{err}");
    assert_eq!(cp, before, "失敗時は展開図を一切変更しない");
}
