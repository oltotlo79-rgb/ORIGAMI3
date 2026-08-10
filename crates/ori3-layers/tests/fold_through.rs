//! fold_through(折り操作プリミティブ)とresolve_driver_edgesのテスト。

use glam::DVec2;
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_geometry::Isometry2;
use ori3_layers::flat_state::{FlatState, point_in_face, representative_point};
use ori3_layers::fold_through::{
    FOLD_PENETRATION_WARNING, FoldDirection, FoldThroughInput, FoldThroughResult, fold_through,
    fold_through_with_additional_crease, propose_fold_through, resolve_driver_edges,
};
use ori3_layers::replay;
use ori3_model::{CreasePattern, Document, Driver, EdgeKind, FaceId, Paper};
use ori3_rigid::layer_order_conflicts;

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
    cp.edges
        .iter()
        .find(|e| e.id == id)
        .expect("辺が存在する")
        .kind
}

fn vertex_pos(cp: &CreasePattern, id: u32) -> DVec2 {
    DVec2::from(cp.vertices.iter().find(|v| v.id == id).unwrap().pos)
}

/// 辺の中点(CP座標)。
fn edge_midpoint(cp: &CreasePattern, id: u32) -> DVec2 {
    let e = cp.edges.iter().find(|e| e.id == id).expect("辺が存在する");
    (vertex_pos(cp, e.v0) + vertex_pos(cp, e.v1)) * 0.5
}

/// 正方形を x=0.5(Up)→ y=0.5(Up)の2回で4層に畳む。
/// 戻り値は(CP, 1回目の結果, 2回目の結果)。
///
/// 2回目の折り線は「1回目の結果の座標系」で書く。fold_throughが返す状態は
/// 表示と同じ「根面(最小面ID)が恒等」の座標系なので、1回目で根面が動く側に
/// あった場合は畳んだ紙の見える位置が動く(ここでは x∈[0.5,1.0] に見える)。
/// 画面から折り線を引く操作と同じ流れになる。
fn four_layer_fixture() -> (CreasePattern, FoldThroughResult, FoldThroughResult) {
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
            line: [[0.5, 0.5], [1.0, 0.5]],
            keep_side_point: [0.75, 0.25],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("2回目の折り");
    (cp, r1, r2)
}

/// 左端を折って外周縁x=0.5を上層へ置き、その下の右側フラップをx=0.7で折る。
fn single_collision_fixture() -> (CreasePattern, FoldThroughResult, FaceId, FoldThroughInput) {
    let mut cp = square_cp();
    let faces = extract_faces(&cp);
    let initial = FlatState::initial(&cp, &faces);
    let first = fold_through(
        &mut cp,
        &faces,
        &initial,
        &FoldThroughInput {
            line: [[0.25, 0.0], [0.25, 1.0]],
            keep_side_point: [0.5, 0.5],
            target_layers: None,
            direction: FoldDirection::Up,
        },
    )
    .expect("左端を右へ折る");
    let faces = extract_faces(&cp);
    let base = faces
        .iter()
        .find(|face| representative_point(&cp, face)[0] > 0.25)
        .expect("右側の基層")
        .id;
    assert_eq!(first.state.order[0], base, "基層の上に左端が重なる");
    let input = FoldThroughInput {
        line: [[0.7, 0.0], [0.7, 1.0]],
        keep_side_point: [0.6, 0.5],
        target_layers: None,
        direction: FoldDirection::Up,
    };
    (cp, first, base, input)
}

