//! FlatState(平坦状態)と面の内部代表点のテスト。

use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{Face, extract_faces, insert_segment};
use ori3_geometry::Isometry2;
use ori3_layers::flat_state::{FlatState, point_in_face, representative_point};
use ori3_model::{CreasePattern, Document, EdgeKind, FaceId, Paper};

/// 正方形(輪郭4辺のみ)のCPを作る。
fn square_cp() -> CreasePattern {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
    .cp
}

/// 中央の縦線 x=0.5 で2つに分けた正方形のCP。
fn half_folded_cp() -> CreasePattern {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Valley);
    cp
}

/// 代表点のx座標が閾値より小さい/大きい面を取り出す。
fn face_on_side(cp: &CreasePattern, faces: &[Face], left: bool) -> FaceId {
    let f = faces
        .iter()
        .find(|f| (representative_point(cp, f)[0] < 0.5) == left)
        .expect("該当する面が存在する");
    f.id
}

fn face_by_id(faces: &[Face], id: FaceId) -> &Face {
    faces.iter().find(|f| f.id == id).expect("面IDが存在する")
}

fn vertex_pos(cp: &CreasePattern, id: ori3_model::VertexId) -> DVec2 {
    DVec2::from(cp.vertices.iter().find(|v| v.id == id).unwrap().pos)
}

#[test]
fn initial_state_is_identity_and_id_order() {
    let cp = half_folded_cp();
    let faces = extract_faces(&cp);
    let state = FlatState::initial(&cp, &faces);

    assert_eq!(state.placements.len(), faces.len());
    for f in &faces {
        assert!(
            state.placements[&f.id].approx_eq(&Isometry2::identity(), 1e-12),
            "初期状態の配置は恒等変換"
        );
    }
    assert_eq!(state.order, vec![0, 1], "初期状態の層順序は面ID昇順");
}

#[test]
fn half_folded_square_is_mirror_pair_with_right_face_on_top() {
    let cp = half_folded_cp();
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 2);

    let left = face_on_side(&cp, &faces, true);
    let right = face_on_side(&cp, &faces, false);
    assert_ne!(left, right);

    // 右半面を中央の縦線 x=0.5 で折り返した平坦状態を手で構築する。
    let mirror = Isometry2::reflection(DVec2::new(0.5, 0.0), DVec2::new(0.5, 1.0));
    let placements: HashMap<FaceId, Isometry2> =
        HashMap::from([(left, Isometry2::identity()), (right, mirror)]);
    let state = FlatState {
        placements,
        order: vec![left, right],
    };

    // (a) 左半面は動かず、右半面の各頂点は x=0.5 に対する鏡映位置へ写る。
    for v in &face_by_id(&faces, left).vertices {
        let p = vertex_pos(&cp, *v);
        let q = state.placements[&left].apply(p);
        assert!((q - p).length() < 1e-12, "左半面は恒等変換で動かない");
    }
    for v in &face_by_id(&faces, right).vertices {
        let p = vertex_pos(&cp, *v);
        let q = state.placements[&right].apply(p);
        let expected = DVec2::new(1.0 - p.x, p.y);
        assert!(
            (q - expected).length() < 1e-12,
            "右半面の頂点 {p:?} は {expected:?} に写る(実際: {q:?})"
        );
    }

    // 2面の配置は鏡映関係(裏返りの有無が逆で、合成すると鏡映になる)。
    assert!(!state.placements[&left].mirrored);
    assert!(state.placements[&right].mirrored);
    let rel = state.placements[&left]
        .inverse()
        .compose(&state.placements[&right]);
    assert!(rel.approx_eq(&mirror, 1e-12), "2面の相対配置は中央線の鏡映");

    // (b) 層順序は[下面, 上面] = [左面, 右面]。
    assert_eq!(state.order, vec![left, right]);
    assert_eq!(*state.order.last().unwrap(), right, "折り返した右面が上");
}

#[test]
fn representative_point_is_inside_its_own_face_only() {
    let mut cp = square_cp();
    // 米字(対角線2本+十字)で8つの三角形を作る。
    insert_segment(&mut cp, [0.0, 0.0], [1.0, 1.0], EdgeKind::Valley);
    insert_segment(&mut cp, [1.0, 0.0], [0.0, 1.0], EdgeKind::Valley);
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 1.0], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.0, 0.5], [1.0, 0.5], EdgeKind::Mountain);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 8);

    for f in &faces {
        let p = representative_point(&cp, f);
        assert!(
            point_in_face(&cp, f, p),
            "代表点 {p:?} は面 {} の内部にある",
            f.id
        );
        // 他の面の境界上に乗っていない(=どの面の代表点かが一意に決まる)。
        let owners: Vec<FaceId> = faces
            .iter()
            .filter(|g| point_in_face(&cp, g, p))
            .map(|g| g.id)
            .collect();
        assert_eq!(owners, vec![f.id], "代表点 {p:?} が属する面は1つだけ");
    }
}

