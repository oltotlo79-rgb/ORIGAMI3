//! 保存した層順序と、幾何から導く表示用の重なり順を分けて検証する。
//!
//! 層順序(`FoldStep.layer_order` → `FlatState::resolve_order` → `Frame3D.layer`)は
//! 「下から0」と定義されている。一方、`Frame3D.surface_rank` は保存順の複製ではなく、
//! 折り終わる直前(t=0.99)の幾何から直接読み取った本当の重なりを表す。
//!
//! なぜ z で読むのか: 完全に畳んだ状態(t=1)では全ての面が z=0 に重なるため、
//! 上下を形から読み取れない。折り終わる直前なら、動いている層だけが浮いており、
//! その符号(上へ浮くか下へ潜るか)が畳み終わりの上下そのものになる。
//!
//! 表示座標系は「根面(最小面ID)を固定した姿勢」なので、根面が動く側にある折りでは
//! 紙全体が裏返って表示される。そのとき記録した層順序も裏返さないと画面と食い違う
//! (この検証が守るのはその一致)。

use std::collections::HashSet;

use glam::{DVec2, DVec3};
use ori3_cp::{Face, extract_faces};
use ori3_layers::flat_state::representative_point;
use ori3_layers::fold_through::{FoldDirection, FoldThroughInput, fold_through};
use ori3_layers::{flat_state_at, replay};
use ori3_model::{Document, EPS, FaceId, Paper};

/// 浮いている(z≠0)と判断する高さ。t=0.99 では紙の大きさに対して十分大きい。
const LIFT_EPS: f64 = 1e-3;
/// 正面積の重なりだけを拾う。製品側surface orderと同じ面積閾値。
const OVERLAP_AREA_EPS: f64 = 1e-14;

/// 2つの投影多角形が正面積で重なる領域の内点を返す。
///
/// 面は凹の場合もあるため、まずear clippingで三角形へ分け、三角形対を凸clipする。
/// 境界が触れるだけの対は面積が無いので返さない。
fn overlap_witnesses(left: &[[f64; 3]], right: &[[f64; 3]]) -> Result<Vec<DVec2>, String> {
    let left = left
        .iter()
        .map(|point| DVec2::new(point[0], point[1]))
        .collect::<Vec<_>>();
    let right = right
        .iter()
        .map(|point| DVec2::new(point[0], point[1]))
        .collect::<Vec<_>>();
    let mut witnesses = Vec::new();
    for left_triangle in triangulate_polygon(&left)? {
        for right_triangle in triangulate_polygon(&right)? {
            let intersection = intersect_convex_polygons(&left_triangle, &right_triangle);
            if polygon_area(&intersection).abs() <= OVERLAP_AREA_EPS {
                continue;
            }
            let center = intersection.iter().copied().sum::<DVec2>() / intersection.len() as f64;
            witnesses.push(center);
            // 中心だけが高さ0付近でも、重なり領域の別の場所に明確な上下があれば拾う。
            witnesses.extend(
                intersection
                    .iter()
                    .copied()
                    .map(|point| (point + center) * 0.5),
            );
        }
    }
    Ok(witnesses)
}

fn triangulate_polygon(boundary: &[DVec2]) -> Result<Vec<Vec<DVec2>>, String> {
    let mut polygon = simple_polygon(boundary);
    if polygon.len() < 3 || polygon_area(&polygon).abs() <= OVERLAP_AREA_EPS {
        return Err("退化した投影多角形".to_string());
    }
    if polygon_area(&polygon) < 0.0 {
        polygon.reverse();
    }
    let mut triangles = Vec::with_capacity(polygon.len().saturating_sub(2));
    while polygon.len() > 3 {
        let count = polygon.len();
        let Some(ear) = (0..count).find(|&index| {
            let a = polygon[(index + count - 1) % count];
            let b = polygon[index];
            let c = polygon[(index + 1) % count];
            (b - a).perp_dot(c - b) > EPS * EPS
                && !polygon.iter().enumerate().any(|(other, &point)| {
                    other != index
                        && other != (index + count - 1) % count
                        && other != (index + 1) % count
                        && point_in_triangle(point, a, b, c)
                })
        }) else {
            return Err("投影多角形を三角形分割できない".to_string());
        };
        triangles.push(vec![
            polygon[(ear + count - 1) % count],
            polygon[ear],
            polygon[(ear + 1) % count],
        ]);
        polygon.remove(ear);
    }
    triangles.push(polygon);
    Ok(triangles)
}