#[test]
fn proposes_reflected_edge_and_guided_fold_removes_the_same_collision() {
    let (cp, first, base, input) = single_collision_fixture();
    let faces = extract_faces(&cp);
    let proposal = propose_fold_through(&cp, &faces, &first.state, &input)
        .expect("提案の幾何計算")
        .expect("単一縁なので提案できる");

    // 障害縁x=.5を主折り線x=.7で反転したx=.9が誘導折り目になる。
    assert!(
        proposal
            .folded_line
            .iter()
            .all(|point| (point[0] - 0.9).abs() < 1e-9)
    );
    assert_eq!(proposal.crease_segments.len(), 1);
    assert!(proposal.message.contains("指定した場所以外"));
    assert!(proposal.message.contains("折り目"));

    let mut rejected_cp = cp.clone();
    let rejected =
        fold_through_with_additional_crease(&mut rejected_cp, &faces, &first.state, &input, false)
            .expect("提案を断っても通常どおり折れる");
    assert!(
        rejected
            .warnings
            .iter()
            .any(|warning| warning == FOLD_PENETRATION_WARNING),
        "提案を断った通常折りには貫通警告が残る: {:?}",
        rejected.warnings
    );

    let mut accepted_cp = cp;
    let accepted =
        fold_through_with_additional_crease(&mut accepted_cp, &faces, &first.state, &input, true)
            .expect("提案を承諾すると巻き込み折りになる");
    assert!(
        accepted
            .warnings
            .iter()
            .all(|warning| warning != FOLD_PENETRATION_WARNING && !warning.contains("裂け")),
        "承諾後は貫通・裂け警告が消える: {:?}",
        accepted.warnings
    );
    assert!(
        accepted_cp.edges.iter().any(|edge| {
            if !matches!(edge.kind, EdgeKind::Mountain | EdgeKind::Valley) {
                return false;
            }
            let midpoint = edge_midpoint(&accepted_cp, edge.id);
            (midpoint.x - 0.9).abs() < 1e-9
        }),
        "展開図へx=.9の誘導折り目が追加される"
    );

    // 同じ「鏡映後フラップが障害縁を横断する」述語で見ると、遠側が巻き戻されて
    // 障害の内側へ入らない。元の基層から分かれた各面の表示位置を直接確認する。
    let accepted_faces = extract_faces(&accepted_cp);
    for face in accepted_faces.iter().filter(|face| {
        let representative = representative_point(&accepted_cp, face);
        representative[0] > 0.25
    }) {
        let representative = representative_point(&accepted_cp, face);
        if representative[0] <= 0.7 {
            continue; // 動かなかった基層部分
        }
        let placement = accepted.state.placements[&face.id];
        let polygon: Vec<DVec2> = face
            .vertices
            .iter()
            .map(|vertex| placement.apply(vertex_pos(&accepted_cp, *vertex)))
            .collect();
        let (min_x, max_x) = polygon
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), point| {
                (lo.min(point.x), hi.max(point.x))
            });
        assert!(
            min_x >= 0.5 - 1e-9 || max_x <= 0.5 + 1e-9,
            "承諾後のフラップ面は障害縁x=.5を横断しない: {polygon:?}"
        );
    }
    let after_proposal = propose_fold_through(
        &accepted_cp,
        &accepted_faces,
        &accepted.state,
        &FoldThroughInput {
            target_layers: Some(vec![base]),
            ..input
        },
    );
    // 面ID体系が変わるため同じ対象IDは無効化されてもよいが、少なくとも同じ衝突を
    // 再提案することはない。
    assert!(
        after_proposal.is_err() || after_proposal.unwrap().is_none(),
        "適用済みの衝突を再提案しない"
    );

    // 保存されるCP+手順から折り直しても平坦になり、一般の山谷/層順検査でも
    // 貫通矛盾が残らない。
    let mut document = Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    });
    document.cp = accepted_cp;
    let mut first_step = first.step.clone();
    first_step.id = 0;
    let mut accepted_step = accepted.step.clone();
    accepted_step.id = 1;
    document.sequence = vec![first_step, accepted_step];
    let replayed = replay(&document, 2, 1.0);
    assert!(
        replayed
            .warnings
            .iter()
            .all(|warning| warning != FOLD_PENETRATION_WARNING),
        "再生後も貫通警告なし: {:?}",
        replayed.warnings
    );
    assert!(
        !layer_order_conflicts(&document.cp, &accepted_faces, &replayed.frame),
        "提案適用後の山谷と層順は矛盾しない"
    );
}

