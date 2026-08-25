//! Task 4-6: M4受け入れテスト — 伝承のカエル(完成形まで)。
//!
//! **アプリが提供する折り操作の列だけ**でカエルを完成させ、回帰テストとして固定する。
//! 手作業の展開図編集は一切せず、次の操作だけを使う:
//!
//! - 重ね折り `fold_through`(半分に折る)
//! - 開いてつぶす折り `squash`(予備基本形への組み替えと、4つの袋を開く工程)
//! - 花弁折り `petal`(4つの袋を同時に→カエルの基本形)
//! - 中割り折り `inside_reverse`(足4本を体の外へ出す)
//! - 段折り `pleat`(体の根元)
//!
//! # 折り順
//!
//! 1. 正方形を半分に2回折り、開いてつぶす折り2回で**予備基本形**(`acceptance_crane.rs`
//!    と同じ手順)
//! 2. 予備基本形の**4つの袋を開いてつぶす**。袋の背(紙の中心から辺の中点へ向かう
//!    折り目)が紙の隅の向きへ倒れ、畳んだ形は90°の正方形から45°のたこ形になる
//!    (紙の1/4がそれぞれ3面に分かれて12面・どの向きにも8層)
//! 3. **花弁折り1回**でカエルの基本形。紙の4隅が紙の中心と同じ1点(先端)へ集まり、
//!    4辺の中点は先端から (√2-1)/2、根元は先端から √2/4 になる。8本の先が
//!    1つの先端へ集まった、無駄のない(3つの制約が全て等号で成り立つ)基本形
//! 4. **足4本を中割り折り**。紙の隅から出た先を、中心線と45°をなす折り線で
//!    体の外(基本形の開き角±22.5°の外)へ出す。前足2本は先端から0.10、
//!    後ろ足2本は0.20のところで折る
//! 5. **体の根元を段折り**して完成(140面)
//!
//! # 花弁折りが1回である理由
//!
//! 手順書では「花弁折り4回」だが、**畳んだ形の上では1回の動きと同じ**になる。
//! 4つの袋は紙の中心から辺の中点へ向かう折り目でつながっていて、その折り目は
//! たこ形の中心線にそのまま乗っている。花弁折りのちょうつがい(先端から√2/4)は
//! この折り目(先端から√2/2−…、中心から0.5)の途中を横切るので、袋を1つだけ
//! 選ぶと必ずそこで紙が裂ける。4つの袋は畳み平面でぴったり重なっているので、
//! 全層を選んだ1回の花弁折りが4回分の動きをそのまま表す。
//!
//! ただし**持ち上げた紙の置き場所は袋ごとに別**で、`petal` は袋ごとに
//! 「その袋のいちばん外側の層の隣」へ置き直す(`LayerTurn::Beside`)。
//! 重なり全体の外側へまとめて回すと4つの袋の紙が入り混じり、足1本をつまんで
//! 中割り折りできなくなる([`each_leg_is_a_bundle_of_neighbouring_layers`])。

use std::collections::HashMap;

use glam::DVec2;
use ori3_cp::{extract_faces, Face};
use ori3_layers::fold_through::{fold_through, FoldDirection, FoldThroughInput};
use ori3_layers::techniques::TechniqueInput;
use ori3_layers::{
    flat_state_at, inside_reverse, petal, pleat, replay, squash, FlatState, FoldThroughResult,
};
use ori3_model::{
    CreasePattern, Document, EdgeKind, Face3D, FaceId, Frame3D, Paper, TechniqueKind,
};
use ori3_rigid::max_seam_gap;

/// 畳んだたこ形の半分の開き角(22.5°)の正接から決まる、袋を開いた後の外形の値。
const HALF: f64 = std::f64::consts::FRAC_PI_8;

type Technique = fn(
    &mut CreasePattern,
    &[Face],
    &FlatState,
    &TechniqueInput,
) -> Result<FoldThroughResult, String>;

/// 単位正方形の紙。
fn square_doc() -> Document {
    Document::new(Paper {
        width_mm: 100.0,
        height_mm: 100.0,
    })
}

/// store.rs の SeqOp::FoldThrough と同じ手順で1手折る。
fn fold(doc: &mut Document, line: [[f64; 2]; 2], keep: [f64; 2], direction: FoldDirection) {
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
            target_layers: None,
            direction,
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
}

/// store.rs の SeqOp::Technique と同じ手順で技法を1回適用する。
/// 戻り値は技法が返した平坦状態(再生一致の検証に使う)。
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

/// 現在の畳んだ状態(面・配置・層順序)。警告が出たら失敗させる。
fn state_of(doc: &Document) -> (Vec<Face>, FlatState) {
    let faces = extract_faces(&doc.cp);
    let (state, warnings) = flat_state_at(doc, &faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "再生の警告: {warnings:?}");
    (faces, state)
}

/// 展開図の頂点の位置(頂点ID→座標)。
fn vertex_pos(cp: &CreasePattern) -> HashMap<u32, DVec2> {
    cp.vertices
        .iter()
        .map(|v| (v.id, DVec2::from(v.pos)))
        .collect()
}

fn explicit_flat_frame(document: &Document, faces: &[Face], state: &FlatState) -> Frame3D {
    let positions = vertex_pos(&document.cp);
    Frame3D {
        faces: faces
            .iter()
            .map(|face| {
                let rank = state
                    .order
                    .iter()
                    .position(|id| *id == face.id)
                    .expect("全ての面が層順序にある");
                Face3D {
                    face: face.id,
                    polygon: face
                        .vertices
                        .iter()
                        .map(|vertex| {
                            let point = state.placements[&face.id].apply(positions[vertex]);
                            [point.x, point.y, 0.0]
                        })
                        .collect(),
                    layer: u32::try_from(rank).expect("層順序はu32に収まる"),
                    surface_rank: u32::try_from(rank).expect("層順序はu32に収まる"),
                    mirrored: state.placements[&face.id].mirrored,
                }
            })
            .collect(),
        warnings: Vec::new(),
    }
}

