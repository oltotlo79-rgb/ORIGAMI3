//! fold_through(折り操作プリミティブ)のテスト。

use glam::DVec2;
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_geometry::Isometry2;
use ori3_layers::flat_state::{FlatState, point_in_face, representative_point};
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, FoldThroughResult, fold_through};
use ori3_model::{CreasePattern, Document, EdgeKind, FaceId, Paper};

/// 正方形(輪郭4辺のみ)のCPを作る。
fn square_cp() -> CreasePattern {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
    .cp
}

fn face_by_id(faces: &[Face], id: FaceId) -> &Face {
    faces.iter().find(|f| f.id == id).expect("面IDが存在する")
}

fn edge_kind(cp: &CreasePattern, id: u32) -> EdgeKind {
    cp.edges.iter().find(|e| e.id == id).expect("辺が存在する").kind
}

/// 辺の中点(CP座標)。
fn edge_midpoint(cp: &CreasePattern, id: u32) -> DVec2 {
    let e = cp.edges.iter().find(|e| e.id == id).expect("辺が存在する");
    let p = |vid: u32| DVec2::from(cp.vertices.iter().find(|v| v.id == vid).unwrap().pos);
    (p(e.v0) + p(e.v1)) * 0.5
}

/// 正方形を x=0.5(Up)→ y=0.5(Up)の2回で4層に畳む。戻り値は(CP, 2回目の結果)。
fn four_layer_fixture() -> (CreasePattern, FoldThroughResult) {
    let mut cp = square_cp();
    let faces = extract_faces(&cp);
    let state = FlatState::initial(&cp, &faces);
    let r1 = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            keep_side_point: [0.25, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("1回目の折り");
    let faces = extract_faces(&cp);
    let r2 = fold_through(
        &mut cp,
        &faces,
        &r1.state,
        &FoldThroughInput {
            line: [[0.0, 0.5], [0.5, 0.5]],
            keep_side_point: [0.25, 0.25],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("2回目の折り");
    (cp, r2)
}

#[test]
fn half_fold_up_makes_two_layers_with_one_valley() {
    let mut cp = square_cp();
    let faces = extract_faces(&cp);
    let state = FlatState::initial(&cp, &faces);
    let res = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            keep_side_point: [0.25, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("半分折りは成功する");

    assert!(res.warnings.is_empty(), "{:?}", res.warnings);
    // CPに谷線1本
    assert_eq!(res.added_edges.len(), 1, "追加される折り線は1本");
    assert_eq!(edge_kind(&cp, res.added_edges[0]), EdgeKind::Valley);

    // 層2枚で、元の右半面が上
    let new_faces = extract_faces(&cp);
    assert_eq!(new_faces.len(), 2);
    assert_eq!(res.state.order.len(), 2);
    let top = *res.state.order.last().unwrap();
    let r = representative_point(&cp, face_by_id(&new_faces, top));
    assert!(r[0] > 0.5, "上の層は元の右半面(代表点 {r:?})");
    let q = res.state.placements[&top].apply(DVec2::from(r));
    assert!(
        (q.x - (1.0 - r[0])).abs() < 1e-9 && (q.y - r[1]).abs() < 1e-9,
        "右半面は x=0.5 の鏡映で左側に重なる(実際: {q:?})"
    );
    assert!(res.state.placements[&top].mirrored, "折り返した層は裏返る");
    let bottom = res.state.order[0];
    assert!(
        res.state.placements[&bottom].approx_eq(&Isometry2::identity(), 1e-12),
        "動かさない側は恒等変換のまま"
    );

    // FoldStep: drivers=谷なので-180が1本、layer_orderあり
    assert_eq!(res.step.kind, ori3_model::TechniqueKind::Simple);
    assert_eq!(res.step.drivers.len(), 1);
    assert_eq!(res.step.drivers[0].hinge, res.added_edges[0]);
    assert_eq!(res.step.drivers[0].target_angle_deg, -180.0);
    assert_eq!(res.step.layer_order.as_ref().map(Vec::len), Some(2));
}

#[test]
fn second_fold_pulls_back_two_lines_with_opposite_kinds() {
    let (cp, r2) = four_layer_fixture();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 4, "面は4つに分かれる");
    assert_eq!(r2.state.order.len(), 4, "層は4枚");
    assert!(r2.warnings.is_empty(), "{:?}", r2.warnings);

    // 2枚の層それぞれへの引き戻しで折り線は2本追加される
    assert_eq!(r2.added_edges.len(), 2);
    for &eid in &r2.added_edges {
        let mid = edge_midpoint(&cp, eid);
        // CP左半分は表向きの層由来なので谷、右半分は裏返った層由来なので山
        let expected = if mid.x < 0.5 {
            EdgeKind::Valley
        } else {
            EdgeKind::Mountain
        };
        assert_eq!(
            edge_kind(&cp, eid),
            expected,
            "mirroredな層では山谷が反転する(辺 {eid}, 中点 {mid:?})"
        );
        let d = r2
            .step
            .drivers
            .iter()
            .find(|d| d.hinge == eid)
            .expect("追加辺ごとにdriverが付く");
        let expected_angle = if expected == EdgeKind::Valley {
            -180.0
        } else {
            180.0
        };
        assert_eq!(d.target_angle_deg, expected_angle);
    }
    assert_eq!(r2.step.drivers.len(), 2);

    // 層順序: 下から 左下→右下→右上→左上 の象限
    let quad = |id: FaceId| {
        let r = representative_point(&cp, face_by_id(&faces, id));
        (r[0] > 0.5, r[1] > 0.5)
    };
    assert_eq!(quad(r2.state.order[0]), (false, false), "最下層は左下象限");
    assert_eq!(quad(r2.state.order[1]), (true, false), "2層目は右下象限");
    assert_eq!(quad(r2.state.order[2]), (true, true), "3層目は右上象限");
    assert_eq!(quad(r2.state.order[3]), (false, true), "最上層は左上象限");

    // 全ての面が左下の1/4領域に写る
    for f in &faces {
        let q = r2.state.placements[&f.id].apply(DVec2::from(representative_point(&cp, f)));
        assert!(
            (-1e-9..0.5 + 1e-9).contains(&q.x) && (-1e-9..0.5 + 1e-9).contains(&q.y),
            "面 {} は左下1/4に畳まれる(実際: {q:?})",
            f.id
        );
    }
}

#[test]
fn pleat_with_up_and_down_makes_three_layers_in_order() {
    let mut cp = square_cp();
    let faces = extract_faces(&cp);
    let state = FlatState::initial(&cp, &faces);
    // 1本目: x=0.75 で右端を左へ(Up=谷)
    let r1 = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.75, 0.0], [0.75, 1.0]],
            keep_side_point: [0.5, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("1本目の折り");
    assert_eq!(edge_kind(&cp, r1.added_edges[0]), EdgeKind::Valley);
    assert_eq!(r1.step.drivers[0].target_angle_deg, -180.0);

    // 2本目: x=0.5 で左側を下へ(Down=山)。フラップは可動側に掛からないので対象は元の左面のみ
    let faces = extract_faces(&cp);
    let r2 = fold_through(
        &mut cp,
        &faces,
        &r1.state,
        &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            keep_side_point: [0.6, 0.5],
            target_layers: None,
            direction: FoldDirection::Down,
        },
    )
    .expect("2本目の折り");
    assert!(r2.warnings.is_empty(), "{:?}", r2.warnings);
    assert_eq!(r2.added_edges.len(), 1);
    assert_eq!(edge_kind(&cp, r2.added_edges[0]), EdgeKind::Mountain);
    assert_eq!(r2.step.drivers[0].target_angle_deg, 180.0);

    // 層3枚: 下から[動いた左側(x<0.5), 中央(0.5..0.75), 右端フラップ(x>0.75)]
    let new_faces = extract_faces(&cp);
    assert_eq!(new_faces.len(), 3);
    assert_eq!(r2.state.order.len(), 3);
    let rep_x = |id: FaceId| representative_point(&cp, face_by_id(&new_faces, id))[0];
    assert!(rep_x(r2.state.order[0]) < 0.5, "Downで動いた面が最下層");
    assert!(
        (0.5..0.75).contains(&rep_x(r2.state.order[1])),
        "中央は動かない"
    );
    assert!(rep_x(r2.state.order[2]) > 0.75, "先に折ったフラップが最上層");
}

#[test]
fn folding_only_top_layer_keeps_lower_layers_in_place() {
    let (mut cp, r2) = four_layer_fixture();
    let cp_before = cp.clone();
    let faces = extract_faces(&cp);
    let top = *r2.state.order.last().unwrap();

    // 山の閉じた角(0.5,0.5)側を残し、上1枚だけを対角線で折る
    let res = fold_through(
        &mut cp,
        &faces,
        &r2.state,
        &FoldThroughInput {
            line: [[0.0, 0.5], [0.5, 0.0]],
            keep_side_point: [0.4, 0.4],
            target_layers: Some(vec![top]),
            direction: FoldDirection::Up,
        },
    )
    .expect("上1枚だけの折り");
    assert!(res.warnings.is_empty(), "{:?}", res.warnings);

    let new_faces = extract_faces(&cp);
    assert_eq!(new_faces.len(), 5, "上1枚だけが2つに分かれる");
    assert_eq!(res.state.order.len(), 5);

    // 上1枚は裏返っている層なので、Upでも展開図上は山になる
    assert_eq!(res.added_edges.len(), 1);
    assert_eq!(edge_kind(&cp, res.added_edges[0]), EdgeKind::Mountain);
    assert_eq!(res.step.drivers.len(), 1);
    assert_eq!(res.step.drivers[0].target_angle_deg, 180.0);

    // 動いたのは上1枚由来の面だけで、他の層の配置は変わらない
    let refl = Isometry2::reflection(DVec2::new(0.0, 0.5), DVec2::new(0.5, 0.0));
    let mut moved: Vec<FaceId> = Vec::new();
    for nf in &new_faces {
        let r = representative_point(&cp, nf);
        let parent = faces
            .iter()
            .find(|f| point_in_face(&cp_before, f, r))
            .expect("親面が見つかる");
        let old = r2.state.placements[&parent.id];
        let now = res.state.placements[&nf.id];
        if now.approx_eq(&old, 1e-9) {
            continue;
        }
        assert_eq!(parent.id, top, "動いたのは対象に指定した上1枚の一部だけ");
        assert!(
            now.approx_eq(&refl.compose(&old), 1e-9),
            "動いた面は折り線の鏡映を重ねた配置になる"
        );
        moved.push(nf.id);
    }
    assert_eq!(moved.len(), 1, "動いた面はちょうど1つ");
    assert_eq!(*res.state.order.last().unwrap(), moved[0], "動いた面が最上層");
}

#[test]
fn invalid_inputs_error_without_touching_cp() {
    let mut cp = square_cp();
    let faces = extract_faces(&cp);
    let state = FlatState::initial(&cp, &faces);
    let before = cp.clone();

    // 紙全体が可動側に丸ごと乗り、折り線がどの面も横切らない
    let err = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[-0.5, 0.0], [-0.5, 1.0]],
            keep_side_point: [-1.0, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect_err("層を横切らない線はエラー");
    assert!(err.contains("横切"), "{err}");
    assert_eq!(cp, before, "失敗時はCPを一切変更しない");

    // 可動側に面が1つもない
    let err = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[2.0, 0.0], [2.0, 1.0]],
            keep_side_point: [0.5, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect_err("対象の層が無ければエラー");
    assert!(err.contains("対象"), "{err}");
    assert_eq!(cp, before);

    // 折り線の2点が一致
    let err = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.5, 0.5], [0.5, 0.5]],
            keep_side_point: [0.25, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect_err("退化した折り線はエラー");
    assert!(err.contains("一致"), "{err}");
    assert_eq!(cp, before);

    // 動かさない側を示す点が折り線上
    let err = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            keep_side_point: [0.5, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect_err("折り線上の点では側が決まらずエラー");
    assert!(err.contains("折り線上"), "{err}");
    assert_eq!(cp, before);
}

#[test]
fn layer_order_round_trips_and_is_deterministic() {
    let (cp_a, r_a) = four_layer_fixture();
    let (cp_b, r_b) = four_layer_fixture();

    // 決定性: 同じ操作列からビット一致の結果が得られる
    assert_eq!(cp_a, cp_b);
    assert_eq!(r_a.step, r_b.step);
    assert_eq!(r_a.state.order, r_b.state.order);

    // FoldStep.layer_orderをresolve_orderで解決すると同じ層順序に戻る
    let faces = extract_faces(&cp_a);
    let points = r_a.step.layer_order.as_ref().expect("layer_orderが設定される");
    assert_eq!(points.len(), 4);
    let (order, warnings) = FlatState::resolve_order(&cp_a, &faces, points);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(order, r_a.state.order, "代表点経由で往復一致");
}

#[test]
fn target_layer_without_movable_part_is_excluded_then_errors() {
    // x=0.75で右端を折った2層。フラップ(元の右端)は畳んだ平面の x∈[0.5,0.75] にしか無い
    let mut cp = square_cp();
    let faces = extract_faces(&cp);
    let state = FlatState::initial(&cp, &faces);
    let r1 = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.75, 0.0], [0.75, 1.0]],
            keep_side_point: [0.5, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("準備の折り");
    let faces = extract_faces(&cp);
    let flap = faces
        .iter()
        .find(|f| representative_point(&cp, f)[0] > 0.75)
        .expect("フラップ面がある")
        .id;
    let before = cp.clone();

    // x<0.25 が可動側。フラップはそこに掛からないので除外され、対象が空になりエラー
    let err = fold_through(
        &mut cp,
        &faces,
        &r1.state,
        &FoldThroughInput {
            line: [[0.25, 0.0], [0.25, 1.0]],
            keep_side_point: [0.4, 0.5],
            target_layers: Some(vec![flap]),
            direction: FoldDirection::Up,
        },
    )
    .expect_err("可動側に掛からない層だけの指定はエラー");
    assert!(err.contains("対象"), "{err}");
    assert_eq!(cp, before, "失敗時はCPを変更しない");
}