#[test]
fn non_parallel_collision_falls_back_to_warning_without_a_proposal() {
    let (cp, first, _base, mut input) = single_collision_fixture();
    // 障害縁はx=.5の鉛直線。主折り線をわずかに傾けると反射後フラップは同じ縁へ
    // 入るが、guideと主折り線が交差するため単一の「遠側」を安全に定義できない。
    input.line = [[0.7, 0.0], [0.8, 1.0]];
    input.keep_side_point = [0.6, 0.5];
    let faces = extract_faces(&cp);
    assert!(
        propose_fold_through(&cp, &faces, &first.state, &input)
            .expect("衝突解析")
            .is_none(),
        "非平行な複雑ケースは提案しない"
    );

    let mut folded_cp = cp;
    let result =
        fold_through_with_additional_crease(&mut folded_cp, &faces, &first.state, &input, false)
            .expect("複雑ケースも操作は止めない");
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning == FOLD_PENETRATION_WARNING),
        "提案できなくても貫通警告へフォールバックする: {:?}",
        result.warnings
    );
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

    // 層2枚。表示座標系は根面(最小面ID=動かした右半面)を固定するので、画面では
    // 紙全体を裏側から見た姿勢になり、動かさなかった左半面が上に重なって見える。
    let new_faces = extract_faces(&cp);
    assert_eq!(new_faces.len(), 2);
    assert_eq!(res.state.order.len(), 2);
    let top = *res.state.order.last().unwrap();
    let r = representative_point(&cp, face_by_id(&new_faces, top));
    assert!(r[0] < 0.5, "上に見える層は元の左半面(代表点 {r:?})");
    let q = res.state.placements[&top].apply(DVec2::from(r));
    assert!(
        (q.x - (1.0 - r[0])).abs() < 1e-9 && (q.y - r[1]).abs() < 1e-9,
        "重なりは x=0.5 の鏡映で表される(実際: {q:?})"
    );
    assert!(
        res.state.placements[&top].mirrored,
        "根面と重ね合わない側は裏返る"
    );
    let bottom = res.state.order[0];
    assert!(
        res.state.placements[&bottom].approx_eq(&Isometry2::identity(), 1e-12),
        "根面(最小面ID)は恒等変換にそろえられる"
    );

    // FoldStep: driverは折り線のCP線分1本(谷=-180)で、追加した辺へ解決できる
    assert_eq!(res.step.kind, ori3_model::TechniqueKind::Simple);
    assert_eq!(res.step.drivers.len(), 1);
    let d = &res.step.drivers[0];
    assert_eq!(d.target_angle_deg, -180.0);
    assert!(
        (d.a[0] - 0.5).abs() < 1e-9 && (d.b[0] - 0.5).abs() < 1e-9,
        "driver線分はCP座標の折り線区間: {d:?}"
    );
    assert_eq!(resolve_driver_edges(&cp, d), res.added_edges);
    assert_eq!(res.step.layer_order.as_ref().map(Vec::len), Some(2));
}

#[test]
fn folds_along_an_existing_diagonal_crease_without_adding_edges() {
    let mut cp = square_cp();
    let existing = insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    assert_eq!(existing.len(), 1, "対角線の既存折り筋は1辺");
    let before = cp.clone();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 2, "既存折り筋で正方形は2面に分かれる");
    let state = FlatState::initial(&cp, &faces);

    let res = fold_through_with_additional_crease(
        &mut cp,
        &faces,
        &state,
        &FoldThroughInput {
            line: [[0.0, 0.0], [1.0, 1.0]],
            keep_side_point: [0.25, 0.75],
            target_layers: None,
            direction: FoldDirection::Up,
        },
        false,
    )
    .expect("既存の対角折り筋に沿って折れる");

    assert_eq!(cp, before, "既存折り筋の再利用ではCPを変更しない");
    assert!(res.added_edges.is_empty(), "折り筋を重複追加しない");
    assert_eq!(res.state.order.len(), 2, "2面の層順を保持する");

    let moved = faces
        .iter()
        .find(|face| {
            let p = representative_point(&cp, face);
            p[0] > p[1]
        })
        .expect("対角線の可動側の面")
        .id;
    let fixed = faces
        .iter()
        .find(|face| {
            let p = representative_point(&cp, face);
            p[0] < p[1]
        })
        .expect("対角線の固定側の面")
        .id;
    assert_ne!(
        res.state.placements[&moved].mirrored, res.state.placements[&fixed].mirrored,
        "可動側だけが折り返される"
    );
    assert_eq!(
        res.state.order.last().copied(),
        Some(if res.state.placements[&moved].mirrored {
            moved
        } else {
            fixed
        }),
        "Up折りの層が表示座標系でも上側に積まれる"
    );

    assert_eq!(
        res.step.drivers.len(),
        1,
        "既存折り筋を1本のdriverとして記録する"
    );
    assert_eq!(res.step.drivers[0].target_angle_deg, -180.0);
    assert_eq!(
        resolve_driver_edges(&cp, &res.step.drivers[0]),
        existing,
        "FoldStepのdriverが既存折り筋へ解決される"
    );
    let points = res
        .step
        .layer_order
        .as_ref()
        .expect("層順をFoldStepへ記録する");
    assert_eq!(points.len(), 2);
    let (recorded_order, warnings) = FlatState::resolve_order(&cp, &faces, points);
    assert!(warnings.is_empty(), "層順の再解決: {warnings:?}");
    assert_eq!(
        recorded_order, res.state.order,
        "FoldStepから既存折り筋で反転した層順を再現できる"
    );
}