/// 面が畳み平面で占める多角形。
fn plane_poly(cp: &CreasePattern, f: &Face, state: &FlatState) -> Vec<DVec2> {
    let pos = vertex_pos(cp);
    let pl = state.placements[&f.id];
    f.vertices
        .iter()
        .filter_map(|v| pos.get(v).copied())
        .map(|p| pl.apply(p))
        .collect()
}

/// 展開図の点 `cp` が畳み平面のどこへ来たか(重なった同じ位置は1つにまとめる)。
fn folded_to(doc: &Document, cp: [f64; 2]) -> Vec<DVec2> {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    let target = DVec2::from(cp);
    let mut out: Vec<DVec2> = Vec::new();
    for f in &faces {
        for v in f.vertices.iter().filter_map(|v| pos.get(v)) {
            if (*v - target).length() < 1e-9 {
                let q = state.placements[&f.id].apply(*v);
                if !out.iter().any(|p| (*p - q).length() < 1e-6) {
                    out.push(q);
                }
            }
        }
    }
    out
}

/// 展開図の点1つが畳み平面の1点へ来ていることを確かめ、その点を返す。
fn only(doc: &Document, cp: [f64; 2], label: &str) -> DVec2 {
    let ps = folded_to(doc, cp);
    assert_eq!(ps.len(), 1, "{label}: 1点に重なる(実際 {ps:?})");
    ps[0]
}

/// 多角形の内部(境界を含む)に点があるか。
fn inside_polygon(poly: &[DVec2], p: DVec2) -> bool {
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let ab = b - a;
        let t = if ab.length_squared() == 0.0 {
            0.0
        } else {
            ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        };
        if (p - (a + ab * t)).length() <= 1e-9 {
            return true;
        }
    }
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) {
            let t = (p.y - a.y) / (b.y - a.y);
            if p.x < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
    }
    inside
}

/// 折り目の向き(山谷)と層順序が食い違わないことを確かめる。
fn assert_fold_senses(doc: &Document, label: &str) {
    let (faces, state) = state_of(doc);
    let mut edge_faces: HashMap<u32, Vec<FaceId>> = HashMap::new();
    for f in &faces {
        let mut ids = f.edges.clone();
        ids.sort_unstable();
        ids.dedup();
        for eid in ids {
            edge_faces.entry(eid).or_default().push(f.id);
        }
    }
    let mut checked = 0usize;
    for e in &doc.cp.edges {
        if !matches!(e.kind, EdgeKind::Mountain | EdgeKind::Valley) {
            continue;
        }
        let Some(fs) = edge_faces.get(&e.id) else {
            continue;
        };
        if fs.len() != 2 {
            continue;
        }
        let (a, b) = (fs[0], fs[1]);
        let (pa, pb) = (state.placements[&a], state.placements[&b]);
        // 折られていない(平らにつながったまま)折り目は上下を決めない
        if pa.mirrored == pb.mirrored {
            continue;
        }
        let above = matches!(
            (e.kind, pa.mirrored),
            (EdgeKind::Valley, false) | (EdgeKind::Mountain, true)
        );
        let ia = state.order.iter().position(|&id| id == a).unwrap();
        let ib = state.order.iter().position(|&id| id == b).unwrap();
        checked += 1;
        assert_eq!(
            ib > ia,
            above,
            "{label}: 折り目(辺{} {:?})でつながる面{a}(層{ia})と面{b}(層{ib})の上下が向きと食い違う",
            e.id,
            e.kind
        );
    }
    assert!(checked > 0, "{label}: 折られた折り目が1本も無い");
}

/// 折り上がりが平ら(全ての面がz=0に乗る)ことを確かめる。
fn assert_flat(doc: &Document, label: &str) {
    let result = replay(doc, doc.sequence.len(), 1.0);
    let (_, state) = state_of(doc);
    assert!(
        result.warnings.is_empty(),
        "{label}: 再生の警告 {:?}",
        result.warnings
    );
    assert!(
        result.skipped.is_empty(),
        "{label}: 飛ばした手順 {:?}",
        result.skipped
    );
    assert!(
        result
            .frame
            .faces
            .iter()
            .all(|f| f.polygon.iter().all(|p| p[2].abs() < 1e-6)),
        "{label}: 折り上がりは平ら"
    );
    assert_eq!(
        result.frame.faces.len(),
        state.placements.len(),
        "{label}: 3D表示と平坦状態の面数"
    );
    for face in &result.frame.faces {
        assert_eq!(
            face.mirrored, state.placements[&face.face].mirrored,
            "{label}: 面{}の3D表示と平坦状態の鏡映偶奇",
            face.face
        );
    }
    assert!(
        state
            .placements
            .values()
            .any(|placement| placement.mirrored)
            && state
                .placements
                .values()
                .any(|placement| !placement.mirrored),
        "{label}: 表向き面と裏返った面の双方を検査する"
    );
}

// ---------------------------------------------------------------------------
// カエルを折る
// ---------------------------------------------------------------------------

/// 予備基本形(`acceptance_crane.rs` の `preliminary_base` と同じ手順)。
/// 畳んだ形は [0,0.5]x[0.5,1] の正方形で、紙の4隅が開いた先端 (0,1) に、
/// 紙の中心が閉じた角 (0.5,0.5) に来る。
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

