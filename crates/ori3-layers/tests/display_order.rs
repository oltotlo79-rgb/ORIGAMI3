//! 記録した層順序が「画面に見える重なり」と一致することの検証。
//!
//! 層順序(`FoldStep.layer_order` → `FlatState::resolve_order` → `Frame3D.layer`)は
//! 「下から0」と定義されている。ここではその定義を、折り終わる直前(t=0.99)の
//! 高さ(z)から直接読み取った本当の重なりと突き合わせる。
//!
//! なぜ z で読むのか: 完全に畳んだ状態(t=1)では全ての面が z=0 に重なるため、
//! 上下を形から読み取れない。折り終わる直前なら、動いている層だけが浮いており、
//! その符号(上へ浮くか下へ潜るか)が畳み終わりの上下そのものになる。
//!
//! 表示座標系は「根面(最小面ID)を固定した姿勢」なので、根面が動く側にある折りでは
//! 紙全体が裏返って表示される。そのとき記録した層順序も裏返さないと画面と食い違う
//! (この検証が守るのはその一致)。

use std::collections::HashSet;

use ori3_cp::extract_faces;
use ori3_layers::flat_state::representative_point;
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::{flat_state_at, replay};
use ori3_model::{Document, FaceId, Paper};

/// 浮いている(z≠0)と判断する高さ。t=0.99 では紙の大きさに対して十分大きい。
const LIFT_EPS: f64 = 1e-3;

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
    let mut step = res.step;
    step.id = u32::try_from(doc.sequence.len()).unwrap();
    doc.cp = cp;
    doc.sequence.push(step);
}

/// 折り終わる直前(t=0.99)に浮いている面と、その符号(+1=上へ浮く / -1=下へ潜る)。
/// 表示座標系(根面固定)で読むので、これが画面に見える重なりの真値になる。
fn lifted_at_t099(doc: &Document, up_to: usize) -> (HashSet<FaceId>, f64) {
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
            assert_eq!(
                z.signum(),
                sign,
                "動いた層は全て同じ向きへ浮く(面 {} の z={z})",
                f.face
            );
        }
    }
    assert!(!lifted.is_empty(), "折り終わる直前には動いた層が浮いている");
    (lifted, sign)
}

/// `up_to` 手目までの層順序が、t=0.99 の高さから読んだ本当の重なりと一致するか。
/// 一致条件: 上へ浮いた層は層順序の末尾(上側)に、下へ潜った層は先頭(下側)に、
/// それぞれ固まって並ぶ。`Frame3D.layer`(下から0)も同じ並びになる。
fn assert_display_order(doc: &Document, up_to: usize, label: &str) {
    let (lifted, sign) = lifted_at_t099(doc, up_to);
    let faces = extract_faces(&doc.cp);
    let (state, warnings) = flat_state_at(doc, &faces, up_to)
        .unwrap_or_else(|e| panic!("{label}: 畳んだ状態が求まらない: {e}"));
    assert!(
        warnings.is_empty(),
        "{label}: 警告なしで再生できる: {warnings:?}"
    );
    let order = state.order;
    assert_eq!(order.len(), faces.len(), "{label}: 層順序は全ての面を含む");

    let n = lifted.len();
    let (slice, where_) = if sign > 0.0 {
        (&order[order.len() - n..], "上側")
    } else {
        (&order[..n], "下側")
    };
    let got: HashSet<FaceId> = slice.iter().copied().collect();
    assert_eq!(
        got, lifted,
        "{label}: 浮いた層({lifted:?})は層順序の{where_}に並ぶはず(order={order:?})"
    );

    // Frame3D.layer も同じ並びで返る(表示・書き出しはこちらを見る)
    let frame = replay(doc, up_to, 1.0).frame;
    for f3 in &frame.faces {
        let expected = order.iter().position(|&id| id == f3.face).unwrap();
        assert_eq!(
            usize::try_from(f3.layer).unwrap(),
            expected,
            "{label}: 面 {} のlayerは層順序の位置と一致する",
            f3.face
        );
        assert_eq!(
            usize::try_from(f3.surface_rank).unwrap(),
            expected,
            "{label}: 面 {} のsurface_rankは既存層順序の位置と一致する",
            f3.face
        );
    }
}

/// 面IDから展開図上の代表点を引く(どの部分の紙かを言い当てるため)。
fn rep_x(doc: &Document, id: FaceId) -> f64 {
    let faces = extract_faces(&doc.cp);
    let f = faces.iter().find(|f| f.id == id).expect("面がある");
    representative_point(&doc.cp, f)[0]
}

/// (a) 根面(最小面ID)が動かない折り。表示座標系はそのままなので素直に一致する。
#[test]
fn display_order_matches_when_root_face_stays() {
    let mut doc = square_doc();
    // x=0.5 で左半分を右へ折る(動かさない側=右)
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 1.0]],
        [0.75, 0.5],
        FoldDirection::Up,
    );

    let (lifted, sign) = lifted_at_t099(&doc, 1);
    assert_eq!(lifted.len(), 1);
    let moved = *lifted.iter().next().unwrap();
    assert!(rep_x(&doc, moved) < 0.5, "浮くのは動かした左半分");
    assert!(sign > 0.0, "手前へ折った層が上へ回って重なる");
    assert_display_order(&doc, 1, "根面が動かない折り");
}