#[test]
fn second_fold_pulls_back_two_lines_with_opposite_kinds() {
    let (cp, _r1, r2) = four_layer_fixture();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 4, "面は4つに分かれる");
    assert_eq!(r2.state.order.len(), 4, "層は4枚");
    assert!(r2.warnings.is_empty(), "{:?}", r2.warnings);

    // 2枚の層それぞれへの引き戻しで折り線は2本追加される
    assert_eq!(r2.added_edges.len(), 2);
    assert_eq!(r2.step.drivers.len(), 2);
    for &eid in &r2.added_edges {
        let mid = edge_midpoint(&cp, eid);
        // 山谷は「画面で見えている面の向き」で決まる。1回目で根面(最小面ID)が
        // CP右半分になったため、画面ではCP右半分が表向き(谷)・左半分が裏返り(山)。
        let expected = if mid.x < 0.5 {
            EdgeKind::Mountain
        } else {
            EdgeKind::Valley
        };
        assert_eq!(
            edge_kind(&cp, eid),
            expected,
            "mirroredな層では山谷が反転する(辺 {eid}, 中点 {mid:?})"
        );
        // 追加辺ごとに、それを解決するDriverLineが正しい角度で存在する
        let d = r2
            .step
            .drivers
            .iter()
            .find(|d| resolve_driver_edges(&cp, d).contains(&eid))
            .expect("追加辺を駆動するDriverLineがある");
        let expected_angle = if expected == EdgeKind::Valley {
            -180.0
        } else {
            180.0
        };
        assert_eq!(d.target_angle_deg, expected_angle);
    }

    // 層順序(画面で見える下→上): 右上→左上→左下→右下 の象限。
    // 紙そのものの重なりは 左下→右下→右上→左上 だが、根面(最小面ID)を固定する
    // 表示座標系では紙を裏側から見た姿勢になるため、上下が入れ替わって見える。
    let quad = |id: FaceId| {
        let r = representative_point(&cp, face_by_id(&faces, id));
        (r[0] > 0.5, r[1] > 0.5)
    };
    assert_eq!(quad(r2.state.order[0]), (true, true), "最下層は右上象限");
    assert_eq!(quad(r2.state.order[1]), (false, true), "2層目は左上象限");
    assert_eq!(quad(r2.state.order[2]), (false, false), "3層目は左下象限");
    assert_eq!(quad(r2.state.order[3]), (true, false), "最上層は右下象限");

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

    // 2本目: 折った紙の1/3の位置で端側を下へ(Down=山)。フラップは可動側に
    // 掛からないので対象は元の左面のみ。
    // 1本目で根面(最小面ID)がフラップ側になったため、畳んだ紙は x∈[0.75,1.5] に
    // 見えている(表示座標系=根面が恒等)。その座標で折り線を引く。
    let faces = extract_faces(&cp);
    let r2 = fold_through(
        &mut cp,
        &faces,
        &r1.state,
        &FoldThroughInput {
            line: [[1.0, 0.0], [1.0, 1.0]],
            keep_side_point: [0.9, 0.5],
            target_layers: None,
            direction: FoldDirection::Down,
        },
    )
    .expect("2本目の折り");
    assert!(r2.warnings.is_empty(), "{:?}", r2.warnings);
    assert_eq!(r2.added_edges.len(), 1);
    // 対象の層は画面では裏返って見えている(1本目で根面がフラップ側になったため)
    // ので、向こうへ折る指定でも展開図には谷が入る
    assert_eq!(edge_kind(&cp, r2.added_edges[0]), EdgeKind::Valley);
    assert_eq!(r2.step.drivers[0].target_angle_deg, -180.0);

    // 層3枚。画面で見える下→上は[動いた側(x<0.5), 右端フラップ(x>0.75), 中央(0.5..0.75)]。
    // 1本目で根面がフラップ側になったため、紙そのものでは中央の上に乗っている
    // フラップが、画面では中央の下に見える。
    let new_faces = extract_faces(&cp);
    assert_eq!(new_faces.len(), 3);
    assert_eq!(r2.state.order.len(), 3);
    let rep_x = |id: FaceId| representative_point(&cp, face_by_id(&new_faces, id))[0];
    assert!(rep_x(r2.state.order[0]) < 0.5, "Downで動いた面が最下層");
    assert!(
        rep_x(r2.state.order[1]) > 0.75,
        "先に折ったフラップは中央より下に見える"
    );
    assert!(
        (0.5..0.75).contains(&rep_x(r2.state.order[2])),
        "中央が最上層に見える"
    );
}