#[test]
fn warns_when_fold_tears_paper() {
    // 4層の上1枚は、畳んだ平面の上端(y=0.5)と右端(x=0.5)の折り目で下の層とつながっている。
    // その上1枚だけを縦線で折ると、折り線を跨がない接続辺が残り紙が裂ける指定になる。
    let (mut cp, r2) = four_layer_fixture();
    let faces = extract_faces(&cp);
    let top = *r2.state.order.last().unwrap();
    let res = fold_through(
        &mut cp,
        &faces,
        &r2.state,
        &FoldThroughInput {
            line: [[0.25, 0.0], [0.25, 0.5]],
            keep_side_point: [0.1, 0.25],
            target_layers: Some(vec![top]),
            direction: FoldDirection::Up,
        },
    )
    .expect("裂ける指定でも警告して続行する");
    assert!(
        res.warnings.iter().any(|w| w.contains("裂け")),
        "{:?}",
        res.warnings
    );
}

#[test]
fn warns_when_line_overlaps_existing_edge_and_keeps_its_kind() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.5, 0.3], [0.5, 0.7], EdgeKind::Aux);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1, "補助線は面を分けない");
    let state = FlatState::initial(&cp, &faces);

    let res = fold_through(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.5, 0.0], [0.5, 1.0]],
            keep_side_point: [0.25, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("既存辺との重なりは警告して続行する");
    assert!(
        res.warnings.iter().any(|w| w.contains("重な")),
        "{:?}",
        res.warnings
    );
    // 重なった区間はAuxのまま、それ以外の区間に谷線が入る
    assert_eq!(res.added_edges.len(), 2);
    for &eid in &res.added_edges {
        assert_eq!(edge_kind(&cp, eid), EdgeKind::Valley);
    }
    assert!(
        cp.edges.iter().any(|e| e.kind == EdgeKind::Aux),
        "既存の補助線の線種は維持される"
    );
    assert_eq!(res.step.drivers.len(), 2, "新しく引かれた区間だけにdriverが付く");
}