/// (b) 根面(最小面ID)が動く折り。いちばん普通の1手目がこれにあたる。
/// 表示は紙全体が裏返った姿勢になるため、記録する層順序も裏返さないと画面と食い違う。
#[test]
fn display_order_matches_when_root_face_moves() {
    let mut doc = square_doc();
    // x=0.5 で右半分を左へ折る(動かさない側=左)
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 1.0]],
        [0.25, 0.5],
        FoldDirection::Up,
    );

    let (lifted, sign) = lifted_at_t099(&doc, 1);
    assert_eq!(lifted.len(), 1);
    let moved = *lifted.iter().next().unwrap();
    // 根面は動かした右半分の側にあり、表示ではそれが固定される。つまり画面では
    // 「動かさないはずの左半分」が回って上へ重なる(紙全体が裏返って見える姿勢)。
    assert!(rep_x(&doc, moved) < 0.5, "表示で動いて見えるのは左半分");
    assert!(sign > 0.0, "回った左半分が上に重なる");
    assert_display_order(&doc, 1, "根面が動く折り");
}

/// (c) 3手順を続けて折る。手順ごとに表示の重なりと一致し続ける
/// (根面が動くかどうかは手順ごとに変わるため、偶奇で食い違わないことの確認)。
#[test]
fn display_order_matches_through_three_folds() {
    let mut doc = square_doc();
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 1.0]],
        [0.25, 0.5],
        FoldDirection::Up,
    );
    assert_display_order(&doc, 1, "3手順の1手目");

    fold(
        &mut doc,
        [[0.0, 0.5], [0.5, 0.5]],
        [0.25, 0.25],
        FoldDirection::Up,
    );
    assert_display_order(&doc, 2, "3手順の2手目");

    fold(
        &mut doc,
        [[0.0, 0.25], [0.5, 0.25]],
        [0.25, 0.1],
        FoldDirection::Up,
    );
    assert_display_order(&doc, 3, "3手順の3手目");
}

/// 向こうへ折る(Down=山)場合も同じ規則で一致する。
#[test]
fn display_order_matches_for_down_fold() {
    let mut doc = square_doc();
    fold(
        &mut doc,
        [[0.5, 0.0], [0.5, 1.0]],
        [0.25, 0.5],
        FoldDirection::Down,
    );
    assert_display_order(&doc, 1, "向こうへ折る");
}

/// 手前・向こうを混ぜた蛇腹でも一致する。
#[test]
fn display_order_matches_for_pleat() {
    let mut doc = square_doc();
    // 右端を手前へ折り、続けて残りの一部を向こうへ折る
    fold(
        &mut doc,
        [[0.75, 0.0], [0.75, 1.0]],
        [0.5, 0.5],
        FoldDirection::Up,
    );
    assert_display_order(&doc, 1, "蛇腹の1本目");
    // 1本目で表示座標系が動くので、2本目の折り線は見えている位置(x∈[0.75,1.5])で引く
    fold(
        &mut doc,
        [[1.0, 0.0], [1.0, 1.0]],
        [0.9, 0.5],
        FoldDirection::Down,
    );
    assert_display_order(&doc, 2, "蛇腹の2本目");
}

/// fold_throughが返す平坦状態は、同じ手順を展開図から求め直した
/// [`flat_state_at`] の結果とぴったり一致する(配置も層順序も同じ座標系)。
/// これが崩れると、記録した層順序と表示の重なりが食い違う。
#[test]
fn fold_through_state_equals_flat_state_at() {
    let mut doc = square_doc();
    let folds: [([[f64; 2]; 2], [f64; 2], FoldDirection); 3] = [
        ([[0.5, 0.0], [0.5, 1.0]], [0.25, 0.5], FoldDirection::Up),
        ([[0.5, 0.5], [1.0, 0.5]], [0.75, 0.25], FoldDirection::Up),
        ([[0.5, 0.25], [1.0, 0.25]], [0.75, 0.1], FoldDirection::Down),
    ];
    for (i, (line, keep, direction)) in folds.into_iter().enumerate() {
        let faces = extract_faces(&doc.cp);
        let up_to = doc.sequence.len();
        let (state, _) = flat_state_at(&doc, &faces, up_to).expect("平らな状態");
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
        .unwrap_or_else(|e| panic!("{}手目が折れない: {e}", i + 1));
        let mut step = res.step;
        step.id = u32::try_from(up_to).unwrap();
        doc.cp = cp;
        doc.sequence.push(step);

        let new_faces = extract_faces(&doc.cp);
        let (replayed, _) =
            flat_state_at(&doc, &new_faces, doc.sequence.len()).expect("平らな状態");
        assert_eq!(
            res.state.order,
            replayed.order,
            "{}手目: 層順序が再生結果と一致する",
            i + 1
        );
        for f in &new_faces {
            assert!(
                res.state.placements[&f.id].approx_eq(&replayed.placements[&f.id], 1e-6),
                "{}手目: 面 {} の配置が再生結果と一致する({:?} と {:?})",
                i + 1,
                f.id,
                res.state.placements[&f.id],
                replayed.placements[&f.id]
            );
        }
    }
}