/// 紙の中心から辺の中点 `mid` へ向かう折り目(=予備基本形の袋の背)。
/// 戻り値は(畳み平面での背の線分, その背でつながる2層(下→上))。
///
/// 畳み平面の座標は1手ごとに全体の等長変換だけずれるので、折り線は毎回この関数で
/// 「今どこにあるか」を読み直してから渡す(UIで見えている折り目をつまむのと同じ)。
fn spine_to(doc: &Document, mid: [f64; 2]) -> ([[f64; 2]; 2], [FaceId; 2]) {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    let (center, m) = (DVec2::new(0.5, 0.5), DVec2::from(mid));
    let mut edge_faces: HashMap<u32, Vec<FaceId>> = HashMap::new();
    for f in &faces {
        for e in &f.edges {
            edge_faces.entry(*e).or_default().push(f.id);
        }
    }
    let rank = |id: &FaceId| {
        state
            .order
            .iter()
            .position(|x| x == id)
            .expect("層順序の面")
    };
    for e in &doc.cp.edges {
        let (Some(&p0), Some(&p1)) = (pos.get(&e.v0), pos.get(&e.v1)) else {
            continue;
        };
        let same = |a: DVec2, b: DVec2| (a - b).length() < 1e-9;
        if !((same(p0, center) && same(p1, m)) || (same(p1, center) && same(p0, m))) {
            continue;
        }
        let fs = edge_faces.get(&e.id).expect("背に面がある");
        assert_eq!(fs.len(), 2, "背は2層をつなぐ");
        let pl = state.placements[&fs[0]];
        let (a, b) = (pl.apply(p0), pl.apply(p1));
        let (lo, hi) = if rank(&fs[0]) < rank(&fs[1]) {
            (fs[0], fs[1])
        } else {
            (fs[1], fs[0])
        };
        return ([[a.x, a.y], [b.x, b.y]], [lo, hi]);
    }
    panic!("紙の中心から {mid:?} への折り目が見つからない");
}

/// カエルの基本形の下ごしらえ: 予備基本形の4つの袋を順に開いてつぶす。
///
/// 袋の背は紙の中心を支点に、紙の隅の向き(閉じた角と開いた先端を結ぶ線)へ倒れる。
/// 背でつながる2層はどちらも二等分線で折り返され、折り返した紙は開いた袋の中に入る。
/// 重なりの外側を回る袋(いちばん下といちばん上をつなぐ1つ)だけは、袋の中が
/// 重なりの外側にあたるので `open_to_back` を反対にする。
///
/// 戻り値は文書と、最後の操作が返した平坦状態(再生一致の検証に使う)。
fn squashed_base() -> (Document, FlatState) {
    let mut doc = preliminary_base();
    let mut last = None;
    for mid in [[0.5, 1.0], [0.0, 0.5], [0.5, 0.0], [1.0, 0.5]] {
        let (line, pocket) = spine_to(&doc, mid);
        let (_, state) = state_of(&doc);
        let outer = pocket[0] == state.order[0] && pocket[1] == *state.order.last().unwrap();
        let corner = only(&doc, [0.0, 0.0], "紙の隅");
        last = Some(apply(
            &mut doc,
            squash,
            pocket.to_vec(),
            line,
            [corner.x, corner.y],
            Some(outer),
        ));
    }
    (doc, last.expect("最後の操作の平坦状態"))
}

/// 展開図の四角い範囲にすっぽり入っている紙(=紙の1/4=1つの袋)の層を下から順に返す。
fn layers_in_quarter(doc: &Document, b: [f64; 4]) -> Vec<FaceId> {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let f = faces.iter().find(|f| f.id == *id).expect("層順序の面");
            f.vertices.iter().filter_map(|v| pos.get(v)).all(|p| {
                p.x >= b[0] - 1e-9 && p.x <= b[2] + 1e-9 && p.y >= b[1] - 1e-9 && p.y <= b[3] + 1e-9
            })
        })
        .collect()
}

/// 紙の1/4ずつの範囲(展開図での四隅の正方形)。
const QUARTERS: [[f64; 4]; 4] = [
    [0.0, 0.0, 0.5, 0.5],
    [0.5, 0.0, 1.0, 0.5],
    [0.5, 0.5, 1.0, 1.0],
    [0.0, 0.5, 0.5, 1.0],
];

/// 畳んだ形を、紙の中心(閉じた角)から見た極座標で測る。
/// 戻り値は(中心からの距離, 中心→紙の隅の向きからの角(度))の一覧。
fn polar_from_apex(doc: &Document) -> Vec<(f64, f64)> {
    let apex = only(doc, [0.5, 0.5], "紙の中心");
    let axis = (only(doc, [0.0, 0.0], "紙の隅") - apex).normalize();
    let (faces, state) = state_of(doc);
    let mut out = Vec::new();
    for f in &faces {
        for p in plane_poly(&doc.cp, f, &state) {
            let v = p - apex;
            let r = v.length();
            let a = if r < 1e-12 {
                0.0
            } else {
                axis.angle_to(v).to_degrees()
            };
            out.push((r, a));
        }
    }
    out
}

/// 予備基本形の4つの袋を開いてつぶすと、90°の正方形が45°のたこ形になる。
#[test]
fn squashing_the_four_pockets_narrows_the_base_to_a_kite() {
    let (doc, _) = squashed_base();
    let (faces, state) = state_of(&doc);
    // 紙の1/4がそれぞれ「もとの紙+折り返した三角2枚」の3面に分かれる
    assert_eq!(faces.len(), 12, "12面");
    assert_eq!(state.order.len(), 12);
    assert_eq!(
        doc.sequence.len(),
        8,
        "折り操作は8手(半分2回・組み替え2回・つぶし4回)"
    );

    // 紙の4隅は1点に、紙の中心も1点に集まったまま
    let apex = only(&doc, [0.5, 0.5], "紙の中心");
    for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
        let p = only(&doc, corner, "紙の隅");
        assert!(
            (p - only(&doc, [0.0, 0.0], "紙の隅")).length() < 1e-9,
            "4隅は1点に重なる(実際 {p:?})"
        );
    }
    // 辺の中点は、開いた背の行き先である中心線の上(中心から0.5)へ来る
    let axis = (only(&doc, [0.0, 0.0], "紙の隅") - apex).normalize();
    for mid in [[0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]] {
        let p = only(&doc, mid, "辺の中点") - apex;
        assert!(
            (p.length() - 0.5).abs() < 1e-9,
            "中心から0.5(実際 {})",
            p.length()
        );
        assert!(axis.angle_to(p).abs() < 1e-9, "中心線の上に乗る");
    }

    // 外形: 中心から見て開き角は±22.5°、いちばん遠い点は紙の隅(√2/2)、
    // たこ形の左右の角は 0.5/cos(22.5°)
    let polar = polar_from_apex(&doc);
    let width = polar.iter().map(|(_, a)| a.abs()).fold(0.0, f64::max);
    let far = polar.iter().map(|(r, _)| *r).fold(0.0, f64::max);
    assert!((width - 22.5).abs() < 1e-9, "開き角は±22.5°(実際 {width})");
    assert!(
        (far - 0.5 * std::f64::consts::SQRT_2).abs() < 1e-9,
        "いちばん遠い点は紙の隅(実際 {far})"
    );
    let side = polar
        .iter()
        .filter(|(_, a)| (a.abs() - 22.5).abs() < 1e-9)
        .map(|(r, _)| *r)
        .fold(0.0, f64::max);
    assert!(
        (side - 0.5 / HALF.cos()).abs() < 1e-9,
        "たこ形の角(実際 {side})"
    );

    // 表示上の重なりは折り目の向きとの一致で確かめる。開いてつぶす折りは既にある
    // 折り目を開く動きを含み、手順再生は前の手順の折り目を180°に固定したまま補間する
    // ので、折り終わる直前(t=0.99)の高さは開く途中の大回りを指してしまう
    // (`acceptance_crane.rs` の冒頭と同じ事情)。折り上がり(t=1)の形と重なりは正しい
    assert_fold_senses(&doc, "つぶした後");
    assert_flat(&doc, "つぶした後");
}