#[test]
fn folding_only_top_layer_keeps_lower_layers_in_place() {
    let (mut cp, _r1, r2) = four_layer_fixture();
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
    assert_eq!(
        resolve_driver_edges(&cp, &res.step.drivers[0]),
        res.added_edges,
        "driver線分は追加した辺へ解決できる"
    );

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
    assert_eq!(
        *res.state.order.last().unwrap(),
        moved[0],
        "動いた面が最上層"
    );
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
    let (cp_a, _r1a, r_a) = four_layer_fixture();
    let (cp_b, _r1b, r_b) = four_layer_fixture();

    // 決定性: 同じ操作列からビット一致の結果が得られる
    assert_eq!(cp_a, cp_b);
    assert_eq!(r_a.step, r_b.step);
    assert_eq!(r_a.state.order, r_b.state.order);

    // FoldStep.layer_orderをresolve_orderで解決すると同じ層順序に戻る
    let faces = extract_faces(&cp_a);
    let points = r_a
        .step
        .layer_order
        .as_ref()
        .expect("layer_orderが設定される");
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
    let (mut cp, _r1, r2) = four_layer_fixture();
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
fn overlapping_aux_line_is_promoted_and_fold_stays_consistent() {
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
    .expect("補助線と重なっていても折れる");
    assert!(
        res.warnings.iter().any(|w| w.contains("補助線")),
        "{:?}",
        res.warnings
    );

    // 補助線は折り線(谷)へ昇格し、面はきちんと2つに分かれる
    assert!(
        cp.edges.iter().all(|e| e.kind != EdgeKind::Aux),
        "重なった補助線は補助線のまま残らない"
    );
    let new_faces = extract_faces(&cp);
    assert_eq!(new_faces.len(), 2, "面が折り線で分割される");
    assert_eq!(res.state.order.len(), 2);
    // 追加辺: 新規の谷2本+昇格した断片1本
    assert_eq!(res.added_edges.len(), 3);
    for &eid in &res.added_edges {
        assert_eq!(edge_kind(&cp, eid), EdgeKind::Valley);
    }

    // 折った後の状態は普通の半分折りと同じ: 根面(右半面)が恒等で、左半面が鏡映で上
    let bottom = res.state.order[0];
    let top = *res.state.order.last().unwrap();
    assert!(
        representative_point(&cp, face_by_id(&new_faces, top))[0] < 0.5,
        "上に見える層は元の左半面"
    );
    assert!(
        res.state.placements[&bottom].approx_eq(&Isometry2::identity(), 1e-12),
        "根面は恒等変換"
    );
    assert!(
        res.state.placements[&top].approx_eq(
            &Isometry2::reflection(DVec2::new(0.5, 0.0), DVec2::new(0.5, 1.0)),
            1e-9
        ),
        "重なりは折り線の鏡映で表される"
    );

    // DriverLineは折り線区間1本(谷=-180)で、昇格した断片も含む3辺を駆動する
    assert_eq!(res.step.drivers.len(), 1);
    assert_eq!(res.step.drivers[0].target_angle_deg, -180.0);
    let resolved = resolve_driver_edges(&cp, &res.step.drivers[0]);
    assert_eq!(resolved.len(), 3, "昇格断片を含む全ての断片が駆動対象");
}

#[test]
fn warns_when_existing_crease_on_the_line_has_the_opposite_kind() {
    // 折り線の一部に逆向きの折り目(山)が既にある状態で谷折りすると、展開図は
    // 山[0..0.4]+谷[0.4..1] になる。平坦(±180°)では折り上がるが折り途中の形が
    // 求まらないため、警告で知らせる必要がある。
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 0.4], EdgeKind::Mountain);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1, "切り込み状の折り目は面を分けない");
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
    .expect("逆向きの折り目があっても折り自体は続行する");
    assert!(
        res.warnings.iter().any(|w| w.contains("反対向き")),
        "山谷の食い違いを警告する: {:?}",
        res.warnings
    );

    // 既存の山はそのまま(上書きしない)、残りの区間に谷が入る
    assert!(
        cp.edges
            .iter()
            .any(|e| e.kind == EdgeKind::Mountain && edge_midpoint(&cp, e.id).y < 0.4),
        "既存の山は線種を変えられない"
    );
    // 山の区間・谷の区間それぞれにDriverLineが付く
    assert_eq!(res.step.drivers.len(), 2);
    assert!(res.step.drivers.iter().any(|d| d.target_angle_deg == 180.0));
    assert!(
        res.step
            .drivers
            .iter()
            .any(|d| d.target_angle_deg == -180.0)
    );
}