#[test]
fn representative_point_of_concave_slit_face_is_inside() {
    let mut cp = square_cp();
    // 下辺の中点から内部へ伸びるぶら下がり線(スリット)。面は凹んだ境界を持つ。
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 0.5], EdgeKind::Mountain);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1, "スリットは面を増やさない");

    let p = representative_point(&cp, &faces[0]);
    assert!(point_in_face(&cp, &faces[0], p), "代表点 {p:?} は面の内部");
    // スリットの線上には乗らない(層の識別が曖昧になるため)。
    assert!(
        (p[0] - 0.5).abs() > 1e-9 || p[1] > 0.5 + 1e-9,
        "代表点 {p:?} はスリット上にない"
    );
    // 同じ面からは常に同じ点(決定的)。
    assert_eq!(p, representative_point(&cp, &faces[0]));
    assert_eq!(p, representative_point(&cp, &extract_faces(&cp)[0]));
}

#[test]
fn representative_point_of_branched_slit_face_is_inside() {
    let mut cp = square_cp();
    // 下辺から伸びてY字に枝分かれするスリット(尖りを潰す処理が入れ子になる)。
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 0.4], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.5, 0.4], [0.35, 0.6], EdgeKind::Mountain);
    insert_segment(&mut cp, [0.5, 0.4], [0.65, 0.6], EdgeKind::Mountain);
    let faces = extract_faces(&cp);
    assert_eq!(faces.len(), 1, "枝分かれしたスリットも面を増やさない");

    let p = representative_point(&cp, &faces[0]);
    assert!(point_in_face(&cp, &faces[0], p), "代表点 {p:?} は面の内部");
    // どの切り込みの線上にも乗らない。
    for (a, b) in [
        ([0.5, 0.0], [0.5, 0.4]),
        ([0.5, 0.4], [0.35, 0.6]),
        ([0.5, 0.4], [0.65, 0.6]),
    ] {
        let d = ori3_geometry::dist_point_segment(DVec2::from(p), DVec2::from(a), DVec2::from(b));
        assert!(d > 1e-9, "代表点 {p:?} は切り込み {a:?}-{b:?} 上にない");
    }
}

#[test]
fn point_in_face_handles_boundary_and_outside() {
    let cp = half_folded_cp();
    let faces = extract_faces(&cp);
    let left = face_by_id(&faces, face_on_side(&cp, &faces, true));

    assert!(point_in_face(&cp, left, [0.25, 0.5]), "内部の点");
    assert!(point_in_face(&cp, left, [0.5, 0.5]), "境界(折り線)上の点");
    assert!(point_in_face(&cp, left, [0.0, 0.0]), "境界(角)の点");
    assert!(point_in_face(&cp, left, [0.0, 0.5]), "境界(左辺)上の点");
    assert!(
        point_in_face(&cp, left, [0.5 + 1e-12, 0.5]),
        "境界からEPS以内の点は内部扱い"
    );
    assert!(!point_in_face(&cp, left, [0.75, 0.5]), "隣の面の内部の点");
    assert!(!point_in_face(&cp, left, [-0.1, 0.5]), "紙の外の点");
}

#[test]
fn concave_slit_face_contains_points_on_both_sides_of_slit() {
    let mut cp = square_cp();
    insert_segment(&mut cp, [0.5, 0.0], [0.5, 0.5], EdgeKind::Mountain);
    let faces = extract_faces(&cp);
    let f = &faces[0];
    assert!(point_in_face(&cp, f, [0.25, 0.25]), "スリットの左側");
    assert!(point_in_face(&cp, f, [0.75, 0.25]), "スリットの右側");
    assert!(point_in_face(&cp, f, [0.5, 0.75]), "スリットの先端より上");
    assert!(point_in_face(&cp, f, [0.5, 0.25]), "スリット上(境界扱い)");
    assert!(!point_in_face(&cp, f, [1.25, 0.25]), "紙の外");
}

#[test]
fn layer_points_round_trip_without_warnings() {
    let cp = half_folded_cp();
    let faces = extract_faces(&cp);
    let left = face_on_side(&cp, &faces, true);
    let right = face_on_side(&cp, &faces, false);
    let state = FlatState {
        placements: HashMap::from([
            (left, Isometry2::identity()),
            (
                right,
                Isometry2::reflection(DVec2::new(0.5, 0.0), DVec2::new(0.5, 1.0)),
            ),
        ]),
        order: vec![left, right],
    };

    let points = state.to_layer_points(&cp, &faces);
    assert_eq!(points.len(), 2);
    let (order, warnings) = FlatState::resolve_order(&cp, &faces, &points);
    assert!(warnings.is_empty(), "警告なし: {warnings:?}");
    assert_eq!(order, state.order, "代表点経由で同じ層順序に戻る");
}