/// 折り返した紙は、それを分けたもとの層のすぐ隣(開いた袋の中)に入る。
/// 紙の1/4ずつが「三角・もとの紙・三角」の3層続きになり、それが4組重なる。
#[test]
fn each_quarter_keeps_its_squashed_triangles_next_to_it() {
    let (doc, _) = squashed_base();
    let (faces, state) = state_of(&doc);
    let mut seen = 0usize;
    for q in QUARTERS {
        let unit = layers_in_quarter(&doc, q);
        assert_eq!(unit.len(), 3, "1/4は3面に分かれる(実際 {unit:?})");
        let at: Vec<usize> = unit
            .iter()
            .map(|id| {
                state
                    .order
                    .iter()
                    .position(|x| x == id)
                    .expect("層順序の面")
            })
            .collect();
        assert_eq!(at[1], at[0] + 1, "折り返した紙はもとの紙の隣(実際 {at:?})");
        assert_eq!(at[2], at[1] + 1, "折り返した紙はもとの紙の隣(実際 {at:?})");
        let corners: Vec<usize> = unit
            .iter()
            .map(|id| {
                faces
                    .iter()
                    .find(|f| f.id == *id)
                    .expect("面")
                    .vertices
                    .len()
            })
            .collect();
        assert_eq!(
            corners,
            vec![3, 4, 3],
            "三角・もとの紙・三角の順(実際 {corners:?})"
        );
        seen += 1;
    }
    assert_eq!(seen, 4, "1/4は4組");

    // 中心線の左右どちらでも紙は8枚重なる(予備基本形の4層が倍になった)
    let apex = only(&doc, [0.5, 0.5], "紙の中心");
    let axis = (only(&doc, [0.0, 0.0], "紙の隅") - apex).normalize();
    for deg in [11.25_f64, -11.25] {
        let (s, c) = deg.to_radians().sin_cos();
        let dir = DVec2::new(axis.x * c - axis.y * s, axis.x * s + axis.y * c);
        let probe = apex + dir * 0.3;
        let layers = faces
            .iter()
            .filter(|f| inside_polygon(&plane_poly(&doc.cp, f, &state), probe))
            .count();
        assert_eq!(layers, 8, "中心線から{deg}°の向きでは8層(実際 {layers})");
    }
}

/// カエルの基本形。下ごしらえのたこ形を**1回の花弁折り**で折る。
///
/// 4つの袋は紙の中心から辺の中点へ向かう折り目でつながっていて、その折り目は
/// たこ形の中心線にそのまま乗っている。花弁折りのちょうつがい(0.3536)は
/// この折り目(中心から0.5)の途中を横切るので、袋を1つだけ選ぶと必ずそこで
/// 紙が裂ける。4つの袋は畳み平面でぴったり重なっているので、全層を選んだ
/// 1回の花弁折りがそのまま「4つの袋を同時に花弁折りする」ことになる。
fn frog_base() -> (Document, FlatState) {
    let (mut doc, _) = squashed_base();
    let apex = only(&doc, [0.5, 0.5], "紙の中心");
    let corner = only(&doc, [0.0, 0.0], "紙の隅");
    let (_, state) = state_of(&doc);
    let all = state.order.clone();
    let line = [[apex.x, apex.y], [corner.x, corner.y]];
    let last = apply(&mut doc, petal, all, line, [corner.x, corner.y], None);
    (doc, last)
}

/// カエルの基本形の畳み平面を測る道具。
/// 戻り値は(先端の位置, 先端から根元へ向かう向き)。
fn frog_axis(doc: &Document) -> (DVec2, DVec2) {
    let apex = only(doc, [0.5, 0.5], "紙の中心");
    let mid = only(doc, [0.5, 1.0], "辺の中点");
    (apex, (mid - apex).normalize())
}