fn simple_polygon(boundary: &[DVec2]) -> Vec<DVec2> {
    let mut polygon = Vec::with_capacity(boundary.len());
    for &point in boundary {
        if polygon
            .last()
            .is_none_or(|previous: &DVec2| (*previous - point).length() > EPS)
        {
            polygon.push(point);
        }
    }
    while polygon.len() > 1 && (polygon[0] - polygon[polygon.len() - 1]).length() <= EPS {
        polygon.pop();
    }
    polygon
}

fn point_in_triangle(point: DVec2, a: DVec2, b: DVec2, c: DVec2) -> bool {
    (b - a).perp_dot(point - a) >= -EPS
        && (c - b).perp_dot(point - b) >= -EPS
        && (a - c).perp_dot(point - c) >= -EPS
}

fn intersect_convex_polygons(subject: &[DVec2], clip: &[DVec2]) -> Vec<DVec2> {
    let mut output = subject.to_vec();
    for index in 0..clip.len() {
        let clip_start = clip[index];
        let clip_end = clip[(index + 1) % clip.len()];
        let input = std::mem::take(&mut output);
        let Some(mut previous) = input.last().copied() else {
            break;
        };
        let mut previous_side = (clip_end - clip_start).perp_dot(previous - clip_start);
        for current in input {
            let current_side = (clip_end - clip_start).perp_dot(current - clip_start);
            let previous_inside = previous_side >= -EPS;
            let current_inside = current_side >= -EPS;
            if previous_inside != current_inside {
                let denominator = previous_side - current_side;
                if denominator.abs() > EPS * EPS {
                    output.push(previous + (current - previous) * (previous_side / denominator));
                }
            }
            if current_inside {
                output.push(current);
            }
            previous = current;
            previous_side = current_side;
        }
    }
    deduplicate_polygon(output)
}

fn deduplicate_polygon(points: Vec<DVec2>) -> Vec<DVec2> {
    let mut output = Vec::with_capacity(points.len());
    for point in points {
        if output
            .last()
            .is_none_or(|previous: &DVec2| (*previous - point).length() > EPS)
        {
            output.push(point);
        }
    }
    if output.len() > 1 && (output[0] - output[output.len() - 1]).length() <= EPS {
        output.pop();
    }
    output
}

fn polygon_area(polygon: &[DVec2]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }
    0.5 * (0..polygon.len())
        .map(|index| polygon[index].perp_dot(polygon[(index + 1) % polygon.len()]))
        .sum::<f64>()
}

/// 3D面の平面を投影点(x,y)で評価した高さ。t=0.99の面は水平に近い。
fn height_at(polygon: &[[f64; 3]], point: DVec2) -> Option<f64> {
    let origin = DVec3::from(*polygon.first()?);
    for index in 1..polygon.len().saturating_sub(1) {
        let first = DVec3::from(polygon[index]) - origin;
        let second = DVec3::from(polygon[index + 1]) - origin;
        let normal = first.cross(second);
        if normal.z.abs() > EPS * EPS {
            return Some(
                origin.z
                    - (normal.x * (point.x - origin.x) + normal.y * (point.y - origin.y))
                        / normal.z,
            );
        }
    }
    None
}