#[test]
fn driver_lines_survive_edge_splits_and_replay_matches_flat_state() {
    // レビューシナリオ: ステップ1の折り線(x=0.5)は2回目の折りの引き戻し挿入で
    // 中点(0.5,0.5)で2辺に分割され、辺IDが変わる。線分参照なら両断片へ解決できる。
    let (cp, r1, r2) = four_layer_fixture();
    assert_eq!(r1.step.drivers.len(), 1);
    let split = resolve_driver_edges(&cp, &r1.step.drivers[0]);
    assert_eq!(split.len(), 2, "分割後の両断片が解決される");
    for &eid in &split {
        assert_eq!(edge_kind(&cp, eid), EdgeKind::Valley);
    }

    // 全ステップのDriverLineを現在の辺へ解決し、solveに与える
    let faces = extract_faces(&cp);
    let mut drivers: Vec<Driver> = Vec::new();
    for dl in r1.step.drivers.iter().chain(r2.step.drivers.iter()) {
        for eid in resolve_driver_edges(&cp, dl) {
            drivers.push(Driver {
                hinge: eid,
                target_angle_deg: dl.target_angle_deg,
            });
        }
    }
    assert_eq!(drivers.len(), 4, "ステップ1の断片2本+ステップ2の2本");
    let result = ori3_rigid::solve(&cp, &faces, &drivers, None);
    assert!(result.converged, "±180°で収束する");

    // 畳んだ位置がFlatStateの配置と一致する(solveが固定する根の面との相対で比較)
    let root = faces[0].id;
    let align = r2.state.placements[&root].inverse();
    for f3 in &result.frame.faces {
        let face = face_by_id(&faces, f3.face);
        let pl = align.compose(&r2.state.placements[&f3.face]);
        assert_eq!(face.vertices.len(), f3.polygon.len());
        for (vid, p3) in face.vertices.iter().zip(&f3.polygon) {
            let q = pl.apply(vertex_pos(&cp, *vid));
            assert!(
                (p3[0] - q.x).abs() < 1e-6 && (p3[1] - q.y).abs() < 1e-6 && p3[2].abs() < 1e-6,
                "面 {} の頂点 {vid}: solve={p3:?} 期待=({}, {}, 0)",
                f3.face,
                q.x,
                q.y
            );
        }
    }
}