/// 花弁折り1回でカエルの基本形になる。8本の先が1つの先端へ集まる。
#[test]
fn petal_folding_the_kite_makes_the_frog_base() {
    let (doc, _) = frog_base();
    let (faces, _) = state_of(&doc);
    assert_eq!(
        doc.sequence.len(),
        9,
        "折り操作は9手(下ごしらえ8手+花弁折り1手)"
    );
    assert_eq!(faces.len(), 40, "40面");

    // 紙の4隅は紙の中心と同じ1点(先端)に集まる = 隅から出る4本の足が最大長
    let (apex, axis) = frog_axis(&doc);
    for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
        let p = only(&doc, corner, "紙の隅");
        assert!(
            (p - apex).length() < 1e-9,
            "紙の隅は紙の中心と同じ先端へ来る(実際 {p:?})"
        );
    }
    // 4つの辺の中点は先端から 0.2071、つぶし折りの角(隅から0.2929の点)は 0.2929
    for (cp, want) in [
        ([0.5, 1.0], 0.5 * std::f64::consts::SQRT_2 - 0.5),
        ([0.0, 0.5], 0.5 * std::f64::consts::SQRT_2 - 0.5),
        ([0.5, 0.0], 0.5 * std::f64::consts::SQRT_2 - 0.5),
        ([1.0, 0.5], 0.5 * std::f64::consts::SQRT_2 - 0.5),
        (
            [std::f64::consts::FRAC_1_SQRT_2, 1.0],
            1.0 - std::f64::consts::FRAC_1_SQRT_2,
        ),
        (
            [0.0, 1.0 - std::f64::consts::FRAC_1_SQRT_2],
            1.0 - std::f64::consts::FRAC_1_SQRT_2,
        ),
    ] {
        let v = only(&doc, cp, "境界の点") - apex;
        assert!(
            (v.length() - want).abs() < 1e-9,
            "{cp:?} は先端から {want}(実際 {})",
            v.length()
        );
        assert!(axis.angle_to(v).abs() < 1e-9, "中心線の上に乗る");
    }
}

/// カエルの基本形の外形: 先端から根元まで √2/4、根元の半幅はその tan(22.5°) 倍。
#[test]
fn the_frog_base_is_a_45_degree_kite_of_half_diagonal() {
    let (doc, _) = frog_base();
    let (faces, state) = state_of(&doc);
    let (apex, axis) = frog_axis(&doc);
    let perp = DVec2::new(-axis.y, axis.x);
    let root = 0.25 * std::f64::consts::SQRT_2;
    let (mut far, mut wide) = (0.0_f64, 0.0_f64);
    for f in &faces {
        for p in plane_poly(&doc.cp, f, &state) {
            far = far.max((p - apex).dot(axis));
            wide = wide.max((p - apex).dot(perp).abs());
        }
        assert!(
            plane_poly(&doc.cp, f, &state)
                .iter()
                .all(|p| (*p - apex).dot(axis) >= -1e-9),
            "先端より外へ出る紙は無い"
        );
    }
    assert!(
        (far - root).abs() < 1e-9,
        "先端から根元まで √2/4(実際 {far})"
    );
    assert!(
        (wide - root * HALF.tan()).abs() < 1e-9,
        "根元の半幅(実際 {wide})"
    );

    // 中心線の左右どちらでも紙は16枚重なる(下ごしらえの8層+折り返した紙8層)
    for deg in [11.25_f64, -11.25] {
        let (s, c) = deg.to_radians().sin_cos();
        let dir = DVec2::new(axis.x * c - axis.y * s, axis.x * s + axis.y * c);
        let probe = apex + dir * (root * 0.5);
        let layers = faces
            .iter()
            .filter(|f| inside_polygon(&plane_poly(&doc.cp, f, &state), probe))
            .count();
        assert_eq!(layers, 16, "中心線から{deg}°の向きでは16層(実際 {layers})");
    }
    assert_fold_senses(&doc, "カエルの基本形");
    assert_flat(&doc, "カエルの基本形");
}

/// 展開図と手順だけから同じ形に折り直せる(3D状態を保存しない設計の検証)。
#[test]
fn the_frog_base_replays_from_the_crease_pattern() {
    let (doc, built) = frog_base();
    let faces = extract_faces(&doc.cp);
    let (replayed, warnings) =
        flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "再生の警告: {warnings:?}");
    assert_eq!(replayed.order, built.order, "層順序が構築時と一致する");
    let pos = vertex_pos(&doc.cp);
    for f in &faces {
        let built_pl = built.placements[&f.id];
        let replay_pl = replayed.placements[&f.id];
        for p in f.vertices.iter().filter_map(|v| pos.get(v)) {
            assert!(
                (built_pl.apply(*p) - replay_pl.apply(*p)).length() < 1e-9,
                "面 {} の位置が構築時と一致する",
                f.id
            );
        }
    }
}

/// 同じ操作列は何度実行しても同じ結果になる(決定性)。
#[test]
fn the_frog_base_is_deterministic() {
    let (a, _) = frog_base();
    let (b, _) = frog_base();
    assert_eq!(a.cp, b.cp, "展開図が一致する");
    assert_eq!(a.sequence, b.sequence, "手順が一致する");
    let frame = |doc: &Document| format!("{:?}", replay(doc, doc.sequence.len(), 1.0).frame);
    assert_eq!(frame(&a), frame(&b), "折り上がりの3D姿勢がビット一致する");
}

// ---------------------------------------------------------------------------
// 完成形(足4本と体の段折り)
// ---------------------------------------------------------------------------

/// 足4本の中割り折り: (紙の隅, 中心線のどちら側へ出すか, 先端からの距離)。
/// 前足2本は先端寄り、後ろ足2本は根元寄りで折る。
const LEGS: [([f64; 2], f64, f64); 4] = [
    ([0.0, 0.0], 1.0, 0.10),
    ([1.0, 0.0], -1.0, 0.10),
    ([1.0, 1.0], 1.0, 0.20),
    ([0.0, 1.0], -1.0, 0.20),
];

/// 体の段折りの位置(先端から)と段の幅。
const PLEAT_AT: f64 = 0.30;
const PLEAT_GAP: f64 = 0.02;

/// 紙の隅から出た足1本の層(下から順)。
///
/// カエルの基本形では紙の4隅がどれも先端(紙の中心と同じ点)に集まっているので、
/// 「その隅を持つ面」を集めれば足1本になる。花弁折りが持ち上げた紙を袋ごとに
/// 置き直すようになって初めて、この3面が層順序の上でひとまとまりになり、
/// 足1本をつまんで中割り折りできる。
fn leg_layers(doc: &Document, corner: [f64; 2]) -> Vec<FaceId> {
    let (faces, state) = state_of(doc);
    let pos = vertex_pos(&doc.cp);
    let t = DVec2::from(corner);
    state
        .order
        .iter()
        .copied()
        .filter(|id| {
            let f = faces.iter().find(|f| f.id == *id).expect("層順序の面");
            f.vertices
                .iter()
                .filter_map(|v| pos.get(v))
                .any(|p| (*p - t).length() < 1e-9)
        })
        .collect()
}