/// t=1の面上の点と同じ材質点をt=0.99の面へ移し、その高さを返す。
/// 面の頂点対応からaffine座標を解くため、途中姿勢で投影位置がずれても別の材質点を
/// 比べることがない。
fn approached_height_at(exact: &[[f64; 3]], approached: &[[f64; 3]], point: DVec3) -> Option<f64> {
    if exact.len() != approached.len() || exact.len() < 3 {
        return None;
    }
    let exact_points = exact.iter().copied().map(DVec3::from).collect::<Vec<_>>();
    let approached_points = approached
        .iter()
        .copied()
        .map(DVec3::from)
        .collect::<Vec<_>>();
    let exact_origin = exact_points[0];
    let (first, second) = (1..exact_points.len()).find_map(|first| {
        (first + 1..exact_points.len())
            .find(|&second| {
                (exact_points[first] - exact_origin)
                    .cross(exact_points[second] - exact_origin)
                    .length_squared()
                    > EPS * EPS
            })
            .map(|second| (first, second))
    })?;
    let exact_first = exact_points[first] - exact_origin;
    let exact_second = exact_points[second] - exact_origin;
    let relative = point - exact_origin;
    let first_squared = exact_first.length_squared();
    let cross = exact_first.dot(exact_second);
    let second_squared = exact_second.length_squared();
    let determinant = first_squared * second_squared - cross * cross;
    if determinant.abs() <= EPS * EPS {
        return None;
    }
    let relative_first = relative.dot(exact_first);
    let relative_second = relative.dot(exact_second);
    let first_weight = (relative_first * second_squared - relative_second * cross) / determinant;
    let second_weight = (relative_second * first_squared - relative_first * cross) / determinant;
    let approached_origin = approached_points[0];
    let approached_point = approached_origin
        + (approached_points[first] - approached_origin) * first_weight
        + (approached_points[second] - approached_origin) * second_weight;
    approached_point.is_finite().then_some(approached_point.z)
}

fn faces_share_edge(faces: &[Face], left: FaceId, right: FaceId) -> bool {
    let left = faces
        .iter()
        .find(|face| face.id == left)
        .expect("左面がある");
    let right = faces
        .iter()
        .find(|face| face.id == right)
        .expect("右面がある");
    (0..left.vertices.len()).any(|left_index| {
        let left_start = left.vertices[left_index];
        let left_end = left.vertices[(left_index + 1) % left.vertices.len()];
        (0..right.vertices.len()).any(|right_index| {
            let right_start = right.vertices[right_index];
            let right_end = right.vertices[(right_index + 1) % right.vertices.len()];
            (left_start == right_start && left_end == right_end)
                || (left_start == right_end && left_end == right_start)
        })
    })
}

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