#[test]
fn resolve_order_keeps_meaning_after_reextraction() {
    // 縦線1本で2面。層順序は[右, 左](右が下)とする。
    let cp = half_folded_cp();
    let faces = extract_faces(&cp);
    let left = face_on_side(&cp, &faces, true);
    let right = face_on_side(&cp, &faces, false);
    let state = FlatState {
        placements: HashMap::from([
            (left, Isometry2::identity()),
            (right, Isometry2::identity()),
        ]),
        order: vec![right, left],
    };
    let points = state.to_layer_points(&cp, &faces);

    // 辺を1本足して面を分割し、面IDを再採番させる。
    let mut cp2 = cp.clone();
    insert_segment(&mut cp2, [0.0, 0.25], [1.0, 0.25], EdgeKind::Mountain);
    let faces2 = extract_faces(&cp2);
    assert_eq!(faces2.len(), 4, "面は4つに増える");

    let (order, warnings) = FlatState::resolve_order(&cp2, &faces2, &points);
    assert!(warnings.is_empty(), "代表点は全て解決できる: {warnings:?}");

    // 全ての面がちょうど1回ずつ含まれる。
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![0, 1, 2, 3]);

    // 先頭2層は元の代表点が属する面(=意味的な順序が維持されている)。
    assert!(
        point_in_face(&cp2, face_by_id(&faces2, order[0]), points[0]),
        "1層目は元の下面(右)の代表点を含む面"
    );
    assert!(
        point_in_face(&cp2, face_by_id(&faces2, order[1]), points[1]),
        "2層目は元の上面(左)の代表点を含む面"
    );
    // 元の右面(x>0.5)側が下、左面(x<0.5)側が上のまま。
    assert!(representative_point(&cp2, face_by_id(&faces2, order[0]))[0] > 0.5);
    assert!(representative_point(&cp2, face_by_id(&faces2, order[1]))[0] < 0.5);

    // 分割で増えた面は解決されず、元の相対順を保って末尾側に補われる。
    let appended = &order[2..];
    let mut expected: Vec<FaceId> = faces2
        .iter()
        .map(|f| f.id)
        .filter(|id| !order[..2].contains(id))
        .collect();
    expected.sort_unstable();
    assert_eq!(
        appended.to_vec(),
        expected,
        "補われた面は面ID順(元の相対順)"
    );
}

#[test]
fn resolve_order_warns_on_unresolvable_point() {
    let cp = half_folded_cp();
    let faces = extract_faces(&cp);
    let left = face_on_side(&cp, &faces, true);
    let right = face_on_side(&cp, &faces, false);

    // 1点目は紙の外(解決不能)、2点目は右面の内部。
    let points = [
        [5.0, 5.0],
        representative_point(&cp, face_by_id(&faces, right)),
    ];
    let (order, warnings) = FlatState::resolve_order(&cp, &faces, &points);

    assert_eq!(warnings.len(), 1, "解決できない点は1件の警告になる");
    assert!(
        warnings[0].contains("代表点"),
        "警告文は日本語で内容を説明する: {}",
        warnings[0]
    );
    // 解決不能な層は飛ばし、残りの面は末尾側に補われるので全面が含まれる。
    assert_eq!(order, vec![right, left]);
}

#[test]
fn resolve_order_skips_duplicated_points_and_keeps_all_faces() {
    let cp = half_folded_cp();
    let faces = extract_faces(&cp);
    let left = face_on_side(&cp, &faces, true);
    let right = face_on_side(&cp, &faces, false);
    let p = representative_point(&cp, face_by_id(&faces, right));

    let (order, warnings) = FlatState::resolve_order(&cp, &faces, &[p, p]);
    assert_eq!(warnings.len(), 1, "同じ面を指す2点目は警告になる");
    assert_eq!(
        order,
        vec![right, left],
        "全ての面がちょうど1回ずつ含まれる"
    );
}

#[test]
fn resolve_order_with_no_points_falls_back_to_face_order() {
    let cp = half_folded_cp();
    let faces = extract_faces(&cp);
    let (order, warnings) = FlatState::resolve_order(&cp, &faces, &[]);
    assert!(warnings.is_empty());
    assert_eq!(order, vec![0, 1], "点が無ければ面ID順");
}