/// 伝承のカエル。カエルの基本形の足4本を中割り折りで体の外へ出し、
/// 体の根元を段折りする。
///
/// 足の先は基本形の先端にあり、中心線と45°をなす折り線で中割り折りすると
/// 中心線から45°の向き(基本形の開き角±22.5°の外)へ出る。畳み平面の座標は
/// 1手ごとに全体の等長変換だけずれるので、折り線は毎回 [`frog_axis`] で
/// 読み直してから渡す。
/// 戻り値は文書と、最後の操作が返した平坦状態(再生一致の検証に使う)。
fn frog() -> (Document, FlatState) {
    let (mut doc, _) = frog_base();
    for (corner, side, along) in LEGS {
        let (apex, axis) = frog_axis(&doc);
        let perp = DVec2::new(-axis.y, axis.x) * side;
        let hinge = apex + axis * along;
        let dir = (axis + perp).normalize();
        let keep = apex + axis * (along + 0.05);
        let leg = leg_layers(&doc, corner);
        assert_eq!(leg.len(), 3, "足1本は3面(実際 {leg:?})");
        apply(
            &mut doc,
            inside_reverse,
            leg,
            [[hinge.x, hinge.y], [hinge.x + dir.x, hinge.y + dir.y]],
            [keep.x, keep.y],
            None,
        );
    }
    let (apex, axis) = frog_axis(&doc);
    let perp = DVec2::new(-axis.y, axis.x);
    let a = apex + axis * PLEAT_AT;
    let r = apex + axis * (PLEAT_AT + PLEAT_GAP);
    let last = apply(
        &mut doc,
        pleat,
        Vec::new(),
        [[a.x, a.y], [(a + perp).x, (a + perp).y]],
        [r.x, r.y],
        None,
    );
    (doc, last)
}

/// 完成形。足4本が体の外へ出て、体の根元に段が入る。
#[test]
fn the_frog_has_four_legs_sticking_out_of_the_body() {
    let (doc, _) = frog();
    let (faces, state) = state_of(&doc);
    assert_eq!(
        doc.sequence.len(),
        14,
        "折り操作は14手(基本形9手+足4手+段折り1手)"
    );
    assert_eq!(faces.len(), 140, "140面");
    assert_eq!(state.order.len(), 140);

    // 足4本の先は、中心線から±45°の向き(基本形の開き角±22.5°の外)に、
    // 先端から along*√2 のところへ出る
    let (apex, axis) = frog_axis(&doc);
    let perp = DVec2::new(-axis.y, axis.x);
    for (corner, side, along) in LEGS {
        let v = only(&doc, corner, "足の先") - apex;
        let want = axis * along - perp * (side * along);
        assert!(
            (v - want).length() < 1e-9,
            "{corner:?} の足の先(実際 {v:?} 期待 {want:?})"
        );
        let deg = axis.angle_to(v).to_degrees();
        assert!(
            (deg.abs() - 45.0).abs() < 1e-9,
            "足は中心線から45°(実際 {deg})"
        );
        assert!(
            (v.length() - along * std::f64::consts::SQRT_2).abs() < 1e-9,
            "足の先までの距離(実際 {})",
            v.length()
        );
    }
    // 4本は4つの別々の向き・距離に出る(重ならない)
    let tips: Vec<DVec2> = LEGS
        .iter()
        .map(|(c, _, _)| only(&doc, *c, "足の先"))
        .collect();
    for i in 0..tips.len() {
        for j in (i + 1)..tips.len() {
            assert!(
                (tips[i] - tips[j]).length() > 1e-6,
                "足{i}と足{j}は別の位置"
            );
        }
    }

    // 体は段折りの分だけ短くなる(根元は √2/4 から段の幅の2倍だけ縮む)
    let root = 0.25 * std::f64::consts::SQRT_2 - 2.0 * PLEAT_GAP;
    let far = faces
        .iter()
        .flat_map(|f| plane_poly(&doc.cp, f, &state))
        .map(|p| (p - apex).dot(axis))
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (far - root).abs() < 1e-9,
        "段折りで体が縮む(実際 {far} 期待 {root})"
    );

    assert_fold_senses(&doc, "完成したカエル");
    assert_flat(&doc, "完成したカエル");
}

/// 折っている最中も紙がつながったままであること(実機で報告された不具合の回帰)。
///
/// 折り目の両端の頂点を、その折り目を共有する2面それぞれの3D多角形から読み、
/// 同じ位置に来ているかを見る。離れていたら紙がちぎれている。
/// 全ヒンジ角を線形補間しただけの値は内部頂点まわりのループ閉包を満たさない。
#[test]
fn frog_paper_stays_connected_while_folding() {
    let (doc, _) = frog();
    let faces = extract_faces(&doc.cp);
    // 全手順×全tは重いので、袋を開く工程(内部頂点が増える)と完成形を代表で見る
    for up_to in [5, 9, doc.sequence.len()] {
        for k in [1, 2, 3] {
            let t = f64::from(k) / 4.0;
            let frame = replay(&doc, up_to, t).frame;
            let gap = max_seam_gap(&doc.cp, &faces, &frame);
            assert!(
                gap < 1e-6,
                "カエル(手順{up_to}, t={t}): 面が {gap:.9} 離れている"
            );
        }
    }
}