/// `up_to` 手目までの保存層順序と、幾何由来の表示順をそれぞれ検証する。
///
/// `Frame3D.layer` は `FlatState.order` の位置を保存する。`surface_rank` は
/// t=0.99で正面積が重なり、かつ高さ差を読める面対についてだけ相対順を照合する。
/// 幾何が拘束しない面のglobal rank値には物理的意味が無いので主張しない。
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

    // 保存用Frame3D.layerはFlatState.orderと同じ並びを返す。
    let exact = replay(doc, up_to, 1.0);
    assert!(
        exact.surface_order_provenance.is_some(),
        "{label}: t=1の表示順は材質seedではなくcompleteな幾何から決まる"
    );
    let frame = exact.frame;
    for f3 in &frame.faces {
        let expected = order.iter().position(|&id| id == f3.face).unwrap();
        assert_eq!(
            usize::try_from(f3.layer).unwrap(),
            expected,
            "{label}: 面 {} のlayerは層順序の位置と一致する",
            f3.face
        );
    }

    // 表示用surface_rankは保存順ではなく、折り切る直前の実際の高さに従う。
    let approached = replay(doc, up_to, 0.99).frame;
    let previous = replay(doc, up_to.saturating_sub(1), 1.0).frame;
    let mut overlap_pairs = 0_usize;
    let mut height_constrained_pairs = 0_usize;
    for left_index in 0..frame.faces.len() {
        for right_index in left_index + 1..frame.faces.len() {
            let left = &frame.faces[left_index];
            let right = &frame.faces[right_index];
            let witnesses =
                overlap_witnesses(&left.polygon, &right.polygon).unwrap_or_else(|error| {
                    panic!("{label}: 面対({}, {})の{error}", left.face, right.face)
                });
            if witnesses.is_empty() {
                continue;
            }
            overlap_pairs += 1;

            // 同じ高さの面対はこの姿勢から上下を決められない。global rankのtie-breakを
            // 答えと誤認せず、LIFT_EPSを越える実測差がある面対だけを拘束する。
            let mut direction = 0_i8;
            let mut strongest_witness = None::<(DVec2, f64)>;
            for witness in witnesses {
                let exact_point = DVec3::new(
                    witness.x,
                    witness.y,
                    height_at(&left.polygon, witness).unwrap_or_else(|| {
                        panic!("{label}: 面 {} の高さ平面を作れない", left.face)
                    }),
                );
                let approached_left = approached
                    .faces
                    .iter()
                    .find(|face| face.face == left.face)
                    .expect("t=0.99にも同じ面がある");
                let approached_right = approached
                    .faces
                    .iter()
                    .find(|face| face.face == right.face)
                    .expect("t=0.99にも同じ面がある");
                let left_height =
                    approached_height_at(&left.polygon, &approached_left.polygon, exact_point)
                        .unwrap_or_else(|| {
                            panic!("{label}: 面 {} の材質点を追跡できない", left.face)
                        });
                let right_height =
                    approached_height_at(&right.polygon, &approached_right.polygon, exact_point)
                        .unwrap_or_else(|| {
                            panic!("{label}: 面 {} の材質点を追跡できない", right.face)
                        });
                let difference = left_height - right_height;
                if strongest_witness.is_none_or(|(_, strongest)| difference.abs() > strongest.abs())
                {
                    strongest_witness = Some((witness, difference));
                }
                let witness_direction = if difference > LIFT_EPS {
                    1
                } else if difference < -LIFT_EPS {
                    -1
                } else {
                    0
                };
                if witness_direction != 0 {
                    if direction == 0 {
                        direction = witness_direction;
                    } else {
                        assert_eq!(
                            direction, witness_direction,
                            "{label}: 面対({}, {})は重なり領域内で高さ方向が交差する",
                            left.face, right.face
                        );
                    }
                }
            }
            if direction == 0 {
                continue;
            }
            height_constrained_pairs += 1;

            let left_rank = left.surface_rank;
            let right_rank = right.surface_rank;
            let (witness, difference) = strongest_witness.expect("高さを測った証人点がある");
            let previous_left = previous
                .faces
                .iter()
                .find(|face| face.face == left.face)
                .map(|face| (face.surface_rank, face.layer));
            let previous_right = previous
                .faces
                .iter()
                .find(|face| face.face == right.face)
                .map(|face| (face.surface_rank, face.layer));
            let saved_layer_agrees = if direction > 0 {
                left.layer > right.layer
            } else {
                left.layer < right.layer
            };
            let shared_hinge = faces_share_edge(&faces, left.face, right.face);
            if direction > 0 {
                assert!(
                    left_rank > right_rank,
                    "{label}: t=0.99で面{}が面{}より上ならt=1のrankも上({left_rank} > {right_rank}); witness=({:.9},{:.9}), dz={difference:.9e}, layer=({}, {}), saved_layer_agrees={saved_layer_agrees}, previous(rank,layer)=({previous_left:?}, {previous_right:?}), moving={lifted:?}, shared_hinge={shared_hinge}, order={order:?}",
                    left.face,
                    right.face,
                    witness.x,
                    witness.y,
                    left.layer,
                    right.layer
                );
            } else {
                assert!(
                    left_rank < right_rank,
                    "{label}: t=0.99で面{}が面{}より下ならt=1のrankも下({left_rank} < {right_rank}); witness=({:.9},{:.9}), dz={difference:.9e}, layer=({}, {}), saved_layer_agrees={saved_layer_agrees}, previous(rank,layer)=({previous_left:?}, {previous_right:?}), moving={lifted:?}, shared_hinge={shared_hinge}, order={order:?}",
                    left.face,
                    right.face,
                    witness.x,
                    witness.y,
                    left.layer,
                    right.layer
                );
            }
        }
    }
    assert!(overlap_pairs > 0, "{label}: 正面積で重なる面対がある");
    assert!(
        height_constrained_pairs > 0,
        "{label}: t=0.99の高さから相対順を決められる面対がある"
    );
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