/// 足1本は層順序の上でひとまとまりになっている(つまんで中割り折りできる)。
/// カエルの基本形で足4本を確かめる。
#[test]
fn each_leg_is_a_bundle_of_neighbouring_layers() {
    let (doc, _) = frog_base();
    let (faces, state) = state_of(&doc);
    let apex = only(&doc, [0.5, 0.5], "紙の中心");
    for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
        let leg = leg_layers(&doc, corner);
        assert_eq!(leg.len(), 3, "足1本は3面(実際 {leg:?})");
        // 足の先の近くでは、足の層が続きになっている(間に別の紙が入らない)
        let (_, axis) = frog_axis(&doc);
        let perp = DVec2::new(-axis.y, axis.x);
        for sign in [1.0_f64, -1.0] {
            let probe = apex + axis * 0.05 + perp * (sign * 0.01);
            let here: Vec<FaceId> = state
                .order
                .iter()
                .copied()
                .filter(|id| {
                    let f = faces.iter().find(|f| f.id == *id).expect("層順序の面");
                    inside_polygon(&plane_poly(&doc.cp, f, &state), probe)
                })
                .collect();
            let at: Vec<usize> = (here.iter().enumerate())
                .filter(|(_, id)| leg.contains(id))
                .map(|(k, _)| k)
                .collect();
            assert_eq!(at.len(), 2, "先端の近くでは足は2層(実際 {at:?})");
            assert_eq!(
                at[1],
                at[0] + 1,
                "足の2層は隣どうし(実際 {at:?} / {here:?})"
            );
        }
    }
}

/// 完成形も展開図と手順だけから同じ形に折り直せる。
#[test]
fn the_frog_replays_from_the_crease_pattern() {
    let (doc, built) = frog();
    let faces = extract_faces(&doc.cp);
    let (replayed, warnings) =
        flat_state_at(&doc, &faces, doc.sequence.len()).expect("平らに畳める");
    assert!(warnings.is_empty(), "再生の警告: {warnings:?}");
    assert_eq!(replayed.order, built.order, "層順序が構築時と一致する");
    let pos = vertex_pos(&doc.cp);
    for f in &faces {
        let (b, r) = (built.placements[&f.id], replayed.placements[&f.id]);
        for p in f.vertices.iter().filter_map(|v| pos.get(v)) {
            assert!(
                (b.apply(*p) - r.apply(*p)).length() < 1e-9,
                "面 {} の位置が一致する",
                f.id
            );
        }
    }
}

/// 完成形も何度実行しても同じ結果になる(決定性)。
#[test]
fn the_frog_is_deterministic() {
    let (a, _) = frog();
    let (b, _) = frog();
    assert_eq!(a.cp, b.cp, "展開図が一致する");
    assert_eq!(a.sequence, b.sequence, "手順が一致する");
    let frame = |doc: &Document| format!("{:?}", replay(doc, doc.sequence.len(), 1.0).frame);
    assert_eq!(frame(&a), frame(&b), "折り上がりの3D姿勢がビット一致する");
}

/// 伝承カエルの受入れ条件を、構築結果そのものから確認する。
///
/// 既存の個別検査は足・再生・決定性を別々に確認している。この検査では花弁、
/// 中割り、段折りを一つの完成形で束ね、同じ操作列を2回構築しても最終の平坦
/// 再生と紙の接続が変わらないことを確認する。
#[test]
fn traditional_frog_has_required_techniques_and_replays_connected_twice() {
    const POSITION_EPS: f64 = 1e-9;
    // 2回測定の最大gapは0。明示した平坦層の組立てで出る丸めだけを許すため、
    // モデル共通EPS(1e-9)を境界にする。可視の裂け(1e-6)より十分小さい。
    const FLAT_GAP_TOLERANCE: f64 = 1e-9;

    let (first, first_built) = frog();
    let (second, second_built) = frog();
    let documents = [
        (&first, &first_built, "1回目"),
        (&second, &second_built, "2回目"),
    ];

    let technique_count = |kind| {
        first
            .sequence
            .iter()
            .filter(|step| step.kind == kind)
            .count()
    };
    assert!(
        technique_count(TechniqueKind::Petal) >= 1,
        "花弁折りを1手以上含む"
    );
    assert!(
        technique_count(TechniqueKind::InsideReverse) >= 1,
        "中割り折りを1手以上含む"
    );
    assert!(
        technique_count(TechniqueKind::Pleat) >= 1,
        "段折りを1手以上含む"
    );

    let mut replayed_signatures = Vec::new();
    let mut connectivity_violations = 0usize;
    for (document, built, label) in documents {
        let faces = extract_faces(&document.cp);
        let (replayed, warnings) = flat_state_at(document, &faces, document.sequence.len())
            .expect("完成したカエルを平坦に再生できる");
        assert!(warnings.is_empty(), "{label}: 再生の警告なし: {warnings:?}");
        assert!(
            !replayed.order.is_empty(),
            "{label}: 完成形の最終層数は1以上"
        );
        assert_eq!(
            replayed.order, built.order,
            "{label}: 構築時の層順序を再生する"
        );

        let positions = vertex_pos(&document.cp);
        let mut signature = Vec::new();
        for face in &faces {
            for vertex in face
                .vertices
                .iter()
                .filter_map(|vertex| positions.get(vertex))
            {
                let built_position = built.placements[&face.id].apply(*vertex);
                let replayed_position = replayed.placements[&face.id].apply(*vertex);
                assert!(
                    (built_position - replayed_position).length() <= POSITION_EPS,
                    "{label}: 面{}の再生位置差は {POSITION_EPS:e} 以下",
                    face.id
                );
                signature.push((face.id, *vertex, replayed_position));
            }
        }
        replayed_signatures.push(signature);

        let flat_frame = explicit_flat_frame(document, &faces, &replayed);
        if max_seam_gap(&document.cp, &faces, &flat_frame) >= FLAT_GAP_TOLERANCE {
            connectivity_violations += 1;
        }
    }
    assert_eq!(
        connectivity_violations, 0,
        "2回の完成形再生で層の接続違反は0件"
    );

    let (first_signature, second_signature) = (&replayed_signatures[0], &replayed_signatures[1]);
    assert_eq!(
        first_signature.len(),
        second_signature.len(),
        "2回の完成形の頂点数は一致する"
    );
    for (
        (first_face, first_vertex, first_position),
        (second_face, second_vertex, second_position),
    ) in first_signature.iter().zip(second_signature)
    {
        assert_eq!(first_face, second_face, "2回の完成形で面IDが一致する");
        assert_eq!(first_vertex, second_vertex, "2回の完成形で頂点IDが一致する");
        assert!(
            (*first_position - *second_position).length() <= POSITION_EPS,
            "2回の完成形の位置差は {POSITION_EPS:e} 以下"
        );
    }
}

// ---------------------------------------------------------------------------
// フロント側テスト用フィクスチャの読み取り検証
// ---------------------------------------------------------------------------

/// 完成形の展開図と面を、フロント側(vitest)が読めるJSONにする。
/// 対称軸の判定(`apps/desktop/src/lib/grabDrive.ts`)を**実データ**で検証するため。
/// 統合テストは1ファイル=1クレートなので `acceptance_crane.rs` と同じ内容を置く。
fn front_fixture_json(doc: &Document, faces: &[Face]) -> String {
    let mut s = String::from("{\n");
    let (w, h) = (doc.paper.width_mm, doc.paper.height_mm);
    s.push_str(&format!(
        "  \"paper\": {{ \"width_mm\": {w:?}, \"height_mm\": {h:?} }},\n"
    ));
    s.push_str("  \"vertices\": [\n");
    for (i, v) in doc.cp.vertices.iter().enumerate() {
        let (id, x, y) = (v.id, v.pos[0], v.pos[1]);
        let comma = if i + 1 < doc.cp.vertices.len() {
            ","
        } else {
            ""
        };
        s.push_str(&format!(
            "    {{ \"id\": {id}, \"pos\": [{x:?}, {y:?}] }}{comma}\n"
        ));
    }
    s.push_str("  ],\n  \"edges\": [\n");
    for (i, e) in doc.cp.edges.iter().enumerate() {
        let (id, v0, v1, kind) = (e.id, e.v0, e.v1, e.kind);
        let comma = if i + 1 < doc.cp.edges.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"id\": {id}, \"v0\": {v0}, \"v1\": {v1}, \"kind\": \"{kind:?}\" }}{comma}\n"
        ));
    }
    s.push_str("  ],\n  \"faces\": [\n");
    for (i, f) in faces.iter().enumerate() {
        let (id, vs, es) = (f.id, &f.vertices, &f.edges);
        let comma = if i + 1 < faces.len() { "," } else { "" };
        s.push_str(&format!(
            "    {{ \"id\": {id}, \"vertices\": {vs:?}, \"edges\": {es:?} }}{comma}\n"
        ));
    }
    s.push_str("  ]\n}\n");
    s
}

/// 明示的な再生成専用: `cargo test -p ori3-layers --test acceptance_frog regenerate_frog_front_fixture -- --ignored --exact`
/// コミット済みHEADの複製内で実行し、生成したfrog.jsonだけを本体へコピーする。
#[test]
#[ignore = "フロント用カエルfixtureを明示的に再生成するときだけ実行する"]
fn regenerate_frog_front_fixture() {
    let (doc, _) = frog();
    let faces = extract_faces(&doc.cp);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/desktop/src/lib/__fixtures__/frog.json");
    std::fs::write(&path, front_fixture_json(&doc, &faces))
        .expect("フロント用カエルfixtureを書き直す");
}

/// 数値を `#` へ置き換えた骨組みと、取り出した数値の並びに分ける。
///
/// 座標は計算機や数学ライブラリの違いで最下位の桁が変わることがあるため、
/// 文字列のまま厳密に比べると、どの計算機で作り直しても他方では一致しなくなる。
/// 骨組み(項目名・並び・整数のID)は厳密に、座標は許容差で比べるために分ける。
fn split_numbers(text: &str) -> (String, Vec<f64>) {
    let chars: Vec<char> = text.chars().collect();
    let mut skeleton = String::with_capacity(text.len());
    let mut numbers = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let head = chars[i];
        let starts_number = head.is_ascii_digit()
            || (head == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit());
        if !starts_number {
            skeleton.push(head);
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < chars.len() {
            let c = chars[i];
            if c.is_ascii_digit() || c == '.' {
                i += 1;
            } else if c == 'e' || c == 'E' {
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
            } else {
                break;
            }
        }
        let token: String = chars[start..i].iter().collect();
        match token.parse::<f64>() {
            Ok(value) => {
                numbers.push(value);
                skeleton.push('#');
            }
            Err(_) => skeleton.push_str(&token),
        }
    }
    (skeleton, numbers)
}

/// apps配下へ書き込まず、既存のカエルフィクスチャが現在の実データと一致するか調べる。
#[test]
fn frog_front_fixture_matches_read_only() {
    /// 座標の差の許容量。紙の一辺を1とした値なので、この差は表示にも計算にも影響しない。
    const COORD_TOLERANCE: f64 = 1e-9;

    let (doc, _) = frog();
    let faces = extract_faces(&doc.cp);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/desktop/src/lib/__fixtures__/frog.json");
    let stored = std::fs::read_to_string(&path).expect("既存のフロント用カエルfixtureを読む");
    let generated = front_fixture_json(&doc, &faces);

    let (stored_shape, stored_numbers) = split_numbers(&stored.replace("\r\n", "\n"));
    let (generated_shape, generated_numbers) = split_numbers(&generated.replace("\r\n", "\n"));

    assert_eq!(
        stored_shape,
        generated_shape,
        "フロント用カエルfixtureの構造が現在の展開図と不一致: {}",
        path.display()
    );
    assert_eq!(
        stored_numbers.len(),
        generated_numbers.len(),
        "フロント用カエルfixtureの数値の個数が不一致: {}",
        path.display()
    );
    for (index, (stored_value, generated_value)) in
        stored_numbers.iter().zip(&generated_numbers).enumerate()
    {
        let scale = stored_value.abs().max(generated_value.abs()).max(1.0);
        assert!(
            (stored_value - generated_value).abs() <= COORD_TOLERANCE * scale,
            "フロント用カエルfixtureの{index}番目の数値が不一致: 保存 {stored_value} / 現在 {generated_value} ({})",
            path.display()
        );
    }
}
